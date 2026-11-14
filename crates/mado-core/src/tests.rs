//! The core end to end over the fake engine (M1-T06, Done when): random
//! commands, with a mirror that applies every patch in order the way the UI
//! does, must end equal to the core's own document.

use mado_types::command::{Command, Origin, ResetTarget};
use mado_types::state::Bounds;
use mado_types::{ClientId, ErrorCode, MadoError, Patch, WindowId};
use serde_json::{Value, json};

use crate::testing::TestCore;
use crate::{Input, Internal};

fn ask(core: &TestCore, origin: Origin, cmd: Command) -> Result<Value, MadoError> {
    core.handle
        .command(origin, cmd)
        .blocking_recv()
        .expect("the core replies")
}

/// The UI side: applies patches strictly in order (spec 01 §3.1).
struct Mirror {
    doc: Value,
    rev: u64,
    seen: usize,
}

impl Mirror {
    fn catch_up(&mut self, patches: &[Patch]) {
        for patch in &patches[self.seen..] {
            assert_eq!(
                patch.rev,
                self.rev + 1,
                "patches arrive in order with no gap"
            );
            let ops = serde_json::to_value(&patch.ops).expect("ops serialise");
            let ops: json_patch::Patch = serde_json::from_value(ops).expect("valid RFC 6902");
            json_patch::patch(&mut self.doc, &ops).expect("applies");
            self.rev = patch.rev;
            self.doc["rev"] = json!(patch.rev);
        }
        self.seen = patches.len();
    }
}

/// A deterministic generator, so a failure reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap_or(1)).unwrap_or(0)
    }
}

fn random_value(rng: &mut Rng, def: &mado_settings::SettingDef) -> Value {
    use mado_settings::types::SettingType;
    match rng.below(6) {
        0 => json!("nonsense"),
        1 => def.default_value(),
        _ => match def.ty {
            SettingType::Bool => json!(rng.below(2) == 0),
            SettingType::Enum(o) => json!(o[rng.below(o.len())]),
            SettingType::Int { min, max, .. } | SettingType::Duration { min, max } => {
                json!(if rng.below(2) == 0 { min } else { max })
            }
            SettingType::Float { min, max } => json!(if rng.below(2) == 0 { min } else { max }),
            SettingType::List(_) => json!([]),
            SettingType::Url => json!("https://example.com/"),
            SettingType::Path => json!("/tmp"),
            SettingType::Json { .. } => json!(rng.below(1000)),
            SettingType::String { .. } | SettingType::Shortcut => json!(""),
        },
    }
}

#[test]
fn ten_thousand_random_commands_keep_the_ui_mirror_in_sync() {
    let core = TestCore::start();
    core.handle.send(Input::Internal(Internal::OpenWindow {
        bounds: Bounds {
            width: 1200.0,
            height: 800.0,
            ..Bounds::default()
        },
    }));
    let first = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    let rev = first["rev"].as_u64().expect("rev");
    // The snapshot already holds every patch up to its revision.
    let mut mirror = Mirror {
        doc: first["state"].clone(),
        rev,
        seen: usize::try_from(rev).expect("small"),
    };
    let window = WindowId::new();
    let origins = [
        Origin::Settings,
        Origin::Ui { window },
        Origin::Control {
            client: ClientId::new(),
        },
        Origin::System,
    ];
    let sections = ["general", "tabs", "privacy", "appearance", "nope"];
    let registry = mado_settings::REGISTRY;
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut ok, mut refused) = (0, 0);
    for _ in 0..10_000 {
        let origin = origins[rng.below(origins.len())];
        let def = &registry[rng.below(registry.len())];
        let cmd = match rng.below(10) {
            0..=5 => Command::SettingsSet {
                key: def.key.into(),
                value: random_value(&mut rng, def),
                token: None,
            },
            6 => Command::SettingsReset(ResetTarget::Key {
                key: def.key.into(),
            }),
            7 => Command::SettingsReset(ResetTarget::Section {
                section: sections[rng.below(sections.len())].into(),
            }),
            8 => Command::SettingsUndo { log_id: None },
            _ => Command::StateGet,
        };
        match ask(&core, origin, cmd) {
            Ok(_) => ok += 1,
            Err(_) => refused += 1,
        }
        mirror.catch_up(&core.sent.lock().expect("lock").patches);
    }
    let last = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    mirror.catch_up(&core.sent.lock().expect("lock").patches);
    assert_eq!(mirror.rev, last["rev"].as_u64().expect("rev"));
    assert_eq!(
        mirror.doc, last["state"],
        "the mirror equals the core's document"
    );
    assert!(
        ok > 1000 && refused > 1000,
        "both paths were exercised: {ok} ok, {refused} refused"
    );
    // The startup window reached the engine and the document.
    let engine = core.engine.lock().expect("lock");
    assert_eq!(engine.windows.len(), 1);
    assert!(
        last["state"]["windows"]
            .as_object()
            .is_some_and(|w| w.len() == 1)
    );
}

#[test]
fn a_forwarded_launch_opens_a_window_or_focuses_the_newest() {
    use mado_types::command::{LaunchRequest, LaunchSource};
    let core = TestCore::start();
    let request = |new_window| LaunchRequest {
        urls: vec![],
        files: vec![],
        new_window,
        private: false,
        source: LaunchSource::Dock,
    };
    ask(
        &core,
        Origin::System,
        Command::LaunchForward(request(false)),
    )
    .expect("forwarded");
    ask(
        &core,
        Origin::System,
        Command::LaunchForward(request(false)),
    )
    .expect("forwarded");
    ask(&core, Origin::System, Command::LaunchForward(request(true))).expect("forwarded");
    let state = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    assert_eq!(
        state["state"]["windows"].as_object().map(|w| w.len()),
        Some(2)
    );
    let log = core.engine.lock().expect("lock").log.clone();
    assert!(
        log.iter()
            .any(|(_, c)| matches!(c, mado_engine::WindowCommand::Focus))
    );
    // Only the control endpoint, after checking the secret, may forward.
    let err = ask(
        &core,
        Origin::Ui {
            window: WindowId::new(),
        },
        Command::LaunchForward(request(true)),
    )
    .expect_err("refused");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}

#[test]
fn a_protected_setting_from_a_control_client_changes_nothing() {
    let core = TestCore::start();
    let before = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    let err = ask(
        &core,
        Origin::Control {
            client: ClientId::new(),
        },
        Command::SettingsSet {
            key: "blocker.enabled".into(),
            value: json!(false),
            token: None,
        },
    )
    .expect_err("refused");
    assert_eq!(err.code, ErrorCode::Protected);
    assert_eq!(
        ask(&core, Origin::System, Command::StateGet).expect("state.get"),
        before
    );
    assert!(core.sent.lock().expect("lock").patches.is_empty());
}

#[test]
fn risky_changes_show_an_undo_notice_that_undoes_them() {
    let core = TestCore::start();
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "blocker.enabled".into(),
            value: json!(false),
            token: None,
        },
    )
    .expect("set");
    let state = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    assert_eq!(state["state"]["blocker"]["enabled"], false);
    assert_eq!(
        state["state"]["settings"]["values"]["blocker.enabled"],
        false
    );
    let notice = state["state"]["notices"][0].clone();
    assert_eq!(notice["messageKey"], "notice.settingChanged");
    let undo: Command = serde_json::from_value(notice["undo"].clone()).expect("an undo command");
    ask(&core, Origin::Settings, undo).expect("undo");
    let state = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    assert_eq!(state["state"]["blocker"]["enabled"], true);
    assert_eq!(core.sent.lock().expect("lock").notices.len(), 1);
}

#[test]
fn settings_export_and_import_through_the_core() {
    let core = TestCore::start();
    let file = core.dir.path().join("settings.json");
    ask(
        &core,
        Origin::Ui {
            window: WindowId::new(),
        },
        Command::SettingsSet {
            key: "appearance.theme".into(),
            value: json!("dark"),
            token: None,
        },
    )
    .expect("set");
    ask(
        &core,
        Origin::Settings,
        Command::SettingsExport {
            path: file.to_string_lossy().into(),
        },
    )
    .expect("export");
    ask(
        &core,
        Origin::Settings,
        Command::SettingsReset(ResetTarget::All { all: true }),
    )
    .expect("reset");
    let report = ask(
        &core,
        Origin::Settings,
        Command::SettingsImport {
            path: file.to_string_lossy().into(),
        },
    )
    .expect("import");
    assert_eq!(report["applied"], json!(["appearance.theme"]));
    let state = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    assert_eq!(
        state["state"]["settings"]["values"]["appearance.theme"],
        "dark"
    );
    let missing = ask(
        &core,
        Origin::Settings,
        Command::SettingsImport {
            path: "/no/such/file.json".into(),
        },
    )
    .expect_err("fails");
    assert_eq!(missing.code, ErrorCode::InvalidInput);
}

#[test]
fn a_panicking_handler_leaves_the_state_unchanged_and_the_core_running() {
    let core = TestCore::start();
    let before = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    core.handle.send(Input::Internal(Internal::Panic));
    let after = ask(&core, Origin::System, Command::StateGet).expect("still answers");
    assert_eq!(after, before);
    assert!(
        core.sent.lock().expect("lock").patches.is_empty(),
        "a panicking handler emits no patch"
    );
}

#[test]
fn values_written_by_the_core_reach_the_database() {
    let mut core = TestCore::start();
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "tabs.memorySaver".into(),
            value: json!("off"),
            token: None,
        },
    )
    .expect("set");
    let (done, rx) = tokio::sync::oneshot::channel();
    core.handle
        .send(Input::Internal(Internal::Shutdown { done }));
    rx.blocking_recv().expect("shut down");
    if let Some(thread) = core.thread.take() {
        thread.join().expect("the core thread ends");
    }
    let conn = rusqlite::Connection::open(core.dir.path().join("profile.sqlite")).expect("open");
    let value: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'tabs.memorySaver'",
            [],
            |r| r.get(0),
        )
        .expect("stored");
    assert_eq!(value, "\"off\"");
}
