//! Commands (spec 01 §4). `Command` is one serde enum; its JSON tag is the
//! command name, `<family>.<verb>`. Each spec defines its family's members;
//! a task adds a variant together with its handler, so the enum holds only
//! commands that do something.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::MadoError;
use crate::ids::{ClientId, TaskId, WindowId};

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
            Self::WindowReportContentRect { .. } => CommandFamily::Window,
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
        }
    }

    /// Queries return data and never change state, so they may run on the
    /// read pool (spec 01 §4.3).
    pub fn is_query(&self) -> bool {
        matches!(
            self,
            Self::StateGet | Self::SettingsGet { .. } | Self::SettingsSearch { .. }
        )
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
