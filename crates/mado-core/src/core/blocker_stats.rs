//! The blocker's statistics and controls (spec 10 §7, §8, 08 §9): counts
//! per site and day, tracker domains, the privacy report, the site switch,
//! the list choice and "Page not working?".

use std::collections::BTreeMap;

use mado_engine::{EngineOp, TabCommand};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::{Notice, NoticeAction};
use mado_types::{ErrorCode, MadoError, TabId};
use rusqlite::params;
use serde_json::{Value, json};

use super::Core;

const DAY_MS: i64 = 86_400_000;
/// Three reloads of one URL within this long suggest the page is broken.
const RELOAD_WINDOW_MS: i64 = 20_000;

/// Counts not yet written.
#[derive(Debug, Default)]
pub struct StatsBuffer {
    /// (day, site) to (ads, trackers).
    sites: BTreeMap<(i64, String), (i64, i64)>,
    /// (day, tracker domain) to blocks.
    trackers: BTreeMap<(i64, String), i64>,
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

fn domain_of(host: &str) -> String {
    psl::domain_str(host).unwrap_or(host).to_owned()
}

impl Core {
    /// A load was blocked on `tab`'s page (10 §7).
    pub(super) fn content_blocked(&mut self, tab: TabId, url: &str) {
        let Some(page) = self.model.tabs.get_mut(&tab) else {
            return;
        };
        page.blocked_count = page.blocked_count.saturating_add(1);
        if self.state.profile.is_private {
            return;
        }
        let Some(site) = host_of(&page.url) else {
            return;
        };
        let page_url = page.url.clone();
        let tracker = self.blocker_trackers.as_ref().is_some_and(|e| {
            mado_blocker::blocks(e, url, &page_url, "other")
                || mado_blocker::blocks(e, url, &page_url, "script")
        });
        let day = self.now() / DAY_MS;
        let entry = self.blocker_stats.sites.entry((day, site)).or_default();
        if tracker {
            entry.1 += 1;
            if let Some(h) = host_of(url) {
                *self
                    .blocker_stats
                    .trackers
                    .entry((day, domain_of(&h)))
                    .or_default() += 1;
            }
        } else {
            entry.0 += 1;
        }
    }

    /// Writes the buffered counts (every 60 s and at quit).
    pub(super) fn flush_blocker_stats(&mut self) -> Option<mado_storage::PendingWrite<()>> {
        let buffer = std::mem::take(&mut self.blocker_stats);
        if buffer.sites.is_empty() && buffer.trackers.is_empty() {
            return None;
        }
        Some(self.profile_db.writer().write(move |c| {
            for ((day, host), (ads, trackers)) in &buffer.sites {
                c.execute(
                    "INSERT INTO blocker_stats (day, host, ads, trackers) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(day, host) DO UPDATE SET ads = ads + ?3, trackers = trackers + ?4",
                    params![day, host, ads, trackers],
                )?;
            }
            for ((day, domain), count) in &buffer.trackers {
                c.execute(
                    "INSERT INTO blocker_tracker_stats (day, domain, count) VALUES (?1, ?2, ?3)
                     ON CONFLICT(day, domain) DO UPDATE SET count = count + ?3",
                    params![day, domain, count],
                )?;
            }
            Ok(())
        }))
    }

    pub(super) fn flush_blocker_stats_later(&mut self) {
        if let Some(pending) = self.flush_blocker_stats() {
            tokio::task::spawn_local(async move {
                if let Err(e) = pending.await {
                    tracing::error!(error = %e, "blocker statistics were not saved");
                }
            });
        }
        self.timers.schedule(
            super::Timer::BlockerStatsFlush,
            std::time::Duration::from_secs(60),
        );
    }

    /// "Page not working?" (10 §8): the third reload of one URL within 20 s
    /// on a page where something was blocked offers to turn the blocker off.
    pub(super) fn reload_seen(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        let Some(t) = self.model.tabs.get(&tab) else {
            return;
        };
        if t.blocked_count == 0 {
            return;
        }
        let (url, now) = (t.url.clone(), self.now());
        let seen = self.reloads.entry(tab).or_default();
        seen.retain(|(at, u)| *u == url && now - at < RELOAD_WINDOW_MS);
        seen.push((now, url.clone()));
        if seen.len() < 3 {
            return;
        }
        seen.clear();
        let Some(host) = host_of(&url) else { return };
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: "notice.pageNotWorking".into(),
            args: BTreeMap::from([("host".to_owned(), json!(host))]),
            action: Some(NoticeAction {
                label_key: "notice.turnOffBlockerHere".into(),
                command: Command::BlockerSetSite {
                    host,
                    enabled: false,
                },
            }),
            undo: None,
            expires_at: Some(now + 15_000),
        };
        self.add_notice(notice, rec);
    }

    /// The controls of 10 §8 and the report of 08 §9.
    pub(super) fn blocker_controls(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        match cmd {
            Command::BlockerSetSite { host, enabled } => {
                let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
                if host.is_empty() || host.contains('/') {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.site.badOrigin",
                    ));
                }
                self.blocker_off_hosts.retain(|h| *h != host);
                if !enabled {
                    self.blocker_off_hosts.push(host.clone());
                    self.blocker_off_hosts.sort();
                }
                self.state
                    .tx(rec)
                    .blocker()
                    .set_off_hosts(self.blocker_off_hosts.clone());
                self.push_blocker_rules();
                let (h, on) = (host.clone(), enabled);
                let pending = self.profile_db.writer().write(move |c| {
                    if on {
                        c.execute("DELETE FROM blocker_site WHERE host = ?1", [h])
                            .map(|_| ())
                    } else {
                        c.execute(
                            "INSERT OR REPLACE INTO blocker_site (host, enabled) VALUES (?1, 0)",
                            [h],
                        )
                        .map(|_| ())
                    }
                });
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "the blocker site switch was not saved");
                    }
                });
                // Its tabs load again, without (or with) the blocker.
                let tabs: Vec<TabId> = self
                    .model
                    .tabs
                    .iter()
                    .filter(|(_, t)| {
                        host_of(&t.url)
                            .is_some_and(|h| mado_engine::host_in(&h, std::slice::from_ref(&host)))
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for tab in tabs {
                    self.outbox.push(EngineOp::Apply(
                        tab,
                        TabCommand::Reload {
                            bypass_cache: false,
                        },
                    ));
                }
                Ok(Value::Null)
            }
            Command::BlockerSetLists { ids } => {
                let known = mado_lists::filter_lists();
                if let Some(bad) = ids
                    .iter()
                    .find(|id| !known.lists.iter().any(|l| &l.id == *id))
                {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.blocker.unknownList",
                    )
                    .arg("id", bad.clone()));
                }
                let now = self.now();
                let out = self.settings.set(
                    "blocker.lists",
                    json!(ids),
                    origin,
                    None,
                    self.tokens.at(now),
                    now,
                )?;
                self.apply_outcome(out, rec);
                Ok(Value::Null)
            }
            Command::BlockerAvailableLists => {
                let selected: Vec<String> = self.blocker_selected_ids();
                let dir = self
                    .blocker
                    .as_ref()
                    .map(|c| c.dir.clone())
                    .unwrap_or_default();
                Ok(json!(
                    mado_lists::filter_lists()
                        .lists
                        .iter()
                        .map(|l| {
                            let meta = mado_blocker::stored_meta(&dir, &l.id).unwrap_or_default();
                            json!({
                                "id": l.id,
                                "kind": l.kind_name(),
                                "enabled": selected.contains(&l.id),
                                "filters": meta.filters,
                                "updatedAt": (meta.updated_at > 0).then_some(meta.updated_at),
                            })
                        })
                        .collect::<Vec<_>>()
                ))
            }
            Command::BlockerCustomFilters => {
                let lines: Vec<String> = self
                    .profile_db
                    .readers()
                    .read(|c| {
                        c.prepare("SELECT line FROM blocker_custom_filters ORDER BY id")?
                            .query_map([], |r| r.get(0))?
                            .collect()
                    })
                    .map_err(|_| MadoError::internal())?;
                Ok(json!(lines.join("\n")))
            }
            Command::SitePrivacyReport { days } => {
                if !matches!(days, 7 | 30) {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.blocker.badDays",
                    ));
                }
                let since = self.now() / DAY_MS - i64::from(days) + 1;
                type Rows = (Vec<(String, i64, i64)>, Vec<(String, i64)>);
                let (mut sites, mut domains): Rows = self
                    .profile_db
                    .readers()
                    .read(move |c| {
                        let sites = c
                            .prepare("SELECT host, SUM(ads), SUM(trackers) FROM blocker_stats WHERE day >= ?1 GROUP BY host")?
                            .query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                            .collect::<rusqlite::Result<_>>()?;
                        let domains = c
                            .prepare("SELECT domain, SUM(count) FROM blocker_tracker_stats WHERE day >= ?1 GROUP BY domain")?
                            .query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?
                            .collect::<rusqlite::Result<_>>()?;
                        Ok((sites, domains))
                    })
                    .map_err(|_| MadoError::internal())?;
                // What is still in memory counts too.
                for ((day, host), (ads, trackers)) in &self.blocker_stats.sites {
                    if *day < since {
                        continue;
                    }
                    match sites.iter_mut().find(|s| &s.0 == host) {
                        Some(s) => {
                            s.1 += ads;
                            s.2 += trackers;
                        }
                        None => sites.push((host.clone(), *ads, *trackers)),
                    }
                }
                for ((day, domain), count) in &self.blocker_stats.trackers {
                    if *day < since {
                        continue;
                    }
                    match domains.iter_mut().find(|d| &d.0 == domain) {
                        Some(d) => d.1 += count,
                        None => domains.push((domain.clone(), *count)),
                    }
                }
                let (ads, trackers) = sites.iter().fold((0, 0), |(a, t), s| (a + s.1, t + s.2));
                sites.sort_by(|x, y| (y.1 + y.2).cmp(&(x.1 + x.2)).then_with(|| x.0.cmp(&y.0)));
                domains.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
                Ok(json!({
                    "days": days,
                    "ads": ads,
                    "trackers": trackers,
                    "sites": sites.iter().take(20).map(|s| json!({ "host": s.0, "ads": s.1, "trackers": s.2 })).collect::<Vec<_>>(),
                    "trackerDomains": domains.iter().take(20).map(|d| json!({ "domain": d.0, "count": d.1 })).collect::<Vec<_>>(),
                }))
            }
            _ => Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.command.unknown",
            )),
        }
    }
}
