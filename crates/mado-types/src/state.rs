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
    /// Goes up whenever bookmarks change, so views that show them fetch
    /// again (bookmarks are not in the document, spec 01 §3).
    pub bookmarks_rev: u64,
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
    /// This build and the web engine under it (spec 15 §5).
    pub about: AboutState,
    /// The welcome sheet waits: import, search engine, default browser
    /// (M2-T17). Never in private or guest windows.
    pub first_run: bool,
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
    /// A private window hidden behind "Unlock with Touch ID" (spec 08 §6).
    pub locked: bool,
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
/// 04.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum PromptState {
    /// The fill menu at a form field (spec 11 §4.1), answered with
    /// `vault.fill` or dismissed with `prompt.dismiss`.
    FillMenu(FillMenuPrompt),
    /// "Allow <name> to control Mado?" (spec 14 §2), answered with
    /// `control.pairRespond`. `offerFull`: the client is the `mado` CLI and
    /// `developer.cliFullControl` is on.
    #[serde(rename_all = "camelCase")]
    ControlPairing {
        id: PromptId,
        client_name: String,
        client_version: String,
        offer_full: bool,
    },
    /// An agent's action that needs the person (spec 13 §9.3), answered
    /// with `ai.approval.respond`.
    #[serde(rename_all = "camelCase")]
    AiApproval {
        id: PromptId,
        window_id: Option<WindowId>,
        tab_id: Option<TabId>,
        task_id: crate::TaskId,
        /// `pay`, `delete`, `send`, `leaveScope`…
        action: String,
        /// The action in plain words.
        detail: String,
        site: String,
        /// For payments, the total read from the page.
        amount: Option<String>,
        allow_for_task: bool,
    },
    /// A page command's result card (spec 13 §6), answered with
    /// `ai.command.respond`. `text` grows while `done` is false.
    #[serde(rename_all = "camelCase")]
    AiResult {
        id: PromptId,
        window_id: WindowId,
        tab_id: TabId,
        command_id: String,
        /// An `en.json` key for built-ins, the user's name for custom ones.
        name: String,
        text: String,
        done: bool,
        error: Option<String>,
        /// The selection came from an editable field: Replace and Insert
        /// below are offered.
        editable: bool,
        /// `<connectionId>:<modelId>`, or `onDevice`.
        model: Option<String>,
    },
    /// "Create a passkey for <rpId>?" or "Sign in to <rpId> with a passkey"
    /// (spec 11 §8), answered with `vault.passkeyRespond`.
    #[serde(rename_all = "camelCase")]
    Passkey {
        id: PromptId,
        window_id: WindowId,
        tab_id: TabId,
        /// `create` or `get`.
        mode: String,
        rp_id: String,
        /// The account being created.
        user_name: String,
        /// The passkeys to choose from (`get`).
        accounts: Vec<PasskeyAccount>,
        /// The vault is locked: unlock first.
        locked: bool,
    },
    /// "Save password for <site>?" or, with `update`, "Update password"
    /// (spec 11 §5), answered with `vault.saveRespond`. The password stays
    /// in the core.
    #[serde(rename_all = "camelCase")]
    SavePassword {
        id: PromptId,
        window_id: WindowId,
        tab_id: TabId,
        site: String,
        username: String,
        update: bool,
    },
    /// Quitting or closing a window would lose typed text (spec 04 §9).
    /// Answered with `session.confirmClose`; Show activates a listed tab.
    #[serde(rename_all = "camelCase")]
    UnsavedText {
        id: PromptId,
        /// The window being closed; `None` when quitting.
        window_id: Option<WindowId>,
        tabs: Vec<UnsavedTab>,
    },
    Unresponsive(UnresponsivePrompt),
    /// Add an extension (spec 09 §2.2), answered with
    /// `extension.confirmInstall`.
    ExtensionInstall(ExtensionInstallPrompt),
    /// An extension does what a Mado feature does (spec 09 §8), answered
    /// with `extension.overlapRespond`.
    #[serde(rename_all = "camelCase")]
    Overlap {
        id: PromptId,
        category: String,
        extension_id: ExtensionId,
        name: String,
        choices: Vec<crate::command::OverlapChoice>,
        default: crate::command::OverlapChoice,
    },
    /// An extension replaced the new tab page, search engine or startup
    /// pages (spec 09 §8), answered with `extension.takeoverRespond`.
    #[serde(rename_all = "camelCase")]
    Takeover {
        id: PromptId,
        extension_id: ExtensionId,
        name: String,
        /// `newTab`, `search` or `startup`.
        what: String,
    },
    /// A site asks for permissions (spec 07 §3). Answered with
    /// `prompt.respond` or `prompt.dismiss`; buttons work from `armedAt`.
    #[serde(rename_all = "camelCase")]
    Permission {
        id: PromptId,
        tab_id: TabId,
        origin: String,
        embedder: Option<String>,
        keys: Vec<String>,
        created_at: i64,
        armed_at: i64,
    },
    /// A link would open another app (spec 07 §6). `prompt.respond`:
    /// `allowThisTime` opens it, `allowWhileVisiting` also remembers the
    /// site and scheme, `never` cancels.
    #[serde(rename_all = "camelCase")]
    AppLink {
        id: PromptId,
        tab_id: TabId,
        origin: String,
        scheme: String,
        app_name: String,
        /// A `data:image/png` URL of the app's icon.
        app_icon: Option<String>,
        created_at: i64,
        armed_at: i64,
    },
    /// `navigator.registerProtocolHandler` (spec 07 §6): allow or block.
    #[serde(rename_all = "camelCase")]
    ProtocolHandler {
        id: PromptId,
        tab_id: TabId,
        origin: String,
        scheme: String,
        created_at: i64,
        armed_at: i64,
    },
    /// `alert`, `confirm`, `prompt` or `beforeunload` (spec 07 §7),
    /// answered with `prompt.answerDialog`. `offerSuppress`: the page
    /// showed another dialog within 5 seconds.
    #[serde(rename_all = "camelCase")]
    Dialog {
        id: PromptId,
        tab_id: TabId,
        origin: String,
        kind: DialogKind,
        message: String,
        default_text: Option<String>,
        offer_suppress: bool,
    },
    /// HTTP authentication, answered with `prompt.answerAuth`.
    #[serde(rename_all = "camelCase")]
    Auth {
        id: PromptId,
        tab_id: TabId,
        host: String,
        port: u16,
        realm: Option<String>,
        is_proxy: bool,
    },
    /// A site asks for a client certificate, answered with
    /// `prompt.answerClientCert`.
    #[serde(rename_all = "camelCase")]
    ClientCert {
        id: PromptId,
        tab_id: TabId,
        host: String,
        identities: Vec<ClientIdentity>,
    },
    /// A secure page sends a form over http (spec 07 §7):
    /// `prompt.respond` `allowThisTime` sends it, `never` does not.
    #[serde(rename_all = "camelCase")]
    InsecureForm {
        id: PromptId,
        tab_id: TabId,
        url: String,
    },
    /// The page's protected video does not play in WebKit here (spec 05
    /// §11). Answered with `nav.openExternally`.
    #[serde(rename_all = "camelCase")]
    OpenInSafari {
        id: PromptId,
        tab_id: TabId,
        host: String,
        reason: SafariReason,
    },
}

/// Why a page should open in Safari.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SafariReason {
    /// Protected video does not play (spec 05 §11).
    Drm,
    /// The checkout wants Apple Pay (spec 07 §9).
    ApplePay,
}

/// The kinds of page dialog (spec 07 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
}

/// A keychain identity offered for a client certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClientIdentity {
    pub id: String,
    pub label: String,
}

/// A tab whose page stopped answering (spec 02 §5.3). Answered with
/// `tab.answerUnresponsive`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnresponsivePrompt {
    pub id: PromptId,
    pub tab_id: TabId,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnsavedTab {
    pub tab_id: TabId,
    pub title: String,
    pub host: String,
}

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
    /// Every installed extension, in install order.
    pub installed: Vec<InstalledExtension>,
    /// Settings an extension's overlap choice or takeover set, by key: the
    /// rows say "Controlled by <name>" (spec 06 §7, 09 §8).
    pub controlled: BTreeMap<SettingKey, ExtensionId>,
    /// Store extensions an import found in another browser that are not
    /// installed, offered for reinstall (spec 09 §11, 12 §5).
    pub imported: Vec<ImportedExtension>,
}

/// An `imported_extensions` row not yet installed or dismissed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportedExtension {
    pub id: ExtensionId,
    pub name: String,
    /// The browser's `import.browser.*` key: `chrome`, `edge`, `brave`,
    /// `arc`.
    pub browser: String,
}

/// An installed extension (spec 09 §1, the `extensions` table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledExtension {
    pub id: ExtensionId,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub source: crate::command::ExtensionSource,
    /// Why it is off when Mado turned it off: `blocked`,
    /// `permissionsIncrease`.
    pub disabled_reason: Option<String>,
    pub site_access: SiteAccessMode,
    /// The sites it may read now.
    pub hosts: Vec<String>,
    /// The API permissions it has now.
    pub permissions: Vec<String>,
    /// Optional permissions and hosts it declared and may be granted.
    pub optional: Vec<String>,
    pub private_allowed: bool,
    pub has_options: bool,
    /// Its manifest `commands` and their shortcuts.
    pub shortcuts: Vec<ExtensionShortcut>,
}

/// Where an extension may run (spec 09 §6, the `site_access` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SiteAccessMode {
    /// Only after its button is clicked, on that tab (`activeTab`).
    OnClick,
    /// The hosts in `hosts`.
    #[default]
    Specific,
    All,
}

impl SiteAccessMode {
    /// The `extensions.site_access` column.
    pub fn column(self) -> &'static str {
        match self {
            Self::OnClick => "on_click",
            Self::Specific => "specific",
            Self::All => "all",
        }
    }

    pub fn from_column(s: &str) -> Self {
        match s {
            "on_click" => Self::OnClick,
            "all" => Self::All,
            _ => Self::Specific,
        }
    }
}

/// One manifest command with its shortcut (spec 09 §11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionShortcut {
    pub command: String,
    pub description: String,
    pub keys: Option<String>,
}

/// A passkey offered in the sign-in sheet (spec 11 §8.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PasskeyAccount {
    pub item_id: String,
    pub user_name: String,
    pub user_display_name: String,
}

/// The fill menu (spec 11 §4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FillMenuPrompt {
    pub id: PromptId,
    pub window_id: WindowId,
    pub tab_id: TabId,
    pub form_key: String,
    /// The form's kind: `login`, `usernameOnly`, `signup`,
    /// `changePassword`, `otp`, `card`, `address`, `identity`.
    pub form_kind: String,
    /// The field, in window CSS pixels.
    pub rect: Rect,
    /// The vault is locked: the menu offers "Unlock to fill".
    pub locked: bool,
    /// Items are still being read.
    pub loading: bool,
    pub items: Vec<FillMenuItem>,
}

/// One entry of the fill menu: no secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FillMenuItem {
    /// `<source>:<id>`, as `vault.fill` takes it (`mado:<id>`).
    pub item_ref: String,
    pub title: String,
    pub subtitle: String,
    pub kind: String,
    /// The source that fills it: `mado`, `onePassword` or `bitwarden`.
    pub source: String,
    /// Every source holding it (a duplicate shows once, 11 §6.4).
    pub sources: Vec<String>,
}

/// What the extension install prompt shows (spec 09 §2.2 step 7, §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionInstallPrompt {
    pub id: PromptId,
    /// The window it shows in.
    pub window_id: Option<WindowId>,
    pub extension_id: ExtensionId,
    pub name: String,
    pub version: String,
    /// A `data:` URL of its icon.
    pub icon: Option<String>,
    /// API permissions it gets.
    pub permissions: Vec<String>,
    /// Sites it may read and change.
    pub hosts: Vec<String>,
    pub all_sites: bool,
    /// What it may ask for later.
    pub optional: Vec<String>,
    /// Permissions Mado does not have ("<feature> isn't available in Mado
    /// yet").
    pub unavailable: Vec<String>,
    /// A required permission is unavailable and the compatibility list
    /// does not vouch for it: "Some features may not work".
    pub may_not_work: bool,
    /// Manifest version 2: "Older format".
    pub older_format: bool,
    /// Installing replaces this installed version.
    pub replaces: Option<String>,
    /// The compatibility list marks it `notSupported`: the `en.json` key
    /// of what does not work in Mado yet (spec 09 §7).
    pub limitation: Option<String>,
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
    /// The last download that changed a list, Unix milliseconds.
    pub lists_updated_at: Option<i64>,
    /// Filters in the engine in use; 0 until the first build.
    pub filters: u32,
    /// The bundled EasyList stands in: no list was downloaded yet.
    pub bundled: bool,
    /// The lists the profile uses, with their state (spec 10 §2).
    pub lists: Vec<BlockerList>,
    /// An update is running.
    pub updating: bool,
    /// Content rule lists on every tab view; 0 until WebKit compiled them
    /// or while blocking is off (spec 10 §4).
    pub rule_lists: u32,
    /// Hosts, with their subdomains, where the profile's blocker is off.
    pub off_hosts: Vec<String>,
}

/// One filter list in use (spec 10 §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BlockerList {
    pub id: String,
    /// `ads`, `trackers`, `annoyances` or `other`.
    pub kind: String,
    /// Filters in the stored copy; 0 before the first download.
    pub filters: u32,
    pub updated_at: Option<i64>,
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
    /// Connections by ID (spec 13 §2); their secrets never appear here.
    #[patch(map)]
    pub connections: BTreeMap<String, AiConnection>,
    /// Every role of spec 13 §4 with its choice and the model it uses now.
    #[patch(map)]
    pub roles: BTreeMap<String, AiRole>,
}

/// A role's choice and what it resolves to (spec 13 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiRole {
    /// `auto` or `<connectionId>:<modelId>`.
    pub model: String,
    pub effort: String,
    pub fallback: Option<String>,
    /// The model the role uses now; `None` without a ready connection.
    pub resolved: Option<String>,
}

/// Where a connection stands (spec 13 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AiConnectionStatus {
    Untested,
    /// Waiting for the provider's sign-in page (OAuth).
    Connecting,
    Testing,
    Ready,
    /// The last test or request failed with `error`.
    Error,
}

/// One AI connection (spec 13 §2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Patchable)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiConnection {
    pub id: String,
    /// `apiKey`, `oauth`, `local`, `subscriptionCli` or `onDevice`.
    pub kind: String,
    /// A provider key such as `anthropic` or `ollama`.
    pub provider: String,
    pub label: String,
    /// Lower goes first.
    pub priority: i64,
    pub status: AiConnectionStatus,
    /// `unauthorized`, `network`, `notFound`, `rateLimited`,
    /// `quotaExceeded`, `invalid`, `refused` or `timedOut`.
    pub error: Option<String>,
    /// The secret's last four characters.
    pub secret_hint: Option<String>,
    /// The server, when not the provider's own.
    pub base_url: Option<String>,
    /// How many models the last test listed.
    pub models: u32,
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

/// Mado's version and the web engine's, for About (spec 15 §5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AboutState {
    pub version: String,
    /// WebKit's version, without macOS's prefix (`625.2.8`).
    pub engine_version: String,
    /// Older than the compatibility list's `minSafeEngine`: a macOS
    /// update with web engine security fixes is waiting.
    pub engine_outdated: bool,
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
