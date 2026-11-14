//! Where the core sends what the UI and overlay webviews mirror (spec 01
//! §4.1): `mado://patch` and `mado://notice`. The app implements it with
//! Tauri events; tests collect into a channel.

use mado_types::Patch;
use mado_types::state::Notice;

pub trait UiSink: Send {
    fn patch(&mut self, patch: &Patch);
    fn notice(&mut self, notice: &Notice);
}
