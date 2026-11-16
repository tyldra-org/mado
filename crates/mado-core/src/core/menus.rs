//! The menu bar and the Dock menu (spec 01 §5.4): what each command ID
//! does in the key window, and the enabled and checked states the app
//! shows, recomputed after every change.

use std::collections::BTreeMap;

use mado_types::command::{Command, NavDisposition, NavSource, Origin};
use mado_types::state::{Bounds, TabLife};
use mado_types::{PatchRecorder, TabId, WindowId};
use serde_json::json;

use super::session::NEW_TAB;
use super::{Core, Internal, lookup_def};

/// Chrome's zoom levels.
const ZOOM_LEVELS: [f64; 17] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
];

/// The next zoom level up (`up`) or down from `zoom`.
pub fn zoom_step(zoom: f64, up: bool) -> f64 {
    if up {
        ZOOM_LEVELS
            .iter()
            .copied()
            .find(|z| *z > zoom + 1e-6)
            .unwrap_or(5.0)
    } else {
        ZOOM_LEVELS
            .iter()
            .rev()
            .copied()
            .find(|z| *z < zoom - 1e-6)
            .unwrap_or(0.25)
    }
}

/// Enabled and checked, per command ID.
pub type MenuState = BTreeMap<String, (bool, bool)>;

/// Commands the window's UI carries out itself (focus, find bar, dialogs).
const UI_ACTIONS: &[&str] = &[
    "openLocation",
    "focusAddressBar",
    "find",
    "findNext",
    "findPrevious",
    "bookmarkTab",
    "bookmarkAllTabs",
];

impl Core {
    fn menu_window(&self, window: Option<WindowId>) -> Option<WindowId> {
        window
            .filter(|w| self.model.windows.contains_key(w))
            .or_else(|| self.current_window())
    }

    fn menu_tab(&self, window: Option<WindowId>) -> Option<TabId> {
        self.model.windows.get(&window?)?.active_tab_id
    }

    /// The command a menu item runs in `window`, if it is one.
    fn menu_command(&self, id: &str, window: Option<WindowId>) -> Option<Command> {
        let tab = self.menu_tab(window);
        let open = |url: &str| -> Option<Command> {
            Some(Command::TabNew {
                window_id: window?,
                url: Some(url.to_owned()),
                position: None,
                background: false,
                opener_id: tab,
            })
        };
        let t = |f: fn(TabId) -> Command| tab.map(f);
        let select = |index: u32, last: bool| -> Option<Command> {
            Some(Command::TabSelectIndex {
                window_id: window?,
                index,
                last,
            })
        };
        let zoom = tab
            .and_then(|t| self.model.tabs.get(&t))
            .map_or(1.0, |t| t.zoom);
        Some(match id {
            "about" => return open("mado://about"),
            "newTab" => return open(&self.new_tab_url()),
            "newWindow" => Command::WindowNew {
                private: false,
                urls: Vec::new(),
            },
            "newPrivateWindow" => Command::WindowNew {
                private: true,
                urls: Vec::new(),
            },
            "reopenClosedTab" => Command::SessionReopenClosed,
            "closeWindow" => Command::WindowClose { window_id: window? },
            "closeTab" => t(|tab_id| Command::TabClose {
                tab_ids: vec![tab_id],
            })?,
            "print" => t(|tab_id| Command::TabPrint { tab_id })?,
            "stop" => t(|tab_id| Command::TabStop { tab_id })?,
            "reload" | "forceReload" => Command::TabReload {
                tab_id: tab?,
                bypass_cache: id == "forceReload",
            },
            "actualSize" => Command::TabZoom {
                tab_id: tab?,
                zoom: 1.0,
            },
            "zoomIn" | "zoomOut" => Command::TabZoom {
                tab_id: tab?,
                zoom: zoom_step(zoom, id == "zoomIn"),
            },
            "reader" => Command::TabReader {
                tab_id: tab?,
                on: !self.model.tabs.get(&tab?)?.reader_active,
            },
            "back" => t(|tab_id| Command::TabBack { tab_id })?,
            "forward" => t(|tab_id| Command::TabForward { tab_id })?,
            "home" => Command::NavOpen {
                window_id: window?,
                tab_id: tab,
                input: {
                    let home = self.setting_str("appearance.homePage");
                    if home.is_empty() {
                        NEW_TAB.to_owned()
                    } else {
                        home
                    }
                },
                disposition: NavDisposition::CurrentTab,
                source: NavSource::Bookmark,
                ctrl_enter: false,
            },
            "showHistory" => return open("mado://history"),
            "bookmarkManager" => return open("mado://bookmarks"),
            "downloads" => return open("mado://downloads"),
            "nextTab" => Command::TabSelectNext { window_id: window? },
            "previousTab" => Command::TabSelectPrevious { window_id: window? },
            "selectLastTab" => return select(0, true),
            s if s.starts_with("selectTab") => {
                let n: u32 = s.trim_start_matches("selectTab").parse().ok()?;
                return select(n - 1, false);
            }
            "duplicateTab" => t(|tab_id| Command::TabDuplicate { tab_id })?,
            "pinTab" => {
                let tab_id = tab?;
                if self.model.tabs.get(&tab_id)?.pinned {
                    Command::TabUnpin {
                        tab_ids: vec![tab_id],
                    }
                } else {
                    Command::TabPin {
                        tab_ids: vec![tab_id],
                    }
                }
            }
            "muteSite" => {
                let t = self.model.tabs.get(&tab?)?;
                Command::TabMuteSite {
                    host: url::Url::parse(&t.url).ok()?.host_str()?.to_owned(),
                    muted: !t.audio.muted,
                }
            }
            "moveTabToNewWindow" => Command::TabToNewWindow {
                tab_ids: vec![tab?],
                bounds: None::<Bounds>,
            },
            "toggleBookmarksBar" => Command::SettingsSet {
                key: "appearance.bookmarksBar".into(),
                value: json!(self.settings.value(lookup_def("appearance.bookmarksBar")) != true),
                token: None,
            },
            _ => return None,
        })
    }

    /// Runs a menu, Dock menu or key equivalent's command in `window` (the
    /// key window; the current one when none is given).
    pub(super) fn menu_action(
        &mut self,
        id: &str,
        window: Option<WindowId>,
        rec: &mut PatchRecorder,
    ) {
        let window = self.menu_window(window);
        match id {
            "settings" | "importBrowserData" => self.ui.open_settings(),
            "quit" => self.internal(Internal::QuitRequested { shortcut: true }, rec),
            "viewSource" => self.view_source(window),
            "developerTools" => {
                if let Some(tab) = self.menu_tab(window) {
                    self.outbox.push(mado_engine::EngineOp::Apply(
                        tab,
                        mado_engine::TabCommand::ShowDevTools,
                    ));
                }
            }
            ui if UI_ACTIONS.contains(&ui) => {
                if let Some(w) = window {
                    self.ui.window_action(w, ui);
                }
            }
            other => {
                let Some(cmd) = self.menu_command(other, window) else {
                    tracing::debug!(id = other, "a menu item has nothing to act on");
                    return;
                };
                let mut reply = None;
                if let Err(e) = self.command(&Origin::System, cmd, rec, &mut reply) {
                    tracing::debug!(id = other, error = %e.message_key, "a menu command failed");
                }
            }
        }
    }

    /// View Source: the page's current document, escaped, in a new tab
    /// next to it (`mado://source?tab=<id>`).
    fn view_source(&mut self, window: Option<WindowId>) {
        let Some(tab) = self.menu_tab(window) else {
            return;
        };
        let inputs = self.inputs.clone();
        self.outbox.push(mado_engine::EngineOp::Apply(
            tab,
            mado_engine::TabCommand::EvaluateInWorld {
                world: mado_engine::World::Mado,
                source: "return { url: location.href, html: '<!doctype html>\\n' + document.documentElement.outerHTML };".into(),
                reply: Box::new(move |r| {
                    let Some((url, html)) = r.ok().and_then(|v| {
                        Some((v["url"].as_str()?.to_owned(), v["html"].as_str()?.to_owned()))
                    }) else {
                        return;
                    };
                    let _ = inputs.send(super::Input::Internal(Internal::SourceRead { tab, url, html }));
                }),
            },
        ));
    }

    pub(super) fn source_read(&mut self, tab: TabId, url: String, html: String) {
        let Some(window) = self.model.tabs.get(&tab).map(|t| t.window_id) else {
            return;
        };
        let (now, prefs) = (self.now(), self.prefs());
        let Ok(new) = self.model.open_tab(
            window,
            "about:blank".into(),
            None,
            true,
            Some(tab),
            now,
            prefs,
        ) else {
            return;
        };
        let esc = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        };
        let page = format!(
            r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'"><meta name="color-scheme" content="light dark"><title>{title}</title><style>body{{margin:0}}pre{{margin:0;padding:1em;font:12px/1.5 ui-monospace,Menlo,monospace;white-space:pre-wrap;word-break:break-all}}</style></head><body><pre>{source}</pre></body></html>"#,
            title = esc(&format!("view-source:{url}")),
            source = esc(&html),
        );
        self.outbox.push(mado_engine::EngineOp::Apply(
            new,
            mado_engine::TabCommand::ShowPage {
                page: mado_engine::tab::GeneratedPage::Source,
                html: page,
            },
        ));
    }

    /// Enabled and checked for every menu command, in the key window.
    pub(super) fn menu_state(&self, window: Option<WindowId>) -> MenuState {
        let window = self.menu_window(window);
        let tab = self.menu_tab(window).and_then(|t| self.model.tabs.get(&t));
        let live = tab.is_some_and(|t| t.life != TabLife::Crashed);
        let mut out = MenuState::new();
        for id in mado_types::menu::command_ids() {
            let enabled = match id {
                "back" => tab.is_some_and(|t| t.can_go_back),
                "forward" => tab.is_some_and(|t| t.can_go_forward),
                "reader" => tab.is_some_and(|t| t.reader_available || t.reader_active),
                "viewSource" => tab.is_some_and(|t| t.url.starts_with("http")),
                "stop" => tab.is_some_and(|t| t.life == TabLife::Loading),
                "muteSite" => tab.is_some_and(|t| t.url.starts_with("http")),
                "about" | "settings" | "quit" | "newWindow" | "newPrivateWindow"
                | "reopenClosedTab" | "importBrowserData" | "toggleBookmarksBar" => true,
                _ => window.is_some() && (tab.is_none() || live),
            };
            let checked = match id {
                "toggleBookmarksBar" => {
                    self.settings.value(lookup_def("appearance.bookmarksBar")) == true
                }
                "pinTab" => tab.is_some_and(|t| t.pinned),
                "muteSite" => tab.is_some_and(|t| t.audio.muted),
                _ => false,
            };
            out.insert(id.to_owned(), (enabled, checked));
        }
        out
    }

    /// Tells the app when an enabled or checked state changed.
    pub(super) fn refresh_menu_state(&mut self) {
        let now = self.menu_state(None);
        if now != self.menu_shown {
            self.ui.menu_state(&now);
            self.menu_shown = now;
        }
    }

    /// The shortcut of each menu command, `""` for none.
    pub(super) fn shortcuts(&self) -> BTreeMap<String, String> {
        mado_types::menu::command_ids()
            .into_iter()
            .map(|id| {
                let keys = mado_settings::lookup(&format!("shortcuts.{id}"))
                    .map(|d| self.settings.value(d))
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                (id.to_owned(), keys)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_follow_chrome() {
        assert!((zoom_step(1.0, true) - 1.1).abs() < 1e-9);
        assert!((zoom_step(1.0, false) - 0.9).abs() < 1e-9);
        assert!((zoom_step(5.0, true) - 5.0).abs() < 1e-9);
        assert!((zoom_step(1.05, false) - 1.0).abs() < 1e-9);
    }
}
