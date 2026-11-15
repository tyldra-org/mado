//! History in the core (spec 12 §2): which navigations become visits and
//! with what transition, page text for search, and the history commands.
//! Writes go to the profile database's writer; queries run on the read
//! pool, off the core thread.

use std::collections::BTreeMap;

use mado_engine::NavKind;
use mado_types::command::Command;
use mado_types::{ErrorCode, MadoError, TabId};
use serde_json::{Value, json};

use super::{Core, Internal, Reply, lookup_def};
use crate::history::{self, Query, Transition};

/// What the core knows about a tab's current main-frame navigation.
#[derive(Debug, Clone, Copy, Default)]
pub struct NavInfo {
    pub kind: Option<NavKind>,
    pub gesture: bool,
    /// The visit of the page now shown, once recorded.
    pub visit: Option<i64>,
    pub committed_at: i64,
}

/// A client or server redirect: a commit within 5 s of the previous one
/// without user input (spec 12 §2.1).
const REDIRECT_WINDOW: i64 = 5_000;

impl Core {
    fn records_history(&self, tab: TabId) -> bool {
        !self.state.profile.is_private
            && self.model.tabs.get(&tab).is_some_and(|t| t.agent.is_none())
    }

    pub(super) fn nav_started(&mut self, tab: TabId, kind: NavKind, gesture: bool) {
        let info = self.navs.entry(tab).or_default();
        info.kind = Some(kind);
        info.gesture = gesture;
    }

    /// A main-frame document committed: end the previous visit and record
    /// this one.
    pub(super) fn nav_committed(&mut self, tab: TabId, url: &str) {
        let now = self.now();
        let previous = self.navs.get(&tab).copied().unwrap_or_default();
        if let Some(visit) = previous.visit {
            self.write_history(move |c| history::end_visit(c, visit, now));
        }
        let pending = self.pending_transition.remove(&tab);
        let search = self.pending_search.remove(&tab);
        let info = self.navs.entry(tab).or_default();
        info.visit = None;
        let quick = previous.committed_at > 0 && now - previous.committed_at < REDIRECT_WINDOW;
        info.committed_at = now;
        if !self.records_history(tab) || !history::recordable(url) {
            return;
        }
        let transition = pending.unwrap_or(match previous.kind {
            Some(NavKind::Typed) => Transition::Typed,
            Some(NavKind::Bookmark) => Transition::Bookmark,
            Some(NavKind::Reload) => Transition::Reload,
            Some(NavKind::Redirect) => Transition::Redirect,
            Some(NavKind::FormSubmit) => Transition::FormSubmit,
            Some(NavKind::Restore) => Transition::Restore,
            Some(NavKind::External) => Transition::External,
            _ if quick && !previous.gesture => Transition::Redirect,
            _ => Transition::Link,
        });
        let url = url.to_owned();
        let title = self
            .model
            .tabs
            .get(&tab)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        let from = previous.visit;
        let tab_text = tab.to_string();
        let pending = self.profile_db.writer().write(move |c| {
            let recorded = history::record(c, &url, &title, transition, &tab_text, from, now)?;
            // A search from the address bar keeps its terms (spec 12 §2.1).
            if let Some((term, engine)) = &search {
                c.execute(
                    "INSERT OR REPLACE INTO search_terms (url_id, term, engine_id)
                     SELECT id, ?2, ?3 FROM urls WHERE url = ?1",
                    rusqlite::params![url, term, engine],
                )?;
            }
            Ok(recorded)
        });
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            match pending.await {
                Ok(Some(r)) => {
                    let _ = inputs.send(super::Input::Internal(Internal::VisitRecorded {
                        tab,
                        visit: r.visit_id,
                    }));
                }
                Ok(None) => {}
                Err(e) => tracing::error!(error = %e, "a visit was not recorded"),
            }
        });
    }

    pub(super) fn visit_recorded(&mut self, tab: TabId, visit: i64) {
        if let Some(info) = self.navs.get_mut(&tab) {
            info.visit = Some(visit);
        }
    }

    pub(super) fn title_changed(&mut self, tab: TabId) {
        if !self.navs.get(&tab).is_some_and(|n| n.visit.is_some()) {
            return;
        }
        let Some(t) = self.model.tabs.get(&tab) else {
            return;
        };
        let (url, title) = (t.url.clone(), t.title.clone());
        self.write_history(move |c| history::set_title(c, &url, &title));
    }

    /// The tab went away: its visit ends.
    pub(super) fn tab_gone(&mut self, tab: TabId) {
        let now = self.now();
        if let Some(visit) = self.navs.remove(&tab).and_then(|n| n.visit) {
            self.write_history(move |c| history::end_visit(c, visit, now));
        }
        self.pending_transition.remove(&tab);
        self.pending_search.remove(&tab);
    }

    /// Page text from `status` (spec 12 §2.2).
    pub(super) fn page_text(&mut self, tab: TabId, text: String) {
        if !self.records_history(tab)
            || self.settings.value(lookup_def("history.indexPageText")) != true
        {
            return;
        }
        let Some(t) = self.model.tabs.get(&tab) else {
            return;
        };
        if history::is_sensitive(&history::host_of(&t.url)) || !history::recordable(&t.url) {
            return;
        }
        let (url, title) = (t.url.clone(), t.title.clone());
        let now = self.now();
        self.write_history(move |c| history::index_text(c, &url, &title, &text, now).map(|_| ()));
    }

    /// The daily retention job (spec 12 §2.4).
    pub(super) fn history_retention(&mut self) {
        let keep = self
            .settings
            .value(lookup_def("history.retention"))
            .as_i64()
            .unwrap_or(31_536_000_000);
        let limit = self
            .settings
            .value(lookup_def("history.indexLimitGb"))
            .as_i64()
            .unwrap_or(2)
            << 30;
        let now = self.now();
        self.write_history(move |c| history::retention(c, keep, limit, now));
    }

    fn write_history(
        &self,
        job: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<()> + Send + 'static,
    ) {
        if self.state.profile.is_private {
            return;
        }
        let pending = self.profile_db.writer().write(job);
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "a history write failed");
            }
        });
    }

    /// `history.*` (spec 12 §2.3). Queries answer from the read pool.
    pub(super) fn history_command(
        &mut self,
        cmd: Command,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let now = self.now();
        let readers = self.profile_db.shared_readers();
        let writer = |core: &Self, job: WriteJob, reply: Option<Reply>| {
            let pending = core.profile_db.writer().write(job);
            tokio::task::spawn_local(async move {
                let result = pending.await.map_err(|e| {
                    MadoError::new(ErrorCode::Internal, "error.history.failed")
                        .arg("reason", e.to_string())
                });
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            });
        };
        match cmd {
            Command::HistoryQuery {
                text,
                host,
                from,
                to,
                limit,
                cursor,
                utc_offset_minutes,
            } => {
                let cursor = cursor.and_then(|c| {
                    let (a, b) = c.split_once(':')?;
                    Some((a.parse().ok()?, b.parse().ok()?))
                });
                let q = Query {
                    text,
                    host,
                    from,
                    to,
                    limit,
                    cursor,
                };
                if let Some(reply) = reply_to.take() {
                    tokio::task::spawn_blocking(move || {
                        let result = readers
                            .read(|c| history::query(c, &q))
                            .map(|visits| grouped(&visits, utc_offset_minutes, q.limit))
                            .map_err(|e| {
                                MadoError::new(ErrorCode::Internal, "error.history.failed")
                                    .arg("reason", e.to_string())
                            });
                        let _ = reply.send(result);
                    });
                }
                Ok(Value::Null)
            }
            Command::HistoryDelete {
                visit_ids,
                url_ids,
                host,
                range,
            } => {
                let range = range.map(|r| (r.from, r.to));
                writer(
                    self,
                    Box::new(move |c| {
                        history::delete(c, &visit_ids, &url_ids, host.as_deref(), range)
                            .map(|n| json!({ "deleted": n }))
                    }),
                    reply_to.take(),
                );
                Ok(Value::Null)
            }
            Command::HistoryExclude { host } => {
                let host = host.trim().to_ascii_lowercase();
                if host.is_empty() || host.len() > 253 || host.contains(['/', ' ', ':']) {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.site.badHost",
                    ));
                }
                writer(
                    self,
                    Box::new(move |c| history::exclude(c, &host, now).map(|()| Value::Null)),
                    reply_to.take(),
                );
                Ok(Value::Null)
            }
            Command::HistoryExport { path } => {
                let path = std::path::PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.file.notAbsolute",
                    ));
                }
                if let Some(reply) = reply_to.take() {
                    tokio::task::spawn_blocking(move || {
                        let result = readers
                            .read(history::export_csv)
                            .map_err(|e| e.to_string())
                            .and_then(|csv| std::fs::write(&path, csv).map_err(|e| e.to_string()))
                            .map(|()| Value::Null)
                            .map_err(|reason| {
                                MadoError::new(ErrorCode::Internal, "error.history.exportFailed")
                                    .arg("reason", reason)
                            });
                        let _ = reply.send(result);
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
}

/// Visits grouped by day in the caller's time zone, newest first, with the
/// cursor for the next page.
fn grouped(visits: &[history::Visit], utc_offset_minutes: i32, limit: u32) -> Value {
    let offset = i64::from(utc_offset_minutes.clamp(-14 * 60, 14 * 60)) * 60_000;
    let mut days: Vec<(String, Vec<&history::Visit>)> = Vec::new();
    for v in visits {
        let day = history::iso(v.visited_at + offset)[..10].to_owned();
        match days.last_mut() {
            Some((d, list)) if *d == day => list.push(v),
            _ => days.push((day, vec![v])),
        }
    }
    let cursor = (visits.len() >= usize::try_from(limit.clamp(1, 500)).unwrap_or(100))
        .then(|| {
            visits
                .last()
                .map(|v| format!("{}:{}", v.visited_at, v.visit_id))
        })
        .flatten();
    json!({
        "days": days.into_iter().map(|(day, visits)| json!({ "day": day, "visits": visits })).collect::<Vec<_>>(),
        "cursor": cursor,
    })
}

pub type Navs = BTreeMap<TabId, NavInfo>;

/// A history write whose result answers a command.
type WriteJob = Box<dyn FnOnce(&rusqlite::Connection) -> rusqlite::Result<Value> + Send>;
