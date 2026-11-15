//! Page scripts in the core (spec 02 §5.0, §5.3): handlers for the page
//! messages M2 owns, the heartbeat and the "Page unresponsive" prompt, and
//! the tab commands that act on the page (spec 01 §4.2).

use std::path::PathBuf;
use std::time::Duration;

use mado_engine::event::FrameInfo;
use mado_engine::page::{Icon, PageMessage};
use mado_engine::{EngineOp, TabCommand, World};
use mado_types::command::{Command, SaveFormat};
use mado_types::state::{FindState, PromptState, TabLife, UnresponsivePrompt};
use mado_types::{ErrorCode, MadoError, PatchRecorder, PromptId, TabId};
use serde_json::{Value, json};

use super::Core;

/// How often the core looks for silent pages, and how long is too long.
pub const HEARTBEAT_CHECK: Duration = Duration::from_secs(2);
const SILENT_FOR: i64 = 10_000;

fn not_found(key: &str) -> MadoError {
    MadoError::new(ErrorCode::NotFound, key)
}

/// The icon to show: 32 px, then a scalable one, then the first.
fn best_icon(icons: &[Icon]) -> Option<&Icon> {
    let has = |i: &&Icon, s: &str| {
        i.sizes
            .as_deref()
            .is_some_and(|x| x.split_whitespace().any(|y| y.eq_ignore_ascii_case(s)))
    };
    icons
        .iter()
        .find(|i| has(i, "32x32"))
        .or_else(|| {
            icons
                .iter()
                .find(|i| has(i, "any") || i.url.ends_with(".svg"))
        })
        .or_else(|| {
            icons
                .iter()
                .find(|i| i.rel.split_whitespace().any(|r| r == "icon"))
        })
        .or_else(|| icons.first())
}

impl Core {
    fn post(&mut self, tab: TabId, message: Value) {
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::PostToPage {
                world: World::Mado,
                message,
            },
        ));
    }

    /// One validated page message (spec 02 §5.0).
    pub(super) fn page_message(
        &mut self,
        tab: TabId,
        frame: &FrameInfo,
        message: PageMessage,
        rec: &mut PatchRecorder,
    ) {
        if !self.model.tabs.contains_key(&tab) {
            return;
        }
        let now = self.now();
        match message {
            PageMessage::Heartbeat(_) if frame.is_main_frame => {
                self.heartbeats.insert(tab, now);
                self.responsive(tab, rec);
            }
            PageMessage::HoverLink(h) if frame.is_main_frame => {
                let window = self.model.tabs[&tab].window_id;
                if let Some(w) = self.model.windows.get_mut(&window)
                    && w.active_tab_id == Some(tab)
                {
                    w.hover_link = h.url;
                }
            }
            PageMessage::Favicons(f) if frame.is_main_frame => {
                // The icon's own URL until the favicon cache serves it as
                // `mado-favicon://` (M2-T10).
                let url = best_icon(&f.icons).map(|i| i.url.clone());
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.favicon = url;
                }
            }
            PageMessage::PageText(p) if frame.is_main_frame => self.page_text(tab, p.text),
            PageMessage::ContextTarget(c) => {
                self.context_targets.insert(tab, c);
            }
            PageMessage::FindResult(f) => {
                let window = self.model.tabs[&tab].window_id;
                if let Some(w) = self.model.windows.get_mut(&window)
                    && w.active_tab_id == Some(tab)
                    && let Some(bar) = w.find_bar.as_mut()
                {
                    bar.matches = f.matches;
                    bar.active = f.active;
                }
            }
            PageMessage::MediaState(m) => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.audio.playing = m.playing && !m.muted;
                }
                let mut playing = self.state.media.playing.clone();
                playing.retain(|t| *t != tab);
                if m.playing {
                    playing.push(tab);
                }
                if playing != self.state.media.playing {
                    self.state.tx(rec).media().set_playing(playing);
                }
            }
            PageMessage::DrmFailed(_) => {
                // The "open in Safari" prompt and site rule come with the
                // address bar (M2-T06).
                self.drm_failed.insert(tab);
            }
            PageMessage::UnsavedInput(u) => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.has_unsaved_input = u.dirty;
                }
            }
            // Until the permission flow decides (M2-T07), a page asking for
            // notifications hears "default" and nothing is shown; location
            // requests are refused; handlers are not registered.
            PageMessage::NotificationRequest(_) => {
                self.post(
                    tab,
                    json!({ "type": "notificationPermission", "permission": "default" }),
                );
            }
            PageMessage::GeolocationGet(g) | PageMessage::GeolocationWatch(g) => {
                self.post(
                    tab,
                    json!({ "type": "geolocationPosition", "id": g.id, "error": 1 }),
                );
            }
            PageMessage::NotificationShow(_)
            | PageMessage::NotificationClose(_)
            | PageMessage::GeolocationClear(_)
            | PageMessage::ProtocolHandlerRegister(_) => {}
            other => tracing::trace!(message = other.name(), "page message for a later task"),
        }
    }

    /// The page answered again: hide its prompt (spec 02 §5.3).
    fn responsive(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        if let Some(id) = self.unresponsive.remove(&tab) {
            let mut prompts = self.state.prompts.clone();
            prompts.remove(&id);
            self.state.tx(rec).set_prompts(prompts);
        }
    }

    /// A live, visible tab, not loading, silent for 10 s: "Page
    /// unresponsive" with Wait and Exit page.
    pub(super) fn heartbeat_check(&mut self, rec: &mut PatchRecorder) {
        let now = self.now();
        let silent: Vec<TabId> = self
            .model
            .windows
            .values()
            .filter_map(|w| w.active_tab_id)
            .filter(|t| {
                self.model
                    .tabs
                    .get(t)
                    .is_some_and(|t| t.life == TabLife::Live)
                    && self
                        .heartbeats
                        .get(t)
                        .is_some_and(|at| now - at > SILENT_FOR)
                    && !self.unresponsive.contains_key(t)
            })
            .collect();
        for tab in silent {
            let id = PromptId::new();
            let title = self.model.tabs[&tab].title.clone();
            self.unresponsive.insert(tab, id);
            let mut prompts = self.state.prompts.clone();
            prompts.insert(
                id,
                PromptState::Unresponsive(UnresponsivePrompt {
                    id,
                    tab_id: tab,
                    title,
                }),
            );
            self.state.tx(rec).set_prompts(prompts);
        }
    }

    /// A new document starts without a heartbeat and with no prompt.
    pub(super) fn new_document(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        self.page_downloads.remove(&tab);
        self.heartbeats.remove(&tab);
        self.responsive(tab, rec);
        self.context_targets.remove(&tab);
        self.drm_failed.remove(&tab);
    }

    pub(super) fn forget_tab(&mut self, tab: TabId) {
        self.page_downloads.remove(&tab);
        self.tab_gone(tab);
        self.heartbeats.remove(&tab);
        self.unresponsive.remove(&tab);
        self.context_targets.remove(&tab);
        self.drm_failed.remove(&tab);
    }

    /// Tab commands that act on the page (spec 01 §4.2).
    pub(super) fn page_command(
        &mut self,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let live = |core: &Self, tab: TabId| -> Result<(), MadoError> {
            let t = core.model.tab(tab)?;
            if t.life == TabLife::Sleeping {
                return Err(MadoError::new(
                    ErrorCode::InvalidInput,
                    "error.tab.sleeping",
                ));
            }
            Ok(())
        };
        match cmd {
            Command::TabReload {
                tab_id,
                bypass_cache,
            } => {
                let t = self.model.tab(tab_id)?;
                if matches!(t.life, TabLife::Crashed) {
                    // A crashed view is gone: make a new one at the same page.
                    self.recreate(tab_id);
                } else {
                    live(self, tab_id)?;
                    self.outbox
                        .push(EngineOp::Apply(tab_id, TabCommand::Reload { bypass_cache }));
                }
            }
            Command::TabStop { tab_id } => {
                live(self, tab_id)?;
                self.outbox.push(EngineOp::Apply(tab_id, TabCommand::Stop));
            }
            Command::TabBack { tab_id } | Command::TabForward { tab_id } => {
                live(self, tab_id)?;
                let back = matches!(cmd, Command::TabBack { .. });
                self.outbox.push(EngineOp::Apply(
                    tab_id,
                    if back {
                        TabCommand::GoBack
                    } else {
                        TabCommand::GoForward
                    },
                ));
            }
            Command::TabZoom { tab_id, zoom } => {
                if !(0.25..=5.0).contains(&zoom) {
                    return Err(MadoError::new(ErrorCode::InvalidInput, "error.tab.badZoom"));
                }
                let t = self
                    .model
                    .tabs
                    .get_mut(&tab_id)
                    .ok_or_else(|| not_found("error.tab.notFound"))?;
                t.zoom = zoom;
            }
            Command::TabFind {
                tab_id,
                query,
                forward,
                case_sensitive,
                close,
            } => {
                live(self, tab_id)?;
                let window = self.model.tab(tab_id)?.window_id;
                let w = self
                    .model
                    .windows
                    .get_mut(&window)
                    .ok_or_else(|| not_found("error.window.notFound"))?;
                if close {
                    w.find_bar = None;
                    self.outbox
                        .push(EngineOp::Apply(tab_id, TabCommand::FindClear));
                    return Ok(Value::Null);
                }
                let current = w.find_bar.as_ref().map(|f| f.query.clone());
                match query {
                    Some(q) if Some(&q) != current.as_ref() => {
                        let q: String = q.chars().take(1000).collect();
                        w.find_bar = Some(FindState {
                            query: q.clone(),
                            matches: None,
                            active: None,
                        });
                        self.outbox.push(EngineOp::Apply(
                            tab_id,
                            TabCommand::Find(mado_engine::tab::FindRequest {
                                query: q,
                                case_sensitive,
                                forward,
                            }),
                        ));
                    }
                    _ => {
                        if w.find_bar.is_none() {
                            w.find_bar = Some(FindState::default());
                        }
                        self.outbox
                            .push(EngineOp::Apply(tab_id, TabCommand::FindNext { forward }));
                    }
                }
            }
            Command::TabPrint { tab_id } => {
                live(self, tab_id)?;
                self.outbox.push(EngineOp::Apply(tab_id, TabCommand::Print));
            }
            Command::TabSave {
                tab_id,
                path,
                format,
            } => {
                live(self, tab_id)?;
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.file.notAbsolute",
                    ));
                }
                let cmd = match format {
                    SaveFormat::Pdf => TabCommand::SavePdf { path },
                    SaveFormat::WebArchive => TabCommand::SaveArchive { path },
                };
                self.outbox.push(EngineOp::Apply(tab_id, cmd));
            }
            Command::TabAnswerUnresponsive { prompt_id, exit } => {
                let tab = self
                    .unresponsive
                    .iter()
                    .find(|(_, p)| **p == prompt_id)
                    .map(|(t, _)| *t)
                    .ok_or_else(|| not_found("error.prompt.notFound"))?;
                self.responsive(tab, rec);
                if exit {
                    self.recreate(tab);
                } else {
                    // Wait: another 10 seconds before asking again.
                    let now = self.now();
                    self.heartbeats.insert(tab, now);
                }
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                return Err(MadoError::internal());
            }
        }
        Ok(Value::Null)
    }

    /// Destroys the tab's view and makes a new one at the same URL (Exit
    /// page, reload of a crashed tab).
    fn recreate(&mut self, tab: TabId) {
        let Some(t) = self.model.tabs.get_mut(&tab) else {
            return;
        };
        t.life = TabLife::Loading;
        t.progress = 0.0;
        if let Some(e) = self.model.extras.get_mut(&tab) {
            e.session = None;
        }
        self.forget_tab(tab);
        let spec = self.spec_for(&self.model.tabs[&tab].clone());
        self.outbox.push(EngineOp::DestroyTab(tab));
        self.outbox.push(EngineOp::CreateTab(spec));
    }
}
