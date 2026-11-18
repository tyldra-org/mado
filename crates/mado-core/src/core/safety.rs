//! Approvals, audit and spending in the core (spec 13 §9): each task's
//! mode, scope, allowed and denied actions and fingerprints; approval
//! cards; `ai_audit` rows; costs against the limits.

use std::collections::BTreeSet;

use mado_agent::safety::{Class, Fingerprints, Mode, needs_approval};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::{PromptState, TaskStatus};
use mado_types::{ErrorCode, MadoError, PromptId, TabId, TaskId};
use rusqlite::params;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::Core;

/// What the core knows about one task's safety (13 §9).
#[derive(Debug, Default)]
pub(super) struct TaskGuard {
    /// The task's own mode; `None` follows `ai.approvalMode`.
    pub mode: Option<Mode>,
    /// Hosts in scope; `None` for a control client's implicit task, which
    /// the person runs themselves.
    pub scope: Option<BTreeSet<String>>,
    pub allowed: BTreeSet<(Class, String)>,
    pub denied: BTreeSet<(Class, String)>,
    pub prints: Fingerprints,
    pub cost_micros: u64,
    /// A raised limit for this task (13 §9.6).
    pub limit_micros: Option<u64>,
    /// A `readOnly` routine's task: consequential actions are refused
    /// without a card (13 §12).
    pub read_only: bool,
}

/// An action waiting for the person.
#[derive(Debug)]
pub(super) struct Pending {
    pub task: TaskId,
    pub class: Class,
    pub site: String,
    pub reply: oneshot::Sender<Result<(), MadoError>>,
}

/// What a tool asks before acting.
#[derive(Debug)]
pub struct Ask {
    pub origin: Origin,
    pub tab: Option<TabId>,
    pub class: Class,
    /// The URL the action is on or goes to.
    pub url: String,
    /// The action in plain words (`Click "Place order"`).
    pub detail: String,
    pub amount: Option<String>,
}

fn denied() -> MadoError {
    MadoError::new(ErrorCode::PermissionDenied, "error.ai.deniedByUser")
}

impl Core {
    fn guard(&mut self, task: TaskId) -> &mut TaskGuard {
        self.guards.entry(task).or_default()
    }

    /// 9.1–9.3: lets the action through, refuses it, or shows a card and
    /// answers when the person does.
    pub(super) fn ask_approval(
        &mut self,
        ask: Ask,
        reply: oneshot::Sender<Result<(), MadoError>>,
        rec: &mut PatchRecorder,
    ) {
        let Some(task) = self.task_of(&ask.origin, rec) else {
            let _ = reply.send(Ok(()));
            return;
        };
        let site = mado_agent::safety::host(&ask.url).unwrap_or_default();
        let mode_setting = Mode::from_key(&self.setting_str("ai.approvalMode"));
        let g = self.guard(task);
        let mode = g.mode.unwrap_or(mode_setting);
        if g.denied.contains(&(ask.class, site.clone())) {
            let _ = reply.send(Err(denied()));
            return;
        }
        if g.read_only && ask.class != Class::LeaveScope {
            let _ = reply.send(Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.ai.readOnlyRoutine",
            )));
            return;
        }
        if ask.class == Class::LeaveScope
            && mado_agent::safety::in_scope(g.scope.as_ref(), &ask.url)
        {
            let _ = reply.send(Ok(()));
            return;
        }
        if g.allowed.contains(&(ask.class, site.clone())) || !needs_approval(mode, ask.class) {
            let _ = reply.send(Ok(()));
            return;
        }
        let id = PromptId::new();
        let window = ask
            .tab
            .and_then(|t| self.model.tabs.get(&t).map(|t| t.window_id))
            .or_else(|| self.current_window());
        let mut prompts = self.state.prompts.clone();
        prompts.insert(
            id,
            PromptState::AiApproval {
                id,
                window_id: window,
                tab_id: ask.tab,
                task_id: task,
                action: ask.class.key().into(),
                detail: ask.detail.clone(),
                site: site.clone(),
                amount: ask.amount.clone(),
                allow_for_task: ask.class.may_allow_for_task(),
            },
        );
        self.state.tx(rec).set_prompts(prompts);
        self.ui.show_notification(
            &id.to_string(),
            "Mado needs your approval",
            &format!("{} on {site}", ask.detail),
        );
        self.audit(task, "approval", json!({ "asked": ask.class.key(), "site": site, "detail": ask.detail, "amount": ask.amount }));
        self.pending_approvals.insert(
            id,
            Pending {
                task,
                class: ask.class,
                site,
                reply,
            },
        );
    }

    /// `ai.approval.respond { promptId, answer }`: `once`, `task` or `deny`.
    pub(super) fn approval_respond(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let Command::AiApprovalRespond { prompt_id, answer } = cmd else {
            return Err(MadoError::new(
                ErrorCode::Unsupported,
                "error.unknownCommand",
            ));
        };
        // Only the person answers (Full control clients included: 14 §2.1).
        if !origin.is_user() {
            return Err(MadoError::new(ErrorCode::Protected, "error.ai.notAllowed"));
        }
        let p = self
            .pending_approvals
            .remove(&prompt_id)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"))?;
        let mut prompts = self.state.prompts.clone();
        prompts.remove(&prompt_id);
        self.state.tx(rec).set_prompts(prompts);
        let (task, class, site) = (p.task, p.class, p.site.clone());
        let result = match answer.as_str() {
            "once" => Ok(()),
            "task" if class.may_allow_for_task() => {
                self.guard(task).allowed.insert((class, site.clone()));
                Ok(())
            }
            _ => {
                // A denial holds for this class on this site for the rest
                // of the task (9.3).
                self.guard(task).denied.insert((class, site.clone()));
                Err(denied())
            }
        };
        self.audit(
            task,
            "approval",
            json!({ "answered": answer, "action": class.key(), "site": site }),
        );
        let _ = p.reply.send(result);
        Ok(Value::Null)
    }

    /// Text a tool read from `origin`, for the cross-site guard (9.5).
    pub(super) fn note_read(
        &mut self,
        origin: &Origin,
        from: &str,
        text: &str,
        rec: &mut PatchRecorder,
    ) {
        if let Some(task) = self.task_of(origin, rec) {
            self.guard(task).prints.note(from, text);
        }
    }

    /// The other site `text` was read from, if it was (9.5).
    pub(super) fn crosses(
        &mut self,
        origin: &Origin,
        target: &str,
        text: &str,
        rec: &mut PatchRecorder,
    ) -> Option<String> {
        let task = self.task_of(origin, rec)?;
        self.guards.get(&task)?.prints.crosses(target, text)
    }

    /// A row of `ai_audit` (9.7). Secrets were removed by the caller.
    pub(super) fn audit(&self, task: TaskId, kind: &str, detail: Value) {
        if self.state.profile.is_private {
            return;
        }
        let (t, k, d, at) = (
            task.to_string(),
            kind.to_owned(),
            detail.to_string(),
            self.now(),
        );
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT INTO ai_audit (task_id, at, kind, detail) SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM ai_tasks WHERE id = ?1)",
                params![t, at, k, d],
            )
            .map(drop)
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an audit row was not written");
            }
        });
    }

    /// The task's row, so its audit rows have a parent.
    pub(super) fn ensure_task_row(&self, task: TaskId, prompt: &str, runtime: &str) {
        if self.state.profile.is_private {
            return;
        }
        let (t, p, r, at) = (
            task.to_string(),
            prompt.to_owned(),
            runtime.to_owned(),
            self.now(),
        );
        let mode = self.setting_str("ai.approvalMode");
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT OR IGNORE INTO ai_tasks (id, prompt, status, mode, runtime, started_at, scope) VALUES (?1, ?2, 'running', ?3, ?4, ?5, '[]')",
                params![t, p, mode, r, at],
            )
            .map(drop)
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "a task row was not written");
            }
        });
    }

    /// 9.6: one model call's cost. Writes the audit row, the task's and
    /// the day's spend; answers whether a limit is now reached (the task
    /// pauses).
    pub(super) fn account(
        &mut self,
        task: TaskId,
        connection: &str,
        provider: &str,
        model: &str,
        usage: (u64, u64, u64),
        rec: &mut PatchRecorder,
    ) -> bool {
        let catalog = mado_lists::models();
        let price = mado_ai::catalog::known(&catalog, provider, model).map(|i| i.price_per_m_tok);
        let (input, output, cached) = usage;
        // US dollars per million tokens is micro-dollars per token.
        let cost = price.map_or(0.0, |p| {
            input.saturating_sub(cached) as f64 * p.input
                + cached as f64 * p.cached_input
                + output as f64 * p.output
        });
        let cost = cost.round().max(0.0) as u64;
        let g = self.guard(task);
        g.cost_micros += cost;
        let (spent, raised) = (g.cost_micros, g.limit_micros);
        self.audit(task, "model_call", json!({ "model": format!("{connection}:{model}"), "tokensIn": input, "tokensOut": output, "cached": cached, "costMicros": cost }));
        if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
            t.set_cost_micros(spent);
        }
        if !self.state.profile.is_private {
            let (t, c, day) = (
                task.to_string(),
                connection.to_owned(),
                self.now() / 86_400_000,
            );
            let pending = self.profile_db.writer().write(move |db| {
                db.execute(
                    "UPDATE ai_tasks SET cost_micros = cost_micros + ?2 WHERE id = ?1",
                    params![t, cost as i64],
                )?;
                db.execute(
                    "INSERT INTO ai_spend (day, connection_id, cost_micros) VALUES (?1, ?2, ?3)
                     ON CONFLICT (day, connection_id) DO UPDATE SET cost_micros = cost_micros + ?3",
                    params![day, c, cost as i64],
                )
                .map(drop)
            });
            tokio::task::spawn_local(async move {
                if let Err(e) = pending.await {
                    tracing::error!(error = %e, "spending was not written");
                }
            });
        }
        let per_task = raised.or_else(|| self.setting("ai.spendLimits.perTask").as_u64());
        let over_task = per_task.is_some_and(|l| spent >= l);
        let over_day = self
            .setting("ai.spendLimits.perDay")
            .as_u64()
            .is_some_and(|l| {
                let day = self.now() / 86_400_000;
                let today: u64 = self
                    .profile_db
                    .readers()
                    .read(move |c| {
                        c.query_row(
                            "SELECT COALESCE(SUM(cost_micros), 0) FROM ai_spend WHERE day = ?1",
                            [day],
                            |r| r.get::<_, i64>(0),
                        )
                    })
                    .unwrap_or(0)
                    .try_into()
                    .unwrap_or(0);
                today.saturating_add(cost) >= l
            });
        if over_task || over_day {
            if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task) {
                t.set_status(TaskStatus::Paused);
                t.set_step("spendLimit".into());
            }
            self.audit(
                task,
                "error",
                json!({ "spendLimit": true, "spentMicros": spent }),
            );
            return true;
        }
        false
    }

    /// Raises a paused task's limit by its current spend plus the setting.
    pub(super) fn raise_limit(&mut self, task: TaskId) {
        let step = self
            .setting("ai.spendLimits.perTask")
            .as_u64()
            .unwrap_or(1_000_000);
        let g = self.guard(task);
        g.limit_micros = Some(g.cost_micros + step);
    }

    /// The scope of a task, from its prompt (9.5).
    pub(super) fn set_scope(&mut self, task: TaskId, prompt: &str, mode: Option<Mode>) {
        let g = self.guard(task);
        g.scope = Some(mado_agent::safety::hosts_in(prompt));
        g.mode = mode;
    }
}
