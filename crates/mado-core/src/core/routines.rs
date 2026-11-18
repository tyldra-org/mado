//! Routines in the core (spec 13 §12): tasks on a schedule. Each enabled
//! routine's next run is kept in `ai_routines.next_run_at`; one timer fires
//! at the earliest. Wake from sleep and clock changes recompute every next
//! run; a run missed while Mado was closed runs once at launch if it is the
//! same calendar day.

use std::collections::BTreeMap;

use jiff::Zoned;
use jiff::tz::TimeZone;
use mado_agent::schedule::{self, Schedule};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::{ErrorCode, MadoError, TaskId};
use rusqlite::params;
use serde_json::{Value, json};

use super::Core;

/// What a routine run answers when it found nothing new (13 §12).
pub(super) const NOTHING_NEW: &str = "NOTHING_NEW";

/// One routine (`ai_routines`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Routine {
    pub id: String,
    pub prompt: String,
    pub schedule: String,
    pub mode: String,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub created_at: i64,
}

impl Routine {
    fn json(&self) -> Value {
        json!({ "id": self.id, "prompt": self.prompt, "schedule": self.schedule, "mode": self.mode, "enabled": self.enabled, "lastRunAt": self.last_run_at, "nextRunAt": self.next_run_at })
    }
}

/// The routines' runtime state in the core.
#[derive(Debug, Default)]
pub(super) struct Routines {
    pub list: BTreeMap<String, Routine>,
    /// Running routine tasks and their routine.
    pub tasks: BTreeMap<TaskId, String>,
    /// The current timer's generation; older timers are ignored.
    pub generation: u64,
}

fn load(c: &rusqlite::Connection) -> rusqlite::Result<BTreeMap<String, Routine>> {
    c.prepare("SELECT id, prompt, schedule, mode, enabled, last_run_at, next_run_at, created_at FROM ai_routines")?
        .query_map([], |r| {
            Ok(Routine {
                id: r.get(0)?,
                prompt: r.get(1)?,
                schedule: r.get(2)?,
                mode: r.get(3)?,
                enabled: r.get(4)?,
                last_run_at: r.get(5)?,
                next_run_at: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?
        .map(|r| r.map(|r| (r.id.clone(), r)))
        .collect()
}

fn zoned(ms: i64) -> Zoned {
    jiff::Timestamp::from_millisecond(ms)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(TimeZone::system())
}

fn bad(key: &str) -> MadoError {
    MadoError::new(ErrorCode::InvalidInput, key.to_owned())
}

fn mode_of(mode: Option<String>) -> Result<String, MadoError> {
    match mode.as_deref() {
        None | Some("readOnly") => Ok("readOnly".into()),
        Some("askBeforeActing") => Ok("askBeforeActing".into()),
        Some(_) => Err(bad("error.ai.routineMode")),
    }
}

/// The next run after `now`, in milliseconds.
fn next_run(r: &Routine, now: i64) -> Option<i64> {
    let s = schedule::parse(&r.schedule).ok()?;
    s.next(&zoned(now), &zoned(r.created_at))
        .map(|z| z.timestamp().as_millisecond())
}

impl Core {
    fn save_routine(&self, r: &Routine) {
        if self.state.profile.is_private {
            return;
        }
        let r = r.clone();
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT INTO ai_routines (id, prompt, schedule, mode, enabled, last_run_at, next_run_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET prompt = excluded.prompt, schedule = excluded.schedule, mode = excluded.mode, enabled = excluded.enabled, last_run_at = excluded.last_run_at, next_run_at = excluded.next_run_at",
                params![r.id, r.prompt, r.schedule, r.mode, r.enabled, r.last_run_at, r.next_run_at, r.created_at],
            )
            .map(drop)
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "a routine was not saved");
            }
        });
    }

    /// At launch: the routines, missed runs of today, `on open` routines,
    /// and the timer.
    pub(super) fn load_routines(&mut self, rec: &mut PatchRecorder) {
        if self.state.profile.is_private {
            return;
        }
        self.routines.list = self.profile_db.readers().read(load).unwrap_or_else(|e| {
            tracing::error!(error = %e, "the routines could not be read");
            BTreeMap::new()
        });
        let now = self.now();
        let ids: Vec<String> = self.routines.list.keys().cloned().collect();
        for id in ids {
            let Some(r) = self.routines.list.get(&id).cloned() else {
                continue;
            };
            if !r.enabled {
                continue;
            }
            let on_open = matches!(schedule::parse(&r.schedule), Ok(Schedule::OnOpen));
            // The latest run missed while Mado was closed; only today's runs.
            let mut latest = None;
            let mut at = r.next_run_at;
            for _ in 0..20_000 {
                match at {
                    Some(a) if a <= now => {
                        latest = Some(a);
                        at = next_run(&r, a);
                    }
                    _ => break,
                }
            }
            let missed = latest.is_some_and(|at| schedule::run_missed(&zoned(at), &zoned(now)));
            if on_open || missed {
                self.run_routine(&id, rec);
            } else {
                self.reschedule_one(&id, now);
            }
        }
        self.arm_routines();
    }

    fn reschedule_one(&mut self, id: &str, now: i64) {
        let Some(r) = self.routines.list.get_mut(id) else {
            return;
        };
        r.next_run_at = if r.enabled { next_run(r, now) } else { None };
        let r = r.clone();
        self.save_routine(&r);
    }

    /// 13 §12: after wake and clock changes, every next run again.
    pub(super) fn recompute_routines(&mut self) {
        let now = self.now();
        let ids: Vec<String> = self.routines.list.keys().cloned().collect();
        for id in ids {
            self.reschedule_one(&id, now);
        }
        self.arm_routines();
    }

    /// One timer for the earliest next run.
    fn arm_routines(&mut self) {
        self.routines.generation += 1;
        let now = self.now();
        let Some(at) = self
            .routines
            .list
            .values()
            .filter(|r| r.enabled)
            .filter_map(|r| r.next_run_at)
            .min()
        else {
            return;
        };
        let wait = u64::try_from((at - now).max(0)).unwrap_or(0);
        self.timers.schedule(
            crate::Timer::Routines(self.routines.generation),
            std::time::Duration::from_millis(wait),
        );
    }

    /// The timer fired: runs the routines that are due.
    pub(super) fn routines_due(&mut self, generation: u64, rec: &mut PatchRecorder) {
        if generation != self.routines.generation {
            return;
        }
        let now = self.now();
        let due: Vec<String> = self
            .routines
            .list
            .values()
            .filter(|r| r.enabled && r.next_run_at.is_some_and(|at| at <= now + 1000))
            .map(|r| r.id.clone())
            .collect();
        for id in due {
            self.run_routine(&id, rec);
        }
        self.arm_routines();
    }

    /// Starts a routine's task, unless routines are off or energy saver is
    /// on (13 §12); either way its next run is set.
    fn run_routine(&mut self, id: &str, rec: &mut PatchRecorder) -> Option<TaskId> {
        let now = self.now();
        let paused = self.setting("features.ai") != true
            || self.setting("features.routines") != true
            || self.energy_saver();
        let r = self.routines.list.get(id)?.clone();
        let mut started = None;
        if !paused {
            let last = r.last_run_at.map_or_else(
                || "never".to_owned(),
                |t| mado_agent::memory::utc(t).1 + " UTC",
            );
            let prompt = format!(
                "{}\n\n(This runs on the schedule \"{}\"; its last run was {last}. If nothing is new since then, answer exactly {NOTHING_NEW}.)",
                r.prompt, r.schedule
            );
            let cmd = Command::AiTaskStart {
                prompt,
                mode: Some("ask".into()),
                access: None,
                runtime: None,
                cli: None,
                model: None,
                effort: None,
                scope: None,
            };
            match self.task_command(&Origin::System, cmd, &mut None, rec) {
                Ok(v) => {
                    if let Ok(task) = serde_json::from_value::<TaskId>(v["taskId"].clone()) {
                        self.routines.tasks.insert(task, id.to_owned());
                        // 13 §12: readOnly routines never act consequentially.
                        if r.mode == "readOnly" {
                            self.guards.entry(task).or_default().read_only = true;
                        }
                        started = Some(task);
                    }
                }
                Err(e) => tracing::warn!(error = %e.message_key, "a routine did not start"),
            }
        }
        if let Some(r) = self.routines.list.get_mut(id) {
            if started.is_some() {
                r.last_run_at = Some(now);
            }
            r.next_run_at = next_run(r, now);
            let r = r.clone();
            self.save_routine(&r);
        }
        started
    }

    /// A routine's task ended: a notification, unless it found nothing new,
    /// which leaves no trace in the task list.
    pub(super) fn routine_task_done(
        &mut self,
        task: TaskId,
        answer: &str,
        rec: &mut PatchRecorder,
    ) {
        let Some(id) = self.routines.tasks.remove(&task) else {
            return;
        };
        if answer.trim() == NOTHING_NEW {
            self.state.tx(rec).ai().tasks().remove(&task);
            return;
        }
        let title: String = self
            .routines
            .list
            .get(&id)
            .map(|r| r.prompt.chars().take(60).collect())
            .unwrap_or_default();
        let body: String = answer
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        self.ui
            .show_notification(&format!("routine-{task}"), &title, &body);
    }

    /// `ai.routine.*`.
    pub(super) fn routine_command(
        &mut self,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        if self.state.profile.is_private {
            return Err(MadoError::new(
                ErrorCode::Unsupported,
                "error.ai.routinesPrivate",
            ));
        }
        let not_found = || MadoError::new(ErrorCode::NotFound, "error.ai.noRoutine");
        let now = self.now();
        let answer = match cmd {
            Command::AiRoutineList => {
                let mut list: Vec<&Routine> = self.routines.list.values().collect();
                list.sort_by_key(|r| r.created_at);
                return Ok(Value::Array(list.into_iter().map(Routine::json).collect()));
            }
            Command::AiRoutineCreate {
                prompt,
                schedule: s,
                mode,
            } => {
                schedule::parse(&s).map_err(|why| bad("error.ai.schedule").arg("reason", why))?;
                if prompt.trim().is_empty() {
                    return Err(bad("error.ai.routinePrompt"));
                }
                let mut r = Routine {
                    id: uuid::Uuid::now_v7().simple().to_string(),
                    prompt,
                    schedule: s,
                    mode: mode_of(mode)?,
                    enabled: true,
                    last_run_at: None,
                    next_run_at: None,
                    created_at: now,
                };
                r.next_run_at = next_run(&r, now);
                self.save_routine(&r);
                let v = json!({ "routineId": r.id, "nextRunAt": r.next_run_at });
                self.routines.list.insert(r.id.clone(), r);
                v
            }
            Command::AiRoutineUpdate {
                routine_id,
                prompt,
                schedule: s,
                mode,
                enabled,
            } => {
                if let Some(s) = &s {
                    schedule::parse(s)
                        .map_err(|why| bad("error.ai.schedule").arg("reason", why))?;
                }
                let mode = mode.map(|m| mode_of(Some(m))).transpose()?;
                let r = self
                    .routines
                    .list
                    .get_mut(&routine_id)
                    .ok_or_else(not_found)?;
                if let Some(p) = prompt {
                    r.prompt = p;
                }
                if let Some(s) = s {
                    r.schedule = s;
                }
                if let Some(m) = mode {
                    r.mode = m;
                }
                if let Some(e) = enabled {
                    r.enabled = e;
                }
                r.next_run_at = if r.enabled { next_run(r, now) } else { None };
                let r = r.clone();
                self.save_routine(&r);
                r.json()
            }
            Command::AiRoutineDelete { routine_id } => {
                self.routines
                    .list
                    .remove(&routine_id)
                    .ok_or_else(not_found)?;
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute("DELETE FROM ai_routines WHERE id = ?1", [routine_id])
                        .map(drop)
                });
                tokio::task::spawn_local(async move {
                    let _ = pending.await;
                });
                Value::Null
            }
            Command::AiRoutineRunNow { routine_id } => {
                if !self.routines.list.contains_key(&routine_id) {
                    return Err(not_found());
                }
                if self.setting("features.routines") != true {
                    return Err(MadoError::new(
                        ErrorCode::Unsupported,
                        "error.ai.routinesOff",
                    ));
                }
                let task = self
                    .run_routine(&routine_id, rec)
                    .ok_or_else(|| MadoError::new(ErrorCode::Internal, "error.ai.routineFailed"))?;
                json!({ "taskId": task })
            }
            _ => {
                return Err(MadoError::new(
                    ErrorCode::Unsupported,
                    "error.unknownCommand",
                ));
            }
        };
        self.arm_routines();
        Ok(answer)
    }
}
