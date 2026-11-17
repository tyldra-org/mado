//! The mado command line (spec 14 §3).
//!
//! Talks to a running Mado profile only through its control endpoint.
//! Specification: `mado-docs/spec/14-control-cli-mcp.md`. Roadmap: M5-T07.

mod catalog;
mod guide;
mod profiles;
mod rpc;
mod tokens;

use std::io::IsTerminal as _;
use std::time::Duration;

use serde_json::{Value, json};

use rpc::{Conn, RpcError};

/// Exit codes (spec 14 §3).
const DONE: i32 = 0;
const FAILED: i32 = 1;
const USAGE: i32 = 2;
const NEEDS_PERSON: i32 = 3;
const NOT_ALLOWED: i32 = 4;
const STOPPED: i32 = 130;

const USAGE_TEXT: &str = "\
usage: mado \"task text\" [--profile P] [--mode ask|auto|full] [--access browser|full]
                         [--cli NAME] [-m MODEL] [--effort LEVEL] [--no-person]
                         [--wait-person MIN] [--json]
       mado <url>...
       mado session list | resume ID [\"text\"] | steer ID \"text\" | queue ID \"text\" | stop ID
       mado every \"SCHEDULE\" \"text\" | every list | run ID | pause ID | resume ID | remove ID
       mado settings get [KEY] | settings set KEY VALUE
       mado memory search \"words\" | memory list | memory show PATH
       mado skills list | skills show NAME
       mado tabs [--json]
       mado call COMMAND ['{json}']
       mado commands [--json]
       mado <group> <verb> [key=value...]     (mado <group> --help)
       mado mcp [--task ID]
       mado setup [claude|codex|cursor|path] | remove [...]
       mado status [--json] | quit [--force] | serve [--stop]
       mado guide [mcp]
";

/// Options every command takes.
#[derive(Debug, Clone, Default)]
struct Opts {
    profile: Option<String>,
    json: bool,
    no_person: bool,
    wait_person: Option<u64>,
    mode: Option<String>,
    access: Option<String>,
    cli: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    force: bool,
    stop: bool,
    task: Option<String>,
    rest: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--profile" => o.profile = Some(value("--profile")?),
            "--json" => o.json = true,
            "--no-person" => o.no_person = true,
            "--wait-person" => {
                o.wait_person = Some(
                    value("--wait-person")?
                        .parse()
                        .map_err(|_| "--wait-person takes minutes")?,
                );
            }
            "--mode" => o.mode = Some(value("--mode")?),
            "--access" => o.access = Some(value("--access")?),
            "--cli" => o.cli = Some(value("--cli")?),
            "-m" | "--model" => o.model = Some(value("-m")?),
            "--effort" => o.effort = Some(value("--effort")?),
            "--force" => o.force = true,
            "--stop" => o.stop = true,
            "--task" => o.task = Some(value("--task")?),
            "-h" | "--help" => o.rest.push("--help".into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            s => o.rest.push(s.to_owned()),
        }
    }
    Ok(o)
}

impl Opts {
    /// How long to wait for the person: `--wait-person`, else 10 minutes
    /// in a terminal and 15 seconds otherwise (14 §3).
    fn person_wait(&self) -> Duration {
        if self.no_person {
            return Duration::ZERO;
        }
        match self.wait_person {
            Some(m) => Duration::from_secs(m * 60),
            None if std::io::stdout().is_terminal() => Duration::from_secs(600),
            None => Duration::from_secs(15),
        }
    }
}

fn looks_like_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("mado://")
        || s.starts_with("file://")
}

/// A value from the command line: JSON when it parses, else a string.
fn value_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_owned()))
}

fn print(v: &Value, json: bool) {
    match v {
        Value::String(s) if !json => println!("{s}"),
        Value::Null if !json => {}
        _ => println!("{}", serde_json::to_string_pretty(v).unwrap_or_default()),
    }
}

fn fail(e: &RpcError) -> i32 {
    eprintln!("mado: {e}");
    if e.exit_code() == NEEDS_PERSON {
        eprintln!("mado: this needs you at the Mac.");
    }
    e.exit_code()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("mado: {e}\n{USAGE_TEXT}");
            std::process::exit(USAGE);
        }
    };
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        std::process::exit(FAILED);
    };
    let code = rt.block_on(async {
        tokio::select! {
            code = run(opts) => code,
            _ = tokio::signal::ctrl_c() => STOPPED,
        }
    });
    std::process::exit(code);
}

fn paths() -> Result<mado_platform::paths::Paths, i32> {
    mado_platform::paths::Paths::system().map_err(|e| {
        eprintln!("mado: {e}");
        FAILED
    })
}

/// The profile to talk to, started hidden when none runs.
fn profile(o: &Opts, start: bool) -> Result<profiles::Running, i32> {
    let p = paths()?;
    if let Some(r) = profiles::pick(&profiles::running(p.root()), o.profile.as_deref()) {
        return Ok(r);
    }
    if !start {
        return Err(FAILED);
    }
    profiles::start(p.root(), o.profile.as_deref()).map_err(|e| {
        eprintln!("mado: {e}");
        FAILED
    })
}

/// A session as the `mado` client: the stored token, or a pairing the
/// person approves on the Mac (14 §2).
async fn session(o: &Opts) -> Result<Conn, i32> {
    let r = profile(o, true)?;
    let account = format!("{}/mado", r.id);
    let mut conn = Conn::connect(&r.endpoint).await.map_err(|e| {
        eprintln!("mado: {e}");
        FAILED
    })?;
    let client = json!({ "name": "mado", "version": env!("CARGO_PKG_VERSION") });
    let token = tokens::get(&account);
    let hello = conn
        .call("hello", json!({ "client": client, "token": token }))
        .await;
    let hello = match hello {
        Err(e) if e.code == -32001 && token.is_some() => {
            // Revoked (14 §6 check 3): this call fails; the next one pairs
            // again.
            tokens::delete(&account);
            eprintln!("mado: this computer's access to Mado was revoked; run again to ask for it.");
            return Err(fail(&e));
        }
        other => other,
    }
    .map_err(|e| fail(&e))?;
    if hello["pairingRequired"] != true {
        return Ok(conn);
    }
    if o.no_person {
        eprintln!("mado: Mado needs you to allow mado on the Mac (--no-person).");
        return Err(NEEDS_PERSON);
    }
    eprintln!("mado: allow mado in Mado's window to continue…");
    let wait = o.person_wait();
    let answer = tokio::time::timeout(wait, async {
        loop {
            let n = conn.notification().await?;
            match n["method"].as_str() {
                Some("pairing.approved") => {
                    return Ok(Some(
                        n["params"]["token"].as_str().unwrap_or_default().to_owned(),
                    ));
                }
                Some("pairing.denied") => return Ok(None),
                _ => {}
            }
        }
    })
    .await;
    match answer {
        Ok(Ok(Some(token))) => {
            tokens::set(&account, &token);
            Ok(conn)
        }
        Ok(Ok(None)) => {
            eprintln!("mado: denied on the Mac.");
            Err(NOT_ALLOWED)
        }
        Ok(Err(e)) => Err(fail(&e)),
        Err(_) => {
            eprintln!("mado: nobody answered on the Mac.");
            Err(NEEDS_PERSON)
        }
    }
}

async fn call(o: &Opts, method: &str, params: Value) -> i32 {
    let mut conn = match session(o).await {
        Ok(c) => c,
        Err(code) => return code,
    };
    match conn.call(method, params).await {
        Ok(v) => {
            print(&v, o.json || !v.is_string());
            DONE
        }
        Err(e) => fail(&e),
    }
}

async fn run(o: Opts) -> i32 {
    let rest: Vec<&str> = o.rest.iter().map(String::as_str).collect();
    match rest.as_slice() {
        [] | ["--help"] | ["help"] => {
            print!("{USAGE_TEXT}");
            if rest.is_empty() { USAGE } else { DONE }
        }
        ["guide"] => {
            print!("{}", guide::GUIDE);
            DONE
        }
        ["guide", "mcp"] => {
            print!("{}", guide::MCP);
            DONE
        }
        ["status"] => status(&o),
        ["quit"] => quit(&o).await,
        ["serve"] if o.stop => quit(&o).await,
        ["serve"] => match profile(&o, true) {
            Ok(r) => {
                println!("{} ({})", r.name, r.id);
                DONE
            }
            Err(c) => c,
        },
        ["commands"] => commands(o.json),
        ["call", name] => call(&o, name, Value::Null).await,
        ["call", name, params] => match serde_json::from_str::<Value>(params) {
            Ok(p) => call(&o, name, p).await,
            Err(e) => {
                eprintln!("mado: the parameters are not JSON: {e}");
                USAGE
            }
        },
        ["tabs"] => tabs(&o).await,
        ["settings", "get"] => call(&o, "settings.get", json!({ "keys": null })).await,
        ["settings", "get", key] => call(&o, "settings.get", json!({ "keys": [key] })).await,
        ["settings", "set", key, value] => {
            call(
                &o,
                "settings.set",
                json!({ "key": key, "value": value_of(value), "token": null }),
            )
            .await
        }
        ["memory", "search", words] => {
            call(&o, "ai.memory.search", json!({ "query": words })).await
        }
        ["memory", "list"] => call(&o, "ai.memory.list", Value::Null).await,
        ["memory", "show", path] => call(&o, "ai.memory.read", json!({ "path": path })).await,
        ["skills", "list"] => call(&o, "ai.skill.list", Value::Null).await,
        ["skills", "show", name] => call(&o, "ai.skill.read", json!({ "name": name })).await,
        ["session", "list"] => call(&o, "ai.task.list", Value::Null).await,
        ["session", "resume", id] => {
            follow(&o, "ai.task.resume", json!({ "taskId": id, "text": null })).await
        }
        ["session", "resume", id, text] => {
            follow(&o, "ai.task.resume", json!({ "taskId": id, "text": text })).await
        }
        ["session", "steer", id, text] => {
            call(&o, "ai.task.steer", json!({ "taskId": id, "text": text })).await
        }
        ["session", "queue", id, text] => {
            call(&o, "ai.task.queue", json!({ "taskId": id, "text": text })).await
        }
        ["session", "stop", id] => call(&o, "ai.task.stop", json!({ "taskId": id })).await,
        ["every", "list"] => call(&o, "ai.routine.list", Value::Null).await,
        ["every", verb @ ("run" | "pause" | "resume" | "remove"), id] => {
            call(
                &o,
                &format!("ai.routine.{verb}"),
                json!({ "routineId": id }),
            )
            .await
        }
        ["every", schedule, text] => {
            call(
                &o,
                "ai.routine.add",
                json!({ "schedule": schedule, "prompt": text, "mode": o.mode }),
            )
            .await
        }
        ["mcp"] => {
            eprintln!("mado: the MCP server arrives with M5-T08.");
            FAILED
        }
        ["setup" | "remove", ..] => {
            eprintln!("mado: setup arrives with M5-T16.");
            FAILED
        }
        urls if urls.iter().all(|u| looks_like_url(u)) => open(&o, urls).await,
        [group, rest @ ..]
            if catalog::all()
                .iter()
                .any(|e| catalog::cli_group(e.group()) == *group) =>
        {
            group_command(&o, group, rest).await
        }
        [text] => task(&o, text).await,
        _ => {
            eprintln!("mado: unknown command\n{USAGE_TEXT}");
            USAGE
        }
    }
}

fn status(o: &Opts) -> i32 {
    let Ok(p) = paths() else { return FAILED };
    let list = profiles::running(p.root());
    let wanted = profiles::pick(&list, o.profile.as_deref());
    if o.json {
        let rows: Vec<Value> = list
            .iter()
            .map(|r| json!({ "profileId": r.id, "name": r.name, "pid": r.pid }))
            .collect();
        println!(
            "{}",
            json!({ "running": wanted.is_some(), "profiles": rows })
        );
    } else {
        for r in &list {
            println!("{} ({}) pid {}", r.name, r.id, r.pid);
        }
        if list.is_empty() {
            println!("Mado is not running.");
        }
    }
    if wanted.is_some() { DONE } else { FAILED }
}

/// Opens URLs in front of the person, as a link from another app would.
async fn open(o: &Opts, urls: &[&str]) -> i32 {
    let r = match profile(o, true) {
        Ok(r) => r,
        Err(c) => return c,
    };
    forward(&r, urls.iter().map(|u| (*u).to_owned()).collect(), None).await
}

async fn forward(
    r: &profiles::Running,
    urls: Vec<String>,
    control: Option<mado_types::command::OwnerAction>,
) -> i32 {
    let Ok(p) = paths() else { return FAILED };
    let Ok(id) = r.id.parse() else { return FAILED };
    let secret = match mado_control::LaunchSecret::read(&p.launch_secret(id)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mado: {e}");
            return FAILED;
        }
    };
    let request = mado_types::command::LaunchRequest {
        urls,
        files: Vec::new(),
        new_window: false,
        private: false,
        source: mado_types::command::LaunchSource::Cli,
        control,
    };
    match mado_control::forward(&r.endpoint, &secret, &request).await {
        Ok(()) => DONE,
        Err(e) => {
            eprintln!("mado: {e}");
            FAILED
        }
    }
}

async fn quit(o: &Opts) -> i32 {
    let Ok(r) = profile(o, false) else {
        return DONE;
    };
    if o.force {
        // SIGTERM: the app quits without questions (spec 01 §1.6).
        let killed = std::process::Command::new("/bin/kill")
            .args(["-TERM", &r.pid.to_string()])
            .status()
            .is_ok_and(|s| s.success());
        return if killed { DONE } else { FAILED };
    }
    forward(&r, Vec::new(), Some(mado_types::command::OwnerAction::Quit)).await
}

fn commands(json: bool) -> i32 {
    let all = catalog::all();
    if json {
        let rows: Vec<Value> = all
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "params": e.params.iter().map(|(k, t)| json!({ "name": k, "type": t })).collect::<Vec<_>>(),
                    "mayNeedFullControl": e.may_need_full(),
                })
            })
            .collect();
        println!("{}", Value::Array(rows));
    } else {
        for e in &all {
            let params: Vec<String> = e.params.iter().map(|(k, t)| format!("{k}: {t}")).collect();
            let mark = if e.may_need_full() {
                "  [may need Full control]"
            } else {
                ""
            };
            println!("{}  {}{mark}", e.name, params.join(", "));
        }
    }
    DONE
}

async fn group_command(o: &Opts, group: &str, rest: &[&str]) -> i32 {
    let verbs: Vec<catalog::Entry> = catalog::all()
        .into_iter()
        .filter(|e| catalog::cli_group(e.group()) == group)
        .collect();
    let [verb, args @ ..] = rest else {
        return group_help(group, &verbs);
    };
    if *verb == "--help" {
        return group_help(group, &verbs);
    }
    let name = catalog::resolve(group, verb);
    let Some(entry) = verbs.iter().find(|e| e.name == name) else {
        eprintln!("mado: {group} has no {verb}");
        group_help(group, &verbs);
        return USAGE;
    };
    let mut params = serde_json::Map::new();
    // Optional parameters default to null, so key=value lists only what
    // matters.
    for (k, t) in &entry.params {
        if t.contains("null") {
            params.insert(k.clone(), Value::Null);
        }
    }
    for a in args {
        let Some((k, v)) = a.split_once('=') else {
            // A lone value fills the first parameter: `password reveal ID`.
            if let Some((k, _)) = entry.params.first() {
                params.insert(k.clone(), value_of(a));
                continue;
            }
            eprintln!("mado: expected key=value, got {a}");
            return USAGE;
        };
        params.insert(k.to_owned(), value_of(v));
    }
    call(o, &name, Value::Object(params)).await
}

fn group_help(group: &str, verbs: &[catalog::Entry]) -> i32 {
    println!("mado {group} <verb> [key=value...]");
    for e in verbs {
        let verb = e.name.split_once('.').map_or(e.name.as_str(), |(_, v)| v);
        let params: Vec<String> = e.params.iter().map(|(k, t)| format!("{k}: {t}")).collect();
        println!("  {verb}  {}", params.join(", "));
    }
    DONE
}

async fn tabs(o: &Opts) -> i32 {
    let mut conn = match session(o).await {
        Ok(c) => c,
        Err(code) => return code,
    };
    let state = match conn.call("state.get", Value::Null).await {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let tabs = &state["state"]["tabs"];
    if o.json {
        println!("{tabs}");
    } else if let Some(map) = tabs.as_object() {
        for (id, t) in map {
            println!(
                "{id}  {}  {}",
                t["title"].as_str().unwrap_or_default(),
                t["url"].as_str().unwrap_or_default()
            );
        }
    }
    DONE
}

/// `mado "task"` (14 §3): starts a task in Mado's agent and follows it.
async fn task(o: &Opts, text: &str) -> i32 {
    let params = json!({
        "prompt": text,
        "mode": o.mode,
        "access": o.access,
        "cli": o.cli,
        "model": o.model,
        "effort": o.effort,
    });
    follow(o, "ai.task.start", params).await
}

async fn follow(o: &Opts, method: &str, params: Value) -> i32 {
    let mut conn = match session(o).await {
        Ok(c) => c,
        Err(code) => return code,
    };
    let started = match conn.call(method, params).await {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let Some(id) = started["taskId"].as_str().map(str::to_owned) else {
        print(&started, o.json);
        return DONE;
    };
    println!("session {id}");
    if let Err(e) = conn.call("task.events", json!({ "taskId": id })).await {
        return fail(&e);
    }
    let wait = o.person_wait();
    loop {
        let n = match conn.notification().await {
            Ok(n) => n,
            Err(e) => return fail(&e),
        };
        let p = &n["params"];
        match n["method"].as_str() {
            Some("task.step") => {
                if o.json {
                    println!("{p}");
                } else {
                    println!("{}", p["text"].as_str().unwrap_or_default());
                }
            }
            Some("task.waiting") => {
                let what = p["reason"].as_str().unwrap_or("you");
                if wait.is_zero() {
                    println!("waiting: {what}");
                    println!("answer: mado session resume {id} \"...\"");
                    let _ = conn.call("ai.task.detach", json!({ "taskId": id })).await;
                    return NEEDS_PERSON;
                }
                eprintln!("mado: waiting for you: {what}");
                let waited = tokio::time::timeout(wait, async {
                    loop {
                        match conn.notification().await {
                            Ok(n) if n["method"] == "task.waiting" => continue,
                            other => return other,
                        }
                    }
                })
                .await;
                match waited {
                    Err(_) => {
                        println!("answer: mado session resume {id} \"...\"");
                        return NEEDS_PERSON;
                    }
                    Ok(Err(e)) => return fail(&e),
                    Ok(Ok(n)) => {
                        if let Some(code) = done(&n, o.json) {
                            return code;
                        }
                    }
                }
            }
            _ => {
                if let Some(code) = done(&n, o.json) {
                    return code;
                }
            }
        }
    }
}

fn done(n: &Value, json: bool) -> Option<i32> {
    if n["method"] != "task.done" {
        return None;
    }
    let p = &n["params"];
    if json {
        println!("{p}");
    } else if let Some(a) = p["answer"].as_str() {
        println!("{a}");
    }
    Some(match p["status"].as_str() {
        Some("done") => DONE,
        Some("cancelled") => STOPPED,
        Some("waiting") => NEEDS_PERSON,
        _ => FAILED,
    })
}
