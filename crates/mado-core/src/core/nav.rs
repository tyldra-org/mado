//! Navigation in the core (spec 05): `nav.open`, suggestions with the
//! remote follow-up, search engines and Shift+Delete.

use std::time::Duration;

use mado_navigation::engines::{self, Engine};
use mado_navigation::suggest::{self, Sources, Visited};
use mado_navigation::{Context, Input, classify};
use mado_types::command::{Command, NavDisposition, NavSource};
use mado_types::state::Bounds;
use mado_types::{ErrorCode, MadoError, PatchRecorder, TabId, WindowId};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};

use super::{Core, Internal, Reply, lookup_def};
use crate::history::{self, Transition};

/// Remote suggestions arrive within this, or not at all (spec 05 §2).
const REMOTE_WAIT: Duration = Duration::from_millis(300);

/// A search engine with its place in the profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRow {
    #[serde(flatten)]
    pub engine: Engine,
    pub active: bool,
    pub source: String,
}

/// Every engine in the profile's `search_engines` table.
pub fn load_engines(c: &Connection) -> rusqlite::Result<Vec<EngineRow>> {
    let mut stmt = c.prepare(
        "SELECT id, name, keyword, search_url, suggest_url, active, source FROM search_engines ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(EngineRow {
            engine: Engine {
                id: r.get(0)?,
                name: r.get(1)?,
                keyword: r.get(2)?,
                search_url: r.get(3)?,
                suggest_url: r.get(4)?,
            },
            active: r.get::<_, i64>(5)? != 0,
            source: r.get(6)?,
        })
    })?;
    rows.collect()
}

/// The profile's engines: the stored ones, with built-ins not stored yet.
pub fn merged_engines(stored: Vec<EngineRow>) -> Vec<EngineRow> {
    let mut out = stored;
    for e in engines::builtin() {
        if !out.iter().any(|r| r.engine.id == e.id) {
            out.push(EngineRow {
                engine: e,
                active: true,
                source: "builtin".into(),
            });
        }
    }
    out
}

fn transition(source: NavSource) -> Transition {
    match source {
        NavSource::Typed | NavSource::Suggestion | NavSource::Paste => Transition::Typed,
        NavSource::Bookmark => Transition::Bookmark,
        NavSource::External => Transition::External,
        NavSource::Agent => Transition::Link,
    }
}

/// Words quoted for an FTS5 `MATCH`.
fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .take(8)
        .map(|w| format!("\"{}\"", w.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Pages whose text holds every word, for the page text provider.
fn page_text(c: &Connection, text: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = c.prepare_cached(
        "SELECT u.url, COALESCE(u.title, '') FROM page_text p JOIN urls u ON u.id = p.rowid
         WHERE page_text MATCH ?1 AND u.hidden = 0 ORDER BY bm25(page_text) LIMIT 2",
    )?;
    let rows = stmt.query_map([fts_query(text)], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

impl Core {
    fn active_keywords(&self) -> Vec<String> {
        self.engines
            .iter()
            .filter(|e| e.active)
            .filter_map(|e| e.engine.keyword.clone())
            .collect()
    }

    fn default_engine(&self) -> Option<&Engine> {
        let id = self.settings.value(lookup_def("search.defaultEngine"));
        let id = id.as_str().unwrap_or_default();
        self.engines
            .iter()
            .find(|e| e.active && e.engine.id == id)
            .map(|e| &e.engine)
    }

    fn classify_input(&self, input: &str, ctrl_enter: bool) -> Input {
        let keywords = self.active_keywords();
        let readers = self.profile_db.readers();
        let visited = |host: &str| {
            readers
                .read(|c| {
                    c.query_row(
                        "SELECT EXISTS (SELECT 1 FROM urls WHERE host = ?1)",
                        [host],
                        |r| r.get::<_, bool>(0),
                    )
                })
                .unwrap_or(false)
        };
        let home = home();
        let ctx = Context {
            keywords: &keywords,
            visited: &visited,
            exists: &|p| p.exists(),
            home: home.as_deref(),
            now: jiff::Zoned::now(),
        };
        classify(input, &ctx, ctrl_enter)
    }

    pub(super) fn nav_command(
        &mut self,
        cmd: Command,
        _rec: &mut PatchRecorder,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        match cmd {
            Command::NavOpen {
                window_id,
                tab_id,
                input,
                disposition,
                source,
                ctrl_enter,
            } => self.nav_open(window_id, tab_id, &input, disposition, source, ctrl_enter),
            Command::NavSuggest {
                window_id,
                input,
                deleted,
                request_id,
                ..
            } => {
                self.nav_suggest(window_id, input, deleted, request_id, reply_to);
                Ok(Value::Null)
            }
            Command::NavRemoveSuggestion { url } => {
                let job = move |c: &Connection| -> rusqlite::Result<()> {
                    let id: Option<i64> = c
                        .query_row("SELECT id FROM urls WHERE url = ?1", [&url], |r| r.get(0))
                        .ok();
                    if let Some(id) = id {
                        c.execute("UPDATE urls SET hidden = 1 WHERE id = ?1", [id])?;
                        history::delete(c, &[], &[id], None, None)?;
                    }
                    Ok(())
                };
                let pending = self.profile_db.writer().write(job);
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "a suggestion was not removed");
                    }
                });
                Ok(Value::Null)
            }
            Command::NavSearchEngines => {
                let default = self
                    .settings
                    .value(lookup_def("search.defaultEngine"))
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let list: Vec<Value> = self
                    .engines
                    .iter()
                    .map(|e| {
                        let mut v = json!(e);
                        v["isDefault"] = json!(e.engine.id == default);
                        v
                    })
                    .collect();
                Ok(json!(list))
            }
            Command::NavSetSearchEngineActive { id, active } => {
                let e = self
                    .engines
                    .iter_mut()
                    .find(|e| e.engine.id == id)
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.search.notFound"))?;
                e.active = active;
                let row = e.clone();
                let now = self.now();
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute(
                        "INSERT INTO search_engines (id, name, keyword, search_url, suggest_url, active, source, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT(id) DO UPDATE SET active = ?6",
                        params![
                            row.engine.id,
                            row.engine.name,
                            row.engine.keyword,
                            row.engine.search_url,
                            row.engine.suggest_url,
                            row.active,
                            row.source,
                            now
                        ],
                    )
                    .map(|_| ())
                });
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "a search engine was not saved");
                    }
                });
                Ok(Value::Null)
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                Err(MadoError::internal())
            }
        }
    }

    fn nav_open(
        &mut self,
        window: WindowId,
        tab: Option<TabId>,
        input: &str,
        disposition: NavDisposition,
        source: NavSource,
        ctrl_enter: bool,
    ) -> Result<Value, MadoError> {
        self.model.window(window)?;
        let mut search = None;
        let url = match self.classify_input(input, ctrl_enter) {
            Input::Url { url } | Input::FilePath { url, .. } => url,
            Input::Search { terms } => {
                let Some(engine) = self.default_engine() else {
                    return Ok(json!({ "needsEngine": true }));
                };
                search = Some((terms.clone(), engine.id.clone()));
                engines::expand(&engine.search_url, &terms)
            }
            Input::Keyword { keyword, query } => {
                if let Some(scope) = keyword.strip_prefix('@') {
                    return Ok(json!({ "scope": scope, "query": query }));
                }
                let engine = self
                    .engines
                    .iter()
                    .find(|e| {
                        e.active
                            && e.engine
                                .keyword
                                .as_deref()
                                .is_some_and(|k| k.eq_ignore_ascii_case(&keyword))
                    })
                    .map(|e| e.engine.clone())
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.search.notFound"))?;
                search = Some((query.clone(), engine.id.clone()));
                engines::expand(&engine.search_url, &query)
            }
            Input::Calculation { result, .. } => return Ok(json!({ "result": result })),
            Input::Refused { reason } => {
                return Err(MadoError::new(ErrorCode::InvalidInput, "error.nav.refused")
                    .arg("reason", serde_json::to_value(reason).unwrap_or_default()));
            }
        };
        let now = self.now();
        let prefs = self.prefs();
        let tab = match disposition {
            NavDisposition::CurrentTab => {
                let tab = tab
                    .or_else(|| self.model.window(window).ok().and_then(|w| w.active_tab_id))
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.tab.notFound"))?;
                self.navigate(tab, url)?;
                tab
            }
            NavDisposition::NewForegroundTab | NavDisposition::NewBackgroundTab => {
                let foreground = disposition == NavDisposition::NewForegroundTab;
                self.model
                    .open_tab(window, url, None, foreground, None, now, prefs)?
            }
            NavDisposition::NewWindow => {
                let id = self.open_window(Bounds::default(), &[url])?;
                *self
                    .model
                    .window(id)?
                    .tab_order
                    .first()
                    .ok_or_else(MadoError::internal)?
            }
        };
        self.pending_transition.insert(tab, transition(source));
        match search {
            Some(s) => {
                self.pending_search.insert(tab, s);
            }
            None => {
                self.pending_search.remove(&tab);
            }
        }
        Ok(json!({ "tabId": tab }))
    }

    /// Local rows now, on the read pool; remote rows later, when allowed.
    fn nav_suggest(
        &mut self,
        window: WindowId,
        input: String,
        deleted: bool,
        request_id: u32,
        reply_to: &mut Option<Reply>,
    ) {
        let private = self.state.profile.is_private;
        let class = self.classify_input(&input, false);
        let tabs: Vec<(String, String, String)> = self
            .state
            .tabs
            .values()
            .filter(|t| t.agent.is_none())
            .map(|t| (t.id.to_string(), t.title.clone(), t.url.clone()))
            .collect();
        let keywords: Vec<(String, String)> = self
            .engines
            .iter()
            .filter(|e| e.active)
            .filter_map(|e| Some((e.engine.keyword.clone()?, e.engine.name.clone())))
            .collect();
        let readers = self.profile_db.shared_readers();
        let reply = reply_to.take();
        let text = input.trim().to_owned();
        let remote = match (&class, self.default_engine()) {
            (Input::Search { terms }, Some(engine))
                if self.settings.value(lookup_def("search.suggestions")) == "on"
                    && (!private
                        || self
                            .settings
                            .value(lookup_def("search.suggestionsInPrivate"))
                            == true) =>
            {
                engine
                    .suggest_url
                    .as_deref()
                    .map(|t| engines::expand(t, terms))
            }
            _ => None,
        };
        let inputs = self.inputs.clone();
        let local = tokio::task::spawn_blocking(move || {
            let found = readers.read(|c| {
                let history: Vec<Visited> = if private {
                    Vec::new()
                } else {
                    history::suggest(c, &text, 8)?
                        .into_iter()
                        .map(|s| Visited {
                            url: s.url,
                            title: s.title,
                            frecency: s.frecency,
                            typed_count: s.typed_count,
                        })
                        .collect()
                };
                let bookmarks: Vec<(String, String)> = crate::bookmarks::search(c, &text, 8)?
                    .into_iter()
                    .filter_map(|n| Some((n.title, n.url?)))
                    .collect();
                let pages = if private || text.split_whitespace().count() < 2 {
                    Vec::new()
                } else {
                    page_text(c, &text).unwrap_or_default()
                };
                Ok((history, bookmarks, pages))
            });
            let (history, bookmarks, pages) = found.unwrap_or_default();
            let src = Sources {
                tabs: &tabs,
                history: &history,
                bookmarks: &bookmarks,
                keywords: &keywords,
                clipboard: None,
                page_text: &pages,
                private,
            };
            let rows = suggest::rank(&input, &class, &src);
            let inline = suggest::inline(input.trim(), &rows, &history, deleted);
            (rows, inline)
        });
        tokio::task::spawn_local(async move {
            let Ok((rows, inline)) = local.await else {
                if let Some(r) = reply {
                    let _ = r.send(Err(MadoError::internal()));
                }
                return;
            };
            if let Some(r) = reply {
                let _ = r.send(Ok(
                    json!({ "requestId": request_id, "rows": rows, "inline": inline }),
                ));
            }
            let Some(url) = remote else { return };
            let fetched = tokio::time::timeout(REMOTE_WAIT, async {
                // No cookies, no referrer: a bare GET (spec 05 §2).
                let body = reqwest::Client::new()
                    .get(&url)
                    .send()
                    .await?
                    .text()
                    .await?;
                Ok::<_, reqwest::Error>(body)
            })
            .await;
            let phrases = match fetched {
                Ok(Ok(body)) => engines::parse_suggestions(&body, suggest::MAX_ROWS),
                _ => return,
            };
            if phrases.is_empty() {
                return;
            }
            let merged = suggest::with_remote(rows, &phrases);
            let _ = inputs.send(super::Input::Internal(Internal::RemoteSuggestions {
                window,
                payload: json!({ "windowId": window, "requestId": request_id, "rows": merged }),
            }));
        });
    }
}
