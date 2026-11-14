//! The state document (spec 01 §3): the tree the core owns and every UI and
//! overlay webview mirrors. Large or private data (history, bookmarks, vault
//! items) is not here; query commands fetch it.
//!
//! This file is the source of truth; spec 01 §3's listing follows it.

use std::collections::BTreeMap;

use mado_types_derive::Patchable;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::command::Command;
use crate::ids::{DownloadId, ExtensionId, GroupId, ProfileId, PromptId, TabId, TaskId, WindowId};

/// A setting key such as `tabs.memorySaver` (spec 06 §8).
pub type SettingKey = String;
/// A feature key from the Features page (spec 06 §6).
pub type FeatureKey = String;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct State {
    /// The revision; the UI sets it from each patch's `rev`.
    #[patch(skip)]
    pub rev: u64,
    #[patch(nested)]
    pub profile: ProfileInfo,
    #[patch(map)]
    pub windows: BTreeMap<WindowId, WindowState>,
    #[patch(map)]
    pub tabs: BTreeMap<TabId, TabState>,
    #[patch(map)]
    pub groups: BTreeMap<GroupId, GroupState>,
    pub saved_groups: Vec<SavedGroupSummary>,
    #[patch(map)]
    pub downloads: BTreeMap<DownloadId, DownloadState>,
    /// Permission, dialog, auth, overlap and other prompts.
    pub prompts: BTreeMap<PromptId, PromptState>,
    /// Toasts, with undo where the action allows it.
    pub notices: Vec<Notice>,
    #[patch(nested)]
    pub extensions: ExtensionsState,
    #[patch(nested)]
    pub media: MediaState,
    #[patch(nested)]
    pub blocker: BlockerState,
    #[patch(nested)]
    pub vault: VaultState,
    #[patch(nested)]
    pub ai: AiState,
    #[patch(nested)]
    pub update: UpdateState,
    #[patch(nested)]
    pub settings: SettingsState,
    #[patch(map)]
    pub features: BTreeMap<FeatureKey, bool>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileInfo {
    pub id: ProfileId,
    pub name: String,
    pub color: String,
    pub avatar: String,
    pub is_private: bool,
    pub is_guest: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A window's frame on screen, in points, and the screen it is on.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub screen: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WindowKind {
    #[default]
    Normal,
    Popup,
    Devtools,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WindowStateKind {
    #[default]
    Normal,
    Maximized,
    Fullscreen,
    Minimized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TabLayout {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SidebarMode {
    #[default]
    Expanded,
    Icons,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sidebar {
    pub width: f64,
    pub mode: SidebarMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Focus {
    #[default]
    Ui,
    Page,
    Overlay,
}

/// The find bar of a window. Counts come from the `find` page script
/// (spec 02, capabilities); `null` until it answers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindState {
    pub query: String,
    pub matches: Option<u32>,
    pub active: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OverlayKind {
    Menu,
    Popup,
    Prompt,
    CommandBar,
    FillMenu,
    ExtensionPopup,
    Approval,
}

/// What the overlay webview shows, and where.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OverlayContent {
    pub kind: OverlayKind,
    pub id: String,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WindowState {
    pub id: WindowId,
    pub kind: WindowKind,
    /// Pinned tabs first, then the rest in strip order.
    pub tab_order: Vec<TabId>,
    pub active_tab_id: Option<TabId>,
    pub bounds: Bounds,
    pub state: WindowStateKind,
    pub layout: TabLayout,
    #[patch(nested)]
    pub sidebar: Sidebar,
    /// The page area in CSS pixels, reported by the UI (spec 01 §5.2).
    pub content_rect: Option<Rect>,
    pub find_bar: Option<FindState>,
    pub focus: Focus,
    pub overlay: Option<OverlayContent>,
    /// Status bubble text.
    pub hover_link: Option<String>,
    pub is_private: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DisplayUrl {
    pub text: String,
    pub emphasis_start: u32,
    pub emphasis_end: u32,
    pub punycode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TabLife {
    #[default]
    Live,
    Loading,
    Sleeping,
    Crashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SecurityLevel {
    Secure,
    NotSecure,
    Dangerous,
    Mixed,
    Local,
    #[default]
    Internal,
}

/// The address bar's security indicator (spec 08).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SecurityState {
    pub level: SecurityLevel,
    pub certificate_error: Option<String>,
    pub http_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioState {
    pub playing: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureState {
    pub camera: bool,
    pub microphone: bool,
    pub screen: bool,
    pub location: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AgentTabStatus {
    Working,
    Waiting,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentTab {
    pub task_id: TaskId,
    pub status: AgentTabStatus,
}

/// Whether a per-site "Open in Safari" rule applies (spec 05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EngineRule {
    #[default]
    Mado,
    External,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TabState {
    pub id: TabId,
    pub window_id: WindowId,
    pub group_id: Option<GroupId>,
    pub pinned: bool,
    pub opener_id: Option<TabId>,
    pub created_at: i64,
    pub last_active_at: i64,
    pub title: String,
    pub custom_title: Option<String>,
    pub url: String,
    pub display_url: DisplayUrl,
    /// A `mado-favicon://` URL served by the app.
    pub favicon: Option<String>,
    pub life: TabLife,
    /// 0 to 1 while loading.
    pub progress: f64,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub security: SecurityState,
    #[patch(nested)]
    pub audio: AudioState,
    #[patch(nested)]
    pub capture: CaptureState,
    /// 1.0 is 100%.
    pub zoom: f64,
    pub blocked_count: u32,
    pub agent: Option<AgentTab>,
    pub has_unsaved_input: bool,
    pub reader_available: bool,
    pub reader_active: bool,
    pub engine_rule: EngineRule,
}

/// Chrome's nine group colours, in the order `group.create` picks them (spec 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GroupColor {
    #[default]
    Grey,
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
    Orange,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GroupState {
    pub id: GroupId,
    pub window_id: Option<WindowId>,
    pub title: String,
    pub color: GroupColor,
    pub collapsed: bool,
    pub saved: bool,
    pub agent_task_id: Option<TaskId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedGroupSummary {
    pub id: GroupId,
    pub title: String,
    pub color: GroupColor,
    pub tab_count: u32,
    pub open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DownloadStatus {
    #[default]
    InProgress,
    Paused,
    Complete,
    Cancelled,
    Failed,
    Dangerous,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DownloadState {
    pub id: DownloadId,
    pub filename: String,
    pub url: String,
    pub state: DownloadStatus,
    pub received: u64,
    pub total: Option<u64>,
    pub bytes_per_second: u64,
    pub danger: Option<String>,
    pub error: Option<String>,
    pub started_at: i64,
}

/// A prompt waiting for the user. Each kind is defined with its feature:
/// permission and app-link prompts in 07, extension prompts in 09, password
/// prompts in 11, approvals in 13, pairing in 14, takeover and restore in
/// 04. M1 defines none; a later task adds a variant with its spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum PromptState {}

/// A button on a notice: an `en.json` label and the command it sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NoticeAction {
    pub label_key: String,
    pub command: Command,
}

/// A toast. Text is an `en.json` key with arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Notice {
    pub id: String,
    pub message_key: String,
    #[ts(type = "Record<string, unknown>")]
    pub args: BTreeMap<String, Value>,
    pub action: Option<NoticeAction>,
    pub undo: Option<Command>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SiteAccess {
    Granted,
    OnClick,
    None,
}

/// One extension action in the toolbar, for the active tab (spec 09 §1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionButton {
    pub id: ExtensionId,
    pub name: String,
    pub icon_url: String,
    pub badge_text: String,
    pub badge_color: String,
    pub title: String,
    pub has_popup: bool,
    pub enabled: bool,
    pub site_access_for_active_tab: SiteAccess,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionsState {
    pub toolbar: Vec<ExtensionButton>,
    pub pinned: Vec<ExtensionId>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaState {
    pub playing: Vec<TabId>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlockerState {
    pub enabled: bool,
    pub lists_updated_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VaultStatus {
    #[default]
    None,
    Locked,
    Unlocked,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultState {
    pub status: VaultStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TaskStatus {
    Queued,
    Running,
    Waiting,
    Paused,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskSummary {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub step: String,
    pub group_id: Option<GroupId>,
    pub cost_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiState {
    pub enabled: bool,
    #[patch(map)]
    pub tasks: BTreeMap<TaskId, TaskSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Downloading,
    Ready,
    Error,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateState {
    pub status: UpdateStatus,
    pub version: Option<String>,
    pub security: bool,
}

/// Values of the settings marked `ui: true` (spec 06 §1), and which of
/// them differ from their default.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingsState {
    #[patch(map)]
    #[ts(type = "Record<string, unknown>")]
    pub values: BTreeMap<SettingKey, Value>,
    pub changed: Vec<SettingKey>,
}

/// The reply to `state.get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StateSnapshot {
    pub rev: u64,
    pub state: State,
}

impl State {
    /// The root handle: every setter on it records into `recorder`.
    pub fn tx<'a>(&'a mut self, recorder: &'a mut crate::patch::PatchRecorder) -> StatePatch<'a> {
        crate::patch::Patchable::patch(self, String::new(), recorder)
    }
}
