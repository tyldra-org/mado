//! Commands and state for Mado's own `mado://` pages in tabs (spec 01
//! §4.4). The `internal` page script relays `invoke`; the core trusts
//! only the engine's word on which frame sent it.

use mado_engine::event::FrameInfo;
use mado_engine::page::Invoke;
use mado_types::command::{Command, InternalPage, Origin};
use mado_types::{ErrorCode, MadoError, Patch, TabId};
use serde_json::{Value, json};

use super::{Core, Input, Internal};

/// The top-level state fields a page reads; patches under them are sent
/// to it, and `state.get` answers only them.
fn reads(page: InternalPage) -> &'static [&'static str] {
    match page {
        InternalPage::Downloads => &["downloads"],
        InternalPage::About => &["update", "about"],
        InternalPage::Extensions => &["extensions"],
        _ => &[],
    }
}

impl Core {
    fn answer(&mut self, tab: TabId, id: u64, result: Result<Value, MadoError>) {
        let message = match result {
            Ok(value) => json!({ "type": "invokeReply", "id": id, "ok": true, "value": value }),
            Err(error) => json!({ "type": "invokeReply", "id": id, "ok": false, "error": error }),
        };
        self.post(tab, message);
    }

    /// `invoke` from a tab: only a `mado://` page's main frame, only what
    /// its allow-list names, run as `Origin::InternalPage`.
    pub(super) fn internal_invoke(&mut self, tab: TabId, frame: &FrameInfo, invoke: Invoke) {
        let page = frame
            .is_main_frame
            .then(|| InternalPage::of_url(&frame.url))
            .flatten();
        let Some(page) = page else {
            tracing::warn!(%tab, url = %frame.url, "an invoke from outside a mado:// page was refused");
            return;
        };
        let cmd: Command = match serde_json::from_value(invoke.command) {
            Ok(cmd) => cmd,
            Err(e) => {
                self.answer(
                    tab,
                    invoke.id,
                    Err(
                        MadoError::new(ErrorCode::InvalidInput, "error.control.badCommand")
                            .arg("reason", e.to_string()),
                    ),
                );
                return;
            }
        };
        if !page.allows(&cmd) {
            self.answer(
                tab,
                invoke.id,
                Err(
                    MadoError::new(ErrorCode::Protected, "error.internal.notAllowed")
                        .arg("command", cmd.name()),
                ),
            );
            return;
        }
        let window = self.model.tabs.get(&tab).map(|t| t.window_id);
        if matches!(cmd, Command::StateGet) {
            let state = serde_json::to_value(&self.state).unwrap_or_default();
            let fields: serde_json::Map<String, Value> = reads(page)
                .iter()
                .map(|f| ((*f).to_owned(), state[*f].clone()))
                .collect();
            self.answer(
                tab,
                invoke.id,
                Ok(json!({ "rev": self.rev, "state": fields, "tabId": tab, "windowId": window })),
            );
            return;
        }
        // A page opens links from its own tab, whatever IDs it sent.
        let cmd = match (cmd, window) {
            (
                Command::NavOpen {
                    input,
                    disposition,
                    source,
                    ctrl_enter,
                    ..
                },
                Some(window_id),
            ) => Command::NavOpen {
                window_id,
                tab_id: Some(tab),
                input,
                disposition,
                source,
                ctrl_enter,
            },
            (cmd, _) => cmd,
        };
        let (reply, rx) = tokio::sync::oneshot::channel();
        let _ = self.inputs.send(Input::Command {
            origin: Origin::InternalPage { tab, page },
            cmd,
            reply,
        });
        let inputs = self.inputs.clone();
        let id = invoke.id;
        tokio::task::spawn_local(async move {
            let result = rx.await.unwrap_or_else(|_| Err(MadoError::internal()));
            let _ = inputs.send(Input::Internal(Internal::InvokeAnswered {
                tab,
                id,
                result,
            }));
        });
    }

    pub(super) fn invoke_answered(
        &mut self,
        tab: TabId,
        id: u64,
        result: Result<Value, MadoError>,
    ) {
        if self.model.tabs.contains_key(&tab) {
            self.answer(tab, id, result);
        }
    }

    /// Sends each internal page the operations of `patch` under the fields
    /// it reads. Revisions that touch none of them are skipped.
    pub(super) fn forward_to_pages(&mut self, patch: &Patch) {
        let targets: Vec<(TabId, InternalPage)> = self
            .model
            .tabs
            .iter()
            .filter_map(|(id, t)| Some((*id, InternalPage::of_url(&t.url)?)))
            .filter(|(_, p)| !reads(*p).is_empty())
            .collect();
        for (tab, page) in targets {
            let ops: Vec<_> = patch
                .ops
                .iter()
                .filter(|op| {
                    reads(page).iter().any(|f| {
                        let path = op.path();
                        path == format!("/{f}") || path.starts_with(&format!("/{f}/"))
                    })
                })
                .collect();
            if !ops.is_empty() {
                self.post(
                    tab,
                    json!({ "type": "patch", "rev": patch.rev, "ops": ops }),
                );
            }
        }
    }
}
