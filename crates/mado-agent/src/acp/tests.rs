use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

/// A scripted ACP agent on the other end of a duplex pipe.
struct Agent {
    lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    w: tokio::io::WriteHalf<DuplexStream>,
}

impl Agent {
    async fn recv(&mut self) -> Value {
        let l = self.lines.next_line().await.expect("read").expect("a line");
        serde_json::from_str(&l).expect("json")
    }
    async fn send(&mut self, v: Value) {
        self.w
            .write_all(format!("{v}\n").as_bytes())
            .await
            .expect("write");
    }
    async fn result(&mut self, id: &Value, r: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": r }))
            .await;
    }
    /// initialize and session/new; answers the first prompt's request.
    async fn open(&mut self) -> Value {
        let init = self.recv().await;
        assert_eq!(init["method"], "initialize");
        assert_eq!(init["params"]["clientCapabilities"]["terminal"], false);
        self.result(
            &init["id"],
            json!({ "protocolVersion": 1, "agentCapabilities": {} }),
        )
        .await;
        let new = self.recv().await;
        assert_eq!(new["method"], "session/new");
        let server = &new["params"]["mcpServers"][0];
        assert_eq!(server["name"], "mado");
        assert_eq!(server["args"], json!(["mcp", "--task", "t1"]));
        assert_eq!(
            server["env"][0],
            json!({ "name": "MADO_TASK_TOKEN", "value": "tok" })
        );
        self.result(&new["id"], json!({ "sessionId": "s1" })).await;
        let prompt = self.recv().await;
        assert_eq!(prompt["method"], "session/prompt");
        prompt
    }
    async fn update(&mut self, u: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1", "update": u } })).await;
    }
}

fn session(dir: &Path, full: bool) -> Session {
    Session {
        cwd: dir.to_owned(),
        mcp: McpServer {
            name: "mado".into(),
            command: "/Applications/Mado.app/Contents/MacOS/mado".into(),
            args: vec!["mcp".into(), "--task".into(), "t1".into()],
            env: vec![("MADO_TASK_TOKEN".into(), "tok".into())],
        },
        prompt: "Find the price".into(),
        full,
    }
}

fn start(
    s: Session,
    ctl: Steering,
) -> (
    Agent,
    tokio::task::JoinHandle<End>,
    mpsc::UnboundedReceiver<Event>,
) {
    let (a, b) = tokio::io::duplex(1 << 16);
    let (ar, aw) = tokio::io::split(a);
    let (br, bw) = tokio::io::split(b);
    let (tx, rx) = mpsc::unbounded_channel();
    let h = tokio::spawn(drive(br, bw, s, ctl, tx));
    (
        Agent {
            lines: BufReader::new(ar).lines(),
            w: aw,
        },
        h,
        rx,
    )
}

#[tokio::test]
async fn a_session_runs_a_prompt_with_steps_and_a_permission() {
    let dir = tempfile::tempdir().expect("dir");
    let (mut agent, h, mut events) = start(session(dir.path(), false), Steering::default());
    let prompt = agent.open().await;
    assert_eq!(prompt["params"]["prompt"][0]["text"], "Find the price");
    agent.update(json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "title": "mcp__mado__open", "kind": "other", "status": "pending" })).await;
    agent.send(json!({ "jsonrpc": "2.0", "id": 7, "method": "session/request_permission", "params": {
        "sessionId": "s1",
        "toolCall": { "toolCallId": "c2", "title": "Bash: rm -rf ~", "kind": "execute", "rawInput": { "command": "rm -rf ~" } },
        "options": [
            { "optionId": "yes", "name": "Allow", "kind": "allow_once" },
            { "optionId": "no", "name": "Reject", "kind": "reject_once" }
        ]
    } })).await;
    let Some(Event::Step(step)) = events.recv().await else {
        panic!("a step")
    };
    assert_eq!(step, "mcp__mado__open");
    let Some(Event::Ask(ask, answer)) = events.recv().await else {
        panic!("an ask")
    };
    assert_eq!(ask.kind, "execute");
    assert!(!ask.is_mado_tool("mado"));
    answer.send(false);
    let r = agent.recv().await;
    assert_eq!(r["id"], 7);
    assert_eq!(
        r["result"]["outcome"],
        json!({ "outcome": "selected", "optionId": "no" })
    );
    agent.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "It costs " } })).await;
    agent.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "$5." } })).await;
    agent
        .result(&prompt["id"], json!({ "stopReason": "end_turn" }))
        .await;
    assert_eq!(h.await.expect("join"), End::Done("It costs $5.".into()));
    assert!(
        matches!(events.recv().await, Some(Event::TurnEnded { stop_reason }) if stop_reason == "end_turn")
    );
}

#[tokio::test]
async fn steering_cancels_the_turn_and_prompts_again() {
    let dir = tempfile::tempdir().expect("dir");
    let ctl = Steering::default();
    let (mut agent, h, _events) = start(session(dir.path(), false), ctl.clone());
    let prompt = agent.open().await;
    ctl.steer.lock().expect("lock").push("only new ones".into());
    let cancel = agent.recv().await;
    assert_eq!(cancel["method"], "session/cancel");
    assert!(cancel.get("id").is_none());
    agent
        .result(&prompt["id"], json!({ "stopReason": "cancelled" }))
        .await;
    let again = agent.recv().await;
    assert_eq!(again["method"], "session/prompt");
    assert_eq!(
        again["params"]["prompt"][0]["text"],
        "The person adds: only new ones"
    );
    ctl.queue
        .lock()
        .expect("lock")
        .push("then the reviews".into());
    agent
        .result(&again["id"], json!({ "stopReason": "end_turn" }))
        .await;
    let next = agent.recv().await;
    assert_eq!(
        next["params"]["prompt"][0]["text"],
        "Next, the person asks: then the reviews"
    );
    agent.update(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Done" } })).await;
    agent
        .result(&next["id"], json!({ "stopReason": "end_turn" }))
        .await;
    assert_eq!(h.await.expect("join"), End::Done("Done".into()));
}

#[tokio::test]
async fn stop_cancels_and_ends() {
    let dir = tempfile::tempdir().expect("dir");
    let ctl = Steering::default();
    let (mut agent, h, _events) = start(session(dir.path(), false), ctl.clone());
    let prompt = agent.open().await;
    ctl.stop.store(true, Ordering::SeqCst);
    assert_eq!(agent.recv().await["method"], "session/cancel");
    agent
        .result(&prompt["id"], json!({ "stopReason": "cancelled" }))
        .await;
    assert_eq!(h.await.expect("join"), End::Cancelled);
}

#[tokio::test]
async fn files_in_the_task_folder_are_free_and_others_ask() {
    let dir = tempfile::tempdir().expect("dir");
    let (mut agent, h, mut events) = start(session(dir.path(), true), Steering::default());
    let prompt = agent.open().await;
    let inside = dir.path().join("notes.txt");
    agent.send(json!({ "jsonrpc": "2.0", "id": 20, "method": "fs/write_text_file", "params": { "sessionId": "s1", "path": inside, "content": "a\nb\nc" } })).await;
    assert_eq!(agent.recv().await["id"], 20);
    agent.send(json!({ "jsonrpc": "2.0", "id": 21, "method": "fs/read_text_file", "params": { "sessionId": "s1", "path": inside, "line": 2, "limit": 1 } })).await;
    assert_eq!(agent.recv().await["result"]["content"], "b");
    agent.send(json!({ "jsonrpc": "2.0", "id": 22, "method": "fs/read_text_file", "params": { "sessionId": "s1", "path": "/etc/hosts" } })).await;
    let Some(Event::Ask(ask, answer)) = events.recv().await else {
        panic!("an ask")
    };
    assert_eq!(ask.path.as_deref(), Some(Path::new("/etc/hosts")));
    answer.send(false);
    assert_eq!(
        agent.recv().await["error"]["message"],
        "denied by the person"
    );
    agent
        .result(&prompt["id"], json!({ "stopReason": "end_turn" }))
        .await;
    assert_eq!(h.await.expect("join"), End::Done(String::new()));
}

#[tokio::test]
async fn without_full_access_files_and_terminals_are_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let (mut agent, h, _events) = start(session(dir.path(), false), Steering::default());
    let prompt = agent.open().await;
    agent.send(json!({ "jsonrpc": "2.0", "id": 30, "method": "fs/read_text_file", "params": { "path": dir.path().join("x") } })).await;
    assert_eq!(agent.recv().await["error"]["code"], -32601);
    agent.send(json!({ "jsonrpc": "2.0", "id": 31, "method": "terminal/create", "params": { "command": "ls" } })).await;
    assert_eq!(agent.recv().await["error"]["code"], -32601);
    agent
        .result(&prompt["id"], json!({ "stopReason": "end_turn" }))
        .await;
    assert!(matches!(h.await.expect("join"), End::Done(_)));
}

#[test]
fn paths_inside_the_folder() {
    assert!(inside(Path::new("/t/a/b"), Path::new("/t/a")));
    assert!(!inside(Path::new("/t/a/../b"), Path::new("/t/a")));
    assert!(!inside(Path::new("a/b"), Path::new("a")));
}
