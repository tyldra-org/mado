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
    /// Shows a page's notification through macOS (spec 07 §1.1).
    fn show_notification(&mut self, _id: &str, _title: &str, _body: &str) {}
    /// Opens a macOS privacy pane (spec 07 §4).
    fn open_system_settings(&mut self, _pane: &str) {}
    /// Opens a private (`guest: false`) or guest window in a temporary
    /// profile's process, with `urls` (spec 12 §1.6).
    fn open_temporary(&mut self, _guest: bool, _urls: &[String]) {}
    /// The running profile's name, colour or avatar changed (Dock badge).
    fn profile_changed(&mut self, _info: &mado_types::state::ProfileInfo) {}
    /// Puts text on the pasteboard (context menu copies, spec 07 §8).
    fn copy_text(&mut self, _text: &str) {}
    /// Opens a link in the app that handles its scheme (spec 07 §6).
    fn launch_app_link(&mut self, _url: &str) {}
    /// Opens a web page in Safari (site engine rules, spec 05 §9, §11).
    fn open_in_safari(&mut self, _url: &str) {}
    /// The core decided to quit (spec 04 §9): the app exits, running the
    /// shutdown that writes the clean-quit marker.
    fn quit(&mut self) {}
    /// Opens the settings window (spec 06 §9).
    fn open_settings(&mut self) {}
    /// A menu command the window's UI carries out itself: `openLocation`,
    /// `find`, `findNext`, `findPrevious`, `bookmarkTab`,
    /// `bookmarkAllTabs` (spec 01 §5.3, §5.4).
    fn window_action(&mut self, _window: mado_types::WindowId, _action: &str) {}
    /// Enabled and checked per menu command, after a change (spec 01 §5.4).
    fn menu_state(&mut self, _state: &std::collections::BTreeMap<String, (bool, bool)>) {}
    /// The key equivalent of each menu command, `""` for none (spec 06 §8).
    fn shortcuts(&mut self, _keys: &std::collections::BTreeMap<String, String>) {}
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

/// The app's profiles (spec 12 §1). They live in the app database and in
/// other processes, so the app owns them; the core calls these off its
/// thread, except `route`, which must be quick.
pub trait Profiles: Send + Sync {
    /// `[{ id, name, color, avatar, current, running }]`.
    fn list(&self) -> Result<serde_json::Value, mado_types::MadoError>;
    /// Returns `{ id }` after starting the new profile's process.
    fn create(
        &self,
        name: &str,
        color: &str,
        avatar: &str,
    ) -> Result<serde_json::Value, mado_types::MadoError>;
    fn update(
        &self,
        id: mado_types::ProfileId,
        name: Option<String>,
        color: Option<String>,
        avatar: Option<String>,
    ) -> Result<mado_types::state::ProfileInfo, mado_types::MadoError>;
    /// Quits the profile's process and deletes its data (spec 12 §1.4).
    fn delete(&self, id: mado_types::ProfileId) -> Result<(), mado_types::MadoError>;
    fn open(&self, id: mado_types::ProfileId) -> Result<(), mado_types::MadoError>;
    fn link_rules(&self) -> Result<Vec<mado_types::command::LinkRule>, mado_types::MadoError>;
    fn set_link_rules(
        &self,
        rules: Vec<mado_types::command::LinkRule>,
    ) -> Result<(), mado_types::MadoError>;
    /// The profile a link from another app belongs in, by the link rules.
    fn route(&self, url: &str) -> Option<mado_types::ProfileId>;
    /// Writes a `.madoprofile` (spec 12 §1.5).
    fn export(&self, id: mado_types::ProfileId, path: &str) -> Result<(), mado_types::MadoError>;
    /// Creates a profile from a `.madoprofile`; returns `{ id }`.
    fn import(&self, path: &str) -> Result<serde_json::Value, mado_types::MadoError>;
}

/// No profile service: tests that do not exercise profiles.
#[derive(Debug, Default)]
pub struct NoProfiles;

fn unavailable() -> mado_types::MadoError {
    mado_types::MadoError::new(
        mado_types::ErrorCode::EngineUnavailable,
        "error.profile.unavailable",
    )
}

impl Profiles for NoProfiles {
    fn list(&self) -> Result<serde_json::Value, mado_types::MadoError> {
        Err(unavailable())
    }
    fn create(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<serde_json::Value, mado_types::MadoError> {
        Err(unavailable())
    }
    fn update(
        &self,
        _: mado_types::ProfileId,
        _: Option<String>,
        _: Option<String>,
        _: Option<String>,
    ) -> Result<mado_types::state::ProfileInfo, mado_types::MadoError> {
        Err(unavailable())
    }
    fn delete(&self, _: mado_types::ProfileId) -> Result<(), mado_types::MadoError> {
        Err(unavailable())
    }
    fn open(&self, _: mado_types::ProfileId) -> Result<(), mado_types::MadoError> {
        Err(unavailable())
    }
    fn link_rules(&self) -> Result<Vec<mado_types::command::LinkRule>, mado_types::MadoError> {
        Err(unavailable())
    }
    fn set_link_rules(
        &self,
        _: Vec<mado_types::command::LinkRule>,
    ) -> Result<(), mado_types::MadoError> {
        Err(unavailable())
    }
    fn route(&self, _: &str) -> Option<mado_types::ProfileId> {
        None
    }
    fn export(&self, _: mado_types::ProfileId, _: &str) -> Result<(), mado_types::MadoError> {
        Err(unavailable())
    }
    fn import(&self, _: &str) -> Result<serde_json::Value, mado_types::MadoError> {
        Err(unavailable())
    }
}

/// No updater: tests and builds without the plugin.
#[derive(Debug, Default)]
pub struct NoUpdates;

impl Updates for NoUpdates {
    fn check(&mut self) {}
    fn relaunch(&mut self) {}
}
