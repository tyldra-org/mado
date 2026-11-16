//! Mado Passwords in the core (spec 11 §2): setup, unlock and lock, the
//! recovery secret and the vault's items. Keys live here only while the
//! vault is unlocked. Every vault operation runs on a blocking thread with
//! its own connection to `vault.sqlite`; Touch ID blocks there too.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::VaultStatus;
use mado_types::{ErrorCode, MadoError};
use mado_vault::{Item, Keys, Stored, Vault, VaultError};
use serde_json::{Value, json};

use super::{Core, Internal, Reply, Timer};

/// How often the inactivity lock looks.
pub(super) const LOCK_CHECK: Duration = Duration::from_secs(30);

/// Mado Passwords' files (spec 11 §2); `None` in private and guest
/// profiles.
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// `<profile>/vault.sqlite`, already migrated.
    pub db: PathBuf,
    /// The profile folder, which holds the wrapped key files.
    pub dir: PathBuf,
    pub enclave: Arc<dyn mado_vault::wrap::Enclave>,
}

impl VaultConfig {
    /// The real Secure Enclave.
    pub fn system(db: PathBuf, dir: PathBuf) -> Self {
        Self {
            db,
            dir,
            enclave: Arc::new(mado_vault::wrap::SecureEnclave),
        }
    }
}

/// What a finished vault job gives the core.
#[derive(Debug)]
pub enum VaultOutcome {
    /// Unlocked (setup, unlock, recover); setup also returns the recovery
    /// secret to show once.
    Keys(Arc<Keys>, Option<String>),
    Value(Value),
    /// A fill menu's items (spec 11 §4.1).
    MenuItems(mado_types::PromptId, Vec<mado_types::state::FillMenuItem>),
    /// A login was saved (spec 11 §5); `changed` when anything was written.
    Saved {
        site: String,
        changed: bool,
    },
    /// The save or update prompt should show.
    OfferSave {
        tab: mado_types::TabId,
        save: Box<super::save::PendingSave>,
        update: bool,
    },
    /// An item resolved for `vault.fill` (spec 11 §4.2).
    Fill {
        target: Box<super::fill::FillTarget>,
        item: Box<Stored>,
        reply: Option<Reply>,
    },
}

pub(super) fn error(e: &VaultError) -> MadoError {
    let (code, key) = match e {
        VaultError::NotSetUp => (ErrorCode::NotFound, "error.vault.notSetUp"),
        VaultError::AlreadySetUp => (ErrorCode::Conflict, "error.vault.alreadySetUp"),
        VaultError::Cancelled => (ErrorCode::PermissionDenied, "error.vault.cancelled"),
        VaultError::BadKey => (ErrorCode::PermissionDenied, "error.vault.badKey"),
        VaultError::BadRecoverySecret => (ErrorCode::InvalidInput, "error.vault.badRecoverySecret"),
        VaultError::NotFound => (ErrorCode::NotFound, "error.vault.notFound"),
        _ => (ErrorCode::Internal, "error.vault.failed"),
    };
    MadoError::new(code, key)
}

fn locked() -> MadoError {
    MadoError::new(ErrorCode::Locked, "error.vault.locked")
}

fn open(db: &std::path::Path) -> Result<rusqlite::Connection, VaultError> {
    let c = rusqlite::Connection::open(db)?;
    c.busy_timeout(Duration::from_secs(5))?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    Ok(c)
}

/// The list entry of an item: no secrets.
pub(super) fn summary(s: &Stored) -> Value {
    let (username, urls) = match &s.item {
        Item::Login(l) => (Some(l.username.clone()), l.urls.clone()),
        Item::Totp(t) => (None, t.urls.clone()),
        Item::Passkey(p) => (
            Some(p.user_name.clone()),
            vec![format!("https://{}", p.rp_id)],
        ),
        _ => (None, Vec::new()),
    };
    json!({
        "id": s.id,
        "kind": s.item.kind(),
        "title": s.item.title(),
        "username": username,
        "urls": urls,
        "createdAt": s.created_at,
        "updatedAt": s.updated_at,
        "lastUsedAt": s.last_used_at,
        "deletedAt": s.deleted_at,
    })
}

/// The whole item, secrets included, with its times.
pub(super) fn full(s: &Stored) -> Value {
    json!({
        "id": s.id,
        "item": s.item,
        "createdAt": s.created_at,
        "updatedAt": s.updated_at,
        "lastUsedAt": s.last_used_at,
        "deletedAt": s.deleted_at,
    })
}

impl Core {
    /// Before the first revision: whether a vault exists, and the purge of
    /// Recently deleted.
    pub(super) fn load_vault(&mut self, rec: &mut PatchRecorder) {
        let Some(config) = self.vault.clone() else {
            return;
        };
        let now = self.now();
        let status = open(&config.db)
            .and_then(|c| {
                let _ = Vault::purge(&c, now);
                Vault::status(&c)
            })
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "the vault could not be read");
                VaultStatus::None
            });
        self.state.tx(rec).vault().set_status(status);
        self.timers.schedule(Timer::VaultLock, LOCK_CHECK);
    }

    /// Purges Recently deleted items older than 30 days (daily).
    pub(super) fn purge_vault(&self) {
        let now = self.now();
        let _ = self.vault_job(None, move |c, _| {
            let n = Vault::purge(c, now)?;
            if n > 0 {
                tracing::info!(purged = n, "Recently deleted items were purged");
            }
            Ok(VaultOutcome::Value(Value::Null))
        });
    }

    fn set_vault_status(&mut self, status: VaultStatus, rec: &mut PatchRecorder) {
        if self.state.vault.status != status {
            self.state.tx(rec).vault().set_status(status);
        }
    }

    /// Locks: the keys go (spec 11 §2.3).
    pub(super) fn lock_vault(&mut self, rec: &mut PatchRecorder) {
        if self.vault_keys.take().is_some() {
            tracing::info!("the vault locked");
        }
        self.vault_fill_until = 0;
        if self.state.vault.status == VaultStatus::Unlocked {
            self.set_vault_status(VaultStatus::Locked, rec);
        }
    }

    /// The inactivity check of `vault.lockAfter`.
    pub(super) fn vault_lock_check(&mut self, rec: &mut PatchRecorder) {
        self.timers.schedule(Timer::VaultLock, LOCK_CHECK);
        let after = self.setting("vault.lockAfter").as_i64().unwrap_or(900_000);
        if self.vault_keys.is_some() && self.now() - self.vault_used_at >= after {
            self.lock_vault(rec);
        }
    }

    /// The keys, marking the vault used.
    pub(super) fn vault_keys(&mut self) -> Result<Arc<Keys>, MadoError> {
        let keys = self.vault_keys.clone().ok_or_else(locked)?;
        self.vault_used_at = self.now();
        Ok(keys)
    }

    /// Runs `job` on a blocking thread with a vault connection; its outcome
    /// comes back as `Internal::VaultDone`.
    pub(super) fn vault_job(
        &self,
        reply: Option<Reply>,
        job: impl FnOnce(&rusqlite::Connection, &VaultConfig) -> Result<VaultOutcome, VaultError>
        + Send
        + 'static,
    ) -> Result<(), MadoError> {
        let config = self
            .vault
            .clone()
            .ok_or_else(|| MadoError::new(ErrorCode::Unsupported, "error.vault.unavailable"))?;
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let result = tokio::task::spawn_blocking(move || {
                let conn = open(&config.db)?;
                job(&conn, &config)
            })
            .await
            .unwrap_or_else(|e| Err(VaultError::Platform(e.to_string())));
            let _ = inputs.send(super::Input::Internal(Internal::VaultDone {
                result: Box::new(result),
                reply,
            }));
        });
        Ok(())
    }

    /// A vault job finished.
    pub(super) fn vault_done(
        &mut self,
        result: Result<VaultOutcome, VaultError>,
        reply: Option<Reply>,
        rec: &mut PatchRecorder,
    ) {
        let answer = match result {
            Ok(VaultOutcome::Keys(keys, secret)) => {
                let now = self.now();
                self.vault_keys = Some(keys);
                self.vault_used_at = now;
                let window = self
                    .setting("vault.fillApprovalWindow")
                    .as_i64()
                    .unwrap_or(300_000);
                self.vault_fill_until = now + window;
                self.set_vault_status(VaultStatus::Unlocked, rec);
                self.refresh_fill_menus(rec);
                Ok(secret.map_or(Value::Null, |s| json!({ "recoverySecret": s })))
            }
            Ok(VaultOutcome::Value(v)) => Ok(v),
            Ok(VaultOutcome::Saved { site, changed }) => {
                if changed {
                    self.saved_notice(&site, rec);
                }
                Ok(Value::Null)
            }
            Ok(VaultOutcome::OfferSave { tab, save, update }) => {
                self.offer_save(tab, *save, update, rec);
                Ok(Value::Null)
            }
            Ok(VaultOutcome::MenuItems(id, items)) => {
                self.fill_menu_items(id, items, rec);
                return;
            }
            Ok(VaultOutcome::Fill {
                target,
                item,
                reply,
            }) => {
                let now = self.now();
                self.fill_resolved(*target, *item, reply, now);
                return;
            }
            Err(e) => {
                if !matches!(
                    e,
                    VaultError::Cancelled | VaultError::BadRecoverySecret | VaultError::NotFound
                ) {
                    tracing::error!(error = %e, "a vault operation failed");
                }
                Err(error(&e))
            }
        };
        let waiting = std::mem::take(&mut self.vault_waiting);
        self.vault_unlocking = false;
        match reply {
            Some(r) => {
                let _ = r.send(answer.clone());
            }
            None => {
                for r in waiting {
                    let _ = r.send(answer.clone());
                }
            }
        }
    }

    /// The `vault` commands (spec 11). Only Mado's own windows send them.
    pub(super) fn vault_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.vault.notAllowed",
            ));
        }
        if self.vault.is_none() {
            return Err(MadoError::new(
                ErrorCode::Unsupported,
                "error.vault.unavailable",
            ));
        }
        let now = self.now();
        match cmd {
            Command::VaultStatus => Ok(json!({ "status": self.state.vault.status })),
            Command::VaultFill {
                tab_id,
                form_key,
                item_ref,
            } => self.vault_fill(origin, tab_id, &form_key, &item_ref, reply_to, rec),
            cmd @ Command::VaultSaveRespond { .. } => self.save_respond(origin, cmd, reply_to, rec),
            Command::VaultLock => {
                self.lock_vault(rec);
                Ok(Value::Null)
            }
            Command::VaultSetup => {
                if self.state.vault.status != VaultStatus::None {
                    return Err(MadoError::new(
                        ErrorCode::Conflict,
                        "error.vault.alreadySetUp",
                    ));
                }
                self.vault_job(reply_to.take(), move |c, cfg| {
                    let (keys, secret) = Vault::setup(c, &*cfg.enclave, &cfg.dir, now)?;
                    Ok(VaultOutcome::Keys(Arc::new(keys), Some(secret)))
                })?;
                Ok(Value::Null)
            }
            Command::VaultUnlock => {
                match self.state.vault.status {
                    VaultStatus::None => return Err(error(&VaultError::NotSetUp)),
                    VaultStatus::Unlocked => return Ok(Value::Null),
                    VaultStatus::Locked => {}
                }
                if let Some(r) = reply_to.take() {
                    self.vault_waiting.push(r);
                }
                if !self.vault_unlocking {
                    self.vault_unlocking = true;
                    self.vault_job(None, |c, cfg| {
                        let keys =
                            Vault::unlock(c, &*cfg.enclave, &cfg.dir, "unlock Mado Passwords")?;
                        Ok(VaultOutcome::Keys(Arc::new(keys), None))
                    })?;
                }
                Ok(Value::Null)
            }
            Command::VaultRecover { secret } => {
                if self.state.vault.status == VaultStatus::None {
                    return Err(error(&VaultError::NotSetUp));
                }
                self.vault_job(reply_to.take(), move |c, cfg| {
                    let keys = Vault::recover(c, &*cfg.enclave, &cfg.dir, &secret)?;
                    Ok(VaultOutcome::Keys(Arc::new(keys), None))
                })?;
                Ok(Value::Null)
            }
            Command::VaultQuery { deleted, site } => {
                let keys = self.vault_keys()?;
                self.vault_job(reply_to.take(), move |c, _| {
                    let items = match site {
                        Some(s) => {
                            let site = mado_vault::item::site_of(&s).unwrap_or(s);
                            Vault::find(c, &keys, &format!("site:{site}"))?
                        }
                        None => Vault::list(c, &keys, deleted)?,
                    };
                    Ok(VaultOutcome::Value(Value::Array(
                        items.iter().map(summary).collect(),
                    )))
                })?;
                Ok(Value::Null)
            }
            Command::VaultGet { id } => {
                let keys = self.vault_keys()?;
                self.vault_job(reply_to.take(), move |c, _| {
                    Ok(VaultOutcome::Value(full(&Vault::get(c, &keys, &id)?)))
                })?;
                Ok(Value::Null)
            }
            Command::VaultSave { id, item } => {
                let item: Item = serde_json::from_value(item).map_err(|e| {
                    MadoError::new(ErrorCode::InvalidInput, "error.vault.badItem")
                        .arg("reason", e.to_string())
                })?;
                let keys = self.vault_keys()?;
                self.vault_job(reply_to.take(), move |c, _| {
                    let id = match id {
                        Some(id) => {
                            Vault::update(c, &keys, &id, &item, now)?;
                            id
                        }
                        None => Vault::add(c, &keys, &item, now)?,
                    };
                    Ok(VaultOutcome::Value(json!({ "id": id })))
                })?;
                Ok(Value::Null)
            }
            Command::VaultDelete { id } => {
                self.vault_keys()?;
                self.vault_job(reply_to.take(), move |c, _| {
                    Vault::delete(c, &id, now)?;
                    Ok(VaultOutcome::Value(Value::Null))
                })?;
                Ok(Value::Null)
            }
            Command::VaultRestore { id } => {
                self.vault_keys()?;
                self.vault_job(reply_to.take(), move |c, _| {
                    Vault::restore(c, &id)?;
                    Ok(VaultOutcome::Value(Value::Null))
                })?;
                Ok(Value::Null)
            }
            Command::VaultDeleteForever { id } => {
                self.vault_keys()?;
                self.vault_job(reply_to.take(), move |c, _| {
                    Vault::delete_forever(c, &id)?;
                    Ok(VaultOutcome::Value(Value::Null))
                })?;
                Ok(Value::Null)
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                Err(MadoError::internal())
            }
        }
    }
}
