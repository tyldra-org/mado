//! Passkeys (spec 11 §8): the `webauthn` page script's requests, the
//! passkey sheets, and conditional mediation through the fill menu.

use std::collections::BTreeMap;

use mado_autofill::passkey::{self, b64};
use mado_engine::event::FrameInfo;
use mado_engine::page::{WebauthnCancel, WebauthnCreate, WebauthnGet};
use mado_engine::tab::World;
use mado_engine::{EngineOp, TabCommand};
use mado_types::command::{Command, Origin, PasskeyAction};
use mado_types::patch::PatchRecorder;
use mado_types::state::{FillMenuItem, PasskeyAccount, PromptState, VaultStatus};
use mado_types::{ErrorCode, MadoError, PromptId, TabId};
use mado_vault::item::{Item, Passkey};
use mado_vault::{Keys, Vault, VaultError};
use serde_json::{Value, json};

use super::vault::VaultOutcome;
use super::{Core, Reply};

/// What a pending request asked.
#[derive(Debug, Clone)]
pub enum Ask {
    Create {
        user_id: String,
        user_name: String,
        user_display_name: String,
        challenge: String,
        exclude: Vec<String>,
    },
    Get {
        challenge: String,
        allow: Vec<String>,
        conditional: bool,
    },
}

/// A request the page waits on.
#[derive(Debug, Clone)]
pub struct Pending {
    pub(super) origin: String,
    pub(super) rp_id: String,
    pub(super) ask: Ask,
    pub(super) prompt: Option<PromptId>,
}

/// The passkeys for `rp_id`, kept to `allow` when it lists any.
pub(super) fn passkeys_for(
    c: &rusqlite::Connection,
    keys: &Keys,
    rp_id: &str,
    allow: &[String],
) -> Result<Vec<(String, Passkey)>, VaultError> {
    Ok(
        Vault::find(c, keys, &format!("rp:{}", rp_id.to_ascii_lowercase()))?
            .into_iter()
            .filter_map(|s| match s.item {
                Item::Passkey(p) if p.rp_id.eq_ignore_ascii_case(rp_id) => Some((s.id, p)),
                _ => None,
            })
            .filter(|(_, p)| allow.is_empty() || allow.contains(&p.credential_id))
            .collect(),
    )
}

pub(super) fn account(id: &str, p: &Passkey) -> PasskeyAccount {
    PasskeyAccount {
        item_id: id.to_owned(),
        user_name: p.user_name.clone(),
        user_display_name: p.user_display_name.clone(),
    }
}

/// The fill menu entry of a passkey (conditional mediation, 11 §8.3).
pub(super) fn menu_item(id: &str, p: &Passkey) -> FillMenuItem {
    FillMenuItem {
        item_ref: format!("passkey:{id}"),
        title: p.rp_id.clone(),
        subtitle: p.user_name.clone(),
        kind: "passkey".into(),
        source: "mado".into(),
        sources: vec!["mado".into()],
    }
}

/// Signs for a stored passkey (11 §8.3 step 3), after Touch ID.
pub(super) fn sign_in(
    c: &rusqlite::Connection,
    cfg: &super::vault::VaultConfig,
    keys: &Keys,
    item_id: &str,
    pending: &Pending,
) -> Result<Value, VaultError> {
    let Ask::Get { challenge, .. } = &pending.ask else {
        return Err(VaultError::NotFound);
    };
    let Item::Passkey(p) = Vault::get(c, keys, item_id)?.item else {
        return Err(VaultError::NotFound);
    };
    if !p.rp_id.eq_ignore_ascii_case(&pending.rp_id) {
        return Err(VaultError::NotFound);
    }
    Vault::unlock(c, &*cfg.enclave, &cfg.dir, "sign in with a passkey")?;
    let der = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &p.private_key)
        .map_err(|_| VaultError::BadKey)?;
    let a = passkey::assert(&der, &pending.rp_id, challenge, &pending.origin)
        .map_err(|e| VaultError::Platform(e.to_string()))?;
    Vault::touch(c, item_id, 0)?;
    Ok(json!({
        "id": p.credential_id,
        "clientDataJSON": b64(&a.client_data_json),
        "authenticatorData": b64(&a.authenticator_data),
        "signature": b64(&a.signature),
        "userHandle": p.user_handle,
    }))
}

fn host_of(origin: &str) -> Option<String> {
    url::Url::parse(origin)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

impl Core {
    /// Whether Mado answers passkey requests (11 §8.1).
    pub(super) fn handles_passkeys(&self) -> bool {
        self.vault.is_some()
            && self.state.vault.status != VaultStatus::None
            && !self.state.profile.is_private
            && matches!(
                self.setting_str("passwords.whoFills").as_str(),
                "mado" | "automatic"
            )
    }

    /// Answers the page's request `id`.
    pub(super) fn webauthn_reply(&mut self, tab: TabId, id: u32, answer: Value) {
        let mut message = answer;
        message["type"] = json!("webauthnResult");
        message["id"] = json!(id);
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::PostToPage {
                world: World::Mado,
                message,
            },
        ));
    }

    fn fall_through(&mut self, tab: TabId, id: u32) {
        self.webauthn_reply(tab, id, json!({ "fallThrough": true }));
    }

    fn webauthn_error(&mut self, tab: TabId, id: u32, error: &str) {
        self.webauthn_reply(tab, id, json!({ "error": error }));
    }

    /// The request's RP ID, checked against the frame (11 §8.2 step 2).
    fn rp_for(
        &mut self,
        tab: TabId,
        id: u32,
        frame: &FrameInfo,
        rp_id: Option<&str>,
    ) -> Option<String> {
        if !frame.is_main_frame || !self.handles_passkeys() {
            self.fall_through(tab, id);
            return None;
        }
        let Some(host) = host_of(&frame.origin) else {
            self.fall_through(tab, id);
            return None;
        };
        let rp = rp_id.map_or(host, str::to_ascii_lowercase);
        if passkey::check_rp(&frame.origin, &rp).is_err() {
            self.webauthn_error(tab, id, "SecurityError");
            return None;
        }
        Some(rp)
    }

    fn passkey_sheet(
        &mut self,
        tab: TabId,
        id: u32,
        mode: &str,
        user_name: String,
        accounts: Vec<PasskeyAccount>,
        rec: &mut PatchRecorder,
    ) {
        let Some(window_id) = self.model.tabs.get(&tab).map(|t| t.window_id) else {
            return;
        };
        let Some(pending) = self.webauthn.get_mut(&(tab, id)) else {
            return;
        };
        let prompt = pending.prompt.unwrap_or_else(PromptId::new);
        pending.prompt = Some(prompt);
        let rp_id = pending.rp_id.clone();
        let locked = self.vault_keys.is_none();
        let mut prompts = self.state.prompts.clone();
        prompts.insert(
            prompt,
            PromptState::Passkey {
                id: prompt,
                window_id,
                tab_id: tab,
                mode: mode.into(),
                rp_id,
                user_name,
                accounts,
                locked,
            },
        );
        self.state.tx(rec).set_prompts(prompts);
    }

    fn close_passkey_sheet(&mut self, prompt: Option<PromptId>, rec: &mut PatchRecorder) {
        if let Some(p) = prompt
            && self.state.prompts.contains_key(&p)
        {
            let mut prompts = self.state.prompts.clone();
            prompts.remove(&p);
            self.state.tx(rec).set_prompts(prompts);
        }
    }

    pub(super) fn webauthn_create(
        &mut self,
        tab: TabId,
        frame: &FrameInfo,
        w: WebauthnCreate,
        rec: &mut PatchRecorder,
    ) {
        let Some(rp_id) = self.rp_for(tab, w.id, frame, w.rp_id.as_deref()) else {
            return;
        };
        if !w.algs.contains(&-7) {
            self.fall_through(tab, w.id);
            return;
        }
        let user_name = if w.user_name.is_empty() {
            w.user_display_name.clone()
        } else {
            w.user_name.clone()
        };
        self.webauthn.insert(
            (tab, w.id),
            Pending {
                origin: frame.origin.clone(),
                rp_id,
                ask: Ask::Create {
                    user_id: w.user_id,
                    user_name: w.user_name,
                    user_display_name: w.user_display_name,
                    challenge: w.challenge,
                    exclude: w.exclude,
                },
                prompt: None,
            },
        );
        self.passkey_sheet(tab, w.id, "create", user_name, Vec::new(), rec);
    }

    pub(super) fn webauthn_get(
        &mut self,
        tab: TabId,
        frame: &FrameInfo,
        w: WebauthnGet,
        rec: &mut PatchRecorder,
    ) {
        let Some(rp_id) = self.rp_for(tab, w.id, frame, w.rp_id.as_deref()) else {
            return;
        };
        let conditional = w.conditional;
        self.webauthn.insert(
            (tab, w.id),
            Pending {
                origin: frame.origin.clone(),
                rp_id,
                ask: Ask::Get {
                    challenge: w.challenge,
                    allow: w.allow,
                    conditional,
                },
                prompt: None,
            },
        );
        if conditional {
            // The fill menu offers the passkeys; the page waits until one is
            // chosen (11 §8.3 step 4).
            self.conditional.insert(tab, w.id);
            return;
        }
        if self.vault_keys.is_none() {
            self.passkey_sheet(tab, w.id, "get", String::new(), Vec::new(), rec);
            return;
        }
        self.find_passkeys(tab, w.id);
    }

    /// Reads the request's passkeys; none falls through (11 §8.3 step 2).
    fn find_passkeys(&mut self, tab: TabId, id: u32) {
        let (Some(keys), Some(p)) = (
            self.vault_keys.clone(),
            self.webauthn.get(&(tab, id)).cloned(),
        ) else {
            return;
        };
        let Ask::Get { allow, .. } = &p.ask else {
            return;
        };
        let (rp, allow) = (p.rp_id.clone(), allow.clone());
        let _ = self.vault_job(None, move |c, _| {
            let accounts = passkeys_for(c, &keys, &rp, &allow)?
                .iter()
                .map(|(i, p)| account(i, p))
                .collect();
            Ok(VaultOutcome::PasskeyAccounts { tab, id, accounts })
        });
    }

    pub(super) fn passkey_accounts(
        &mut self,
        tab: TabId,
        id: u32,
        accounts: Vec<PasskeyAccount>,
        rec: &mut PatchRecorder,
    ) {
        if !self.webauthn.contains_key(&(tab, id)) {
            return;
        }
        if accounts.is_empty() {
            let prompt = self.webauthn.remove(&(tab, id)).and_then(|p| p.prompt);
            self.close_passkey_sheet(prompt, rec);
            self.fall_through(tab, id);
            return;
        }
        self.passkey_sheet(tab, id, "get", String::new(), accounts, rec);
    }

    /// After an unlock: locked sign-in sheets read their passkeys.
    pub(super) fn refresh_passkey_sheets(&mut self) {
        let waiting: Vec<(TabId, u32)> = self
            .webauthn
            .iter()
            .filter(|(_, p)| {
                p.prompt.is_some()
                    && matches!(
                        p.ask,
                        Ask::Get {
                            conditional: false,
                            ..
                        }
                    )
            })
            .map(|(k, _)| *k)
            .collect();
        for (tab, id) in waiting {
            self.find_passkeys(tab, id);
        }
    }

    pub(super) fn webauthn_cancel(
        &mut self,
        tab: TabId,
        c: &WebauthnCancel,
        rec: &mut PatchRecorder,
    ) {
        if let Some(p) = self.webauthn.remove(&(tab, c.id)) {
            self.close_passkey_sheet(p.prompt, rec);
        }
        if self.conditional.get(&tab) == Some(&c.id) {
            self.conditional.remove(&tab);
        }
    }

    /// A new document: its requests are gone.
    pub(super) fn forget_webauthn(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        let gone: Vec<(TabId, u32)> = self
            .webauthn
            .keys()
            .filter(|(t, _)| *t == tab)
            .copied()
            .collect();
        for k in gone {
            if let Some(p) = self.webauthn.remove(&k) {
                self.close_passkey_sheet(p.prompt, rec);
            }
        }
        self.conditional.remove(&tab);
    }

    /// The conditional request waiting on `tab`: its RP ID and allowed IDs.
    pub(super) fn conditional_for(&self, tab: TabId) -> Option<(String, Vec<String>)> {
        let id = self.conditional.get(&tab)?;
        let p = self.webauthn.get(&(tab, *id))?;
        match &p.ask {
            Ask::Get { allow, .. } => Some((p.rp_id.clone(), allow.clone())),
            Ask::Create { .. } => None,
        }
    }

    /// A passkey chosen in the fill menu answers the conditional request.
    pub(super) fn fill_passkey(
        &mut self,
        tab: TabId,
        item_id: String,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let id = *self
            .conditional
            .get(&tab)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.vault.noForm"))?;
        self.use_passkey(tab, id, item_id, reply_to)
    }

    fn use_passkey(
        &mut self,
        tab: TabId,
        id: u32,
        item_id: String,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let keys = self.vault_keys()?;
        let pending = self
            .webauthn
            .get(&(tab, id))
            .cloned()
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"))?;
        self.vault_job(reply_to.take(), move |c, cfg| {
            // Cancelling Touch ID or a missing key answers the page too.
            let answer = match sign_in(c, cfg, &keys, &item_id, &pending) {
                Ok(credential) => json!({ "credential": credential }),
                Err(e) => {
                    tracing::info!(error = %e, "a passkey sign-in did not happen");
                    json!({ "error": "NotAllowedError" })
                }
            };
            Ok(VaultOutcome::Webauthn { tab, id, answer })
        })?;
        Ok(Value::Null)
    }

    /// A passkey job finished: answer the page.
    pub(super) fn webauthn_done(
        &mut self,
        tab: TabId,
        id: u32,
        answer: Value,
        rec: &mut PatchRecorder,
    ) {
        if let Some(p) = self.webauthn.remove(&(tab, id)) {
            self.close_passkey_sheet(p.prompt, rec);
        }
        if self.conditional.get(&tab) == Some(&id) {
            self.conditional.remove(&tab);
        }
        self.close_fill_menus(tab, rec);
        self.webauthn_reply(tab, id, answer);
    }

    /// `vault.passkeyRespond`.
    pub(super) fn passkey_respond(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let Command::VaultPasskeyRespond {
            prompt_id,
            action,
            item_id,
        } = cmd
        else {
            return Err(MadoError::internal());
        };
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.vault.notAllowed",
            ));
        }
        let (tab, id) = self
            .webauthn
            .iter()
            .find(|(_, p)| p.prompt == Some(prompt_id))
            .map(|(k, _)| *k)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"))?;
        match action {
            PasskeyAction::Cancel => {
                self.webauthn_done(tab, id, json!({ "error": "NotAllowedError" }), rec);
                Ok(Value::Null)
            }
            PasskeyAction::OtherDevice => {
                self.webauthn_done(tab, id, json!({ "fallThrough": true }), rec);
                Ok(Value::Null)
            }
            PasskeyAction::Use => {
                let item = item_id.ok_or_else(|| {
                    MadoError::new(ErrorCode::InvalidInput, "error.vault.notFound")
                })?;
                self.use_passkey(tab, id, item, reply_to)
            }
            PasskeyAction::Save => {
                let keys = self.vault_keys()?;
                let pending = self
                    .webauthn
                    .get(&(tab, id))
                    .cloned()
                    .ok_or_else(MadoError::internal)?;
                let now = self.now();
                self.vault_job(reply_to.take(), move |c, cfg| {
                    let made = create_passkey(c, cfg, &keys, &pending, now);
                    let answer = made.unwrap_or_else(|e| {
                        tracing::info!(error = %e, "a passkey was not created");
                        json!({ "error": "NotAllowedError" })
                    });
                    Ok(VaultOutcome::Webauthn { tab, id, answer })
                })?;
                Ok(Value::Null)
            }
        }
    }
}

/// 11 §8.2 steps 4 and 5: after Touch ID, a new key, stored as a passkey.
fn create_passkey(
    c: &rusqlite::Connection,
    cfg: &super::vault::VaultConfig,
    keys: &Keys,
    pending: &Pending,
    now: i64,
) -> Result<Value, VaultError> {
    let Ask::Create {
        user_id,
        user_name,
        user_display_name,
        challenge,
        exclude,
    } = &pending.ask
    else {
        return Err(VaultError::NotFound);
    };
    let known = passkeys_for(c, keys, &pending.rp_id, &[])?;
    if known
        .iter()
        .any(|(_, p)| exclude.contains(&p.credential_id))
    {
        return Ok(json!({ "error": "InvalidStateError" }));
    }
    Vault::unlock(c, &*cfg.enclave, &cfg.dir, "create a passkey")?;
    let made = passkey::create(&pending.rp_id, challenge, &pending.origin)
        .map_err(|e| VaultError::Platform(e.to_string()))?;
    let credential_id = b64(&made.credential_id);
    Vault::add(
        c,
        keys,
        &Item::Passkey(Passkey {
            rp_id: pending.rp_id.clone(),
            credential_id: credential_id.clone(),
            user_handle: user_id.clone(),
            user_name: user_name.clone(),
            user_display_name: user_display_name.clone(),
            private_key: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &made.private_key,
            ),
            algorithm: -7,
            created_at: now,
            login_id: None,
            sign_count: 0,
        }),
        now,
    )?;
    Ok(json!({ "credential": {
                        "id": credential_id,
                        "clientDataJSON": b64(&made.client_data_json),
                        "attestationObject": b64(&made.attestation_object),
                        "authenticatorData": b64(&made.authenticator_data),
                        "publicKey": b64(&made.public_key),
                    } }))
}

/// Requests per tab and page ID.
pub(super) type Requests = BTreeMap<(TabId, u32), Pending>;
