//! A deterministic engine for tests (roadmap M1-T06). It keeps the windows
//! it was asked to create and reports each result through a callback, the
//! way the main thread reports to the core. Pages, navigation, permissions
//! and downloads join it with the `Engine` trait in M2-T01.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mado_engine::{WindowCommand, WindowEngine, WindowEvent};
use mado_types::WindowId;
use mado_types::state::{Bounds, WindowKind, WindowStateKind};

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

/// What the fake engine holds; shared so tests can look inside.
#[derive(Debug, Default)]
pub struct FakeState {
    pub windows: BTreeMap<WindowId, FakeWindow>,
    /// Every command, in order.
    pub log: Vec<(WindowId, WindowCommand)>,
}

type Report = Box<dyn Fn(WindowEvent) + Send>;

pub struct FakeEngine {
    state: Arc<Mutex<FakeState>>,
    report: Report,
}

impl std::fmt::Debug for FakeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeEngine")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl FakeEngine {
    /// `report` receives the engine's events, as the core's input would.
    pub fn new(report: impl Fn(WindowEvent) + Send + 'static) -> Self {
        Self {
            state: Arc::default(),
            report: Box::new(report),
        }
    }

    pub fn state(&self) -> Arc<Mutex<FakeState>> {
        Arc::clone(&self.state)
    }
}

impl WindowEngine for FakeEngine {
    fn apply_window(&mut self, window: WindowId, cmd: WindowCommand) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.log.push((window, cmd.clone()));
        let mut event = None;
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
                event = Some(WindowEvent::Created { window: id });
            }
            WindowCommand::Close => {
                state.windows.remove(&window);
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
        if let Some(event) = event {
            (self.report)(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn creating_a_window_reports_it_and_later_commands_change_it() {
        let (tx, rx) = mpsc::channel();
        let mut engine = FakeEngine::new(move |e| {
            let _ = tx.send(e);
        });
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
        assert_eq!(rx.try_recv(), Ok(WindowEvent::Created { window: id }));
        engine.apply_window(id, WindowCommand::SetTitle("Mado".into()));
        engine.apply_window(
            id,
            WindowCommand::ShowOverlay {
                rect: Default::default(),
            },
        );
        let state = engine.state();
        let state = state.lock().expect("lock");
        assert_eq!(state.windows[&id].title, "Mado");
        assert!(state.windows[&id].overlay);
        assert_eq!(state.log.len(), 3);
    }
}
