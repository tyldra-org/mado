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
fn only_a_window_s_own_ui_reports_its_page_area() {
    let core = TestCore::start();
    core.handle.send(Input::Internal(Internal::OpenWindow {
        bounds: Bounds::default(),
    }));
    let state = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    let id: WindowId = state["state"]["windows"]
        .as_object()
        .and_then(|w| w.keys().next())
        .and_then(|k| k.parse().ok())
        .expect("a window");
    let rect = mado_types::state::Rect {
        x: 0.0,
        y: 40.0,
        width: 800.0,
        height: 560.0,
    };
    let report = |origin| {
        ask(
            &core,
            origin,
            Command::WindowReportContentRect {
                window_id: id,
                rect,
                scale: 2.0,
            },
        )
    };
    assert_eq!(
        report(Origin::Ui {
            window: WindowId::new()
        })
        .expect_err("refused")
        .code,
        ErrorCode::PermissionDenied
    );
    report(Origin::Ui { window: id }).expect("stored");
    let state = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    assert_eq!(
        state["state"]["windows"][id.to_string()]["contentRect"]["height"],
        560.0
    );
}

#[test]
fn test_hooks_drop_a_patch_fake_the_clock_and_authentication() {
    use crate::TestHook;
    let core = TestCore::start();
    core.handle
        .send(Input::Internal(Internal::Hook(TestHook::DropNextPatch)));
    for theme in ["dark", "light"] {
        ask(
            &core,
            Origin::Settings,
            Command::SettingsSet {
                key: "appearance.theme".into(),
                value: json!(theme),
                token: None,
            },
        )
        .expect("set");
    }
    let revs: Vec<u64> = core
        .sent
        .lock()
        .expect("lock")
        .patches
        .iter()
        .map(|p| p.rev)
        .collect();
    assert_eq!(revs, vec![2], "revision 1 was dropped");

    core.handle
        .send(Input::Internal(Internal::Hook(TestHook::FakeAuthSuccess)));
    core.handle
        .send(Input::Internal(Internal::Hook(TestHook::FakeClock(5_000))));
    let token = ask(
        &core,
        Origin::Settings,
        Command::AppAuthenticate {
            reason: "test".into(),
        },
    )
    .expect("faked")["token"]
        .as_str()
        .expect("a token")
        .to_owned();
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "vault.lockAfter".into(),
            value: json!(60_000),
            token: Some(token),
        },
    )
    .expect("authorised");
    let log = ask(&core, Origin::Settings, Command::StateGet).expect("state.get");
    assert_eq!(
        log["state"]["settings"]["values"]["vault.lockAfter"],
        60_000
    );
}

#[test]
fn a_ready_update_shows_a_relaunch_notice_and_relaunch_needs_one() {
    use mado_types::state::{UpdateState, UpdateStatus};
    let core = TestCore::start();
    let err = ask(&core, Origin::Settings, Command::UpdateRelaunch).expect_err("nothing ready");
    assert_eq!(err.code, ErrorCode::NotFound);
    core.handle
        .send(Input::Internal(Internal::UpdateStatus(UpdateState {
            status: UpdateStatus::Ready,
            version: Some("0.1.0".into()),
            security: false,
        })));
    let s = ask(&core, Origin::System, Command::StateGet).expect("state.get");
    assert_eq!(s["state"]["update"]["status"], "ready");
    assert_eq!(
        s["state"]["notices"][0]["action"]["command"]["type"],
        "update.relaunch"
    );
    ask(&core, Origin::Settings, Command::UpdateRelaunch).expect("relaunch accepted");
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

/// Waits until `check` holds, polling the fake engine.
fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    // Generous: CI runners are slower than a developer's Mac.
    for _ in 0..3000 {
        if check() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

/// Opens a window and returns it, with the fake engine's view of it.
fn window(core: &TestCore) -> WindowId {
    core.handle.send(Input::Internal(Internal::OpenWindow {
        bounds: Bounds::default(),
    }));
    eventually("the window", || {
        !core
            .engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .windows
            .is_empty()
    });
    *core
        .engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .windows
        .keys()
        .next()
        .expect("a window")
}

fn new_tab(core: &TestCore, window: WindowId, url: &str) -> mado_types::TabId {
    let v = ask(
        core,
        Origin::Ui { window },
        Command::TabNew {
            window_id: window,
            url: Some(url.into()),
            position: None,
            background: false,
            opener_id: None,
        },
    )
    .expect("tab.new");
    serde_json::from_value(v["tabId"].clone()).expect("a tab ID")
}

#[test]
fn the_core_decides_every_navigation_the_engine_asks_about() {
    use mado_engine::{ExternalReason, NavDecision};
    let core = TestCore::start();
    let state = || core.engine.lock().unwrap_or_else(|e| e.into_inner());
    let w = window(&core);
    let tab = new_tab(&core, w, "http://news.example/");
    // HTTPS-first: the core redirects, then allows the HTTPS load.
    eventually("the upgraded page", || {
        state()
            .tabs
            .get(&tab)
            .and_then(|t| t.url().map(str::to_owned))
            == Some("https://news.example/".into())
    });
    // An app link never navigates; mado: is refused to a web page.
    core.page
        .page_navigates(tab, "zoommtg://zoom.us/join?confno=1", true);
    core.page.page_navigates(tab, "mado://settings", true);
    let mine = || -> Vec<NavDecision> {
        state()
            .decisions
            .iter()
            .filter(|d| d.0 == tab)
            .map(|d| d.2.clone())
            .collect()
    };
    eventually("every decision", || mine().len() == 4);
    assert_eq!(
        mine(),
        [
            NavDecision::Redirect {
                url: "https://news.example/".into()
            },
            NavDecision::Allow,
            NavDecision::OpenExternal {
                url: "zoommtg://zoom.us/join?confno=1".into(),
                reason: ExternalReason::AppLink,
            },
            NavDecision::Cancel,
        ]
    );
    assert_eq!(state().tabs[&tab].history, ["https://news.example/"]);
}

#[test]
fn https_first_follows_its_setting() {
    use mado_engine::NavDecision;
    let core = TestCore::start();
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "security.httpsFirst".into(),
            value: json!("off"),
            token: None,
        },
    )
    .expect("set");
    let w = window(&core);
    let tab = new_tab(&core, w, "http://news.example/");
    let state = || core.engine.lock().unwrap_or_else(|e| e.into_inner());
    eventually("the decision", || {
        state().decisions.iter().any(|d| d.0 == tab)
    });
    let first = state()
        .decisions
        .iter()
        .find(|d| d.0 == tab)
        .map(|d| d.2.clone());
    assert_eq!(first, Some(NavDecision::Allow));
}

fn state_of(core: &TestCore) -> mado_types::state::State {
    let snap = ask(core, Origin::System, Command::StateGet).expect("state.get");
    serde_json::from_value(snap["state"].clone()).expect("a state")
}

fn startup(core: &TestCore) {
    core.handle.send(Input::Internal(Internal::Startup {
        bounds: Bounds::default(),
    }));
    eventually("the first window", || !state_of(core).windows.is_empty());
}

fn tab_ids(v: &[mado_types::TabId]) -> Vec<mado_types::TabId> {
    v.to_vec()
}

/// 04 §12 check 3: quit with 3 windows, relaunch with `newTab`, ⌘⇧T: all
/// three windows come back with their groups and pins.
#[test]
fn a_relaunch_then_reopen_closed_brings_back_every_window() {
    let core = TestCore::start();
    startup(&core);
    let first = *state_of(&core).windows.keys().next().expect("window");
    let mut windows = vec![first];
    for _ in 0..2 {
        let v = ask(
            &core,
            Origin::System,
            Command::WindowNew {
                private: false,
                urls: vec!["https://a.example/".into()],
            },
        )
        .expect("window.new");
        windows.push(serde_json::from_value(v["windowId"].clone()).expect("id"));
    }
    let mut tabs = Vec::new();
    for (i, w) in windows.iter().enumerate() {
        for n in 0..3 {
            tabs.push((i, new_tab(&core, *w, &format!("https://w{i}t{n}.example/"))));
        }
    }
    let pick = |w: usize, n: usize| {
        tabs.iter()
            .filter(|(i, _)| *i == w)
            .nth(n)
            .map(|t| t.1)
            .expect("tab")
    };
    ask(
        &core,
        Origin::System,
        Command::TabPin {
            tab_ids: tab_ids(&[pick(0, 0)]),
        },
    )
    .expect("pin");
    ask(
        &core,
        Origin::System,
        Command::GroupCreate {
            tab_ids: tab_ids(&[pick(1, 1), pick(1, 2)]),
            title: Some("Work".into()),
            color: None,
        },
    )
    .expect("group");
    let before = state_of(&core);
    let dir = core.stop();

    let core = TestCore::start_in(dir, true);
    startup(&core);
    let fresh = state_of(&core);
    assert_eq!(fresh.windows.len(), 1, "newTab opens one window");
    assert!(
        fresh
            .tabs
            .values()
            .any(|t| t.pinned && t.url == "https://w0t0.example/"),
        "pinned tabs return"
    );
    ask(&core, Origin::System, Command::SessionReopenClosed).expect("reopen");
    let after = state_of(&core);
    assert_eq!(after.windows.len(), 4);
    let urls = |s: &mado_types::state::State| {
        let mut u: Vec<String> = s
            .tabs
            .values()
            .filter(|t| t.url != "mado://newtab")
            .map(|t| t.url.clone())
            .collect();
        u.sort();
        u
    };
    let mut want = urls(&before);
    want.push("https://w0t0.example/".into());
    want.sort();
    assert_eq!(urls(&after), want);
    let group = after
        .groups
        .values()
        .find(|g| g.title == "Work")
        .expect("the group came back");
    let members = after
        .tabs
        .values()
        .filter(|t| t.group_id == Some(group.id))
        .count();
    assert_eq!(members, 2);
    assert_eq!(after.tabs.values().filter(|t| t.pinned).count(), 2);
}

/// 04 §12 check 4: in Maximum mode a tab playing audio, a tab with typed
/// text and a tab on an always-awake site never sleep; others do.
#[test]
fn maximum_memory_saver_never_sleeps_exempt_tabs() {
    let core = TestCore::start();
    for (key, value) in [
        ("tabs.memorySaver", json!("maximum")),
        ("tabs.keepAwakeSites", json!(["awake.example"])),
    ] {
        ask(
            &core,
            Origin::Settings,
            Command::SettingsSet {
                key: key.into(),
                value,
                token: None,
            },
        )
        .expect("set");
    }
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let audio = new_tab(&core, w, "https://music.example/");
    let typed = new_tab(&core, w, "https://form.example/");
    let awake = new_tab(&core, w, "https://www.awake.example/");
    let plain = new_tab(&core, w, "https://plain.example/");
    let front = new_tab(&core, w, "https://front.example/");
    // Loads commit asynchronously and reset typed text: let them finish.
    eventually("every tab loaded", || {
        let s = state_of(&core);
        [audio, typed, awake, plain, front]
            .iter()
            .all(|t| s.tabs[t].life == mado_types::state::TabLife::Live)
    });
    core.page.send(mado_engine::EngineEvent::AudioChanged {
        tab: audio,
        playing: true,
    });
    core.handle
        .send(Input::Internal(Internal::TestUnsavedInput { tab: typed }));
    eventually("audio and typed text", || {
        let s = state_of(&core);
        s.tabs[&audio].audio.playing && s.tabs[&typed].has_unsaved_input
    });
    core.handle
        .send(Input::Internal(Internal::MemoryPressure { critical: true }));
    eventually("the plain tab sleeps", || {
        state_of(&core).tabs[&plain].life == mado_types::state::TabLife::Sleeping
    });
    let s = state_of(&core);
    for t in [audio, typed, awake, front] {
        assert_ne!(
            s.tabs[&t].life,
            mado_types::state::TabLife::Sleeping,
            "{}",
            s.tabs[&t].url
        );
    }
    // Its view is gone; waking it restores the saved session.
    eventually("the view is gone", || {
        !core
            .engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tabs
            .contains_key(&plain)
    });
    ask(
        &core,
        Origin::Ui { window: w },
        Command::TabActivate { tab_id: plain },
    )
    .expect("activate");
    eventually("the view is back", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tabs
            .get(&plain)
            .and_then(|t| t.url().map(str::to_owned))
            == Some("https://plain.example/".into())
    });
}

/// 04 §12 check 5: a saved group closes and reopens with its tabs in order
/// and their titles, with only the first tab loading.
#[test]
fn a_saved_group_reopens_in_order_from_its_snapshots() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let a = new_tab(&core, w, "https://a.example/");
    let b = new_tab(&core, w, "https://b.example/");
    eventually("titles", || {
        state_of(&core).tabs[&b].title == "https://b.example/"
    });
    let v = ask(
        &core,
        Origin::System,
        Command::GroupCreate {
            tab_ids: vec![a, b],
            title: Some("G".into()),
            color: None,
        },
    )
    .expect("group");
    let g: mado_types::GroupId = serde_json::from_value(v["groupId"].clone()).expect("id");
    ask(&core, Origin::System, Command::GroupClose { group_id: g }).expect("close");
    let s = state_of(&core);
    assert_eq!(
        s.saved_groups.iter().find(|x| x.id == g).map(|x| x.open),
        Some(false)
    );
    let views_before = core
        .engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .tabs
        .len();
    ask(
        &core,
        Origin::System,
        Command::GroupReopen {
            group_id: g,
            window_id: w,
        },
    )
    .expect("reopen");
    let s = state_of(&core);
    let order: Vec<&str> = s.windows[&w]
        .tab_order
        .iter()
        .filter(|t| s.tabs[*t].group_id == Some(g))
        .map(|t| s.tabs[t].title.as_str())
        .collect();
    assert_eq!(order, ["https://a.example/", "https://b.example/"]);
    eventually("only the first tab has a view", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tabs
            .len()
            == views_before + 1
    });
}

/// The bookmarks bar reloads the tree whenever `bookmarksRev` goes up, so
/// reading the tree must not raise it (that looped forever); a change must.
#[test]
fn reading_the_bookmark_tree_leaves_bookmarks_rev_alone() {
    let core = TestCore::start();
    let rev = |core: &TestCore| state_of(core).bookmarks_rev;
    let tree = || Command::BookmarkTree { parent_id: None };
    ask(&core, Origin::System, tree()).expect("bookmarks.tree");
    ask(&core, Origin::System, tree()).expect("bookmarks.tree");
    assert_eq!(rev(&core), 0, "reads leave the rev alone");
    ask(
        &core,
        Origin::System,
        Command::BookmarkAdd {
            parent_id: None,
            title: "Example".into(),
            url: "https://example.com/".into(),
            index: None,
        },
    )
    .expect("bookmarks.add");
    eventually("the rev to go up", || rev(&core) == 1);
}

fn downloads_into(core: &TestCore) -> std::path::PathBuf {
    let dir = core.dir.path().join("Downloads");
    std::fs::create_dir_all(&dir).expect("dir");
    ask(
        core,
        Origin::Settings,
        Command::SettingsSet {
            key: "downloads.folder".into(),
            value: json!(dir.to_string_lossy()),
            token: None,
        },
    )
    .expect("folder");
    dir
}

fn download_state(core: &TestCore) -> mado_types::state::DownloadState {
    state_of(core)
        .downloads
        .values()
        .next()
        .cloned()
        .expect("a download")
}

/// 12 §4.2 and 08 §12 check 3: a download goes to a temporary name,
/// completes under its own name with the quarantine mark.
#[test]
fn a_download_completes_renamed_and_marked() {
    use mado_types::state::DownloadStatus;
    let core = TestCore::start();
    let dir = downloads_into(&core);
    std::fs::write(dir.join("report.pdf"), b"older").expect("clash");
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://files.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].life == mado_types::state::TabLife::Live
    });
    let id = core.page.download(
        tab,
        7,
        "https://files.example/report.pdf",
        "report.pdf",
        Some(5),
    );
    let temp = dir.join("report (1).pdf.madodownload");
    eventually("the destination", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .downloads
            .get(&id)
            .and_then(|d| d.destination.clone())
            == Some(mado_engine::event::DownloadDestination::Path(temp.clone()))
    });
    std::fs::write(&temp, b"hello").expect("bytes");
    core.page.download_progress(id, 5);
    core.page.download_done(id);
    eventually("complete", || {
        download_state(&core).state == DownloadStatus::Complete
    });
    let s = download_state(&core);
    assert_eq!(s.filename, "report (1).pdf");
    let done = dir.join("report (1).pdf");
    assert!(done.exists() && !temp.exists());
    let out = std::process::Command::new("/usr/bin/xattr")
        .arg("-p")
        .arg("com.apple.quarantine")
        .arg(&done)
        .output()
        .expect("xattr");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("0081;"));
}

/// 08 §4: a dangerous type from a site never visited before waits for
/// Keep; an insecure download is blocked until kept.
#[test]
fn dangerous_and_insecure_downloads_wait_for_keep() {
    use mado_types::state::DownloadStatus;
    let core = TestCore::start();
    let dir = downloads_into(&core);
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://new.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].life == mado_types::state::TabLife::Live
    });
    let id = core
        .page
        .download(tab, 8, "https://new.example/Setup.dmg", "Setup.dmg", None);
    let temp = dir.join("Setup.dmg.madodownload");
    eventually("the destination", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .downloads
            .get(&id)
            .and_then(|d| d.destination.clone())
            .is_some()
    });
    std::fs::write(&temp, b"app").expect("bytes");
    core.page.download_done(id);
    eventually("held", || {
        download_state(&core).state == DownloadStatus::Dangerous
    });
    let did = download_state(&core).id;
    assert!(
        ask(
            &core,
            Origin::System,
            Command::DownloadOpen { download_id: did }
        )
        .is_err(),
        "not openable while dangerous"
    );
    ask(
        &core,
        Origin::System,
        Command::DownloadKeep { download_id: did },
    )
    .expect("keep");
    assert_eq!(download_state(&core).state, DownloadStatus::Complete);
    assert!(dir.join("Setup.dmg").exists());
}

/// 12 §4.2 step 7: pause keeps resume data; resume continues under the
/// same download; quitting pauses running downloads first.
#[test]
fn pause_resume_and_quit_keep_resume_data() {
    use mado_types::state::DownloadStatus;
    let core = TestCore::start();
    downloads_into(&core);
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://big.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].life == mado_types::state::TabLife::Live
    });
    let id = core
        .page
        .download(tab, 9, "https://big.example/big.iso", "big.iso", Some(1000));
    eventually("started", || state_of(&core).downloads.len() == 1);
    core.page.download_progress(id, 400);
    let did = download_state(&core).id;
    ask(
        &core,
        Origin::System,
        Command::DownloadPause { download_id: did },
    )
    .expect("pause");
    eventually("paused", || {
        download_state(&core).state == DownloadStatus::Paused
    });
    ask(
        &core,
        Origin::System,
        Command::DownloadResume { download_id: did },
    )
    .expect("resume");
    assert_eq!(download_state(&core).state, DownloadStatus::InProgress);
    // Quit: the running download is paused with resume data first.
    core.handle
        .send(Input::Internal(Internal::QuitRequested { shortcut: false }));
    eventually("stored for the next launch", || {
        let s = download_state(&core);
        s.state == DownloadStatus::Failed && s.error.as_deref() == Some("interrupted")
    });
    let dir = core.stop();
    let core = TestCore::start_in(dir, true);
    let s = download_state(&core);
    assert_eq!(s.state, DownloadStatus::Failed);
    ask(
        &core,
        Origin::System,
        Command::DownloadResume { download_id: s.id },
    )
    .expect("resumes after relaunch");
}

/// 12 §4.2 step 4 and 8, §4.3: a file larger than the free space fails, a
/// second download from the same page is refused with a notice, and each
/// command refuses the states it does not apply to.
#[test]
fn download_commands_follow_the_state() {
    use mado_engine::event::DownloadDestination;
    use mado_types::state::DownloadStatus;
    let core = TestCore::start();
    let dir = downloads_into(&core);
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let destination = |id| {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .downloads
            .get(&id)
            .and_then(|d| d.destination.clone())
    };
    let by_name = |name: &str| {
        state_of(&core)
            .downloads
            .values()
            .find(|d| d.filename == name)
            .cloned()
    };
    let cmd = |c: Command| ask(&core, Origin::System, c);
    let code = |r: Result<Value, MadoError>| r.expect_err("refused").message_key;

    let tab = new_tab(&core, w, "https://files.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].life == mado_types::state::TabLife::Live
    });
    let first = core
        .page
        .download(tab, 21, "https://files.example/a.zip", "a.zip", Some(10));
    eventually("a destination", || destination(first).is_some());
    let second = core
        .page
        .download(tab, 22, "https://files.example/b.zip", "b.zip", Some(10));
    eventually("the second refused", || {
        destination(second) == Some(DownloadDestination::Cancel)
    });
    assert!(
        state_of(&core)
            .notices
            .iter()
            .any(|n| n.message_key == "notice.multipleDownloadsBlocked")
    );
    let a = by_name("a.zip").expect("a.zip listed");
    assert_eq!(
        code(cmd(Command::DownloadRemove { download_id: a.id })),
        "error.download.running"
    );
    assert_eq!(
        code(cmd(Command::DownloadOpen { download_id: a.id })),
        "error.download.notComplete"
    );
    assert_eq!(
        code(cmd(Command::DownloadKeep { download_id: a.id })),
        "error.download.notHeld"
    );
    cmd(Command::DownloadCancel { download_id: a.id }).expect("cancel");
    eventually("cancelled", || {
        by_name("a.zip").is_some_and(|d| d.state == DownloadStatus::Cancelled)
    });
    let tabs_before = state_of(&core).tabs.len();
    cmd(Command::DownloadOpenSource { download_id: a.id }).expect("open source");
    let st = state_of(&core);
    assert_eq!(st.tabs.len(), tabs_before + 1);
    assert!(
        st.tabs
            .values()
            .filter(|t| t.url == "https://files.example/")
            .count()
            >= 2
    );

    // Too big for any disk.
    let big = new_tab(&core, w, "https://big.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&big].life == mado_types::state::TabLife::Live
    });
    let huge = core.page.download(
        big,
        23,
        "https://big.example/huge.iso",
        "huge.iso",
        Some(u64::MAX / 2),
    );
    eventually("refused for space", || {
        destination(huge) == Some(DownloadDestination::Cancel)
    });
    let h = by_name("huge.iso").expect("huge.iso listed");
    assert_eq!(h.state, DownloadStatus::Failed);
    assert_eq!(h.error.as_deref(), Some("insufficientSpace"));

    // An http download is held; Discard deletes its file.
    let plain = new_tab(&core, w, "http://plain.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&plain].life == mado_types::state::TabLife::Live
    });
    let id = core
        .page
        .download(plain, 24, "http://plain.example/c.txt", "c.txt", None);
    eventually("a destination", || destination(id).is_some());
    let temp = dir.join("c.txt.madodownload");
    std::fs::write(&temp, b"c").expect("bytes");
    core.page.download_done(id);
    eventually("blocked", || {
        by_name("c.txt").is_some_and(|d| d.state == DownloadStatus::Blocked)
    });
    let c = by_name("c.txt").expect("c.txt listed");
    assert_eq!(c.danger.as_deref(), Some("insecure"));
    cmd(Command::DownloadDiscard { download_id: c.id }).expect("discard");
    assert!(!temp.exists() && !dir.join("c.txt").exists());

    // Clear list keeps nothing that has ended; Remove takes one.
    cmd(Command::DownloadRemove { download_id: h.id }).expect("remove");
    assert!(by_name("huge.iso").is_none());
    cmd(Command::DownloadClearList).expect("clear");
    assert!(state_of(&core).downloads.is_empty());
}

/// 05 §4: `displayUrl` shows the registrable domain, IDN as Unicode only
/// when safe, and the full URL when `addressBar.show` is `fullUrl`.
#[test]
fn display_urls_follow_the_spoof_checks_and_the_setting() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let safe = new_tab(&core, w, "https://www.xn--bcher-kva.de/regal");
    let spoof = new_tab(&core, w, "https://xn--ggle-55da.com/login");
    let shown = |tab| state_of(&core).tabs[&tab].display_url.clone();
    assert_eq!(shown(safe).text, "bücher.de");
    assert_eq!(shown(spoof).text, "xn--ggle-55da.com");
    assert!(shown(spoof).punycode);
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "addressBar.show".into(),
            value: json!("fullUrl"),
            token: None,
        },
    )
    .expect("set");
    let full = shown(safe);
    assert_eq!(full.text, "https://www.bücher.de/regal");
    assert_eq!((full.emphasis_start, full.emphasis_end), (12, 21));
}
