//! Adding two-factor codes from a page (spec 11 §7): `vault.scanQr` takes
//! a snapshot of the tab, finds an `otpauth` QR code, and
//! `vault.attachTotp` adds it to a login or as a code of its own. The
//! secret stays in the core between the two.

use mado_engine::EngineOp;
use mado_engine::tab::{Image, TabCommand};
use mado_types::command::Origin;
use mado_types::{ErrorCode, MadoError, TabId};
use mado_vault::Vault;
use mado_vault::item::{Item, Totp as TotpItem, site_of};
use serde_json::{Value, json};

use super::vault::VaultOutcome;
use super::{Core, Internal, Reply};

/// A scanned code waiting for `vault.attachTotp`.
#[derive(Debug, Clone)]
pub struct Scan {
    pub(super) uri: String,
    pub(super) site: Option<String>,
    pub(super) title: String,
}

impl Core {
    pub(super) fn scan_qr(
        &mut self,
        origin: &Origin,
        tab: TabId,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.vault.notAllowed",
            ));
        }
        self.vault_keys()?;
        let url = self
            .model
            .tabs
            .get(&tab)
            .map(|t| t.url.clone())
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.tab.notFound"))?;
        let Some(reply) = reply_to.take() else {
            return Ok(Value::Null);
        };
        let inputs = self.inputs.clone();
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::Snapshot {
                full_page: false,
                reply: Box::new(move |image: Result<Image, mado_engine::EngineError>| {
                    let _ = inputs.send(super::Input::Internal(Internal::QrSnapshot {
                        url,
                        image: image.ok().map(|i| i.bytes),
                        reply,
                    }));
                }),
            },
        ));
        Ok(Value::Null)
    }

    /// The snapshot arrived: decode it off the core thread.
    pub(super) fn qr_snapshot(&mut self, url: String, image: Option<Vec<u8>>, reply: Reply) {
        let Ok(keys) = self.vault_keys() else {
            let _ = reply.send(Err(MadoError::new(ErrorCode::Locked, "error.vault.locked")));
            return;
        };
        let site = site_of(&url);
        let _ = self.vault_job(Some(reply), move |c, _| {
            let Some((uri, totp)) = image.as_deref().and_then(mado_autofill::qr::find_totp) else {
                return Ok(VaultOutcome::Scanned(None));
            };
            let logins = match &site {
                Some(s) => Vault::find(c, &keys, &format!("site:{s}"))?
                    .into_iter()
                    .filter_map(|x| match &x.item {
                        Item::Login(l) => Some(json!({ "id": x.id, "title": x.item.title(), "username": l.username })),
                        _ => None,
                    })
                    .collect(),
                None => Vec::new(),
            };
            let title = totp
                .issuer
                .clone()
                .or_else(|| site.clone())
                .unwrap_or_default();
            Ok(VaultOutcome::Scanned(Some(Box::new((
                Scan { uri, site: site.clone(), title },
                json!({ "issuer": totp.issuer, "account": totp.account, "site": site, "logins": logins }),
            )))))
        });
    }

    /// A scan finished: keep the code and answer without it.
    pub(super) fn scanned(
        &mut self,
        found: Option<Box<(Scan, Value)>>,
    ) -> Result<Value, MadoError> {
        let Some(found) = found else {
            return Err(MadoError::new(ErrorCode::NotFound, "error.vault.noQr"));
        };
        let (scan, mut answer) = *found;
        let id = mado_types::PromptId::new().to_string();
        if self.scans.len() >= 8 {
            self.scans.clear();
        }
        self.scans.insert(id.clone(), scan);
        answer["scanId"] = json!(id);
        Ok(answer)
    }

    pub(super) fn attach_totp(
        &mut self,
        origin: &Origin,
        scan_id: &str,
        item_id: Option<String>,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.vault.notAllowed",
            ));
        }
        let keys = self.vault_keys()?;
        let scan = self
            .scans
            .remove(scan_id)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.vault.noQr"))?;
        let now = self.now();
        self.vault_job(reply_to.take(), move |c, _| {
            let id = match item_id {
                Some(id) => {
                    let Item::Login(mut l) = Vault::get(c, &keys, &id)?.item else {
                        return Err(mado_vault::VaultError::NotFound);
                    };
                    l.totp = Some(scan.uri);
                    Vault::update(c, &keys, &id, &Item::Login(l), now)?;
                    id
                }
                None => Vault::add(
                    c,
                    &keys,
                    &Item::Totp(TotpItem {
                        title: scan.title,
                        uri: scan.uri,
                        urls: scan
                            .site
                            .map(|s| format!("https://{s}/"))
                            .into_iter()
                            .collect(),
                    }),
                    now,
                )?,
            };
            Ok(VaultOutcome::Value(json!({ "id": id })))
        })?;
        Ok(Value::Null)
    }
}
