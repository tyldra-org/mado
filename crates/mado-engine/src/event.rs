//! What the engine reports to the core (spec 02 §1). Events that wait on a
//! decision carry a [`Reply`] whose safe default is listed on each variant
//! (spec 02 §4.3).

use std::path::PathBuf;

use mado_types::{TabId, WindowId};
use serde_json::Value;

use crate::caps::PermissionKind;
use crate::nav::{NavDecision, NavRequest, NetError};
use crate::reply::Reply;
use crate::tab::{NavKind, PopupDecision, PopupRequest, World};

/// The engine's ID for one navigation of one tab.
pub type NavId = u64;

/// A favicon the page declares, from the `status` script or the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconCandidate {
    pub url: String,
    /// `sizes` attribute, such as `32x32` or `any`.
    pub sizes: Option<String>,
    pub rel: String,
}

/// The security of the committed main-frame document (spec 08).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineSecurity {
    /// Loaded over a trusted TLS connection.
    pub secure: bool,
    /// Some content came over plain HTTP.
    pub mixed_content: bool,
    /// Subject summary of the leaf certificate, for the certificate viewer.
    pub certificate_subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub kind: PermissionKind,
    /// The requesting frame's origin, from the engine.
    pub origin: String,
    /// The top-level origin when the request comes from a subframe.
    pub embedder: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Grant,
    /// Safe default.
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt { default: String },
    BeforeUnload,
}

/// `alert`, `confirm`, `prompt` and `beforeunload` (spec 07 §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsDialog {
    pub kind: DialogKind,
    pub message: String,
    pub frame: FrameInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogResponse {
    /// Safe default: `false` for confirm, `null` for prompt, stay on the
    /// page for `beforeunload`.
    Dismiss,
    /// OK, Leave; with the prompt's text.
    Accept(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthChallenge {
    pub host: String,
    pub port: u16,
    pub realm: Option<String>,
    /// `basic`, `digest`, `ntlm`, `negotiate`.
    pub scheme: String,
    pub is_proxy: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub enum AuthResponse {
    /// Safe default.
    Cancel,
    Credentials {
        user: String,
        password: String,
    },
}

impl std::fmt::Debug for AuthResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancel => f.write_str("Cancel"),
            Self::Credentials { .. } => f.write_str("Credentials(..)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCertRequest {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientCertResponse {
    /// Safe default: continue without a certificate.
    Cancel,
    /// The keychain identity's persistent reference, base64.
    Identity(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertError {
    pub host: String,
    /// Engine error code (`NSURLErrorDomain` or `OSStatus`).
    pub code: i64,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertDecision {
    /// Safe default.
    Reject,
    Proceed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChooserRequest {
    pub multiple: bool,
    pub directories: bool,
    /// MIME types and extensions from the input's `accept`.
    pub accept: Vec<String>,
}

/// The engine's ID for one download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EngineDownloadId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineDownload {
    pub id: EngineDownloadId,
    pub url: String,
    pub suggested_filename: String,
    pub mime: Option<String>,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    Cancelled,
    Network(NetError),
    /// Writing the file failed.
    File(String),
    /// The engine can resume with this data (spec 12 §4).
    Interrupted {
        resume_data: Vec<u8>,
    },
}

/// The frame a message or dialog came from, as the engine reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInfo {
    pub is_main_frame: bool,
    /// The frame's security origin, from the engine, never the payload.
    pub origin: String,
    pub url: String,
}

/// What was under the pointer when the context menu opened (spec 07 §8).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextData {
    pub page_url: String,
    pub frame_url: Option<String>,
    pub link_url: Option<String>,
    pub link_text: Option<String>,
    pub image_url: Option<String>,
    pub media_url: Option<String>,
    pub media_kind: Option<MediaKind>,
    pub selection: Option<String>,
    pub editable: bool,
    pub spelling_suggestions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

/// The context menu item the user chose (spec 07 §8). Items the core runs
/// itself (open in new tab, save, search) and items the engine runs (edit
/// commands, media controls, inspect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextAction {
    OpenLinkInNewTab,
    OpenLinkInNewWindow,
    OpenLinkInPrivateWindow,
    OpenLinkInProfile(mado_types::ProfileId),
    OpenLinkInSafari,
    SaveLinkAs,
    CopyLink,
    CopyCleanLink,
    CopyLinkAsMarkdown,
    OpenImageInNewTab,
    SaveImageAs,
    CopyImage,
    CopyImageAddress,
    AddToPhotos,
    SearchImage,
    MediaPlayPause,
    MediaMute,
    MediaLoop,
    MediaShowControls,
    MediaPictureInPicture,
    SaveVideoAs,
    Copy,
    SearchSelection,
    LookUp,
    Speak,
    CopyLinkToHighlight,
    Undo,
    Redo,
    Cut,
    Paste,
    PasteAndMatchStyle,
    SelectAll,
    Spelling(String),
    EmojiAndSymbols,
    Back,
    Forward,
    Reload,
    SavePageAs,
    Print,
    CopyPageAsMarkdown,
    Reader,
    ViewSource,
    Inspect,
}

/// Why a tab's web content process went away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoneReason {
    Crashed,
    /// The system reclaimed it under memory pressure.
    Killed,
}

/// Everything the engine reports.
#[derive(Debug)]
pub enum EngineEvent {
    /// `WindowCommand::Create` finished: the window and its UI and overlay
    /// webviews exist.
    WindowCreated {
        window: WindowId,
    },
    /// The user closed the window with its close button.
    WindowCloseRequested {
        window: WindowId,
    },
    TabCreated {
        tab: TabId,
    },
    NavigationStarted {
        tab: TabId,
        url: String,
        nav_id: NavId,
        is_main_frame: bool,
        user_gesture: bool,
        kind: NavKind,
    },
    NavigationRedirected {
        tab: TabId,
        nav_id: NavId,
        url: String,
    },
    NavigationCommitted {
        tab: TabId,
        nav_id: NavId,
        url: String,
        is_same_document: bool,
    },
    NavigationFinished {
        tab: TabId,
        nav_id: NavId,
    },
    NavigationFailed {
        tab: TabId,
        nav_id: NavId,
        url: String,
        error: NetError,
    },
    TitleChanged {
        tab: TabId,
        title: String,
    },
    UrlChanged {
        tab: TabId,
        url: String,
    },
    FaviconChanged {
        tab: TabId,
        icons: Vec<IconCandidate>,
    },
    ProgressChanged {
        tab: TabId,
        progress: f64,
    },
    HistoryStateChanged {
        tab: TabId,
        can_go_back: bool,
        can_go_forward: bool,
    },
    SecurityChanged {
        tab: TabId,
        state: EngineSecurity,
    },
    AudioChanged {
        tab: TabId,
        playing: bool,
    },
    CaptureChanged {
        tab: TabId,
        camera: bool,
        microphone: bool,
        screen: bool,
    },
    FullscreenChanged {
        tab: TabId,
        fullscreen: bool,
    },
    HoverLink {
        tab: TabId,
        url: Option<String>,
    },
    /// Default: `PopupDecision::Block`.
    NewWindowRequested {
        tab: TabId,
        request: PopupRequest,
        reply: Reply<PopupDecision>,
    },
    /// Default: `NavDecision::Cancel`.
    DecidePolicy {
        tab: TabId,
        request: NavRequest,
        reply: Reply<NavDecision>,
    },
    /// Default: `PermissionDecision::Deny`.
    PermissionRequested {
        tab: TabId,
        request: PermissionRequest,
        reply: Reply<PermissionDecision>,
    },
    /// Default: `DialogResponse::Dismiss`.
    DialogRequested {
        tab: TabId,
        dialog: JsDialog,
        reply: Reply<DialogResponse>,
    },
    /// Default: `AuthResponse::Cancel`.
    AuthRequested {
        tab: TabId,
        challenge: AuthChallenge,
        reply: Reply<AuthResponse>,
    },
    /// Default: `ClientCertResponse::Cancel`.
    ClientCertificateRequested {
        tab: TabId,
        request: ClientCertRequest,
        reply: Reply<ClientCertResponse>,
    },
    /// Default: `CertDecision::Reject`.
    CertificateError {
        tab: TabId,
        url: String,
        error: CertError,
        reply: Reply<CertDecision>,
    },
    /// Default: no files.
    FileChooser {
        tab: TabId,
        request: FileChooserRequest,
        reply: Reply<Vec<PathBuf>>,
    },
    DownloadStarted {
        tab: Option<TabId>,
        download: EngineDownload,
    },
    DownloadProgress {
        id: EngineDownloadId,
        received: u64,
        total: Option<u64>,
    },
    DownloadFinished {
        id: EngineDownloadId,
        result: Result<PathBuf, DownloadError>,
    },
    /// Default: `None` (the menu closes with nothing chosen).
    ContextMenu {
        tab: TabId,
        context: ContextData,
        reply: Reply<Option<ContextAction>>,
    },
    /// Raw, unvalidated: the core validates it with `page::validate`.
    PageMessage {
        tab: TabId,
        world: World,
        frame: FrameInfo,
        message: Value,
    },
    ProcessGone {
        tab: TabId,
        reason: GoneReason,
    },
    Unresponsive {
        tab: TabId,
    },
    Responsive {
        tab: TabId,
    },
    /// `window.close()` from the page.
    CloseRequested {
        tab: TabId,
    },
    FindResult {
        tab: TabId,
        matches: Option<u32>,
        active: Option<u32>,
    },
}

impl EngineEvent {
    /// The variant's name, for logs that must not carry URLs or page data.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WindowCreated { .. } => "windowCreated",
            Self::WindowCloseRequested { .. } => "windowCloseRequested",
            Self::TabCreated { .. } => "tabCreated",
            Self::NavigationStarted { .. } => "navigationStarted",
            Self::NavigationRedirected { .. } => "navigationRedirected",
            Self::NavigationCommitted { .. } => "navigationCommitted",
            Self::NavigationFinished { .. } => "navigationFinished",
            Self::NavigationFailed { .. } => "navigationFailed",
            Self::TitleChanged { .. } => "titleChanged",
            Self::UrlChanged { .. } => "urlChanged",
            Self::FaviconChanged { .. } => "faviconChanged",
            Self::ProgressChanged { .. } => "progressChanged",
            Self::HistoryStateChanged { .. } => "historyStateChanged",
            Self::SecurityChanged { .. } => "securityChanged",
            Self::AudioChanged { .. } => "audioChanged",
            Self::CaptureChanged { .. } => "captureChanged",
            Self::FullscreenChanged { .. } => "fullscreenChanged",
            Self::HoverLink { .. } => "hoverLink",
            Self::NewWindowRequested { .. } => "newWindowRequested",
            Self::DecidePolicy { .. } => "decidePolicy",
            Self::PermissionRequested { .. } => "permissionRequested",
            Self::DialogRequested { .. } => "dialogRequested",
            Self::AuthRequested { .. } => "authRequested",
            Self::ClientCertificateRequested { .. } => "clientCertificateRequested",
            Self::CertificateError { .. } => "certificateError",
            Self::FileChooser { .. } => "fileChooser",
            Self::DownloadStarted { .. } => "downloadStarted",
            Self::DownloadProgress { .. } => "downloadProgress",
            Self::DownloadFinished { .. } => "downloadFinished",
            Self::ContextMenu { .. } => "contextMenu",
            Self::PageMessage { .. } => "pageMessage",
            Self::ProcessGone { .. } => "processGone",
            Self::Unresponsive { .. } => "unresponsive",
            Self::Responsive { .. } => "responsive",
            Self::CloseRequested { .. } => "closeRequested",
            Self::FindResult { .. } => "findResult",
        }
    }
}
