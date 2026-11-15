//! Downloads in the core (spec 12 §4, 08 §4): where each file goes, its
//! state machine, the safety verdict when it completes, pause and resume
//! (also across a quit), and the download commands.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mado_engine::event::{
    DownloadCommand, DownloadDestination, DownloadError, EngineDownload, EngineDownloadId,
};
use mado_engine::{EngineOp, Reply as EngineReply};
use mado_types::command::Command;
use mado_types::state::{DownloadState, DownloadStatus, Notice};
use mado_types::{DownloadId, ErrorCode, MadoError, PatchRecorder, TabId};
use rusqlite::params;
use serde_json::Value;

use super::{Core, Internal, lookup_def};
use crate::downloads as rules;

/// What the core keeps about a download beyond the document.
#[derive(Debug, Clone, Default)]
pub struct Dl {
    pub engine: Option<EngineDownloadId>,
    /// Where the bytes are written (`.madodownload`).
    pub temp: PathBuf,
    pub url: String,
    pub tab_url: Option<String>,
    pub mime: Option<String>,
    pub resume: Option<Vec<u8>>,
    pub pausing: bool,
    pub insecure: bool,
    /// For the speed: the last progress time and bytes.
    pub last: (i64, u64),
}

/// Engine IDs Mado picks for resumed downloads, above the adapter's.
const RESUME_IDS: u64 = 1 << 62;

fn status_word(s: DownloadStatus) -> &'static str {
    match s {
        DownloadStatus::InProgress => "in_progress",
        DownloadStatus::Paused => "paused",
        DownloadStatus::Complete => "complete",
        DownloadStatus::Cancelled => "cancelled",
        DownloadStatus::Failed => "failed",
        DownloadStatus::Dangerous => "dangerous",
        DownloadStatus::Blocked => "blocked",
    }
}

fn parse_status(s: &str) -> DownloadStatus {
    match s {
        "paused" => DownloadStatus::Paused,
        "complete" => DownloadStatus::Complete,
        "cancelled" => DownloadStatus::Cancelled,
        "dangerous" => DownloadStatus::Dangerous,
        "blocked" => DownloadStatus::Blocked,
        _ => DownloadStatus::Failed,
    }
}

fn not_found() -> MadoError {
    MadoError::new(ErrorCode::NotFound, "error.download.notFound")
}

fn default_folder() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Downloads")
}

/// Loads the download list (spec 03): a download that was running when
/// Mado stopped is failed, resumable when it left resume data.
pub fn load(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<(DownloadState, Dl)>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, tab_url, path, mime, total_bytes, received_bytes, state, danger, error, resume_data, started_at
         FROM downloads WHERE removed = 0 ORDER BY started_at",
    )?;
    let rows = stmt.query_map([], |r| {
        let id: String = r.get(0)?;
        let path: Option<String> = r.get(3)?;
        let state: String = r.get(7)?;
        let resume: Option<Vec<u8>> = r.get(10)?;
        let status = if state == "in_progress" {
            DownloadStatus::Failed
        } else {
            parse_status(&state)
        };
        let path = PathBuf::from(path.unwrap_or_default());
        Ok((
            DownloadState {
                id: id.parse().unwrap_or_default(),
                filename: rules::without_suffix(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                url: r.get(1)?,
                state: status,
                received: u64::try_from(r.get::<_, i64>(6)?).unwrap_or(0),
                total: r
                    .get::<_, Option<i64>>(5)?
                    .and_then(|t| u64::try_from(t).ok()),
                bytes_per_second: 0,
                danger: r.get(8)?,
                error: if state == "in_progress" && resume.is_none() {
                    Some("interrupted".into())
                } else {
                    r.get(9)?
                },
                started_at: r.get(11)?,
            },
            Dl {
                temp: path,
                url: r.get(1)?,
                tab_url: r.get(2)?,
                mime: r.get(4)?,
                resume,
                ..Dl::default()
            },
        ))
    })?;
    rows.collect()
}

impl Core {
    fn save_download(&self, id: DownloadId) {
        let (Some(s), Some(d)) = (
            self.state.downloads.get(&id).cloned(),
            self.dls.get(&id).cloned(),
        ) else {
            return;
        };
        if self.state.profile.is_private {
            return;
        }
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT INTO downloads (id, url, tab_url, path, mime, total_bytes, received_bytes, state, danger, error, resume_data, started_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET path = ?4, total_bytes = ?6, received_bytes = ?7, state = ?8, danger = ?9,
                     error = ?10, resume_data = ?11, ended_at = CASE WHEN ?8 IN ('complete','cancelled','failed') THEN ?13 ELSE NULL END",
                params![
                    s.id.to_string(),
                    d.url,
                    d.tab_url,
                    d.temp.to_string_lossy(),
                    d.mime,
                    s.total.and_then(|t| i64::try_from(t).ok()),
                    i64::try_from(s.received).unwrap_or(i64::MAX),
                    status_word(s.state),
                    s.danger,
                    s.error,
                    d.resume,
                    s.started_at,
                    s.started_at,
                ],
            )
            .map(|_| ())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "a download row was not written");
            }
        });
    }

    fn set_download(
        &mut self,
        id: DownloadId,
        rec: &mut PatchRecorder,
        f: impl FnOnce(&mut DownloadState),
    ) {
        let Some(mut s) = self.state.downloads.get(&id).cloned() else {
            return;
        };
        f(&mut s);
        self.state.tx(rec).downloads().insert(id, s);
        self.save_download(id);
    }

    /// Spec 12 §4.2 steps 1 to 4: name, folder, free space, then the
    /// engine writes to the temporary name.
    pub(super) fn download_started(
        &mut self,
        tab: Option<TabId>,
        d: EngineDownload,
        reply: EngineReply<DownloadDestination>,
        rec: &mut PatchRecorder,
    ) {
        let now = self.now();
        // A second download from the same page without a navigation asks
        // for `automaticDownloads` (07); until the permission flow
        // (M2-T07) it is refused, with a notice.
        if let Some(tab) = tab {
            let n = self.page_downloads.entry(tab).or_default();
            *n += 1;
            if *n > 1 {
                reply.send(DownloadDestination::Cancel);
                let notice = Notice {
                    id: format!("n{}", self.next_notice),
                    message_key: "notice.multipleDownloadsBlocked".into(),
                    args: BTreeMap::new(),
                    action: None,
                    undo: None,
                    expires_at: Some(now + 8000),
                };
                self.next_notice += 1;
                self.add_notice(notice, rec);
                return;
            }
        }
        let folder = self
            .settings
            .value(lookup_def("downloads.folder"))
            .as_str()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_folder);
        let name = rules::sanitize(if d.suggested_filename.is_empty() {
            &d.url
        } else {
            &d.suggested_filename
        });
        let name = if d.suggested_filename.is_empty() {
            rules::sanitize(&rules::name_from_url(&d.url))
        } else {
            name
        };
        let busy: Vec<PathBuf> = self.dls.values().map(|x| x.temp.clone()).collect();
        let target = rules::unique(&folder, &name, &|p: &Path| {
            p.exists() || busy.iter().any(|b| b == p)
        });
        let temp = rules::with_suffix(&target);
        let id = DownloadId::new();
        let tab_url = tab
            .and_then(|t| self.model.tabs.get(&t))
            .map(|t| t.url.clone());
        let mut state = DownloadState {
            id,
            filename: target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            url: d.url.clone(),
            state: DownloadStatus::InProgress,
            received: 0,
            total: d.total,
            bytes_per_second: 0,
            danger: None,
            error: None,
            started_at: now,
        };
        let free = mado_platform::files::free_space(&folder).ok();
        let fits = match (d.total, free) {
            (Some(total), Some(free)) => total.saturating_add(rules::SPARE_BYTES) <= free,
            _ => true,
        };
        let insecure = rules::insecure(&d.url, tab_url.as_deref());
        self.dls.insert(
            id,
            Dl {
                engine: Some(d.id),
                temp: temp.clone(),
                url: d.url.clone(),
                tab_url,
                mime: d.mime.clone(),
                insecure,
                last: (now, 0),
                ..Dl::default()
            },
        );
        self.dl_by_engine.insert(d.id, id);
        if !fits {
            state.state = DownloadStatus::Failed;
            state.error = Some("insufficientSpace".into());
            reply.send(DownloadDestination::Cancel);
        } else if self.settings.value(lookup_def("downloads.askWhere")) == true {
            reply.send(DownloadDestination::Ask(temp));
        } else {
            reply.send(DownloadDestination::Path(temp));
        }
        self.state.tx(rec).downloads().insert(id, state);
        self.save_download(id);
    }

    pub(super) fn download_progress(
        &mut self,
        engine: EngineDownloadId,
        received: u64,
        total: Option<u64>,
        rec: &mut PatchRecorder,
    ) {
        let Some(&id) = self.dl_by_engine.get(&engine) else {
            return;
        };
        let now = self.now();
        let speed = self.dls.get_mut(&id).map_or(0, |d| {
            let (at, bytes) = d.last;
            let dt = u64::try_from(now - at).unwrap_or(0);
            if dt < 500 {
                return u64::MAX;
            }
            d.last = (now, received);
            received.saturating_sub(bytes).saturating_mul(1000) / dt.max(1)
        });
        let Some(mut s) = self.state.downloads.get(&id).cloned() else {
            return;
        };
        s.received = received;
        s.total = total.or(s.total);
        if speed != u64::MAX {
            s.bytes_per_second = speed;
        }
        self.state.tx(rec).downloads().insert(id, s);
    }

    /// The engine finished: complete, paused, cancelled or failed.
    pub(super) fn download_finished(
        &mut self,
        engine: EngineDownloadId,
        result: Result<PathBuf, DownloadError>,
        rec: &mut PatchRecorder,
    ) {
        let Some(id) = self.dl_by_engine.remove(&engine) else {
            return;
        };
        match result {
            Ok(written) => {
                if let Some(d) = self.dls.get_mut(&id) {
                    d.temp = written;
                    d.engine = None;
                }
                self.verdict(id);
            }
            Err(DownloadError::Interrupted { resume_data }) => {
                let pausing = self.dls.get(&id).is_some_and(|d| d.pausing);
                if let Some(d) = self.dls.get_mut(&id) {
                    d.resume = Some(resume_data);
                    d.pausing = false;
                    d.engine = None;
                }
                self.set_download(id, rec, |s| {
                    s.bytes_per_second = 0;
                    if pausing {
                        s.state = DownloadStatus::Paused;
                    } else {
                        s.state = DownloadStatus::Failed;
                        s.error = Some("interrupted".into());
                    }
                });
                self.quit_when_downloads_paused();
            }
            Err(DownloadError::Cancelled) => {
                if let Some(d) = self.dls.get_mut(&id) {
                    d.engine = None;
                    let _ = std::fs::remove_file(&d.temp);
                }
                self.set_download(id, rec, |s| {
                    if s.state == DownloadStatus::InProgress {
                        s.state = DownloadStatus::Cancelled;
                    }
                    s.bytes_per_second = 0;
                });
                self.quit_when_downloads_paused();
            }
            Err(e) => {
                if let Some(d) = self.dls.get_mut(&id) {
                    d.engine = None;
                }
                let reason = match e {
                    DownloadError::Network(n) => format!("network:{n:?}"),
                    DownloadError::File(f) => format!("file:{f}"),
                    _ => "failed".into(),
                };
                self.set_download(id, rec, |s| {
                    s.state = DownloadStatus::Failed;
                    s.error = Some(reason);
                    s.bytes_per_second = 0;
                });
                self.quit_when_downloads_paused();
            }
        }
    }

    /// Spec 08 §4 at completion, off the core thread: a dangerous type from
    /// a site with no visit before today, or an insecure download, waits
    /// for Keep or Discard; anything else is renamed and marked.
    fn verdict(&mut self, id: DownloadId) {
        let Some(d) = self.dls.get(&id).cloned() else {
            return;
        };
        let Some(name) = self.state.downloads.get(&id).map(|s| s.filename.clone()) else {
            return;
        };
        let readers = self.profile_db.shared_readers();
        let inputs = self.inputs.clone();
        let now = self.now();
        let host = d
            .tab_url
            .as_deref()
            .map(crate::history::host_of)
            .unwrap_or_else(|| crate::history::host_of(&d.url));
        tokio::task::spawn_blocking(move || {
            let first_visit = rules::dangerous_type(&name)
                && readers
                    .read(|c| {
                        c.query_row(
                            "SELECT COUNT(*) FROM visits v JOIN urls u ON u.id = v.url_id
                             WHERE (u.host = ?1 OR u.host LIKE '%.' || ?1) AND v.visited_at < ?2",
                            params![host, now - 86_400_000],
                            |r| r.get::<_, i64>(0),
                        )
                    })
                    .map_or(true, |n| n == 0);
            let danger = if d.insecure {
                Some("insecure".to_owned())
            } else if first_visit {
                Some("dangerousType".to_owned())
            } else {
                None
            };
            let _ = inputs.send(super::Input::Internal(Internal::DownloadVerdict {
                id,
                danger,
            }));
        });
    }

    pub(super) fn download_verdict(
        &mut self,
        id: DownloadId,
        danger: Option<String>,
        rec: &mut PatchRecorder,
    ) {
        match danger {
            Some(why) => {
                let blocked = why == "insecure";
                self.set_download(id, rec, |s| {
                    s.state = if blocked {
                        DownloadStatus::Blocked
                    } else {
                        DownloadStatus::Dangerous
                    };
                    s.danger = Some(why);
                    s.bytes_per_second = 0;
                });
            }
            None => self.finish_file(id, rec),
        }
    }

    /// Renames the temporary file, marks it and completes (spec 12 §4.2
    /// step 5); opens safe types when the setting says so.
    fn finish_file(&mut self, id: DownloadId, rec: &mut PatchRecorder) {
        let Some(d) = self.dls.get(&id).cloned() else {
            return;
        };
        let dir = d.temp.parent().map(Path::to_path_buf).unwrap_or_default();
        let wanted = rules::without_suffix(&d.temp);
        let name = wanted
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let target = if wanted.exists() {
            rules::unique(&dir, &name, &|p: &Path| p.exists())
        } else {
            wanted
        };
        let result = std::fs::rename(&d.temp, &target);
        let secs = u64::try_from(self.now() / 1000).unwrap_or(0);
        if result.is_ok()
            && let Err(e) = mado_platform::files::quarantine(&target, secs)
        {
            tracing::warn!(error = %e, "the quarantine mark was not set");
        }
        let open = result.is_ok()
            && self.settings.value(lookup_def("downloads.openSafeTypes")) == "safe"
            && rules::safe_to_open(&name)
            && !rules::dangerous_type(&name);
        if let Some(x) = self.dls.get_mut(&id) {
            x.temp.clone_from(&target);
        }
        let file = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.set_download(id, rec, |s| match &result {
            Ok(()) => {
                s.state = DownloadStatus::Complete;
                s.filename = file;
                s.danger = None;
                s.bytes_per_second = 0;
                if let Some(t) = s.total {
                    s.received = s.received.max(t);
                }
            }
            Err(e) => {
                s.state = DownloadStatus::Failed;
                s.error = Some(format!("file:{e}"));
            }
        });
        if open {
            tokio::task::spawn_blocking(move || mado_platform::files::open(&target));
        }
    }

    /// Quitting waits for running downloads to pause, so they resume after
    /// the next launch (spec 12 §4.2 step 7).
    pub(super) fn pause_downloads_for_quit(&mut self) -> bool {
        let running: Vec<(DownloadId, EngineDownloadId)> = self
            .dls
            .iter()
            .filter_map(|(id, d)| Some((*id, d.engine?)))
            .filter(|(id, _)| {
                self.state
                    .downloads
                    .get(id)
                    .is_some_and(|s| s.state == DownloadStatus::InProgress)
            })
            .collect();
        for (id, engine) in &running {
            if let Some(d) = self.dls.get_mut(id) {
                d.pausing = false;
            }
            self.outbox
                .push(EngineOp::Download(*engine, DownloadCommand::Pause));
        }
        self.quit_after_downloads = !running.is_empty();
        self.quit_after_downloads
    }

    fn quit_when_downloads_paused(&mut self) {
        if self.quit_after_downloads && self.dls.values().all(|d| d.engine.is_none()) {
            self.quit_after_downloads = false;
            self.ui.quit();
        }
    }

    pub(super) fn download_command(
        &mut self,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let id = match &cmd {
            Command::DownloadPause { download_id }
            | Command::DownloadResume { download_id }
            | Command::DownloadCancel { download_id }
            | Command::DownloadKeep { download_id }
            | Command::DownloadDiscard { download_id }
            | Command::DownloadOpen { download_id }
            | Command::DownloadReveal { download_id }
            | Command::DownloadCopyFile { download_id }
            | Command::DownloadCopyPath { download_id }
            | Command::DownloadRemove { download_id }
            | Command::DownloadOpenSource { download_id } => Some(*download_id),
            _ => None,
        };
        let state = id.and_then(|i| self.state.downloads.get(&i).map(|s| s.state));
        let dl = id.and_then(|i| self.dls.get(&i).cloned());
        let bad = |key: &str| MadoError::new(ErrorCode::InvalidInput, key);
        match cmd {
            Command::DownloadPause { .. } => {
                let (Some(id), Some(engine)) = (id, dl.as_ref().and_then(|d| d.engine)) else {
                    return Err(bad("error.download.notRunning"));
                };
                if let Some(d) = self.dls.get_mut(&id) {
                    d.pausing = true;
                }
                self.outbox
                    .push(EngineOp::Download(engine, DownloadCommand::Pause));
            }
            Command::DownloadResume { .. } => {
                let id = id.ok_or_else(not_found)?;
                let (data, path) = dl
                    .and_then(|d| Some((d.resume?, d.temp)))
                    .ok_or_else(|| bad("error.download.notResumable"))?;
                self.next_resume += 1;
                let engine = EngineDownloadId(RESUME_IDS + self.next_resume);
                self.dl_by_engine.insert(engine, id);
                if let Some(d) = self.dls.get_mut(&id) {
                    d.engine = Some(engine);
                    d.resume = None;
                }
                self.set_download(id, rec, |s| {
                    s.state = DownloadStatus::InProgress;
                    s.error = None;
                });
                self.outbox.push(EngineOp::Download(
                    engine,
                    DownloadCommand::Resume {
                        resume_data: data,
                        path,
                    },
                ));
            }
            Command::DownloadCancel { .. } => {
                let id = id.ok_or_else(not_found)?;
                match dl.as_ref().and_then(|d| d.engine) {
                    Some(engine) => self
                        .outbox
                        .push(EngineOp::Download(engine, DownloadCommand::Cancel)),
                    None => {
                        if let Some(d) = &dl {
                            let _ = std::fs::remove_file(&d.temp);
                        }
                        self.set_download(id, rec, |s| s.state = DownloadStatus::Cancelled);
                    }
                }
            }
            Command::DownloadKeep { .. } => {
                let id = id.ok_or_else(not_found)?;
                if !matches!(
                    state,
                    Some(DownloadStatus::Dangerous | DownloadStatus::Blocked)
                ) {
                    return Err(bad("error.download.notHeld"));
                }
                self.finish_file(id, rec);
            }
            Command::DownloadDiscard { .. } => {
                let id = id.ok_or_else(not_found)?;
                if !matches!(
                    state,
                    Some(DownloadStatus::Dangerous | DownloadStatus::Blocked)
                ) {
                    return Err(bad("error.download.notHeld"));
                }
                if let Some(d) = &dl {
                    let _ = std::fs::remove_file(&d.temp);
                }
                self.set_download(id, rec, |s| s.state = DownloadStatus::Cancelled);
            }
            Command::DownloadOpen { .. }
            | Command::DownloadReveal { .. }
            | Command::DownloadCopyFile { .. } => {
                if state != Some(DownloadStatus::Complete) {
                    return Err(bad("error.download.notComplete"));
                }
                let path = dl.map(|d| d.temp).ok_or_else(not_found)?;
                let job = match cmd {
                    Command::DownloadOpen { .. } => {
                        mado_platform::files::open
                            as fn(&Path) -> Result<(), mado_platform::PlatformError>
                    }
                    Command::DownloadReveal { .. } => mado_platform::files::reveal,
                    _ => mado_platform::files::copy_file,
                };
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = job(&path) {
                        tracing::warn!(error = %e, "a download action failed");
                    }
                });
            }
            Command::DownloadCopyPath { .. } => {
                let path = dl.map(|d| d.temp).ok_or_else(not_found)?;
                let text = path.to_string_lossy().into_owned();
                tokio::task::spawn_blocking(move || mado_platform::files::copy_text(&text));
            }
            Command::DownloadRemove { .. } => {
                let id = id.ok_or_else(not_found)?;
                if state == Some(DownloadStatus::InProgress) {
                    return Err(bad("error.download.running"));
                }
                self.forget_download(id, rec);
            }
            Command::DownloadOpenSource { .. } => {
                let url = dl
                    .and_then(|d| d.tab_url.or(Some(d.url)))
                    .ok_or_else(not_found)?;
                let window = self
                    .current_window()
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.window.notFound"))?;
                let (now, prefs) = (self.now(), self.prefs());
                self.model
                    .open_tab(window, url, None, true, None, now, prefs)?;
            }
            Command::DownloadClearList => {
                let done: Vec<DownloadId> = self
                    .state
                    .downloads
                    .values()
                    .filter(|s| {
                        !matches!(
                            s.state,
                            DownloadStatus::InProgress
                                | DownloadStatus::Paused
                                | DownloadStatus::Dangerous
                                | DownloadStatus::Blocked
                        )
                    })
                    .map(|s| s.id)
                    .collect();
                for id in done {
                    self.forget_download(id, rec);
                }
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                return Err(MadoError::internal());
            }
        }
        Ok(Value::Null)
    }

    fn forget_download(&mut self, id: DownloadId, rec: &mut PatchRecorder) {
        self.state.tx(rec).downloads().remove(&id);
        self.dls.remove(&id);
        if !self.state.profile.is_private {
            let pending = self.profile_db.writer().write(move |c| {
                c.execute(
                    "UPDATE downloads SET removed = 1 WHERE id = ?1",
                    [id.to_string()],
                )
                .map(|_| ())
            });
            tokio::task::spawn_local(async move {
                let _ = pending.await;
            });
        }
    }
}
