//! The core actor (spec 01 §2): the state document and every service, one
//! input at a time, on a dedicated thread with a `current_thread` runtime.

mod applinks;
mod blocker;
mod blocker_stats;
mod clear;
mod cosmetic;
mod dialogs;
mod dl;
mod errors;
mod ext_imported;
mod ext_manage;
mod ext_overlap;
mod ext_update;
mod extensions;
mod fill;
mod import;
mod internal;
mod lists;
mod lock;
mod marks;
mod menus;
mod nav;
mod pages;
mod profiles;
mod reader;
mod save;
mod session;
mod site;
pub(crate) mod vault;
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

use crate::store::{self, Loaded};
use crate::tabs::{ClosedEntry, Model};
use crate::timers::{Timer, Timers};
use crate::tokens::Tokens;
use crate::ui::{UiSink, Updates};
pub use blocker::BlockerConfig;
pub use extensions::ExtensionsConfig;
pub use lists::ListsConfig;
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
    /// A vault job finished (spec 11 §2).
    VaultDone {
        result: Box<Result<vault::VaultOutcome, mado_vault::VaultError>>,
        reply: Option<Reply>,
    },
    /// The screen locked, the display slept or the session went to the
    /// login window: the vault locks (spec 11 §2.3).
    ScreenLocked,
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
    /// An extension package was verified and unpacked, or refused (spec 09
    /// §2.2), with the window it was asked from.
    ExtensionStaged {
        result: Result<Box<extensions::Staged>, &'static str>,
        window: Option<WindowId>,
    },
    /// The blocker's engine was built or found in the cache (spec 10 §3).
    BlockerBuilt(Box<blocker::Built>),
    /// The profile's custom filters or hidden elements changed on disk.
    BlockerProfileChanged,
    /// A filter list update finished (spec 10 §2).
    BlockerUpdated(mado_blocker::UpdateReport),
    /// Whether the blocker's engines block a request; tests only.
    #[cfg(test)]
    BlockerCheck {
        url: String,
        source: String,
        kind: String,
        reply: std::sync::mpsc::Sender<bool>,
    },
    /// A store extension's update was verified and unpacked (spec 09 §10).
    ExtensionUpdateStaged(Box<extensions::Staged>),
    /// The last good copies of the signed lists were read (spec 09 §7).
    ListsLoaded(Vec<mado_lists::List>),
    /// The daily lists fetch finished.
    ListsFetched(lists::Fetched),
    /// The captive portal probe found a portal after a failed load.
    CaptivePortal {
        tab: TabId,
        url: String,
        portal: String,
    },
    /// Loads `url` again in `tab` (after clearing a site's cookies).
    Reload { tab: TabId, url: String },
    /// The app for an app link was looked up (spec 07 §6).
    AppLinkResolved(applinks::Resolved),
    /// macOS answered its own permission prompt (spec 07 §4).
    OsAnswered { token: u64, granted: bool },
    /// A shim's permission request was decided (spec 07 §1.1).
    ShimDecided {
        tab: TabId,
        then: site::ShimThen,
        granted: bool,
    },
    /// A page notification was clicked in macOS.
    NotificationClicked { id: String },
    /// A held automatic download was allowed or not (spec 07 §1).
    DownloadAllowed { token: u64, granted: bool },
    /// This profile's name, colour or avatar changed (spec 12 §1.2).
    ProfileChanged(mado_types::state::ProfileInfo),
    /// An OpenSearch description was fetched and parsed (spec 05 §3).
    EngineDiscovered(mado_navigation::engines::Engine),
    /// Remote search suggestions merged with the local rows (spec 05 §2).
    RemoteSuggestions { window: WindowId, payload: Value },
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
    /// Clearing on quit is done (spec 08 §7).
    ClearedOnExit,
    /// Whether this profile may quit for a security update now: no input
    /// for 10 minutes and no typed text in a tab (spec 15 §5).
    IdleForUpdate { reply: oneshot::Sender<bool> },
    /// A command an internal page sent finished (spec 01 §4.4).
    InvokeAnswered {
        tab: TabId,
        id: u64,
        result: Result<Value, MadoError>,
    },
    /// A menu bar, Dock menu or key equivalent item was chosen (spec 01
    /// §5.4), in the key window when there is one.
    Menu {
        id: String,
        window: Option<WindowId>,
    },
    /// The document a View Source asked for.
    SourceRead {
        tab: TabId,
        url: String,
        html: String,
    },
    /// Another browser's data was read for `import.run` (spec 12 §5).
    BrowserImportRead {
        browser: mado_import::Browser,
        result: Box<Result<mado_import::Imported, MadoError>>,
        reply: Reply,
    },
    /// The import's database work is done.
    BrowserImportApplied {
        browser: mado_import::Browser,
        result: Result<import::Counts, String>,
        reply: Reply,
    },
    /// The `reader` probe answered for the page at `url` (spec 08 §10).
    ReaderProbed { tab: TabId, url: String, ok: bool },
    /// Readability's article for the page at `url`, or none.
    ReaderExtracted {
        tab: TabId,
        url: String,
        article: Option<reader::Article>,
    },
    /// The answer to `window.unlock`'s authentication (spec 08 §6).
    UnlockAnswered {
        window: WindowId,
        ok: bool,
        reply: Reply,
    },
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
    /// Presses a button of the error page on `tab`, as a click would.
    ErrorAction { tab: TabId, action: String },
    /// Runs the private window lock check now, as its timer would.
    PrivateLockCheck,
    /// Runs the security update check now, as its timer would.
    SecurityCheck,
    /// Fetches the signed lists now, as their timer would.
    ListsFetch,
    /// Checks for extension updates now, as their timer would.
    ExtensionUpdateCheck,
    /// Runs the vault's inactivity check now, as its timer would.
    VaultLockCheck,
}

/// A security update relaunches on its own after this long ready, once
/// nobody has used Mado for as long (spec 15 §5).
const SECURITY_GRACE_MS: i64 = 10 * 60_000;
const SECURITY_CHECK: Duration = Duration::from_secs(60);

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
    /// This build and the web engine (About, spec 15 §5).
    pub about: mado_types::state::AboutState,
    /// Open the last session whatever `startup.mode` says: this profile
    /// quit for an update (spec 15 §5).
    pub restore_session: bool,
    /// The time source, Unix milliseconds. Tests pass a fixed clock.
    pub clock: fn() -> i64,
    /// The captive portal probe (spec 05 §8); `None` turns detection off.
    pub captive_probe: Option<String>,
    /// The signed lists' fetch (spec 09 §7); `None` turns it off.
    pub lists: Option<ListsConfig>,
    /// Extensions (spec 09); `None` for private and guest profiles.
    pub extensions: Option<ExtensionsConfig>,
    /// The blocker's shared files (spec 10); `None` turns it off.
    pub blocker: Option<BlockerConfig>,
    /// macOS privacy permissions (spec 07 §4); tests pass fakes.
    pub os: OsPermissions,
    /// The app's profiles (spec 12 §1).
    pub profiles: std::sync::Arc<dyn crate::ui::Profiles>,
    /// Hosts treated as HSTS-preloaded besides the bundled list; tests only.
    pub hsts_extra: std::collections::BTreeSet<String>,
    /// The user's home folder, where other browsers keep their data (spec
    /// 12 §5.1). Tests pass a fixture home.
    pub home: PathBuf,
    /// Mado Passwords (spec 11); `None` in private and guest profiles.
    pub vault: Option<vault::VaultConfig>,
}

/// How the core asks macOS about its own permissions.
#[derive(Debug, Clone, Copy)]
pub struct OsPermissions {
    /// The current status; never prompts.
    pub status:
        fn(mado_platform::permissions::OsPermission) -> mado_platform::permissions::OsStatus,
    /// Shows macOS's prompt if it was never answered; blocks.
    pub request:
        fn(mado_platform::permissions::OsPermission) -> mado_platform::permissions::OsStatus,
    /// The app that opens a link of another scheme (spec 07 §6).
    pub handler: fn(&str) -> Option<mado_platform::applinks::Handler>,
    /// Keychain identities for client certificates (spec 07 §7).
    pub identities: fn() -> Vec<mado_platform::keychain::ClientIdentity>,
    /// Whether Mado is the default web browser.
    pub is_default_browser: fn() -> Result<bool, String>,
    /// Asks macOS to make Mado the default browser; macOS confirms.
    pub make_default_browser: fn() -> Result<(), String>,
    /// Unix milliseconds of the person's last mouse-down or key-down in a
    /// Mado window (spec 11 §4.1 step 3).
    pub last_gesture: fn() -> i64,
}

impl OsPermissions {
    /// The real macOS permissions.
    pub fn system() -> Self {
        use mado_platform::permissions::{OsStatus, request, status};
        Self {
            status: |p| status(p).unwrap_or(OsStatus::NotDetermined),
            request: |p| request(p).unwrap_or(OsStatus::Denied),
            handler: |url| mado_platform::applinks::handler(url).ok().flatten(),
            identities: mado_platform::keychain::identities,
            is_default_browser: || {
                mado_platform::default_browser::is_default().map_err(|e| e.to_string())
            },
            make_default_browser: || {
                mado_platform::default_browser::make_default().map_err(|e| e.to_string())
            },
            // The app passes the engine's monitor; without one, no gesture.
            last_gesture: || 0,
        }
    }
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
    /// App links (spec 07 §6): launches per tab, remembered sites and
    /// schemes, default web handlers, and the URL or template each prompt
    /// is about.
    launches: mado_permissions::LaunchLimiter<TabId>,
    app_link_rules: std::collections::BTreeSet<(String, String)>,
    protocol_handlers: BTreeMap<String, String>,
    app_link_urls: BTreeMap<PromptId, String>,
    handler_templates: BTreeMap<PromptId, String>,
    /// Open dialogs and sheets with the answers they hold (spec 07 §7).
    held: dialogs::Held,
    /// Site settings (spec 07).
    sites: mado_permissions::Store,
    dismissals: mado_permissions::Dismissals,
    pending_perms: BTreeMap<PromptId, site::Pending>,
    perm_queue: site::Queue,
    /// "Allow this time" grants, until the tab's next document.
    this_time: BTreeMap<TabId, std::collections::BTreeSet<(String, String)>>,
    os_waiting: BTreeMap<u64, mado_engine::Reply<mado_engine::event::PermissionDecision>>,
    next_os_wait: u64,
    /// Downloads waiting for `automaticDownloads`, by token.
    held_downloads: BTreeMap<
        u64,
        (
            TabId,
            mado_engine::event::EngineDownload,
            mado_engine::Reply<mado_engine::event::DownloadDestination>,
        ),
    >,
    next_held: u64,
    /// Auto-revoked rows, by notice token, for Undo.
    revoked: BTreeMap<u64, Vec<mado_permissions::Row>>,
    /// Hosts that open in Safari (site engine rules, spec 05 §9).
    engine_rules: std::collections::BTreeSet<String>,
    /// Hosts that sent HSTS this session, hosts the user proceeded to
    /// despite their certificate, and extra preloaded hosts (tests).
    hsts_seen: std::collections::BTreeSet<String>,
    proceeded: std::collections::BTreeSet<String>,
    hsts_extra: std::collections::BTreeSet<String>,
    /// When each tab last opened a pop-up, for tab-under (spec 05 §7).
    popup_opened: BTreeMap<TabId, i64>,
    /// Error pages on screen, by tab (spec 05 §6).
    error_pages: BTreeMap<TabId, errors::ErrorPage>,
    /// HTTPS-first upgrades in flight (spec 05 §5).
    upgrades: BTreeMap<TabId, errors::Upgrade>,
    /// `httpAllowed` hosts and when they expire.
    http_allowed: BTreeMap<String, i64>,
    /// http URLs the user chose to continue to, allowed once.
    continue_once: std::collections::BTreeSet<String>,
    captive_probe: Option<String>,
    lists: Option<ListsConfig>,
    extensions: Option<ExtensionsConfig>,
    blocker: Option<BlockerConfig>,
    /// The shared engine, once built (spec 10 §3).
    blocker_engine: Option<mado_blocker::Engine>,
    /// The profile's custom filters and hidden elements.
    blocker_custom: Option<mado_blocker::Engine>,
    blocker_building: bool,
    /// The lists changed during a build: build again after it.
    blocker_rebuild_again: bool,
    /// The shared engine's WebKit rule lists.
    blocker_shards: Vec<(String, PathBuf)>,
    /// The profile's own rule list: custom filters and hidden elements.
    blocker_profile_rules: Option<String>,
    /// Hosts where the profile's blocker is off.
    blocker_off_hosts: Vec<String>,
    /// The rules last sent to the engine.
    blocker_sent: Option<mado_engine::BlockerRules>,
    /// The resources (scriptlets, redirects) the engines use.
    blocker_resources: Vec<mado_blocker::Resource>,
    /// The `trackers` lists' engine, for the statistics.
    blocker_trackers: Option<mado_blocker::Engine>,
    /// Mado Passwords' files; `None` without a vault.
    vault: Option<vault::VaultConfig>,
    /// The vault's keys while it is unlocked.
    vault_keys: Option<std::sync::Arc<mado_vault::Keys>>,
    /// `vault.unlock` calls waiting for the one Touch ID sheet.
    vault_waiting: Vec<Reply>,
    vault_unlocking: bool,
    /// The last fill or vault page use, for `vault.lockAfter`.
    vault_used_at: i64,
    /// Fills need no new authentication until then (`vault.fillApprovalWindow`).
    vault_fill_until: i64,
    /// Where each open fill menu's values would go.
    fill_targets: BTreeMap<PromptId, fill::FillTarget>,
    /// A username filled on a two-step sign-in, per tab (11 §4.2 step 5).
    two_step: BTreeMap<TabId, fill::FollowUp>,
    /// A login just filled, for the code step (11 §4.2 step 6).
    otp_after: BTreeMap<TabId, fill::FollowUp>,
    /// A submitted login per tab, waiting for the next page (11 §5).
    captures: BTreeMap<TabId, save::Capture>,
    /// The username of a two-step sign-in's first page: site, name, time.
    username_hints: BTreeMap<TabId, (String, String, i64)>,
    /// A generated password per tab: site, password, time.
    generated: BTreeMap<TabId, (String, String, i64)>,
    /// What each open save prompt would store.
    save_prompts: BTreeMap<PromptId, save::PendingSave>,
    /// Blocked counts not yet written.
    blocker_stats: blocker_stats::StatsBuffer,
    /// Recent reloads per tab, for "Page not working?".
    reloads: BTreeMap<TabId, Vec<(i64, String)>>,
    /// Tabs whose document has cosmetic filtering, with its exceptions.
    cosmetic_tabs: BTreeMap<TabId, std::collections::HashSet<String>>,
    /// Packages waiting for their install prompt's answer.
    extension_installs: BTreeMap<mado_types::PromptId, extensions::Staged>,
    /// Overlap choices by category (spec 09 §8).
    overlaps: BTreeMap<String, ext_overlap::OverlapRow>,
    /// Takeovers by extension and what it took over.
    takeovers: ext_overlap::Takeovers,
    /// Installed extensions' files and pages.
    extension_info: BTreeMap<mado_types::ExtensionId, extensions::ExtInfo>,
    /// The toolbar actions of the focused window's active tab.
    extension_actions: (Option<TabId>, Vec<mado_engine::ExtensionActionInfo>),
    /// Extensions loaded in the engine.
    extension_loads: BTreeMap<mado_types::ExtensionId, mado_engine::ExtensionLoad>,
    /// What extensions last saw of windows and tabs.
    extension_windows: mado_engine::ExtensionWindows,
    os: OsPermissions,
    profiles: std::sync::Arc<dyn crate::ui::Profiles>,
    /// The profile's search engines (spec 05 §3).
    engines: Vec<nav::EngineRow>,
    /// Searches waiting for their visit, for `search_terms`.
    pending_search: BTreeMap<TabId, (String, String)>,
    /// The URL and `addressBar.show` mode each tab's `displayUrl` was
    /// computed for (spec 05 §4).
    displayed: BTreeMap<TabId, (String, bool)>,
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
    /// The forms each frame of a tab reported (spec 11 §3).
    page_forms: BTreeMap<TabId, Vec<pages::FrameForms>>,
    /// Tabs whose protected video failed (M2-T06 offers Safari).
    drm_failed: BTreeSet<TabId>,
    /// `privacy.clearOnExit` is clearing before the app quits.
    clearing_on_exit: bool,
    /// When the user last sent a command from one of Mado's windows.
    last_input: i64,
    /// Since when a security update has been ready to install.
    security_ready_at: Option<i64>,
    restore_session: bool,
    /// The menu states the app shows (spec 01 §5.4).
    menu_shown: menus::MenuState,
    /// The user's home folder, for importing (spec 12 §5.1).
    home: PathBuf,
    /// Hosts that always open in Reader (`reader_sites`).
    reader_always: BTreeSet<String>,
    /// The article page each tab's reader page came from.
    reader_sources: BTreeMap<TabId, String>,
    /// Private windows in the background, since when (spec 08 §6).
    background_since: BTreeMap<WindowId, i64>,
    /// The bundled page scripts; `gpc` joins them while `privacy.gpc` is on.
    page_scripts: mado_engine::PageScriptSet,
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
            // IO for the network requests the core makes itself (remote
            // suggestions, the captive portal probe).
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
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
        let engines = nav::merged_engines(
            config
                .profile_db
                .readers()
                .read(nav::load_engines)
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "the search engines could not be read");
                    Vec::new()
                }),
        );
        let links = config
            .profile_db
            .readers()
            .read(applinks::load)
            .unwrap_or_default();
        let sites = mado_permissions::Store::new(
            config
                .profile_db
                .readers()
                .read(site::load)
                .unwrap_or_default(),
        );
        let engine_rules = config
            .profile_db
            .readers()
            .read(nav::load_engine_rules)
            .unwrap_or_default();
        let http_allowed = config
            .profile_db
            .readers()
            .read(|c| errors::load_http_allowed(c, (config.clock)()))
            .unwrap_or_default();
        let page_scripts = config.page_scripts;
        // The welcome sheet comes once per Mac, never in a temporary profile.
        let first_run = !config.profile.is_private
            && !config.profile.is_guest
            && config
                .app_db
                .readers()
                .read(|c| {
                    use rusqlite::OptionalExtension as _;
                    c.query_row(
                        "SELECT 1 FROM app_state WHERE k = 'first_run_done'",
                        [],
                        |_| Ok(()),
                    )
                    .optional()
                })
                .ok()
                .flatten()
                .is_none();
        let reader_always = config
            .profile_db
            .readers()
            .read(reader::load)
            .unwrap_or_default();
        let mut core = Self {
            state: State {
                profile: config.profile,
                about: config.about.clone(),
                first_run,
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
            displayed: BTreeMap::new(),
            popup_opened: BTreeMap::new(),
            hsts_seen: std::collections::BTreeSet::new(),
            proceeded: std::collections::BTreeSet::new(),
            hsts_extra: config.hsts_extra.clone(),
            engine_rules,
            sites,
            held: dialogs::Held::default(),
            launches: mado_permissions::LaunchLimiter::new(),
            app_link_rules: links.0.into_iter().collect(),
            protocol_handlers: links.1,
            app_link_urls: BTreeMap::new(),
            handler_templates: BTreeMap::new(),
            dismissals: mado_permissions::Dismissals::default(),
            pending_perms: BTreeMap::new(),
            perm_queue: BTreeMap::new(),
            this_time: BTreeMap::new(),
            os_waiting: BTreeMap::new(),
            next_os_wait: 0,
            revoked: BTreeMap::new(),
            held_downloads: BTreeMap::new(),
            next_held: 0,
            error_pages: BTreeMap::new(),
            upgrades: BTreeMap::new(),
            http_allowed,
            continue_once: std::collections::BTreeSet::new(),
            captive_probe: config.captive_probe.clone(),
            lists: config.lists.clone(),
            extensions: config.extensions.clone(),
            blocker: config.blocker.clone(),
            blocker_engine: None,
            blocker_custom: None,
            blocker_building: false,
            blocker_rebuild_again: false,
            blocker_shards: Vec::new(),
            blocker_profile_rules: None,
            blocker_off_hosts: Vec::new(),
            blocker_sent: None,
            blocker_resources: Vec::new(),
            cosmetic_tabs: BTreeMap::new(),
            blocker_trackers: None,
            vault: config.vault.clone(),
            vault_keys: None,
            vault_waiting: Vec::new(),
            vault_unlocking: false,
            vault_used_at: 0,
            vault_fill_until: 0,
            fill_targets: BTreeMap::new(),
            two_step: BTreeMap::new(),
            otp_after: BTreeMap::new(),
            captures: BTreeMap::new(),
            username_hints: BTreeMap::new(),
            generated: BTreeMap::new(),
            save_prompts: BTreeMap::new(),
            blocker_stats: blocker_stats::StatsBuffer::default(),
            reloads: BTreeMap::new(),
            extension_installs: BTreeMap::new(),
            extension_loads: BTreeMap::new(),
            extension_info: BTreeMap::new(),
            overlaps: BTreeMap::new(),
            takeovers: BTreeMap::new(),
            extension_actions: (None, Vec::new()),
            extension_windows: mado_engine::ExtensionWindows::default(),
            os: config.os,
            profiles: std::sync::Arc::clone(&config.profiles),
            engines,
            pending_search: BTreeMap::new(),
            dls: BTreeMap::new(),
            dl_by_engine: BTreeMap::new(),
            page_downloads: BTreeMap::new(),
            next_resume: 0,
            quit_after_downloads: false,
            pending_transition: BTreeMap::new(),
            context_targets: BTreeMap::new(),
            page_forms: BTreeMap::new(),
            drm_failed: BTreeSet::new(),
            clearing_on_exit: false,
            last_input: (config.clock)(),
            security_ready_at: None,
            restore_session: config.restore_session,
            menu_shown: menus::MenuState::new(),
            home: config.home,
            reader_always,
            reader_sources: BTreeMap::new(),
            background_since: BTreeMap::new(),
            page_scripts,
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
        let shortcuts = core.shortcuts();
        core.ui.shortcuts(&shortcuts);
        for (state, info) in downloads {
            core.dls.insert(state.id, info);
            core.state.downloads.insert(state.id, state);
        }
        // The proxy and page scripts apply to every tab view, so before the
        // first one.
        core.engine.send(EngineOp::SetProxy(core.proxy()));
        core.engine.send(EngineOp::SetPrivacy(core.privacy()));
        core.engine.send(EngineOp::SetPageScripts(core.scripts()));
        core.timers
            .schedule(Timer::HeartbeatCheck, pages::HEARTBEAT_CHECK);
        core.timers.schedule(Timer::DailyBackup, DAY);
        core.timers
            .schedule(Timer::AutoRevoke, std::time::Duration::from_secs(600));
        core.load_lists();
        core.load_extensions();
        core.start_blocker();
        core.load_vault(&mut PatchRecorder::new());
        core
    }

    /// The installed extensions, before the first revision.
    fn load_extensions(&mut self) {
        if self.extensions.is_none() || self.state.profile.is_private {
            return;
        }
        if let Ok((overlaps, takeovers)) = self.profile_db.readers().read(ext_overlap::load) {
            self.overlaps = overlaps;
            self.takeovers = takeovers;
        }
        let dir = self.extensions_dir().unwrap_or_default();
        let rows = self
            .profile_db
            .readers()
            .read(|c| extensions::load(c, &dir))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "the extensions could not be read");
                Vec::new()
            });
        let mut ext = self.state.extensions.clone();
        for row in rows {
            if row.pinned {
                ext.pinned.push(row.installed.id.clone());
            }
            self.extension_info
                .insert(row.installed.id.clone(), row.info);
            if row.installed.enabled {
                self.load_installed(&row.installed);
            }
            ext.installed.push(row.installed);
        }
        let mut rec = PatchRecorder::new();
        self.state.tx(&mut rec).set_extensions(ext);
        self.refresh_imported(&mut rec);
        self.apply_blocked_extensions(&mut rec);
        self.refresh_controlled(&mut rec);
        self.timers
            .schedule(Timer::ExtensionUpdate, ext_update::FIRST);
        // Before any tab view, so every tab gets the extensions.
        for op in std::mem::take(&mut self.outbox) {
            self.engine.send(op);
        }
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
                    if matches!(
                        origin,
                        Origin::Ui { .. }
                            | Origin::Overlay { .. }
                            | Origin::Settings
                            | Origin::InternalPage { .. }
                    ) {
                        self.last_input = self.now();
                    }
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
        self.refresh_display_urls();
        self.sync_extension_windows();
        self.settle(&model_before, &mut rec);
        // A handler that answers later took the reply.
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
    }

    /// Recomputes `displayUrl` for tabs whose URL or display mode changed:
    /// the registrable domain, and IDN as Unicode only when it passes the
    /// spoof checks (spec 05 §4).
    fn refresh_display_urls(&mut self) {
        let full = self.settings.value(lookup_def("addressBar.show")) == "fullUrl";
        let top = mado_navigation::TopDomains::current();
        self.displayed
            .retain(|t, _| self.model.tabs.contains_key(t));
        for (id, tab) in &mut self.model.tabs {
            if self
                .displayed
                .get(id)
                .is_some_and(|(url, f)| *url == tab.url && *f == full)
            {
                continue;
            }
            let d = mado_navigation::display(&tab.url, full, &top);
            tab.display_url = mado_types::state::DisplayUrl {
                text: d.text,
                emphasis_start: d.emphasis.0,
                emphasis_end: d.emphasis.1,
                punycode: d.punycode,
            };
            self.displayed.insert(*id, (tab.url.clone(), full));
        }
    }

    /// After a handler or timer: the document follows the model, one patch
    /// goes out, then the engine calls (spec 01 §2.1 steps 5 and 6), then
    /// the database write.
    fn settle(&mut self, model_before: &Model, rec: &mut PatchRecorder) {
        let (first, last) = self.reconcile(model_before, rec);
        self.emit(rec);
        self.refresh_menu_state();
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
        self.forward_to_pages(&patch);
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
                    // Links from other apps go to the profile a link rule
                    // names (spec 12 §1.3).
                    let own = self.state.profile.id;
                    let mut request = request;
                    if request.source == mado_types::command::LaunchSource::Link {
                        let profiles = std::sync::Arc::clone(&self.profiles);
                        request.urls.retain(|url| match profiles.route(url) {
                            Some(other) if other != own => {
                                self.ui.open_in_profile(other, url);
                                false
                            }
                            _ => true,
                        });
                        if request.urls.is_empty() {
                            return Ok(Value::Null);
                        }
                    }
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
            cmd @ (Command::PromptRespond { .. }
            | Command::PromptDismiss { .. }
            | Command::PermissionList { .. }
            | Command::PermissionSet { .. }
            | Command::PermissionReset { .. }
            | Command::SiteStopCapture { .. }) => self.prompt_command(origin, cmd, rec, reply_to),
            Command::PermissionUndoRevoke { token } => self.undo_revoke(token),
            Command::SiteShowCertificate { tab_id } => {
                if !self.model.tabs.contains_key(&tab_id) {
                    return Err(MadoError::new(ErrorCode::NotFound, "error.tab.notFound"));
                }
                self.outbox.push(EngineOp::Apply(
                    tab_id,
                    mado_engine::TabCommand::ShowCertificate,
                ));
                Ok(Value::Null)
            }
            Command::WindowUnlock { window_id } => self.unlock_command(origin, window_id, reply_to),
            cmd @ (Command::TabReader { .. } | Command::SiteSetReaderAlways { .. }) => {
                self.reader_command(cmd)
            }
            cmd @ (Command::ImportDetect | Command::ImportRun { .. }) => {
                self.import_command(origin, cmd, reply_to)
            }
            cmd @ (Command::DataClear { .. } | Command::SiteClearData { .. }) => {
                self.data_command(origin, cmd, rec, reply_to)
            }
            cmd @ (Command::PromptAnswerDialog { .. }
            | Command::PromptAnswerAuth { .. }
            | Command::PromptAnswerClientCert { .. }) => self.sheet_command(cmd, rec),
            Command::AppFinishFirstRun => {
                if self.state.first_run {
                    self.state.tx(rec).set_first_run(false);
                    let pending = self
                        .app_db
                        .writer()
                        .write(|c| {
                            c.execute(
                                "INSERT OR REPLACE INTO app_state (k, v) VALUES ('first_run_done', '1')",
                                [],
                            )
                            .map(|_| ())
                        });
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "the end of the first run was not saved");
                        }
                    });
                }
                Ok(Value::Null)
            }
            Command::BlockerUpdateNow => {
                self.update_blocker(true, rec);
                Ok(Value::Null)
            }
            cmd @ (Command::BlockerSetSite { .. }
            | Command::BlockerSetLists { .. }
            | Command::BlockerAvailableLists
            | Command::BlockerCustomFilters
            | Command::SitePrivacyReport { .. }) => self.blocker_controls(origin, cmd, rec),
            cmd @ (Command::BlockerPickElement { .. }
            | Command::BlockerHidden { .. }
            | Command::BlockerUnhide { .. }
            | Command::BlockerSetCustomFilters { .. }) => self.blocker_command(cmd),
            cmd @ (Command::ExtensionInstall { .. } | Command::ExtensionConfirmInstall { .. }) => {
                self.extension_command(origin, cmd, rec)
            }
            Command::AppOpenSettings { section } => {
                match section {
                    Some(s) => self.ui.open_settings_at(&s),
                    None => self.ui.open_settings(),
                }
                Ok(Value::Null)
            }
            cmd @ (Command::ExtensionOverlapRespond { .. }
            | Command::ExtensionTakeoverRespond { .. }
            | Command::ExtensionOverlaps
            | Command::ExtensionOverlapReset { .. }) => self.overlap_command(origin, cmd, rec),
            cmd if cmd.family() == mado_types::command::CommandFamily::Vault => {
                self.vault_command(origin, cmd, reply_to, rec)
            }
            cmd if cmd.family() == mado_types::command::CommandFamily::Extension => {
                self.extension_manage(origin, cmd, rec, reply_to)
            }
            cmd @ (Command::AppDefaultBrowser | Command::AppSetDefaultBrowser) => {
                if matches!(cmd, Command::AppSetDefaultBrowser) && !origin.is_user() {
                    return Err(MadoError::new(ErrorCode::Protected, "error.app.notAllowed"));
                }
                let (is, make) = (self.os.is_default_browser, self.os.make_default_browser);
                if let Some(reply) = reply_to.take() {
                    tokio::task::spawn_blocking(move || {
                        let answer = if matches!(cmd, Command::AppSetDefaultBrowser) {
                            make().map(|()| Value::Null)
                        } else {
                            is().map(|d| json!({ "isDefault": d }))
                        };
                        let _ = reply.send(answer.map_err(|reason| {
                            MadoError::new(ErrorCode::Internal, "error.app.defaultBrowser")
                                .arg("reason", reason)
                        }));
                    });
                }
                Ok(Value::Null)
            }
            Command::AppOpenSystemSettings { pane } => {
                self.ui.open_system_settings(&pane);
                Ok(Value::Null)
            }
            cmd @ (Command::ProfileList
            | Command::ProfileCreate { .. }
            | Command::ProfileUpdate { .. }
            | Command::ProfileDelete { .. }
            | Command::ProfileOpen { .. }
            | Command::ProfileLinkRules
            | Command::ProfileExport { .. }
            | Command::ProfileImport { .. }
            | Command::ProfileLinkRulesSet { .. }) => {
                self.profile_command(origin, cmd, rec, reply_to)
            }
            Command::ProfileOpenGuest => {
                self.ui.open_temporary(true, &[]);
                Ok(Value::Null)
            }
            cmd @ (Command::NavOpen { .. }
            | Command::NavSuggest { .. }
            | Command::NavRemoveSuggestion { .. }
            | Command::NavSearchEngines
            | Command::NavOpenExternally { .. }
            | Command::NavSetSearchEngineActive { .. }) => self.nav_command(cmd, rec, reply_to),
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
            Internal::ClearedOnExit => self.cleared_on_exit(),
            Internal::IdleForUpdate { reply } => {
                let _ = reply.send(self.idle_for_update());
            }
            Internal::InvokeAnswered { tab, id, result } => self.invoke_answered(tab, id, result),
            Internal::Menu { id, window } => self.menu_action(&id, window, rec),
            Internal::SourceRead { tab, url, html } => self.source_read(tab, url, html),
            Internal::BrowserImportRead {
                browser,
                result,
                reply,
            } => self.import_read(browser, *result, reply),
            Internal::BrowserImportApplied {
                browser,
                result,
                reply,
            } => self.import_applied(browser, result, reply, rec),
            Internal::ReaderProbed { tab, url, ok } => self.reader_probed(tab, &url, ok),
            Internal::ReaderExtracted { tab, url, article } => {
                self.reader_extracted(tab, url, article);
            }
            Internal::UnlockAnswered { window, ok, reply } => {
                self.unlock_answered(window, ok, reply);
            }
            Internal::Terminate => self.terminate(),
            #[cfg(test)]
            Internal::TestUnsavedInput { tab } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.has_unsaved_input = true;
                }
            }
            Internal::VaultDone { result, reply } => self.vault_done(*result, reply, rec),
            Internal::ScreenLocked => self.lock_vault(rec),
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
            Internal::RemoteSuggestions { window, payload } => self.ui.suggest(window, &payload),
            Internal::CaptivePortal { tab, url, portal } => self.captive_portal(tab, url, portal),
            Internal::ListsLoaded(updated) => self.lists_loaded(&updated, rec),
            Internal::BlockerBuilt(built) => self.blocker_built(*built, rec),
            Internal::BlockerUpdated(report) => self.blocker_updated(report, rec),
            Internal::BlockerProfileChanged => self.load_custom_filters(),
            Internal::ExtensionUpdateStaged(staged) => self.extension_update_staged(*staged, rec),
            #[cfg(test)]
            Internal::BlockerCheck {
                url,
                source,
                kind,
                reply,
            } => {
                let _ = reply.send(self.blocker_blocks(&url, &source, &kind));
            }
            Internal::ExtensionStaged { result, window } => {
                self.extension_staged(result, window, rec);
            }
            Internal::ListsFetched(fetched) => self.lists_fetched(fetched, rec),
            Internal::EngineDiscovered(engine) => self.engine_discovered(engine),
            Internal::ProfileChanged(info) => self.profile_changed(info, rec),
            Internal::OsAnswered { token, granted } => self.os_answered(token, granted),
            Internal::AppLinkResolved(r) => self.app_link_resolved(r, rec),
            Internal::ShimDecided { tab, then, granted } => self.shim_decided(tab, then, granted),
            Internal::NotificationClicked { id } => self.notification_clicked(&id),
            Internal::DownloadAllowed { token, granted } => {
                self.download_allowed(token, granted, rec)
            }
            Internal::Reload { tab, url } => {
                let _ = self.navigate(tab, url);
            }
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
                if ready && update.security {
                    self.security_ready_at = Some(self.now());
                    self.timers
                        .schedule(Timer::SecurityRelaunch, SECURITY_CHECK);
                } else if update.status != mado_types::state::UpdateStatus::Ready {
                    self.security_ready_at = None;
                }
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
                TestHook::ErrorAction { tab, action } => self.press_error_action(tab, &action),
                TestHook::PrivateLockCheck => self.private_lock_check(),
                TestHook::SecurityCheck => self.fire(Timer::SecurityRelaunch),
                TestHook::ListsFetch => self.fire(Timer::ListsFetch),
                TestHook::ExtensionUpdateCheck => self.fire(Timer::ExtensionUpdate),
                TestHook::VaultLockCheck => self.fire(Timer::VaultLock),
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
            EngineEvent::ContextMenuChosen {
                tab,
                context,
                action,
            } => self.context_chosen(tab, context, action),
            EngineEvent::DialogRequested { tab, dialog, reply } => {
                self.dialog_requested(tab, dialog, reply, rec);
            }
            EngineEvent::AuthRequested {
                tab,
                challenge,
                reply,
            } => self.auth_requested(tab, challenge, reply, rec),
            EngineEvent::ClientCertificateRequested {
                tab,
                request,
                reply,
            } => self.client_cert_requested(tab, request, reply, rec),
            EngineEvent::PermissionRequested {
                tab,
                request,
                reply,
            } => {
                self.permission_requested(tab, request, reply, rec);
            }
            EngineEvent::DecidePolicy {
                tab,
                request,
                reply,
            } => {
                let Some(reply) = self.insecure_form(tab, &request, reply, rec) else {
                    return;
                };
                let decision = self.decide_navigation(tab, &request);
                reply.send(decision);
            }
            EngineEvent::NavigationFailed {
                tab,
                nav_id,
                url,
                error,
            } => {
                self.navigation_failed(tab, url.clone(), &error);
                self.tab_event(
                    EngineEvent::NavigationFailed {
                        tab,
                        nav_id,
                        url,
                        error,
                    },
                    rec,
                );
            }
            EngineEvent::ProcessGone { tab, reason } => {
                self.tab_event(EngineEvent::ProcessGone { tab, reason }, rec);
                if reason == mado_engine::event::GoneReason::Crashed {
                    self.page_crashed(tab);
                }
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
        if out.changes.iter().any(|c| {
            matches!(
                c.key,
                "safety.dangerousSiteWarnings"
                    | "privacy.trackingPrevention"
                    | "privacy.webrtcHideIp"
            )
        }) {
            self.outbox.push(EngineOp::SetPrivacy(self.privacy()));
        }
        if out.changes.iter().any(|c| c.key.starts_with("shortcuts.")) {
            let shortcuts = self.shortcuts();
            self.ui.shortcuts(&shortcuts);
        }
        if out.changes.iter().any(|c| c.key == "blocker.lists") {
            self.blocker_lists_changed(rec);
        }
        if out.changes.iter().any(|c| c.key == "blocker.enabled") {
            self.push_blocker_rules();
        }
        if out.changes.iter().any(|c| c.key == "privacy.gpc") {
            self.outbox.push(EngineOp::SetPageScripts(self.scripts()));
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
            Timer::UpgradeTimeout(tab) => self.upgrade_timed_out(tab),
            Timer::ListsFetch => self.fetch_lists(),
            Timer::BlockerUpdate => self.update_blocker(false, &mut rec),
            Timer::ExtensionUpdate => self.check_extension_updates(),
            Timer::VaultLock => self.vault_lock_check(&mut rec),
            Timer::BlockerStatsFlush => self.flush_blocker_stats_later(),
            Timer::AutoRevoke => {
                self.auto_revoke(&mut rec);
                self.timers
                    .schedule(Timer::AutoRevoke, std::time::Duration::from_secs(86_400));
            }
            Timer::QuitNow => {
                if self.quit_after_downloads {
                    self.quit_after_downloads = false;
                    self.quit_now();
                }
            }
            Timer::ClearOnExitLimit => self.cleared_on_exit(),
            Timer::SecurityRelaunch => {
                if let Some(since) = self.security_ready_at {
                    if self.now() - since >= SECURITY_GRACE_MS && self.idle_for_update() {
                        self.security_ready_at = None;
                        self.updates.relaunch_when_idle();
                    } else {
                        self.timers
                            .schedule(Timer::SecurityRelaunch, SECURITY_CHECK);
                    }
                }
            }
            Timer::PrivateLock(_) => self.private_lock_check(),
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
                let mut jobs = vec![(self.profile_db.path().to_owned(), Kind::Profile)];
                if let Some(v) = &self.vault {
                    jobs.push((v.db.clone(), Kind::Vault));
                    self.purge_vault();
                }
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
        // The network audit (spec 08 §11, 16 §3): with `test-hooks`, a
        // profile on the system proxy uses the recording proxy instead.
        #[cfg(feature = "test-hooks")]
        if s("proxy.mode") == "system"
            && let Ok(http) = std::env::var("MADO_TEST_PROXY")
            && !http.is_empty()
        {
            return mado_engine::ProxyConfig::Manual {
                http,
                socks5: String::new(),
                bypass: Vec::new(),
            };
        }
        mado_engine::ProxyConfig::from_settings(
            &s("proxy.mode"),
            &s("proxy.http"),
            &s("proxy.socks5"),
            bypass,
        )
    }

    /// The engine's side of `safety.dangerousSiteWarnings` and
    /// `privacy.trackingPrevention` (spec 08 §1, §8). WebKit's tracking
    /// prevention is always on; `off` and `balanced` both leave it as is.
    fn privacy(&self) -> mado_engine::PrivacyConfig {
        mado_engine::PrivacyConfig {
            dangerous_site_warnings: self
                .settings
                .get::<mado_settings::SafetyDangerousSiteWarnings>(),
            block_third_party_cookies: self
                .settings
                .get::<mado_settings::PrivacyTrackingPrevention>()
                == "strict",
            hide_local_ip: self.settings.get::<mado_settings::PrivacyWebrtcHideIp>(),
        }
    }

    /// The bundled page scripts, with `gpc` while `privacy.gpc` is on.
    fn scripts(&self) -> mado_engine::PageScriptSet {
        let mut set = self.page_scripts.clone();
        if self.settings.get::<mado_settings::PrivacyGpc>() {
            set.scripts.push(mado_engine::page::gpc());
        }
        set
    }

    /// No input for 10 minutes and no typed text in any tab (spec 15 §5).
    fn idle_for_update(&self) -> bool {
        self.now() - self.last_input >= SECURITY_GRACE_MS
            && !self.model.tabs.values().any(|t| t.has_unsaved_input)
    }

    /// Lets queued writes finish; the app then checkpoints and releases the
    /// lock (spec 01 §1.6).
    async fn shutdown(&mut self) {
        if let Some(stats) = self.flush_blocker_stats() {
            let _ = stats.await;
        }
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
