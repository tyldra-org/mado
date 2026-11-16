//! Importing from other browsers (spec 12 §5): `import.detect` and
//! `import.run`. Reading happens off the core thread with `mado-import`;
//! the database work runs on the writer; tabs open here.

use std::collections::BTreeSet;

use mado_import::{Browser, ImportError, Imported, Kind};
use mado_types::command::{Command, ImportBrowser, ImportType, Origin};
use mado_types::state::{Bounds, Notice};
use mado_types::{ErrorCode, MadoError, PatchRecorder};
use rusqlite::params;
use serde_json::{Value, json};

use super::{Core, Internal, Reply};

/// History older than this stays behind unless `allHistory` (12 §5.2).
const HISTORY_DAYS: i64 = 90;

fn browser(b: ImportBrowser) -> Browser {
    match b {
        ImportBrowser::Chrome => Browser::Chrome,
        ImportBrowser::Edge => Browser::Edge,
        ImportBrowser::Brave => Browser::Brave,
        ImportBrowser::Arc => Browser::Arc,
        ImportBrowser::Firefox => Browser::Firefox,
        ImportBrowser::Safari => Browser::Safari,
    }
}

fn kind(t: ImportType) -> Kind {
    match t {
        ImportType::Bookmarks => Kind::Bookmarks,
        ImportType::History => Kind::History,
        ImportType::SearchEngines => Kind::SearchEngines,
        ImportType::Tabs => Kind::Tabs,
        ImportType::Extensions => Kind::Extensions,
    }
}

fn error(e: &ImportError) -> MadoError {
    match e {
        ImportError::FullDiskAccess => {
            MadoError::new(ErrorCode::PermissionDenied, "error.import.fullDiskAccess")
        }
        other => MadoError::new(ErrorCode::Internal, "error.import.failed")
            .arg("reason", other.to_string()),
    }
}

/// What one run added, for the reply and the notice.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    bookmarks: usize,
    visits: usize,
    engines: usize,
    tabs: usize,
    extensions: usize,
}

impl Core {
    pub(super) fn import_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let home = self.home.clone();
        match cmd {
            Command::ImportDetect => {
                if let Some(reply) = reply_to.take() {
                    tokio::task::spawn_blocking(move || {
                        let found: Vec<Value> = mado_import::detect(&home)
                            .into_iter()
                            .map(|s| json!({ "browser": s.browser, "profile": s.profile, "name": s.name }))
                            .collect();
                        let _ = reply.send(Ok(json!(found)));
                    });
                }
                Ok(Value::Null)
            }
            Command::ImportRun {
                browser: b,
                profile,
                types,
                all_history,
            } => {
                if !origin.is_user() {
                    return Err(MadoError::new(
                        ErrorCode::Protected,
                        "error.import.notAllowed",
                    ));
                }
                if self.state.profile.is_private {
                    return Err(MadoError::new(
                        ErrorCode::Unsupported,
                        "error.import.private",
                    ));
                }
                let since = (!all_history).then(|| self.now() - HISTORY_DAYS * 86_400_000);
                let kinds: Vec<Kind> = types.into_iter().map(kind).collect();
                let b = browser(b);
                if let Some(reply) = reply_to.take() {
                    let inputs = self.inputs.clone();
                    tokio::task::spawn_blocking(move || {
                        let source = mado_import::detect(&home)
                            .into_iter()
                            .find(|s| s.browser == b && s.profile == profile);
                        let result = match source {
                            Some(s) => mado_import::read(&s, &kinds, since, &home).map_err(|e| {
                                tracing::warn!(error = %e, "an import failed");
                                error(&e)
                            }),
                            None => {
                                Err(MadoError::new(ErrorCode::NotFound, "error.import.notFound"))
                            }
                        };
                        let _ = inputs.send(super::Input::Internal(Internal::BrowserImportRead {
                            browser: b,
                            result: Box::new(result),
                            reply,
                        }));
                    });
                }
                Ok(Value::Null)
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                Err(MadoError::internal())
            }
        }
    }

    /// Applies what was read: tabs and saved groups now, the rest on the
    /// writer; answers with the counts and shows them in a notice.
    pub(super) fn import_read(
        &mut self,
        b: Browser,
        result: Result<Imported, MadoError>,
        reply: Reply,
    ) {
        let data = match result {
            Ok(d) => d,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let tabs = self.import_tabs(&data);
        let now = self.now();
        let (bar, other, visits, extensions) = (
            data.bar.clone(),
            data.other.clone(),
            data.visits.clone(),
            data.extensions.clone(),
        );
        let engines = self.import_engines(&data);
        let counts = Counts {
            tabs,
            engines: engines
                .iter()
                .filter(|e| e.engine.id.starts_with("imported-"))
                .count(),
            ..Counts::default()
        };
        let browser_key = format!("{b:?}").to_ascii_lowercase();
        let write = self.profile_db.writer().write(move |c| {
            let (bar_root, other_root) = crate::bookmarks::roots(c, now)?;
            let bookmarks = crate::bookmarks::merge(c, &bar_root, &bar, now)?
                + crate::bookmarks::merge(c, &other_root, &other, now)?;
            let added = crate::history::import_visits(c, &visits, now)?;
            for e in &engines {
                c.execute(
                    "INSERT INTO search_engines (id, name, keyword, search_url, suggest_url, active, source, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(id) DO UPDATE SET active = ?6",
                    params![e.engine.id, e.engine.name, e.engine.keyword, e.engine.search_url, e.engine.suggest_url, e.active, e.source, now],
                )?;
            }
            let mut ext = 0;
            for x in &extensions {
                ext += c.execute(
                    "INSERT OR IGNORE INTO imported_extensions (id, name, browser, imported_at) VALUES (?1, ?2, ?3, ?4)",
                    params![x.id, x.name, browser_key, now],
                )?;
            }
            Ok((bookmarks, added, ext))
        });
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let result = write.await.map(|(bookmarks, visits, extensions)| Counts {
                bookmarks,
                visits,
                extensions,
                ..counts
            });
            let _ = inputs.send(super::Input::Internal(Internal::BrowserImportApplied {
                browser: b,
                result: result.map_err(|e| e.to_string()),
                reply,
            }));
        });
    }

    pub(super) fn import_applied(
        &mut self,
        b: Browser,
        result: Result<Counts, String>,
        reply: Reply,
        rec: &mut PatchRecorder,
    ) {
        let counts = match result {
            Ok(c) => c,
            Err(reason) => {
                tracing::error!(%reason, "an import was not saved");
                let _ = reply.send(Err(MadoError::new(
                    ErrorCode::Internal,
                    "error.import.failed",
                )
                .arg("reason", reason)));
                return;
            }
        };
        if counts.bookmarks > 0 {
            let _ = self
                .inputs
                .send(super::Input::Internal(Internal::BookmarksChanged));
        }
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: "notice.imported".into(),
            args: [
                ("browser".to_owned(), json!(b.name())),
                ("bookmarks".to_owned(), json!(counts.bookmarks)),
                ("visits".to_owned(), json!(counts.visits)),
                ("tabs".to_owned(), json!(counts.tabs)),
            ]
            .into(),
            action: None,
            undo: None,
            expires_at: None,
        };
        self.add_notice(notice, rec);
        if counts.extensions > 0 {
            self.offer_imported(b.name(), rec);
        }
        let _ = reply.send(Ok(json!({
            "bookmarks": counts.bookmarks,
            "visits": counts.visits,
            "engines": counts.engines,
            "tabs": counts.tabs,
            "extensions": counts.extensions,
        })));
    }

    /// New engines, inactive; the browser's default engine, new or one
    /// Mado has (same search URL or name), is turned on. Returns the rows
    /// to save.
    fn import_engines(&mut self, data: &Imported) -> Vec<super::nav::EngineRow> {
        let mut out = Vec::new();
        let default_url = data
            .default_engine
            .as_deref()
            .and_then(|name| data.engines.iter().find(|e| e.name == name))
            .map(|e| e.search_url.clone());
        let is_default = |r: &super::nav::EngineRow| {
            default_url.as_deref() == Some(r.engine.search_url.as_str())
                || data
                    .default_engine
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&r.engine.name))
        };
        for r in &mut self.engines {
            if is_default(r) && !r.active {
                r.active = true;
                out.push(r.clone());
            }
        }
        for e in &data.engines {
            if self
                .engines
                .iter()
                .any(|r| r.engine.search_url == e.search_url)
            {
                continue;
            }
            let row = super::nav::EngineRow {
                engine: mado_navigation::engines::Engine {
                    id: format!("imported-{}", uuid::Uuid::now_v7()),
                    name: e.name.clone(),
                    keyword: (!e.keyword.is_empty()).then(|| e.keyword.clone()),
                    search_url: e.search_url.clone(),
                    suggest_url: None,
                },
                active: data.default_engine.as_deref() == Some(e.name.as_str()),
                // The table knows builtin, user, opensearch and extension
                // engines; an imported one is the user's own.
                source: "user".into(),
            };
            self.engines.push(row.clone());
            out.push(row);
        }
        out
    }

    /// Open tabs in new windows with their pins and groups; Arc spaces as
    /// saved groups. Returns how many tabs opened.
    fn import_tabs(&mut self, data: &Imported) -> usize {
        let mut opened = 0;
        let now = self.now();
        for window in &data.windows {
            let urls: Vec<String> = window.iter().map(|t| t.url.clone()).collect();
            let Ok(id) = self.open_window(Bounds::default(), &urls) else {
                continue;
            };
            let ids = self
                .model
                .window(id)
                .map(|w| w.tab_order.clone())
                .unwrap_or_default();
            opened += ids.len();
            let pinned: Vec<_> = ids
                .iter()
                .zip(window)
                .filter(|(_, t)| t.pinned)
                .map(|(id, _)| *id)
                .collect();
            let _ = self.model.pin(&pinned, true);
            let keys: BTreeSet<&str> = window.iter().filter_map(|t| t.group.as_deref()).collect();
            for key in keys {
                let members: Vec<_> = self
                    .model
                    .window(id)
                    .map(|w| w.tab_order.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|tab| {
                        self.model.tabs.get(tab).is_some_and(|t| {
                            window
                                .iter()
                                .any(|w| w.group.as_deref() == Some(key) && w.url == t.url)
                        })
                    })
                    .collect();
                let _ = self.model.create_group(&members, None, None, now);
            }
        }
        for space in &data.spaces {
            let urls: Vec<String> = space.tabs.iter().map(|t| t.url.clone()).collect();
            let Ok(id) = self.open_window(Bounds::default(), &urls) else {
                continue;
            };
            let tabs = self
                .model
                .window(id)
                .map(|w| w.tab_order.clone())
                .unwrap_or_default();
            if let Ok(group) = self
                .model
                .create_group(&tabs, Some(space.title.clone()), None, now)
                && self.model.set_saved(group, true).is_ok()
            {
                let _ = self.model.close_group(group, now);
            }
            let _ = self.model.remove_window(id);
        }
        opened
    }
}
