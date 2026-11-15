//! A core over temporary databases and the fake engine, for tests.

use std::sync::{Arc, Mutex};

use mado_engine::{EngineHandle, EventSink};
use mado_engine_fake::{FakeControl, FakeEngine, FakeState};
use mado_settings::{Settings, Strings};
use mado_storage::{Kind, Storage, open};
use mado_types::Patch;
use mado_types::state::{Notice, ProfileInfo};

use crate::{CoreConfig, CoreHandle, Input, UiSink};

#[derive(Debug, Default)]
pub struct Sent {
    pub patches: Vec<Patch>,
    pub notices: Vec<Notice>,
    /// Pages handed to Safari.
    pub safari: Vec<String>,
    /// Links handed to other apps.
    pub launched: Vec<String>,
    /// Text put on the pasteboard.
    pub copied: Vec<String>,
    /// URLs handed to other profiles.
    pub other_profiles: Vec<(mado_types::ProfileId, String)>,
    /// Temporary windows asked for: guest or not, and their URLs.
    pub temporary: Vec<(bool, Vec<String>)>,
    /// How many times the core asked the app to quit.
    pub quits: usize,
}

struct Collect(Arc<Mutex<Sent>>);

impl UiSink for Collect {
    fn quit(&mut self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).quits += 1;
    }
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
    fn open_in_profile(&mut self, profile: mado_types::ProfileId, url: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .other_profiles
            .push((profile, url.to_owned()));
    }
    fn open_temporary(&mut self, guest: bool, urls: &[String]) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .temporary
            .push((guest, urls.to_vec()));
    }
    fn copy_text(&mut self, text: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .copied
            .push(text.to_owned());
    }
    fn launch_app_link(&mut self, url: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .launched
            .push(url.to_owned());
    }
    fn open_in_safari(&mut self, url: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .safari
            .push(url.to_owned());
    }
}

thread_local! {
    /// The profile service the next `TestCore` on this thread uses.
    pub static PROFILES: std::cell::RefCell<std::sync::Arc<dyn crate::ui::Profiles>> =
        std::cell::RefCell::new(std::sync::Arc::new(crate::ui::NoProfiles));
    /// Whether the next `TestCore` on this thread is a private profile.
    pub static PRIVATE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The captive portal probe the next `TestCore` on this thread uses.
    pub static CAPTIVE_PROBE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub struct TestCore {
    pub handle: CoreHandle,
    pub sent: Arc<Mutex<Sent>>,
    pub engine: Arc<Mutex<FakeState>>,
    pub page: FakeControl,
    pub dir: tempfile::TempDir,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl TestCore {
    pub fn start() -> Self {
        Self::start_in(tempfile::tempdir().expect("temp dir"), true)
    }

    /// A core over the databases in `dir`, as a relaunch would find them.
    pub fn start_in(dir: tempfile::TempDir, clean_start: bool) -> Self {
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
        let events_tx = Mutex::new(events_tx);
        let engine = FakeEngine::new(EventSink::new(move |e| {
            if let Ok(tx) = events_tx.lock() {
                let _ = tx.send(e);
            }
        }));
        let engine_state = engine.state();
        let page = engine.control();
        let sink = EventSink::new({
            let page = page.clone();
            move |e| page.send(e)
        });
        let engine = Arc::new(Mutex::new(engine));
        // The main thread's role: run each call on the engine.
        let engine = EngineHandle::new(move |op| {
            let mut engine = engine.lock().unwrap_or_else(|e| e.into_inner());
            op.run(&mut *engine, &sink);
        });
        let (handle, thread) = crate::start(CoreConfig {
            profile: ProfileInfo {
                name: "Test".into(),
                is_private: PRIVATE.get(),
                ..ProfileInfo::default()
            },
            app_db: Storage::start(&app).expect("app storage"),
            profile_db: Storage::start(&profile).expect("profile storage"),
            backups,
            settings: Settings::new(Strings::english()),
            engine,
            captive_probe: CAPTIVE_PROBE.with_borrow(Clone::clone),
            profiles: PROFILES.with_borrow(Clone::clone),
            hsts_extra: ["hsts.example".to_owned()].into(),
            // Camera denied at the OS, everything else answered yes.
            os: crate::OsPermissions {
                status: |p| match p {
                    mado_platform::permissions::OsPermission::Camera => {
                        mado_platform::permissions::OsStatus::Denied
                    }
                    _ => mado_platform::permissions::OsStatus::Granted,
                },
                request: |_| mado_platform::permissions::OsStatus::Granted,
                // Only zoommtg: and mailto: have an app here.
                handler: |url| {
                    let name = if url.starts_with("zoommtg:") {
                        "zoom.us"
                    } else if url.starts_with("mailto:") {
                        "Mail"
                    } else {
                        return None;
                    };
                    Some(mado_platform::applinks::Handler {
                        name: name.into(),
                        icon_png: Some(vec![137, 80, 78, 71]),
                    })
                },
                identities: || {
                    vec![mado_platform::keychain::ClientIdentity {
                        id: "cert-1".into(),
                        label: "Ada Lovelace".into(),
                    }]
                },
            },
            user_agent: mado_engine::UserAgent::EngineDefault,
            clean_start,
            page_scripts: mado_engine::PageScriptSet::default(),
            ui: Box::new(Collect(Arc::clone(&sent))),
            updates: Box::new(crate::NoUpdates),
            clock: || 1_800_000_000_000,
        })
        .expect("core starts");
        // The main thread's role: hand engine events back to the core.
        let forward = handle.clone();
        std::thread::spawn(move || {
            while let Ok(e) = events_rx.recv() {
                if !forward.send(Input::Engine(e)) {
                    break;
                }
            }
        });
        Self {
            handle,
            sent,
            engine: engine_state,
            page,
            dir,
            thread: Some(thread),
        }
    }
}

impl TestCore {
    /// Quits the way the app does and hands back the databases.
    pub fn stop(mut self) -> tempfile::TempDir {
        let (done, rx) = tokio::sync::oneshot::channel();
        self.handle
            .send(Input::Internal(crate::Internal::Shutdown { done }));
        let _ = rx.blocking_recv();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.dir
    }
}
