//! Tasks and the Mado runtime (spec 13 §7.1, §7.2, §7.4): a loop that asks
//! the `tasks` model, runs its tool calls through the tools and their
//! approvals, and ends with an answer and its sources; steering, queueing,
//! questions to the person, helpers, limits and followers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mado_ai::{Message, ModelRequest, Part, Role, ToolSpec};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::{TaskStatus, TaskSummary};
use mado_types::{ErrorCode, MadoError, TaskId};
use rusqlite::params;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::chat::{Attempt, answer};
use super::{Core, Input, Internal, Reply};

/// 13 §7.1: at most this many tasks run at once; the rest wait.
const RUNNING_MAX: usize = 3;
/// 13 §7.2: steps per task.
const STEPS_MAX: usize = 200;
/// Helpers per task at most (`ai.helpersPerTask`, max 6).
const HELPERS_MAX: usize = 6;
/// Agent tabs close this long after their task ends (13 §7.3).
pub(super) const TABS_CLOSE_AFTER: Duration = Duration::from_secs(15 * 60);

/// What the core keeps for a task while it lives.
#[derive(Debug)]
pub(super) struct TaskCtl {
    pub prompt: String,
    pub steer: Arc<Mutex<Vec<String>>>,
    pub queue: Arc<Mutex<Vec<String>>>,
    pub waiting: Arc<Mutex<Option<oneshot::Sender<String>>>>,
    pub stop: Arc<AtomicBool>,
    pub followers: Vec<mpsc::UnboundedSender<Value>>,
    pub detached: bool,
    pub steps: Vec<String>,
    pub start: Option<Start>,
}

/// How a queued task starts.
#[derive(Debug, Clone)]
pub(super) struct Start {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// What a running task tells the core.
#[derive(Debug)]
pub enum TaskEvent {
    Step(String),
    Waiting(String),
    Resumed,
    Done {
        status: TaskStatus,
        answer: String,
        sources: Vec<String>,
    },
}

/// Everything the loop needs, so it runs off the core's state.
struct Env {
    task: TaskId,
    inputs: mpsc::UnboundedSender<Input>,
    attempts: Vec<Attempt>,
    secrets: Arc<dyn mado_ai::secrets::Secrets>,
    effort: mado_ai::Effort,
    system: String,
    prompt: String,
    tools: Vec<ToolSpec>,
    ctx: super::tools::Ctx,
    steer: Arc<Mutex<Vec<String>>>,
    queue: Arc<Mutex<Vec<String>>>,
    waiting: Arc<Mutex<Option<oneshot::Sender<String>>>>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    depth: u8,
}

impl Env {
    fn emit(&self, e: TaskEvent) {
        let _ = self.inputs.send(Input::Internal(Internal::TaskEvent {
            task: self.task,
            event: e,
        }));
    }

    /// Waits for the person's answer (`ask`, `handoff`, a spend limit).
    async fn person(&self, reason: String) -> Option<String> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut w) = self.waiting.lock() {
            *w = Some(tx);
        }
        self.emit(TaskEvent::Waiting(reason));
        let stop = Arc::clone(&self.stop);
        let answered = tokio::select! {
            a = rx => a.ok(),
            () = async move {
                while !stop.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => None,
        };
        self.emit(TaskEvent::Resumed);
        answered
    }
}

/// The tools a model is offered: every tool but those whose task has not
/// come yet (memory and skills, M5-T14).
fn offered() -> Vec<ToolSpec> {
    mado_types::tools::all()
        .into_iter()
        .filter(|t| {
            !matches!(
                t.name,
                "memory_search" | "memory_read" | "remember" | "skills" | "skill" | "skill_save"
            )
        })
        .map(|t| ToolSpec {
            name: t.name.into(),
            description: t.description.into(),
            parameters: t.parameters,
        })
        .collect()
}

fn summary(name: &str, args: &Value) -> String {
    let what = [
        "url", "ref", "keys", "query", "question", "reason", "action", "prompt",
    ]
    .iter()
    .find_map(|k| args.get(*k).and_then(Value::as_str))
    .unwrap_or_default();
    let what: String = what.chars().take(80).collect();
    if what.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {what}")
    }
}

/// How a run ends: its status, answer and sources.
type Ending =
    std::pin::Pin<Box<dyn std::future::Future<Output = (TaskStatus, String, Vec<String>)>>>;

/// The Mado runtime's loop (13 §7.2).
fn run(env: Env) -> Ending {
    Box::pin(async move {
        let mut messages = vec![Message::user(env.prompt.clone())];
        let mut sources: Vec<String> = Vec::new();
        let mut helpers = 0usize;
        for _ in 0..STEPS_MAX {
            if env.stop.load(Ordering::SeqCst) {
                return (TaskStatus::Cancelled, String::new(), sources);
            }
            if Instant::now() > env.deadline {
                env.emit(TaskEvent::Step("time limit reached".into()));
                return (
                    TaskStatus::Failed,
                    "The task ran out of time.".into(),
                    sources,
                );
            }
            let steering: Vec<String> = env
                .steer
                .lock()
                .map(|mut s| std::mem::take(&mut *s))
                .unwrap_or_default();
            for s in steering {
                messages.push(Message::user(format!("The person adds: {s}")));
            }
            let Some(at) = env.attempts.first() else {
                return (
                    TaskStatus::Failed,
                    "No model for the tasks role.".into(),
                    sources,
                );
            };
            let req = ModelRequest {
                model: at.model.clone(),
                system: env.system.clone(),
                messages: messages.clone(),
                tools: env.tools.clone(),
                effort: env.effort,
                max_output_tokens: 8192,
                web_search: false,
            };
            let mut turn = None;
            let mut used = at.clone();
            for at in &env.attempts {
                match answer(
                    &env.secrets,
                    at,
                    &ModelRequest {
                        model: at.model.clone(),
                        ..req.clone()
                    },
                    |_| {},
                )
                .await
                {
                    Ok(t) => {
                        turn = Some(t);
                        used = at.clone();
                        break;
                    }
                    Err(e) => {
                        let moves = matches!(e, mado_ai::ClientError::Failed(f) if f.fails_over());
                        env.emit(TaskEvent::Step(format!(
                            "model error: {}",
                            e.failure().key()
                        )));
                        if !moves {
                            break;
                        }
                    }
                }
            }
            let Some(turn) = turn else {
                return (
                    TaskStatus::Failed,
                    "The model did not answer.".into(),
                    sources,
                );
            };
            let (reply, rx) = oneshot::channel();
            let _ = env.inputs.send(Input::Internal(Internal::ModelUsage {
                task: env.task,
                connection: used.connection.clone(),
                provider: used.provider.key().into(),
                model: used.model.clone(),
                usage: turn.usage,
                reply,
            }));
            if rx.await.unwrap_or(false) {
                // 13 §9.6: paused until the limit is raised or the task stops.
                if env.person("spend limit".into()).await.is_none() {
                    return (TaskStatus::Cancelled, String::new(), sources);
                }
            }
            for (u, _) in &turn.sources {
                if !sources.contains(u) {
                    sources.push(u.clone());
                }
            }
            messages.push(turn.message.clone());
            let calls: Vec<(String, String, Value)> = turn
                .tool_calls()
                .into_iter()
                .map(|(i, n, a)| (i.to_owned(), n.to_owned(), a.clone()))
                .collect();
            if calls.is_empty() {
                let queued: Vec<String> = env
                    .queue
                    .lock()
                    .map(|mut q| std::mem::take(&mut *q))
                    .unwrap_or_default();
                if !queued.is_empty() {
                    for q in queued {
                        messages.push(Message::user(format!("Next, the person asks: {q}")));
                    }
                    continue;
                }
                for u in mado_agent::chat::sources_in(&turn.text) {
                    if !sources.contains(&u) {
                        sources.push(u);
                    }
                }
                return (TaskStatus::Done, turn.text, sources);
            }
            let mut results = Vec::new();
            let mut images = Vec::new();
            for (id, name, args) in calls {
                env.emit(TaskEvent::Step(summary(&name, &args)));
                let result: Result<Value, MadoError> = match name.as_str() {
                    "ask" => {
                        let q = args["question"].as_str().unwrap_or("A question").to_owned();
                        match env.person(q).await {
                            Some(a) => Ok(json!({ "answer": a })),
                            None => return (TaskStatus::Cancelled, String::new(), sources),
                        }
                    }
                    "handoff" => {
                        let r = args["reason"]
                            .as_str()
                            .unwrap_or("The person's turn")
                            .to_owned();
                        match env.person(r).await {
                            Some(a) => Ok(json!({ "handedBack": true, "note": a })),
                            None => return (TaskStatus::Cancelled, String::new(), sources),
                        }
                    }
                    "delegate" if env.depth == 0 && helpers < HELPERS_MAX => {
                        helpers += 1;
                        let sub = Env {
                            task: env.task,
                            inputs: env.inputs.clone(),
                            attempts: env.attempts.clone(),
                            secrets: Arc::clone(&env.secrets),
                            effort: env.effort,
                            system: env.system.clone(),
                            prompt: args["prompt"].as_str().unwrap_or_default().to_owned(),
                            tools: env
                                .tools
                                .iter()
                                .filter(|t| t.name != "delegate")
                                .cloned()
                                .collect(),
                            ctx: env.ctx.clone(),
                            steer: Arc::default(),
                            queue: Arc::default(),
                            waiting: Arc::clone(&env.waiting),
                            stop: Arc::clone(&env.stop),
                            deadline: env.deadline,
                            depth: 1,
                        };
                        let (status, text, found) = run(sub).await;
                        for u in found {
                            if !sources.contains(&u) {
                                sources.push(u);
                            }
                        }
                        Ok(json!({ "status": format!("{status:?}"), "answer": text }))
                    }
                    "delegate" => Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.ai.tooManyHelpers",
                    )),
                    _ => super::tools::run_audited(&env.ctx, &name, args.clone()).await,
                };
                if let Ok(v) = &result
                    && let Some(u) = v.get("url").and_then(Value::as_str)
                    && name != "fetch"
                    && !sources.iter().any(|s| s == u)
                {
                    sources.push(u.to_owned());
                }
                let (content, is_error) = match result {
                    Ok(mut v) => {
                        if let Some(img) = v.get("image").cloned()
                            && let Some(o) = v.as_object_mut()
                        {
                            o.remove("image");
                            images.push(Part::Image {
                                media_type: img["mimeType"].as_str().unwrap_or("image/png").into(),
                                data: img["data"].as_str().unwrap_or_default().into(),
                            });
                        }
                        let text = v.to_string();
                        (text.chars().take(40_000).collect::<String>(), false)
                    }
                    Err(e) => {
                        let why = if e.message_key == "error.ai.deniedByUser" {
                            "denied by user".to_owned()
                        } else {
                            e.message_key.clone()
                        };
                        (json!({ "error": why, "args": e.args }).to_string(), true)
                    }
                };
                results.push(Part::ToolResult {
                    id,
                    name,
                    content,
                    is_error,
                });
            }
            results.extend(images);
            messages.push(Message {
                role: Role::User,
                parts: results,
            });
        }
        (
            TaskStatus::Failed,
            "The task reached its step limit.".into(),
            sources,
        )
    })
}

impl Core {
    fn running_tasks(&self) -> usize {
        self.task_ctl.values().filter(|c| c.start.is_none()).count()
    }

    pub(super) fn task_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        if self.setting("features.agents") == false {
            return Err(MadoError::new(ErrorCode::Unsupported, "error.ai.agentsOff"));
        }
        let no_task = || MadoError::new(ErrorCode::NotFound, "error.ai.noTask");
        match cmd {
            Command::AiTaskStart {
                prompt,
                mode,
                cli,
                model,
                effort,
                scope,
                ..
            } => {
                if cli.is_some() {
                    return self.start_cli_task(origin, prompt, cli, mode, rec);
                }
                let task = TaskId::new();
                let title: String = prompt.chars().take(60).collect();
                self.state.tx(rec).ai().tasks().insert(
                    task,
                    TaskSummary {
                        id: task,
                        title,
                        status: TaskStatus::Queued,
                        step: String::new(),
                        group_id: None,
                        cost_micros: 0,
                    },
                );
                let mode = mode.as_deref().map(mado_agent::safety::Mode::from_key);
                let scope_prompt = match &scope {
                    Some(s) if !s.is_empty() => format!("{prompt} {}", s.join(" ")),
                    _ => prompt.clone(),
                };
                self.ensure_task_row(task, &prompt, "mado");
                self.set_scope(task, &scope_prompt, mode);
                self.task_ctl.insert(
                    task,
                    TaskCtl {
                        prompt,
                        steer: Arc::default(),
                        queue: Arc::default(),
                        waiting: Arc::default(),
                        stop: Arc::default(),
                        followers: Vec::new(),
                        detached: false,
                        steps: Vec::new(),
                        start: Some(Start { model, effort }),
                    },
                );
                self.task_queue.push_back(task);
                self.start_queued(rec);
                Ok(json!({ "taskId": task }))
            }
            Command::AiTaskSteer { task_id, text } => {
                let c = self.task_ctl.get(&task_id).ok_or_else(no_task)?;
                if let Ok(mut s) = c.steer.lock() {
                    s.push(text);
                }
                Ok(Value::Null)
            }
            Command::AiTaskQueue { task_id, text } => {
                let c = self.task_ctl.get(&task_id).ok_or_else(no_task)?;
                if let Ok(mut q) = c.queue.lock() {
                    q.push(text);
                }
                Ok(Value::Null)
            }
            Command::AiTaskAnswer { task_id, text } => {
                let c = self.task_ctl.get(&task_id).ok_or_else(no_task)?;
                let w = c.waiting.lock().ok().and_then(|mut w| w.take());
                match w {
                    Some(tx) => {
                        let _ = tx.send(text);
                        Ok(Value::Null)
                    }
                    None => Err(MadoError::new(ErrorCode::Conflict, "error.ai.notWaiting")),
                }
            }
            Command::AiTaskResume { task_id, text } => {
                let c = self.task_ctl.get_mut(&task_id).ok_or_else(no_task)?;
                c.detached = false;
                let w = c.waiting.lock().ok().and_then(|mut w| w.take());
                match (w, text) {
                    (Some(tx), t) => {
                        let _ = tx.send(t.unwrap_or_default());
                    }
                    (None, Some(t)) => {
                        if let Ok(mut s) = c.steer.lock() {
                            s.push(t);
                        }
                    }
                    (None, None) => {}
                }
                Ok(json!({ "taskId": task_id }))
            }
            Command::AiTaskStop { task_id } => {
                let c = self.task_ctl.get(&task_id).ok_or_else(no_task)?;
                c.stop.store(true, Ordering::SeqCst);
                if c.start.is_some() {
                    self.task_queue.retain(|t| *t != task_id);
                    self.task_finished(
                        task_id,
                        TaskStatus::Cancelled,
                        String::new(),
                        Vec::new(),
                        rec,
                    );
                }
                Ok(Value::Null)
            }
            Command::AiTaskDetach { task_id } => {
                if let Some(c) = self.task_ctl.get_mut(&task_id) {
                    c.detached = true;
                }
                Ok(Value::Null)
            }
            Command::AiTaskList => {
                let rows = self
                    .profile_db
                    .readers()
                    .read(|c| {
                        c.prepare("SELECT id, prompt, status, started_at, ended_at, cost_micros FROM ai_tasks ORDER BY started_at DESC LIMIT 200")?
                            .query_map([], |r| {
                                Ok(json!({
                                    "id": r.get::<_, String>(0)?, "prompt": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?,
                                    "startedAt": r.get::<_, i64>(3)?, "endedAt": r.get::<_, Option<i64>>(4)?, "costMicros": r.get::<_, i64>(5)?,
                                }))
                            })?
                            .collect::<rusqlite::Result<Vec<Value>>>()
                    })
                    .map_err(|_| MadoError::internal())?;
                Ok(Value::Array(rows))
            }
            Command::AiTaskGet { task_id } => self.task_get(task_id),
            Command::AiTaskExport { task_id, path } => {
                if !origin.is_user() {
                    return Err(MadoError::new(ErrorCode::Protected, "error.ai.notAllowed"));
                }
                let v = self.task_get(task_id)?;
                let path = std::path::PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.file.notAbsolute",
                    ));
                }
                let text = serde_json::to_string_pretty(&v).unwrap_or_default();
                if let Some(reply) = reply_to.take() {
                    tokio::task::spawn_blocking(move || {
                        let _ =
                            reply.send(std::fs::write(&path, text).map(|()| Value::Null).map_err(
                                |e| {
                                    MadoError::new(ErrorCode::Internal, "error.file.write")
                                        .arg("reason", e.to_string())
                                },
                            ));
                    });
                }
                Ok(Value::Null)
            }
            _ => Err(MadoError::new(
                ErrorCode::Unsupported,
                "error.unknownCommand",
            )),
        }
    }

    fn task_get(&self, task: TaskId) -> Result<Value, MadoError> {
        let id = task.to_string();
        let row = self
            .profile_db
            .readers()
            .read(move |c| {
                use rusqlite::OptionalExtension as _;
                let task = c
                    .query_row(
                        "SELECT prompt, status, mode, runtime, started_at, ended_at, answer, cost_micros FROM ai_tasks WHERE id = ?1",
                        [&id],
                        |r| {
                            Ok(json!({
                                "id": id, "prompt": r.get::<_, String>(0)?, "status": r.get::<_, String>(1)?, "mode": r.get::<_, String>(2)?,
                                "runtime": r.get::<_, String>(3)?, "startedAt": r.get::<_, i64>(4)?, "endedAt": r.get::<_, Option<i64>>(5)?,
                                "answer": r.get::<_, Option<String>>(6)?, "costMicros": r.get::<_, i64>(7)?,
                            }))
                        },
                    )
                    .optional()?;
                let audit: Vec<Value> = c
                    .prepare("SELECT at, kind, detail FROM ai_audit WHERE task_id = ?1 ORDER BY id")?
                    .query_map([&id], |r| {
                        let detail: Value = serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null);
                        Ok(json!({ "at": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?, "detail": detail }))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                Ok(task.map(|mut t| {
                    t["audit"] = Value::Array(audit);
                    t
                }))
            })
            .map_err(|_| MadoError::internal())?
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.ai.noTask"))?;
        let mut row = row;
        if let Some(c) = self.task_ctl.get(&task) {
            row["steps"] = json!(c.steps);
        }
        Ok(row)
    }

    /// Starts queued tasks while fewer than three run (13 §7.1).
    pub(super) fn start_queued(&mut self, rec: &mut PatchRecorder) {
        while self.running_tasks() < RUNNING_MAX {
            let Some(task) = self.task_queue.pop_front() else {
                return;
            };
            let Some(c) = self.task_ctl.get_mut(&task) else {
                continue;
            };
            let Some(start) = c.start.take() else {
                continue;
            };
            let attempts = match start.model.as_deref() {
                Some(m) => self.attempts_for(m),
                None => self.ai_attempts("tasks"),
            };
            let effort = mado_agent::chat::effort(start.effort.as_deref().unwrap_or_else(|| {
                self.state
                    .ai
                    .roles
                    .get("tasks")
                    .map_or("auto", |r| r.effort.as_str())
            }));
            let Some(c) = self.task_ctl.get(&task) else {
                continue;
            };
            let scope: Vec<String> = self
                .guards
                .get(&task)
                .and_then(|g| g.scope.clone())
                .map(|s| s.into_iter().collect())
                .unwrap_or_default();
            let minutes = self
                .setting("ai.taskTimeLimit")
                .as_i64()
                .unwrap_or(1_800_000);
            let env = Env {
                task,
                inputs: self.inputs.clone(),
                attempts,
                secrets: Arc::clone(&self.ai.secrets),
                effort,
                system: mado_agent::chat::task_system(&c.prompt, &scope),
                prompt: c.prompt.clone(),
                tools: offered(),
                ctx: self.tool_ctx(Origin::Agent { task }),
                steer: Arc::clone(&c.steer),
                queue: Arc::clone(&c.queue),
                waiting: Arc::clone(&c.waiting),
                stop: Arc::clone(&c.stop),
                deadline: Instant::now()
                    + Duration::from_millis(u64::try_from(minutes).unwrap_or(1_800_000)),
                depth: 0,
            };
            if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
                t.set_status(TaskStatus::Running);
            }
            let inputs = self.inputs.clone();
            tokio::task::spawn_local(async move {
                let (status, answer, sources) = run(env).await;
                let _ = inputs.send(Input::Internal(Internal::TaskEvent {
                    task,
                    event: TaskEvent::Done {
                        status,
                        answer,
                        sources,
                    },
                }));
            });
        }
    }

    pub(super) fn task_event(&mut self, task: TaskId, event: TaskEvent, rec: &mut PatchRecorder) {
        let note = match &event {
            TaskEvent::Step(text) => {
                if let Some(c) = self.task_ctl.get_mut(&task) {
                    c.steps.push(text.clone());
                }
                if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
                    t.set_step(text.clone());
                }
                json!({ "jsonrpc": "2.0", "method": "task.step", "params": { "taskId": task, "text": text } })
            }
            TaskEvent::Waiting(reason) => {
                if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
                    t.set_status(TaskStatus::Waiting);
                    t.set_step(reason.clone());
                }
                self.audit(task, "handoff", json!({ "reason": reason }));
                json!({ "jsonrpc": "2.0", "method": "task.waiting", "params": { "taskId": task, "reason": reason } })
            }
            TaskEvent::Resumed => {
                if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
                    t.set_status(TaskStatus::Running);
                }
                return;
            }
            TaskEvent::Done {
                status,
                answer,
                sources,
            } => {
                let n = json!({ "jsonrpc": "2.0", "method": "task.done", "params": { "taskId": task, "status": status, "answer": answer, "sources": sources } });
                self.notify_followers(task, &n);
                self.task_finished(task, *status, answer.clone(), sources.clone(), rec);
                return;
            }
        };
        self.notify_followers(task, &note);
    }

    fn notify_followers(&mut self, task: TaskId, note: &Value) {
        let Some(c) = self.task_ctl.get_mut(&task) else {
            return;
        };
        let before = c.followers.len();
        c.followers.retain(|f| f.send(note.clone()).is_ok());
        // A follower that left without detaching (Ctrl+C) stops the task.
        if c.followers.len() < before && c.followers.is_empty() && !c.detached {
            c.stop.store(true, Ordering::SeqCst);
        }
    }

    fn task_finished(
        &mut self,
        task: TaskId,
        status: TaskStatus,
        answer: String,
        sources: Vec<String>,
        rec: &mut PatchRecorder,
    ) {
        self.task_ctl.remove(&task);
        if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
            t.set_status(status);
            t.set_step(String::new());
        }
        let key = serde_json::to_value(status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        self.audit(task, "done", json!({ "status": key, "sources": sources }));
        if !self.state.profile.is_private {
            let (t, at, a) = (task.to_string(), self.now(), answer);
            let pending = self.profile_db.writer().write(move |c| {
                c.execute(
                    "UPDATE ai_tasks SET status = ?2, ended_at = ?3, answer = ?4 WHERE id = ?1",
                    params![t, key, at, a],
                )
                .map(drop)
            });
            tokio::task::spawn_local(async move {
                if let Err(e) = pending.await {
                    tracing::error!(error = %e, "a task's end was not written");
                }
            });
        }
        self.timers
            .schedule(crate::Timer::AgentTabsClose(task), TABS_CLOSE_AFTER);
        self.start_queued(rec);
    }

    /// 13 §7.3: the task's agent tabs close, unless the person is looking
    /// at one.
    pub(super) fn close_agent_tabs(&mut self, task: TaskId) {
        let tabs: Vec<mado_types::TabId> = self
            .model
            .tabs
            .values()
            .filter(|t| t.agent.as_ref().is_some_and(|a| a.task_id == task))
            .map(|t| t.id)
            .collect();
        let shown = tabs.iter().any(|id| {
            self.model.tabs.get(id).is_some_and(|t| {
                self.model
                    .windows
                    .get(&t.window_id)
                    .is_some_and(|w| w.active_tab_id == Some(*id))
            })
        });
        if !shown && !tabs.is_empty() {
            self.close_tabs(&tabs, false);
        }
    }

    /// `task.events` (14 §2): a follower of a task's steps.
    pub(super) fn follow_task(
        &mut self,
        task: TaskId,
    ) -> Result<mpsc::UnboundedReceiver<Value>, MadoError> {
        let c = self
            .task_ctl
            .get_mut(&task)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.ai.noTask"))?;
        let (tx, rx) = mpsc::unbounded_channel();
        c.followers.push(tx);
        c.detached = false;
        Ok(rx)
    }

    /// `cli` tasks (ACP, M5-T13).
    fn start_cli_task(
        &mut self,
        _origin: &Origin,
        _prompt: String,
        _cli: Option<String>,
        _mode: Option<String>,
        _rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        Err(MadoError::new(ErrorCode::Unsupported, "error.ai.notYet"))
    }
}

/// Queued tasks in start order.
pub(super) type Queue = VecDeque<TaskId>;
