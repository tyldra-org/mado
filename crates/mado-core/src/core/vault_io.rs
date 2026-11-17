//! Password import and export (spec 11 §10): CSV from other password
//! managers and browsers, CSV out, and `.madovault` files.

use mado_autofill::csvio;
use mado_types::{ErrorCode, MadoError};
use mado_vault::item::{Item, Login, PasswordChange, site_of};
use mado_vault::{Keys, Vault, VaultError, backup};
use serde_json::{Value, json};

use super::vault::VaultOutcome;
use super::{Core, Reply};

/// What a merge did to one login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Merged {
    Added,
    Updated,
    Unchanged,
}

/// Adds `login`, or merges it into the item with the same site and
/// username: the newer password wins and the other goes to the history.
/// `changed_at` says when the imported password changed, if known.
fn merge_login(
    c: &rusqlite::Connection,
    keys: &Keys,
    mut login: Login,
    changed_at: Option<i64>,
    now: i64,
) -> Result<Merged, VaultError> {
    let site = login.urls.iter().find_map(|u| site_of(u));
    let existing = match &site {
        Some(s) => Vault::find(c, keys, &format!("site:{s}"))?
            .into_iter()
            .find(|x| matches!(&x.item, Item::Login(l) if l.username == login.username)),
        None => None,
    };
    let Some(found) = existing else {
        Vault::add(c, keys, &Item::Login(login), now)?;
        return Ok(Merged::Added);
    };
    let Item::Login(mut old) = found.item else {
        return Ok(Merged::Unchanged);
    };
    let mut changed = false;
    for u in login.urls.drain(..) {
        if !old.urls.contains(&u) {
            old.urls.push(u);
            changed = true;
        }
    }
    if old.totp.is_none() && login.totp.is_some() {
        old.totp = login.totp.take();
        changed = true;
    }
    if old.notes.is_none() && login.notes.is_some() {
        old.notes = login.notes.take();
        changed = true;
    }
    if !login.password.is_empty() && login.password != old.password {
        if changed_at.is_some_and(|t| t < found.updated_at) {
            // The vault's password is newer: keep it, remember this one.
            if !old
                .password_history
                .iter()
                .any(|h| h.password == login.password)
            {
                old.password_history.push(PasswordChange {
                    password: login.password,
                    changed_at: changed_at.unwrap_or(0),
                });
                changed = true;
            }
        } else {
            // Vault::update moves the old password to the history.
            old.password = login.password;
            changed = true;
        }
    }
    if changed {
        Vault::update(c, keys, &found.id, &Item::Login(old), now)?;
        Ok(Merged::Updated)
    } else {
        Ok(Merged::Unchanged)
    }
}

/// A `.madoprofile`'s passwords, waiting in the profile folder.
pub(super) const PENDING: &str = "imported.madovault";

fn absolute(path: &str) -> Result<std::path::PathBuf, MadoError> {
    let p = std::path::PathBuf::from(path);
    if p.is_absolute() {
        Ok(p)
    } else {
        Err(MadoError::new(
            ErrorCode::InvalidInput,
            "error.vault.badPath",
        ))
    }
}

fn io(e: &std::io::Error) -> VaultError {
    VaultError::Platform(e.to_string())
}

impl Core {
    pub(super) fn vault_import_csv(
        &mut self,
        path: &str,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let path = absolute(path)?;
        let keys = self.vault_keys()?;
        let now = self.now();
        self.vault_job(reply_to.take(), move |c, _| {
            let text = std::fs::read_to_string(&path).map_err(|e| io(&e))?;
            let (format, rows) =
                csvio::import(&text).map_err(|e| VaultError::Platform(format!("csv: {e}")))?;
            let (mut added, mut updated, mut unchanged) = (0, 0, 0);
            for row in rows {
                match merge_login(c, &keys, row.login, row.changed_at, now)? {
                    Merged::Added => added += 1,
                    Merged::Updated => updated += 1,
                    Merged::Unchanged => unchanged += 1,
                }
            }
            Ok(VaultOutcome::Value(json!({
                "format": format.name(), "added": added, "updated": updated, "unchanged": unchanged,
            })))
        })?;
        Ok(Value::Null)
    }

    pub(super) fn vault_export_csv(
        &mut self,
        path: &str,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let path = absolute(path)?;
        let keys = self.vault_keys()?;
        self.vault_job(reply_to.take(), move |c, cfg| {
            Vault::unlock(c, &*cfg.enclave, &cfg.dir, "export your passwords")?;
            let logins: Vec<Login> = Vault::list(c, &keys, false)?
                .into_iter()
                .filter_map(|s| match s.item {
                    Item::Login(l) => Some(l),
                    _ => None,
                })
                .collect();
            mado_vault::wrap::write_private(&path, csvio::export(&logins).as_bytes())
                .map_err(|e| io(&e))?;
            Ok(VaultOutcome::Value(json!({ "logins": logins.len() })))
        })?;
        Ok(Value::Null)
    }

    pub(super) fn vault_export(
        &mut self,
        path: &str,
        passphrase: String,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let path = absolute(path)?;
        if passphrase.chars().count() < 8 {
            return Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.vault.shortPassphrase",
            ));
        }
        let keys = self.vault_keys()?;
        self.vault_job(reply_to.take(), move |c, cfg| {
            Vault::unlock(c, &*cfg.enclave, &cfg.dir, "export your passwords")?;
            let items: Vec<Item> = Vault::list(c, &keys, false)?
                .into_iter()
                .map(|s| s.item)
                .collect();
            let file = backup::seal(&items, &passphrase)?;
            mado_vault::wrap::write_private(&path, &file).map_err(|e| io(&e))?;
            Ok(VaultOutcome::Value(json!({ "items": items.len() })))
        })?;
        Ok(Value::Null)
    }

    pub(super) fn vault_import(
        &mut self,
        path: &str,
        passphrase: String,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let path = absolute(path)?;
        let keys = self.vault_keys()?;
        let now = self.now();
        self.vault_job(reply_to.take(), move |c, cfg| {
            let bytes = std::fs::read(&path).map_err(|e| io(&e))?;
            let items = backup::open(&bytes, &passphrase)?;
            if path == cfg.dir.join(PENDING) {
                let _ = std::fs::remove_file(&path);
            }
            let (mut added, mut updated) = (0, 0);
            for item in items {
                match item {
                    Item::Login(l) => match merge_login(c, &keys, l, None, now)? {
                        Merged::Added => added += 1,
                        Merged::Updated => updated += 1,
                        Merged::Unchanged => {}
                    },
                    other => {
                        Vault::add(c, &keys, &other, now)?;
                        added += 1;
                    }
                }
            }
            Ok(VaultOutcome::Value(
                json!({ "added": added, "updated": updated }),
            ))
        })?;
        Ok(Value::Null)
    }
}
