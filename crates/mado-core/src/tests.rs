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
        control: None,
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
    // 07 §1: the second download asks for `automaticDownloads`; its
    // buttons do nothing for the first 500 ms (07 §10 check 2).
    let prompt = || {
        state_of(&core).prompts.values().find_map(|p| match p {
            mado_types::state::PromptState::Permission { id, keys, .. }
                if keys == &["automaticDownloads".to_owned()] =>
            {
                Some(*id)
            }
            _ => None,
        })
    };
    eventually("the prompt", || prompt().is_some());
    let id = prompt().expect("a prompt");
    let respond = |decision| {
        ask(
            &core,
            Origin::Ui { window: w },
            Command::PromptRespond { id, decision },
        )
    };
    assert_eq!(
        code(respond(mado_types::command::PromptDecision::Never)),
        "error.prompt.notArmed"
    );
    core.handle
        .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
            1_800_000_000_600,
        ))));
    respond(mado_types::command::PromptDecision::Never).expect("never");
    eventually("the second refused", || {
        destination(second) == Some(DownloadDestination::Cancel)
    });
    // Blocked now: the third asks nothing and shows a notice.
    let third = core
        .page
        .download(tab, 25, "https://files.example/c.zip", "c.zip", Some(10));
    eventually("the third refused", || {
        destination(third) == Some(DownloadDestination::Cancel)
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

fn nav_open(core: &TestCore, window: WindowId, input: &str) -> Result<Value, MadoError> {
    ask(
        core,
        Origin::Ui { window },
        Command::NavOpen {
            window_id: window,
            tab_id: None,
            input: input.into(),
            disposition: mado_types::command::NavDisposition::NewForegroundTab,
            source: mado_types::command::NavSource::Typed,
            ctrl_enter: false,
        },
    )
}

/// 05 §1 and §3: `nav.open` sends each class where it belongs.
#[test]
fn nav_open_follows_the_classifier() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    assert_eq!(
        nav_open(&core, w, "rust book").expect("open"),
        json!({ "needsEngine": true })
    );
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "search.defaultEngine".into(),
            value: json!("duckduckgo"),
            token: None,
        },
    )
    .expect("pick an engine");
    let url_of = |v: Value| {
        let id: mado_types::TabId = serde_json::from_value(v["tabId"].clone()).expect("a tab");
        state_of(&core).tabs[&id].url.clone()
    };
    assert_eq!(
        url_of(nav_open(&core, w, "rust book").expect("search")),
        "https://duckduckgo.com/?q=rust+book"
    );
    assert_eq!(
        url_of(nav_open(&core, w, "bing.com lifetimes").expect("keyword")),
        "https://www.bing.com/search?q=lifetimes"
    );
    assert_eq!(
        url_of(nav_open(&core, w, "example.com/a b").expect("url")),
        "https://example.com/a%20b"
    );
    assert_eq!(
        nav_open(&core, w, "2+2").expect("calc"),
        json!({ "result": "4" })
    );
    assert_eq!(
        nav_open(&core, w, "@tabs mail").expect("scope"),
        json!({ "scope": "tabs", "query": "mail" })
    );
    let refused = nav_open(&core, w, "javascript:alert(1)").expect_err("refused");
    assert_eq!(refused.message_key, "error.nav.refused");
    let engines = ask(&core, Origin::System, Command::NavSearchEngines).expect("engines");
    let list = engines.as_array().expect("a list");
    assert_eq!(list.len(), 8);
    assert!(
        list.iter()
            .any(|e| e["id"] == "duckduckgo" && e["isDefault"] == true)
    );
}

/// 05 §2 and §12 check 2: local suggestions for a 50,000-URL history,
/// with open tabs, the typed row and the inline completion, within 30 ms.
#[test]
fn suggestions_for_a_large_history_come_back_in_time() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    {
        let mut conn =
            rusqlite::Connection::open(core.dir.path().join("profile.sqlite")).expect("open");
        let tx = conn.transaction().expect("tx");
        for i in 0..50_000 {
            tx.execute(
                "INSERT INTO urls (url, host, title, visit_count, typed_count, last_visit_at, frecency)
                 VALUES (?1, ?2, ?3, 1, 0, 0, ?4)",
                rusqlite::params![
                    format!("https://site{i}.example/page"),
                    format!("site{i}.example"),
                    format!("Page {i}"),
                    f64::from(i % 1000)
                ],
            )
            .expect("row");
        }
        tx.execute(
            "INSERT INTO urls (url, host, title, visit_count, typed_count, last_visit_at, frecency)
             VALUES ('https://github.com/', 'github.com', 'GitHub', 9, 4, 0, 2000)",
            [],
        )
        .expect("github");
        tx.commit().expect("commit");
    }
    let suggest = |input: &str| {
        ask(
            &core,
            Origin::Ui { window: w },
            Command::NavSuggest {
                window_id: w,
                input: input.into(),
                cursor: 0,
                deleted: false,
                request_id: 7,
            },
        )
        .expect("nav.suggest")
    };
    suggest("warm");
    // The fastest of three: a shared CI runner can stall any single call,
    // and the check is about the query, not the scheduler.
    let (out, took) = (0..3)
        .map(|_| {
            let started = std::time::Instant::now();
            let out = suggest("git");
            (out, started.elapsed())
        })
        .min_by_key(|(_, took)| *took)
        .expect("three runs");
    assert_eq!(out["requestId"], 7);
    assert_eq!(out["rows"][0]["url"], "https://github.com/");
    assert_eq!(out["inline"], "hub.com");
    let out = suggest("site12");
    assert!(out["rows"].as_array().is_some_and(|r| r.len() > 1));
    assert!(took < std::time::Duration::from_millis(30), "took {took:?}");
}

fn error_pages(core: &TestCore) -> Vec<(mado_types::TabId, String, String)> {
    core.engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .error_pages
        .clone()
}

fn unreachable(core: &TestCore, origin: &str, error: mado_engine::NetError) {
    core.engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unreachable
        .insert(origin.into(), error);
}

/// The error page's button for `action`, as its link.
fn action_link(html: &str, action: &str) -> Option<String> {
    let start = html.find("mado://error-action/?t=")?;
    let end = html[start..].find('"')? + start;
    let link = html[start..end].replace("&amp;", "&");
    let token = link.split("t=").nth(1)?.split('&').next()?.to_owned();
    Some(format!("mado://error-action/?t={token}&a={action}"))
}

/// 05 §12 check 4 on the fake engine: an http URL loads over HTTPS when it
/// can; a host without HTTPS shows the fallback page, whose Continue loads
/// it over http and remembers the host.
#[test]
fn https_first_upgrades_and_falls_back() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "http://example.com/");
    eventually("https", || {
        state_of(&core).tabs[&tab].url == "https://example.com/"
    });
    unreachable(
        &core,
        "https://nohttps.example",
        mado_engine::NetError::Refused,
    );
    let tab = new_tab(&core, w, "http://nohttps.example/page");
    eventually("the fallback page", || {
        error_pages(&core)
            .iter()
            .any(|(t, _, h)| *t == tab && h.contains(r#"data-kind="httpsFallback""#))
    });
    let html = error_pages(&core)
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    // A wrong token does nothing.
    core.page
        .page_navigates(tab, "mado://error-action/?t=guess&a=continue", true);
    core.page.page_navigates(
        tab,
        &action_link(&html, "continue").expect("continue"),
        true,
    );
    eventually("the http page", || {
        state_of(&core).tabs[&tab].url == "http://nohttps.example/page"
    });
    // Remembered: the next visit goes straight to http.
    let again = new_tab(&core, w, "http://nohttps.example/other");
    eventually("no upgrade", || {
        state_of(&core).tabs[&again].url == "http://nohttps.example/other"
    });
}

/// 05 §12 check 5 on the fake engine: each error kind shows its page with
/// the failed URL kept, and an error page is not a visit.
#[test]
fn every_error_kind_shows_its_page() {
    use mado_engine::NetError;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let kinds = [
        (NetError::Offline, "offline"),
        (NetError::DnsFailed, "dnsFailed"),
        (NetError::TimedOut, "timedOut"),
        (NetError::Refused, "refused"),
        (NetError::Reset, "reset"),
        (NetError::Certificate, "certificate"),
        (NetError::UnsupportedScheme, "unsupportedScheme"),
        (NetError::TooManyRedirects, "tooManyRedirects"),
        (NetError::Other(-1), "failed"),
    ];
    for (i, (error, kind)) in kinds.into_iter().enumerate() {
        let origin = format!("https://e{i}.example");
        unreachable(&core, &origin, error);
        let url = format!("{origin}/x");
        let tab = new_tab(&core, w, &url);
        eventually(kind, || {
            error_pages(&core).iter().any(|(t, u, h)| {
                *t == tab && *u == url && h.contains(&format!(r#"data-kind="{kind}""#))
            })
        });
        eventually("the URL kept", || state_of(&core).tabs[&tab].url == url);
    }
    let tab = new_tab(&core, w, "https://fine.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://fine.example/"
    });
    core.page.crash(tab);
    eventually("crashed", || {
        error_pages(&core)
            .iter()
            .any(|(t, _, h)| *t == tab && h.contains(r#"data-kind="crashed""#))
    });
    let visits = ask(
        &core,
        Origin::System,
        Command::HistoryQuery {
            text: None,
            host: None,
            from: None,
            to: None,
            limit: 100,
            cursor: None,
            utc_offset_minutes: 0,
        },
    )
    .expect("history");
    let urls = visits.to_string();
    assert!(!urls.contains("e0.example"), "an error page is not a visit");
}

/// 05 §8: a failed load behind a captive portal shows "Sign in to the
/// network", whose button opens the portal.
#[test]
fn a_captive_portal_turns_the_error_into_sign_in() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let body = "<html>Log in to the hotel Wi-Fi</html>";
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    crate::testing::CAPTIVE_PROBE
        .with_borrow_mut(|p| *p = Some(format!("http://127.0.0.1:{port}/hotspot")));
    let core = TestCore::start();
    crate::testing::CAPTIVE_PROBE.with_borrow_mut(|p| *p = None);
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    unreachable(
        &core,
        "https://news.example",
        mado_engine::NetError::TimedOut,
    );
    let tab = new_tab(&core, w, "https://news.example/");
    eventually("sign in", || {
        error_pages(&core)
            .iter()
            .any(|(t, _, h)| *t == tab && h.contains(r#"data-kind="captivePortal""#))
    });
    let html = error_pages(&core)
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    let before = state_of(&core).tabs.len();
    core.page
        .page_navigates(tab, &action_link(&html, "signIn").expect("sign in"), true);
    eventually("the portal tab", || {
        state_of(&core)
            .tabs
            .values()
            .any(|t| t.url == format!("http://127.0.0.1:{port}/hotspot"))
            && state_of(&core).tabs.len() == before + 1
    });
}

/// 05 §7 on the fake engine: ⌘-click opens a background tab after the
/// opener, ⌘⇧ a foreground one, ⇧ a new window, ⌥ downloads; the page
/// stays. A cross-site move without a gesture right after a pop-up is
/// blocked (tab-under).
#[test]
fn link_modifiers_choose_where_a_link_opens() {
    use mado_engine::NavDecision;
    use mado_engine::tab::Modifiers;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://a.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://a.example/"
    });
    let tabs_in = |w: WindowId| state_of(&core).windows[&w].tab_order.clone();
    let before = tabs_in(w).len();
    let click = |url: &str, m: Modifiers| core.page.page_clicks(tab, url, m);
    click(
        "https://b.example/",
        Modifiers {
            command: true,
            ..Modifiers::default()
        },
    );
    eventually("a background tab", || tabs_in(w).len() == before + 1);
    let st = state_of(&core);
    assert_eq!(st.windows[&w].active_tab_id, Some(tab), "background");
    let pos = tabs_in(w).iter().position(|t| *t == tab).expect("opener");
    assert_eq!(
        st.tabs[&tabs_in(w)[pos + 1]].url,
        "https://b.example/",
        "after the opener"
    );
    click(
        "https://c.example/",
        Modifiers {
            command: true,
            shift: true,
            ..Modifiers::default()
        },
    );
    eventually("a foreground tab", || {
        let st = state_of(&core);
        st.windows[&w]
            .active_tab_id
            .is_some_and(|a| st.tabs[&a].url == "https://c.example/")
    });
    let windows = state_of(&core).windows.len();
    click(
        "https://d.example/",
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    eventually("a new window", || {
        state_of(&core).windows.len() == windows + 1
    });
    click(
        "https://e.example/file.zip",
        Modifiers {
            option: true,
            ..Modifiers::default()
        },
    );
    eventually("a download", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decisions
            .iter()
            .any(|(_, u, d)| u == "https://e.example/file.zip" && *d == NavDecision::Download)
    });
    assert_eq!(
        state_of(&core).tabs[&tab].url,
        "https://a.example/",
        "the page stays"
    );
}

/// 05 §7, tab-under: right after it opens a pop-up, the opener cannot move
/// to another site without a gesture of its own; its own site is fine.
#[test]
fn a_tab_under_after_a_popup_is_blocked() {
    use mado_engine::tab::{PopupFeatures, PopupRequest};
    use mado_engine::{NavDecision, Reply};
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://news.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://news.example/"
    });
    let (tx, rx) = std::sync::mpsc::channel();
    let (reply, _expiry) = Reply::new(mado_engine::tab::PopupDecision::Block, move |d| {
        let _ = tx.send(d);
    });
    core.page
        .send(mado_engine::EngineEvent::NewWindowRequested {
            tab,
            request: PopupRequest {
                opener: tab,
                tab: mado_types::TabId::new(),
                url: "https://ads.example/".into(),
                features: PopupFeatures::default(),
                user_gesture: true,
                modifiers: mado_engine::tab::Modifiers::default(),
            },
            reply,
        });
    let decision = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a decision");
    assert_ne!(decision, mado_engine::tab::PopupDecision::Block);
    core.page
        .page_navigates(tab, "https://other.example/", false);
    core.page
        .page_navigates(tab, "https://news.example/next", false);
    eventually("the same-site move", || {
        state_of(&core).tabs[&tab].url == "https://news.example/next"
    });
    let decided = core
        .engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .decisions
        .clone();
    assert!(
        decided
            .iter()
            .any(|(_, u, d)| u == "https://other.example/" && *d == NavDecision::Cancel)
    );
}

/// 05 §11 and §9: a DRM failure asks to open the page in Safari; "Always"
/// sends it there and makes the site open in Safari from then on.
#[test]
fn a_drm_failure_offers_safari_and_always_remembers_the_site() {
    use mado_engine::page::{DrmFailed, PageMessage};
    use mado_types::command::ExternalAnswer;
    use mado_types::state::PromptState;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://tv.example/watch");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://tv.example/watch"
    });
    core.page.send(mado_engine::EngineEvent::PageMessage {
        tab,
        world: mado_engine::tab::World::Mado,
        frame: mado_engine::event::FrameInfo {
            is_main_frame: true,
            origin: "https://tv.example".into(),
            url: "https://tv.example/watch".into(),
        },
        message: PageMessage::DrmFailed(DrmFailed { key_system: None }),
    });
    eventually("the prompt", || {
        state_of(&core).prompts.values().any(|p| {
            matches!(p, PromptState::OpenInSafari { tab_id, host, .. } if *tab_id == tab && host == "tv.example")
        })
    });
    ask(
        &core,
        Origin::Ui { window: w },
        Command::NavOpenExternally {
            tab_id: tab,
            answer: ExternalAnswer::Always,
        },
    )
    .expect("answer");
    assert!(state_of(&core).prompts.is_empty());
    let safari = || {
        core.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .safari
            .clone()
    };
    assert_eq!(safari(), vec!["https://tv.example/watch".to_owned()]);
    // From now on the site opens in Safari, Mado's own navigations included.
    let _ = new_tab(&core, w, "https://www.tv.example/show");
    eventually("Safari again", || safari().len() == 2);
}

/// 05 §3: a discovered OpenSearch engine is added turned off.
#[test]
fn a_discovered_engine_is_added_inactive() {
    let core = TestCore::start();
    core.handle.send(Input::Internal(Internal::EngineDiscovered(
        mado_navigation::engines::Engine {
            id: "opensearch:docs.example".into(),
            name: "Docs".into(),
            keyword: Some("docs.example".into()),
            search_url: "https://docs.example/s?q={searchTerms}".into(),
            suggest_url: None,
        },
    )));
    let engines = ask(&core, Origin::System, Command::NavSearchEngines).expect("engines");
    let found = engines
        .as_array()
        .and_then(|l| {
            l.iter()
                .find(|e| e["id"] == "opensearch:docs.example")
                .cloned()
        })
        .expect("the engine");
    assert_eq!(found["active"], false);
    assert_eq!(found["source"], "opensearch");
}

/// Sends a permission request as the engine does; returns its answer.
fn request_permission(
    core: &TestCore,
    tab: mado_types::TabId,
    kind: mado_engine::PermissionKind,
    origin: &str,
) -> std::sync::mpsc::Receiver<mado_engine::event::PermissionDecision> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (reply, _expiry) =
        mado_engine::Reply::new(mado_engine::event::PermissionDecision::Deny, move |d| {
            let _ = tx.send(d);
        });
    core.page
        .send(mado_engine::EngineEvent::PermissionRequested {
            tab,
            request: mado_engine::event::PermissionRequest {
                kind,
                origin: origin.into(),
                embedder: None,
            },
            reply,
        });
    rx
}

fn permission_prompt(core: &TestCore, key: &str) -> Option<mado_types::PromptId> {
    state_of(core).prompts.values().find_map(|p| match p {
        mado_types::state::PromptState::Permission { id, keys, .. }
            if keys.iter().any(|k| k == key) =>
        {
            Some(*id)
        }
        _ => None,
    })
}

/// 07 §10 checks 1 to 3 on the fake engine: ask prompts, the answer
/// persists across a relaunch, block denies without asking, three
/// dismissals block, and a key macOS denied shows the infobar instead.
#[test]
fn permissions_ask_remember_and_respect_macos() {
    use mado_engine::PermissionKind;
    use mado_engine::event::PermissionDecision;
    use mado_types::command::PromptDecision;
    let wait = |rx: &std::sync::mpsc::Receiver<PermissionDecision>| {
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("an answer")
    };
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://meet.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://meet.example/"
    });
    let site = "https://meet.example";
    // Ask: a prompt that cannot be answered for 500 ms.
    let rx = request_permission(&core, tab, PermissionKind::Microphone, site);
    std::thread::sleep(std::time::Duration::from_millis(300));
    eprintln!(
        "DBG prompts={:?} answer={:?}",
        state_of(&core).prompts,
        rx.try_recv()
    );
    eventually("the prompt", || {
        permission_prompt(&core, "microphone").is_some()
    });
    let id = permission_prompt(&core, "microphone").expect("prompt");
    let respond = |id, decision| {
        ask(
            &core,
            Origin::Ui { window: w },
            Command::PromptRespond { id, decision },
        )
    };
    assert_eq!(
        respond(id, PromptDecision::AllowWhileVisiting)
            .expect_err("not armed")
            .message_key,
        "error.prompt.notArmed"
    );
    // A control client may never answer a prompt.
    let client = mado_types::ClientId::new();
    assert!(
        ask(
            &core,
            Origin::Control { client },
            Command::PromptRespond {
                id,
                decision: PromptDecision::AllowWhileVisiting
            }
        )
        .is_err()
    );
    core.handle
        .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
            1_800_000_000_600,
        ))));
    respond(id, PromptDecision::AllowWhileVisiting).expect("allow");
    assert_eq!(wait(&rx), PermissionDecision::Grant);
    // Remembered: no prompt the second time.
    assert_eq!(
        wait(&request_permission(
            &core,
            tab,
            PermissionKind::Microphone,
            site
        )),
        PermissionDecision::Grant
    );
    // Block from Site settings denies without asking.
    ask(
        &core,
        Origin::Settings,
        Command::PermissionSet {
            origin: site.into(),
            key: "notifications".into(),
            value: mado_types::command::SiteValue::Block,
            embedder: None,
        },
    )
    .expect("set");
    assert_eq!(
        wait(&request_permission(
            &core,
            tab,
            PermissionKind::Notifications,
            site
        )),
        PermissionDecision::Deny
    );
    // Three dismissals in a week block the site.
    let other = "https://ads.example";
    for _ in 0..3 {
        let rx = request_permission(&core, tab, PermissionKind::ClipboardRead, other);
        eventually("a prompt", || {
            permission_prompt(&core, "clipboardRead").is_some()
        });
        let id = permission_prompt(&core, "clipboardRead").expect("prompt");
        ask(
            &core,
            Origin::Ui { window: w },
            Command::PromptDismiss { id },
        )
        .expect("dismiss");
        assert_eq!(wait(&rx), PermissionDecision::Deny);
    }
    let list = ask(
        &core,
        Origin::Settings,
        Command::PermissionList {
            origin: Some(other.into()),
        },
    )
    .expect("list");
    assert_eq!(list[0]["value"], "block");
    assert_eq!(list[0]["source"], "default");
    // Camera, denied in macOS: no prompt, the infobar instead (check 3).
    assert_eq!(
        wait(&request_permission(
            &core,
            tab,
            PermissionKind::Camera,
            site
        )),
        PermissionDecision::Deny
    );
    assert!(permission_prompt(&core, "camera").is_none());
    assert!(
        state_of(&core)
            .notices
            .iter()
            .any(|n| n.message_key == "notice.osPermissionDenied")
    );
    // The microphone grant survives a relaunch.
    let dir = core.stop();
    let core = TestCore::start_in(dir, true);
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://meet.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://meet.example/"
    });
    assert_eq!(
        wait(&request_permission(
            &core,
            tab,
            PermissionKind::Microphone,
            site
        )),
        PermissionDecision::Grant
    );
}

/// 07 §1.1: the notifications shim hears the site's answer.
#[test]
fn the_notification_shim_follows_the_site_setting() {
    use mado_engine::page::{NotificationRequest, PageMessage};
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://news.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://news.example/"
    });
    ask(
        &core,
        Origin::Settings,
        Command::PermissionSet {
            origin: "https://news.example".into(),
            key: "notifications".into(),
            value: mado_types::command::SiteValue::Allow,
            embedder: None,
        },
    )
    .expect("set");
    core.page.send(mado_engine::EngineEvent::PageMessage {
        tab,
        world: mado_engine::tab::World::Page,
        frame: mado_engine::event::FrameInfo {
            is_main_frame: true,
            origin: "https://news.example".into(),
            url: "https://news.example/".into(),
        },
        message: PageMessage::NotificationRequest(NotificationRequest {}),
    });
    eventually("granted", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .posted
            .iter()
            .any(|(t, m)| {
                *t == tab && m["type"] == "notificationPermission" && m["permission"] == "granted"
            })
    });
}

fn app_link_prompt(core: &TestCore) -> Option<(mado_types::PromptId, String, Option<String>)> {
    state_of(core).prompts.values().find_map(|p| match p {
        mado_types::state::PromptState::AppLink {
            id,
            app_name,
            app_icon,
            ..
        } => Some((*id, app_name.clone(), app_icon.clone())),
        _ => None,
    })
}

/// 07 §6 on the fake engine: no gesture or a blocked scheme opens nothing;
/// no app gives a quiet notice; otherwise the prompt names the app, and
/// "Always" lets the site open it without asking; a fourth launch within
/// 10 seconds is refused; mailto: opens with no prompt.
#[test]
fn app_links_ask_with_the_apps_name_and_remember() {
    use mado_engine::tab::Modifiers;
    use mado_types::command::PromptDecision;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://meet.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://meet.example/"
    });
    let launched = || {
        core.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .launched
            .clone()
    };
    let zoom = "zoommtg://zoom.us/join?confno=1";
    core.page.page_navigates(tab, zoom, false);
    core.page.page_navigates(tab, "search-ms:query=x", true);
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        app_link_prompt(&core).is_none(),
        "no gesture, blocked scheme: nothing"
    );
    core.page
        .page_clicks(tab, "unknownapp://x", Modifiers::default());
    eventually("the no-app notice", || {
        state_of(&core)
            .notices
            .iter()
            .any(|n| n.message_key == "notice.noAppForLink")
    });
    core.page.page_clicks(tab, zoom, Modifiers::default());
    eventually("the prompt", || app_link_prompt(&core).is_some());
    let (id, name, icon) = app_link_prompt(&core).expect("prompt");
    assert_eq!(name, "zoom.us");
    assert_eq!(icon.as_deref(), Some("data:image/png;base64,iVBORw=="));
    core.handle
        .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
            1_800_000_000_600,
        ))));
    ask(
        &core,
        Origin::Ui { window: w },
        Command::PromptRespond {
            id,
            decision: PromptDecision::AllowWhileVisiting,
        },
    )
    .expect("open");
    assert_eq!(launched(), vec![zoom.to_owned()]);
    core.page.page_clicks(tab, zoom, Modifiers::default());
    eventually("opened without asking", || launched().len() == 2);
    assert!(app_link_prompt(&core).is_none());
    // The fourth launch from the tab within 10 seconds (the no-app link
    // counted too) is refused.
    core.page
        .page_clicks(tab, "mailto:a@b.example", Modifiers::default());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(launched().len(), 2);
    // Ten seconds later, mailto: opens with no prompt.
    core.handle
        .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
            1_800_000_011_000,
        ))));
    core.page
        .page_clicks(tab, "mailto:a@b.example", Modifiers::default());
    eventually("mail", || {
        launched().last().is_some_and(|u| u == "mailto:a@b.example")
    });
}

/// 07 §6: a registered mail handler opens mailto: links in a new tab.
#[test]
fn a_registered_handler_takes_mailto_links() {
    use mado_engine::page::{PageMessage, ProtocolHandlerRegister};
    use mado_engine::tab::Modifiers;
    use mado_types::command::PromptDecision;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://mail.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://mail.example/"
    });
    core.page.send(mado_engine::EngineEvent::PageMessage {
        tab,
        world: mado_engine::tab::World::Page,
        frame: mado_engine::event::FrameInfo {
            is_main_frame: true,
            origin: "https://mail.example".into(),
            url: "https://mail.example/".into(),
        },
        message: PageMessage::ProtocolHandlerRegister(ProtocolHandlerRegister {
            scheme: "mailto".into(),
            url: "https://mail.example/compose?to=%s".into(),
        }),
    });
    let id = || {
        state_of(&core).prompts.values().find_map(|p| match p {
            mado_types::state::PromptState::ProtocolHandler { id, scheme, .. }
                if scheme == "mailto" =>
            {
                Some(*id)
            }
            _ => None,
        })
    };
    eventually("the prompt", || id().is_some());
    core.handle
        .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
            1_800_000_000_600,
        ))));
    ask(
        &core,
        Origin::Ui { window: w },
        Command::PromptRespond {
            id: id().expect("prompt"),
            decision: PromptDecision::AllowWhileVisiting,
        },
    )
    .expect("allow");
    core.page
        .page_clicks(tab, "mailto:a@b.example", Modifiers::default());
    eventually("the compose tab", || {
        state_of(&core)
            .tabs
            .values()
            .any(|t| t.url == "https://mail.example/compose?to=mailto%3Aa%40b.example")
    });
}

fn ask_engine<T: Send + 'static>(
    default: T,
) -> (mado_engine::Reply<T>, std::sync::mpsc::Receiver<T>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let (reply, _expiry) = mado_engine::Reply::new(default, move |v| {
        let _ = tx.send(v);
    });
    (reply, rx)
}

fn frame(main: bool, origin: &str) -> mado_engine::event::FrameInfo {
    mado_engine::event::FrameInfo {
        is_main_frame: main,
        origin: origin.into(),
        url: format!("{origin}/"),
    }
}

/// 07 §7 and §10 check 5: dialogs ask in the tab, a second within 5
/// seconds offers to stop them, cross-origin frames show nothing.
#[test]
fn page_dialogs_ask_and_can_be_stopped() {
    use mado_engine::event::{DialogKind, DialogResponse, JsDialog};
    use mado_types::state::PromptState;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://app.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://app.example/"
    });
    let dialog = |kind: DialogKind, main: bool, origin: &str| {
        let (reply, rx) = ask_engine(DialogResponse::Dismiss);
        core.page.send(mado_engine::EngineEvent::DialogRequested {
            tab,
            dialog: JsDialog {
                kind,
                message: "Hello".into(),
                frame: frame(main, origin),
            },
            reply,
        });
        rx
    };
    let open = || {
        state_of(&core).prompts.values().find_map(|p| match p {
            PromptState::Dialog {
                id, offer_suppress, ..
            } => Some((*id, *offer_suppress)),
            _ => None,
        })
    };
    let answer = |id, accept, text: Option<&str>, suppress| {
        ask(
            &core,
            Origin::Ui { window: w },
            Command::PromptAnswerDialog {
                id,
                accept,
                text: text.map(str::to_owned),
                suppress,
            },
        )
        .expect("answer")
    };
    let wait = |rx: std::sync::mpsc::Receiver<DialogResponse>| {
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("an answer")
    };
    // A cross-origin frame's alert shows nothing.
    assert_eq!(
        wait(dialog(DialogKind::Alert, false, "https://ads.example")),
        DialogResponse::Dismiss
    );
    assert!(open().is_none());
    let rx = dialog(
        DialogKind::Prompt {
            default: "x".into(),
        },
        true,
        "https://app.example",
    );
    eventually("the dialog", || open().is_some());
    let (id, offer) = open().expect("dialog");
    assert!(!offer);
    answer(id, true, Some("typed"), false);
    assert_eq!(wait(rx), DialogResponse::Accept(Some("typed".into())));
    let rx = dialog(DialogKind::Confirm, true, "https://app.example");
    eventually("the second", || open().is_some());
    let (id, offer) = open().expect("dialog");
    assert!(offer, "a second dialog within 5 s offers to stop them");
    answer(id, false, None, true);
    assert_eq!(wait(rx), DialogResponse::Dismiss);
    // Stopped: answered without showing; leaving is never blocked.
    assert_eq!(
        wait(dialog(DialogKind::Alert, true, "https://app.example")),
        DialogResponse::Dismiss
    );
    assert_eq!(
        wait(dialog(
            DialogKind::BeforeUnload,
            true,
            "https://app.example"
        )),
        DialogResponse::Accept(None)
    );
    assert!(open().is_none());
}

/// 07 §7: sign-in and client certificates ask; the certificate choice is
/// kept for the session; an https form posting to http waits.
#[test]
fn sign_in_certificates_and_insecure_forms_ask() {
    use mado_engine::event::{AuthChallenge, AuthResponse, ClientCertRequest, ClientCertResponse};
    use mado_engine::{NavDecision, NavKind, NavPhase, NavRequest};
    use mado_types::state::PromptState;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://bank.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://bank.example/"
    });
    let prompt = |want: &str| {
        state_of(&core)
            .prompts
            .values()
            .find_map(|p| match (p, want) {
                (PromptState::Auth { id, .. }, "auth")
                | (PromptState::ClientCert { id, .. }, "cert")
                | (PromptState::InsecureForm { id, .. }, "form") => Some(*id),
                _ => None,
            })
    };
    let ui = Origin::Ui { window: w };
    let (reply, rx) = ask_engine(AuthResponse::Cancel);
    core.page.send(mado_engine::EngineEvent::AuthRequested {
        tab,
        challenge: AuthChallenge {
            host: "bank.example".into(),
            port: 443,
            realm: Some("Staff".into()),
            scheme: "basic".into(),
            is_proxy: false,
        },
        reply,
    });
    eventually("sign in", || prompt("auth").is_some());
    ask(
        &core,
        ui,
        Command::PromptAnswerAuth {
            id: prompt("auth").expect("auth"),
            user: Some("ada".into()),
            password: Some("pw".into()),
        },
    )
    .expect("answer");
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("an answer"),
        AuthResponse::Credentials {
            user: "ada".into(),
            password: "pw".into()
        }
    );
    let cert = || {
        let (reply, rx) = ask_engine(ClientCertResponse::Cancel);
        core.page
            .send(mado_engine::EngineEvent::ClientCertificateRequested {
                tab,
                request: ClientCertRequest {
                    host: "bank.example".into(),
                    port: 443,
                },
                reply,
            });
        rx
    };
    let rx = cert();
    eventually("the certificate sheet", || prompt("cert").is_some());
    ask(
        &core,
        ui,
        Command::PromptAnswerClientCert {
            id: prompt("cert").expect("cert"),
            identity: Some("cert-1".into()),
        },
    )
    .expect("answer");
    let identity = ClientCertResponse::Identity("cert-1".into());
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("an answer"),
        identity
    );
    assert_eq!(
        cert()
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("remembered"),
        identity
    );
    let (reply, rx) = ask_engine(NavDecision::Cancel);
    core.page.send(mado_engine::EngineEvent::DecidePolicy {
        tab,
        request: NavRequest {
            url: "http://bank.example/login".into(),
            source_url: Some("https://bank.example/".into()),
            is_main_frame: true,
            user_gesture: true,
            initiated_by_mado: false,
            kind: NavKind::FormSubmit,
            phase: NavPhase::Action {
                modifiers: mado_engine::tab::Modifiers::default(),
                method: "POST".into(),
            },
        },
        reply,
    });
    eventually("the warning", || prompt("form").is_some());
    ask(
        &core,
        ui,
        Command::PromptRespond {
            id: prompt("form").expect("form"),
            decision: mado_types::command::PromptDecision::AllowThisTime,
        },
    )
    .expect("send anyway");
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("an answer"),
        NavDecision::Allow
    );
}

/// 07 §8: the context menu items the core runs.
#[test]
fn context_menu_items_open_and_copy() {
    use mado_engine::event::{ContextAction, ContextData};
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://news.example/story");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://news.example/story"
    });
    let context = ContextData {
        page_url: "https://news.example/story".into(),
        link_url: Some("https://other.example/a?utm_source=x&id=1".into()),
        link_text: Some("Other [site]".into()),
        selection: Some("big news".into()),
        ..ContextData::default()
    };
    let choose = |action| {
        core.page.send(mado_engine::EngineEvent::ContextMenuChosen {
            tab,
            context: context.clone(),
            action,
        });
    };
    choose(ContextAction::OpenLinkInNewTab);
    eventually("a background tab", || {
        let st = state_of(&core);
        st.tabs
            .values()
            .any(|t| t.url == "https://other.example/a?utm_source=x&id=1")
            && st.windows[&w].active_tab_id == Some(tab)
    });
    choose(ContextAction::CopyCleanLink);
    choose(ContextAction::CopyLinkAsMarkdown);
    choose(ContextAction::CopyLinkToHighlight);
    let copied = || {
        core.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .copied
            .clone()
    };
    eventually("three copies", || copied().len() == 3);
    assert_eq!(
        copied(),
        vec![
            "https://other.example/a?id=1".to_owned(),
            "[Other site](https://other.example/a?utm_source=x&id=1)".to_owned(),
            "https://news.example/story#:~:text=big%20news".to_owned(),
        ]
    );
}

/// 07 §9: a checkout asking for Apple Pay gets the Safari offer, once.
#[test]
fn an_apple_pay_checkout_is_offered_safari() {
    use mado_engine::page::{ApplePayProbe, PageMessage};
    use mado_types::state::{PromptState, SafariReason};
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://shop.example/checkout");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://shop.example/checkout"
    });
    for _ in 0..2 {
        core.page.send(mado_engine::EngineEvent::PageMessage {
            tab,
            world: mado_engine::tab::World::Page,
            frame: frame(true, "https://shop.example"),
            message: PageMessage::ApplePayProbe(ApplePayProbe {}),
        });
    }
    eventually("the offer", || {
        state_of(&core).prompts.values().any(|p| {
            matches!(p, PromptState::OpenInSafari { reason: SafariReason::ApplePay, host, .. } if host == "shop.example")
        })
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(state_of(&core).prompts.len(), 1, "offered once");
}

/// 04 and 07 §1: "Mute site" goes through the permission store, so a page
/// of that site opened afterwards starts muted, in this run.
#[test]
fn a_muted_site_stays_muted_in_new_tabs() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    ask(
        &core,
        Origin::Ui { window: w },
        Command::TabMuteSite {
            host: "noisy.example".into(),
            muted: true,
        },
    )
    .expect("mute site");
    let tab = new_tab(&core, w, "https://www.noisy.example/video");
    eventually("muted", || state_of(&core).tabs[&tab].audio.muted);
}

/// Routes `*.acme.com` to one profile; everything else unavailable.
struct RoutingProfiles(mado_types::ProfileId);

impl crate::Profiles for RoutingProfiles {
    fn list(&self) -> Result<Value, MadoError> {
        Ok(json!([]))
    }
    fn create(&self, _: &str, _: &str, _: &str) -> Result<Value, MadoError> {
        Ok(json!({}))
    }
    fn update(
        &self,
        id: mado_types::ProfileId,
        name: Option<String>,
        color: Option<String>,
        _: Option<String>,
    ) -> Result<mado_types::state::ProfileInfo, MadoError> {
        Ok(mado_types::state::ProfileInfo {
            id,
            name: name.unwrap_or_default(),
            color: color.unwrap_or_default(),
            ..mado_types::state::ProfileInfo::default()
        })
    }
    fn delete(&self, _: mado_types::ProfileId) -> Result<(), MadoError> {
        Ok(())
    }
    fn open(&self, _: mado_types::ProfileId) -> Result<(), MadoError> {
        Ok(())
    }
    fn link_rules(&self) -> Result<Vec<mado_types::command::LinkRule>, MadoError> {
        Ok(Vec::new())
    }
    fn set_link_rules(&self, _: Vec<mado_types::command::LinkRule>) -> Result<(), MadoError> {
        Ok(())
    }
    fn route(&self, url: &str) -> Option<mado_types::ProfileId> {
        url.contains("acme.com").then_some(self.0)
    }
    fn export(&self, _: mado_types::ProfileId, _: &str) -> Result<(), MadoError> {
        Ok(())
    }
    fn import(&self, _: &str) -> Result<Value, MadoError> {
        Ok(json!({}))
    }
}

/// 12 §1.3: links from other apps that a rule sends to another profile go
/// there; the rest open here. Profile commands come only from Mado's UI.
#[test]
fn links_from_other_apps_follow_link_rules() {
    use mado_types::command::{LaunchRequest, LaunchSource};
    let work = mado_types::ProfileId::new();
    crate::testing::PROFILES.with_borrow_mut(|p| *p = std::sync::Arc::new(RoutingProfiles(work)));
    let core = TestCore::start();
    crate::testing::PROFILES.with_borrow_mut(|p| *p = std::sync::Arc::new(crate::NoProfiles));
    startup(&core);
    ask(
        &core,
        Origin::System,
        Command::LaunchForward(LaunchRequest {
            urls: vec![
                "https://docs.acme.com/x".into(),
                "https://news.example/".into(),
            ],
            files: vec![],
            new_window: false,
            private: false,
            source: LaunchSource::Link,
            control: None,
        }),
    )
    .expect("forward");
    let handed = core
        .sent
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .other_profiles
        .clone();
    assert_eq!(handed, vec![(work, "https://docs.acme.com/x".to_owned())]);
    let urls: Vec<String> = state_of(&core)
        .tabs
        .values()
        .map(|t| t.url.clone())
        .collect();
    assert!(urls.contains(&"https://news.example/".to_owned()));
    assert!(!urls.iter().any(|u| u.contains("acme")));
    let client = mado_types::ClientId::new();
    let refused = ask(
        &core,
        Origin::Control { client },
        Command::ProfileDelete { id: work },
    )
    .expect_err("control clients may not");
    assert_eq!(refused.message_key, "error.profile.notAllowed");
    let own = state_of(&core).profile.id;
    let me =
        ask(&core, Origin::Settings, Command::ProfileDelete { id: own }).expect_err("not itself");
    assert_eq!(me.message_key, "error.profile.current");
    // Renaming this profile updates the document.
    ask(
        &core,
        Origin::Settings,
        Command::ProfileUpdate {
            id: own,
            name: Some("Home".into()),
            color: Some("green".into()),
            avatar: None,
        },
    )
    .expect("update");
    eventually("renamed", || state_of(&core).profile.name == "Home");
}

/// 12 §1.6: a private window from a normal profile opens in a temporary
/// profile's process; a guest window too.
#[test]
fn private_and_guest_windows_open_temporary_profiles() {
    let core = TestCore::start();
    startup(&core);
    let windows = state_of(&core).windows.len();
    let v = ask(
        &core,
        Origin::System,
        Command::WindowNew {
            private: true,
            urls: vec!["https://a.example/".into()],
        },
    )
    .expect("private");
    assert_eq!(v["private"], true);
    ask(&core, Origin::Settings, Command::ProfileOpenGuest).expect("guest");
    let asked = core
        .sent
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .temporary
        .clone();
    assert_eq!(
        asked,
        vec![
            (false, vec!["https://a.example/".to_owned()]),
            (true, vec![])
        ]
    );
    assert_eq!(state_of(&core).windows.len(), windows, "no window here");
}

/// 08 §12 check 1: a first visit to a lookalike of paypal.com shows "Did
/// you mean paypal.com?"; Go to goes there; once bookmarked, or after
/// Continue, the site loads.
#[test]
fn a_lookalike_domain_asks_did_you_mean() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let spoof = "https://xn--pypal-4ve.com/login";
    let tab = new_tab(&core, w, "https://start.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://start.example/"
    });
    core.page.page_navigates(tab, spoof, true);
    eventually("the page", || {
        error_pages(&core)
            .iter()
            .any(|(t, u, h)| *t == tab && u == spoof && h.contains(r#"data-kind="lookalike""#))
    });
    let html = error_pages(&core)
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    assert!(html.contains("Did you mean paypal.com?"));
    core.page
        .page_navigates(tab, &action_link(&html, "goTo").expect("go to"), true);
    eventually("paypal", || {
        state_of(&core).tabs[&tab].url == "https://paypal.com/"
    });
    // Bookmarked: no page.
    ask(
        &core,
        Origin::System,
        Command::BookmarkAdd {
            parent_id: None,
            title: "Shop".into(),
            url: "https://xn--pypal-4ve.com/".into(),
            index: None,
        },
    )
    .expect("bookmark");
    let other = new_tab(&core, w, "https://start.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&other].url == "https://start.example/"
    });
    let pages = error_pages(&core).len();
    core.page.page_navigates(other, spoof, true);
    eventually("loads", || state_of(&core).tabs[&other].url == spoof);
    assert_eq!(error_pages(&core).len(), pages);
    // Continue on another lookalike is remembered.
    let third = new_tab(&core, w, "https://start.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&third].url == "https://start.example/"
    });
    core.page
        .page_navigates(third, "https://paypal-login.com/", true);
    eventually("the page", || {
        error_pages(&core).iter().any(|(t, _, _)| *t == third)
    });
    let html = error_pages(&core)
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    core.page.page_navigates(
        third,
        &action_link(&html, "continue").expect("continue"),
        true,
    );
    // The error page already shows this URL: wait until the engine may load
    // it, or that load could land after the next navigation.
    eventually("continued", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decisions
            .iter()
            .any(|(t, u, d)| {
                *t == third
                    && u == "https://paypal-login.com/"
                    && *d == mado_engine::NavDecision::Allow
            })
    });
    core.page
        .page_navigates(third, "https://paypal-login.com/again", true);
    eventually("again", || {
        state_of(&core).tabs[&third].url == "https://paypal-login.com/again"
    });
}

/// 08 §6: with `privacy.lockPrivateWindows` on, a private window five
/// minutes in the background hides its page until the user authenticates;
/// focus in time keeps it open, and a normal profile never locks.
#[test]
fn private_windows_lock_in_the_background() {
    crate::testing::PRIVATE.set(true);
    let core = TestCore::start();
    crate::testing::PRIVATE.set(false);
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://a.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://a.example/"
    });
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "privacy.lockPrivateWindows".into(),
            value: json!(true),
            token: None,
        },
    )
    .expect("set");
    let clock = |now: i64| {
        core.handle
            .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
                now,
            ))));
    };
    let focus = |focused: bool| {
        core.handle.send(Input::Engine(
            mado_engine::EngineEvent::WindowFocusChanged { window: w, focused },
        ));
    };
    let check = || {
        core.handle.send(Input::Internal(Internal::Hook(
            crate::TestHook::PrivateLockCheck,
        )));
    };
    let locked = || state_of(&core).windows[&w].locked;
    let visible = || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tab_log
            .iter()
            .filter(|(t, _)| *t == tab)
            .map(|(_, c)| *c)
            .filter(|c| *c == "setVisible")
            .count()
    };
    clock(1_000_000);
    focus(false);
    clock(1_000_000 + 299_000);
    check();
    assert!(!locked(), "not yet five minutes");
    focus(true);
    clock(1_000_000 + 600_000);
    check();
    assert!(!locked(), "back in front in time");
    clock(2_000_000);
    focus(false);
    check();
    assert!(!locked());
    let shown = visible();
    clock(2_000_000 + 300_000);
    check();
    eventually("locked", locked);
    eventually("hidden", || visible() == shown + 1);
    assert_eq!(
        ask(
            &core,
            Origin::Control {
                client: ClientId::new()
            },
            Command::WindowUnlock { window_id: w }
        )
        .expect_err("only Mado's windows")
        .code,
        ErrorCode::Protected
    );
    core.handle.send(Input::Internal(Internal::Hook(
        crate::TestHook::FakeAuthSuccess,
    )));
    ask(
        &core,
        Origin::Ui { window: w },
        Command::WindowUnlock { window_id: w },
    )
    .expect("unlocked");
    assert!(!locked());
    eventually("shown", || visible() == shown + 2);
}

/// 08 §10: the probe marks an article, Reader shows it sanitised at
/// `mado://reader`, Back leaves it, "Always open in Reader" opens it by
/// itself, and with `features.reader` off nothing runs.
#[test]
fn reader_mode_shows_the_article_and_remembers_sites() {
    let core = TestCore::start();
    startup(&core);
    core.engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .eval_answers = vec![
        ("// reader-probe".into(), json!(true)),
        (
            "// reader-extract".into(),
            json!({
                "title": "A long read",
                "byline": "Ada",
                "content": "<p>Text</p><script>alert(1)</script>",
                "length": 4,
                "siteName": "News",
            }),
        ),
    ];
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://news.example/a");
    eventually("an article", || state_of(&core).tabs[&tab].reader_available);
    ask(
        &core,
        Origin::Ui { window: w },
        Command::TabReader {
            tab_id: tab,
            on: true,
        },
    )
    .expect("reader");
    eventually("in Reader", || {
        let t = &state_of(&core).tabs[&tab];
        t.reader_active && t.url.starts_with("mado://reader?tab=")
    });
    let html = core
        .engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .generated_pages
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    assert!(
        html.contains("A long read") && html.contains("<p>Text</p>") && !html.contains("alert")
    );
    ask(
        &core,
        Origin::Ui { window: w },
        Command::TabReader {
            tab_id: tab,
            on: false,
        },
    )
    .expect("leave");
    eventually("back", || {
        state_of(&core).tabs[&tab].url == "https://news.example/a"
    });
    ask(
        &core,
        Origin::Ui { window: w },
        Command::SiteSetReaderAlways {
            host: "news.example".into(),
            always: true,
        },
    )
    .expect("always");
    let other = new_tab(&core, w, "https://news.example/b");
    eventually("opened by itself", || {
        state_of(&core).tabs[&other].reader_active
    });
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "features.reader".into(),
            value: json!(false),
            token: None,
        },
    )
    .expect("off");
    assert_eq!(
        ask(
            &core,
            Origin::Ui { window: w },
            Command::TabReader {
                tab_id: tab,
                on: true
            }
        )
        .expect_err("off")
        .code,
        ErrorCode::Unsupported
    );
}

/// 12 §7 check 6: each fixture browser imports its bookmarks, history,
/// search engines and tabs; a second run adds nothing, and Arc's spaces
/// become saved groups.
#[test]
fn importing_from_each_browser_is_complete_and_idempotent() {
    use mado_types::command::{ImportBrowser, ImportType};
    let all = vec![
        ImportType::Bookmarks,
        ImportType::History,
        ImportType::SearchEngines,
        ImportType::Tabs,
        ImportType::Extensions,
    ];
    // (browser, fixture, bar + other links, visits in 90 days, engines, tabs)
    let cases = [
        // Google is built in already; only Wikipedia is new.
        (ImportBrowser::Chrome, "chrome", "Default", 3, 3, 1, 3),
        (
            ImportBrowser::Firefox,
            "firefox",
            "Profiles/abcd1234.default-release",
            3,
            2,
            1,
            2,
        ),
        (ImportBrowser::Safari, "safari", "", 3, 2, 0, 0),
        (ImportBrowser::Arc, "arc", "Default", 1, 0, 0, 1),
    ];
    for (browser, fixture, profile, links, visits, engines, tabs) in cases {
        let home = mado_import::fixtures::home(fixture);
        crate::testing::IMPORT_HOME.with_borrow_mut(|h| *h = home.path().to_owned());
        let core = TestCore::start();
        crate::testing::IMPORT_HOME.with_borrow_mut(|h| *h = "/nonexistent".into());
        startup(&core);
        // 2026-11-15: the fixtures' recent visits are within 90 days.
        core.handle
            .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
                1_794_700_800_000,
            ))));
        let found = ask(&core, Origin::Settings, Command::ImportDetect).expect("detect");
        assert_eq!(found[0]["profile"], profile, "{fixture}: {found}");
        let run = Command::ImportRun {
            browser,
            profile: profile.into(),
            types: all.clone(),
            all_history: false,
        };
        assert_eq!(
            ask(
                &core,
                Origin::Control {
                    client: ClientId::new()
                },
                run.clone()
            )
            .expect_err("only Mado's windows")
            .code,
            ErrorCode::Protected
        );
        let windows_before = state_of(&core).windows.len();
        let counts = ask(&core, Origin::Settings, run.clone()).expect("import");
        assert_eq!(counts["bookmarks"], links, "{fixture}: {counts}");
        assert_eq!(counts["visits"], visits, "{fixture}: {counts}");
        assert_eq!(counts["engines"], engines, "{fixture}: {counts}");
        assert_eq!(counts["tabs"], tabs, "{fixture}: {counts}");
        let st = state_of(&core);
        assert!(st.windows.len() >= windows_before);
        let again = ask(
            &core,
            Origin::Settings,
            Command::ImportRun {
                browser,
                profile: profile.into(),
                types: vec![
                    ImportType::Bookmarks,
                    ImportType::History,
                    ImportType::SearchEngines,
                ],
                all_history: false,
            },
        )
        .expect("again");
        assert_eq!(
            (
                again["bookmarks"].as_u64(),
                again["visits"].as_u64(),
                again["engines"].as_u64()
            ),
            (Some(0), Some(0), Some(0)),
            "{fixture}: idempotent"
        );
        let conn =
            rusqlite::Connection::open(core.dir.path().join("profile.sqlite")).expect("open");
        let n = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).expect(sql) };
        assert_eq!(
            n("SELECT COUNT(*) FROM bookmarks WHERE kind = 'link'"),
            links
        );
        assert_eq!(
            n("SELECT COUNT(*) FROM visits WHERE transition = 'imported'"),
            visits
        );
        match browser {
            ImportBrowser::Chrome => {
                assert_eq!(n("SELECT COUNT(*) FROM imported_extensions"), 1);
                let engines =
                    ask(&core, Origin::Settings, Command::NavSearchEngines).expect("engines");
                let wiki = engines
                    .as_array()
                    .and_then(|a| a.iter().find(|e| e["name"] == "Wikipedia"))
                    .expect("Wikipedia");
                assert_eq!(wiki["active"], true, "the default is on: {wiki}");
                let pinned = st
                    .tabs
                    .values()
                    .filter(|t| t.pinned && t.url == "https://mail.example/")
                    .count();
                assert_eq!(pinned, 1);
                assert_eq!(st.groups.len(), 1, "the tab group came along");
            }
            ImportBrowser::Arc => {
                let saved = st
                    .saved_groups
                    .iter()
                    .find(|g| g.title == "Work")
                    .expect("the space");
                assert_eq!(saved.tab_count, 2);
                assert!(!saved.open);
            }
            _ => {}
        }
    }
}

/// Safari without Full Disk Access gets an error the sheet explains.
#[test]
fn an_import_without_full_disk_access_says_so() {
    use mado_types::command::{ImportBrowser, ImportType};
    use std::os::unix::fs::PermissionsExt;
    let home = mado_import::fixtures::home("safari");
    std::fs::set_permissions(
        home.path().join("Library/Safari/Bookmarks.plist"),
        std::fs::Permissions::from_mode(0o000),
    )
    .expect("chmod");
    crate::testing::IMPORT_HOME.with_borrow_mut(|h| *h = home.path().to_owned());
    let core = TestCore::start();
    crate::testing::IMPORT_HOME.with_borrow_mut(|h| *h = "/nonexistent".into());
    let err = ask(
        &core,
        Origin::Settings,
        Command::ImportRun {
            browser: ImportBrowser::Safari,
            profile: String::new(),
            types: vec![ImportType::Bookmarks],
            all_history: false,
        },
    )
    .expect_err("refused");
    assert_eq!(
        (err.code, err.message_key.as_str()),
        (ErrorCode::PermissionDenied, "error.import.fullDiskAccess")
    );
}

/// 01 §4.4: a `mado://` page's main frame may send what its allow-list
/// names and gets the answer; anything else, or the same message from a
/// web page or a subframe, is refused. Pages that read state get only
/// their fields, in `state.get` and in patches.
#[test]
fn internal_pages_invoke_through_their_allow_lists() {
    use mado_engine::event::FrameInfo;
    use mado_engine::page::{Invoke, PageMessage};
    use mado_types::state::{UpdateState, UpdateStatus};
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "mado://history");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "mado://history"
    });
    let posted = || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .posted
            .iter()
            .filter(|(t, _)| *t == tab)
            .map(|(_, m)| m.clone())
            .collect::<Vec<_>>()
    };
    let send = |url: &str, main: bool, id: u64, command: Value| {
        core.page.send(mado_engine::EngineEvent::PageMessage {
            tab,
            world: mado_engine::tab::World::Mado,
            frame: FrameInfo {
                is_main_frame: main,
                origin: url.split('/').take(3).collect::<Vec<_>>().join("/"),
                url: url.into(),
            },
            message: PageMessage::Invoke(Invoke { id, command }),
        });
    };
    let reply = |id: u64| {
        eventually("a reply", || {
            posted()
                .iter()
                .any(|m| m["type"] == "invokeReply" && m["id"] == id)
        });
        posted()
            .into_iter()
            .find(|m| m["type"] == "invokeReply" && m["id"] == id)
            .expect("reply")
    };
    send(
        "mado://history",
        true,
        1,
        json!({ "type": "history.query", "text": null, "host": null, "from": null, "to": null, "limit": 10, "cursor": null, "utcOffsetMinutes": 0 }),
    );
    let r = reply(1);
    assert_eq!(r["ok"], true, "{r}");
    send(
        "mado://history",
        true,
        2,
        json!({ "type": "settings.set", "key": "privacy.gpc", "value": false, "token": null }),
    );
    let r = reply(2);
    assert_eq!(
        (r["ok"].clone(), r["error"]["code"].clone()),
        (json!(false), json!("protected")),
        "{r}"
    );
    // The same from a web page or a subframe gets no answer at all.
    send(
        "https://evil.example/",
        true,
        3,
        json!({ "type": "history.query" }),
    );
    send(
        "mado://history",
        false,
        4,
        json!({ "type": "history.query" }),
    );
    send("mado://history", true, 5, json!({ "type": "state.get" }));
    let r = reply(5);
    assert_eq!(
        r["value"]["state"],
        json!({}),
        "the history page reads no state: {r}"
    );
    assert_eq!(r["value"]["tabId"], json!(tab));
    // nav.open acts from the page's own tab, whatever it names.
    send(
        "mado://history",
        true,
        6,
        json!({ "type": "nav.open", "windowId": "01a1d7aa-0000-7000-8000-00000000dead", "tabId": null, "input": "https://example.org/", "disposition": "currentTab", "source": "typed", "ctrlEnter": false }),
    );
    assert_eq!(reply(6)["ok"], true);
    eventually("navigated", || {
        state_of(&core).tabs[&tab].url == "https://example.org/"
    });
    assert!(!posted().iter().any(|m| m["id"] == 3 || m["id"] == 4));
    // The About page: state.get has only `update`, and patches follow it.
    let about = new_tab(&core, w, "mado://about");
    eventually("loaded", || {
        state_of(&core).tabs[&about].url == "mado://about"
    });
    core.page.send(mado_engine::EngineEvent::PageMessage {
        tab: about,
        world: mado_engine::tab::World::Mado,
        frame: FrameInfo {
            is_main_frame: true,
            origin: "mado://about".into(),
            url: "mado://about".into(),
        },
        message: PageMessage::Invoke(Invoke {
            id: 9,
            command: json!({ "type": "state.get" }),
        }),
    });
    let about_posted = || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .posted
            .iter()
            .filter(|(t, _)| *t == about)
            .map(|(_, m)| m.clone())
            .collect::<Vec<_>>()
    };
    eventually("the snapshot", || {
        about_posted().iter().any(|m| m["id"] == 9)
    });
    let snap = about_posted()
        .into_iter()
        .find(|m| m["id"] == 9)
        .expect("snapshot");
    let keys: Vec<String> = snap["value"]["state"]
        .as_object()
        .expect("state")
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, ["about", "update"]);
    assert_eq!(snap["value"]["state"]["about"]["version"], "0.1.0");
    core.handle
        .send(Input::Internal(Internal::UpdateStatus(UpdateState {
            status: UpdateStatus::Ready,
            version: Some("0.1.0".into()),
            security: false,
        })));
    eventually("a filtered patch", || {
        about_posted().iter().any(|m| {
            m["type"] == "patch"
                && m["ops"].as_array().is_some_and(|ops| {
                    !ops.is_empty()
                        && ops
                            .iter()
                            .all(|o| o["path"].as_str().is_some_and(|p| p.starts_with("/update")))
                })
        })
    });
    assert!(
        !posted().iter().any(|m| m["type"] == "patch"),
        "the history page reads no state"
    );
}

/// 01 §5.4: every menu item runs its command in the key window, the UI
/// carries out its own, and enabled and checked states follow the state.
#[test]
fn every_menu_item_runs_its_command() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let first = new_tab(&core, w, "https://a.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&first].url == "https://a.example/"
    });
    let menu = |id: &str| {
        core.handle.send(Input::Internal(Internal::Menu {
            id: id.into(),
            window: Some(w),
        }));
        let _ = ask(&core, Origin::System, Command::StateGet);
    };
    let sent = || core.sent.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        sent().shortcuts.get("newTab").map(String::as_str),
        Some("Cmd+T")
    );
    assert_eq!(
        sent().shortcuts.len(),
        mado_types::menu::command_ids().len()
    );
    let tabs = || state_of(&core).windows[&w].tab_order.len();
    let before = tabs();
    menu("newTab");
    assert_eq!(tabs(), before + 1);
    menu("closeTab");
    assert_eq!(tabs(), before);
    menu("reopenClosedTab");
    assert_eq!(tabs(), before + 1);
    menu("selectTab1");
    let active = || state_of(&core).windows[&w].active_tab_id;
    let first_tab = state_of(&core).windows[&w].tab_order[0];
    assert_eq!(active(), Some(first_tab));
    menu("selectLastTab");
    assert_eq!(
        active(),
        state_of(&core).windows[&w].tab_order.last().copied()
    );
    menu("zoomIn");
    let a = active().expect("active");
    assert!((state_of(&core).tabs[&a].zoom - 1.1).abs() < 1e-9);
    menu("actualSize");
    assert!((state_of(&core).tabs[&a].zoom - 1.0).abs() < 1e-9);
    menu("pinTab");
    assert!(state_of(&core).tabs[&a].pinned);
    eventually("checked", || {
        sent().menu_state.get("pinTab") == Some(&(true, true))
    });
    menu("pinTab");
    assert!(!state_of(&core).tabs[&a].pinned);
    assert_eq!(
        sent().menu_state.get("back"),
        Some(&(false, false)),
        "nothing to go back to"
    );
    menu("toggleBookmarksBar");
    eventually("bar checked", || {
        sent()
            .menu_state
            .get("toggleBookmarksBar")
            .is_some_and(|s| s.1)
    });
    menu("showHistory");
    assert!(
        state_of(&core)
            .tabs
            .values()
            .any(|t| t.url == "mado://history")
    );
    menu("openLocation");
    menu("find");
    assert_eq!(
        sent().window_actions,
        vec![(w, "openLocation".to_owned()), (w, "find".to_owned())]
    );
    menu("settings");
    assert_eq!(sent().settings_opened, 1);
    let windows = state_of(&core).windows.len();
    menu("newWindow");
    assert_eq!(state_of(&core).windows.len(), windows + 1);
    // Every ID is handled by something.
    for id in mado_types::menu::command_ids() {
        let known = [
            "about",
            "settings",
            "quit",
            "importBrowserData",
            "developerTools",
            "viewSource",
            "openLocation",
            "focusAddressBar",
            "find",
            "findNext",
            "findPrevious",
            "bookmarkTab",
            "bookmarkAllTabs",
            "newTab",
            "newWindow",
            "newPrivateWindow",
            "reopenClosedTab",
            "closeWindow",
            "closeTab",
            "print",
            "stop",
            "reload",
            "forceReload",
            "actualSize",
            "zoomIn",
            "zoomOut",
            "reader",
            "back",
            "forward",
            "home",
            "showHistory",
            "bookmarkManager",
            "downloads",
            "nextTab",
            "previousTab",
            "selectLastTab",
            "duplicateTab",
            "pinTab",
            "muteSite",
            "moveTabToNewWindow",
            "toggleBookmarksBar",
        ];
        assert!(
            known.contains(&id) || id.starts_with("selectTab"),
            "{id} has no action"
        );
    }
    // View Source: the page's document, escaped, in a new tab beside it.
    core.engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .eval_answers = vec![(
        "return { url".into(),
        json!({ "url": "https://a.example/", "html": "<p>hi</p>" }),
    )];
    menu("selectTab1");
    let source_of = active().expect("active");
    menu("viewSource");
    eventually("the source tab", || {
        state_of(&core)
            .tabs
            .values()
            .any(|t| t.url.starts_with("mado://source?tab=") && t.opener_id == Some(source_of))
    });
    let page = core
        .engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .generated_pages
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    assert!(
        page.contains("&lt;p&gt;hi&lt;/p&gt;") && page.contains("view-source:https://a.example/")
    );
    // A changed shortcut reaches the app.
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "shortcuts.duplicateTab".into(),
            value: json!("Cmd+Shift+K"),
            token: None,
        },
    )
    .expect("set");
    assert_eq!(
        sent().shortcuts.get("duplicateTab").map(String::as_str),
        Some("Cmd+Shift+K")
    );
}

/// 15 §5: a ready security update relaunches by itself after 10 minutes,
/// once nobody has used Mado for 10 minutes and no tab holds typed text;
/// after the relaunch the last session comes back whatever startup.mode
/// says.
#[test]
fn security_updates_relaunch_when_idle_and_sessions_come_back() {
    use mado_types::state::{UpdateState, UpdateStatus};
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let tab = new_tab(&core, w, "https://kept.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&tab].url == "https://kept.example/"
    });
    let clock = |now: i64| {
        core.handle
            .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
                now,
            ))))
    };
    let relaunches = || {
        core.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .relaunches
            .clone()
    };
    clock(1_000_000);
    // Input at 1_000_000.
    let _ = ask(&core, Origin::Ui { window: w }, Command::StateGet);
    core.handle
        .send(Input::Internal(Internal::UpdateStatus(UpdateState {
            status: UpdateStatus::Ready,
            version: Some("0.2.0".into()),
            security: true,
        })));
    let check = || {
        let _ = ask(&core, Origin::System, Command::StateGet);
        core.handle.send(Input::Internal(Internal::Hook(
            crate::TestHook::SecurityCheck,
        )));
        let _ = ask(&core, Origin::System, Command::StateGet);
    };
    clock(1_000_000 + 9 * 60_000);
    check();
    assert!(relaunches().is_empty(), "not 10 minutes yet");
    clock(1_000_000 + 11 * 60_000);
    // Someone typed into a page: no relaunch.
    core.handle
        .send(Input::Internal(Internal::TestUnsavedInput { tab }));
    check();
    assert!(relaunches().is_empty(), "typed text waits");
    let (tx, rx) = tokio::sync::oneshot::channel();
    core.handle
        .send(Input::Internal(Internal::IdleForUpdate { reply: tx }));
    assert!(!rx.blocking_recv().expect("answer"));
    core.page
        .page_navigates(tab, "https://kept.example/next", true);
    eventually("typed text gone", || {
        !state_of(&core).tabs[&tab].has_unsaved_input
    });
    check();
    assert_eq!(relaunches(), vec![true]);
    // The relaunched profile opens its last session even with newTab.
    let dir = core.stop();
    crate::testing::RESTORE.set(true);
    let again = TestCore::start_in(dir, true);
    crate::testing::RESTORE.set(false);
    startup(&again);
    eventually("the session", || {
        state_of(&again)
            .tabs
            .values()
            .any(|t| t.url == "https://kept.example/next")
    });
}

/// M2-T17: the welcome sheet shows until finished, once per Mac, never in
/// a private profile; the default browser is offered and only Mado's own
/// windows may ask macOS to change it.
#[test]
fn the_first_run_shows_once_and_offers_the_default_browser() {
    let core = TestCore::start();
    assert!(state_of(&core).first_run);
    assert_eq!(
        ask(&core, Origin::Settings, Command::AppDefaultBrowser).expect("query"),
        json!({ "isDefault": false })
    );
    assert_eq!(
        ask(
            &core,
            Origin::Control {
                client: ClientId::new()
            },
            Command::AppSetDefaultBrowser
        )
        .expect_err("not from a control client")
        .code,
        ErrorCode::Protected
    );
    ask(&core, Origin::Settings, Command::AppSetDefaultBrowser).expect("asked macOS");
    assert_eq!(
        ask(&core, Origin::Settings, Command::AppDefaultBrowser).expect("query"),
        json!({ "isDefault": true })
    );
    crate::testing::DEFAULT_BROWSER.store(false, std::sync::atomic::Ordering::SeqCst);
    ask(&core, Origin::Settings, Command::AppFinishFirstRun).expect("finished");
    assert!(!state_of(&core).first_run);
    let dir = core.stop();
    let again = TestCore::start_in(dir, true);
    assert!(!state_of(&again).first_run, "once per Mac");
    crate::testing::PRIVATE.set(true);
    let private = TestCore::start();
    crate::testing::PRIVATE.set(false);
    assert!(!state_of(&private).first_run);
}

/// 08 §1, §8: the dangerous-site warning, strict tracking prevention and
/// Global Privacy Control reach the engine at start and when they change.
#[test]
fn privacy_settings_reach_the_engine() {
    let core = TestCore::start();
    startup(&core);
    let engine = || core.engine.lock().unwrap_or_else(|e| e.into_inner());
    let has_gpc =
        |s: &mado_engine_fake::FakeState| s.page_scripts.scripts.iter().any(|s| s.name == "gpc");
    assert_eq!(
        engine().privacy,
        Some(mado_engine::PrivacyConfig::default())
    );
    assert!(has_gpc(&engine()), "privacy.gpc is on by default");
    let set = |key: &str, value: Value| {
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
    };
    set("privacy.trackingPrevention", json!("strict"));
    set("privacy.gpc", json!(false));
    set("safety.dangerousSiteWarnings", json!(false));
    eventually("applied", || {
        let s = engine();
        s.privacy
            == Some(mado_engine::PrivacyConfig {
                dangerous_site_warnings: false,
                block_third_party_cookies: true,
                hide_local_ip: true,
            })
            && !has_gpc(&s)
    });
}

/// 08 §7: `data.clear` removes the range's history (recounting what
/// stays), site settings and website data, and only Mado's own windows may
/// ask; `privacy.clearOnExit` clears before the app quits.
#[test]
fn clearing_browsing_data_by_range_and_on_quit() {
    use mado_types::command::{ClearRange, DataType};
    let core = TestCore::start();
    startup(&core);
    let now: i64 = 1_800_000_000_000;
    core.handle
        .send(Input::Internal(Internal::Hook(crate::TestHook::FakeClock(
            now,
        ))));
    {
        let conn =
            rusqlite::Connection::open(core.dir.path().join("profile.sqlite")).expect("open");
        conn.execute_batch(&format!(
            "INSERT INTO urls (id, url, host, visit_count, last_visit_at) VALUES
               (1, 'https://old.example/', 'old.example', 1, {old}),
               (2, 'https://both.example/', 'both.example', 2, {recent});
             INSERT INTO visits (url_id, visited_at, transition) VALUES
               (1, {old}, 'link'), (2, {old}, 'link'), (2, {recent}, 'link');
             INSERT INTO site_settings (origin, key, value, granted_at, source) VALUES
               ('https://old.example', 'camera', 'allow', {old}, 'user');
             INSERT INTO form_history (name, value, host, uses, last_used_at) VALUES
               ('q', 'recent', 'a.example', 1, {recent}), ('q', 'old', 'a.example', 1, {old});",
            old = now - 2 * 86_400_000,
            recent = now - 600_000,
        ))
        .expect("seed");
    }
    ask(
        &core,
        Origin::Settings,
        Command::PermissionSet {
            origin: "https://new.example".into(),
            key: "camera".into(),
            value: mado_types::command::SiteValue::Allow,
            embedder: None,
        },
    )
    .expect("a new site setting");
    let clear = Command::DataClear {
        range: ClearRange::Hour,
        types: vec![
            DataType::History,
            DataType::SiteSettings,
            DataType::FormHistory,
            DataType::Cookies,
        ],
    };
    assert_eq!(
        ask(
            &core,
            Origin::Control {
                client: ClientId::new()
            },
            clear.clone()
        )
        .expect_err("control clients may not")
        .code,
        ErrorCode::Protected
    );
    ask(&core, Origin::Settings, clear).expect("cleared");
    let conn = rusqlite::Connection::open(core.dir.path().join("profile.sqlite")).expect("open");
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).expect(sql) };
    assert_eq!(
        count("SELECT COUNT(*) FROM visits"),
        2,
        "the old visits stay"
    );
    assert_eq!(
        count("SELECT visit_count FROM urls WHERE id = 2"),
        1,
        "recounted"
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM site_settings"),
        1,
        "the old grant stays"
    );
    assert_eq!(count("SELECT COUNT(*) FROM form_history"), 1);
    let listed = ask(
        &core,
        Origin::Settings,
        Command::PermissionList {
            origin: Some("https://new.example".into()),
        },
    )
    .expect("list");
    assert!(!listed.to_string().contains("camera"), "{listed}");
    let cleared = core
        .engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .cleared
        .clone();
    let last = cleared.last().expect("website data cleared");
    assert!(last.kinds.cookies && last.kinds.local_storage && !last.kinds.cache);
    assert_eq!(last.since, Some(now - 3_600_000));
    // One site's data.
    ask(
        &core,
        Origin::Settings,
        Command::SiteClearData {
            origin: "https://www.example.com".into(),
        },
    )
    .expect("site data");
    assert_eq!(
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cleared
            .last()
            .and_then(|r| r.hosts.clone()),
        Some(vec!["www.example.com".to_owned()])
    );
    // Clear on quit.
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "privacy.clearOnExit".into(),
            value: json!(["history", "cache"]),
            token: None,
        },
    )
    .expect("set");
    let before = core
        .engine
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .cleared
        .len();
    core.handle
        .send(Input::Internal(Internal::QuitRequested { shortcut: false }));
    eventually("quit", || {
        core.sent.lock().unwrap_or_else(|e| e.into_inner()).quits == 1
    });
    assert_eq!(count("SELECT COUNT(*) FROM visits"), 0);
    let engine = core.engine.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(engine.cleared.len(), before + 1);
    let all = engine.cleared.last().expect("cache cleared");
    assert!(all.kinds.cache && !all.kinds.cookies && all.since.is_none());
}

/// 08 §3 and §12 check 2: a bad certificate shows the interstitial with
/// Proceed, which loads the site for the session and marks it dangerous;
/// an HSTS host has no Proceed.
#[test]
fn certificate_errors_offer_proceed_except_on_hsts_hosts() {
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    unreachable(
        &core,
        "https://expired.example",
        mado_engine::NetError::Certificate,
    );
    unreachable(
        &core,
        "https://hsts.example",
        mado_engine::NetError::Certificate,
    );
    let tab = new_tab(&core, w, "https://expired.example/");
    eventually("the interstitial", || {
        error_pages(&core)
            .iter()
            .any(|(t, _, h)| *t == tab && h.contains(r#"data-kind="certificate""#))
    });
    let html = error_pages(&core)
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    core.page
        .page_navigates(tab, &action_link(&html, "proceed").expect("proceed"), true);
    eventually("loaded", || {
        let st = state_of(&core);
        st.tabs[&tab].url == "https://expired.example/"
            && st.tabs[&tab].security.level == mado_types::state::SecurityLevel::Dangerous
            && st.tabs[&tab].life == mado_types::state::TabLife::Live
    });
    assert_eq!(
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allowed_certificates,
        vec!["expired.example".to_owned()]
    );
    let hsts = new_tab(&core, w, "https://hsts.example/");
    eventually("no proceed", || {
        error_pages(&core)
            .iter()
            .any(|(t, _, h)| *t == hsts && h.contains(r#"data-kind="certificateHsts""#))
    });
    let html = error_pages(&core)
        .last()
        .map(|p| p.2.clone())
        .expect("a page");
    assert!(!html.contains("a=proceed"));
    // Both offer the certificate viewer.
    core.page.page_navigates(
        hsts,
        &action_link(&html, "viewCertificate").expect("view"),
        true,
    );
    eventually("the viewer", || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tab_log
            .iter()
            .any(|(t, c)| *t == hsts && *c == "showCertificate")
    });
}

/// Serves `files` by path over HTTP and logs each path asked for.
fn serve_files(
    files: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>,
) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = std::sync::Arc::clone(&log);
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = head.split(' ').nth(1).unwrap_or("/").to_owned();
            let body = files
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&path)
                .cloned();
            seen.lock().unwrap_or_else(|e| e.into_inner()).push(path);
            let (status, body) = match body {
                Some(b) => ("200 OK", b),
                None => ("404 Not Found", Vec::new()),
            };
            let _ = write!(
                s,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(&body);
        }
    });
    (port, log)
}

/// M3-T01, Done when: the daily fetch installs a signed list (About's
/// engine check follows the new compatibility list), rejects a tampered
/// one, and the last good copy stays in use and on disk.
#[test]
fn a_tampered_list_is_rejected_and_the_last_good_copy_stays_in_use() {
    use mado_lists::List;
    let keys = minisign::KeyPair::generate_unencrypted_keypair().expect("keys");
    let sign = |serial: u64, bytes: &[u8]| {
        minisign::sign(
            None,
            &keys.sk,
            std::io::Cursor::new(bytes),
            Some(&mado_lists::trusted_comment(List::Compat, serial)),
            None,
        )
        .expect("sign")
        .into_string()
        .into_bytes()
    };
    let compat =
        |engine: &str| format!(r#"{{"minSafeEngine":{{"webkit":"{engine}"}}}}"#).into_bytes();
    let files = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let publish = |bytes: Vec<u8>, sig: Vec<u8>| {
        let mut f = files.lock().unwrap_or_else(|e| e.into_inner());
        f.insert("/lists/compat.json".to_owned(), bytes);
        f.insert("/lists/compat.json.minisig".to_owned(), sig);
    };
    let (port, log) = serve_files(std::sync::Arc::clone(&files));
    let dir = tempfile::tempdir().expect("dir");
    let store = std::sync::Arc::new(
        mado_lists::Store::new(dir.path().join("Lists"), &keys.pk.to_base64()).expect("store"),
    );
    crate::testing::LISTS.with_borrow_mut(|l| {
        *l = Some(crate::ListsConfig {
            store,
            url: format!("http://127.0.0.1:{port}/lists/"),
        });
    });
    let core = TestCore::start();
    crate::testing::LISTS.with_borrow_mut(|l| *l = None);
    let outdated = || state_of(&core).about.engine_outdated;
    let fetches = || {
        log.lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|p| p.ends_with("hsts-preload.txt.gz.minisig"))
            .count()
    };
    let fetch = |n: usize| {
        core.handle
            .send(Input::Internal(Internal::Hook(crate::TestHook::ListsFetch)));
        eventually("the fetch", || fetches() == n);
        // The install runs off the core thread after the last request.
        std::thread::sleep(std::time::Duration::from_millis(300));
    };
    assert!(!outdated(), "625.2.8 is newer than the bundled minimum");

    // Tampered: signed over one list, serving another.
    publish(compat("900.1"), sign(100, &compat("600.1")));
    fetch(1);
    assert!(!outdated(), "a tampered list changes nothing");

    publish(compat("900.1"), sign(200, &compat("900.1")));
    fetch(2);
    eventually("the new minimum", outdated);
    let cached = dir.path().join("Lists/compat.json.signed");
    let good = std::fs::read(&cached).expect("the last good copy");

    // A newer tampered copy and a replayed older one leave the good copy.
    publish(compat("100.1"), sign(300, &compat("900.1")));
    fetch(3);
    publish(compat("100.1"), sign(150, &compat("100.1")));
    fetch(4);
    assert!(outdated(), "still the good copy");
    assert_eq!(std::fs::read(&cached).expect("cached"), good);
    assert_eq!(mado_lists::serial(List::Compat), 200);
}

/// A store extension: a CRX signed by its developer and `publisher`.
fn test_extension(
    manifest: &str,
    publisher: Option<&mado_extensions::fixtures::Signer>,
) -> (String, Vec<u8>) {
    use mado_extensions::fixtures::{Signer, crx, zip_of};
    let dev = Signer::ec(3);
    let id = mado_extensions::id_from_key(&dev.public_key());
    let zip = zip_of(&[
        ("manifest.json", manifest.as_bytes()),
        ("icon.png", b"\x89PNG fake"),
    ]);
    let signers: Vec<&Signer> = std::iter::once(&dev).chain(publisher).collect();
    (id, crx(&signers, &dev, &zip, &[]))
}

/// M3-T03 (09 §2): "Add to Mado" downloads, verifies and unpacks the
/// package, the prompt shows what it asks for, Add installs it into the
/// profile, and it is still there after a restart. Themes and packages
/// without the store's proof are refused; only Mado's windows may install;
/// the store page's Remove from Mado removes it.
#[test]
fn a_store_extension_installs_through_the_prompt() {
    use mado_extensions::fixtures::{Signer, key_hash};
    use mado_types::state::PromptState;
    let publisher = Signer::ec(9);
    let manifest = r#"{"name":"Test Blocker","version":"1.2","manifest_version":3,
        "action":{"default_popup":"popup.html"},"icons":{"128":"icon.png"},
        "permissions":["storage","history"],"host_permissions":["<all_urls>"]}"#;
    let (id, package) = test_extension(manifest, Some(&publisher));
    let theme = test_extension(
        r#"{"name":"Dark","version":"1","manifest_version":3,"theme":{}}"#,
        Some(&publisher),
    )
    .1;
    let files = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let (port, _) = serve_files(std::sync::Arc::clone(&files));
    let serve = |bytes: &[u8]| {
        files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(format!("/crx/{id}"), bytes.to_vec());
    };
    let dir = tempfile::tempdir().expect("dir");
    let config = crate::ExtensionsConfig {
        dir: dir.path().join("Extensions"),
        package_url: std::sync::Arc::new(move |id| format!("http://127.0.0.1:{port}/crx/{id}")),
        extra_publishers: vec![key_hash(&publisher.public_key())],
    };
    crate::testing::EXTENSIONS.with_borrow_mut(|e| *e = Some(config.clone()));
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let install = |origin: Origin| {
        ask(
            &core,
            origin,
            Command::ExtensionInstall {
                id: Some(id.parse().expect("id")),
                source: mado_types::command::ExtensionSource::Store,
                path: None,
            },
        )
    };
    let err = install(Origin::Control {
        client: ClientId::new(),
    })
    .expect_err("protected");
    assert_eq!(err.code, ErrorCode::Protected);

    let notices = || -> Vec<String> {
        state_of(&core)
            .notices
            .iter()
            .map(|n| n.message_key.clone())
            .collect()
    };
    serve(&theme);
    install(Origin::Ui { window: w }).expect("asked");
    eventually("the theme refused", || {
        notices().contains(&"notice.extensionTheme".to_owned())
    });
    let unproven = test_extension(manifest, None).1;
    serve(&unproven);
    install(Origin::Ui { window: w }).expect("asked");
    eventually("no store proof", || {
        notices().contains(&"notice.extensionUnverified".to_owned())
    });
    assert!(state_of(&core).prompts.is_empty());

    serve(&package);
    install(Origin::Ui { window: w }).expect("asked");
    let prompt = || {
        state_of(&core).prompts.values().find_map(|p| match p {
            PromptState::ExtensionInstall(p) => Some(p.clone()),
            _ => None,
        })
    };
    eventually("the prompt", || prompt().is_some());
    let p = prompt().expect("prompt");
    assert_eq!(
        (p.name.as_str(), p.version.as_str(), p.window_id),
        ("Test Blocker", "1.2", Some(w))
    );
    assert!(p.all_sites && p.may_not_work && !p.older_format);
    assert_eq!(p.permissions, ["storage"]);
    assert_eq!(p.unavailable, ["history"]);
    assert!(
        p.icon
            .as_deref()
            .is_some_and(|i| i.starts_with("data:image/png;base64,"))
    );
    ask(
        &core,
        Origin::Overlay { window: w },
        Command::ExtensionConfirmInstall {
            prompt_id: p.id,
            add: true,
        },
    )
    .expect("added");
    let st = state_of(&core);
    assert!(st.prompts.is_empty());
    assert_eq!(st.extensions.installed.len(), 1);
    assert_eq!(st.extensions.pinned, [id.parse().expect("id")]);
    assert!(config.dir.join(&id).join("1.2/manifest.json").exists());
    assert!(notices().contains(&"notice.extensionAdded".to_owned()));
    let root = core.stop();

    crate::testing::EXTENSIONS.with_borrow_mut(|e| *e = Some(config.clone()));
    let core = TestCore::start_in(root, false);
    let st = state_of(&core);
    assert_eq!(
        st.extensions.installed.first().map(|e| e.name.as_str()),
        Some("Test Blocker")
    );
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    // Remove from Mado, from a store page only.
    let send = |tab, remove| {
        core.page.send(mado_engine::EngineEvent::PageMessage {
            tab,
            world: mado_engine::tab::World::Mado,
            frame: frame(true, "https://chromewebstore.google.com"),
            message: mado_engine::page::PageMessage::StoreInstall(
                mado_engine::page::StoreInstall {
                    id: id.clone(),
                    remove,
                },
            ),
        });
    };
    let elsewhere = new_tab(&core, w, "https://evil.example/");
    eventually("loaded", || {
        state_of(&core).tabs[&elsewhere].url == "https://evil.example/"
    });
    send(elsewhere, true);
    let store = new_tab(
        &core,
        w,
        &format!("https://chromewebstore.google.com/detail/x/{id}"),
    );
    eventually("loaded", || {
        state_of(&core).tabs[&store].url.contains("chromewebstore")
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(
        state_of(&core).extensions.installed.len(),
        1,
        "not from another site"
    );
    send(store, true);
    eventually("removed", || {
        state_of(&core).extensions.installed.is_empty()
    });
    assert!(!config.dir.join(&id).exists());
}

/// 09 §2.3: Developer mode installs an unpacked folder in place, with
/// the ID Chrome gives it; without Developer mode it is refused.
#[test]
fn developer_mode_installs_an_unpacked_folder() {
    use mado_types::state::PromptState;
    let core = TestCore::start();
    startup(&core);
    let folder = tempfile::tempdir().expect("dir");
    std::fs::write(
        folder.path().join("manifest.json"),
        r#"{"name":"Mine","version":"0.1","manifest_version":3}"#,
    )
    .expect("manifest");
    let install = || {
        ask(
            &core,
            Origin::Settings,
            Command::ExtensionInstall {
                id: None,
                source: mado_types::command::ExtensionSource::Unpacked,
                path: Some(folder.path().to_string_lossy().into_owned()),
            },
        )
    };
    assert_eq!(install().expect_err("off").code, ErrorCode::InvalidInput);
    core.handle.send(Input::Internal(Internal::Hook(
        crate::TestHook::FakeAuthSuccess,
    )));
    let token = ask(
        &core,
        Origin::Settings,
        Command::AppAuthenticate {
            reason: "test".into(),
        },
    )
    .expect("faked")["token"]
        .as_str()
        .map(str::to_owned);
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "extensions.developerMode".into(),
            value: json!(true),
            token,
        },
    )
    .expect("developer mode");
    install().expect("asked");
    eventually("the prompt", || {
        state_of(&core)
            .prompts
            .values()
            .any(|p| matches!(p, PromptState::ExtensionInstall(_)))
    });
    let p = state_of(&core)
        .prompts
        .values()
        .find_map(|p| match p {
            PromptState::ExtensionInstall(p) => Some(p.clone()),
            _ => None,
        })
        .expect("prompt");
    let expected = mado_extensions::id_from_path(&folder.path().canonicalize().expect("path"));
    assert_eq!(p.extension_id.as_str(), expected);
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionConfirmInstall {
            prompt_id: p.id,
            add: true,
        },
    )
    .expect("added");
    let st = state_of(&core);
    assert_eq!(
        st.extensions.installed[0].source,
        mado_types::command::ExtensionSource::Unpacked
    );
    assert!(
        folder.path().join("manifest.json").exists(),
        "left in place"
    );
}

/// M3-T10, 10 §9 check 4 in the core: with no list downloaded and no
/// network, the profile's engine blocks with the bundled EasyList, and its
/// custom filters block too.
#[test]
fn the_blocker_starts_with_the_bundled_easylist_and_custom_filters() {
    let dir = tempfile::tempdir().expect("dir");
    crate::testing::BLOCKER.with_borrow_mut(|b| {
        *b = Some(crate::BlockerConfig {
            dir: dir.path().join("Blocker"),
            update: false,
        });
    });
    let core = TestCore::start();
    crate::testing::BLOCKER.with_borrow_mut(|b| *b = None);
    // The engine takes a second to build the first time.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while state_of(&core).blocker.filters == 0 {
        assert!(std::time::Instant::now() < deadline, "no engine");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let st = state_of(&core).blocker;
    assert!(st.bundled && st.filters > 50_000);
    assert!(
        st.lists
            .iter()
            .any(|l| l.id == "easylist" && l.updated_at.is_none())
    );
    let check = |url: &str| {
        let (tx, rx) = std::sync::mpsc::channel();
        core.handle.send(Input::Internal(Internal::BlockerCheck {
            url: url.into(),
            source: "https://news.example/".into(),
            kind: "script".into(),
            reply: tx,
        }));
        rx.recv().expect("answer")
    };
    assert!(check(
        "https://pagead2.googlesyndication.com/pagead/show_ads.js"
    ));
    assert!(!check("https://news.example/app.js"));
    assert!(!check("https://tracker.mine.example/t.js"));
    let root = core.stop();

    let conn = rusqlite::Connection::open(root.path().join("profile.sqlite")).expect("db");
    conn.execute(
        "INSERT INTO blocker_custom_filters (line, created_at) VALUES ('||tracker.mine.example^', 1)",
        [],
    )
    .expect("insert");
    drop(conn);
    crate::testing::BLOCKER.with_borrow_mut(|b| {
        *b = Some(crate::BlockerConfig {
            dir: dir.path().join("Blocker"),
            update: false,
        });
    });
    let core = TestCore::start_in(root, false);
    crate::testing::BLOCKER.with_borrow_mut(|b| *b = None);
    let check = |url: &str| {
        let (tx, rx) = std::sync::mpsc::channel();
        core.handle.send(Input::Internal(Internal::BlockerCheck {
            url: url.into(),
            source: "https://news.example/".into(),
            kind: "script".into(),
            reply: tx,
        }));
        rx.recv().expect("answer")
    };
    assert!(check("https://tracker.mine.example/t.js"), "custom filter");

    // M3-T11: WebKit gets the shared shards then the profile's own list,
    // and none while the blocker is off.
    let rules = || {
        core.engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .blocker_rules
            .clone()
    };
    for _ in 0..3000 {
        if rules().is_some_and(|r| r.lists.len() >= 2) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        rules().is_some_and(|r| r.lists.len() >= 2),
        "{:?} filters {}",
        rules(),
        state_of(&core).blocker.filters
    );
    let r = rules().expect("rules");
    assert!(r.lists[0].identifier.starts_with("mado-") && r.store.ends_with("Blocker/WebKit"));
    assert!(
        r.lists
            .last()
            .is_some_and(|l| l.identifier.starts_with("mado-site-"))
    );
    let mado_engine::RuleSource::File(shard) = &r.lists[0].source else {
        panic!("a shard file");
    };
    let shard: Vec<Value> =
        serde_json::from_slice(&std::fs::read(shard).expect("shard")).expect("json");
    assert!(shard.len() > 10_000 && shard.len() <= 150_000);
    core.handle.send(Input::Internal(Internal::Hook(
        crate::TestHook::FakeAuthSuccess,
    )));
    let token = ask(
        &core,
        Origin::Settings,
        Command::AppAuthenticate { reason: "t".into() },
    )
    .ok()
    .and_then(|v| v["token"].as_str().map(str::to_owned));
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "blocker.enabled".into(),
            value: json!(false),
            token,
        },
    )
    .expect("off");
    eventually("no rules", || rules().is_some_and(|r| r.lists.is_empty()));
}

/// M3-T06 (09 §6, §11): management commands, protection, the toolbar,
/// popups, options pages and shortcut conflicts, on the fake engine.
#[test]
fn extensions_are_managed_and_shown_in_the_toolbar() {
    use mado_types::state::SiteAccessMode;
    let core = TestCore::start();
    startup(&core);
    let w = *state_of(&core).windows.keys().next().expect("window");
    let folder = tempfile::tempdir().expect("dir");
    std::fs::write(
        folder.path().join("manifest.json"),
        r#"{"name":"Mine","version":"1","manifest_version":3,"action":{"default_popup":"p.html"},
            "options_page":"options.html","permissions":["storage"],"optional_permissions":["tabs"],
            "host_permissions":["https://*.example/*"],
            "commands":{"go":{"description":"Go","suggested_key":{"default":"Ctrl+Shift+Y"}}}}"#,
    )
    .expect("manifest");
    core.handle.send(Input::Internal(Internal::Hook(
        crate::TestHook::FakeAuthSuccess,
    )));
    let token = || {
        ask(
            &core,
            Origin::Settings,
            Command::AppAuthenticate { reason: "t".into() },
        )
        .ok()
        .and_then(|v| v["token"].as_str().map(str::to_owned))
    };
    ask(
        &core,
        Origin::Settings,
        Command::SettingsSet {
            key: "extensions.developerMode".into(),
            value: json!(true),
            token: token(),
        },
    )
    .expect("developer mode");
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionInstall {
            id: None,
            source: mado_types::command::ExtensionSource::Unpacked,
            path: Some(folder.path().to_string_lossy().into_owned()),
        },
    )
    .expect("asked");
    eventually("the prompt", || !state_of(&core).prompts.is_empty());
    let prompt = *state_of(&core).prompts.keys().next().expect("prompt");
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionConfirmInstall {
            prompt_id: prompt,
            add: true,
        },
    )
    .expect("added");
    let ext = state_of(&core).extensions.installed[0].clone();
    let id = ext.id.clone();
    assert_eq!(ext.optional, ["tabs"]);
    assert_eq!(ext.shortcuts[0].keys.as_deref(), Some("Ctrl+Shift+Y"));
    struct Seen {
        extensions: Vec<mado_engine::ExtensionLoad>,
        extension_ops: Vec<String>,
    }
    let engine = || {
        let s = core.engine.lock().unwrap_or_else(|e| e.into_inner());
        Seen {
            extensions: s.extensions.clone(),
            extension_ops: s.extension_ops.clone(),
        }
    };
    eventually("loaded", || engine().extensions.len() == 1);

    // Protected from control clients; allowed for Mado's windows.
    let control = Origin::Control {
        client: ClientId::new(),
    };
    let off = Command::ExtensionSetEnabled {
        id: id.clone(),
        enabled: false,
    };
    assert_eq!(
        ask(&core, control, off.clone())
            .expect_err("protected")
            .code,
        ErrorCode::Protected
    );
    ask(&core, Origin::Settings, off).expect("off");
    eventually("unloaded", || engine().extensions.is_empty());
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionSetEnabled {
            id: id.clone(),
            enabled: true,
        },
    )
    .expect("on");
    eventually("loaded again", || engine().extensions.len() == 1);

    // Site access on click loads no hosts; an optional permission adds.
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionSetSiteAccess {
            id: id.clone(),
            mode: SiteAccessMode::OnClick,
            hosts: None,
        },
    )
    .expect("on click");
    eventually("no hosts", || {
        engine()
            .extensions
            .first()
            .is_some_and(|e| e.hosts.is_empty())
    });
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionGrantOptional {
            id: id.clone(),
            permissions: vec!["tabs".into()],
        },
    )
    .expect("granted");
    eventually("tabs", || {
        engine()
            .extensions
            .first()
            .is_some_and(|e| e.permissions.contains(&"tabs".into()))
    });
    let not_optional = ask(
        &core,
        Origin::Settings,
        Command::ExtensionGrantOptional {
            id: id.clone(),
            permissions: vec!["history".into()],
        },
    );
    assert_eq!(
        not_optional.expect_err("not declared").code,
        ErrorCode::InvalidInput
    );

    // Shortcuts: Mado's own is a conflict naming it.
    let taken = ask(
        &core,
        Origin::Settings,
        Command::ExtensionSetShortcut {
            id: id.clone(),
            command: "go".into(),
            keys: Some("Cmd+T".into()),
        },
    )
    .expect_err("taken");
    assert_eq!(taken.code, ErrorCode::Conflict);
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionSetShortcut {
            id: id.clone(),
            command: "go".into(),
            keys: Some("Cmd+Shift+9".into()),
        },
    )
    .expect("set");
    assert!(
        state_of(&core).extensions.installed[0].shortcuts[0]
            .keys
            .as_deref()
            .is_some_and(|k| k.contains('9'))
    );

    // The toolbar follows the engine's actions for the active tab.
    let tab = state_of(&core).windows[&w].active_tab_id.expect("tab");
    core.page.send(mado_engine::EngineEvent::ExtensionActions {
        tab,
        actions: vec![mado_engine::ExtensionActionInfo {
            id: id.to_string(),
            label: "Mine!".into(),
            badge: "3".into(),
            icon: None,
            has_popup: true,
            enabled: true,
        }],
    });
    eventually("the button", || {
        state_of(&core)
            .extensions
            .toolbar
            .first()
            .is_some_and(|b| b.badge_text == "3" && b.has_popup)
    });
    ask(
        &core,
        Origin::Ui { window: w },
        Command::ExtensionOpenPopup {
            id: id.clone(),
            window_id: w,
            anchor: mado_types::state::Rect {
                x: 1.0,
                y: 2.0,
                width: 20.0,
                height: 20.0,
            },
        },
    )
    .expect("popup");
    eventually("the action", || {
        engine()
            .extension_ops
            .iter()
            .any(|o| o.starts_with("Action("))
    });

    // Options open in a tab under the extension's base URL.
    core.page.send(mado_engine::EngineEvent::ExtensionLoaded {
        id: id.to_string(),
        base_url: "webkit-extension://abc/".into(),
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionOpenOptions { id: id.clone() },
    )
    .expect("options");
    eventually("the options tab", || {
        state_of(&core)
            .tabs
            .values()
            .any(|t| t.url == "webkit-extension://abc/options.html")
    });
    let errors = ask(
        &core,
        Origin::Settings,
        Command::ExtensionErrors { id: id.clone() },
    )
    .expect("errors");
    assert!(errors[0].as_str().is_some_and(|e| e.contains("no errors")));

    // Pin, then remove.
    ask(
        &core,
        Origin::Ui { window: w },
        Command::ExtensionPin {
            id: id.clone(),
            pinned: false,
        },
    )
    .expect("unpin");
    assert!(state_of(&core).extensions.pinned.is_empty());
    ask(
        &core,
        Origin::Settings,
        Command::ExtensionRemove {
            id: id.clone(),
            delete_data: true,
        },
    )
    .expect("removed");
    assert!(state_of(&core).extensions.installed.is_empty());
    eventually("unloaded", || engine().extensions.is_empty());
}
