//! AI connections in the core (spec 13 §2): adding, testing, OAuth,
//! local servers. Secrets go to the Keychain at once and never into the
//! state, the database or logs; the state keeps their last four characters.

use std::sync::Arc;

use std::collections::BTreeMap;

use mado_ai::catalog::{Available, RoleChoice};
use mado_ai::connection::{self, Api, Kind, Provider};
use mado_ai::models::RemoteModel;
use mado_ai::secrets::{self, Secrets};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::{AiConnection, AiConnectionStatus, AiRole};
use mado_types::{ErrorCode, MadoError};
use rusqlite::params;
use serde_json::{Value, json};

use super::{Core, Internal, Reply};

/// What the core needs for AI (spec 13); tests pass fakes and mocks.
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub secrets: Arc<dyn Secrets>,
    /// OpenRouter's site, for its sign-in page (13 §2.2).
    pub openrouter_site: String,
    /// OpenRouter's API, for trading the sign-in code for a key.
    pub openrouter_api: String,
    /// The local servers `ai.connection.detectLocal` probes.
    pub local: Vec<(Provider, String)>,
    /// Never ask Apple's on-device model (tests).
    pub skip_on_device: bool,
    /// Where subscription CLIs are looked for; `None` asks the login shell
    /// (13 §2.3). Tests pass a folder of fakes.
    pub cli_dirs: Option<Vec<std::path::PathBuf>>,
    /// The `mado` CLI, which CLI agents start as Mado's MCP server (13 §7.2).
    pub mado_cli: std::path::PathBuf,
}

impl AiConfig {
    /// The Keychain under the profile's service (12 §1.4) and the real
    /// addresses.
    pub fn system(profile: mado_types::ProfileId) -> Self {
        Self {
            secrets: Arc::new(Keychain {
                service: mado_platform::keychain::profile_service(profile),
            }),
            openrouter_site: "https://openrouter.ai".into(),
            openrouter_api: "https://openrouter.ai/api/v1".into(),
            local: mado_ai::local::defaults(),
            skip_on_device: false,
            cli_dirs: None,
            mado_cli: std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("mado")))
                .unwrap_or_else(|| "mado".into()),
        }
    }

    /// Secrets kept in memory, for private and guest profiles.
    pub fn in_memory() -> Self {
        Self {
            secrets: Arc::new(secrets::Memory::default()),
            ..Self::system(mado_types::ProfileId::new())
        }
    }
}

/// Every subscription CLI of `models.json` (13 §2.3), looked for in `dirs`
/// or the login shell's `PATH`. Blocks: runs `--version`s.
pub(super) fn detect_clis(dirs: Option<Vec<std::path::PathBuf>>) -> Vec<mado_agent::cli::Found> {
    let dirs = dirs.unwrap_or_else(mado_agent::cli::search_dirs);
    let models = mado_lists::models();
    models
        .cli
        .iter()
        .map(|(key, info)| {
            let name = Provider::from_key(key).map_or_else(|| key.clone(), |p| p.name().to_owned());
            mado_agent::cli::detect(key, &name, info, &dirs)
        })
        .collect()
}

/// The profile's Keychain service, account `ai.<connectionId>`.
#[derive(Debug)]
struct Keychain {
    service: String,
}

impl Secrets for Keychain {
    fn get(&self, connection: &str) -> Result<Option<String>, String> {
        mado_platform::keychain::get(&self.service, &secrets::account(connection))
            .map(|b| b.map(|b| String::from_utf8_lossy(&b).into_owned()))
            .map_err(|e| e.to_string())
    }

    fn set(&self, connection: &str, secret: &str) -> Result<(), String> {
        mado_platform::keychain::set(
            &self.service,
            &secrets::account(connection),
            secret.as_bytes(),
        )
        .map_err(|e| e.to_string())
    }

    fn delete(&self, connection: &str) -> Result<(), String> {
        mado_platform::keychain::delete(&self.service, &secrets::account(connection))
            .map_err(|e| e.to_string())
    }
}

/// Work the core spawned for AI, coming back.
#[derive(Debug)]
pub enum AiEvent {
    /// A chat job's progress (13 §5).
    Chat(super::chat::ChatEvent),
    /// A page command's progress (13 §6).
    Command(super::page_ai::CommandEvent),
    /// Whether Apple's on-device model can answer.
    OnDevice(bool),
    /// A connection test finished: its models, or the failure's key.
    Tested {
        id: String,
        result: Result<Vec<RemoteModel>, String>,
        reply: Option<Reply>,
    },
    /// OpenRouter's sign-in gave a key, or failed (`refused`,
    /// `timedOut`, `network`).
    Signed {
        id: String,
        result: Result<String, String>,
    },
}

/// One row of `ai_connections`' `config`: what the state shows besides
/// the columns. Never the secret.
fn config_json(c: &AiConnection) -> String {
    json!({ "baseUrl": c.base_url, "hint": c.secret_hint, "error": c.error }).to_string()
}

fn status_key(s: AiConnectionStatus) -> &'static str {
    match s {
        AiConnectionStatus::Ready => "ready",
        AiConnectionStatus::Error => "error",
        _ => "untested",
    }
}

/// The saved connections, with how many models each listed.
pub(super) fn load(c: &rusqlite::Connection) -> rusqlite::Result<Vec<AiConnection>> {
    let mut stmt = c.prepare(
        "SELECT id, kind, provider, label, priority, config, status,
                (SELECT COUNT(*) FROM ai_models m WHERE m.connection_id = ai_connections.id)
         FROM ai_connections ORDER BY priority, id",
    )?;
    stmt.query_map([], |r| {
        let config: Value = serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null);
        let text = |k: &str| config[k].as_str().map(str::to_owned);
        Ok(AiConnection {
            id: r.get(0)?,
            kind: r.get(1)?,
            provider: r.get(2)?,
            label: r.get(3)?,
            priority: r.get(4)?,
            status: match r.get::<_, String>(6)?.as_str() {
                "ready" => AiConnectionStatus::Ready,
                "error" => AiConnectionStatus::Error,
                _ => AiConnectionStatus::Untested,
            },
            error: text("error"),
            secret_hint: text("hint"),
            base_url: text("baseUrl"),
            models: r.get(7)?,
        })
    })?
    .collect()
}

/// One row of `ai_models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelRow {
    pub connection: String,
    pub model: String,
    pub name: Option<String>,
    pub show: bool,
    pub starred: bool,
}

/// The listed models and the role choices.
pub(super) fn load_models(
    c: &rusqlite::Connection,
) -> rusqlite::Result<(Vec<ModelRow>, BTreeMap<String, RoleChoice>)> {
    let mut stmt = c.prepare(
        "SELECT connection_id, model_id, info, show, starred FROM ai_models ORDER BY connection_id, model_id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let info: Value = serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null);
            Ok(ModelRow {
                connection: r.get(0)?,
                model: r.get(1)?,
                name: info["name"].as_str().map(str::to_owned),
                show: r.get(3)?,
                starred: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = c.prepare("SELECT role, model, effort, fallback FROM ai_roles")?;
    let roles = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                RoleChoice {
                    model: r
                        .get::<_, Option<String>>(1)?
                        .unwrap_or_else(|| "auto".into()),
                    effort: r
                        .get::<_, Option<String>>(2)?
                        .unwrap_or_else(|| "auto".into()),
                    fallback: r.get(3)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    Ok((rows, roles))
}

const EFFORTS: [&str; 6] = ["auto", "low", "medium", "high", "xhigh", "max"];

fn not_allowed() -> MadoError {
    MadoError::new(ErrorCode::Protected, "error.ai.notAllowed")
}

fn not_found() -> MadoError {
    MadoError::new(ErrorCode::NotFound, "error.ai.noConnection")
}

/// A failure key as the error a command answers.
fn failure_error(key: &str) -> MadoError {
    let code = match key {
        "network" | "timedOut" => ErrorCode::NetworkFailed,
        "unauthorized" | "refused" => ErrorCode::PermissionDenied,
        "notFound" => ErrorCode::NotFound,
        _ => ErrorCode::Unsupported,
    };
    MadoError::new(code, format!("error.ai.{key}"))
}

impl Core {
    /// Before the first revision: the saved connections.
    pub(super) fn load_ai(&mut self) {
        if self.state.profile.is_private {
            return;
        }
        let rows = self.profile_db.readers().read(load).unwrap_or_else(|e| {
            tracing::error!(error = %e, "the AI connections could not be read");
            Vec::new()
        });
        self.state.ai.connections = rows.into_iter().map(|c| (c.id.clone(), c)).collect();
        let (models, roles) = self
            .profile_db
            .readers()
            .read(load_models)
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "the AI models could not be read");
                Default::default()
            });
        self.ai_models = models;
        self.ai_roles = roles;
        let on = self.page_commands_on();
        self.ui.ai_menu(on);
        self.probe_on_device();
        // The first retention run soon after start, then daily (13 §5).
        self.timers.schedule(
            crate::Timer::AiRetention,
            std::time::Duration::from_secs(5 * 60),
        );
        // The first filing an hour after start, then daily (13 §10).
        self.timers.schedule(
            crate::Timer::MemoryFiling,
            std::time::Duration::from_secs(3600),
        );
        self.state.ai.roles = self.resolved_roles();
    }

    /// Asks Apple's on-device helper whether its model is there, only with
    /// AI on (13 §13 check 1: no AI process with AI off).
    pub(super) fn probe_on_device(&self) {
        if self.ai.skip_on_device || self.setting("features.ai") != true {
            return;
        }
        let inputs = self.inputs.clone();
        std::thread::spawn(move || {
            let ready =
                mado_platform::ondevice::status() == mado_platform::ondevice::Status::Available;
            let _ = inputs.send(super::Input::Internal(Internal::Ai(AiEvent::OnDevice(
                ready,
            ))));
        });
    }

    /// Every model of a connection that exists, with its connection's
    /// readiness and priority.
    pub(super) fn ai_available(&self) -> Vec<Available> {
        self.ai_models
            .iter()
            .filter_map(|m| {
                let c = self.state.ai.connections.get(&m.connection)?;
                Some(Available {
                    connection: m.connection.clone(),
                    provider: c.provider.clone(),
                    model: m.model.clone(),
                    name: m.name.clone(),
                    show: m.show,
                    starred: m.starred,
                    ready: c.status == AiConnectionStatus::Ready,
                    priority: c.priority,
                })
            })
            .collect()
    }

    fn resolved_roles(&self) -> BTreeMap<String, AiRole> {
        let catalog = mado_lists::models();
        let available = self.ai_available();
        mado_ai::catalog::resolve_all(&catalog, &available, &self.ai_roles)
            .into_iter()
            .map(|(role, resolved)| {
                let c = self
                    .ai_roles
                    .get(&role)
                    .cloned()
                    .unwrap_or_else(RoleChoice::auto);
                (
                    role,
                    AiRole {
                        model: c.model,
                        effort: c.effort,
                        fallback: c.fallback,
                        resolved,
                    },
                )
            })
            .collect()
    }

    /// Puts each role's current model in the state (13 §4) after a
    /// connection, model or role changed.
    pub(super) fn refresh_roles(&mut self, rec: &mut PatchRecorder) {
        let next = self.resolved_roles();
        for (role, value) in next {
            if self.state.ai.roles.get(&role) != Some(&value) {
                self.state.tx(rec).ai().roles().insert(role, value);
            }
        }
    }

    fn connection(&self, id: &str) -> Result<AiConnection, MadoError> {
        self.state
            .ai
            .connections
            .get(id)
            .cloned()
            .ok_or_else(not_found)
    }

    /// Writes a connection's row; `None` deletes it with its models.
    fn persist_connection(&self, id: String, row: Option<AiConnection>, now: i64) {
        if self.state.profile.is_private {
            return;
        }
        let pending = self.profile_db.writer().write(move |c| {
            match row {
                Some(r) => c.execute(
                    "INSERT INTO ai_connections (id, kind, provider, label, priority, config, secret_ref, status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT (id) DO UPDATE SET label = excluded.label, priority = excluded.priority,
                       config = excluded.config, secret_ref = excluded.secret_ref, status = excluded.status",
                    params![
                        r.id,
                        r.kind,
                        r.provider,
                        r.label,
                        r.priority,
                        config_json(&r),
                        r.secret_hint.as_ref().map(|_| secrets::account(&r.id)),
                        status_key(r.status),
                        now
                    ],
                )?,
                None => {
                    c.execute("DELETE FROM ai_models WHERE connection_id = ?1", [&id])?;
                    c.execute("DELETE FROM ai_connections WHERE id = ?1", [&id])?
                }
            };
            Ok(())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an AI connection was not saved");
            }
        });
    }

    fn put_connection(&mut self, conn: AiConnection, rec: &mut PatchRecorder) {
        let now = self.now();
        self.persist_connection(conn.id.clone(), Some(conn.clone()), now);
        self.state
            .tx(rec)
            .ai()
            .connections()
            .insert(conn.id.clone(), conn);
    }

    pub(super) fn ai_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        // 13: with `features.ai` off none of this runs, except answering what
        // control clients' tools raise: they work with AI off (14 §1), so
        // their approval cards and agent tabs must still be answerable.
        if self.setting("features.ai") != true
            && !matches!(
                cmd,
                Command::AiApprovalRespond { .. } | Command::AiTabHandBack { .. }
            )
        {
            return Err(MadoError::new(ErrorCode::Unsupported, "error.ai.off"));
        }
        if cmd.name().starts_with("ai.routine.") {
            return self.routine_command(cmd, rec);
        }
        if cmd.name().starts_with("ai.memory.") || cmd.name().starts_with("ai.skill.") {
            return self.memory_command(origin, cmd, reply_to, rec);
        }
        if matches!(
            cmd,
            Command::AiChatSend { .. }
                | Command::AiChatList
                | Command::AiChatGet { .. }
                | Command::AiChatDelete { .. }
                | Command::AiChatCancel { .. }
        ) {
            return self.chat_command(origin, cmd, reply_to);
        }
        if cmd.name().starts_with("ai.task.") && !matches!(cmd, Command::AiTaskRaiseLimit { .. }) {
            return self.task_command(origin, cmd, reply_to, rec);
        }
        if let Command::AiTaskRaiseLimit { task_id } = cmd {
            if !origin.is_user() {
                return Err(not_allowed());
            }
            self.raise_limit(task_id);
            if let Some(mut t) = self.state.tx(rec).ai().tasks().get(&task_id) {
                t.set_status(mado_types::state::TaskStatus::Running);
                t.set_step(String::new());
            }
            return Ok(Value::Null);
        }
        if matches!(cmd, Command::AiApprovalRespond { .. }) {
            return self.approval_respond(origin, cmd, rec);
        }
        if let Command::AiTabHandBack { tab_id } = cmd {
            if !origin.is_user() {
                return Err(not_allowed());
            }
            self.handed_back.insert(tab_id);
            return Ok(Value::Null);
        }
        if matches!(
            cmd,
            Command::AiCommandList
                | Command::AiCommandRun { .. }
                | Command::AiCommandRespond { .. }
        ) {
            return self.ai_page_command(origin, cmd, rec);
        }
        match cmd {
            Command::AiConnectionAdd {
                kind,
                provider,
                label,
                base_url,
                secret,
            } => {
                // Connections decide where page text goes: Mado's windows
                // only (and the CLI with full control, 14 §2.1).
                if !origin.is_user() {
                    return Err(not_allowed());
                }
                self.add_connection(&kind, &provider, label, base_url, secret, rec)
            }
            Command::AiConnectionUpdate {
                id,
                label,
                priority,
                secret,
            } => {
                if !origin.is_user() {
                    return Err(not_allowed());
                }
                let mut conn = self.connection(&id)?;
                if let Some(label) = label.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()) {
                    conn.label = label;
                }
                if let Some(priority) = priority {
                    conn.priority = priority;
                }
                let secret = secret
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty());
                if let Some(s) = &secret {
                    conn.secret_hint = Some(secrets::hint(s));
                    conn.status = AiConnectionStatus::Testing;
                    conn.error = None;
                }
                self.put_connection(conn, rec);
                self.refresh_roles(rec);
                if secret.is_some() {
                    self.test_connection(&id, secret, None);
                }
                Ok(Value::Null)
            }
            Command::AiConnectionRemove { id } => {
                if !origin.is_user() {
                    return Err(not_allowed());
                }
                self.connection(&id)?;
                self.state.tx(rec).ai().connections().remove(&id);
                self.persist_connection(id.clone(), None, self.now());
                self.ai_models.retain(|m| m.connection != id);
                self.refresh_roles(rec);
                let store = Arc::clone(&self.ai.secrets);
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = store.delete(&id) {
                        tracing::warn!(error = %e, "an AI secret was not deleted");
                    }
                });
                Ok(Value::Null)
            }
            Command::AiConnectionTest { id } => {
                let mut conn = self.connection(&id)?;
                conn.status = AiConnectionStatus::Testing;
                self.state
                    .tx(rec)
                    .ai()
                    .connections()
                    .insert(id.clone(), conn);
                self.test_connection(&id, None, reply_to.take());
                Ok(Value::Null)
            }
            Command::AiConnectionDetectLocal => {
                let Some(reply) = reply_to.take() else {
                    return Ok(Value::Null);
                };
                let taken: Vec<String> = self
                    .state
                    .ai
                    .connections
                    .values()
                    .filter_map(|c| {
                        c.base_url.clone().or_else(|| {
                            Provider::from_key(&c.provider)
                                .and_then(Provider::default_base)
                                .map(str::to_owned)
                        })
                    })
                    .collect();
                let candidates = self.ai.local.clone();
                tokio::task::spawn_local(async move {
                    let Ok(client) = mado_ai::http::client() else {
                        let _ = reply.send(Ok(json!([])));
                        return;
                    };
                    let found = mado_ai::local::detect(&client, &candidates).await;
                    let list: Vec<Value> = found
                        .into_iter()
                        .filter(|f| !taken.contains(&f.base_url))
                        .map(|f| json!({ "provider": f.provider.key(), "baseUrl": f.base_url, "models": f.models }))
                        .collect();
                    let _ = reply.send(Ok(Value::Array(list)));
                });
                Ok(Value::Null)
            }
            Command::AiCliList => {
                let Some(reply) = reply_to.take() else {
                    return Ok(Value::Null);
                };
                let dirs = self.ai.cli_dirs.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = reply.send(Ok(json!(
                        detect_clis(dirs)
                            .into_iter()
                            .map(|f| {
                                let ready = f.ready();
                                let mut v = json!(f);
                                v["ready"] = json!(ready);
                                v
                            })
                            .collect::<Vec<_>>()
                    )));
                });
                Ok(Value::Null)
            }
            Command::AiCliRun { provider, what } => {
                if !origin.is_user() {
                    return Err(not_allowed());
                }
                let info = mado_lists::models()
                    .cli
                    .get(&provider)
                    .cloned()
                    .ok_or_else(|| {
                        MadoError::new(ErrorCode::InvalidInput, "error.ai.unknownProvider")
                    })?;
                let command = match what.as_str() {
                    "install" => info.install,
                    "login" => info.login,
                    _ => {
                        return Err(MadoError::new(
                            ErrorCode::InvalidInput,
                            "error.control.badCommand",
                        ));
                    }
                };
                let dirs = self.ai.cli_dirs.clone();
                let tmp = std::env::temp_dir();
                let reply = reply_to.take();
                tokio::task::spawn_blocking(move || {
                    let dirs = dirs.unwrap_or_else(mado_agent::cli::search_dirs);
                    let r = mado_agent::cli::open_in_terminal(&command, &tmp, &dirs)
                        .map(|()| Value::Null)
                        .map_err(|e| {
                            MadoError::new(ErrorCode::Internal, "error.ai.terminal")
                                .arg("reason", e.to_string())
                        });
                    if let Some(reply) = reply {
                        let _ = reply.send(r);
                    }
                });
                Ok(Value::Null)
            }
            Command::AiModelList => Ok(serde_json::to_value(mado_ai::catalog::picker(
                &mado_lists::models(),
                &self.ai_available(),
            ))
            .unwrap_or_default()),
            Command::AiModelSet {
                connection_id,
                model_id,
                show,
                starred,
            } => {
                if !origin.is_user() {
                    return Err(not_allowed());
                }
                let row = self
                    .ai_models
                    .iter_mut()
                    .find(|m| m.connection == connection_id && m.model == model_id)
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.ai.noModel"))?;
                if let Some(v) = show {
                    row.show = v;
                }
                if let Some(v) = starred {
                    row.starred = v;
                }
                let (show, starred) = (row.show, row.starred);
                if !self.state.profile.is_private {
                    let pending = self.profile_db.writer().write(move |c| {
                        c.execute(
                            "UPDATE ai_models SET show = ?3, starred = ?4 WHERE connection_id = ?1 AND model_id = ?2",
                            params![connection_id, model_id, show, starred],
                        )
                        .map(drop)
                    });
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "a model choice was not saved");
                        }
                    });
                }
                self.refresh_roles(rec);
                Ok(Value::Null)
            }
            Command::AiRoleSet {
                role,
                model,
                effort,
                fallback,
            } => {
                if !origin.is_user() {
                    return Err(not_allowed());
                }
                let invalid = |k: &str| MadoError::new(ErrorCode::InvalidInput, k.to_owned());
                if !mado_lists::ROLES.contains(&role.as_str()) {
                    return Err(invalid("error.ai.unknownRole"));
                }
                if !EFFORTS.contains(&effort.as_str()) {
                    return Err(invalid("error.ai.unknownEffort"));
                }
                let exists = |r: &str| {
                    r.split_once(':').is_some_and(|(c, m)| {
                        self.ai_models
                            .iter()
                            .any(|x| x.connection == c && x.model == m)
                    })
                };
                if model != "auto" && !exists(&model) {
                    return Err(invalid("error.ai.noModel"));
                }
                let fallback = fallback.filter(|f| !f.is_empty() && f != "auto");
                if fallback.as_deref().is_some_and(|f| !exists(f)) {
                    return Err(invalid("error.ai.noModel"));
                }
                let choice = RoleChoice {
                    model,
                    effort,
                    fallback,
                };
                if !self.state.profile.is_private {
                    let (r, c) = (role.clone(), choice.clone());
                    let pending = self.profile_db.writer().write(move |db| {
                        db.execute(
                            "INSERT INTO ai_roles (role, model, effort, fallback) VALUES (?1, ?2, ?3, ?4)
                             ON CONFLICT (role) DO UPDATE SET model = excluded.model, effort = excluded.effort, fallback = excluded.fallback",
                            params![r, c.model, c.effort, c.fallback],
                        )
                        .map(drop)
                    });
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "a role was not saved");
                        }
                    });
                }
                self.ai_roles.insert(role, choice);
                self.refresh_roles(rec);
                Ok(Value::Null)
            }
            _ => Err(MadoError::new(
                ErrorCode::Unsupported,
                "error.unknownCommand",
            )),
        }
    }

    fn add_connection(
        &mut self,
        kind: &str,
        provider: &str,
        label: Option<String>,
        base_url: Option<String>,
        secret: Option<String>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let invalid = |key: &str| MadoError::new(ErrorCode::InvalidInput, key.to_owned());
        let kind = Kind::from_key(kind).ok_or_else(|| invalid("error.ai.unknownKind"))?;
        let provider =
            Provider::from_key(provider).ok_or_else(|| invalid("error.ai.unknownProvider"))?;
        let secret = secret
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let mut base_url = base_url
            .map(|b| b.trim().to_owned())
            .filter(|b| !b.is_empty());
        // The key OpenRouter's sign-in gives works on the API it came from.
        if kind == Kind::Oauth && provider.default_base() != Some(self.ai.openrouter_api.as_str()) {
            base_url = Some(self.ai.openrouter_api.clone());
        }
        connection::validate(provider, kind, base_url.as_deref(), secret.is_some())
            .map_err(|e| invalid(e.message_key()))?;
        let id = uuid::Uuid::now_v7().simple().to_string();
        let same = self
            .state
            .ai
            .connections
            .values()
            .filter(|c| c.provider == provider.key())
            .count();
        let label = label
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| match same {
                0 => provider.name().to_owned(),
                n => format!("{} {}", provider.name(), n + 1),
            });
        let priority = self
            .state
            .ai
            .connections
            .values()
            .map(|c| c.priority + 1)
            .max()
            .unwrap_or(0);
        let conn = AiConnection {
            id: id.clone(),
            kind: kind.key().into(),
            provider: provider.key().into(),
            label,
            priority,
            status: if kind == Kind::Oauth {
                AiConnectionStatus::Connecting
            } else {
                AiConnectionStatus::Testing
            },
            error: None,
            secret_hint: secret.as_deref().map(secrets::hint),
            base_url,
            models: 0,
        };
        self.put_connection(conn, rec);
        if kind == Kind::Oauth {
            self.start_oauth(&id)?;
        } else {
            self.test_connection(&id, secret, None);
        }
        Ok(json!({ "id": id }))
    }

    /// 13 §2.2: a loopback server on a random port, the sign-in page in a
    /// new tab, and the key from the code.
    fn start_oauth(&mut self, id: &str) -> Result<(), MadoError> {
        let pkce = mado_ai::oauth::Pkce::new().map_err(|_| MadoError::internal())?;
        let listener =
            std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|_| MadoError::internal())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| MadoError::internal())?;
        let port = listener
            .local_addr()
            .map_err(|_| MadoError::internal())?
            .port();
        let url = mado_ai::oauth::auth_url(&self.ai.openrouter_site, port, &pkce.challenge);
        match self.current_window() {
            Some(window) => {
                let (now, prefs) = (self.now(), self.prefs());
                self.model
                    .open_tab(window, url, None, true, None, now, prefs)?;
            }
            None => {
                self.open_window(mado_types::state::Bounds::default(), &[url])?;
            }
        }
        let (inputs, id, api) = (
            self.inputs.clone(),
            id.to_owned(),
            self.ai.openrouter_api.clone(),
        );
        tokio::task::spawn_local(async move {
            use mado_ai::oauth::{OauthError, callback, exchange};
            let result = async {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                let code = callback(listener, mado_ai::oauth::WAIT).await?;
                let client = mado_ai::http::client()?;
                exchange(&client, &api, &code, &pkce.verifier).await
            }
            .await
            .map_err(|e: OauthError| {
                tracing::info!(error = %e, "the OpenRouter sign-in did not finish");
                match e {
                    OauthError::TimedOut => "timedOut",
                    OauthError::Refused | OauthError::Status(_) => "refused",
                    OauthError::Io(_) | OauthError::Http(_) => "network",
                }
                .to_owned()
            });
            let _ = inputs.send(super::Input::Internal(Internal::Ai(AiEvent::Signed {
                id,
                result,
            })));
        });
        Ok(())
    }

    /// Lists the connection's models; `store` is a new secret to save
    /// first. The answer comes back as `AiEvent::Tested`.
    fn test_connection(&self, id: &str, store: Option<String>, reply: Option<Reply>) {
        let Some(conn) = self.state.ai.connections.get(id) else {
            return;
        };
        let provider = Provider::from_key(&conn.provider);
        if provider.is_some_and(|p| p.kinds() == [Kind::SubscriptionCli]) {
            // 13 §2.3: installed, and its ACP command can start. It lists no
            // models: tasks pick it by name.
            let (inputs, id, dirs) = (self.inputs.clone(), id.to_owned(), self.ai.cli_dirs.clone());
            let key = conn.provider.clone();
            tokio::task::spawn_blocking(move || {
                let found = detect_clis(dirs).into_iter().find(|f| f.provider == key);
                let result = match found {
                    Some(f) if f.ready() => Ok(Vec::new()),
                    Some(f) if f.path.is_some() && f.needs_node && !f.node_ok => {
                        Err("needsNode".to_owned())
                    }
                    _ => Err("notInstalled".to_owned()),
                };
                let _ = inputs.send(super::Input::Internal(Internal::Ai(AiEvent::Tested {
                    id,
                    result,
                    reply,
                })));
            });
            return;
        }
        if provider == Some(Provider::Apple) {
            // Apple's on-device model has one model and no key.
            let (inputs, id, skip) = (self.inputs.clone(), id.to_owned(), self.ai.skip_on_device);
            std::thread::spawn(move || {
                let ready = !skip
                    && mado_platform::ondevice::status()
                        == mado_platform::ondevice::Status::Available;
                let result = if ready {
                    Ok(vec![RemoteModel {
                        id: "apple-on-device".into(),
                        name: Some("Apple Intelligence".into()),
                        context_tokens: Some(4096),
                    }])
                } else {
                    Err("unavailable".to_owned())
                };
                let _ = inputs.send(super::Input::Internal(Internal::Ai(AiEvent::Tested {
                    id,
                    result,
                    reply,
                })));
            });
            return;
        }
        let api = provider.map_or(Api::None, Provider::api);
        let base = provider
            .ok_or(connection::ConnectionError::BadUrl)
            .and_then(|p| connection::base_url(p, conn.base_url.as_deref()));
        let (inputs, id, secrets) = (
            self.inputs.clone(),
            id.to_owned(),
            Arc::clone(&self.ai.secrets),
        );
        tokio::task::spawn_local(async move {
            let result = async {
                let base = base.map_err(|_| "invalid".to_owned())?;
                let key = id.clone();
                let secret = tokio::task::spawn_blocking(move || {
                    if let Some(s) = &store {
                        secrets.set(&key, s)?;
                    }
                    secrets.get(&key)
                })
                .await
                .map_err(|_| "keychain".to_owned())?
                .map_err(|e| {
                    tracing::warn!(error = %e, "an AI secret could not be read or saved");
                    "keychain".to_owned()
                })?;
                let client = mado_ai::http::client().map_err(|_| "network".to_owned())?;
                mado_ai::models::list(&client, api, &base, secret.as_deref())
                    .await
                    .map_err(|f| f.key().to_owned())
            }
            .await;
            let _ = inputs.send(super::Input::Internal(Internal::Ai(AiEvent::Tested {
                id,
                result,
                reply,
            })));
        });
    }

    pub(super) fn ai_event(&mut self, event: AiEvent, rec: &mut PatchRecorder) {
        match event {
            AiEvent::Chat(e) => self.chat_event(e),
            AiEvent::Command(e) => self.command_event(e, rec),
            AiEvent::OnDevice(ready) => self.ondevice_ready = ready,
            AiEvent::Tested { id, result, reply } => {
                let Some(mut conn) = self.state.ai.connections.get(&id).cloned() else {
                    if let Some(r) = reply {
                        let _ = r.send(Err(not_found()));
                    }
                    return;
                };
                let answer = match &result {
                    Ok(models) => {
                        conn.status = AiConnectionStatus::Ready;
                        conn.error = None;
                        conn.models = u32::try_from(models.len()).unwrap_or(u32::MAX);
                        Ok(json!({ "models": models.len() }))
                    }
                    Err(key) => {
                        conn.status = AiConnectionStatus::Error;
                        conn.error = Some(key.clone());
                        Err(failure_error(key))
                    }
                };
                if let Ok(models) = result {
                    let mut rows: Vec<ModelRow> = models
                        .iter()
                        .map(|m| {
                            let old = self
                                .ai_models
                                .iter()
                                .find(|x| x.connection == id && x.model == m.id);
                            ModelRow {
                                connection: id.clone(),
                                model: m.id.clone(),
                                name: m.name.clone(),
                                show: old.is_none_or(|o| o.show),
                                starred: old.is_some_and(|o| o.starred),
                            }
                        })
                        .collect();
                    self.ai_models.retain(|x| x.connection != id);
                    self.ai_models.append(&mut rows);
                    self.save_models(&id, models);
                }
                self.put_connection(conn, rec);
                self.refresh_roles(rec);
                if let Some(r) = reply {
                    let _ = r.send(answer);
                }
            }
            AiEvent::Signed { id, result } => {
                let Some(mut conn) = self.state.ai.connections.get(&id).cloned() else {
                    return;
                };
                match result {
                    Ok(key) => {
                        conn.secret_hint = Some(secrets::hint(&key));
                        conn.status = AiConnectionStatus::Testing;
                        conn.error = None;
                        self.put_connection(conn, rec);
                        self.test_connection(&id, Some(key), None);
                    }
                    Err(e) => {
                        conn.status = AiConnectionStatus::Error;
                        conn.error = Some(e);
                        self.put_connection(conn, rec);
                        self.refresh_roles(rec);
                    }
                }
            }
        }
    }

    /// Keeps the listed models in `ai_models` (13 §4), with their show and
    /// star choices; models no longer listed go.
    fn save_models(&self, id: &str, models: Vec<RemoteModel>) {
        if self.state.profile.is_private {
            return;
        }
        let id = id.to_owned();
        let pending = self.profile_db.writer().write(move |c| {
            {
                let mut put = c.prepare(
                    "INSERT INTO ai_models (connection_id, model_id, info) VALUES (?1, ?2, ?3)
                     ON CONFLICT (connection_id, model_id) DO UPDATE SET info = excluded.info",
                )?;
                for m in &models {
                    let info = serde_json::to_string(m).unwrap_or_default();
                    put.execute(params![id, m.id, info])?;
                }
            }
            let listed: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
            let listed = serde_json::to_string(&listed).unwrap_or_else(|_| "[]".into());
            c.execute(
                "DELETE FROM ai_models WHERE connection_id = ?1
                   AND model_id NOT IN (SELECT value FROM json_each(?2))",
                params![id, listed],
            )?;
            Ok(())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "the AI models were not saved");
            }
        });
    }
}
