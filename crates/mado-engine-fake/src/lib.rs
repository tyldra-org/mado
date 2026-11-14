//! A deterministic engine for tests (spec 16 §1, core integration). It keeps
//! the windows and tab views the core asked for, simulates pages and their
//! navigation (every navigation passes `DecidePolicy` first, as on WebKit),
//! and reports each result through an [`EventSink`], the way the main thread
//! reports to the core. Tests drive page-side behaviour through
//! [`FakeControl`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use mado_engine::event::{EngineSecurity, GoneReason};
use mado_engine::tab::{Image, ImageFormat, Modifiers, SessionBlob};
use mado_engine::{
    Callback, Capabilities, ClearDataRequest, Engine, EngineError, EngineEvent, EventSink,
    NavDecision, NavKind, NavPhase, NavRequest, PageScriptSet, ProxyConfig, Reply, TabCommand,
    TabInitial, TabSpec, WindowCommand,
};
use mado_types::state::{Bounds, Rect, WindowKind, WindowStateKind};
use mado_types::{TabId, WindowId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A window as the fake engine sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct FakeWindow {
    pub bounds: Bounds,
    pub kind: WindowKind,
    pub private: bool,
    pub state: WindowStateKind,
    pub title: String,
    pub overlay: bool,
    pub focused: bool,
}

/// A tab view as the fake engine sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct FakeTab {
    pub window: WindowId,
    pub private: bool,
    pub agent: bool,
    /// Back and forward history; `index` is the current entry.
    pub history: Vec<String>,
    pub index: usize,
    pub zoom: f64,
    pub muted: bool,
    pub media_suspended: bool,
    pub visible: bool,
    pub frame: Option<Rect>,
    /// Applies from the next navigation, as on WebKit.
    pub javascript: bool,
    /// What the current page was loaded with.
    pub page_javascript: bool,
    pub focused: bool,
}

impl FakeTab {
    pub fn url(&self) -> Option<&str> {
        self.history.get(self.index).map(String::as_str)
    }
}

#[derive(Serialize, Deserialize)]
struct Session {
    history: Vec<String>,
    index: usize,
}

/// What the fake engine holds; shared so tests can look inside.
#[derive(Debug, Default)]
pub struct FakeState {
    pub windows: BTreeMap<WindowId, FakeWindow>,
    pub tabs: BTreeMap<TabId, FakeTab>,
    /// Every window command, in order.
    pub log: Vec<(WindowId, WindowCommand)>,
    /// Every tab command's name, in order.
    pub tab_log: Vec<(TabId, &'static str)>,
    /// Every navigation decision the core gave, in order.
    pub decisions: Vec<(TabId, String, NavDecision)>,
    pub page_scripts: PageScriptSet,
    pub proxy: ProxyConfig,
    pub cleared: Vec<ClearDataRequest>,
    next_nav: u64,
}

fn lock(state: &Mutex<FakeState>) -> MutexGuard<'_, FakeState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Drives the page side of the fake engine from a test.
#[derive(Debug, Clone)]
pub struct FakeControl {
    state: Arc<Mutex<FakeState>>,
    events: EventSink,
}

impl FakeControl {
    /// The page itself navigates (a link, a form, script), from its current
    /// URL. The core decides; an allowed navigation commits.
    pub fn page_navigates(&self, tab: TabId, url: &str, user_gesture: bool) {
        let source = lock(&self.state)
            .tabs
            .get(&tab)
            .and_then(|t| t.url().map(str::to_owned));
        self.navigate(tab, url, source, false, user_gesture, NavKind::Link);
    }

    /// The page asks for a decision on a subframe navigation; nothing commits.
    pub fn subframe_navigates(&self, tab: TabId, url: &str) {
        let source = lock(&self.state)
            .tabs
            .get(&tab)
            .and_then(|t| t.url().map(str::to_owned));
        let request = NavRequest {
            url: url.to_owned(),
            source_url: source,
            is_main_frame: false,
            user_gesture: false,
            initiated_by_mado: false,
            kind: NavKind::Other,
            phase: NavPhase::Action {
                modifiers: Modifiers::default(),
                method: "GET".into(),
            },
        };
        self.decide(tab, request, |_, _, _| {});
    }

    /// A response arrived; the core decides whether it downloads.
    pub fn response(&self, tab: TabId, url: &str, mime: &str, attachment: bool, can_show: bool) {
        let request = NavRequest {
            url: url.to_owned(),
            source_url: None,
            is_main_frame: true,
            user_gesture: false,
            initiated_by_mado: false,
            kind: NavKind::Other,
            phase: NavPhase::Response {
                mime: mime.to_owned(),
                attachment,
                can_show,
            },
        };
        self.decide(tab, request, |_, _, _| {});
    }

    /// The tab's content process dies.
    pub fn crash(&self, tab: TabId) {
        self.events.send(EngineEvent::ProcessGone {
            tab,
            reason: GoneReason::Crashed,
        });
    }

    /// Raises any event, for paths the fake does not simulate.
    pub fn send(&self, event: EngineEvent) {
        self.events.send(event);
    }

    fn navigate(
        &self,
        tab: TabId,
        url: &str,
        source_url: Option<String>,
        initiated_by_mado: bool,
        user_gesture: bool,
        kind: NavKind,
    ) {
        let request = NavRequest {
            url: url.to_owned(),
            source_url,
            is_main_frame: true,
            user_gesture,
            initiated_by_mado,
            kind,
            phase: NavPhase::Action {
                modifiers: Modifiers::default(),
                method: "GET".into(),
            },
        };
        self.decide(tab, request, move |control, decision, url| match decision {
            NavDecision::Allow => control.commit(tab, &url, kind, true),
            NavDecision::Redirect { url: to } => {
                control.navigate(tab, &to, None, true, user_gesture, NavKind::Redirect);
            }
            NavDecision::Cancel | NavDecision::OpenExternal { .. } | NavDecision::Download => {}
        });
    }

    /// Asks the core, records its answer, then runs `then`.
    fn decide(
        &self,
        tab: TabId,
        request: NavRequest,
        then: impl FnOnce(&FakeControl, NavDecision, String) + Send + 'static,
    ) {
        let control = self.clone();
        let url = request.url.clone();
        let (reply, _expiry) = Reply::new(NavDecision::Cancel, move |decision: NavDecision| {
            lock(&control.state)
                .decisions
                .push((tab, url.clone(), decision.clone()));
            then(&control, decision, url);
        });
        self.events.send(EngineEvent::DecidePolicy {
            tab,
            request,
            reply,
        });
    }

    /// Loads `url` as the current page: a new history entry, or the same
    /// entry for a reload or history move.
    fn commit(&self, tab: TabId, url: &str, kind: NavKind, new_entry: bool) {
        let nav_id;
        let (back, forward) = {
            let mut state = lock(&self.state);
            state.next_nav += 1;
            nav_id = state.next_nav;
            let Some(t) = state.tabs.get_mut(&tab) else {
                return;
            };
            if new_entry {
                if t.history.is_empty() {
                    t.history.push(url.to_owned());
                    t.index = 0;
                } else {
                    t.history.truncate(t.index + 1);
                    t.history.push(url.to_owned());
                    t.index = t.history.len() - 1;
                }
            }
            t.page_javascript = t.javascript;
            (t.index > 0, t.index + 1 < t.history.len())
        };
        let url = url.to_owned();
        for event in [
            EngineEvent::NavigationStarted {
                tab,
                url: url.clone(),
                nav_id,
                is_main_frame: true,
                user_gesture: false,
                kind,
            },
            EngineEvent::NavigationCommitted {
                tab,
                nav_id,
                url: url.clone(),
                is_same_document: false,
            },
            EngineEvent::UrlChanged {
                tab,
                url: url.clone(),
            },
            EngineEvent::TitleChanged {
                tab,
                title: url.clone(),
            },
            EngineEvent::SecurityChanged {
                tab,
                state: EngineSecurity {
                    secure: url.starts_with("https:"),
                    ..EngineSecurity::default()
                },
            },
            EngineEvent::HistoryStateChanged {
                tab,
                can_go_back: back,
                can_go_forward: forward,
            },
            EngineEvent::ProgressChanged { tab, progress: 1.0 },
            EngineEvent::NavigationFinished { tab, nav_id },
        ] {
            self.events.send(event);
        }
    }

    /// Moves to history entry `to` and commits it.
    fn go(&self, tab: TabId, to: i64) -> Result<(), EngineError> {
        let url = {
            let mut state = lock(&self.state);
            let t = state.tabs.get_mut(&tab).ok_or(EngineError::NoTab)?;
            let to = usize::try_from(to)
                .ok()
                .filter(|i| *i < t.history.len())
                .ok_or_else(|| EngineError::Failed("no such history entry".into()))?;
            t.index = to;
            t.history[to].clone()
        };
        self.commit(tab, &url, NavKind::BackForward, false);
        Ok(())
    }
}

pub struct FakeEngine {
    caps: Capabilities,
    control: FakeControl,
}

impl std::fmt::Debug for FakeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeEngine").finish_non_exhaustive()
    }
}

impl FakeEngine {
    /// `events` receives the engine's events, as the core's input would.
    pub fn new(events: EventSink) -> Self {
        Self {
            caps: Capabilities::webkit(),
            control: FakeControl {
                state: Arc::default(),
                events,
            },
        }
    }

    pub fn state(&self) -> Arc<Mutex<FakeState>> {
        Arc::clone(&self.control.state)
    }

    pub fn control(&self) -> FakeControl {
        self.control.clone()
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        lock(&self.control.state)
    }
}

impl Engine for FakeEngine {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn create_tab(&mut self, spec: TabSpec) -> Result<(), EngineError> {
        if !self.lock().windows.contains_key(&spec.window) {
            return Err(EngineError::NoWindow);
        }
        let (history, index) = match &spec.initial {
            TabInitial::Restore(blob) => {
                let session: Session = serde_json::from_slice(&blob.0)
                    .map_err(|e| EngineError::Failed(format!("bad session: {e}")))?;
                (session.history, session.index)
            }
            _ => (Vec::new(), 0),
        };
        self.lock().tabs.insert(
            spec.tab,
            FakeTab {
                window: spec.window,
                private: spec.private,
                agent: spec.agent,
                history,
                index,
                zoom: spec.zoom,
                muted: false,
                media_suspended: false,
                visible: spec.visible,
                frame: spec.frame,
                javascript: spec.javascript,
                page_javascript: spec.javascript,
                focused: false,
            },
        );
        self.control.send(EngineEvent::TabCreated { tab: spec.tab });
        match spec.initial {
            TabInitial::Url(url) => {
                self.control
                    .navigate(spec.tab, &url, None, true, false, NavKind::Typed);
            }
            TabInitial::Popup(request) => {
                self.control.navigate(
                    spec.tab,
                    &request.url,
                    None,
                    false,
                    request.user_gesture,
                    NavKind::Link,
                );
            }
            TabInitial::Restore(_) => {
                let url = self.lock().tabs[&spec.tab].url().map(str::to_owned);
                if let Some(url) = url {
                    self.control.commit(spec.tab, &url, NavKind::Restore, false);
                }
            }
            TabInitial::Blank => {}
        }
        Ok(())
    }

    fn destroy_tab(&mut self, tab: TabId) {
        self.lock().tabs.remove(&tab);
    }

    fn apply(&mut self, tab: TabId, cmd: TabCommand) -> Result<(), EngineError> {
        let exists = {
            let mut state = self.lock();
            let exists = state.tabs.contains_key(&tab);
            if exists {
                state.tab_log.push((tab, cmd.name()));
            }
            exists
        };
        if !exists {
            cmd.fail(EngineError::NoTab);
            return Err(EngineError::NoTab);
        }
        let control = self.control.clone();
        let mut state = self.lock();
        let t = state.tabs.get_mut(&tab).ok_or(EngineError::NoTab)?;
        match cmd {
            TabCommand::Navigate { url, kind, .. } => {
                drop(state);
                control.navigate(tab, &url, None, true, false, kind);
            }
            TabCommand::Reload { .. } => {
                let url = t.url().map(str::to_owned);
                drop(state);
                if let Some(url) = url {
                    control.commit(tab, &url, NavKind::Reload, false);
                }
            }
            TabCommand::GoBack => {
                let to = i64::try_from(t.index).unwrap_or(0) - 1;
                drop(state);
                control.go(tab, to)?;
            }
            TabCommand::GoForward => {
                let to = i64::try_from(t.index).unwrap_or(0) + 1;
                drop(state);
                control.go(tab, to)?;
            }
            TabCommand::GoToIndex(offset) => {
                let to = i64::try_from(t.index).unwrap_or(0) + i64::from(offset);
                drop(state);
                control.go(tab, to)?;
            }
            TabCommand::SetFrame(rect) => t.frame = Some(rect),
            TabCommand::SetVisible(v) => t.visible = v,
            TabCommand::Focus => t.focused = true,
            TabCommand::SetZoom(z) => t.zoom = z,
            TabCommand::SetMuted(m) => t.muted = m,
            TabCommand::SetMediaSuspended(s) => t.media_suspended = s,
            TabCommand::SetJavaScriptEnabled(js) => t.javascript = js,
            TabCommand::Find(_) | TabCommand::FindNext { .. } => {
                drop(state);
                control.send(EngineEvent::FindResult {
                    tab,
                    matches: Some(0),
                    active: None,
                });
            }
            TabCommand::Snapshot { reply, .. } => {
                drop(state);
                reply(Ok(Image {
                    width: 1,
                    height: 1,
                    format: ImageFormat::Png,
                    bytes: Vec::new(),
                }));
            }
            TabCommand::EvaluateInWorld { reply, .. } => {
                drop(state);
                reply(Ok(Value::Null));
            }
            TabCommand::SaveSession { reply } => {
                let blob = serde_json::to_vec(&Session {
                    history: t.history.clone(),
                    index: t.index,
                })
                .map(SessionBlob)
                .map_err(|e| EngineError::Failed(e.to_string()));
                drop(state);
                reply(blob);
            }
            TabCommand::Stop
            | TabCommand::FindClear
            | TabCommand::Print
            | TabCommand::SavePdf { .. }
            | TabCommand::SaveArchive { .. }
            | TabCommand::PostToPage { .. }
            | TabCommand::ShowDevTools
            | TabCommand::ShowSource
            | TabCommand::SetReader(_)
            | TabCommand::SetImagesEnabled(_)
            | TabCommand::ExitFullscreen
            | TabCommand::MediaControl(_) => {}
        }
        Ok(())
    }

    fn apply_window(&mut self, window: WindowId, cmd: WindowCommand) {
        let mut state = self.lock();
        state.log.push((window, cmd.clone()));
        let mut created = None;
        match cmd {
            WindowCommand::Create {
                window: id,
                bounds,
                kind,
                private,
            } => {
                state.windows.insert(
                    id,
                    FakeWindow {
                        bounds,
                        kind,
                        private,
                        state: WindowStateKind::Normal,
                        title: String::new(),
                        overlay: false,
                        focused: true,
                    },
                );
                created = Some(id);
            }
            WindowCommand::Close => {
                state.windows.remove(&window);
                state.tabs.retain(|_, t| t.window != window);
            }
            other => {
                if let Some(w) = state.windows.get_mut(&window) {
                    match other {
                        WindowCommand::Focus => w.focused = true,
                        WindowCommand::SetBounds(b) => w.bounds = b,
                        WindowCommand::SetState(s) => w.state = s,
                        WindowCommand::ShowOverlay { .. } => w.overlay = true,
                        WindowCommand::HideOverlay => w.overlay = false,
                        WindowCommand::SetTitle(t) => w.title = t,
                        WindowCommand::Create { .. } | WindowCommand::Close => {}
                    }
                }
            }
        }
        drop(state);
        if let Some(window) = created {
            self.control.send(EngineEvent::WindowCreated { window });
        }
    }

    fn set_page_scripts(&mut self, scripts: PageScriptSet) {
        self.lock().page_scripts = scripts;
    }

    fn clear_data(&mut self, req: ClearDataRequest, done: Callback<Result<(), EngineError>>) {
        self.lock().cleared.push(req);
        done(Ok(()));
    }

    fn set_proxy(&mut self, proxy: ProxyConfig) {
        self.lock().proxy = proxy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn engine() -> (FakeEngine, mpsc::Receiver<EngineEvent>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let engine = FakeEngine::new(EventSink::new(move |e| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.send(e);
            }
        }));
        (engine, rx)
    }

    fn window(engine: &mut FakeEngine) -> WindowId {
        let id = WindowId::new();
        engine.apply_window(
            id,
            WindowCommand::Create {
                window: id,
                bounds: Bounds::default(),
                kind: WindowKind::Normal,
                private: false,
            },
        );
        id
    }

    fn spec(window: WindowId, initial: TabInitial) -> TabSpec {
        TabSpec {
            tab: TabId::new(),
            window,
            initial,
            private: false,
            agent: false,
            user_agent: mado_engine::UserAgent::EngineDefault,
            zoom: 1.0,
            visible: true,
            frame: None,
            javascript: true,
        }
    }

    /// Answers every decision with `decide`, and collects the other events.
    fn drain(
        rx: &mpsc::Receiver<EngineEvent>,
        decide: impl Fn(&NavRequest) -> NavDecision,
    ) -> Vec<&'static str> {
        let mut names = Vec::new();
        while let Ok(event) = rx.try_recv() {
            names.push(event.name());
            if let EngineEvent::DecidePolicy { request, reply, .. } = event {
                reply.send(decide(&request));
            }
        }
        names
    }

    #[test]
    fn creating_a_window_reports_it_and_later_commands_change_it() {
        let (mut engine, rx) = engine();
        let id = window(&mut engine);
        assert!(matches!(rx.try_recv(), Ok(EngineEvent::WindowCreated { window }) if window == id));
        engine.apply_window(id, WindowCommand::SetTitle("Mado".into()));
        let state = engine.state();
        assert_eq!(lock(&state).windows[&id].title, "Mado");
    }

    #[test]
    fn navigation_waits_for_the_core_and_commits_only_when_allowed() {
        let (mut engine, rx) = engine();
        let w = window(&mut engine);
        let s = spec(w, TabInitial::Url("https://a.example/".into()));
        let tab = s.tab;
        engine.create_tab(s).expect("created");
        let names = drain(&rx, |_| NavDecision::Allow);
        assert!(names.contains(&"decidePolicy"));
        assert!(names.contains(&"navigationCommitted"));
        engine
            .apply(
                tab,
                TabCommand::Navigate {
                    url: "https://b.example/".into(),
                    referrer: None,
                    kind: NavKind::Typed,
                },
            )
            .expect("applied");
        let names = drain(&rx, |_| NavDecision::Cancel);
        assert!(!names.contains(&"navigationCommitted"));
        let state = engine.state();
        assert_eq!(lock(&state).tabs[&tab].url(), Some("https://a.example/"));
    }

    #[test]
    fn a_saved_session_restores_back_and_forward_history() {
        let (mut engine, rx) = engine();
        let w = window(&mut engine);
        let s = spec(w, TabInitial::Url("https://a.example/".into()));
        let tab = s.tab;
        engine.create_tab(s).expect("created");
        drain(&rx, |_| NavDecision::Allow);
        engine
            .control()
            .page_navigates(tab, "https://b.example/", true);
        drain(&rx, |_| NavDecision::Allow);
        engine.apply(tab, TabCommand::GoBack).expect("back");
        let (tx, blob_rx) = mpsc::channel();
        engine
            .apply(
                tab,
                TabCommand::SaveSession {
                    reply: Box::new(move |b| {
                        let _ = tx.send(b);
                    }),
                },
            )
            .expect("saved");
        let blob = blob_rx.recv().expect("blob").expect("ok");
        engine.destroy_tab(tab);
        let restored = spec(w, TabInitial::Restore(blob));
        let tab2 = restored.tab;
        engine.create_tab(restored).expect("restored");
        let state = engine.state();
        let t = lock(&state).tabs[&tab2].clone();
        assert_eq!(t.history, ["https://a.example/", "https://b.example/"]);
        assert_eq!(t.url(), Some("https://a.example/"));
    }

    #[test]
    fn javascript_changes_apply_from_the_next_navigation() {
        let (mut engine, rx) = engine();
        let w = window(&mut engine);
        let s = spec(w, TabInitial::Url("https://a.example/".into()));
        let tab = s.tab;
        engine.create_tab(s).expect("created");
        drain(&rx, |_| NavDecision::Allow);
        engine
            .apply(tab, TabCommand::SetJavaScriptEnabled(false))
            .expect("set");
        let state = engine.state();
        assert!(lock(&state).tabs[&tab].page_javascript);
        engine
            .apply(
                tab,
                TabCommand::Reload {
                    bypass_cache: false,
                },
            )
            .expect("reload");
        assert!(!lock(&state).tabs[&tab].page_javascript);
    }

    #[test]
    fn commands_for_a_missing_tab_fail_and_answer_their_callback() {
        let (mut engine, _rx) = engine();
        let (tx, rx) = mpsc::channel();
        let result = engine.apply(
            TabId::new(),
            TabCommand::EvaluateInWorld {
                world: mado_engine::World::Mado,
                source: "1".into(),
                reply: Box::new(move |r| {
                    let _ = tx.send(r);
                }),
            },
        );
        assert_eq!(result, Err(EngineError::NoTab));
        assert_eq!(rx.try_recv(), Ok(Err(EngineError::NoTab)));
    }
}
