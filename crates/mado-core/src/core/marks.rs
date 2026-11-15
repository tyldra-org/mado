//! Bookmarks in the core (spec 12 §3): queries on the read pool, changes on
//! the writer, Undo for deletions, and opening a folder's links as tabs.
//! The document carries only `bookmarksRev`, which goes up on each change.

use std::collections::BTreeMap;

use mado_types::command::{BookmarkSortKey, Command, OpenDisposition};
use mado_types::state::Notice;
use mado_types::state::NoticeAction;
use mado_types::{ErrorCode, MadoError, PatchRecorder, WindowId};
use serde_json::{Value, json};

use super::{Core, Internal, Reply, UNDO_NOTICE};
use crate::bookmarks::{self, Kind, Node};
use crate::timers::Timer;

fn failed(e: impl std::fmt::Display) -> MadoError {
    MadoError::new(ErrorCode::InvalidInput, "error.bookmark.failed").arg("reason", e.to_string())
}

type Job = Box<dyn FnOnce(&rusqlite::Connection) -> rusqlite::Result<Value> + Send>;

impl Core {
    /// Runs a bookmark change on the writer; the reply gets its result and
    /// every view hears `bookmarksRev` go up.
    fn change(&self, job: Job, reply: Option<Reply>) {
        self.on_writer(job, reply, true);
    }

    /// Runs `job` on the writer. Only a change bumps `bookmarksRev`: views
    /// reload the tree when it goes up, so a read that bumped it would loop.
    fn on_writer(&self, job: Job, reply: Option<Reply>, notify: bool) {
        let pending = self.profile_db.writer().write(job);
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let result = pending.await.map_err(failed);
            if notify && result.is_ok() {
                let _ = inputs.send(super::Input::Internal(Internal::BookmarksChanged));
            }
            if let Some(reply) = reply {
                let _ = reply.send(result);
            }
        });
    }

    fn read(
        &self,
        job: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<Value> + Send + 'static,
        reply: Option<Reply>,
    ) {
        let readers = self.profile_db.shared_readers();
        if let Some(reply) = reply {
            tokio::task::spawn_blocking(move || {
                let _ = reply.send(readers.read(job).map_err(failed));
            });
        }
    }

    pub(super) fn bookmarks_changed(&mut self, rec: &mut PatchRecorder) {
        let next = self.state.bookmarks_rev + 1;
        self.state.tx(rec).set_bookmarks_rev(next);
    }

    pub(super) fn bookmark_command(
        &mut self,
        cmd: Command,
        rec: &mut PatchRecorder,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let now = self.now();
        let reply = reply_to.take();
        match cmd {
            Command::BookmarkTree { parent_id } => {
                // Roots are created on first use, so the tree is a write
                // the first time; the writer serves it. Creating the empty
                // roots changes nothing a view shows, so the rev stays.
                self.on_writer(
                    Box::new(move |c| {
                        bookmarks::tree(c, parent_id.as_deref(), now).map(|t| json!(t))
                    }),
                    reply,
                    false,
                );
            }
            Command::BookmarkSearch { query } => self.read(
                move |c| bookmarks::search(c, &query, 100).map(|t| json!(t)),
                reply,
            ),
            Command::BookmarkRecent { limit } => self.read(
                move |c| bookmarks::recent(c, limit).map(|t| json!(t)),
                reply,
            ),
            Command::BookmarkOfUrl { url } => {
                self.read(move |c| bookmarks::of_url(c, &url).map(|t| json!(t)), reply)
            }
            Command::BookmarkAdd {
                parent_id,
                title,
                url,
                index,
            } => {
                if url::Url::parse(&url).is_err() {
                    if let Some(r) = reply {
                        let _ = r.send(Err(MadoError::new(
                            ErrorCode::InvalidInput,
                            "error.bookmark.badUrl",
                        )));
                    }
                    return Ok(Value::Null);
                }
                self.change(
                    Box::new(move |c| {
                        bookmarks::add(
                            c,
                            parent_id.as_deref(),
                            Kind::Link,
                            &title,
                            Some(&url),
                            index.map(|i| i as usize),
                            now,
                        )
                        .map(|id| json!({ "id": id }))
                    }),
                    reply,
                );
            }
            Command::BookmarkAddFolder {
                parent_id,
                title,
                index,
            } => self.change(
                Box::new(move |c| {
                    bookmarks::add(
                        c,
                        parent_id.as_deref(),
                        Kind::Folder,
                        &title,
                        None,
                        index.map(|i| i as usize),
                        now,
                    )
                    .map(|id| json!({ "id": id }))
                }),
                reply,
            ),
            Command::BookmarkUpdate { id, title, url } => self.change(
                Box::new(move |c| {
                    bookmarks::update(c, &id, title.as_deref(), url.as_deref(), now)
                        .map(|()| Value::Null)
                }),
                reply,
            ),
            Command::BookmarkMove {
                id,
                parent_id,
                index,
            } => self.change(
                Box::new(move |c| {
                    bookmarks::move_to(c, &id, &parent_id, index as usize, now)
                        .map(|()| Value::Null)
                }),
                reply,
            ),
            Command::BookmarkSort { folder_id, by } => self.change(
                Box::new(move |c| {
                    bookmarks::sort(c, &folder_id, by == BookmarkSortKey::Url).map(|()| Value::Null)
                }),
                reply,
            ),
            Command::BookmarkDelete { ids } => {
                let pending = self
                    .profile_db
                    .writer()
                    .write(move |c| bookmarks::delete(c, &ids));
                let inputs = self.inputs.clone();
                tokio::task::spawn_local(async move {
                    let result = pending.await.map_err(failed);
                    if let Ok(nodes) = &result {
                        let _ = inputs.send(super::Input::Internal(Internal::BookmarksDeleted(
                            nodes.clone(),
                        )));
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(result.map(|_| Value::Null));
                    }
                });
            }
            Command::BookmarkUndoDelete { token } => {
                let nodes = self.deleted_bookmarks.remove(&token).ok_or_else(|| {
                    MadoError::new(ErrorCode::NotFound, "error.bookmark.undoExpired")
                })?;
                self.change(
                    Box::new(move |c| bookmarks::restore(c, &nodes, now).map(|()| Value::Null)),
                    reply,
                );
            }
            Command::BookmarkOpenFolder {
                id,
                disposition,
                window_id,
            } => {
                let readers = self.profile_db.shared_readers();
                let inputs = self.inputs.clone();
                let window = window_id.or_else(|| self.current_window());
                tokio::task::spawn_blocking(move || {
                    let result = readers
                        .read(|c| bookmarks::links_in(c, &id))
                        .map_err(failed);
                    if let Ok(urls) = &result {
                        let _ = inputs.send(super::Input::Internal(Internal::OpenUrls {
                            urls: urls.clone(),
                            disposition,
                            window,
                        }));
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(result.map(|u| json!({ "opened": u.len() })));
                    }
                });
            }
            Command::BookmarkAddAllTabs {
                window_id,
                folder_name,
            } => {
                let w = self.model.window(window_id)?;
                let tabs: Vec<(String, String)> = w
                    .tab_order
                    .iter()
                    .filter_map(|t| self.model.tabs.get(t))
                    .filter(|t| url::Url::parse(&t.url).is_ok())
                    .map(|t| {
                        (
                            t.custom_title.clone().unwrap_or_else(|| t.title.clone()),
                            t.url.clone(),
                        )
                    })
                    .collect();
                self.change(
                    Box::new(move |c| {
                        let folder =
                            bookmarks::add(c, None, Kind::Folder, &folder_name, None, None, now)?;
                        for (title, url) in &tabs {
                            bookmarks::add(
                                c,
                                Some(&folder),
                                Kind::Link,
                                title,
                                Some(url),
                                None,
                                now,
                            )?;
                        }
                        Ok(json!({ "id": folder }))
                    }),
                    reply,
                );
            }
            Command::BookmarkExport { path } => {
                let path = std::path::PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.file.notAbsolute",
                    ));
                }
                // The writer, as the export may create the roots.
                self.change(
                    Box::new(move |c| {
                        let html = bookmarks::export_html(c, now)?;
                        std::fs::write(&path, html)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        Ok(Value::Null)
                    }),
                    reply,
                );
            }
            Command::BookmarkImport { path } => {
                let path = std::path::PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.file.notAbsolute",
                    ));
                }
                let pending_read = std::fs::metadata(&path).map_err(failed).and_then(|m| {
                    if m.len() > 64 << 20 {
                        Err(failed("the file is larger than 64 MB"))
                    } else {
                        Ok(())
                    }
                });
                if let Err(e) = pending_read {
                    if let Some(r) = reply {
                        let _ = r.send(Err(e));
                    }
                    return Ok(Value::Null);
                }
                self.change(
                    Box::new(move |c| {
                        let html = std::fs::read_to_string(&path)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        bookmarks::import(c, &bookmarks::parse_html(&html), now)
                            .map(|n| json!({ "imported": n }))
                    }),
                    reply,
                );
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                return Err(MadoError::internal());
            }
        }
        let _ = rec;
        Ok(Value::Null)
    }

    /// Deleted bookmarks wait 10 seconds for Undo.
    pub(super) fn bookmarks_deleted(&mut self, nodes: Vec<Node>, rec: &mut PatchRecorder) {
        if nodes.is_empty() {
            return;
        }
        let now = self.now();
        let token = self.next_notice;
        self.deleted_bookmarks.insert(token, nodes);
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let undo = Command::BookmarkUndoDelete { token };
        let notice = Notice {
            id: id.clone(),
            message_key: "notice.bookmarkDeleted".into(),
            args: BTreeMap::new(),
            action: Some(NoticeAction {
                label_key: "notice.undo".into(),
                command: undo.clone(),
            }),
            undo: Some(undo),
            expires_at: Some(now + i64::try_from(UNDO_NOTICE.as_millis()).unwrap_or(10_000)),
        };
        self.add_notice(notice, rec);
        self.timers.schedule(Timer::NoticeExpiry(id), UNDO_NOTICE);
        self.timers
            .schedule(Timer::ForgetDeletedBookmarks(token), UNDO_NOTICE);
        self.bookmarks_changed(rec);
    }

    /// Opens links as tabs (`bookmark.openFolder`).
    pub(super) fn open_urls(
        &mut self,
        urls: &[String],
        disposition: OpenDisposition,
        window: Option<WindowId>,
    ) -> Result<(), MadoError> {
        if urls.is_empty() {
            return Ok(());
        }
        let now = self.now();
        let prefs = self.prefs();
        let target = match (disposition, window) {
            (OpenDisposition::NewTabs, Some(w)) if self.model.windows.contains_key(&w) => w,
            _ => {
                return self
                    .open_window(mado_types::state::Bounds::default(), urls)
                    .map(|_| ());
            }
        };
        for (i, url) in urls.iter().enumerate() {
            let tab = self
                .model
                .open_tab(target, url.clone(), None, i == 0, None, now, prefs)?;
            self.pending_transition
                .insert(tab, crate::history::Transition::Bookmark);
        }
        Ok(())
    }
}
