//! Clearing browsing data (spec 08 §7): `data.clear`, `site.clearData`
//! and `privacy.clearOnExit`. Passwords live in the vault and are never
//! cleared here.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use mado_engine::{ClearDataRequest, DataKinds, EngineOp};
use mado_types::command::{Command, DataType, Origin};
use mado_types::state::DownloadStatus;
use mado_types::{ErrorCode, MadoError, PatchRecorder};
use rusqlite::params;
use serde_json::Value;

use super::{Core, Internal, Reply, Timer};

/// Quit anyway if clearing on quit takes longer than this.
pub const CLEAR_ON_EXIT_LIMIT: Duration = Duration::from_secs(5);

type Pending = Pin<Box<dyn Future<Output = ()>>>;

/// The engine's data kinds for `types`: `cookies` is every kind of site
/// data except the cache, `cache` the cache (spec 08 §7).
fn engine_kinds(types: &BTreeSet<DataType>) -> Option<DataKinds> {
    let site = types.contains(&DataType::Cookies);
    let cache = types.contains(&DataType::Cache);
    (site || cache).then_some(DataKinds {
        cookies: site,
        cache,
        local_storage: site,
        indexed_db: site,
        service_workers: site,
        other_site_data: site,
    })
}

/// The `DELETE` statements for `types` from `since` on (Unix
/// milliseconds; `None` for all time). History is cleared separately.
fn statements(types: &BTreeSet<DataType>) -> Vec<&'static str> {
    let mut out = Vec::new();
    if types.contains(&DataType::SiteSettings) {
        out.push("DELETE FROM site_settings WHERE granted_at >= ?1");
    }
    if types.contains(&DataType::FormHistory) {
        out.push("DELETE FROM form_history WHERE last_used_at >= ?1");
    }
    if types.contains(&DataType::BlockerStats) {
        // `day` counts days since 1970-01-01 UTC (spec 03).
        out.push("DELETE FROM blocker_stats WHERE day >= ?1 / 86400000");
    }
    if types.contains(&DataType::AiChats) {
        out.push("DELETE FROM ai_messages WHERE chat_id IN (SELECT id FROM ai_chats WHERE updated_at >= ?1)");
        out.push("DELETE FROM ai_chats WHERE updated_at >= ?1");
    }
    out
}

impl Core {
    pub(super) fn data_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.data.notAllowed",
            ));
        }
        let done = match cmd {
            Command::DataClear { range, types } => {
                let since = range.millis().map(|ms| self.now() - ms);
                self.clear(since, &types.into_iter().collect(), rec)
            }
            Command::SiteClearData { origin } => {
                let host = url::Url::parse(&origin)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or(origin)
                    .to_ascii_lowercase();
                if host.is_empty() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.site.badOrigin",
                    ));
                }
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.outbox.push(EngineOp::ClearData(
                    ClearDataRequest {
                        kinds: engine_kinds(&BTreeSet::from([DataType::Cookies, DataType::Cache]))
                            .unwrap_or_default(),
                        since: None,
                        hosts: Some(vec![host]),
                    },
                    Box::new(move |_| {
                        let _ = tx.send(());
                    }),
                ));
                Box::pin(async move {
                    let _ = rx.await;
                })
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                return Err(MadoError::internal());
            }
        };
        if let Some(reply) = reply_to.take() {
            tokio::task::spawn_local(async move {
                done.await;
                let _ = reply.send(Ok(Value::Null));
            });
        } else {
            tokio::task::spawn_local(done);
        }
        Ok(Value::Null)
    }

    /// Clears `types` from `since` on and returns a future that ends when
    /// the database and the engine are done. In-memory state changes now.
    fn clear(
        &mut self,
        since: Option<i64>,
        types: &BTreeSet<DataType>,
        rec: &mut PatchRecorder,
    ) -> Pending {
        let from = since.unwrap_or(i64::MIN);
        let mut pending: Vec<Pending> = Vec::new();
        if types.contains(&DataType::DownloadsList) {
            let gone: Vec<_> = self
                .state
                .downloads
                .values()
                .filter(|d| {
                    d.started_at >= from
                        && !matches!(
                            d.state,
                            DownloadStatus::InProgress
                                | DownloadStatus::Paused
                                | DownloadStatus::Dangerous
                                | DownloadStatus::Blocked
                        )
                })
                .map(|d| d.id)
                .collect();
            for id in gone {
                self.forget_download(id, rec);
            }
        }
        if types.contains(&DataType::SiteSettings) {
            self.sites.remove_granted_since(since);
            self.http_allowed.retain(|_, expires| {
                // An `httpAllowed` row lasts 30 days from when it was granted.
                *expires != 0 && *expires - super::errors::HTTP_ALLOWED_MS < from
            });
        }
        let history = types.contains(&DataType::History);
        let sql = statements(types);
        let now = self.now();
        if history || !sql.is_empty() {
            let write = self.profile_db.writer().write(move |c| {
                if history {
                    crate::history::delete(c, &[], &[], None, Some((from, i64::MAX)))?;
                    crate::history::recount(c, now)?;
                }
                for s in &sql {
                    c.execute(s, params![from])?;
                }
                Ok(())
            });
            pending.push(Box::pin(async move {
                if let Err(e) = write.await {
                    tracing::error!(error = %e, "browsing data was not cleared");
                }
            }));
        }
        if let Some(kinds) = engine_kinds(types) {
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.outbox.push(EngineOp::ClearData(
                ClearDataRequest {
                    kinds,
                    since,
                    hosts: None,
                },
                Box::new(move |result| {
                    if let Err(e) = &result {
                        tracing::error!(error = %e, "website data was not cleared");
                    }
                    let _ = tx.send(());
                }),
            ));
            pending.push(Box::pin(async move {
                let _ = rx.await;
            }));
        }
        Box::pin(async move {
            for p in pending {
                p.await;
            }
        })
    }

    /// Quits, after clearing what `privacy.clearOnExit` lists (spec 08 §7).
    /// Quits anyway after [`CLEAR_ON_EXIT_LIMIT`].
    pub(super) fn quit_now(&mut self) {
        if self.clearing_on_exit {
            return;
        }
        let types: BTreeSet<DataType> = self
            .settings
            .get::<mado_settings::PrivacyClearOnExit>()
            .iter()
            .filter_map(|t| serde_json::from_value(Value::String(t.clone())).ok())
            .collect();
        if types.is_empty() {
            self.ui.quit();
            return;
        }
        self.clearing_on_exit = true;
        // The app is going: the windows that would show these patches close.
        let mut rec = PatchRecorder::new();
        let done = self.clear(None, &types, &mut rec);
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            done.await;
            let _ = inputs.send(super::Input::Internal(Internal::ClearedOnExit));
        });
        self.timers
            .schedule(Timer::ClearOnExitLimit, CLEAR_ON_EXIT_LIMIT);
    }

    /// Clearing on quit finished or ran out of time.
    pub(super) fn cleared_on_exit(&mut self) {
        if std::mem::take(&mut self.clearing_on_exit) {
            self.ui.quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_mean_every_kind_of_site_data_but_the_cache() {
        let k = engine_kinds(&BTreeSet::from([DataType::Cookies])).expect("kinds");
        assert!(k.cookies && k.local_storage && k.indexed_db && k.service_workers && !k.cache);
        let c = engine_kinds(&BTreeSet::from([DataType::Cache])).expect("kinds");
        assert!(c.cache && !c.cookies);
        assert!(engine_kinds(&BTreeSet::from([DataType::History])).is_none());
    }

    #[test]
    fn every_statement_is_valid_sql() {
        let c = rusqlite::Connection::open_in_memory().expect("db");
        c.execute_batch(include_str!(
            "../../../mado-storage/migrations/profile/0001_initial.sql"
        ))
        .expect("schema");
        let all = BTreeSet::from([
            DataType::SiteSettings,
            DataType::FormHistory,
            DataType::BlockerStats,
            DataType::AiChats,
        ]);
        for s in statements(&all) {
            c.execute(s, params![0_i64]).expect(s);
        }
    }
}
