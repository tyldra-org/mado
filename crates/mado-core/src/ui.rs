//! Where the core sends what the UI and overlay webviews mirror (spec 01
//! §4.1): `mado://patch` and `mado://notice`. The app implements it with
//! Tauri events; tests collect into a channel.

use mado_types::Patch;
use mado_types::state::Notice;

pub trait UiSink: Send {
    fn patch(&mut self, patch: &Patch);
    fn notice(&mut self, notice: &Notice);
    /// Remote search suggestions for one window's address bar, as the
    /// `mado://suggest` event (spec 05 §2).
    fn suggest(&mut self, _window: mado_types::WindowId, _payload: &serde_json::Value) {}
    /// The core decided to quit (spec 04 §9): the app exits, running the
    /// shutdown that writes the clean-quit marker.
    fn quit(&mut self) {}
    /// Hands `url` to the process of `profile`, starting it if needed
    /// (`tab.moveToProfile`, spec 04 §2).
    fn open_in_profile(&mut self, _profile: mado_types::ProfileId, _url: &str) {}
}

/// The app's updater (spec 15 §5): the core owns `state.update` and the
/// commands; the app checks, downloads and installs.
pub trait Updates: Send {
    /// Checks the channel now; progress comes back as
    /// `Internal::UpdateStatus`.
    fn check(&mut self);
    /// Installs the ready update and relaunches.
    fn relaunch(&mut self);
}

/// No updater: tests and builds without the plugin.
#[derive(Debug, Default)]
pub struct NoUpdates;

impl Updates for NoUpdates {
    fn check(&mut self) {}
    fn relaunch(&mut self) {}
}
