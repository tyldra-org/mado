//! Page dialogs, HTTP authentication, client certificates and the
//! insecure form warning (spec 07 §7).

use std::collections::{BTreeMap, BTreeSet};

use mado_engine::event::{
    AuthChallenge, AuthResponse, ClientCertRequest, ClientCertResponse, DialogResponse, JsDialog,
};
use mado_engine::{NavDecision, NavPhase, NavRequest, Reply};
use mado_types::command::{Command, PromptDecision};
use mado_types::state::{ClientIdentity, DialogKind, PromptState};
use mado_types::{ErrorCode, MadoError, PatchRecorder, PromptId, TabId};
use serde_json::Value;

use super::Core;

/// A second dialog within this offers "Don't allow this page to show more
/// dialogs".
const DIALOG_BURST_MS: i64 = 5_000;

/// Answers the core holds for open sheets.
#[derive(Default)]
pub struct Held {
    pub dialogs: BTreeMap<PromptId, (TabId, mado_engine::event::DialogKind, Reply<DialogResponse>)>,
    pub auth: BTreeMap<PromptId, (TabId, Reply<AuthResponse>)>,
    pub certs: BTreeMap<PromptId, (TabId, String, Reply<ClientCertResponse>)>,
    pub forms: BTreeMap<PromptId, (TabId, Reply<NavDecision>)>,
    /// When each tab's document last showed a dialog.
    pub last_dialog: BTreeMap<TabId, i64>,
    /// Documents whose dialogs are answered without showing.
    pub suppressed: BTreeSet<TabId>,
    /// Client certificate choices per origin, for the session.
    pub cert_choice: BTreeMap<String, Option<String>>,
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Held")
            .field("dialogs", &self.dialogs.len())
            .finish_non_exhaustive()
    }
}

fn kind_of(k: &mado_engine::event::DialogKind) -> (DialogKind, Option<String>) {
    match k {
        mado_engine::event::DialogKind::Alert => (DialogKind::Alert, None),
        mado_engine::event::DialogKind::Confirm => (DialogKind::Confirm, None),
        mado_engine::event::DialogKind::Prompt { default } => {
            (DialogKind::Prompt, Some(default.clone()))
        }
        mado_engine::event::DialogKind::BeforeUnload => (DialogKind::BeforeUnload, None),
    }
}

/// What an unanswered dialog says: OK for alerts and `beforeunload` on a
/// suppressed page (leaving is never blocked), cancel otherwise.
fn quiet_answer(kind: &mado_engine::event::DialogKind) -> DialogResponse {
    match kind {
        mado_engine::event::DialogKind::BeforeUnload => DialogResponse::Accept(None),
        _ => DialogResponse::Dismiss,
    }
}

impl Core {
    fn add_prompt(&mut self, id: PromptId, prompt: PromptState, rec: &mut PatchRecorder) {
        let mut prompts = self.state.prompts.clone();
        prompts.insert(id, prompt);
        self.state.tx(rec).set_prompts(prompts);
    }

    fn drop_prompt(&mut self, id: PromptId, rec: &mut PatchRecorder) {
        let mut prompts = self.state.prompts.clone();
        if prompts.remove(&id).is_some() {
            self.state.tx(rec).set_prompts(prompts);
        }
    }

    pub(super) fn dialog_requested(
        &mut self,
        tab: TabId,
        dialog: JsDialog,
        reply: Reply<DialogResponse>,
        rec: &mut PatchRecorder,
    ) {
        let top = self
            .model
            .tabs
            .get(&tab)
            .and_then(|t| mado_permissions::origin_of(&t.url));
        // Cross-origin subframes show nothing, as Chrome does.
        if !dialog.frame.is_main_frame && top.as_deref() != Some(dialog.frame.origin.as_str()) {
            reply.send(DialogResponse::Dismiss);
            return;
        }
        if self.held.suppressed.contains(&tab) {
            reply.send(quiet_answer(&dialog.kind));
            return;
        }
        let now = self.now();
        let offer_suppress = self
            .held
            .last_dialog
            .insert(tab, now)
            .is_some_and(|at| now - at < DIALOG_BURST_MS);
        let (kind, default_text) = kind_of(&dialog.kind);
        let id = PromptId::new();
        self.add_prompt(
            id,
            PromptState::Dialog {
                id,
                tab_id: tab,
                origin: dialog.frame.origin.clone(),
                kind,
                message: dialog.message.chars().take(10_000).collect(),
                default_text,
                offer_suppress,
            },
            rec,
        );
        self.held.dialogs.insert(id, (tab, dialog.kind, reply));
    }

    pub(super) fn auth_requested(
        &mut self,
        tab: TabId,
        c: AuthChallenge,
        reply: Reply<AuthResponse>,
        rec: &mut PatchRecorder,
    ) {
        let id = PromptId::new();
        self.add_prompt(
            id,
            PromptState::Auth {
                id,
                tab_id: tab,
                host: c.host,
                port: c.port,
                realm: c.realm,
                is_proxy: c.is_proxy,
            },
            rec,
        );
        self.held.auth.insert(id, (tab, reply));
    }

    pub(super) fn client_cert_requested(
        &mut self,
        tab: TabId,
        r: ClientCertRequest,
        reply: Reply<ClientCertResponse>,
        rec: &mut PatchRecorder,
    ) {
        let origin = format!("https://{}:{}", r.host, r.port);
        // Remembered for the session.
        if let Some(choice) = self.held.cert_choice.get(&origin) {
            reply.send(
                choice
                    .clone()
                    .map_or(ClientCertResponse::Cancel, ClientCertResponse::Identity),
            );
            return;
        }
        let identities: Vec<ClientIdentity> = (self.os.identities)()
            .into_iter()
            .map(|i| ClientIdentity {
                id: i.id,
                label: i.label,
            })
            .collect();
        if identities.is_empty() {
            reply.send(ClientCertResponse::Cancel);
            return;
        }
        let id = PromptId::new();
        self.add_prompt(
            id,
            PromptState::ClientCert {
                id,
                tab_id: tab,
                host: r.host,
                identities,
            },
            rec,
        );
        self.held.certs.insert(id, (tab, origin, reply));
    }

    /// 07 §7: an https page posting to http waits for "Send anyway".
    /// Returns the reply back when this is not such a navigation.
    pub(super) fn insecure_form(
        &mut self,
        tab: TabId,
        request: &NavRequest,
        reply: Reply<NavDecision>,
        rec: &mut PatchRecorder,
    ) -> Option<Reply<NavDecision>> {
        let post = matches!(&request.phase, NavPhase::Action { method, .. } if method == "POST");
        let from_https = self
            .model
            .tabs
            .get(&tab)
            .is_some_and(|t| t.url.starts_with("https:"));
        if !(request.is_main_frame
            && post
            && from_https
            && request.url.starts_with("http:")
            && !request.initiated_by_mado)
        {
            return Some(reply);
        }
        let id = PromptId::new();
        self.add_prompt(
            id,
            PromptState::InsecureForm {
                id,
                tab_id: tab,
                url: request.url.clone(),
            },
            rec,
        );
        self.held.forms.insert(id, (tab, reply));
        None
    }

    pub(super) fn sheet_command(
        &mut self,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let not_found = || MadoError::new(ErrorCode::NotFound, "error.prompt.notFound");
        match cmd {
            Command::PromptAnswerDialog {
                id,
                accept,
                text,
                suppress,
            } => {
                let (tab, kind, reply) = self.held.dialogs.remove(&id).ok_or_else(not_found)?;
                self.drop_prompt(id, rec);
                if suppress {
                    self.held.suppressed.insert(tab);
                }
                let prompt = matches!(kind, mado_engine::event::DialogKind::Prompt { .. });
                reply.send(if accept {
                    DialogResponse::Accept(if prompt {
                        Some(text.unwrap_or_default())
                    } else {
                        None
                    })
                } else {
                    DialogResponse::Dismiss
                });
            }
            Command::PromptAnswerAuth { id, user, password } => {
                let (_, reply) = self.held.auth.remove(&id).ok_or_else(not_found)?;
                self.drop_prompt(id, rec);
                reply.send(match user {
                    Some(user) => AuthResponse::Credentials {
                        user,
                        password: password.unwrap_or_default(),
                    },
                    None => AuthResponse::Cancel,
                });
            }
            Command::PromptAnswerClientCert { id, identity } => {
                let (_, origin, reply) = self.held.certs.remove(&id).ok_or_else(not_found)?;
                self.drop_prompt(id, rec);
                self.held.cert_choice.insert(origin, identity.clone());
                reply.send(
                    identity.map_or(ClientCertResponse::Cancel, ClientCertResponse::Identity),
                );
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                return Err(MadoError::internal());
            }
        }
        Ok(Value::Null)
    }

    /// `prompt.respond` on the insecure form warning.
    pub(super) fn answer_insecure_form(
        &mut self,
        id: PromptId,
        decision: PromptDecision,
        rec: &mut PatchRecorder,
    ) -> bool {
        let Some((_, reply)) = self.held.forms.remove(&id) else {
            return false;
        };
        self.drop_prompt(id, rec);
        reply.send(if decision == PromptDecision::Never {
            NavDecision::Cancel
        } else {
            NavDecision::Allow
        });
        true
    }

    /// A new document or a closed tab: its sheets close with their safe
    /// answers and dialog suppression ends.
    pub(super) fn forget_sheets(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        self.held.suppressed.remove(&tab);
        self.held.last_dialog.remove(&tab);
        let ids: Vec<PromptId> = self
            .held
            .dialogs
            .iter()
            .filter(|(_, d)| d.0 == tab)
            .map(|(id, _)| *id)
            .chain(
                self.held
                    .auth
                    .iter()
                    .filter(|(_, d)| d.0 == tab)
                    .map(|(id, _)| *id),
            )
            .chain(
                self.held
                    .certs
                    .iter()
                    .filter(|(_, d)| d.0 == tab)
                    .map(|(id, _)| *id),
            )
            .collect();
        for id in ids {
            self.held.dialogs.remove(&id);
            self.held.auth.remove(&id);
            self.held.certs.remove(&id);
            self.drop_prompt(id, rec);
        }
    }
}
