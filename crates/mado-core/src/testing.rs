//! A core over temporary databases and the fake engine, for tests.

use std::sync::{Arc, Mutex};

use mado_engine_fake::{FakeEngine, FakeState};
use mado_settings::{Settings, Strings};
use mado_storage::{Kind, Storage, open};
use mado_types::Patch;
use mado_types::state::{Notice, ProfileInfo};

use crate::{CoreConfig, CoreHandle, Input, UiSink};

#[derive(Debug, Default)]
pub struct Sent {
    pub patches: Vec<Patch>,
    pub notices: Vec<Notice>,
}

struct Collect(Arc<Mutex<Sent>>);

impl UiSink for Collect {
    fn patch(&mut self, patch: &Patch) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .patches
            .push(patch.clone());
    }
    fn notice(&mut self, notice: &Notice) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .notices
            .push(notice.clone());
    }
}

pub struct TestCore {
    pub handle: CoreHandle,
    pub sent: Arc<Mutex<Sent>>,
    pub engine: Arc<Mutex<FakeState>>,
    pub dir: tempfile::TempDir,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl TestCore {
    pub fn start() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let backups = root.join("Backups");
        let app = open(
            &root.join("app.sqlite"),
            Kind::App,
            &backups,
            &root.join("Broken"),
        )
        .expect("app db");
        let profile = open(
            &root.join("profile.sqlite"),
            Kind::Profile,
            &backups,
            &root.join("Broken"),
        )
        .expect("profile db");
        let sent = Arc::new(Mutex::new(Sent::default()));
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        let engine = FakeEngine::new(move |e| {
            let _ = events_tx.send(e);
        });
        let engine_state = engine.state();
        let (handle, thread) = crate::start(CoreConfig {
            profile: ProfileInfo {
                name: "Test".into(),
                ..ProfileInfo::default()
            },
            app_db: Storage::start(&app).expect("app storage"),
            profile_db: Storage::start(&profile).expect("profile storage"),
            backups,
            settings: Settings::new(Strings::english()),
            engine: Box::new(engine),
            ui: Box::new(Collect(Arc::clone(&sent))),
            updates: Box::new(crate::NoUpdates),
            clock: || 1_800_000_000_000,
        })
        .expect("core starts");
        // The main thread's role: hand engine events back to the core.
        let forward = handle.clone();
        std::thread::spawn(move || {
            while let Ok(e) = events_rx.recv() {
                if !forward.send(Input::Window(e)) {
                    break;
                }
            }
        });
        Self {
            handle,
            sent,
            engine: engine_state,
            dir,
            thread: Some(thread),
        }
    }
}
