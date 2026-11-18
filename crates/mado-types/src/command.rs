//! Commands (spec 01 §4). `Command` is one serde enum; its JSON tag is the
//! command name, `<family>.<verb>`. Each spec defines its family's members;
//! a task adds a variant together with its handler, so the enum holds only
//! commands that do something.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::MadoError;
use crate::ids::{ClientId, GroupId, TabId, TaskId, WindowId};

/// Every command family of spec 01 §4.2, with the spec that defines it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CommandFamily {
    /// 01
    State,
    /// 04
    Session,
    /// 04
    Window,
    /// 04
    Tab,
    /// 04
    Group,
    /// 05
    Nav,
    /// 12
    History,
    /// 12
    Bookmark,
    /// 12
    Download,
    /// 06
    Settings,
    /// 07
    Permission,
    /// 07
    Prompt,
    /// 07, 08
    Site,
    /// 08
    Data,
    /// 09
    Extension,
    /// 10
    Blocker,
    /// 11
    Vault,
    /// 12
    Profile,
    /// 12
    Import,
    /// 13
    Ai,
    /// 01, 14
    Launch,
    /// 14
    Control,
    /// 15
    Update,
    /// 01
    App,
}

impl CommandFamily {
    pub const ALL: [CommandFamily; 24] = [
        Self::State,
        Self::Session,
        Self::Window,
        Self::Tab,
        Self::Group,
        Self::Nav,
        Self::History,
        Self::Bookmark,
        Self::Download,
        Self::Settings,
        Self::Permission,
        Self::Prompt,
        Self::Site,
        Self::Data,
        Self::Extension,
        Self::Blocker,
        Self::Vault,
        Self::Profile,
        Self::Import,
        Self::Ai,
        Self::Launch,
        Self::Control,
        Self::Update,
        Self::App,
    ];

    /// The name's prefix, as in `settings` of `settings.get`.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Session => "session",
            Self::Window => "window",
            Self::Tab => "tab",
            Self::Group => "group",
            Self::Nav => "nav",
            Self::History => "history",
            Self::Bookmark => "bookmark",
            Self::Download => "download",
            Self::Settings => "settings",
            Self::Permission => "permission",
            Self::Prompt => "prompt",
            Self::Site => "site",
            Self::Data => "data",
            Self::Extension => "extension",
            Self::Blocker => "blocker",
            Self::Vault => "vault",
            Self::Profile => "profile",
            Self::Import => "import",
            Self::Ai => "ai",
            Self::Launch => "launch",
            Self::Control => "control",
            Self::Update => "update",
            Self::App => "app",
        }
    }
}

/// Where a launch came from (spec 01 §1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LaunchSource {
    Dock,
    Link,
    File,
    Cli,
    Login,
}

/// A second launch handed to the process that owns the profile (spec 01 §1.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LaunchRequest {
    pub urls: Vec<String>,
    pub files: Vec<String>,
    pub new_window: bool,
    pub private: bool,
    pub source: LaunchSource,
    /// Another Mado process asks the owner to act (spec 12 §1.2, §1.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<OwnerAction>,
}

/// What one Mado process asks another profile's owner to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OwnerAction {
    /// Quit, as before the profile is deleted.
    Quit,
    /// The profile's name, colour or avatar changed: reload them.
    ProfileChanged,
    /// Quit so an update can install, and open the last session at the
    /// next launch (spec 15 §5).
    QuitForUpdate,
    /// The same, only if the profile has been idle for 10 minutes with no
    /// typed text in a tab; refused otherwise (security updates).
    QuitForUpdateIfIdle,
}

/// A link rule (spec 12 §1.3): URLs from other apps matching `pattern` open
/// in `profileId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkRule {
    pub pattern: String,
    pub profile_id: crate::ids::ProfileId,
}

/// Where `nav.open` opens (spec 05 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NavDisposition {
    CurrentTab,
    NewForegroundTab,
    NewBackgroundTab,
    NewWindow,
}

/// How the input reached `nav.open` (spec 05 §1); `typed` and
/// `suggestion` record the visit as typed (spec 12 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NavSource {
    Typed,
    Suggestion,
    Paste,
    Bookmark,
    External,
    Agent,
}

/// An answer to an overlap prompt (spec 09 §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OverlapChoice {
    /// The extension does it; Mado's feature steps back.
    ExtensionOnly,
    /// Both run.
    Both,
    /// Mado does it; the extension is turned off.
    MadoOnly,
    /// The password manager connects to Mado (M4).
    MadoConnect,
}

impl OverlapChoice {
    pub fn column(self) -> &'static str {
        match self {
            Self::ExtensionOnly => "extensionOnly",
            Self::Both => "both",
            Self::MadoOnly => "madoOnly",
            Self::MadoConnect => "madoConnect",
        }
    }
}

/// Where an extension comes from (spec 09 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExtensionSource {
    Store,
    /// A folder, loaded in place (Developer mode).
    Unpacked,
    /// A `.crx` file (Developer mode).
    CrxFile,
    /// A `.zip` file, unpacked into the profile (Developer mode).
    Zip,
}

impl ExtensionSource {
    /// The `extensions.source` column.
    pub fn column(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Unpacked | Self::Zip => "unpacked",
            Self::CrxFile => "crx_file",
        }
    }
}

/// An answer to a permission prompt (spec 07 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PromptDecision {
    /// Writes `allow` for the site.
    AllowWhileVisiting,
    /// Grants for this document only; nothing is written.
    AllowThisTime,
    /// Writes `block`.
    Never,
}

/// A site setting's value (spec 07 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SiteValue {
    Allow,
    Block,
    Ask,
}

/// What a page captures, for `site.stopCapture` (spec 07 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CaptureKind {
    Camera,
    Microphone,
    Screen,
    Location,
}

/// The answer to "This site needs Safari to play" (spec 05 §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExternalAnswer {
    /// Open this page in Safari.
    Once,
    /// Open it, and open this site in Safari from now on.
    Always,
    /// Stay in Mado.
    Dismiss,
}

/// `settings.reset`: one key, one section, or everything (spec 06 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", untagged)]
#[ts(export)]
pub enum ResetTarget {
    Key { key: String },
    Section { section: String },
    All { all: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum Command {
    /// Returns `StateSnapshot`: the whole document and its revision.
    #[serde(rename = "state.get")]
    StateGet,
    /// Accepted only from the control endpoint with the launch secret.
    #[serde(rename = "launch.forward")]
    LaunchForward(LaunchRequest),
    /// Returns `{ [key]: value }` for `keys`, or every setting.
    #[serde(rename = "settings.get")]
    SettingsGet {
        #[serde(default)]
        keys: Option<Vec<String>>,
    },
    /// `token` is the one-time token from `app.authenticate`, for the
    /// settings that need OS authentication (spec 06 §5).
    #[serde(rename = "settings.set", rename_all = "camelCase")]
    SettingsSet {
        key: String,
        #[ts(type = "unknown")]
        value: Value,
        #[serde(default)]
        token: Option<String>,
    },
    #[serde(rename = "settings.reset")]
    SettingsReset(ResetTarget),
    #[serde(rename = "settings.undo", rename_all = "camelCase")]
    SettingsUndo {
        #[serde(default)]
        log_id: Option<i64>,
    },
    /// Returns `[{ key, section, score }]`, best first.
    #[serde(rename = "settings.search")]
    SettingsSearch { query: String },
    #[serde(rename = "settings.export")]
    SettingsExport { path: String },
    /// Returns a report: applied, skipped unknown, skipped protected, invalid.
    #[serde(rename = "settings.import")]
    SettingsImport { path: String },
    /// Checks the update channel now (spec 15 §5).
    #[serde(rename = "update.check")]
    UpdateCheck,
    /// Installs a ready update and relaunches Mado.
    #[serde(rename = "update.relaunch")]
    UpdateRelaunch,
    /// Shows Touch ID (or the password) with `reason` and returns
    /// `{ token }`, valid once for 60 seconds (spec 06 §5).
    #[serde(rename = "app.authenticate")]
    AppAuthenticate { reason: String },
    // Windows, tabs, groups and sessions (spec 04 §2).
    /// A window with one New Tab page or `urls`. `private` opens a private
    /// window in its own process (spec 01).
    #[serde(rename = "window.new", rename_all = "camelCase")]
    WindowNew {
        #[serde(default)]
        private: bool,
        #[serde(default)]
        urls: Vec<String>,
    },
    #[serde(rename = "window.close", rename_all = "camelCase")]
    WindowClose { window_id: WindowId },
    /// The layout is an app setting; `windowId` is ignored in v1.
    #[serde(rename = "window.setLayout", rename_all = "camelCase")]
    WindowSetLayout {
        #[serde(default)]
        window_id: Option<WindowId>,
        layout: crate::state::TabLayout,
    },
    /// Returns `{ tabId }`.
    #[serde(rename = "tab.new", rename_all = "camelCase")]
    TabNew {
        window_id: WindowId,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        position: Option<TabPosition>,
        #[serde(default)]
        background: bool,
        #[serde(default)]
        opener_id: Option<TabId>,
    },
    #[serde(rename = "tab.close", rename_all = "camelCase")]
    TabClose { tab_ids: Vec<TabId> },
    #[serde(rename = "tab.activate", rename_all = "camelCase")]
    TabActivate { tab_id: TabId },
    /// `toGroupId`: absent keeps the group rules of a drop (join when
    /// dropped inside a group, leave when outside); `null` leaves any group.
    #[serde(rename = "tab.move", rename_all = "camelCase")]
    TabMove {
        tab_ids: Vec<TabId>,
        #[serde(default)]
        to_window_id: Option<WindowId>,
        to_index: u32,
        #[serde(default, with = "double_option")]
        #[ts(type = "string | null | undefined")]
        to_group_id: Option<Option<GroupId>>,
    },
    /// Returns `{ windowId }`.
    #[serde(rename = "tab.toNewWindow", rename_all = "camelCase")]
    TabToNewWindow {
        tab_ids: Vec<TabId>,
        #[serde(default)]
        bounds: Option<crate::state::Bounds>,
    },
    #[serde(rename = "tab.pin", rename_all = "camelCase")]
    TabPin { tab_ids: Vec<TabId> },
    #[serde(rename = "tab.unpin", rename_all = "camelCase")]
    TabUnpin { tab_ids: Vec<TabId> },
    #[serde(rename = "tab.resetPin", rename_all = "camelCase")]
    TabResetPin { tab_id: TabId },
    /// An empty title clears the custom title.
    #[serde(rename = "tab.rename", rename_all = "camelCase")]
    TabRename { tab_id: TabId, title: String },
    /// Returns `{ tabId }`.
    #[serde(rename = "tab.duplicate", rename_all = "camelCase")]
    TabDuplicate { tab_id: TabId },
    #[serde(rename = "tab.sleep", rename_all = "camelCase")]
    TabSleep { tab_ids: Vec<TabId> },
    #[serde(rename = "tab.mute", rename_all = "camelCase")]
    TabMute { tab_ids: Vec<TabId>, muted: bool },
    #[serde(rename = "tab.muteSite", rename_all = "camelCase")]
    TabMuteSite { host: String, muted: bool },
    #[serde(rename = "tab.moveToProfile", rename_all = "camelCase")]
    TabMoveToProfile {
        tab_id: TabId,
        profile_id: crate::ids::ProfileId,
    },
    #[serde(rename = "tab.selectNext", rename_all = "camelCase")]
    TabSelectNext { window_id: WindowId },
    #[serde(rename = "tab.selectPrevious", rename_all = "camelCase")]
    TabSelectPrevious { window_id: WindowId },
    /// 0-based; ⌘9 sends `last: true`.
    #[serde(rename = "tab.selectIndex", rename_all = "camelCase")]
    TabSelectIndex {
        window_id: WindowId,
        #[serde(default)]
        index: u32,
        #[serde(default)]
        last: bool,
    },
    #[serde(rename = "tab.closeOthers", rename_all = "camelCase")]
    TabCloseOthers { tab_id: TabId },
    #[serde(rename = "tab.closeRight", rename_all = "camelCase")]
    TabCloseRight { tab_id: TabId },
    #[serde(rename = "tab.reloadAll", rename_all = "camelCase")]
    TabReloadAll { window_id: WindowId },
    #[serde(rename = "tab.reload", rename_all = "camelCase")]
    TabReload {
        tab_id: TabId,
        #[serde(default)]
        bypass_cache: bool,
    },
    #[serde(rename = "tab.stop", rename_all = "camelCase")]
    TabStop { tab_id: TabId },
    #[serde(rename = "tab.back", rename_all = "camelCase")]
    TabBack { tab_id: TabId },
    #[serde(rename = "tab.forward", rename_all = "camelCase")]
    TabForward { tab_id: TabId },
    /// 1.0 is 100%; 0.25 to 5.
    #[serde(rename = "tab.zoom", rename_all = "camelCase")]
    TabZoom { tab_id: TabId, zoom: f64 },
    /// A new `query` starts a search; the same query moves to the next
    /// match (`forward`); `close` ends it.
    #[serde(rename = "tab.find", rename_all = "camelCase")]
    TabFind {
        tab_id: TabId,
        #[serde(default)]
        query: Option<String>,
        #[serde(default = "yes")]
        forward: bool,
        #[serde(default)]
        case_sensitive: bool,
        #[serde(default)]
        close: bool,
    },
    #[serde(rename = "tab.print", rename_all = "camelCase")]
    TabPrint { tab_id: TabId },
    /// Saves the page as PDF or a web archive at `path`.
    #[serde(rename = "tab.save", rename_all = "camelCase")]
    TabSave {
        tab_id: TabId,
        path: String,
        format: SaveFormat,
    },
    /// Answers the "Page unresponsive" prompt (spec 02 §5.3).
    #[serde(rename = "tab.answerUnresponsive", rename_all = "camelCase")]
    TabAnswerUnresponsive {
        prompt_id: crate::ids::PromptId,
        exit: bool,
    },
    /// Returns `{ title, url, sleeping, memoryBytes?, snapshot? }` (spec 04 §11).
    #[serde(rename = "tab.hoverInfo", rename_all = "camelCase")]
    TabHoverInfo { tab_id: TabId },
    /// Returns `{ groupId }`.
    #[serde(rename = "group.create", rename_all = "camelCase")]
    GroupCreate {
        tab_ids: Vec<TabId>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        color: Option<crate::state::GroupColor>,
    },
    #[serde(rename = "group.update", rename_all = "camelCase")]
    GroupUpdate {
        group_id: GroupId,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        color: Option<crate::state::GroupColor>,
        #[serde(default)]
        collapsed: Option<bool>,
    },
    #[serde(rename = "group.ungroup", rename_all = "camelCase")]
    GroupUngroup { group_id: GroupId },
    #[serde(rename = "group.close", rename_all = "camelCase")]
    GroupClose { group_id: GroupId },
    #[serde(rename = "group.reopen", rename_all = "camelCase")]
    GroupReopen {
        group_id: GroupId,
        window_id: WindowId,
    },
    #[serde(rename = "group.delete", rename_all = "camelCase")]
    GroupDelete { group_id: GroupId },
    /// The Undo of `group.delete`, within 10 seconds.
    #[serde(rename = "group.undoDelete", rename_all = "camelCase")]
    GroupUndoDelete { group_id: GroupId },
    #[serde(rename = "group.setSaved", rename_all = "camelCase")]
    GroupSetSaved { group_id: GroupId, saved: bool },
    /// `windowId` absent: a new window.
    #[serde(rename = "group.moveToWindow", rename_all = "camelCase")]
    GroupMoveToWindow {
        group_id: GroupId,
        #[serde(default)]
        window_id: Option<WindowId>,
    },
    // Downloads (spec 12 §4.3).
    #[serde(rename = "download.pause", rename_all = "camelCase")]
    DownloadPause { download_id: crate::ids::DownloadId },
    #[serde(rename = "download.resume", rename_all = "camelCase")]
    DownloadResume { download_id: crate::ids::DownloadId },
    #[serde(rename = "download.cancel", rename_all = "camelCase")]
    DownloadCancel { download_id: crate::ids::DownloadId },
    /// Keeps a dangerous or insecure download.
    #[serde(rename = "download.keep", rename_all = "camelCase")]
    DownloadKeep { download_id: crate::ids::DownloadId },
    /// Deletes a dangerous or insecure download.
    #[serde(rename = "download.discard", rename_all = "camelCase")]
    DownloadDiscard { download_id: crate::ids::DownloadId },
    /// Opens with the default app; refused while dangerous.
    #[serde(rename = "download.open", rename_all = "camelCase")]
    DownloadOpen { download_id: crate::ids::DownloadId },
    #[serde(rename = "download.reveal", rename_all = "camelCase")]
    DownloadReveal { download_id: crate::ids::DownloadId },
    #[serde(rename = "download.copyFile", rename_all = "camelCase")]
    DownloadCopyFile { download_id: crate::ids::DownloadId },
    #[serde(rename = "download.copyPath", rename_all = "camelCase")]
    DownloadCopyPath { download_id: crate::ids::DownloadId },
    /// From the list only; the file stays.
    #[serde(rename = "download.remove", rename_all = "camelCase")]
    DownloadRemove { download_id: crate::ids::DownloadId },
    /// Opens the page the download came from.
    #[serde(rename = "download.openSource", rename_all = "camelCase")]
    DownloadOpenSource { download_id: crate::ids::DownloadId },
    /// Removes every finished download from the list.
    #[serde(rename = "download.clearList")]
    DownloadClearList,
    // Bookmarks (spec 12 §3.2). Queries return nodes `{ id, parentId,
    // kind, title, url, index, root, createdAt, children? }`.
    #[serde(rename = "bookmark.tree", rename_all = "camelCase")]
    BookmarkTree {
        #[serde(default)]
        parent_id: Option<String>,
    },
    #[serde(rename = "bookmark.search", rename_all = "camelCase")]
    BookmarkSearch { query: String },
    /// Returns `{ id }`. Default parent: the last folder used, else Other
    /// bookmarks.
    #[serde(rename = "bookmark.add", rename_all = "camelCase")]
    BookmarkAdd {
        #[serde(default)]
        parent_id: Option<String>,
        title: String,
        url: String,
        #[serde(default)]
        index: Option<u32>,
    },
    /// Returns `{ id }`.
    #[serde(rename = "bookmark.addFolder", rename_all = "camelCase")]
    BookmarkAddFolder {
        #[serde(default)]
        parent_id: Option<String>,
        title: String,
        #[serde(default)]
        index: Option<u32>,
    },
    #[serde(rename = "bookmark.update", rename_all = "camelCase")]
    BookmarkUpdate {
        id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        url: Option<String>,
    },
    #[serde(rename = "bookmark.move", rename_all = "camelCase")]
    BookmarkMove {
        id: String,
        parent_id: String,
        index: u32,
    },
    /// Offers Undo for 10 seconds (`bookmark.undoDelete`).
    #[serde(rename = "bookmark.delete", rename_all = "camelCase")]
    BookmarkDelete { ids: Vec<String> },
    #[serde(rename = "bookmark.undoDelete", rename_all = "camelCase")]
    BookmarkUndoDelete { token: u64 },
    #[serde(rename = "bookmark.sort", rename_all = "camelCase")]
    BookmarkSort {
        folder_id: String,
        by: BookmarkSortKey,
    },
    #[serde(rename = "bookmark.openFolder", rename_all = "camelCase")]
    BookmarkOpenFolder {
        id: String,
        disposition: OpenDisposition,
        #[serde(default)]
        window_id: Option<WindowId>,
    },
    #[serde(rename = "bookmark.recent", rename_all = "camelCase")]
    BookmarkRecent {
        #[serde(default = "ten")]
        limit: u32,
    },
    /// The bookmarks of one URL, for ⌘D's dialog.
    #[serde(rename = "bookmark.ofUrl", rename_all = "camelCase")]
    BookmarkOfUrl { url: String },
    /// Returns `{ id }` of the new folder.
    #[serde(rename = "bookmark.addAllTabs", rename_all = "camelCase")]
    BookmarkAddAllTabs {
        window_id: WindowId,
        folder_name: String,
    },
    #[serde(rename = "bookmark.export", rename_all = "camelCase")]
    BookmarkExport { path: String },
    /// Returns `{ imported }`, the number of links.
    #[serde(rename = "bookmark.import", rename_all = "camelCase")]
    BookmarkImport { path: String },
    // Profiles (spec 12 §1).
    /// Returns `[{ id, name, color, avatar, current, running }]`.
    #[serde(rename = "profile.list")]
    ProfileList,
    /// Returns `{ id }`; the new profile opens in its own process.
    #[serde(rename = "profile.create", rename_all = "camelCase")]
    ProfileCreate {
        name: String,
        color: String,
        #[serde(default)]
        avatar: String,
    },
    #[serde(rename = "profile.update", rename_all = "camelCase")]
    ProfileUpdate {
        id: crate::ids::ProfileId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        avatar: Option<String>,
    },
    /// Quits the profile's process and deletes its folder, website data and
    /// Keychain items. Not the profile asking.
    #[serde(rename = "profile.delete", rename_all = "camelCase")]
    ProfileDelete { id: crate::ids::ProfileId },
    /// Starts the profile, or opens a window in its running process.
    #[serde(rename = "profile.open", rename_all = "camelCase")]
    ProfileOpen { id: crate::ids::ProfileId },
    #[serde(rename = "profile.linkRules")]
    ProfileLinkRules,
    /// Writes the profile to a `.madoprofile` at `path` (absolute). The
    /// passphrase protects the vault, which joins the file with M4.
    #[serde(rename = "profile.export", rename_all = "camelCase")]
    ProfileExport {
        id: crate::ids::ProfileId,
        path: String,
        #[serde(default)]
        passphrase: Option<String>,
    },
    /// Creates a profile from a `.madoprofile`; returns `{ id }`.
    #[serde(rename = "profile.import", rename_all = "camelCase")]
    ProfileImport {
        path: String,
        #[serde(default)]
        passphrase: Option<String>,
    },
    /// Opens a guest window: a temporary profile deleted on close.
    #[serde(rename = "profile.openGuest")]
    ProfileOpenGuest,
    #[serde(rename = "profile.linkRules.set", rename_all = "camelCase")]
    ProfileLinkRulesSet { rules: Vec<LinkRule> },
    // Navigation (spec 05).
    /// Opens what was typed or chosen. Returns `{ tabId }` when a page
    /// opens, `{ needsEngine: true }` for a search before an engine is
    /// picked, `{ scope, query }` for `@tabs`, `@bookmarks` and
    /// `@history`, and `{ result }` for a calculation.
    #[serde(rename = "nav.open", rename_all = "camelCase")]
    NavOpen {
        window_id: WindowId,
        #[serde(default)]
        tab_id: Option<TabId>,
        input: String,
        disposition: NavDisposition,
        source: NavSource,
        /// ⌘Enter: a bare word becomes `www.<word>.com`.
        #[serde(default)]
        ctrl_enter: bool,
    },
    /// Returns `{ requestId, rows, inline }` from local providers within
    /// 30 ms. Remote search suggestions follow as the `mado://suggest`
    /// event. `deleted`: the last keystroke deleted text, so nothing
    /// completes inline.
    #[serde(rename = "nav.suggest", rename_all = "camelCase")]
    NavSuggest {
        window_id: WindowId,
        input: String,
        #[serde(default)]
        cursor: u32,
        #[serde(default)]
        deleted: bool,
        request_id: u32,
    },
    /// Shift+Delete on a history row: hidden from suggestions and deleted
    /// from history.
    #[serde(rename = "nav.removeSuggestion", rename_all = "camelCase")]
    NavRemoveSuggestion { url: String },
    // Permissions (spec 07).
    /// Refused before the prompt's `armedAt`.
    #[serde(rename = "prompt.respond", rename_all = "camelCase")]
    PromptRespond {
        id: crate::PromptId,
        decision: PromptDecision,
    },
    /// Escape, clicking away: denies this request only.
    #[serde(rename = "prompt.dismiss", rename_all = "camelCase")]
    PromptDismiss { id: crate::PromptId },
    /// Answers a page dialog. `suppress`: answer this document's later
    /// dialogs without showing them.
    #[serde(rename = "prompt.answerDialog", rename_all = "camelCase")]
    PromptAnswerDialog {
        id: crate::PromptId,
        accept: bool,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        suppress: bool,
    },
    /// Answers HTTP authentication; no user cancels.
    #[serde(rename = "prompt.answerAuth", rename_all = "camelCase")]
    PromptAnswerAuth {
        id: crate::PromptId,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    /// Answers a client certificate request; no identity continues without.
    #[serde(rename = "prompt.answerClientCert", rename_all = "camelCase")]
    PromptAnswerClientCert {
        id: crate::PromptId,
        #[serde(default)]
        identity: Option<String>,
    },
    /// Returns the site settings, of one origin or all.
    #[serde(rename = "permission.list", rename_all = "camelCase")]
    PermissionList {
        #[serde(default)]
        origin: Option<String>,
    },
    /// Only from Mado's own UI and pages, never a control client or agent.
    #[serde(rename = "permission.set", rename_all = "camelCase")]
    PermissionSet {
        origin: String,
        key: String,
        value: SiteValue,
        #[serde(default)]
        embedder: Option<String>,
    },
    /// Forgets `origin`'s setting for `key`, or all of them.
    #[serde(rename = "permission.reset", rename_all = "camelCase")]
    PermissionReset {
        origin: String,
        #[serde(default)]
        key: Option<String>,
    },
    #[serde(rename = "site.stopCapture", rename_all = "camelCase")]
    SiteStopCapture { tab_id: TabId, kind: CaptureKind },
    /// Unlocks a locked private window after Touch ID or the password
    /// (spec 08 §6). Only from Mado's own windows.
    #[serde(rename = "window.unlock", rename_all = "camelCase")]
    WindowUnlock { window_id: WindowId },
    /// Returns `[{ browser, profile, name }]`: installed browsers and their
    /// profiles (spec 12 §5.1).
    #[serde(rename = "import.detect")]
    ImportDetect,
    /// Imports `types` from one browser profile; answers
    /// `{ bookmarks, visits, engines, tabs, extensions }` counts once done.
    /// History is the last 90 days unless `allHistory`. Only from Mado's
    /// own windows.
    #[serde(rename = "import.run", rename_all = "camelCase")]
    ImportRun {
        browser: ImportBrowser,
        profile: String,
        types: Vec<ImportType>,
        #[serde(default)]
        all_history: bool,
    },
    /// Clears browsing data of `types` from the last `range` (spec 08 §7).
    /// Only from Mado's own windows. Answers once everything is gone.
    #[serde(rename = "data.clear", rename_all = "camelCase")]
    DataClear {
        range: ClearRange,
        types: Vec<DataType>,
    },
    /// Shows the certificate of the tab's page (spec 08 §3).
    #[serde(rename = "site.showCertificate", rename_all = "camelCase")]
    SiteShowCertificate { tab_id: TabId },
    /// Turns Reader on for the tab's article, or leaves it (spec 08 §10).
    #[serde(rename = "tab.reader", rename_all = "camelCase")]
    TabReader { tab_id: TabId, on: bool },
    /// Opens `host`'s pages in Reader whenever they have an article.
    #[serde(rename = "site.setReaderAlways", rename_all = "camelCase")]
    SiteSetReaderAlways { host: String, always: bool },
    /// Removes the cookies and storage of `origin`'s site (spec 08 §7).
    #[serde(rename = "site.clearData", rename_all = "camelCase")]
    SiteClearData { origin: String },
    /// Undoes an automatic revoke (the notice's Undo).
    #[serde(rename = "permission.undoRevoke", rename_all = "camelCase")]
    PermissionUndoRevoke { token: u64 },
    /// Closes the welcome sheet for good (M2-T17).
    #[serde(rename = "app.finishFirstRun")]
    AppFinishFirstRun,
    /// Returns `{ isDefault }`: whether Mado is the default browser.
    #[serde(rename = "app.defaultBrowser")]
    AppDefaultBrowser,
    /// Asks macOS to make Mado the default browser (macOS confirms). Only
    /// from Mado's own windows.
    #[serde(rename = "app.setDefaultBrowser")]
    AppSetDefaultBrowser,
    /// Downloads the filter lists now and rebuilds the blocker (spec 10 §8).
    #[serde(rename = "blocker.updateNow")]
    BlockerUpdateNow,
    /// Starts the element picker on the tab's page (spec 10 §8).
    #[serde(rename = "blocker.pickElement", rename_all = "camelCase")]
    BlockerPickElement { tab_id: TabId },
    /// The elements hidden on `host`: `[{ id, selector, createdAt }]`.
    #[serde(rename = "blocker.hidden", rename_all = "camelCase")]
    BlockerHidden { host: String },
    /// Shows a hidden element again.
    #[serde(rename = "blocker.unhide", rename_all = "camelCase")]
    BlockerUnhide { id: i64 },
    /// Turns the blocker off or on for `host` and its subdomains, and
    /// reloads their tabs.
    #[serde(rename = "blocker.setSite", rename_all = "camelCase")]
    BlockerSetSite { host: String, enabled: bool },
    /// Chooses the filter lists (`blocker.lists`).
    #[serde(rename = "blocker.setLists", rename_all = "camelCase")]
    BlockerSetLists { ids: Vec<String> },
    /// Every list Mado offers: `[{ id, kind, enabled, filters, updatedAt }]`.
    #[serde(rename = "blocker.availableLists")]
    BlockerAvailableLists,
    /// The profile's custom filters, as text.
    #[serde(rename = "blocker.customFilters")]
    BlockerCustomFilters,
    /// Blocked ads and trackers over `days` (7 or 30): totals, the top 20
    /// sites and the top tracker domains (spec 08 §9).
    #[serde(rename = "site.privacyReport", rename_all = "camelCase")]
    SitePrivacyReport { days: u32 },
    /// Replaces the profile's custom filters, one per line.
    #[serde(rename = "blocker.setCustomFilters", rename_all = "camelCase")]
    BlockerSetCustomFilters { text: String },
    // Extensions (spec 09). Protected: only Mado's own windows.
    /// Installs from the Chrome Web Store (`id`), or in Developer mode from
    /// a folder, a `.crx` or a `.zip` (`path`). Shows the install prompt.
    #[serde(rename = "extension.install", rename_all = "camelCase")]
    ExtensionInstall {
        #[serde(default)]
        id: Option<crate::ExtensionId>,
        source: ExtensionSource,
        #[serde(default)]
        path: Option<String>,
    },
    /// Answers the install prompt: Add or Cancel.
    #[serde(rename = "extension.confirmInstall", rename_all = "camelCase")]
    ExtensionConfirmInstall {
        prompt_id: crate::PromptId,
        add: bool,
    },
    // Management (spec 09 §11). Enabling, removing and permissions are
    // protected.
    /// Returns `state.extensions.installed`.
    #[serde(rename = "extension.list")]
    ExtensionList,
    #[serde(rename = "extension.setEnabled", rename_all = "camelCase")]
    ExtensionSetEnabled {
        id: crate::ExtensionId,
        enabled: bool,
    },
    /// `deleteData` also removes what the extension stored.
    #[serde(rename = "extension.remove", rename_all = "camelCase")]
    ExtensionRemove {
        id: crate::ExtensionId,
        #[serde(default)]
        delete_data: bool,
    },
    /// `hosts` with `specific`: the sites it may read, a subset of what
    /// it asked for; omitted keeps its listed hosts.
    #[serde(rename = "extension.setSiteAccess", rename_all = "camelCase")]
    ExtensionSetSiteAccess {
        id: crate::ExtensionId,
        mode: crate::state::SiteAccessMode,
        #[serde(default)]
        hosts: Option<Vec<String>>,
    },
    #[serde(rename = "extension.setPrivateAllowed", rename_all = "camelCase")]
    ExtensionSetPrivateAllowed {
        id: crate::ExtensionId,
        allowed: bool,
    },
    /// Grants optional permissions or hosts it declared.
    #[serde(rename = "extension.grantOptional", rename_all = "camelCase")]
    ExtensionGrantOptional {
        id: crate::ExtensionId,
        permissions: Vec<String>,
    },
    #[serde(rename = "extension.revokeOptional", rename_all = "camelCase")]
    ExtensionRevokeOptional {
        id: crate::ExtensionId,
        permissions: Vec<String>,
    },
    /// `keys` like `Cmd+Shift+Y`, or none to clear. A shortcut Mado or
    /// another extension holds is a `Conflict` naming the holder.
    #[serde(rename = "extension.setShortcut", rename_all = "camelCase")]
    ExtensionSetShortcut {
        id: crate::ExtensionId,
        command: String,
        #[serde(default)]
        keys: Option<String>,
    },
    #[serde(rename = "extension.pin", rename_all = "camelCase")]
    ExtensionPin {
        id: crate::ExtensionId,
        pinned: bool,
    },
    /// Reloads an unpacked extension from its folder.
    #[serde(rename = "extension.reload", rename_all = "camelCase")]
    ExtensionReload { id: crate::ExtensionId },
    /// Developer mode: the extension's errors, as text.
    #[serde(rename = "extension.errors", rename_all = "camelCase")]
    ExtensionErrors { id: crate::ExtensionId },
    /// Developer mode: makes the extension inspectable in Safari's Web
    /// Inspector (Develop menu); `target` is `background` or `popup`.
    #[serde(rename = "extension.inspect", rename_all = "camelCase")]
    ExtensionInspect {
        id: crate::ExtensionId,
        target: String,
    },
    /// Clicks the extension's toolbar button: its popup at `anchor` (the
    /// button, in the window's UI pixels), or `action.onClicked`.
    #[serde(rename = "extension.openPopup", rename_all = "camelCase")]
    ExtensionOpenPopup {
        id: crate::ExtensionId,
        window_id: crate::WindowId,
        anchor: crate::state::Rect,
    },
    /// Opens the extension's options page in a tab.
    #[serde(rename = "extension.openOptions", rename_all = "camelCase")]
    ExtensionOpenOptions { id: crate::ExtensionId },
    /// Answers an overlap prompt.
    #[serde(rename = "extension.overlapRespond", rename_all = "camelCase")]
    ExtensionOverlapRespond {
        prompt_id: crate::PromptId,
        choice: OverlapChoice,
    },
    /// Answers a takeover prompt: keep it, or turn the extension off.
    #[serde(rename = "extension.takeoverRespond", rename_all = "camelCase")]
    ExtensionTakeoverRespond {
        prompt_id: crate::PromptId,
        keep: bool,
    },
    /// The overlap choices: `[{ category, choice, extensionId, name }]`.
    #[serde(rename = "extension.overlaps")]
    ExtensionOverlaps,
    /// Forgets a category's choice, undoing its effects; the prompt comes
    /// back for its extension.
    #[serde(rename = "extension.overlapReset", rename_all = "camelCase")]
    ExtensionOverlapReset { category: String },
    /// Stops offering imported extension `id` for reinstall, or every one
    /// without `id`. Protected.
    #[serde(rename = "extension.dismissImported", rename_all = "camelCase")]
    ExtensionDismissImported {
        #[serde(default)]
        id: Option<crate::ExtensionId>,
    },
    // Mado Passwords (spec 11). Only Mado's own windows send them.
    /// `{ status }`: `none`, `locked` or `unlocked`.
    #[serde(rename = "vault.status")]
    VaultStatus,
    /// Turns Mado Passwords on; answers `{ recoverySecret }`, shown once.
    #[serde(rename = "vault.setup")]
    VaultSetup,
    /// Unlocks with Touch ID or the account password.
    #[serde(rename = "vault.unlock")]
    VaultUnlock,
    #[serde(rename = "vault.lock")]
    VaultLock,
    /// Opens the vault with its recovery secret, as on a new Mac.
    #[serde(rename = "vault.recover", rename_all = "camelCase")]
    VaultRecover { secret: String },
    /// Item summaries without secrets: every item, Recently deleted, or a
    /// site's items.
    #[serde(rename = "vault.query", rename_all = "camelCase")]
    VaultQuery {
        #[serde(default)]
        deleted: bool,
        #[serde(default)]
        site: Option<String>,
    },
    /// One item, `{ id, item, code, codeRemaining, … }`. Passwords, card
    /// numbers and codes' secrets are blank unless `token` (from
    /// `app.authenticate`) is valid; passkeys' private keys never leave.
    #[serde(rename = "vault.get", rename_all = "camelCase")]
    VaultGet {
        id: String,
        #[serde(default)]
        token: Option<String>,
    },
    /// Adds an item, or replaces item `id`. `item` is the item JSON of
    /// spec 11 §2.2.
    #[serde(rename = "vault.save", rename_all = "camelCase")]
    VaultSave {
        #[serde(default)]
        id: Option<String>,
        #[ts(type = "Record<string, unknown>")]
        item: serde_json::Value,
    },
    /// Moves an item to Recently deleted.
    #[serde(rename = "vault.delete", rename_all = "camelCase")]
    VaultDelete { id: String },
    #[serde(rename = "vault.restore", rename_all = "camelCase")]
    VaultRestore { id: String },
    #[serde(rename = "vault.deleteForever", rename_all = "camelCase")]
    VaultDeleteForever { id: String },
    /// The fill sources and their status (spec 11 §6):
    /// `[{ id, enabled, status }]`, `status` one of `notInstalled`,
    /// `locked`, `ready` or `{ error }`.
    #[serde(rename = "vault.sources")]
    VaultSources,
    /// Imports a password CSV (spec 11 §10); `{ format, added, updated,
    /// unchanged }`. `path` is absolute.
    #[serde(rename = "vault.importCsv", rename_all = "camelCase")]
    VaultImportCsv { path: String },
    /// Writes every login as CSV, after Touch ID.
    #[serde(rename = "vault.exportCsv", rename_all = "camelCase")]
    VaultExportCsv { path: String },
    /// Writes every item as a `.madovault` sealed with `passphrase`.
    #[serde(rename = "vault.export", rename_all = "camelCase")]
    VaultExport { path: String, passphrase: String },
    /// Adds the items of a `.madovault`, merging logins.
    #[serde(rename = "vault.import", rename_all = "camelCase")]
    VaultImport { path: String, passphrase: String },
    /// Password health (spec 11 §9): `{ reused: [[item]], weak: [item],
    /// leaked: [item] | null }`, items `{ id, title, username }`. Leaks are
    /// checked only with `checkLeaks`.
    #[serde(rename = "vault.health", rename_all = "camelCase")]
    VaultHealth {
        #[serde(default)]
        check_leaks: bool,
    },
    /// Answers a passkey sheet (spec 11 §8): `save` (create), `use` with
    /// `itemId` (get), `otherDevice` (the system's own WebAuthn), or
    /// `cancel`.
    #[serde(rename = "vault.passkeyRespond", rename_all = "camelCase")]
    VaultPasskeyRespond {
        prompt_id: crate::PromptId,
        action: PasskeyAction,
        #[serde(default)]
        item_id: Option<String>,
    },
    /// Looks for an `otpauth` QR code in the tab's visible area (spec 11
    /// §7): `{ scanId, issuer, account, site, logins: [{ id, title,
    /// username }] }`, or `error.vault.noQr`.
    #[serde(rename = "vault.scanQr", rename_all = "camelCase")]
    VaultScanQr { tab_id: crate::TabId },
    /// Adds a scanned code: to Mado login `itemId`, or as a code of its own
    /// for the scanned site.
    #[serde(rename = "vault.attachTotp", rename_all = "camelCase")]
    VaultAttachTotp {
        scan_id: String,
        #[serde(default)]
        item_id: Option<String>,
    },
    /// Unlocks Bitwarden with its master password (spec 11 §6.3); the
    /// session is kept wrapped by Mado's vault.
    #[serde(rename = "vault.bitwardenUnlock", rename_all = "camelCase")]
    VaultBitwardenUnlock { password: String },
    /// Locks Bitwarden in Mado and forgets its session.
    #[serde(rename = "vault.bitwardenLock")]
    VaultBitwardenLock,
    /// Adds an AI connection (spec 13 §2.2) and tests it; answers `{ id }`.
    /// `secret` (an API key) is stored in the Keychain and never shown
    /// again. An `oauth` connection opens the provider's sign-in page.
    #[serde(rename = "ai.connection.add", rename_all = "camelCase")]
    AiConnectionAdd {
        kind: String,
        provider: String,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        secret: Option<String>,
    },
    /// Changes a connection's label, priority or secret; a new secret is
    /// tested.
    #[serde(rename = "ai.connection.update", rename_all = "camelCase")]
    AiConnectionUpdate {
        id: String,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        priority: Option<i64>,
        #[serde(default)]
        secret: Option<String>,
    },
    /// Removes a connection, its secret and its models.
    #[serde(rename = "ai.connection.remove", rename_all = "camelCase")]
    AiConnectionRemove { id: String },
    /// Lists the connection's models; answers `{ models }` (a count) or
    /// the failure, which the connection's status keeps too.
    #[serde(rename = "ai.connection.test", rename_all = "camelCase")]
    AiConnectionTest { id: String },
    /// Local servers that answer and are not connected yet (spec 13
    /// §2.2): `[{ provider, baseUrl, models }]`.
    #[serde(rename = "ai.connection.detectLocal")]
    AiConnectionDetectLocal,
    /// The subscription CLIs found on this Mac (spec 13 §2.3): `[{ provider,
    /// name, path, version, acp, needsNode, nodeOk, install, login,
    /// loginHint, ready }]`.
    #[serde(rename = "ai.cli.list")]
    AiCliList,
    /// Runs a CLI's official `install` or `login` command in the person's
    /// own terminal (spec 13 §2.3). Protected: Mado's windows only.
    #[serde(rename = "ai.cli.run", rename_all = "camelCase")]
    AiCliRun { provider: String, what: String },
    /// The memory files (spec 13 §10): `[{ path, bytes, modifiedAt }]`.
    #[serde(rename = "ai.memory.list")]
    AiMemoryList,
    /// One memory file: `{ path, text }`.
    #[serde(rename = "ai.memory.read", rename_all = "camelCase")]
    AiMemoryRead { path: String },
    /// Paragraphs matching the words: `[{ path, line, heading, text }]`.
    #[serde(rename = "ai.memory.search", rename_all = "camelCase")]
    AiMemorySearch { query: String },
    /// The person edits a memory file. Protected.
    #[serde(rename = "ai.memory.write", rename_all = "camelCase")]
    AiMemoryWrite { path: String, text: String },
    /// The person deletes a memory file. Protected.
    #[serde(rename = "ai.memory.delete", rename_all = "camelCase")]
    AiMemoryDelete { path: String },
    /// Files the inbox now instead of at the day's filing; answers `{ day,
    /// files }`. Protected.
    #[serde(rename = "ai.memory.file")]
    AiMemoryFile,
    /// Undoes a day's filing (`YYYY-MM-DD`). Protected.
    #[serde(rename = "ai.memory.undo", rename_all = "camelCase")]
    AiMemoryUndo { day: String },
    /// The skills (spec 13 §11): `[{ name, description, sites, builtin }]`.
    #[serde(rename = "ai.skill.list")]
    AiSkillList,
    /// One skill's `SKILL.md`: `{ name, document, builtin }`.
    #[serde(rename = "ai.skill.read", rename_all = "camelCase")]
    AiSkillRead { name: String },
    /// Deletes one of the person's skills. Protected.
    #[serde(rename = "ai.skill.delete", rename_all = "camelCase")]
    AiSkillDelete { name: String },
    /// A routine (spec 13 §12): a task on a schedule. `mode` is `readOnly`
    /// (default) or `askBeforeActing`. Answers `{ routineId, nextRunAt }`.
    #[serde(rename = "ai.routine.create", rename_all = "camelCase")]
    AiRoutineCreate {
        prompt: String,
        schedule: String,
        #[serde(default)]
        mode: Option<String>,
    },
    #[serde(rename = "ai.routine.update", rename_all = "camelCase")]
    AiRoutineUpdate {
        routine_id: String,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        schedule: Option<String>,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        enabled: Option<bool>,
    },
    #[serde(rename = "ai.routine.delete", rename_all = "camelCase")]
    AiRoutineDelete { routine_id: String },
    /// Runs a routine now; answers `{ taskId }`.
    #[serde(rename = "ai.routine.runNow", rename_all = "camelCase")]
    AiRoutineRunNow { routine_id: String },
    /// `[{ id, prompt, schedule, mode, enabled, lastRunAt, nextRunAt }]`.
    #[serde(rename = "ai.routine.list")]
    AiRoutineList,
    /// Every model of every connection for pickers (spec 13 §4): starred
    /// first, then shown ones by connection; `[{ connectionId, modelId,
    /// name, show, starred, info }]`, `info` null for unknown models.
    #[serde(rename = "ai.model.list")]
    AiModelList,
    /// Shows, hides, stars or unstars a model in pickers.
    #[serde(rename = "ai.model.set", rename_all = "camelCase")]
    AiModelSet {
        connection_id: String,
        model_id: String,
        #[serde(default)]
        show: Option<bool>,
        #[serde(default)]
        starred: Option<bool>,
    },
    /// Sends a chat message (spec 13 §5) with the pages, selection,
    /// screenshot and files it is about; answers `{ chatId, messageId }`
    /// at once. The answer streams on the `mado://ai` event.
    #[serde(rename = "ai.chat.send", rename_all = "camelCase")]
    AiChatSend {
        #[serde(default)]
        chat_id: Option<String>,
        text: String,
        #[serde(default)]
        context: ChatContext,
    },
    /// Chats, newest first: `[{ id, title, updatedAt }]`.
    #[serde(rename = "ai.chat.list")]
    AiChatList,
    /// A chat's messages: `{ id, title, messages: [{ id, role, text,
    /// sources, notes, model, createdAt }] }`.
    #[serde(rename = "ai.chat.get", rename_all = "camelCase")]
    AiChatGet { chat_id: String },
    #[serde(rename = "ai.chat.delete", rename_all = "camelCase")]
    AiChatDelete { chat_id: String },
    /// Stops the answer being written.
    #[serde(rename = "ai.chat.cancel", rename_all = "camelCase")]
    AiChatCancel { chat_id: String },
    /// The page commands (spec 13 §6): `[{ id, name, output, builtin,
    /// onDevice, forEditable }]`.
    #[serde(rename = "ai.command.list")]
    AiCommandList,
    /// Runs a page command on `text` from `tabId`; answers `{ promptId }`
    /// and shows the result card. `model` overrides the command's.
    #[serde(rename = "ai.command.run", rename_all = "camelCase")]
    AiCommandRun {
        tab_id: TabId,
        command_id: String,
        text: String,
        #[serde(default)]
        editable: bool,
        #[serde(default)]
        model: Option<String>,
    },
    /// Answers a result card: `copy`, `replace`, `insert`, `retry`
    /// (with `model` for "Retry with another model") or `close`.
    #[serde(rename = "ai.command.respond", rename_all = "camelCase")]
    AiCommandRespond {
        prompt_id: crate::PromptId,
        action: String,
        #[serde(default)]
        model: Option<String>,
    },
    /// Answers an approval card (spec 13 §9.3): `once`, `task` (not for
    /// payments and deletes) or `deny`.
    #[serde(rename = "ai.approval.respond", rename_all = "camelCase")]
    AiApprovalRespond {
        prompt_id: crate::PromptId,
        answer: String,
    },
    /// Starts an agent task (spec 13 §7.1); answers `{ taskId }`. From the
    /// CLI, `cli` names a subscription CLI runtime (M5-T13).
    #[serde(rename = "ai.task.start", rename_all = "camelCase")]
    AiTaskStart {
        prompt: String,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        access: Option<String>,
        #[serde(default)]
        runtime: Option<String>,
        #[serde(default)]
        cli: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        scope: Option<Vec<String>>,
    },
    /// Text the agent reads at its next step.
    #[serde(rename = "ai.task.steer", rename_all = "camelCase")]
    AiTaskSteer { task_id: TaskId, text: String },
    /// Text for after the current goal.
    #[serde(rename = "ai.task.queue", rename_all = "camelCase")]
    AiTaskQueue { task_id: TaskId, text: String },
    /// Answers a task waiting for the person (`ask`, `handoff`).
    #[serde(rename = "ai.task.answer", rename_all = "camelCase")]
    AiTaskAnswer { task_id: TaskId, text: String },
    /// `mado session resume`: answers a waiting task, or steers a running
    /// one, and follows it; answers `{ taskId }`.
    #[serde(rename = "ai.task.resume", rename_all = "camelCase")]
    AiTaskResume {
        task_id: TaskId,
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "ai.task.stop", rename_all = "camelCase")]
    AiTaskStop { task_id: TaskId },
    /// The follower leaves on purpose (exit 3): the task keeps waiting.
    #[serde(rename = "ai.task.detach", rename_all = "camelCase")]
    AiTaskDetach { task_id: TaskId },
    /// Tasks, newest first.
    #[serde(rename = "ai.task.list")]
    AiTaskList,
    /// A task with its steps, answer and audit.
    #[serde(rename = "ai.task.get", rename_all = "camelCase")]
    AiTaskGet { task_id: TaskId },
    /// Writes a task and its audit as JSON (13 §9.7).
    #[serde(rename = "ai.task.export", rename_all = "camelCase")]
    AiTaskExport { task_id: TaskId, path: String },
    /// Raises a paused task's spending limit (spec 13 §9.6): its spend so
    /// far plus `ai.spendLimits.perTask` again.
    #[serde(rename = "ai.task.raiseLimit", rename_all = "camelCase")]
    AiTaskRaiseLimit { task_id: TaskId },
    /// Hands an agent tab back to its task after the person took it over
    /// (spec 13 §7.3), the task view's Continue.
    #[serde(rename = "ai.tab.handBack", rename_all = "camelCase")]
    AiTabHandBack { tab_id: TabId },
    /// Paired control clients (spec 14 §2): `[{ id, name, scopes,
    /// createdAt, lastUsedAt, revoked }]`.
    #[serde(rename = "control.clients")]
    ControlClients,
    /// Revokes a client: its next call fails with `unauthorized`.
    #[serde(rename = "control.revoke", rename_all = "camelCase")]
    ControlRevoke { client_id: ClientId },
    /// Answers a pairing card: `scopes` from `read`, `act`, `tasks`,
    /// `settings` and `full`.
    #[serde(rename = "control.pairRespond", rename_all = "camelCase")]
    ControlPairRespond {
        prompt_id: crate::PromptId,
        allow: bool,
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// Sets a role's model (`auto` or `<connectionId>:<modelId>`), effort
    /// and fallback model (spec 13 §4).
    #[serde(rename = "ai.role.set", rename_all = "camelCase")]
    AiRoleSet {
        role: String,
        model: String,
        effort: String,
        #[serde(default)]
        fallback: Option<String>,
    },
    /// Answers the save or update prompt (spec 11 §5); `username` and
    /// `password` are the prompt's edits.
    #[serde(rename = "vault.saveRespond", rename_all = "camelCase")]
    VaultSaveRespond {
        prompt_id: crate::PromptId,
        action: SaveAction,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    /// Fills a detected form with an item from the fill menu (spec 11
    /// §4.2). Needs a native click in the last second.
    #[serde(rename = "vault.fill", rename_all = "camelCase")]
    VaultFill {
        tab_id: crate::TabId,
        form_key: String,
        item_ref: String,
    },
    /// Opens the settings window, at `section` when given.
    #[serde(rename = "app.openSettings", rename_all = "camelCase")]
    AppOpenSettings {
        #[serde(default)]
        section: Option<String>,
    },
    /// Opens the macOS privacy pane for `pane` (`camera`, `microphone`,
    /// `location`, `screen`, `speechRecognition`).
    #[serde(rename = "app.openSystemSettings", rename_all = "camelCase")]
    AppOpenSystemSettings { pane: String },
    /// Answers the `openInSafari` prompt of `tabId`.
    #[serde(rename = "nav.openExternally", rename_all = "camelCase")]
    NavOpenExternally {
        tab_id: TabId,
        answer: ExternalAnswer,
    },
    /// Returns every search engine with `active` and `isDefault`.
    #[serde(rename = "nav.searchEngines")]
    NavSearchEngines,
    /// Turns an engine (an OpenSearch one, say) on or off.
    #[serde(rename = "nav.setSearchEngineActive", rename_all = "camelCase")]
    NavSetSearchEngineActive { id: String, active: bool },
    // History (spec 12 §2.3).
    /// Returns `{ days: [{ day, visits }], cursor }`, newest first. `day`
    /// is `YYYY-MM-DD` in the caller's time zone (`utcOffsetMinutes`, as
    /// JavaScript's `-getTimezoneOffset()`).
    #[serde(rename = "history.query", rename_all = "camelCase")]
    HistoryQuery {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        from: Option<i64>,
        #[serde(default)]
        to: Option<i64>,
        #[serde(default = "page")]
        limit: u32,
        /// From the previous page's answer.
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        utc_offset_minutes: i32,
    },
    /// Returns `{ deleted }`.
    #[serde(rename = "history.delete", rename_all = "camelCase")]
    HistoryDelete {
        #[serde(default)]
        visit_ids: Vec<i64>,
        #[serde(default)]
        url_ids: Vec<i64>,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        range: Option<TimeRange>,
    },
    #[serde(rename = "history.exclude", rename_all = "camelCase")]
    HistoryExclude { host: String },
    #[serde(rename = "history.export", rename_all = "camelCase")]
    HistoryExport { path: String },
    /// Pops the closed stack (spec 04 §6).
    #[serde(rename = "session.reopenClosed")]
    SessionReopenClosed,
    /// Returns the top 25 closed entries, newest first.
    #[serde(rename = "session.recentlyClosed")]
    SessionRecentlyClosed,
    /// Answers the quit prompt (spec 04 §9): `proceed` quits or closes.
    #[serde(rename = "session.confirmClose", rename_all = "camelCase")]
    SessionConfirmClose {
        prompt_id: crate::ids::PromptId,
        proceed: bool,
    },
    /// The page area of a window in the UI's CSS pixels, reported whenever
    /// it moves or resizes (spec 01 §5.2).
    #[serde(rename = "window.reportContentRect", rename_all = "camelCase")]
    WindowReportContentRect {
        window_id: WindowId,
        rect: crate::state::Rect,
        scale: f64,
    },
}

impl Command {
    pub fn family(&self) -> CommandFamily {
        match self {
            Self::StateGet => CommandFamily::State,
            Self::LaunchForward(_) => CommandFamily::Launch,
            Self::SettingsGet { .. }
            | Self::SettingsSet { .. }
            | Self::SettingsReset(_)
            | Self::SettingsUndo { .. }
            | Self::SettingsSearch { .. }
            | Self::SettingsExport { .. }
            | Self::SettingsImport { .. } => CommandFamily::Settings,
            Self::WindowReportContentRect { .. }
            | Self::WindowNew { .. }
            | Self::WindowUnlock { .. }
            | Self::WindowClose { .. }
            | Self::WindowSetLayout { .. } => CommandFamily::Window,
            Self::TabNew { .. }
            | Self::TabClose { .. }
            | Self::TabActivate { .. }
            | Self::TabMove { .. }
            | Self::TabToNewWindow { .. }
            | Self::TabPin { .. }
            | Self::TabUnpin { .. }
            | Self::TabResetPin { .. }
            | Self::TabRename { .. }
            | Self::TabDuplicate { .. }
            | Self::TabSleep { .. }
            | Self::TabMute { .. }
            | Self::TabMuteSite { .. }
            | Self::TabMoveToProfile { .. }
            | Self::TabSelectNext { .. }
            | Self::TabSelectPrevious { .. }
            | Self::TabSelectIndex { .. }
            | Self::TabCloseOthers { .. }
            | Self::TabCloseRight { .. }
            | Self::TabReloadAll { .. }
            | Self::TabHoverInfo { .. }
            | Self::TabReload { .. }
            | Self::TabStop { .. }
            | Self::TabBack { .. }
            | Self::TabForward { .. }
            | Self::TabZoom { .. }
            | Self::TabFind { .. }
            | Self::TabPrint { .. }
            | Self::TabSave { .. }
            | Self::TabAnswerUnresponsive { .. } => CommandFamily::Tab,
            Self::GroupCreate { .. }
            | Self::GroupUpdate { .. }
            | Self::GroupUngroup { .. }
            | Self::GroupClose { .. }
            | Self::GroupReopen { .. }
            | Self::GroupDelete { .. }
            | Self::GroupUndoDelete { .. }
            | Self::GroupSetSaved { .. }
            | Self::GroupMoveToWindow { .. } => CommandFamily::Group,
            Self::DownloadPause { .. }
            | Self::DownloadResume { .. }
            | Self::DownloadCancel { .. }
            | Self::DownloadKeep { .. }
            | Self::DownloadDiscard { .. }
            | Self::DownloadOpen { .. }
            | Self::DownloadReveal { .. }
            | Self::DownloadCopyFile { .. }
            | Self::DownloadCopyPath { .. }
            | Self::DownloadRemove { .. }
            | Self::DownloadOpenSource { .. }
            | Self::DownloadClearList => CommandFamily::Download,
            Self::BookmarkTree { .. }
            | Self::BookmarkSearch { .. }
            | Self::BookmarkAdd { .. }
            | Self::BookmarkAddFolder { .. }
            | Self::BookmarkUpdate { .. }
            | Self::BookmarkMove { .. }
            | Self::BookmarkDelete { .. }
            | Self::BookmarkUndoDelete { .. }
            | Self::BookmarkSort { .. }
            | Self::BookmarkOpenFolder { .. }
            | Self::BookmarkRecent { .. }
            | Self::BookmarkOfUrl { .. }
            | Self::BookmarkAddAllTabs { .. }
            | Self::BookmarkExport { .. }
            | Self::BookmarkImport { .. } => CommandFamily::Bookmark,
            Self::NavOpen { .. }
            | Self::NavSuggest { .. }
            | Self::NavRemoveSuggestion { .. }
            | Self::NavSearchEngines
            | Self::NavOpenExternally { .. }
            | Self::NavSetSearchEngineActive { .. } => CommandFamily::Nav,
            Self::PromptRespond { .. }
            | Self::PromptDismiss { .. }
            | Self::PromptAnswerDialog { .. }
            | Self::PromptAnswerAuth { .. }
            | Self::PromptAnswerClientCert { .. } => CommandFamily::Prompt,
            Self::PermissionList { .. }
            | Self::PermissionSet { .. }
            | Self::PermissionReset { .. }
            | Self::PermissionUndoRevoke { .. } => CommandFamily::Permission,
            Self::AppOpenSystemSettings { .. }
            | Self::AppFinishFirstRun
            | Self::AppDefaultBrowser
            | Self::AppSetDefaultBrowser => CommandFamily::App,
            Self::BlockerUpdateNow
            | Self::BlockerPickElement { .. }
            | Self::BlockerHidden { .. }
            | Self::BlockerUnhide { .. }
            | Self::BlockerSetCustomFilters { .. }
            | Self::BlockerSetSite { .. }
            | Self::BlockerSetLists { .. }
            | Self::BlockerAvailableLists
            | Self::BlockerCustomFilters => CommandFamily::Blocker,
            Self::SitePrivacyReport { .. } => CommandFamily::Site,
            Self::ExtensionInstall { .. }
            | Self::ExtensionConfirmInstall { .. }
            | Self::ExtensionList
            | Self::ExtensionSetEnabled { .. }
            | Self::ExtensionRemove { .. }
            | Self::ExtensionSetSiteAccess { .. }
            | Self::ExtensionSetPrivateAllowed { .. }
            | Self::ExtensionGrantOptional { .. }
            | Self::ExtensionRevokeOptional { .. }
            | Self::ExtensionSetShortcut { .. }
            | Self::ExtensionPin { .. }
            | Self::ExtensionReload { .. }
            | Self::ExtensionErrors { .. }
            | Self::ExtensionInspect { .. }
            | Self::ExtensionOpenPopup { .. }
            | Self::ExtensionOpenOptions { .. }
            | Self::ExtensionOverlapRespond { .. }
            | Self::ExtensionTakeoverRespond { .. }
            | Self::ExtensionOverlaps
            | Self::ExtensionOverlapReset { .. }
            | Self::ExtensionDismissImported { .. } => CommandFamily::Extension,
            Self::VaultStatus
            | Self::VaultSetup
            | Self::VaultUnlock
            | Self::VaultLock
            | Self::VaultRecover { .. }
            | Self::VaultQuery { .. }
            | Self::VaultGet { .. }
            | Self::VaultSave { .. }
            | Self::VaultDelete { .. }
            | Self::VaultRestore { .. }
            | Self::VaultDeleteForever { .. }
            | Self::VaultFill { .. }
            | Self::VaultSaveRespond { .. }
            | Self::VaultSources
            | Self::VaultImportCsv { .. }
            | Self::VaultExportCsv { .. }
            | Self::VaultExport { .. }
            | Self::VaultImport { .. }
            | Self::VaultHealth { .. }
            | Self::VaultPasskeyRespond { .. }
            | Self::VaultScanQr { .. }
            | Self::VaultAttachTotp { .. }
            | Self::VaultBitwardenUnlock { .. }
            | Self::VaultBitwardenLock => CommandFamily::Vault,
            Self::AiConnectionAdd { .. }
            | Self::AiConnectionUpdate { .. }
            | Self::AiConnectionRemove { .. }
            | Self::AiConnectionTest { .. }
            | Self::AiConnectionDetectLocal
            | Self::AiCliList
            | Self::AiCliRun { .. }
            | Self::AiMemoryList
            | Self::AiMemoryRead { .. }
            | Self::AiMemorySearch { .. }
            | Self::AiMemoryWrite { .. }
            | Self::AiMemoryDelete { .. }
            | Self::AiMemoryFile
            | Self::AiMemoryUndo { .. }
            | Self::AiSkillList
            | Self::AiSkillRead { .. }
            | Self::AiSkillDelete { .. }
            | Self::AiRoutineCreate { .. }
            | Self::AiRoutineUpdate { .. }
            | Self::AiRoutineDelete { .. }
            | Self::AiRoutineRunNow { .. }
            | Self::AiRoutineList
            | Self::AiModelList
            | Self::AiModelSet { .. }
            | Self::AiRoleSet { .. }
            | Self::AiChatSend { .. }
            | Self::AiChatList
            | Self::AiChatGet { .. }
            | Self::AiChatDelete { .. }
            | Self::AiChatCancel { .. }
            | Self::AiCommandList
            | Self::AiCommandRun { .. }
            | Self::AiCommandRespond { .. }
            | Self::AiTabHandBack { .. }
            | Self::AiApprovalRespond { .. }
            | Self::AiTaskRaiseLimit { .. }
            | Self::AiTaskStart { .. }
            | Self::AiTaskSteer { .. }
            | Self::AiTaskQueue { .. }
            | Self::AiTaskAnswer { .. }
            | Self::AiTaskResume { .. }
            | Self::AiTaskStop { .. }
            | Self::AiTaskDetach { .. }
            | Self::AiTaskList
            | Self::AiTaskGet { .. }
            | Self::AiTaskExport { .. } => CommandFamily::Ai,
            Self::ControlClients | Self::ControlRevoke { .. } | Self::ControlPairRespond { .. } => {
                CommandFamily::Control
            }
            Self::AppOpenSettings { .. } => CommandFamily::App,
            Self::ProfileList
            | Self::ProfileCreate { .. }
            | Self::ProfileUpdate { .. }
            | Self::ProfileDelete { .. }
            | Self::ProfileOpen { .. }
            | Self::ProfileLinkRules
            | Self::ProfileOpenGuest
            | Self::ProfileExport { .. }
            | Self::ProfileImport { .. }
            | Self::ProfileLinkRulesSet { .. } => CommandFamily::Profile,
            Self::SiteStopCapture { .. }
            | Self::SiteClearData { .. }
            | Self::SiteShowCertificate { .. }
            | Self::SiteSetReaderAlways { .. } => CommandFamily::Site,
            Self::TabReader { .. } => CommandFamily::Tab,
            Self::DataClear { .. } => CommandFamily::Data,
            Self::ImportDetect | Self::ImportRun { .. } => CommandFamily::Import,
            Self::HistoryQuery { .. }
            | Self::HistoryDelete { .. }
            | Self::HistoryExclude { .. }
            | Self::HistoryExport { .. } => CommandFamily::History,
            Self::SessionReopenClosed
            | Self::SessionRecentlyClosed
            | Self::SessionConfirmClose { .. } => CommandFamily::Session,
            Self::AppAuthenticate { .. } => CommandFamily::App,
            Self::UpdateCheck | Self::UpdateRelaunch => CommandFamily::Update,
        }
    }

    /// The wire name, `<family>.<verb>`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::StateGet => "state.get",
            Self::LaunchForward(_) => "launch.forward",
            Self::SettingsGet { .. } => "settings.get",
            Self::SettingsSet { .. } => "settings.set",
            Self::SettingsReset(_) => "settings.reset",
            Self::SettingsUndo { .. } => "settings.undo",
            Self::SettingsSearch { .. } => "settings.search",
            Self::SettingsExport { .. } => "settings.export",
            Self::SettingsImport { .. } => "settings.import",
            Self::WindowReportContentRect { .. } => "window.reportContentRect",
            Self::WindowNew { .. } => "window.new",
            Self::WindowUnlock { .. } => "window.unlock",
            Self::WindowClose { .. } => "window.close",
            Self::WindowSetLayout { .. } => "window.setLayout",
            Self::TabNew { .. } => "tab.new",
            Self::TabClose { .. } => "tab.close",
            Self::TabActivate { .. } => "tab.activate",
            Self::TabMove { .. } => "tab.move",
            Self::TabToNewWindow { .. } => "tab.toNewWindow",
            Self::TabPin { .. } => "tab.pin",
            Self::TabUnpin { .. } => "tab.unpin",
            Self::TabResetPin { .. } => "tab.resetPin",
            Self::TabRename { .. } => "tab.rename",
            Self::TabDuplicate { .. } => "tab.duplicate",
            Self::TabSleep { .. } => "tab.sleep",
            Self::TabMute { .. } => "tab.mute",
            Self::TabMuteSite { .. } => "tab.muteSite",
            Self::TabMoveToProfile { .. } => "tab.moveToProfile",
            Self::TabSelectNext { .. } => "tab.selectNext",
            Self::TabSelectPrevious { .. } => "tab.selectPrevious",
            Self::TabSelectIndex { .. } => "tab.selectIndex",
            Self::TabCloseOthers { .. } => "tab.closeOthers",
            Self::TabCloseRight { .. } => "tab.closeRight",
            Self::TabReloadAll { .. } => "tab.reloadAll",
            Self::TabHoverInfo { .. } => "tab.hoverInfo",
            Self::TabReload { .. } => "tab.reload",
            Self::TabStop { .. } => "tab.stop",
            Self::TabBack { .. } => "tab.back",
            Self::TabForward { .. } => "tab.forward",
            Self::TabZoom { .. } => "tab.zoom",
            Self::TabFind { .. } => "tab.find",
            Self::TabPrint { .. } => "tab.print",
            Self::TabSave { .. } => "tab.save",
            Self::TabAnswerUnresponsive { .. } => "tab.answerUnresponsive",
            Self::GroupCreate { .. } => "group.create",
            Self::GroupUpdate { .. } => "group.update",
            Self::GroupUngroup { .. } => "group.ungroup",
            Self::GroupClose { .. } => "group.close",
            Self::GroupReopen { .. } => "group.reopen",
            Self::GroupDelete { .. } => "group.delete",
            Self::GroupUndoDelete { .. } => "group.undoDelete",
            Self::GroupSetSaved { .. } => "group.setSaved",
            Self::GroupMoveToWindow { .. } => "group.moveToWindow",
            Self::DownloadPause { .. } => "download.pause",
            Self::DownloadResume { .. } => "download.resume",
            Self::DownloadCancel { .. } => "download.cancel",
            Self::DownloadKeep { .. } => "download.keep",
            Self::DownloadDiscard { .. } => "download.discard",
            Self::DownloadOpen { .. } => "download.open",
            Self::DownloadReveal { .. } => "download.reveal",
            Self::DownloadCopyFile { .. } => "download.copyFile",
            Self::DownloadCopyPath { .. } => "download.copyPath",
            Self::DownloadRemove { .. } => "download.remove",
            Self::DownloadOpenSource { .. } => "download.openSource",
            Self::DownloadClearList => "download.clearList",
            Self::BookmarkTree { .. } => "bookmark.tree",
            Self::BookmarkSearch { .. } => "bookmark.search",
            Self::BookmarkAdd { .. } => "bookmark.add",
            Self::BookmarkAddFolder { .. } => "bookmark.addFolder",
            Self::BookmarkUpdate { .. } => "bookmark.update",
            Self::BookmarkMove { .. } => "bookmark.move",
            Self::BookmarkDelete { .. } => "bookmark.delete",
            Self::BookmarkUndoDelete { .. } => "bookmark.undoDelete",
            Self::BookmarkSort { .. } => "bookmark.sort",
            Self::BookmarkOpenFolder { .. } => "bookmark.openFolder",
            Self::BookmarkRecent { .. } => "bookmark.recent",
            Self::BookmarkOfUrl { .. } => "bookmark.ofUrl",
            Self::BookmarkAddAllTabs { .. } => "bookmark.addAllTabs",
            Self::BookmarkExport { .. } => "bookmark.export",
            Self::BookmarkImport { .. } => "bookmark.import",
            Self::NavOpen { .. } => "nav.open",
            Self::NavSuggest { .. } => "nav.suggest",
            Self::NavRemoveSuggestion { .. } => "nav.removeSuggestion",
            Self::NavSearchEngines => "nav.searchEngines",
            Self::NavOpenExternally { .. } => "nav.openExternally",
            Self::PromptRespond { .. } => "prompt.respond",
            Self::PromptDismiss { .. } => "prompt.dismiss",
            Self::PromptAnswerDialog { .. } => "prompt.answerDialog",
            Self::PromptAnswerAuth { .. } => "prompt.answerAuth",
            Self::PromptAnswerClientCert { .. } => "prompt.answerClientCert",
            Self::PermissionList { .. } => "permission.list",
            Self::PermissionSet { .. } => "permission.set",
            Self::PermissionReset { .. } => "permission.reset",
            Self::SiteStopCapture { .. } => "site.stopCapture",
            Self::SiteClearData { .. } => "site.clearData",
            Self::SiteShowCertificate { .. } => "site.showCertificate",
            Self::SiteSetReaderAlways { .. } => "site.setReaderAlways",
            Self::TabReader { .. } => "tab.reader",
            Self::DataClear { .. } => "data.clear",
            Self::ImportDetect => "import.detect",
            Self::ImportRun { .. } => "import.run",
            Self::PermissionUndoRevoke { .. } => "permission.undoRevoke",
            Self::AppOpenSystemSettings { .. } => "app.openSystemSettings",
            Self::AppFinishFirstRun => "app.finishFirstRun",
            Self::AppDefaultBrowser => "app.defaultBrowser",
            Self::AppSetDefaultBrowser => "app.setDefaultBrowser",
            Self::BlockerUpdateNow => "blocker.updateNow",
            Self::BlockerPickElement { .. } => "blocker.pickElement",
            Self::BlockerHidden { .. } => "blocker.hidden",
            Self::BlockerUnhide { .. } => "blocker.unhide",
            Self::BlockerSetCustomFilters { .. } => "blocker.setCustomFilters",
            Self::BlockerSetSite { .. } => "blocker.setSite",
            Self::BlockerSetLists { .. } => "blocker.setLists",
            Self::BlockerAvailableLists => "blocker.availableLists",
            Self::BlockerCustomFilters => "blocker.customFilters",
            Self::SitePrivacyReport { .. } => "site.privacyReport",
            Self::ExtensionInstall { .. } => "extension.install",
            Self::ExtensionConfirmInstall { .. } => "extension.confirmInstall",
            Self::ExtensionList => "extension.list",
            Self::ExtensionSetEnabled { .. } => "extension.setEnabled",
            Self::ExtensionRemove { .. } => "extension.remove",
            Self::ExtensionSetSiteAccess { .. } => "extension.setSiteAccess",
            Self::ExtensionSetPrivateAllowed { .. } => "extension.setPrivateAllowed",
            Self::ExtensionGrantOptional { .. } => "extension.grantOptional",
            Self::ExtensionRevokeOptional { .. } => "extension.revokeOptional",
            Self::ExtensionSetShortcut { .. } => "extension.setShortcut",
            Self::ExtensionPin { .. } => "extension.pin",
            Self::ExtensionReload { .. } => "extension.reload",
            Self::ExtensionErrors { .. } => "extension.errors",
            Self::ExtensionInspect { .. } => "extension.inspect",
            Self::ExtensionOpenPopup { .. } => "extension.openPopup",
            Self::ExtensionOpenOptions { .. } => "extension.openOptions",
            Self::ExtensionOverlapRespond { .. } => "extension.overlapRespond",
            Self::ExtensionTakeoverRespond { .. } => "extension.takeoverRespond",
            Self::ExtensionOverlaps => "extension.overlaps",
            Self::ExtensionOverlapReset { .. } => "extension.overlapReset",
            Self::ExtensionDismissImported { .. } => "extension.dismissImported",
            Self::VaultStatus => "vault.status",
            Self::VaultSetup => "vault.setup",
            Self::VaultUnlock => "vault.unlock",
            Self::VaultLock => "vault.lock",
            Self::VaultRecover { .. } => "vault.recover",
            Self::VaultQuery { .. } => "vault.query",
            Self::VaultGet { .. } => "vault.get",
            Self::VaultSave { .. } => "vault.save",
            Self::VaultDelete { .. } => "vault.delete",
            Self::VaultRestore { .. } => "vault.restore",
            Self::VaultDeleteForever { .. } => "vault.deleteForever",
            Self::VaultFill { .. } => "vault.fill",
            Self::VaultSaveRespond { .. } => "vault.saveRespond",
            Self::VaultSources => "vault.sources",
            Self::VaultImportCsv { .. } => "vault.importCsv",
            Self::VaultExportCsv { .. } => "vault.exportCsv",
            Self::VaultExport { .. } => "vault.export",
            Self::VaultImport { .. } => "vault.import",
            Self::VaultHealth { .. } => "vault.health",
            Self::VaultPasskeyRespond { .. } => "vault.passkeyRespond",
            Self::VaultScanQr { .. } => "vault.scanQr",
            Self::VaultAttachTotp { .. } => "vault.attachTotp",
            Self::VaultBitwardenUnlock { .. } => "vault.bitwardenUnlock",
            Self::VaultBitwardenLock => "vault.bitwardenLock",
            Self::AiConnectionAdd { .. } => "ai.connection.add",
            Self::AiConnectionUpdate { .. } => "ai.connection.update",
            Self::AiConnectionRemove { .. } => "ai.connection.remove",
            Self::AiConnectionTest { .. } => "ai.connection.test",
            Self::AiConnectionDetectLocal => "ai.connection.detectLocal",
            Self::AiCliList => "ai.cli.list",
            Self::AiCliRun { .. } => "ai.cli.run",
            Self::AiMemoryList => "ai.memory.list",
            Self::AiMemoryRead { .. } => "ai.memory.read",
            Self::AiMemorySearch { .. } => "ai.memory.search",
            Self::AiMemoryWrite { .. } => "ai.memory.write",
            Self::AiMemoryDelete { .. } => "ai.memory.delete",
            Self::AiMemoryFile => "ai.memory.file",
            Self::AiMemoryUndo { .. } => "ai.memory.undo",
            Self::AiSkillList => "ai.skill.list",
            Self::AiSkillRead { .. } => "ai.skill.read",
            Self::AiSkillDelete { .. } => "ai.skill.delete",
            Self::AiRoutineCreate { .. } => "ai.routine.create",
            Self::AiRoutineUpdate { .. } => "ai.routine.update",
            Self::AiRoutineDelete { .. } => "ai.routine.delete",
            Self::AiRoutineRunNow { .. } => "ai.routine.runNow",
            Self::AiRoutineList => "ai.routine.list",
            Self::AiModelList => "ai.model.list",
            Self::AiModelSet { .. } => "ai.model.set",
            Self::AiRoleSet { .. } => "ai.role.set",
            Self::AiChatSend { .. } => "ai.chat.send",
            Self::AiChatList => "ai.chat.list",
            Self::AiChatGet { .. } => "ai.chat.get",
            Self::AiChatDelete { .. } => "ai.chat.delete",
            Self::AiChatCancel { .. } => "ai.chat.cancel",
            Self::AiCommandList => "ai.command.list",
            Self::AiCommandRun { .. } => "ai.command.run",
            Self::AiCommandRespond { .. } => "ai.command.respond",
            Self::AiTabHandBack { .. } => "ai.tab.handBack",
            Self::AiApprovalRespond { .. } => "ai.approval.respond",
            Self::AiTaskRaiseLimit { .. } => "ai.task.raiseLimit",
            Self::AiTaskStart { .. } => "ai.task.start",
            Self::AiTaskSteer { .. } => "ai.task.steer",
            Self::AiTaskQueue { .. } => "ai.task.queue",
            Self::AiTaskAnswer { .. } => "ai.task.answer",
            Self::AiTaskResume { .. } => "ai.task.resume",
            Self::AiTaskStop { .. } => "ai.task.stop",
            Self::AiTaskDetach { .. } => "ai.task.detach",
            Self::AiTaskList => "ai.task.list",
            Self::AiTaskGet { .. } => "ai.task.get",
            Self::AiTaskExport { .. } => "ai.task.export",
            Self::ControlClients => "control.clients",
            Self::ControlRevoke { .. } => "control.revoke",
            Self::ControlPairRespond { .. } => "control.pairRespond",
            Self::AppOpenSettings { .. } => "app.openSettings",
            Self::ProfileList => "profile.list",
            Self::ProfileCreate { .. } => "profile.create",
            Self::ProfileUpdate { .. } => "profile.update",
            Self::ProfileDelete { .. } => "profile.delete",
            Self::ProfileOpen { .. } => "profile.open",
            Self::ProfileLinkRules => "profile.linkRules",
            Self::ProfileOpenGuest => "profile.openGuest",
            Self::ProfileExport { .. } => "profile.export",
            Self::ProfileImport { .. } => "profile.import",
            Self::ProfileLinkRulesSet { .. } => "profile.linkRules.set",
            Self::NavSetSearchEngineActive { .. } => "nav.setSearchEngineActive",
            Self::HistoryQuery { .. } => "history.query",
            Self::HistoryDelete { .. } => "history.delete",
            Self::HistoryExclude { .. } => "history.exclude",
            Self::HistoryExport { .. } => "history.export",
            Self::SessionReopenClosed => "session.reopenClosed",
            Self::SessionRecentlyClosed => "session.recentlyClosed",
            Self::SessionConfirmClose { .. } => "session.confirmClose",
            Self::AppAuthenticate { .. } => "app.authenticate",
            Self::UpdateCheck => "update.check",
            Self::UpdateRelaunch => "update.relaunch",
        }
    }

    /// The control scope a client needs for this command (spec 14 §2):
    /// `read` for queries, `settings` for settings, `tasks` for chats and
    /// tasks, `act` for the rest. Protected commands also need `full`.
    pub fn control_scope(&self) -> &'static str {
        match self.family() {
            CommandFamily::Settings if !self.is_query() => "settings",
            _ if self.is_query() => "read",
            _ if matches!(
                self,
                Self::AiChatSend { .. } | Self::AiChatCancel { .. } | Self::AiChatDelete { .. }
            ) || self.name().starts_with("ai.task.")
                || self.name().starts_with("ai.routine.") =>
            {
                "tasks"
            }
            _ => "act",
        }
    }

    /// Queries return data and never change state, so they may run on the
    /// read pool (spec 01 §4.3).
    pub fn is_query(&self) -> bool {
        matches!(
            self,
            Self::StateGet
                | Self::SettingsGet { .. }
                | Self::SettingsSearch { .. }
                | Self::TabHoverInfo { .. }
                | Self::SessionRecentlyClosed
                | Self::HistoryQuery { .. }
                | Self::NavSuggest { .. }
                | Self::NavSearchEngines
                | Self::PermissionList { .. }
                | Self::ProfileList
                | Self::ProfileLinkRules
                | Self::BookmarkTree { .. }
                | Self::BookmarkSearch { .. }
                | Self::BookmarkRecent { .. }
                | Self::BookmarkOfUrl { .. }
                | Self::ImportDetect
                | Self::AppDefaultBrowser
                | Self::ExtensionList
                | Self::BlockerHidden { .. }
                | Self::ExtensionOverlaps
                | Self::BlockerAvailableLists
                | Self::BlockerCustomFilters
                | Self::SitePrivacyReport { .. }
                | Self::AiChatList
                | Self::AiChatGet { .. }
                | Self::AiModelList
                | Self::AiCliList
                | Self::AiMemoryList
                | Self::AiMemoryRead { .. }
                | Self::AiMemorySearch { .. }
                | Self::AiSkillList
                | Self::AiSkillRead { .. }
                | Self::AiRoutineList
        )
    }
}

/// What a chat message is about (spec 13 §5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatContext {
    #[serde(default)]
    pub tab_ids: Vec<TabId>,
    #[serde(default)]
    pub selection: Option<ChatSelection>,
    #[serde(default)]
    pub screenshot_tab_id: Option<TabId>,
    /// Absolute paths of text files.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatSelection {
    pub tab_id: TabId,
    pub text: String,
}

/// How a passkey sheet was answered (spec 11 §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PasskeyAction {
    Save,
    Use,
    OtherDevice,
    Cancel,
}

/// How the save prompt was answered (spec 11 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SaveAction {
    Save,
    NotNow,
    /// Never on this site.
    Never,
}

fn yes() -> bool {
    true
}

fn page() -> u32 {
    100
}

fn ten() -> u32 {
    10
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BookmarkSortKey {
    Name,
    Url,
}

/// Where links open (bookmark folders, context menus).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OpenDisposition {
    /// New tabs in the window, the first one active.
    NewTabs,
    NewWindow,
}

/// Unix milliseconds, `from` included, `to` not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TimeRange {
    pub from: i64,
    pub to: i64,
}

/// A browser to import from (spec 12 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ImportBrowser {
    Chrome,
    Edge,
    Brave,
    Arc,
    Firefox,
    Safari,
}

/// What `import.run` brings over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ImportType {
    Bookmarks,
    History,
    SearchEngines,
    Tabs,
    Extensions,
}

/// How far back `data.clear` reaches (spec 08 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ClearRange {
    Hour,
    Day,
    Week,
    Month,
    All,
}

impl ClearRange {
    /// The span in milliseconds; `None` for all time.
    pub fn millis(self) -> Option<i64> {
        const HOUR: i64 = 3_600_000;
        match self {
            Self::Hour => Some(HOUR),
            Self::Day => Some(24 * HOUR),
            Self::Week => Some(7 * 24 * HOUR),
            Self::Month => Some(30 * 24 * HOUR),
            Self::All => None,
        }
    }
}

/// What `data.clear` and `privacy.clearOnExit` clear (spec 08 §7).
/// Passwords are never among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DataType {
    History,
    DownloadsList,
    Cookies,
    Cache,
    SiteSettings,
    FormHistory,
    BlockerStats,
    AiChats,
}

/// `tab.save` formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SaveFormat {
    Pdf,
    WebArchive,
}

/// Where `tab.new` puts the tab (spec 04 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", untagged)]
#[ts(export)]
pub enum TabPosition {
    Named(NamedPosition),
    Index { index: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NamedPosition {
    AfterActive,
    End,
}

/// `Option<Option<T>>` that tells a missing field from `null`.
mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(
        value: &Option<Option<T>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            None => s.serialize_none(),
            Some(inner) => inner.serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

/// A `mado://` page that runs in a tab and may send commands (spec 01 §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum InternalPage {
    NewTab,
    History,
    Downloads,
    Bookmarks,
    About,
    Credits,
    /// Lists extensions; managing them is in Settings › Extensions.
    Extensions,
    /// Points to Settings › Passwords, where the vault is managed (no
    /// secret reaches a page in a tab).
    Passwords,
}

impl InternalPage {
    /// The page a `mado://<page>` URL names.
    pub fn of_url(url: &str) -> Option<Self> {
        let rest = url.strip_prefix("mado://")?;
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        Some(match host {
            "newtab" => Self::NewTab,
            "history" => Self::History,
            "downloads" => Self::Downloads,
            "bookmarks" => Self::Bookmarks,
            "about" => Self::About,
            "credits" => Self::Credits,
            "extensions" => Self::Extensions,
            "passwords" => Self::Passwords,
            _ => return None,
        })
    }

    /// Whether this page may send `cmd` (its allow-list). `state.get` is
    /// answered with only the fields the page reads and its own tab and
    /// window; `nav.open` always acts from the page's own tab.
    pub fn allows(self, cmd: &Command) -> bool {
        use Command as C;
        let opens = matches!(cmd, C::NavOpen { .. } | C::StateGet);
        match self {
            Self::NewTab => {
                opens
                    || matches!(
                        cmd,
                        C::NavSuggest { .. }
                            | C::BookmarkTree { .. }
                            | C::BookmarkRecent { .. }
                            | C::HistoryQuery { .. }
                    )
            }
            Self::History => opens || cmd.family() == CommandFamily::History,
            Self::Downloads => opens || cmd.family() == CommandFamily::Download,
            Self::Bookmarks => {
                opens
                    || (cmd.family() == CommandFamily::Bookmark
                        && !matches!(cmd, C::BookmarkExport { .. } | C::BookmarkImport { .. }))
            }
            Self::About => {
                matches!(cmd, C::UpdateCheck | C::UpdateRelaunch | C::StateGet)
                    || matches!(cmd, C::AppOpenSystemSettings { pane } if pane == "softwareUpdate")
            }
            Self::Credits => matches!(cmd, C::StateGet),
            Self::Extensions => matches!(
                cmd,
                C::StateGet
                    | C::ExtensionList
                    | C::ExtensionOpenOptions { .. }
                    | C::AppOpenSettings { .. }
            ),
            Self::Passwords => matches!(cmd, C::StateGet | C::AppOpenSettings { .. }),
        }
    }
}

/// Who sent a command. The app sets it from the transport (the calling
/// webview's label, the control client), never from the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Origin {
    Ui {
        window: WindowId,
    },
    Overlay {
        window: WindowId,
    },
    Settings,
    Control {
        client: ClientId,
    },
    /// A control client with the `full` scope (spec 14 §2.1): the CLI,
    /// treated as the settings window for protection checks.
    FullControl {
        client: ClientId,
    },
    Agent {
        task: TaskId,
    },
    /// A `mado://` page in a tab, through its `internal` page script.
    InternalPage {
        tab: TabId,
        page: InternalPage,
    },
    System,
}

impl Origin {
    /// Control clients and agents may not touch protected settings or
    /// commands (spec 06, protected settings).
    pub fn is_user(&self) -> bool {
        // Internal pages are web content: what they may do is their
        // allow-list, and protected commands are never on it (01 §4.4).
        !matches!(
            self,
            Self::Control { .. } | Self::Agent { .. } | Self::InternalPage { .. }
        )
    }
}

/// What `mado_invoke` receives from a webview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Request {
    pub id: String,
    pub cmd: Command,
}

/// What `mado_invoke` returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase")]
#[ts(export)]
pub enum Response {
    Ok {
        id: String,
        #[ts(type = "unknown")]
        value: Value,
    },
    Err {
        id: String,
        error: MadoError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_tagged_with_their_name() {
        let cmd = Command::LaunchForward(LaunchRequest {
            urls: vec!["https://example.com/".into()],
            files: vec![],
            new_window: true,
            private: false,
            source: LaunchSource::Cli,
            control: None,
        });
        let json = serde_json::to_value(&cmd).expect("serialises");
        assert_eq!(json["type"], cmd.name());
        assert_eq!(json["newWindow"], true);
        assert_eq!(
            serde_json::from_value::<Command>(json).expect("parses"),
            cmd
        );
        assert_eq!(
            serde_json::to_value(Command::StateGet).expect("serialises"),
            serde_json::json!({ "type": "state.get" })
        );
    }

    #[test]
    fn every_command_name_starts_with_its_family() {
        for cmd in [
            Command::StateGet,
            Command::LaunchForward(LaunchRequest {
                urls: vec![],
                files: vec![],
                new_window: false,
                private: false,
                source: LaunchSource::Dock,
                control: None,
            }),
            Command::SettingsGet { keys: None },
            Command::SettingsSet {
                key: "a.b".into(),
                value: Value::Bool(true),
                token: None,
            },
            Command::SettingsReset(ResetTarget::All { all: true }),
            Command::SettingsUndo { log_id: None },
            Command::SettingsSearch { query: "x".into() },
            Command::SettingsExport {
                path: "/tmp/x".into(),
            },
            Command::SettingsImport {
                path: "/tmp/x".into(),
            },
        ] {
            let json = serde_json::to_value(&cmd).expect("serialises");
            assert_eq!(json["type"], cmd.name());
            assert_eq!(
                serde_json::from_value::<Command>(json).expect("parses"),
                cmd
            );
            assert!(
                cmd.name()
                    .starts_with(&format!("{}.", cmd.family().prefix()))
            );
        }
    }

    #[test]
    fn unknown_commands_are_refused() {
        assert!(
            serde_json::from_value::<Command>(serde_json::json!({ "type": "tab.nuke" })).is_err()
        );
    }

    #[test]
    fn control_and_agents_are_not_the_user() {
        assert!(Origin::Settings.is_user());
        assert!(
            !Origin::Control {
                client: ClientId::new()
            }
            .is_user()
        );
        assert!(
            !Origin::Agent {
                task: TaskId::new()
            }
            .is_user()
        );
    }
}
