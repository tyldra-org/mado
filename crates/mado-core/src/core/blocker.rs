//! The blocker's lists and engines in the core (spec 10 §2, §3): the
//! shared engine, built off the core thread from the stored lists or the
//! bundled EasyList, the profile's custom engine, and the daily update.
//! Applying them to pages arrives with M3-T11 and M3-T13.

use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
use mado_blocker::Engine;
use mado_blocker::{Build, Resource};
use mado_types::patch::PatchRecorder;
use mado_types::state::BlockerList;
use serde_json::Value;

use super::{Core, Internal, Timer};

/// How often the lists are downloaded.
const EVERY: Duration = Duration::from_secs(24 * 3600);
/// Not before the app is up.
const FIRST: Duration = Duration::from_secs(60);
/// After a failed download.
const RETRY: Duration = Duration::from_secs(3600);

/// Where the shared blocker files live, and whether to download.
#[derive(Debug, Clone)]
pub struct BlockerConfig {
    /// `<root>/Blocker/`.
    pub dir: PathBuf,
    /// Download lists; tests turn it off.
    pub update: bool,
}

/// A finished build, with what the state shows.
pub struct Built {
    build: Option<Build>,
    resources: Vec<Resource>,
    lists: Vec<BlockerList>,
}

impl std::fmt::Debug for Built {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Built")
            .field("filters", &self.build.as_ref().map(|b| b.filters))
            .field("resources", &self.resources.len())
            .finish_non_exhaustive()
    }
}

fn kind(k: mado_lists::ListKind) -> &'static str {
    match k {
        mado_lists::ListKind::Ads => "ads",
        mado_lists::ListKind::Trackers => "trackers",
        mado_lists::ListKind::Annoyances => "annoyances",
        mado_lists::ListKind::Other => "other",
    }
}

fn build_now(dir: &Path, lists: &[mado_lists::FilterList]) -> Built {
    let build = match mado_blocker::build(dir, lists) {
        Ok(b) => Some(b),
        Err((e, kept)) => {
            tracing::warn!(error = %e, "the blocker kept its previous build");
            kept
        }
    };
    let lists = lists
        .iter()
        .map(|l| {
            let meta = mado_blocker::stored_meta(dir, &l.id).unwrap_or_default();
            BlockerList {
                id: l.id.clone(),
                kind: kind(l.kind).into(),
                filters: u32::try_from(meta.filters).unwrap_or(u32::MAX),
                updated_at: (meta.updated_at > 0).then_some(meta.updated_at),
            }
        })
        .collect();
    Built {
        build,
        resources: mado_blocker::stored_resources(dir),
        lists,
    }
}

impl Core {
    fn blocker_lists(&self) -> Vec<mado_lists::FilterList> {
        let setting: Vec<String> = match self.setting("blocker.lists") {
            Value::Array(a) => a
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        mado_blocker::selected(&mado_lists::filter_lists(), &setting)
    }

    /// At start: the engine from the cache or a build, the profile's own
    /// filters, and the update schedule.
    pub(super) fn start_blocker(&mut self) {
        if self.blocker.is_none() {
            return;
        }
        self.rebuild_blocker();
        self.load_custom_filters();
        self.schedule_blocker_update();
    }

    /// Builds (or finds) the shared engine off the core thread.
    pub(super) fn rebuild_blocker(&mut self) {
        let Some(config) = self.blocker.clone() else {
            return;
        };
        if self.blocker_building {
            self.blocker_rebuild_again = true;
            return;
        }
        self.blocker_building = true;
        let lists = self.blocker_lists();
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let built = tokio::task::spawn_blocking(move || build_now(&config.dir, &lists)).await;
            if let Ok(built) = built {
                let _ = inputs.send(super::Input::Internal(Internal::BlockerBuilt(Box::new(
                    built,
                ))));
            }
        });
    }

    pub(super) fn blocker_built(&mut self, built: Built, rec: &mut PatchRecorder) {
        self.blocker_building = false;
        let mut state = self.state.blocker.clone();
        if let Some(build) = &built.build
            && let Some(engine) = mado_blocker::engine_from(&build.bytes, built.resources)
        {
            tracing::info!(
                filters = build.filters,
                bundled = build.bundled,
                "blocker engine ready"
            );
            self.blocker_engine = Some(engine);
            state.filters = u32::try_from(build.filters).unwrap_or(u32::MAX);
            state.bundled = build.bundled;
        }
        state.lists_updated_at = built.lists.iter().filter_map(|l| l.updated_at).max();
        state.lists = built.lists;
        if state != self.state.blocker {
            self.state.tx(rec).set_blocker(state);
        }
        if std::mem::take(&mut self.blocker_rebuild_again) {
            self.rebuild_blocker();
        }
    }

    /// The profile's custom filters and hidden elements (10 §3).
    pub(super) fn load_custom_filters(&mut self) {
        let lines = self
            .profile_db
            .readers()
            .read(|c| {
                let mut lines: Vec<String> = c
                    .prepare("SELECT line FROM blocker_custom_filters ORDER BY id")?
                    .query_map([], |r| r.get(0))?
                    .filter_map(Result::ok)
                    .collect();
                let hidden = c
                    .prepare("SELECT host, selector FROM blocker_hidden ORDER BY id")?
                    .query_map([], |r| {
                        Ok(format!(
                            "{}##{}",
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?
                        ))
                    })?
                    .filter_map(Result::ok)
                    .collect::<Vec<_>>();
                lines.extend(hidden);
                Ok(lines)
            })
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "the custom filters could not be read");
                Vec::new()
            });
        self.blocker_custom = (!lines.is_empty()).then(|| mado_blocker::custom_engine(&lines));
    }

    fn update_stamp(dir: &Path) -> PathBuf {
        dir.join("updated-at")
    }

    fn schedule_blocker_update(&mut self) {
        let Some(config) = &self.blocker else { return };
        if !config.update {
            return;
        }
        let last: i64 = std::fs::read_to_string(Self::update_stamp(&config.dir))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let every = i64::try_from(EVERY.as_millis()).unwrap_or(i64::MAX);
        let due = last
            .saturating_add(every)
            .saturating_sub(self.now())
            .clamp(0, every);
        let wait = Duration::from_millis(u64::try_from(due).unwrap_or(0)).max(FIRST);
        self.timers.schedule(Timer::BlockerUpdate, wait);
    }

    /// Downloads the lists (the daily timer and `blocker.updateNow`).
    pub(super) fn update_blocker(&mut self, forced: bool, rec: &mut PatchRecorder) {
        let Some(config) = self.blocker.clone() else {
            return;
        };
        if !config.update || self.state.blocker.updating {
            return;
        }
        if !forced {
            let last: i64 = std::fs::read_to_string(Self::update_stamp(&config.dir))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            if self.now() - last < i64::try_from(EVERY.as_millis()).unwrap_or(i64::MAX) {
                // Another profile updated meanwhile.
                self.schedule_blocker_update();
                return;
            }
        }
        self.state.tx(rec).blocker().set_updating(true);
        let lists = self.blocker_lists();
        let resources = mado_lists::filter_lists().resources.clone();
        let now = self.now();
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let report = match mado_blocker::client() {
                Ok(client) => {
                    mado_blocker::update(&client, &config.dir, &lists, &resources, now).await
                }
                Err(_) => mado_blocker::UpdateReport {
                    failed: lists.iter().map(|l| l.id.clone()).collect(),
                    ..Default::default()
                },
            };
            let _ = inputs.send(super::Input::Internal(Internal::BlockerUpdated(report)));
        });
    }

    pub(super) fn blocker_updated(
        &mut self,
        report: mado_blocker::UpdateReport,
        rec: &mut PatchRecorder,
    ) {
        self.state.tx(rec).blocker().set_updating(false);
        let Some(config) = self.blocker.clone() else {
            return;
        };
        if report.failed.is_empty() {
            if let Err(e) = std::fs::create_dir_all(&config.dir).and_then(|()| {
                std::fs::write(Self::update_stamp(&config.dir), self.now().to_string())
            }) {
                tracing::warn!(error = %e, "the blocker update stamp was not written");
            }
            self.timers.schedule(Timer::BlockerUpdate, EVERY);
        } else {
            self.timers.schedule(Timer::BlockerUpdate, RETRY);
        }
        if !report.changed.is_empty() || report.resources_changed {
            self.rebuild_blocker();
        }
    }

    /// `blocker.lists` changed: download what is new, then rebuild.
    pub(super) fn blocker_lists_changed(&mut self, rec: &mut PatchRecorder) {
        if self.blocker.as_ref().is_some_and(|c| c.update) {
            self.update_blocker(true, rec);
        }
        self.rebuild_blocker();
    }

    /// Whether the shared or the custom engine blocks a request.
    #[cfg(test)]
    pub(super) fn blocker_blocks(&self, url: &str, source: &str, kind: &str) -> bool {
        let check = |e: &Option<Engine>| {
            e.as_ref()
                .is_some_and(|e| mado_blocker::blocks(e, url, source, kind))
        };
        check(&self.blocker_engine) || check(&self.blocker_custom)
    }
}
