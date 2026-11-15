//! The core actor (spec 01 §2): the state document and every service, one
//! input at a time, on a dedicated thread with a `current_thread` runtime.

mod dl;
mod marks;
mod pages;
mod session;
mod visits;

use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::time::Duration;

use mado_engine::tab::SessionBlob;
use mado_engine::{EngineEvent, EngineHandle, EngineOp, UserAgent, WindowCommand};
use mado_settings::types::Scope;
use mado_settings::{Change, Outcome, Settings, Write, lookup, persist};
use mado_storage::{BackupReason, Kind, Storage, backup, prune_backups};
use mado_types::command::{Command, Origin};
use mado_types::state::{Bounds, Notice, NoticeAction, ProfileInfo, State, StatePatch};
use mado_types::{
    ErrorCode, GroupId, MadoError, Patch, PatchRecorder, PromptId, StateSnapshot, TabId, WindowId,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::navigation::{self, HttpsFirst, PolicyContext};
use crate::store::{self, Loaded};
use crate::tabs::{ClosedEntry, Model};
use crate::timers::{Timer, Timers};
use crate::tokens::Tokens;
use crate::ui::{UiSink, Updates};
use session::CloseAction;
pub use session::Power;

pub type Reply = oneshot::Sender<Result<Value, MadoError>>;

/// Everything the core reacts to.
#[derive(Debug)]
pub enum Input {
    Command {
        origin: Origin,
        cmd: Command,
        reply: Reply,
    },
    /// An event from the engine, on the main thread (spec 02 §1).
    Engine(EngineEvent),
    Internal(Internal),
}

/// Results of work the core spawned, and requests from the app itself.
#[derive(Debug)]
pub enum Internal {
    /// The app is up: restore or open windows per `startup.mode` (spec 04 §8).
    Startup { bounds: Bounds },
    /// Open a normal window (the Dock icon with no window).
    OpenWindow { bounds: Bounds },
    /// ⌘Q (`shortcut`), the Dock's Quit or the system asked Mado to quit.
    QuitRequested { shortcut: bool },
    /// SIGTERM: quit without questions, after pausing downloads.
    Terminate,
    /// macOS reported memory pressure (spec 04 §7).
    MemoryPressure { critical: bool },
    /// The power source changed (energy saver, spec 04 §7).
    Power(Power),
    /// A tab's history, saved after a navigation.
    SessionSaved { tab: TabId, blob: SessionBlob },
    /// The sleep procedure saved what it needs; the view may go.
    SleepReady {
        tab: TabId,
        blob: Option<SessionBlob>,
    },
    /// The hover-card picture of a tab going to sleep.
    SnapshotTaken { tab: TabId, jpeg: Vec<u8> },
    /// History recorded the visit of a tab's page.
    VisitRecorded { tab: TabId, visit: i64 },
    /// A bookmark change was written.
    BookmarksChanged,
    /// The safety check of a completed download (spec 08 §4).
    DownloadVerdict {
        id: mado_types::DownloadId,
        danger: Option<String>,
    },
    /// Bookmarks were deleted; they wait for Undo.
    BookmarksDeleted(Vec<crate::bookmarks::Node>),
    /// Opens links as tabs (a bookmark folder).
    OpenUrls {
        urls: Vec<String>,
        disposition: mado_types::command::OpenDisposition,
        window: Option<WindowId>,
    },
    /// A settings file read off the core thread for `settings.import`.
    ImportRead {
        origin: Origin,
        result: Result<Value, String>,
        reply: Reply,
    },
    /// `app.authenticate` succeeded; the token is good for 60 seconds.
    TokenIssued { token: String },
    /// The answer to `app.authenticate`, from off the core thread.
    Authenticated {
        result: Result<String, String>,
        reply: Reply,
    },
    /// What the updater is doing (spec 15 §5).
    UpdateStatus(mado_types::state::UpdateState),
    /// Stops the loop after the queued writes (spec 01 §1.6).
    Shutdown { done: oneshot::Sender<()> },
    /// Marks a tab as holding typed text, as the `forms` page script will
    /// (M2-T04).
    #[cfg(test)]
    TestUnsavedInput { tab: TabId },
    /// A command handler that panics, to prove the core survives one.
    #[cfg(any(test, feature = "test-hooks"))]
    Panic,
    /// Test-only hooks (spec 16 §3). Compiled only with `test-hooks`.
    #[cfg(any(test, feature = "test-hooks"))]
    Hook(TestHook),
}

/// Spec 16 §3. Fake memory pressure and the fixture proxy configuration act
/// on tabs and the engine, and arrive with them in M2.
#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestHook {
    /// The next patch is counted but not sent, so a UI sees a gap.
    DropNextPatch,
    /// Every later `now` returns this, in Unix milliseconds.
    FakeClock(i64),
    /// `app.authenticate` succeeds without asking the OS.
    FakeAuthSuccess,
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
    pub engine: EngineHandle,
    /// The user agent tab views send (spec 02 §6).
    pub user_agent: UserAgent,
    /// The last run of this profile quit cleanly (spec 04 §8).
    pub clean_start: bool,
    /// The page scripts every tab gets (spec 02 §5).
    pub page_scripts: mado_engine::PageScriptSet,
    pub ui: Box<dyn UiSink>,
    pub updates: Box<dyn Updates>,
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
    engine: EngineHandle,
    user_agent: UserAgent,
    clean_start: bool,
    /// Windows, tabs and groups (spec 04); the document mirrors it.
    model: Model,
    /// What the database holds, to write only what changed.
    persisted: Model,
    /// What the last run left, until `Internal::Startup` uses it.
    loaded: Loaded,
    /// The closed stack, oldest first, with database IDs.
    closed: Vec<(i64, ClosedEntry)>,
    /// Groups deleted in the last 10 seconds, for Undo.
    deleted_groups: BTreeMap<GroupId, ClosedEntry>,
    close_prompts: BTreeMap<PromptId, CloseAction>,
    quit_pressed_at: Option<i64>,
    power: Power,
    /// Tabs whose view the engine made itself (popups).
    adopted: BTreeSet<TabId>,
    /// Tabs going to sleep, waiting for their session.
    sleeping: BTreeSet<TabId>,
    /// When each tab's page last sent a heartbeat (spec 02 §5.3).
    heartbeats: BTreeMap<TabId, i64>,
    /// Tabs showing "Page unresponsive", with their prompt.
    unresponsive: BTreeMap<TabId, PromptId>,
    /// Downloads beyond the document (spec 12 §4).
    dls: BTreeMap<mado_types::DownloadId, dl::Dl>,
    dl_by_engine: BTreeMap<mado_engine::event::EngineDownloadId, mado_types::DownloadId>,
    /// Downloads each page started, for `automaticDownloads`.
    page_downloads: BTreeMap<TabId, u32>,
    next_resume: u64,
    /// Quitting waits for downloads to pause.
    quit_after_downloads: bool,
    /// Deleted bookmarks waiting for Undo, by token.
    deleted_bookmarks: BTreeMap<u64, Vec<crate::bookmarks::Node>>,
    /// Each tab's current navigation, for history (spec 12 §2.1).
    navs: visits::Navs,
    /// Transitions the core knows before the engine reports the visit
    /// (links from other apps, session restore, bookmarks, typed).
    pending_transition: BTreeMap<TabId, crate::history::Transition>,
    /// The element under the pointer at the last context menu (M2-T07).
    context_targets: BTreeMap<TabId, mado_engine::page::ContextTarget>,
    /// Tabs whose protected video failed (M2-T06 offers Safari).
    drm_failed: BTreeSet<TabId>,
    ui: Box<dyn UiSink>,
    updates: Box<dyn Updates>,
    clock: fn() -> i64,
    /// Engine commands a handler queued; sent after its patch (01 §2.1 step 6).
    outbox: Vec<EngineOp>,
    next_notice: u64,
    inputs: mpsc::UnboundedSender<Input>,
    #[cfg(any(test, feature = "test-hooks"))]
    hooks: Hooks,
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Default)]
struct Hooks {
    drop_next_patch: bool,
    clock: Option<i64>,
    auth_success: bool,
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
        let mut loaded = if config.profile.is_private {
            Loaded::default()
        } else {
            config
                .profile_db
                .readers()
                .read(store::load)
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "the last session could not be read");
                    Loaded::default()
                })
        };
        let closed = std::mem::take(&mut loaded.closed);
        let downloads = if config.profile.is_private {
            Vec::new()
        } else {
            config
                .profile_db
                .readers()
                .read(dl::load)
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "the download list could not be read");
                    Vec::new()
                })
        };
        let page_scripts = config.page_scripts;
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
            user_agent: config.user_agent,
            clean_start: config.clean_start,
            model: Model::default(),
            persisted: Model::default(),
            loaded,
            closed,
            deleted_groups: BTreeMap::new(),
            close_prompts: BTreeMap::new(),
            quit_pressed_at: None,
            power: Power::default(),
            adopted: BTreeSet::new(),
            sleeping: BTreeSet::new(),
            heartbeats: BTreeMap::new(),
            unresponsive: BTreeMap::new(),
            navs: BTreeMap::new(),
            deleted_bookmarks: BTreeMap::new(),
            dls: BTreeMap::new(),
            dl_by_engine: BTreeMap::new(),
            page_downloads: BTreeMap::new(),
            next_resume: 0,
            quit_after_downloads: false,
            pending_transition: BTreeMap::new(),
            context_targets: BTreeMap::new(),
            drm_failed: BTreeSet::new(),
            ui: config.ui,
            updates: config.updates,
            clock: config.clock,
            outbox: Vec::new(),
            next_notice: 1,
            inputs,
            #[cfg(any(test, feature = "test-hooks"))]
            hooks: Hooks::default(),
        };
        core.mirror_all_settings();
        for (state, info) in downloads {
            core.dls.insert(state.id, info);
            core.state.downloads.insert(state.id, state);
        }
        // The proxy and page scripts apply to every tab view, so before the
        // first one.
        core.engine.send(EngineOp::SetProxy(core.proxy()));
        core.engine.send(EngineOp::SetPageScripts(page_scripts));
        core.timers
            .schedule(Timer::HeartbeatCheck, pages::HEARTBEAT_CHECK);
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
        let model_before = self.model.clone();
        let mut rec = PatchRecorder::new();
        let mut reply = None;
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let result = match input {
                Input::Command {
                    origin,
                    cmd,
                    reply: r,
                } => {
                    reply = Some(r);
                    self.command(&origin, cmd, &mut rec, &mut reply)
                }
                Input::Engine(event) => {
                    self.engine_event(event, &mut rec);
                    Ok(Value::Null)
                }
                Input::Internal(internal) => {
                    self.internal(internal, &mut rec);
                    Ok(Value::Null)
                }
            };
            #[cfg(debug_assertions)]
            if let Err(broken) = self.model.check() {
                panic!("tab model invariant broken: {broken}");
            }
            result
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
            self.model = model_before.clone();
            self.outbox.clear();
            rec.take();
            Err(MadoError::internal())
        });
        self.settle(&model_before, &mut rec);
        // A handler that answers later took the reply.
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
    }

    /// After a handler or timer: the document follows the model, one patch
    /// goes out, then the engine calls (spec 01 §2.1 steps 5 and 6), then
    /// the database write.
    fn settle(&mut self, model_before: &Model, rec: &mut PatchRecorder) {
        let (first, last) = self.reconcile(model_before, rec);
        self.emit(rec);
        let queued = std::mem::take(&mut self.outbox);
        for op in first.into_iter().chain(queued).chain(last) {
            self.engine.send(op);
        }
        self.persist_model();
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
        #[cfg(any(test, feature = "test-hooks"))]
        if std::mem::take(&mut self.hooks.drop_next_patch) {
            return;
        }
        self.ui.patch(&patch);
    }

    fn now(&self) -> i64 {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(now) = self.hooks.clock {
            return now;
        }
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
            Command::LaunchForward(request) => match origin {
                Origin::System => {
                    // Links from other apps (spec 05 §9): tabs in the current
                    // window, or a new window when asked or none is open.
                    let current = self.current_window();
                    let external = self
                        .settings
                        .value(lookup_def("general.externalLinks"))
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    match current {
                        Some(window) if !request.new_window && external != "newWindow" => {
                            let prefs = self.prefs();
                            for url in &request.urls {
                                let tab = self.model.open_tab(
                                    window,
                                    url.clone(),
                                    None,
                                    true,
                                    None,
                                    now,
                                    prefs,
                                )?;
                                self.pending_transition
                                    .insert(tab, crate::history::Transition::External);
                            }
                            self.outbox
                                .push(EngineOp::ApplyWindow(window, WindowCommand::Focus));
                        }
                        _ => {
                            let id = self.open_window(Bounds::default(), &request.urls)?;
                            if !request.urls.is_empty() {
                                let tabs = self.model.window(id)?.tab_order.clone();
                                for tab in tabs {
                                    self.pending_transition
                                        .insert(tab, crate::history::Transition::External);
                                }
                            }
                            self.outbox
                                .push(EngineOp::ApplyWindow(id, WindowCommand::Focus));
                        }
                    }
                    Ok(Value::Null)
                }
                _ => Err(MadoError::new(
                    ErrorCode::PermissionDenied,
                    "error.launch.notAllowed",
                )),
            },
            Command::UpdateCheck => {
                self.updates.check();
                Ok(Value::Null)
            }
            Command::UpdateRelaunch => {
                if self.state.update.status != mado_types::state::UpdateStatus::Ready {
                    return Err(MadoError::new(ErrorCode::NotFound, "error.update.notReady"));
                }
                self.updates.relaunch();
                Ok(Value::Null)
            }
            Command::AppAuthenticate { reason } => {
                // Only Mado's own windows may ask; the prompt names Mado.
                if !origin.is_user() {
                    return Err(MadoError::new(
                        ErrorCode::Protected,
                        "error.auth.notAllowed",
                    ));
                }
                #[cfg(any(test, feature = "test-hooks"))]
                let fake = self.hooks.auth_success;
                #[cfg(not(any(test, feature = "test-hooks")))]
                let fake = false;
                if let Some(reply) = reply_to.take() {
                    let inputs = self.inputs.clone();
                    tokio::task::spawn_blocking(move || {
                        let result = if fake {
                            mado_platform::auth::token()
                        } else {
                            mado_platform::auth::authenticate(&reason)
                        }
                        .map_err(|e| e.to_string());
                        let _ =
                            inputs.send(Input::Internal(Internal::Authenticated { result, reply }));
                    });
                }
                Ok(Value::Null)
            }
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
            cmd @ (Command::DownloadPause { .. }
            | Command::DownloadResume { .. }
            | Command::DownloadCancel { .. }
            | Command::DownloadKeep { .. }
            | Command::DownloadDiscard { .. }
            | Command::DownloadOpen { .. }
            | Command::DownloadReveal { .. }
            | Command::DownloadCopyFile { .. }
            | Command::DownloadCopyPath { .. }
            | Command::DownloadRemove { .. }
            | Command::DownloadOpenSource { .. }
            | Command::DownloadClearList) => self.download_command(cmd, rec),
            cmd @ (Command::BookmarkTree { .. }
            | Command::BookmarkSearch { .. }
            | Command::BookmarkAdd { .. }
            | Command::BookmarkAddFolder { .. }
            | Command::BookmarkUpdate { .. }
            | Command::BookmarkMove { .. }
            | Command::BookmarkDelete { .. }
            | Command::BookmarkUndoDelete { .. }
            | Command::BookmarkSort { .. }
            | Command::BookmarkOpenFolder { .. }
            | Command::BookmarkRecent { .. }
            | Command::BookmarkOfUrl { .. }
            | Command::BookmarkAddAllTabs { .. }
            | Command::BookmarkExport { .. }
            | Command::BookmarkImport { .. }) => self.bookmark_command(cmd, rec, reply_to),
            cmd @ (Command::HistoryQuery { .. }
            | Command::HistoryDelete { .. }
            | Command::HistoryExclude { .. }
            | Command::HistoryExport { .. }) => self.history_command(cmd, reply_to),
            other => self.tab_command(origin, other, rec),
        }
    }

    fn internal(&mut self, internal: Internal, rec: &mut PatchRecorder) {
        match internal {
            Internal::Startup { bounds } => {
                if let Err(e) = self.startup(bounds, rec) {
                    tracing::error!(error = %e.message_key, "startup could not open a window");
                }
            }
            Internal::OpenWindow { bounds } => {
                if let Err(e) = self.open_window(bounds, &[]) {
                    tracing::error!(error = %e.message_key, "a window could not open");
                }
            }
            Internal::QuitRequested { shortcut } => self.quit_requested(shortcut, rec),
            Internal::Terminate => self.terminate(),
            #[cfg(test)]
            Internal::TestUnsavedInput { tab } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.has_unsaved_input = true;
                }
            }
            Internal::MemoryPressure { critical } => {
                tracing::info!(critical, "memory pressure");
                self.sleep_check(true);
            }
            Internal::Power(power) => self.power = power,
            Internal::SessionSaved { tab, blob } => {
                if let Some(e) = self.model.extras.get_mut(&tab) {
                    e.session = Some(blob);
                }
            }
            Internal::SleepReady { tab, blob } => self.sleep_ready(tab, blob),
            Internal::VisitRecorded { tab, visit } => self.visit_recorded(tab, visit),
            Internal::BookmarksChanged => self.bookmarks_changed(rec),
            Internal::DownloadVerdict { id, danger } => self.download_verdict(id, danger, rec),
            Internal::BookmarksDeleted(nodes) => self.bookmarks_deleted(nodes, rec),
            Internal::OpenUrls {
                urls,
                disposition,
                window,
            } => {
                if let Err(e) = self.open_urls(&urls, disposition, window) {
                    tracing::error!(error = %e.message_key, "the links did not open");
                }
            }
            Internal::SnapshotTaken { tab, jpeg } => {
                if let Some(e) = self.model.extras.get_mut(&tab) {
                    e.snapshot = Some(jpeg);
                }
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
            Internal::UpdateStatus(update) => {
                let ready = update.status == mado_types::state::UpdateStatus::Ready
                    && self.state.update.status != mado_types::state::UpdateStatus::Ready;
                let mut tx = self.state.tx(rec);
                let mut u = tx.update();
                u.set_status(update.status);
                u.set_version(update.version.clone());
                u.set_security(update.security);
                if ready {
                    // "Relaunch to update" (spec 15 §5), until the UI has its own button.
                    let notice = Notice {
                        id: format!("n{}", self.next_notice),
                        message_key: "notice.updateReady".into(),
                        args: BTreeMap::from([(
                            "version".to_owned(),
                            json!(update.version.unwrap_or_default()),
                        )]),
                        action: Some(NoticeAction {
                            label_key: "notice.relaunch".into(),
                            command: Command::UpdateRelaunch,
                        }),
                        undo: None,
                        expires_at: None,
                    };
                    self.next_notice += 1;
                    let mut notices = self.state.notices.clone();
                    notices.push(notice.clone());
                    self.state.tx(rec).set_notices(notices);
                    self.ui.notice(&notice);
                }
            }
            Internal::TokenIssued { token } => {
                let now = self.now();
                self.tokens.issue(token, now);
            }
            Internal::Authenticated { result, reply } => {
                let answer = result
                    .map(|token| {
                        let now = self.now();
                        self.tokens.issue(token.clone(), now);
                        json!({ "token": token })
                    })
                    .map_err(|reason| {
                        MadoError::new(ErrorCode::PermissionDenied, "error.auth.failed")
                            .arg("reason", reason)
                    });
                let _ = reply.send(answer);
            }
            Internal::Shutdown { .. } => {}
            #[cfg(any(test, feature = "test-hooks"))]
            Internal::Hook(hook) => match hook {
                TestHook::DropNextPatch => self.hooks.drop_next_patch = true,
                TestHook::FakeClock(now) => self.hooks.clock = Some(now),
                TestHook::FakeAuthSuccess => self.hooks.auth_success = true,
            },
            #[cfg(any(test, feature = "test-hooks"))]
            Internal::Panic => {
                self.state.tx(rec).profile().set_name("half-done".into());
                panic!("test hook: a handler panicked");
            }
        }
    }

    fn engine_event(&mut self, event: EngineEvent, rec: &mut PatchRecorder) {
        match event {
            EngineEvent::DecidePolicy { request, reply, .. } => {
                let https_first = HttpsFirst::from_setting(
                    self.settings
                        .value(lookup_def("security.httpsFirst"))
                        .as_str()
                        .unwrap_or_default(),
                );
                let context = PolicyContext {
                    https_first,
                    opens_externally: &|_| false,
                    http_fallback: &|_| false,
                };
                reply.send(navigation::decide(&request, &context));
            }
            // Tabs and windows (04). Page, permission, dialog and download
            // events get their handlers with their tasks; an unanswered
            // reply takes its safe default when dropped.
            other => self.tab_event(other, rec),
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
        if out.changes.iter().any(|c| c.key.starts_with("proxy.")) {
            self.outbox.push(EngineOp::SetProxy(self.proxy()));
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
        let model_before = self.model.clone();
        match timer {
            Timer::SleepCheck => {
                self.sleep_check(false);
                self.timers
                    .schedule(Timer::SleepCheck, session::SLEEP_CHECK);
            }
            Timer::HeartbeatCheck => {
                self.heartbeat_check(&mut rec);
                self.timers
                    .schedule(Timer::HeartbeatCheck, pages::HEARTBEAT_CHECK);
            }
            Timer::QuitNow => {
                if self.quit_after_downloads {
                    self.quit_after_downloads = false;
                    self.ui.quit();
                }
            }
            Timer::ForgetDeletedBookmarks(token) => {
                self.deleted_bookmarks.remove(&token);
            }
            Timer::ForgetDeletedGroup(group) => {
                self.deleted_groups.remove(&group);
            }
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
                self.history_retention();
            }
        }
        self.settle(&model_before, &mut rec);
    }

    /// The profile's proxy from `proxy.*` (spec 06: proxy settings call
    /// `engine.set_proxy`).
    fn proxy(&self) -> mado_engine::ProxyConfig {
        let s = |k: &str| {
            self.settings
                .value(lookup_def(k))
                .as_str()
                .unwrap_or_default()
                .to_owned()
        };
        let bypass = self
            .settings
            .value(lookup_def("proxy.bypass"))
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        mado_engine::ProxyConfig::from_settings(
            &s("proxy.mode"),
            &s("proxy.http"),
            &s("proxy.socks5"),
            bypass,
        )
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
            "general.openAtLogin" => {
                let on = change.new == true;
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = mado_platform::login::set(on) {
                        tracing::warn!(error = %e, "open at login was not changed");
                    }
                });
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
