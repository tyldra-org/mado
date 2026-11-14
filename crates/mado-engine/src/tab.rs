//! Tabs as the engine sees them (spec 02 §1): how a tab view is created and
//! every command the core sends to one.

use std::path::PathBuf;

use mado_types::state::Rect;
use mado_types::{TabId, WindowId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::EngineError;
use crate::reply::Callback;

/// A tab's back and forward history and current page, as the engine saves
/// it (WebKit: the `interactionState` bytes, spec 02 §4.4).
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionBlob(pub Vec<u8>);

impl std::fmt::Debug for SessionBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionBlob({} bytes)", self.0.len())
    }
}

/// What a new tab view shows first.
#[derive(Debug, Clone, PartialEq)]
pub enum TabInitial {
    Url(String),
    /// Wake a sleeping tab, or restore a session: history and current page.
    Restore(SessionBlob),
    Blank,
    /// The view WebKit already made for `window.open` (spec 02 §4.3).
    Popup(PopupRequest),
}

/// The user agent a tab view sends (spec 02 §6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UserAgent {
    /// The engine's own string, unchanged.
    #[default]
    EngineDefault,
    /// The engine's string with `Version/<version> Safari/605.1.15`
    /// appended, so sites treat Mado as Safari.
    Safari { version: String },
}

/// Everything needed to create one tab view.
#[derive(Debug, Clone, PartialEq)]
pub struct TabSpec {
    pub tab: TabId,
    pub window: WindowId,
    pub initial: TabInitial,
    pub private: bool,
    /// Agent tabs never take focus and record no history (spec 13).
    pub agent: bool,
    pub user_agent: UserAgent,
    /// 1.0 is 100%.
    pub zoom: f64,
    pub visible: bool,
    /// The page area in the window, in points. `None` until the UI reports it.
    pub frame: Option<Rect>,
    /// The site's JavaScript setting for the first navigation (spec 07 §1).
    pub javascript: bool,
}

/// Why a navigation happened. Feeds history transitions (spec 12 §2.1) and
/// link rules (spec 05 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NavKind {
    /// Typed in the address bar or chosen from its suggestions.
    Typed,
    Link,
    Bookmark,
    Reload,
    Redirect,
    FormSubmit,
    BackForward,
    /// Session restore and waking a tab.
    Restore,
    /// A link from another app.
    External,
    /// Script, or anything the engine cannot tell apart.
    #[default]
    Other,
}

/// Window features of a `window.open` call or link target.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PopupFeatures {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

impl PopupFeatures {
    /// `window.open` with a size opens a popup window; without, a tab
    /// (spec 05 §7).
    pub fn wants_window(&self) -> bool {
        self.width.is_some() || self.height.is_some()
    }
}

/// A page asked for a new window or tab (`window.open`, `target=_blank`).
#[derive(Debug, Clone, PartialEq)]
pub struct PopupRequest {
    pub opener: TabId,
    /// The new tab, already created by the adapter (WebKit must return the
    /// view synchronously).
    pub tab: TabId,
    pub url: String,
    pub features: PopupFeatures,
    /// A user activation within the last 5 seconds (spec 05 §7).
    pub user_gesture: bool,
    /// Modifier keys and button of the click that opened it, if any.
    pub modifiers: Modifiers,
}

/// Where a popup goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupDecision {
    Tab {
        foreground: bool,
    },
    Window,
    /// Safe default: the adapter destroys the view it created.
    Block,
}

/// Modifier keys and mouse button of a click (spec 05 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    pub control: bool,
    /// 0 primary, 1 middle, 2 secondary.
    pub button: u8,
}

/// The two script worlds (spec 02 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum World {
    /// `WKContentWorld.world(name: "mado")`: page code cannot see it.
    Mado,
    /// The page's own world, only where a page-visible API must be replaced.
    Page,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FindRequest {
    pub query: String,
    pub case_sensitive: bool,
    pub forward: bool,
}

/// Media keys and the media button (spec 02 §4.4 `MediaControl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
}

/// An encoded image from `Snapshot`.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Image({}x{} {:?}, {} bytes)",
            self.width,
            self.height,
            self.format,
            self.bytes.len()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

/// What the core asks of one tab view (spec 02 §1, mapped in §4.4).
pub enum TabCommand {
    Navigate {
        url: String,
        referrer: Option<String>,
        kind: NavKind,
    },
    Reload {
        bypass_cache: bool,
    },
    Stop,
    GoBack,
    GoForward,
    GoToIndex(i32),
    SetFrame(Rect),
    SetVisible(bool),
    Focus,
    SetZoom(f64),
    SetMuted(bool),
    SetMediaSuspended(bool),
    Find(FindRequest),
    FindNext {
        forward: bool,
    },
    FindClear,
    Print,
    SavePdf {
        path: PathBuf,
    },
    SaveArchive {
        path: PathBuf,
    },
    Snapshot {
        full_page: bool,
        reply: Callback<Result<Image, EngineError>>,
    },
    EvaluateInWorld {
        world: World,
        source: String,
        reply: Callback<Result<Value, EngineError>>,
    },
    PostToPage {
        world: World,
        message: Value,
    },
    SaveSession {
        reply: Callback<Result<SessionBlob, EngineError>>,
    },
    ShowDevTools,
    ShowSource,
    SetReader(bool),
    /// Stored; applies from the next navigation (spec 02 §4.4).
    SetJavaScriptEnabled(bool),
    SetImagesEnabled(bool),
    ExitFullscreen,
    MediaControl(MediaAction),
}

impl TabCommand {
    /// The variant's name, for logs that must not carry URLs or page data.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Navigate { .. } => "navigate",
            Self::Reload { .. } => "reload",
            Self::Stop => "stop",
            Self::GoBack => "goBack",
            Self::GoForward => "goForward",
            Self::GoToIndex(_) => "goToIndex",
            Self::SetFrame(_) => "setFrame",
            Self::SetVisible(_) => "setVisible",
            Self::Focus => "focus",
            Self::SetZoom(_) => "setZoom",
            Self::SetMuted(_) => "setMuted",
            Self::SetMediaSuspended(_) => "setMediaSuspended",
            Self::Find(_) => "find",
            Self::FindNext { .. } => "findNext",
            Self::FindClear => "findClear",
            Self::Print => "print",
            Self::SavePdf { .. } => "savePdf",
            Self::SaveArchive { .. } => "saveArchive",
            Self::Snapshot { .. } => "snapshot",
            Self::EvaluateInWorld { .. } => "evaluateInWorld",
            Self::PostToPage { .. } => "postToPage",
            Self::SaveSession { .. } => "saveSession",
            Self::ShowDevTools => "showDevTools",
            Self::ShowSource => "showSource",
            Self::SetReader(_) => "setReader",
            Self::SetJavaScriptEnabled(_) => "setJavaScriptEnabled",
            Self::SetImagesEnabled(_) => "setImagesEnabled",
            Self::ExitFullscreen => "exitFullscreen",
            Self::MediaControl(_) => "mediaControl",
        }
    }

    /// Answers a command that carries a callback with `error`, for an
    /// engine that cannot run it. Other commands are dropped.
    pub fn fail(self, error: EngineError) {
        match self {
            Self::Snapshot { reply, .. } => reply(Err(error)),
            Self::EvaluateInWorld { reply, .. } => reply(Err(error)),
            Self::SaveSession { reply } => reply(Err(error)),
            _ => {}
        }
    }
}

impl std::fmt::Debug for TabCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TabCommand::{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_failed_command_answers_its_callback() {
        let (tx, rx) = mpsc::channel();
        TabCommand::SaveSession {
            reply: Box::new(move |r| {
                let _ = tx.send(r);
            }),
        }
        .fail(EngineError::NoTab);
        assert_eq!(rx.try_recv(), Ok(Err(EngineError::NoTab)));
    }

    #[test]
    fn debug_output_never_carries_the_url() {
        let cmd = TabCommand::Navigate {
            url: "https://secret.example/".into(),
            referrer: None,
            kind: NavKind::Typed,
        };
        assert_eq!(format!("{cmd:?}"), "TabCommand::navigate");
    }

    #[test]
    fn a_sized_window_open_is_a_window() {
        assert!(
            PopupFeatures {
                width: Some(400.0),
                ..Default::default()
            }
            .wants_window()
        );
        assert!(!PopupFeatures::default().wants_window());
    }
}
