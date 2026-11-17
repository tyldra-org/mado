//! Control clients in the core (spec 14 §2): pairing, tokens, scopes,
//! revoking and full control. The endpoint in `mado-control` carries the
//! messages; everything it lets through is decided here.

use std::collections::BTreeMap;

use data_encoding::BASE64URL_NOPAD;
use mado_control::{ClientInfo, HelloAnswer, PairingDecision};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::{Notice, PromptState};
use mado_types::{ClientId, ErrorCode, MadoError, PromptId};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use super::{Core, Reply};

/// A pairing card waits this long (spec 14 §2).
pub(super) const PAIRING_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// The scopes a pairing card offers.
pub const SCOPES: [&str; 5] = ["read", "act", "tasks", "settings", "full"];

/// A paired client (`control_clients`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Client {
    pub id: ClientId,
    pub name: String,
    pub token_hash: Vec<u8>,
    pub scopes: Vec<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked: bool,
}

/// A pairing card on screen.
#[derive(Debug)]
pub(super) struct Pairing {
    pub name: String,
    pub offer_full: bool,
    pub decision: oneshot::Sender<PairingDecision>,
}

pub(super) fn load(c: &rusqlite::Connection) -> rusqlite::Result<BTreeMap<ClientId, Client>> {
    let mut stmt = c.prepare(
        "SELECT id, name, token_hash, scopes, created_at, last_used_at, revoked FROM control_clients",
    )?;
    let rows = stmt.query_map([], |r| {
        let id: String = r.get(0)?;
        let scopes: String = r.get(3)?;
        Ok((
            id,
            r.get::<_, String>(1)?,
            r.get::<_, Vec<u8>>(2)?,
            scopes,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, bool>(6)?,
        ))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (id, name, token_hash, scopes, created_at, last_used_at, revoked) = row?;
        let Ok(id) = id.parse::<ClientId>() else {
            continue;
        };
        out.insert(
            id,
            Client {
                id,
                name,
                token_hash,
                scopes: serde_json::from_str(&scopes).unwrap_or_default(),
                created_at,
                last_used_at,
                revoked,
            },
        );
    }
    Ok(out)
}

fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn err(code: ErrorCode, key: &str) -> MadoError {
    MadoError::new(code, key.to_owned())
}

/// Calls that reach secrets or macOS's own prompts: the person answers
/// those on the Mac, never a client (spec 14 §2.1).
fn needs_person(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::VaultGet { .. }
            | Command::VaultExport { .. }
            | Command::VaultExportCsv { .. }
            | Command::VaultDeleteForever { .. }
            | Command::VaultFill { .. }
            | Command::AppAuthenticate { .. }
    )
}

impl Core {
    pub(super) fn load_control(&mut self) {
        if self.state.profile.is_private {
            return;
        }
        self.control_clients = self.profile_db.readers().read(load).unwrap_or_else(|e| {
            tracing::error!(error = %e, "the control clients could not be read");
            BTreeMap::new()
        });
    }

    fn control_on(&self) -> bool {
        self.setting("developer.controlEnabled") == true
    }

    /// Whether `client`'s session has full control now: the scope, the
    /// switch, and only the `mado` CLI (14 §2.1).
    fn full(&self, client: &Client) -> bool {
        client.scopes.iter().any(|s| s == "full")
            && client.name == "mado"
            && self.setting("developer.cliFullControl") == true
    }

    fn session_scopes(&self, client: &Client) -> Vec<String> {
        let full = self.full(client);
        client
            .scopes
            .iter()
            .filter(|s| *s != "full" || full)
            .cloned()
            .collect()
    }

    /// `hello` (14 §2): a valid token opens a session; no token opens a
    /// pairing card.
    pub(super) fn control_hello(
        &mut self,
        info: ClientInfo,
        token: Option<String>,
        reply: oneshot::Sender<HelloAnswer>,
        rec: &mut PatchRecorder,
    ) {
        if !self.control_on() {
            let _ = reply.send(HelloAnswer::Refused(err(
                ErrorCode::PermissionDenied,
                "error.control.disabled",
            )));
            return;
        }
        if let Some(token) = token {
            let h = hash(&token);
            let now = self.now();
            let found = self
                .control_clients
                .values_mut()
                .find(|c| !c.revoked && c.token_hash == h)
                .map(|c| {
                    c.last_used_at = Some(now);
                    c.clone()
                });
            let answer = match found {
                Some(c) => {
                    self.touch_client(c.id, now);
                    HelloAnswer::Session {
                        client: c.id,
                        scopes: self.session_scopes(&c),
                    }
                }
                None => HelloAnswer::Refused(err(
                    ErrorCode::PermissionDenied,
                    "error.control.unauthorized",
                )),
            };
            let _ = reply.send(answer);
            return;
        }
        // At most one pending pairing per client name.
        if self.pairings.values().any(|p| p.name == info.name) {
            let _ = reply.send(HelloAnswer::Refused(err(
                ErrorCode::Conflict,
                "error.control.pairingPending",
            )));
            return;
        }
        let id = PromptId::new();
        let offer_full = info.name == "mado" && self.setting("developer.cliFullControl") == true;
        let (tx, rx) = oneshot::channel();
        self.pairings.insert(
            id,
            Pairing {
                name: info.name.clone(),
                offer_full,
                decision: tx,
            },
        );
        let mut prompts = self.state.prompts.clone();
        prompts.insert(
            id,
            PromptState::ControlPairing {
                id,
                client_name: info.name,
                client_version: info.version,
                offer_full,
            },
        );
        self.state.tx(rec).set_prompts(prompts);
        self.timers
            .schedule(crate::Timer::PairingTimeout(id), PAIRING_WAIT);
        let _ = reply.send(HelloAnswer::Pairing {
            pairing_id: id.to_string(),
            decision: rx,
        });
    }

    fn touch_client(&self, id: ClientId, now: i64) {
        if self.state.profile.is_private {
            return;
        }
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "UPDATE control_clients SET last_used_at = ?2 WHERE id = ?1",
                params![id.to_string(), now],
            )
            .map(drop)
        });
        tokio::task::spawn_local(async move {
            let _ = pending.await;
        });
    }

    fn end_pairing(&mut self, id: PromptId, rec: &mut PatchRecorder) -> Option<Pairing> {
        let mut prompts = self.state.prompts.clone();
        if prompts.remove(&id).is_some() {
            self.state.tx(rec).set_prompts(prompts);
        }
        self.pairings.remove(&id)
    }

    /// Two minutes without an answer: denied.
    pub(super) fn pairing_timed_out(&mut self, id: PromptId, rec: &mut PatchRecorder) {
        if let Some(p) = self.end_pairing(id, rec) {
            let _ = p.decision.send(PairingDecision::Denied);
        }
    }

    pub(super) fn control_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        // Only Mado's own windows manage clients.
        if !matches!(
            origin,
            Origin::Settings | Origin::Ui { .. } | Origin::Overlay { .. } | Origin::System
        ) {
            return Err(err(ErrorCode::Protected, "error.control.notAllowed"));
        }
        match cmd {
            Command::ControlClients => Ok(json!(
                self.control_clients
                    .values()
                    .map(|c| json!({
                        "id": c.id,
                        "name": c.name,
                        "scopes": c.scopes,
                        "createdAt": c.created_at,
                        "lastUsedAt": c.last_used_at,
                        "revoked": c.revoked,
                    }))
                    .collect::<Vec<_>>()
            )),
            Command::ControlRevoke { client_id } => {
                let c = self
                    .control_clients
                    .get_mut(&client_id)
                    .ok_or_else(|| err(ErrorCode::NotFound, "error.control.noClient"))?;
                c.revoked = true;
                let pending = self.profile_db.writer().write(move |db| {
                    db.execute(
                        "UPDATE control_clients SET revoked = 1 WHERE id = ?1",
                        [client_id.to_string()],
                    )
                    .map(drop)
                });
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "a revocation was not saved");
                    }
                });
                Ok(Value::Null)
            }
            Command::ControlPairRespond {
                prompt_id,
                allow,
                scopes,
            } => {
                let pairing = self
                    .end_pairing(prompt_id, rec)
                    .ok_or_else(|| err(ErrorCode::NotFound, "error.prompt.notFound"))?;
                if !allow {
                    let _ = pairing.decision.send(PairingDecision::Denied);
                    return Ok(Value::Null);
                }
                let mut granted: Vec<String> = SCOPES
                    .iter()
                    .filter(|s| scopes.iter().any(|x| x == *s))
                    .filter(|s| **s != "full" || pairing.offer_full)
                    .map(|s| (*s).to_owned())
                    .collect();
                if granted.is_empty() {
                    granted.push("read".into());
                }
                let mut bytes = [0u8; 32];
                getrandom::fill(&mut bytes).map_err(|_| MadoError::internal())?;
                let token = BASE64URL_NOPAD.encode(&bytes);
                let now = self.now();
                let client = Client {
                    id: ClientId::new(),
                    name: pairing.name,
                    token_hash: hash(&token),
                    scopes: granted.clone(),
                    created_at: now,
                    last_used_at: Some(now),
                    revoked: false,
                };
                let row = client.clone();
                if !self.state.profile.is_private {
                    let pending = self.profile_db.writer().write(move |db| {
                        db.execute(
                            "INSERT INTO control_clients (id, name, token_hash, scopes, created_at, last_used_at, revoked) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                            params![
                                row.id.to_string(),
                                row.name,
                                row.token_hash,
                                serde_json::to_string(&row.scopes).unwrap_or_default(),
                                row.created_at,
                                row.last_used_at
                            ],
                        )
                        .map(drop)
                    });
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "a control client was not saved");
                        }
                    });
                }
                let id = client.id;
                let scopes = self.session_scopes(&client);
                self.control_clients.insert(id, client);
                let _ = pairing.decision.send(PairingDecision::Approved {
                    client: id,
                    token,
                    scopes,
                });
                Ok(json!({ "clientId": id }))
            }
            _ => Err(err(ErrorCode::Unsupported, "error.unknownCommand")),
        }
    }

    /// A paired client's call (14 §2): checks the switch, the token's
    /// client and its scopes, then runs the command as that client.
    pub(super) fn control_call(
        &mut self,
        client: ClientId,
        method: String,
        params: Value,
        reply: Reply,
        rec: &mut PatchRecorder,
    ) {
        if let Some(name) = method.strip_prefix("tool.") {
            match self.tool_check(client, name) {
                Ok(origin) => self.tool_call(origin, name.to_owned(), params, reply),
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            }
            return;
        }
        let result = self.control_check(client, &method, params);
        let (origin, cmd) = match result {
            Ok(x) => x,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        if let Origin::FullControl { .. } = origin {
            self.audit_full(client, &method, &cmd, rec);
        }
        let _ = self
            .inputs
            .send(super::Input::Command { origin, cmd, reply });
    }

    fn control_check(
        &mut self,
        client: ClientId,
        method: &str,
        params: Value,
    ) -> Result<(Origin, Command), MadoError> {
        if !self.control_on() {
            return Err(err(ErrorCode::PermissionDenied, "error.control.disabled"));
        }
        let c = self
            .control_clients
            .get(&client)
            .filter(|c| !c.revoked)
            .cloned()
            .ok_or_else(|| err(ErrorCode::PermissionDenied, "error.control.unauthorized"))?;
        if method.starts_with("tool.") || method.starts_with("control.") || method == "task.events"
        {
            return Err(err(ErrorCode::NotFound, "error.control.unknownMethod"));
        }
        let mut object = match params {
            Value::Object(o) => o,
            Value::Null => serde_json::Map::new(),
            _ => return Err(err(ErrorCode::InvalidInput, "error.control.badCommand")),
        };
        object.insert("type".into(), json!(method));
        let cmd: Command = serde_json::from_value(Value::Object(object)).map_err(|e| {
            err(ErrorCode::InvalidInput, "error.control.badCommand").arg("reason", e.to_string())
        })?;
        let full = self.full(&c);
        if !full && !c.scopes.iter().any(|s| s == cmd.control_scope()) {
            return Err(
                err(ErrorCode::Protected, "error.control.scope").arg("scope", cmd.control_scope())
            );
        }
        if full && needs_person(&cmd) {
            return Err(err(ErrorCode::PermissionDenied, "error.control.needsUser"));
        }
        let now = self.now();
        if let Some(x) = self.control_clients.get_mut(&client) {
            x.last_used_at = Some(now);
        }
        Ok((
            if full {
                Origin::FullControl { client }
            } else {
                Origin::Control { client }
            },
            cmd,
        ))
    }

    /// A `tool.<name>` call: `read` for tools that only read, `act` for
    /// the rest (14 §2).
    fn tool_check(&mut self, client: ClientId, name: &str) -> Result<Origin, MadoError> {
        if !self.control_on() {
            return Err(err(ErrorCode::PermissionDenied, "error.control.disabled"));
        }
        let c = self
            .control_clients
            .get(&client)
            .filter(|c| !c.revoked)
            .cloned()
            .ok_or_else(|| err(ErrorCode::PermissionDenied, "error.control.unauthorized"))?;
        let tool = mado_agent::tools::find(name)
            .ok_or_else(|| err(ErrorCode::NotFound, "error.control.unknownMethod"))?;
        let scope = if tool.read_only { "read" } else { "act" };
        let full = self.full(&c);
        if !full && !c.scopes.iter().any(|s| s == scope) {
            return Err(err(ErrorCode::Protected, "error.control.scope").arg("scope", scope));
        }
        // Tools act as an ordinary control client: a full session's
        // protected reach is for direct commands only (14 §2.1).
        Ok(Origin::Control { client })
    }

    /// 14 §2.1: every full-control call goes to the audit log and shows a
    /// notice; a setting's own notice carries its Undo.
    fn audit_full(
        &mut self,
        client: ClientId,
        method: &str,
        cmd: &Command,
        rec: &mut PatchRecorder,
    ) {
        let name = self
            .control_clients
            .get(&client)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let now = self.now();
        let mut detail = serde_json::to_value(cmd).unwrap_or(Value::Null);
        // Secrets never go to the log.
        for key in ["secret", "password", "passphrase", "token", "item"] {
            if detail.get(key).is_some() {
                detail[key] = json!("…");
            }
        }
        let (cid, n, m, d) = (
            client.to_string(),
            name.clone(),
            method.to_owned(),
            detail.to_string(),
        );
        if !self.state.profile.is_private {
            let pending = self.profile_db.writer().write(move |c| {
                c.execute(
                    "INSERT INTO control_audit (client_id, client_name, at, method, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![cid, n, now, m, d],
                )
                .map(drop)
            });
            tokio::task::spawn_local(async move {
                if let Err(e) = pending.await {
                    tracing::error!(error = %e, "a control call was not audited");
                }
            });
        }
        if matches!(
            cmd,
            Command::SettingsSet { .. } | Command::SettingsReset { .. }
        ) || cmd.is_query()
        {
            return;
        }
        let notice = Notice {
            id: format!("n{}", self.next_notice),
            message_key: "notice.controlCall".into(),
            args: BTreeMap::from([
                ("client".to_owned(), json!(name)),
                ("command".to_owned(), json!(method)),
            ]),
            action: None,
            undo: None,
            expires_at: None,
        };
        self.next_notice += 1;
        self.add_notice(notice, rec);
    }

    /// `developer.cliFullControl` turned off: `full` leaves every client.
    pub(super) fn full_control_off(&mut self) {
        for c in self.control_clients.values_mut() {
            c.scopes.retain(|s| s != "full");
        }
        if self.state.profile.is_private {
            return;
        }
        let pending = self.profile_db.writer().write(|c| {
            c.execute(
                "UPDATE control_clients SET scopes = (SELECT json_group_array(value) FROM json_each(control_clients.scopes) WHERE value != 'full')",
                [],
            )
            .map(drop)
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "full control was not removed");
            }
        });
    }
}
