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
                | Self::BookmarkTree { .. }
                | Self::BookmarkSearch { .. }
                | Self::BookmarkRecent { .. }
                | Self::BookmarkOfUrl { .. }
        )
    }
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

/// Who sent a command. The app sets it from the transport (the calling
/// webview's label, the control client), never from the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Origin {
    Ui { window: WindowId },
    Overlay { window: WindowId },
    Settings,
    Control { client: ClientId },
    Agent { task: TaskId },
    System,
}

impl Origin {
    /// Control clients and agents may not touch protected settings or
    /// commands (spec 06, protected settings).
    pub fn is_user(&self) -> bool {
        !matches!(self, Self::Control { .. } | Self::Agent { .. })
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
