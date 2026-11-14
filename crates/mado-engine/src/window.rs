//! Window commands (spec 02 §1). The main thread owns window creation:
//! `Create` builds the Tauri window with the UI webview (`ui-<WindowId>`)
//! and the overlay webview (`overlay-<WindowId>`), then reports the window
//! to the core.

use mado_types::WindowId;
use mado_types::state::{Bounds, Rect, WindowKind, WindowStateKind};

/// What the core asks of one window. Sent to the main thread.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowCommand {
    Create {
        window: WindowId,
        bounds: Bounds,
        kind: WindowKind,
        private: bool,
    },
    Close,
    Focus,
    SetBounds(Bounds),
    SetState(WindowStateKind),
    /// Shows the overlay webview over `rect`, in the UI's CSS pixels.
    ShowOverlay {
        rect: Rect,
    },
    HideOverlay,
    /// Profile name and active tab title, for the OS window list.
    SetTitle(String),
}

/// The webview labels the app gives a window's webviews. The app derives a
/// command's origin from the calling webview's label (spec 01 §4.1).
pub fn ui_label(window: WindowId) -> String {
    format!("ui-{window}")
}

pub fn overlay_label(window: WindowId) -> String {
    format!("overlay-{window}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_carry_the_window_id() {
        let w = WindowId::new();
        assert_eq!(ui_label(w), format!("ui-{w}"));
        assert_eq!(overlay_label(w), format!("overlay-{w}"));
    }
}
