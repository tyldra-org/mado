//! Saving (spec 11 §5): a submitted login becomes a save or update prompt
//! once the next page shows it worked; a generated password saves itself.

use mado_engine::event::FrameInfo;
use mado_engine::page::{CredentialsSubmitted, FieldClass, FormKind};
use mado_types::command::{Command, Origin, SaveAction};
use mado_types::patch::PatchRecorder;
use mado_types::state::{Notice, PromptState, VaultStatus};
use mado_types::{ErrorCode, MadoError, PromptId, TabId};
use mado_vault::item::{Item, Login, site_of};
use mado_vault::{Vault, VaultError};
use serde_json::{Value, json};

use super::vault::VaultOutcome;
use super::{Core, Reply};

/// A capture waits this long for its outcome.
const CAPTURE_MS: i64 = 30_000;

/// A submitted login, waiting for the next page.
#[derive(Debug, Clone)]
pub(super) struct Capture {
    pub site: String,
    pub url: String,
    pub username: String,
    pub password: String,
    pub at: i64,
}

/// What a save prompt would store.
#[derive(Debug, Clone)]
pub struct PendingSave {
    pub(super) site: String,
    pub(super) url: String,
    pub(super) username: String,
    pub(super) password: String,
}

/// The item a save writes: the existing login of that username, updated,
/// or a new one.
fn save_into(
    c: &rusqlite::Connection,
    keys: &mado_vault::Keys,
    save: &PendingSave,
    now: i64,
) -> Result<bool, VaultError> {
    let existing = Vault::find(c, keys, &format!("site:{}", save.site))?
        .into_iter()
        .find(|s| matches!(&s.item, Item::Login(l) if l.username == save.username));
    match existing {
        Some(s) => {
            let Item::Login(mut l) = s.item else {
                return Ok(false);
            };
            if l.password == save.password {
                return Ok(false);
            }
            l.password.clone_from(&save.password);
            Vault::update(c, keys, &s.id, &Item::Login(l), now)?;
            Ok(true)
        }
        None => {
            let item = Item::Login(Login {
                title: String::new(),
                urls: vec![save.url.clone()],
                username: save.username.clone(),
                password: save.password.clone(),
                ..Login::default()
            });
            Vault::add(c, keys, &item, now)?;
            Ok(true)
        }
    }
}

impl Core {
    /// Whether this tab may offer saving at all (11 §5 step 5).
    fn may_offer_save(&self, tab: TabId) -> bool {
        self.vault.is_some()
            && self.state.vault.status != VaultStatus::None
            && self.setting("vault.offerToSave") == true
            && self.setting_str("passwords.whoFills") != "extension"
            && self.setting_str("vault.saveDestination.logins") == "mado"
            && self.model.tabs.get(&tab).is_some_and(|t| t.agent.is_none())
    }

    /// 11 §5 step 1: a form was submitted.
    pub(super) fn credentials_submitted(
        &mut self,
        tab: TabId,
        frame: &FrameInfo,
        c: CredentialsSubmitted,
    ) {
        let Some(tab_url) = self.model.tabs.get(&tab).map(|t| t.url.clone()) else {
            return;
        };
        let top_origin = url::Url::parse(&tab_url).map(|u| u.origin().ascii_serialization());
        if !frame.is_main_frame && top_origin.as_deref() != Ok(frame.origin.as_str()) {
            return;
        }
        if !self.may_offer_save(tab) {
            return;
        }
        let Some(site) = site_of(&frame.origin) else {
            return;
        };
        let now = self.now();
        if c.kind == FormKind::UsernameOnly {
            self.username_hints.insert(tab, (site, c.username, now));
            return;
        }
        let username = if c.username.is_empty() {
            self.username_hints
                .get(&tab)
                .filter(|(s, _, at)| *s == site && now - at <= CAPTURE_MS * 2)
                .map(|(_, u, _)| u.clone())
                .unwrap_or_default()
        } else {
            c.username
        };
        self.captures.insert(
            tab,
            Capture {
                site,
                url: format!("{}/", frame.origin.trim_end_matches('/')),
                username,
                password: c.password,
                at: now,
            },
        );
    }

    /// 11 §5 step 3: the tab's main frame reported its forms after a
    /// capture. No visible password field means the sign-in worked.
    pub(super) fn save_check(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        let now = self.now();
        let Some(capture) = self.captures.get(&tab).cloned() else {
            return;
        };
        if now - capture.at > CAPTURE_MS {
            self.captures.remove(&tab);
            return;
        }
        let still_asking = self.page_forms.get(&tab).is_some_and(|frames| {
            frames.iter().filter(|f| f.main).any(|f| {
                f.forms.iter().any(|form| {
                    matches!(
                        form.kind,
                        FormKind::Login | FormKind::Signup | FormKind::ChangePassword
                    ) && form.fields.iter().any(|x| {
                        x.visible
                            && matches!(x.class, FieldClass::Password | FieldClass::NewPassword)
                    })
                })
            })
        });
        if still_asking {
            return;
        }
        self.captures.remove(&tab);
        let save = PendingSave {
            site: capture.site.clone(),
            url: capture.url.clone(),
            username: capture.username.clone(),
            password: capture.password.clone(),
        };
        let generated = self.generated.remove(&tab).is_some_and(|(site, pw, at)| {
            site == capture.site && pw == capture.password && now - at <= 10 * 60_000
        });
        match self.vault_keys.clone() {
            Some(keys) if generated => {
                // A generated password saves itself, with a notice.
                let _ = self.vault_job(None, move |c, _| {
                    let changed = save_into(c, &keys, &save, now)?;
                    Ok(VaultOutcome::Saved {
                        site: save.site,
                        changed,
                    })
                });
            }
            Some(keys) => {
                let _ = self.vault_job(None, move |c, _| {
                    if Vault::is_never_save(c, &keys, &save.site)? {
                        return Ok(VaultOutcome::Value(Value::Null));
                    }
                    let existing = Vault::find(c, &keys, &format!("site:{}", save.site))?
                        .into_iter()
                        .find_map(|s| match s.item {
                            Item::Login(l) if l.username == save.username => Some(l.password),
                            _ => None,
                        });
                    match existing {
                        Some(pw) if pw == save.password => Ok(VaultOutcome::Value(Value::Null)),
                        Some(_) => Ok(VaultOutcome::OfferSave {
                            tab,
                            save: Box::new(save),
                            update: true,
                        }),
                        None => Ok(VaultOutcome::OfferSave {
                            tab,
                            save: Box::new(save),
                            update: false,
                        }),
                    }
                });
            }
            None => self.offer_save(tab, save, false, rec),
        }
    }

    /// Shows the save or update prompt.
    pub(super) fn offer_save(
        &mut self,
        tab: TabId,
        save: PendingSave,
        update: bool,
        rec: &mut PatchRecorder,
    ) {
        let Some(window_id) = self.model.tabs.get(&tab).map(|t| t.window_id) else {
            return;
        };
        let mut prompts = self.state.prompts.clone();
        let old: Vec<PromptId> = prompts
            .iter()
            .filter(
                |(_, p)| matches!(p, PromptState::SavePassword { tab_id, .. } if *tab_id == tab),
            )
            .map(|(id, _)| *id)
            .collect();
        for id in old {
            prompts.remove(&id);
            self.save_prompts.remove(&id);
        }
        let id = PromptId::new();
        prompts.insert(
            id,
            PromptState::SavePassword {
                id,
                window_id,
                tab_id: tab,
                site: save.site.clone(),
                username: save.username.clone(),
                update,
            },
        );
        self.state.tx(rec).set_prompts(prompts);
        self.save_prompts.insert(id, save);
    }

    pub(super) fn saved_notice(&mut self, site: &str, rec: &mut PatchRecorder) {
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: "notice.passwordSaved".into(),
            args: [("site".to_owned(), json!(site))].into(),
            action: None,
            undo: None,
            expires_at: Some(self.now() + 6000),
        };
        self.add_notice(notice, rec);
    }

    /// `vault.saveRespond`.
    pub(super) fn save_respond(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let Command::VaultSaveRespond {
            prompt_id,
            action,
            username,
            password,
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
        let Some(mut save) = self.save_prompts.get(&prompt_id).cloned() else {
            return Err(MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"));
        };
        if let Some(u) = username {
            save.username = u;
        }
        if let Some(p) = password.filter(|p| !p.is_empty()) {
            save.password = p;
        }
        // Saving and Never need the keys; a locked vault answers Locked and
        // the prompt stays for the unlock.
        let keys = match action {
            SaveAction::NotNow => None,
            _ => Some(self.vault_keys()?),
        };
        self.save_prompts.remove(&prompt_id);
        let mut prompts = self.state.prompts.clone();
        prompts.remove(&prompt_id);
        self.state.tx(rec).set_prompts(prompts);
        let now = self.now();
        let reply = reply_to.take();
        match (action, keys) {
            (SaveAction::Save, Some(keys)) => {
                self.vault_job(reply, move |c, _| {
                    save_into(c, &keys, &save, now)?;
                    Ok(VaultOutcome::Saved {
                        site: save.site,
                        changed: true,
                    })
                })?;
            }
            (SaveAction::Never, Some(keys)) => {
                self.vault_job(reply, move |c, _| {
                    Vault::never_save(c, &keys, &save.site, true)?;
                    Ok(VaultOutcome::Value(Value::Null))
                })?;
            }
            _ => {
                if let Some(r) = reply {
                    let _ = r.send(Ok(Value::Null));
                }
            }
        }
        Ok(Value::Null)
    }
}
