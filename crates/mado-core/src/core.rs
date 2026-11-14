//! The core actor (spec 01 §2): the state document and every service, one
//! input at a time, on a dedicated thread with a `current_thread` runtime.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::time::Duration;

use mado_engine::{WindowCommand, WindowEngine, WindowEvent};
use mado_settings::types::Scope;
use mado_settings::{Change, Outcome, Settings, Write, lookup, persist};
use mado_storage::{BackupReason, Kind, Storage, backup, prune_backups};
use mado_types::command::{Command, Origin};
use mado_types::state::{
    Bounds, Notice, NoticeAction, ProfileInfo, State, StatePatch, WindowState,
};
use mado_types::{ErrorCode, MadoError, Patch, PatchRecorder, StateSnapshot, WindowId};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::timers::{Timer, Timers};
use crate::tokens::Tokens;
use crate::ui::UiSink;

pub type Reply = oneshot::Sender<Result<Value, MadoError>>;

/// Everything the core reacts to.
#[derive(Debug)]
pub enum Input {
    Command {
        origin: Origin,
        cmd: Command,
        reply: Reply,
    },
    Window(WindowEvent),
    Internal(Internal),
}

/// Results of work the core spawned, and requests from the app itself.
#[derive(Debug)]
pub enum Internal {
    /// Open a normal window (startup, or the Dock icon with no window).
    OpenWindow { bounds: Bounds },
    /// A settings file read off the core thread for `settings.import`.
    ImportRead {
        origin: Origin,
        result: Result<Value, String>,
        reply: Reply,
    },
    /// `app.authenticate` succeeded; the token is good for 60 seconds.
    TokenIssued { token: String },
    /// Stops the loop after the queued writes (spec 01 §1.6).
    Shutdown { done: oneshot::Sender<()> },
    /// A command handler that panics, to prove the core survives one.
    #[cfg(any(test, feature = "test-hooks"))]
    Panic,
}

/// How long a notice with Undo stays (spec 06 §3).
const UNDO_NOTICE: Duration = Duration::from_secs(10);
const DAY: Duration = Duration::from_secs(24 * 3600);

/// What the core needs from the app.
pub struct CoreConfig {
    pub profile: ProfileInfo,
    pub app_db: Storage,
    pub profile_db: Storage,
    /// `<profile>/Backups/`.
    pub backups: PathBuf,
    pub settings: Settings,
    pub engine: Box<dyn WindowEngine>,
    pub ui: Box<dyn UiSink>,
    /// The time source, Unix milliseconds. Tests pass a fixed clock.
    pub clock: fn() -> i64,
}

impl std::fmt::Debug for CoreConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreConfig")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

pub fn system_clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A handle to send inputs to a running core.
#[derive(Debug, Clone)]
pub struct CoreHandle {
    tx: mpsc::UnboundedSender<Input>,
}

impl CoreHandle {
    pub fn send(&self, input: Input) -> bool {
        self.tx.send(input).is_ok()
    }

    /// Sends a command; the receiver gets its reply.
    pub fn command(
        &self,
        origin: Origin,
        cmd: Command,
    ) -> oneshot::Receiver<Result<Value, MadoError>> {
        let (reply, rx) = oneshot::channel();
        if let Err(mpsc::error::SendError(Input::Command { reply, .. })) =
            self.tx.send(Input::Command { origin, cmd, reply })
        {
            let _ = reply.send(Err(MadoError::new(
                ErrorCode::Internal,
                "error.coreStopped",
            )));
        }
        rx
    }
}

pub struct Core {
    state: State,
    rev: u64,
    settings: Settings,
    tokens: Tokens,
    timers: Timers,
    app_db: Storage,
    profile_db: Storage,
    backups: PathBuf,
    engine: Box<dyn WindowEngine>,
    ui: Box<dyn UiSink>,
    clock: fn() -> i64,
    /// Engine commands a handler queued; sent after its patch (01 §2.1 step 6).
    outbox: Vec<(WindowId, WindowCommand)>,
    next_notice: u64,
    inputs: mpsc::UnboundedSender<Input>,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core")
            .field("rev", &self.rev)
            .finish_non_exhaustive()
    }
}

/// Starts the core on its own thread. Returns its handle and the thread,
/// which ends after `Internal::Shutdown` or when every handle is dropped.
pub fn start(config: CoreConfig) -> std::io::Result<(CoreHandle, std::thread::JoinHandle<()>)> {
    let (tx, rx) = mpsc::unbounded_channel();
    let handle = CoreHandle { tx: tx.clone() };
    let thread = std::thread::Builder::new()
        .name("mado-core".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(error = %e, "the core runtime did not start");
                    return;
                }
            };
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, async move {
                let core = Core::new(config, tx);
                core.run(rx).await;
            });
        })?;
    Ok((handle, thread))
}

impl Core {
    fn new(config: CoreConfig, inputs: mpsc::UnboundedSender<Input>) -> Self {
        let mut core = Self {
            state: State {
                profile: config.profile,
                ..State::default()
            },
            rev: 0,
            settings: config.settings,
            tokens: Tokens::default(),
            timers: Timers::default(),
            app_db: config.app_db,
            profile_db: config.profile_db,
            backups: config.backups,
            engine: config.engine,
            ui: config.ui,
            clock: config.clock,
            outbox: Vec::new(),
            next_notice: 1,
            inputs,
        };
        core.mirror_all_settings();
        core.timers.schedule(Timer::DailyBackup, DAY);
        core
    }

    /// Fills `state.settings`, `state.features` and the mirrors of
    /// feature switches before the first revision.
    fn mirror_all_settings(&mut self) {
        let mut rec = PatchRecorder::new();
        let changed: Vec<Change> = mado_settings::REGISTRY
            .iter()
            .map(|d| {
                let value = self.settings.value(d);
                Change {
                    key: d.key,
                    old: Value::Null,
                    new: value,
                }
            })
            .collect();
        let mut tx = self.state.tx(&mut rec);
        mirror(&mut tx, &self.settings, &changed);
        // The initial document is revision 0; nothing to emit.
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Input>) {
        loop {
            tokio::select! {
                input = rx.recv() => match input {
                    Some(Input::Internal(Internal::Shutdown { done })) => {
                        self.shutdown().await;
                        let _ = done.send(());
                        return;
                    }
                    Some(input) => self.step(input),
                    None => return,
                },
                timer = self.timers.next() => self.fire(timer),
            }
        }
    }

    /// One input, end to end (spec 01 §2.1): handle, patch, engine, reply.
    fn step(&mut self, input: Input) {
        let state_before = self.state.clone();
        let settings_before = self.settings.clone();
        let mut rec = PatchRecorder::new();
        let mut reply = None;
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| match input {
            Input::Command {
                origin,
                cmd,
                reply: r,
            } => {
                reply = Some(r);
                self.command(&origin, cmd, &mut rec, &mut reply)
            }
            Input::Window(event) => {
                self.window_event(event, &mut rec);
                Ok(Value::Null)
            }
            Input::Internal(internal) => {
                self.internal(internal, &mut rec);
                Ok(Value::Null)
            }
        }));
        let result = result.unwrap_or_else(|panic| {
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            tracing::error!(panic = %what, "a core handler panicked; the state is unchanged");
            self.state = state_before;
            self.settings = settings_before;
            self.outbox.clear();
            rec.take();
            Err(MadoError::internal())
        });
        self.emit(&mut rec);
        for (window, cmd) in std::mem::take(&mut self.outbox) {
            self.engine.apply_window(window, cmd);
        }
        // A handler that answers later took the reply.
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
    }

    fn emit(&mut self, rec: &mut PatchRecorder) {
        if rec.is_empty() {
            return;
        }
        self.rev += 1;
        self.state.rev = self.rev;
        let patch = Patch {
            rev: self.rev,
            ops: rec.take(),
        };
        self.ui.patch(&patch);
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let now = self.now();
        match cmd {
            Command::StateGet => serde_json::to_value(StateSnapshot {
                rev: self.rev,
                state: self.state.clone(),
            })
            .map_err(|_| MadoError::internal()),
            // Accepted only from the control endpoint with the launch secret,
            // which checks it before the command reaches the core (spec 01 §1.3).
            Command::LaunchForward(_) => match origin {
                Origin::System => Ok(Value::Null),
                _ => Err(MadoError::new(
                    ErrorCode::PermissionDenied,
                    "error.launch.notAllowed",
                )),
            },
            Command::SettingsGet { keys } => Ok(self.settings.get_many(keys.as_deref())?),
            Command::SettingsSearch { query } => Ok(json!(self.settings.search(&query))),
            Command::SettingsSet { key, value, token } => {
                let out = self.settings.set(
                    &key,
                    value,
                    origin,
                    token.as_deref(),
                    self.tokens.at(now),
                    now,
                )?;
                self.apply_outcome(out, rec);
                Ok(Value::Null)
            }
            Command::SettingsReset(target) => {
                let out = self.settings.reset(&target, origin, now)?;
                self.apply_outcome(out, rec);
                Ok(Value::Null)
            }
            Command::SettingsUndo { log_id } => {
                let out = self.settings.undo(log_id, origin, now)?;
                self.apply_outcome(out, rec);
                Ok(Value::Null)
            }
            Command::SettingsExport { path } => {
                let file = self.settings.export();
                if let Some(reply) = reply_to.take() {
                    tokio::task::spawn_blocking(move || {
                        let text = serde_json::to_string_pretty(&file).unwrap_or_default();
                        let result = std::fs::write(&path, text)
                            .map(|()| Value::Null)
                            .map_err(|e| file_error(&e.to_string()));
                        let _ = reply.send(result);
                    });
                }
                Ok(Value::Null)
            }
            Command::SettingsImport { path } => {
                // Read off the core thread; the import itself runs back here.
                if let Some(reply) = reply_to.take() {
                    let inputs = self.inputs.clone();
                    let origin = *origin;
                    tokio::task::spawn_blocking(move || {
                        let result = std::fs::read_to_string(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()));
                        let _ = inputs.send(Input::Internal(Internal::ImportRead {
                            origin,
                            result,
                            reply,
                        }));
                    });
                }
                Ok(Value::Null)
            }
        }
    }

    fn internal(&mut self, internal: Internal, rec: &mut PatchRecorder) {
        match internal {
            Internal::OpenWindow { bounds } => {
                let id = WindowId::new();
                let layout = if self.settings.value(lookup_def("appearance.layout")) == "horizontal"
                {
                    mado_types::state::TabLayout::Horizontal
                } else {
                    mado_types::state::TabLayout::Vertical
                };
                let window = WindowState {
                    id,
                    bounds: bounds.clone(),
                    layout,
                    is_private: self.state.profile.is_private,
                    ..WindowState::default()
                };
                self.state.tx(rec).windows().insert(id, window);
                self.outbox.push((
                    id,
                    WindowCommand::Create {
                        window: id,
                        bounds,
                        kind: mado_types::state::WindowKind::Normal,
                        private: self.state.profile.is_private,
                    },
                ));
            }
            Internal::ImportRead {
                origin,
                result,
                reply,
            } => {
                let now = self.now();
                let outcome = result
                    .map_err(|reason| file_error(&reason))
                    .and_then(|file| {
                        self.settings
                            .import(&file, &origin, now)
                            .map_err(MadoError::from)
                    });
                let answer = outcome.map(|(out, report)| {
                    self.apply_outcome(out, rec);
                    json!(report)
                });
                let _ = reply.send(answer);
            }
            Internal::TokenIssued { token } => {
                let now = self.now();
                self.tokens.issue(token, now);
            }
            Internal::Shutdown { .. } => {}
            #[cfg(any(test, feature = "test-hooks"))]
            Internal::Panic => {
                self.state.tx(rec).profile().set_name("half-done".into());
                panic!("test hook: a handler panicked");
            }
        }
    }

    fn window_event(&mut self, event: WindowEvent, rec: &mut PatchRecorder) {
        match event {
            WindowEvent::Created { .. } => {}
            WindowEvent::CloseRequested { window } => {
                if self.state.tx(rec).windows().remove(&window).is_some() {
                    self.outbox.push((window, WindowCommand::Close));
                }
            }
        }
    }

    /// Mirrors a settings change into the document, persists it, runs its
    /// side effects and offers Undo where the setting asks for it.
    fn apply_outcome(&mut self, out: Outcome, rec: &mut PatchRecorder) {
        let now = self.now();
        {
            let mut tx = self.state.tx(rec);
            mirror(&mut tx, &self.settings, &out.changes);
            side_effects(&mut tx, &out.changes);
        }
        if let Some(log_id) = out.undo_notice
            && let Some(change) = out.changes.first()
        {
            let id = format!("n{}", self.next_notice);
            self.next_notice += 1;
            let notice = Notice {
                id: id.clone(),
                message_key: "notice.settingChanged".into(),
                args: BTreeMap::from([("label".to_owned(), json!(lookup_def(change.key).label))]),
                action: Some(NoticeAction {
                    label_key: "notice.undo".into(),
                    command: Command::SettingsUndo {
                        log_id: Some(log_id),
                    },
                }),
                undo: Some(Command::SettingsUndo {
                    log_id: Some(log_id),
                }),
                expires_at: Some(now + i64::try_from(UNDO_NOTICE.as_millis()).unwrap_or(10_000)),
            };
            let mut notices = self.state.notices.clone();
            notices.push(notice.clone());
            self.state.tx(rec).set_notices(notices);
            self.ui.notice(&notice);
            self.timers.schedule(Timer::NoticeExpiry(id), UNDO_NOTICE);
        }
        self.persist(out.writes);
    }

    /// Queues the rows on each database's writer. Writes keep their order;
    /// a failure is logged (the value in memory stays, and is written again
    /// by the next change of that key).
    fn persist(&self, writes: Vec<Write>) {
        if writes.is_empty() {
            return;
        }
        let app_writes = writes.clone();
        let app = self
            .app_db
            .writer()
            .write(move |c| persist(&app_writes, Scope::App, c));
        let profile = self
            .profile_db
            .writer()
            .write(move |c| persist(&writes, Scope::Profile, c));
        tokio::task::spawn_local(async move {
            for (db, result) in [("app", app.await), ("profile", profile.await)] {
                if let Err(e) = result {
                    tracing::error!(database = db, error = %e, "a settings write failed");
                }
            }
        });
    }

    fn fire(&mut self, timer: Timer) {
        let mut rec = PatchRecorder::new();
        match timer {
            Timer::NoticeExpiry(id) => {
                if self.state.notices.iter().any(|n| n.id == id) {
                    let notices = self
                        .state
                        .notices
                        .iter()
                        .filter(|n| n.id != id)
                        .cloned()
                        .collect();
                    self.state.tx(&mut rec).set_notices(notices);
                }
            }
            Timer::DailyBackup => {
                let jobs = [(self.profile_db.path().to_owned(), Kind::Profile)];
                let dir = self.backups.clone();
                tokio::task::spawn_blocking(move || {
                    for (path, kind) in jobs {
                        if let Err(e) = backup(&path, kind, &dir, BackupReason::Daily)
                            .and_then(|_| prune_backups(kind, &dir))
                        {
                            tracing::error!(error = %e, "the daily backup failed");
                        }
                    }
                });
                self.timers.schedule(Timer::DailyBackup, DAY);
            }
        }
        self.emit(&mut rec);
    }

    /// Lets queued writes finish; the app then checkpoints and releases the
    /// lock (spec 01 §1.6).
    async fn shutdown(&mut self) {
        let _ = self.app_db.writer().write(|_| Ok(())).await;
        let _ = self
            .profile_db
            .writer()
            .write(|c| c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);"))
            .await;
    }
}

fn lookup_def(key: &str) -> &'static mado_settings::SettingDef {
    lookup(key).expect("the key comes from the registry")
}

/// `state.settings` (ui keys), `state.features`, and the fields that mirror
/// a setting directly.
fn mirror(tx: &mut StatePatch<'_>, settings: &Settings, changes: &[Change]) {
    for change in changes {
        let def = lookup_def(change.key);
        if def.opts.ui {
            tx.settings()
                .values()
                .insert(change.key.to_owned(), change.new.clone());
        }
        if let Some(feature) = change.key.strip_prefix("features.") {
            tx.features().insert(feature.to_owned(), change.new == true);
        }
        match change.key {
            "features.ai" => {
                tx.ai().set_enabled(change.new == true);
            }
            "blocker.enabled" => {
                tx.blocker().set_enabled(change.new == true);
            }
            _ => {}
        }
    }
    let changed: Vec<String> = settings.changed().into_iter().map(str::to_owned).collect();
    tx.settings().set_changed(changed);
}

/// Side effects of a change (spec 06 §4) that exist in M1. Engine-side
/// effects (blocker rules, proxy, HTTPS-first) arrive with their engines.
fn side_effects(tx: &mut StatePatch<'_>, changes: &[Change]) {
    for change in changes {
        match change.key {
            "appearance.layout" => {
                let layout = if change.new == "horizontal" {
                    mado_types::state::TabLayout::Horizontal
                } else {
                    mado_types::state::TabLayout::Vertical
                };
                let ids: Vec<WindowId> = tx.get().windows.keys().copied().collect();
                for id in ids {
                    if let Some(mut w) = tx.windows().get(&id) {
                        w.set_layout(layout);
                    }
                }
            }
            // Turning AI or agents off stops their tasks (spec 06 §6).
            "features.ai" | "features.agents" if change.new == false => {
                let ids: Vec<mado_types::TaskId> = tx.get().ai.tasks.keys().copied().collect();
                for id in ids {
                    if let Some(mut task) = tx.ai().tasks().get(&id) {
                        task.set_status(mado_types::state::TaskStatus::Cancelled);
                    }
                }
            }
            _ => {}
        }
    }
}

fn file_error(reason: &str) -> MadoError {
    MadoError::new(ErrorCode::InvalidInput, "error.settings.file").arg("reason", reason)
}
