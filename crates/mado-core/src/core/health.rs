//! `vault.health` (spec 11 §9): reused and weak logins from the vault,
//! and leaked ones from Have I Been Pwned's range API only when asked.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mado_autofill::health::{self, Entry};
use mado_types::{ErrorCode, MadoError};
use mado_vault::item::{Item, site_of};
use mado_vault::{Keys, Vault};
use serde_json::{Value, json};

use super::vault::VaultConfig;
use super::{Core, Reply};

/// The logins, and each one's title and username for the answer.
type Read = (Vec<Entry>, BTreeMap<String, (String, String)>);

fn read(cfg: &VaultConfig, keys: &Keys) -> Result<Read, mado_vault::VaultError> {
    let c = rusqlite::Connection::open(&cfg.db)?;
    let mut entries = Vec::new();
    let mut names = BTreeMap::new();
    for s in Vault::list(&c, keys, false)? {
        if let Item::Login(l) = &s.item {
            let site = l.urls.iter().find_map(|u| site_of(u)).unwrap_or_default();
            names.insert(s.id.clone(), (s.item.title(), l.username.clone()));
            entries.push(Entry {
                id: s.id.clone(),
                site,
                password: l.password.clone(),
            });
        }
    }
    Ok((entries, names))
}

/// The vault's passwords, so memory refuses them (13 §10). Blocks.
pub(super) fn vault_passwords(cfg: &VaultConfig, keys: &Keys) -> Vec<String> {
    read(cfg, keys)
        .map(|(entries, _)| entries.into_iter().map(|e| e.password).collect())
        .unwrap_or_default()
}

impl Core {
    pub(super) fn vault_health(
        &mut self,
        check_leaks: bool,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        let keys = self.vault_keys()?;
        let cfg = self.vault.clone().ok_or_else(MadoError::internal)?;
        let Some(reply) = reply_to.take() else {
            return Ok(Value::Null);
        };
        tokio::task::spawn_local(async move {
            let answer = check(cfg, keys, check_leaks).await;
            let _ = reply.send(answer);
        });
        Ok(Value::Null)
    }
}

async fn check(cfg: VaultConfig, keys: Arc<Keys>, check_leaks: bool) -> Result<Value, MadoError> {
    let failed = || MadoError::new(ErrorCode::Internal, "error.vault.failed");
    let read_cfg = cfg.clone();
    let ((entries, names), mut report) = tokio::task::spawn_blocking(move || {
        let read = read(&read_cfg, &keys)?;
        let report = health::local(&read.0);
        Ok::<_, mado_vault::VaultError>((read, report))
    })
    .await
    .map_err(|_| failed())?
    .map_err(|_| failed())?;
    if check_leaks {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|_| failed())?;
        let mut by_prefix: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for e in entries.iter().filter(|e| !e.password.is_empty()) {
            let (prefix, suffix) = health::hash(&e.password);
            by_prefix
                .entry(prefix)
                .or_default()
                .push((e.id.clone(), suffix));
        }
        let mut leaked = BTreeSet::new();
        for (prefix, ids) in by_prefix {
            let url = format!("{}/range/{prefix}", cfg.hibp.trim_end_matches('/'));
            let answer = client
                .get(&url)
                .header("Add-Padding", "true")
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|e| {
                    tracing::warn!(error = %e, "the leak check failed");
                    MadoError::new(ErrorCode::NetworkFailed, "error.vault.leakCheckFailed")
                })?
                .text()
                .await
                .map_err(|_| {
                    MadoError::new(ErrorCode::NetworkFailed, "error.vault.leakCheckFailed")
                })?;
            for (id, suffix) in ids {
                if health::in_range(&answer, &suffix) {
                    leaked.insert(id);
                }
            }
        }
        report.leaked = Some(leaked);
    }
    let item = |id: &String| {
        let (title, username) = names.get(id).cloned().unwrap_or_default();
        json!({ "id": id, "title": title, "username": username })
    };
    Ok(json!({
        "reused": report.reused.iter().map(|ids| ids.iter().map(item).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "weak": report.weak.iter().map(item).collect::<Vec<_>>(),
        "leaked": report.leaked.as_ref().map(|l| l.iter().map(item).collect::<Vec<_>>()),
    }))
}
