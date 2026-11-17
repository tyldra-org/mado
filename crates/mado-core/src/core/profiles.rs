//! Profile commands in the core (spec 12 §1): checks here, the work in the
//! app's `Profiles` service, off the core thread.

use std::sync::Arc;

use mado_types::command::{Command, Origin};
use mado_types::{ErrorCode, MadoError, PatchRecorder};
use serde_json::{Value, json};

use super::{Core, Internal, Reply};
use crate::ui::Profiles;

const COLORS: &[&str] = &[
    "blue", "red", "orange", "yellow", "green", "teal", "purple", "pink", "grey",
];

fn bad(key: &str) -> MadoError {
    MadoError::new(ErrorCode::InvalidInput, key)
}

fn check_name(name: &str) -> Result<String, MadoError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(bad("error.profile.badName"));
    }
    Ok(name.to_owned())
}

fn check_path(path: &str) -> Result<(), MadoError> {
    if std::path::Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(bad("error.file.notAbsolute"))
    }
}

fn check_color(color: &str) -> Result<(), MadoError> {
    if COLORS.contains(&color) {
        Ok(())
    } else {
        Err(bad("error.profile.badColor"))
    }
}

impl Core {
    pub(super) fn profile_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        _rec: &mut PatchRecorder,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let changes = !matches!(cmd, Command::ProfileList | Command::ProfileLinkRules);
        if changes && !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.profile.notAllowed",
            ));
        }
        let own = self.state.profile.id;
        type Job = Box<dyn FnOnce(&dyn Profiles) -> Result<Value, MadoError> + Send>;
        let job: Job = match cmd {
            Command::ProfileList => Box::new(|p| p.list()),
            Command::ProfileCreate {
                name,
                color,
                avatar,
            } => {
                let name = check_name(&name)?;
                check_color(&color)?;
                Box::new(move |p| p.create(&name, &color, &avatar))
            }
            Command::ProfileUpdate {
                id,
                name,
                color,
                avatar,
            } => {
                let name = name.as_deref().map(check_name).transpose()?;
                if let Some(c) = &color {
                    check_color(c)?;
                }
                let inputs = self.inputs.clone();
                Box::new(move |p| {
                    let info = p.update(id, name, color, avatar)?;
                    if id == own {
                        let _ = inputs.send(super::Input::Internal(Internal::ProfileChanged(
                            info.clone(),
                        )));
                    }
                    Ok(json!(info))
                })
            }
            Command::ProfileDelete { id } => {
                // The process cannot delete the profile it runs: another
                // profile's window or the profile picker does (spec 12 §1.4).
                if id == own {
                    return Err(bad("error.profile.current"));
                }
                Box::new(move |p| p.delete(id).map(|()| Value::Null))
            }
            Command::ProfileOpen { id } => Box::new(move |p| p.open(id).map(|()| Value::Null)),
            Command::ProfileExport {
                id,
                path,
                passphrase,
            } => {
                check_path(&path)?;
                // This profile's passwords join the file, sealed with the
                // export's passphrase (12 §1.5); another profile's process
                // holds its own keys.
                let vault = match (&passphrase, &self.vault) {
                    (Some(pass), Some(cfg))
                        if id == own
                            && self.state.vault.status != mado_types::state::VaultStatus::None =>
                    {
                        Some((cfg.db.clone(), self.vault_keys()?, pass.clone()))
                    }
                    _ => None,
                };
                Box::new(move |p| {
                    let file = vault
                        .map(|(db, keys, pass)| {
                            let c = rusqlite::Connection::open(db)?;
                            let items: Vec<mado_vault::Item> =
                                mado_vault::Vault::list(&c, &keys, false)?
                                    .into_iter()
                                    .map(|s| s.item)
                                    .collect();
                            mado_vault::backup::seal(&items, &pass)
                        })
                        .transpose()
                        .map_err(|e| super::vault::error(&e))?;
                    p.export(id, &path, file.as_deref()).map(|()| Value::Null)
                })
            }
            Command::ProfileImport { path, .. } => {
                check_path(&path)?;
                Box::new(move |p| p.import(&path))
            }
            Command::ProfileLinkRules => Box::new(|p| p.link_rules().map(|r| json!(r))),
            Command::ProfileLinkRulesSet { rules } => {
                for r in &rules {
                    if !valid_pattern(&r.pattern) {
                        return Err(
                            bad("error.profile.badPattern").arg("pattern", r.pattern.clone())
                        );
                    }
                }
                Box::new(move |p| p.set_link_rules(rules).map(|()| Value::Null))
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                return Err(MadoError::internal());
            }
        };
        let service = Arc::clone(&self.profiles);
        if let Some(reply) = reply_to.take() {
            tokio::task::spawn_blocking(move || {
                let _ = reply.send(job(&*service));
            });
        }
        Ok(Value::Null)
    }

    pub(super) fn profile_changed(
        &mut self,
        info: mado_types::state::ProfileInfo,
        rec: &mut PatchRecorder,
    ) {
        let mut profile = self.state.profile.clone();
        profile.name = info.name.clone();
        profile.color = info.color.clone();
        profile.avatar = info.avatar.clone();
        self.state.tx(rec).set_profile(profile);
        self.ui.profile_changed(&info);
    }
}

/// A link rule pattern (spec 12 §1.3): `host` or `*.domain`, then an
/// optional path prefix starting with `/`.
pub fn valid_pattern(p: &str) -> bool {
    let (host, path) = p.split_once('/').map_or((p, ""), |(h, rest)| (h, rest));
    let host = host.strip_prefix("*.").unwrap_or(host);
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && !path.contains(char::is_whitespace)
}
