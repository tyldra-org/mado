//! Where the core sends what the UI and overlay webviews mirror (spec 01
//! §4.1): `mado://patch` and `mado://notice`. The app implements it with
//! Tauri events; tests collect into a channel.

use mado_types::Patch;
use mado_types::state::Notice;

pub trait UiSink: Send {
    fn patch(&mut self, patch: &Patch);
    fn notice(&mut self, notice: &Notice);
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
