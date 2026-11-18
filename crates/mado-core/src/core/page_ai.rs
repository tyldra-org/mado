//! Page commands in the core (spec 13 §6): "Ask AI" on a selection, the
//! result card, and Replace or Insert below through the `context` script.

use std::sync::Arc;

use mado_agent::commands::{self, Inputs, PageCommand};
use mado_engine::{EngineOp, TabCommand, World};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::PromptState;
use mado_types::{ErrorCode, MadoError, PromptId, TabId};
use serde_json::{Value, json};

use super::ai::AiEvent;
use super::chat::{Attempt, answer};
use super::{Core, Internal};

/// A command run's progress.
#[derive(Debug)]
pub enum CommandEvent {
    /// Start a run from the page menu.
    FromMenu {
        tab: TabId,
        command: String,
        text: String,
        editable: bool,
    },
    Delta {
        prompt: PromptId,
        delta: String,
    },
    Done {
        prompt: PromptId,
        text: String,
        model: Option<String>,
        error: Option<String>,
    },
}

/// What a result card's run needs to run again.
#[derive(Debug, Clone)]
pub(super) struct Run {
    pub tab: TabId,
    pub command: PageCommand,
    pub text: String,
    pub editable: bool,
}

fn language() -> &'static str {
    commands::language_name(&std::env::var("LANG").unwrap_or_default())
}

impl Core {
    /// `features.ai` and `features.textCommands` (spec 13 §6).
    pub(super) fn page_commands_on(&self) -> bool {
        self.setting("features.ai") == true && self.setting("features.textCommands") == true
    }

    fn page_commands(&self) -> Vec<PageCommand> {
        let mut all = commands::builtins(language());
        all.extend(commands::custom(&self.setting("ai.commands.custom")));
        all
    }

    pub(super) fn ai_page_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        match cmd {
            Command::AiCommandList => Ok(json!(
                self.page_commands()
                    .iter()
                    .map(|c| json!({
                        "id": c.id,
                        "name": c.name,
                        "output": c.output,
                        "builtin": c.builtin,
                        "onDevice": c.on_device,
                        "forEditable": c.for_editable,
                    }))
                    .collect::<Vec<_>>()
            )),
            Command::AiCommandRun {
                tab_id,
                command_id,
                text,
                editable,
                model,
            } => {
                if !origin.is_user() {
                    return Err(MadoError::new(ErrorCode::Protected, "error.ai.notAllowed"));
                }
                let id = self.run_page_command(tab_id, &command_id, text, editable, model, rec)?;
                Ok(json!({ "promptId": id }))
            }
            Command::AiCommandRespond {
                prompt_id,
                action,
                model,
            } => {
                if !origin.is_user() {
                    return Err(MadoError::new(ErrorCode::Protected, "error.ai.notAllowed"));
                }
                self.respond_page_command(prompt_id, &action, model, rec)
            }
            _ => Err(MadoError::new(
                ErrorCode::Unsupported,
                "error.unknownCommand",
            )),
        }
    }

    /// Starts a run and shows its card; `model` overrides the command's.
    fn run_page_command(
        &mut self,
        tab: TabId,
        command_id: &str,
        text: String,
        editable: bool,
        model: Option<String>,
        rec: &mut PatchRecorder,
    ) -> Result<PromptId, MadoError> {
        if !self.page_commands_on() {
            return Err(MadoError::new(ErrorCode::Unsupported, "error.ai.off"));
        }
        let command = self
            .page_commands()
            .into_iter()
            .find(|c| c.id == command_id)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.ai.noCommand"))?;
        let (window, url, title) = self
            .model
            .tabs
            .get(&tab)
            .map(|t| (t.window_id, t.url.clone(), t.title.clone()))
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.tab.notFound"))?;
        let sensitive = mado_lists::sensitive_hosts().banking.clone();
        let allowed: Vec<String> = self
            .setting("ai.allowedSensitiveSites")
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(reason) = mado_agent::untrusted::refused(&url, &sensitive, &allowed) {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                format!("error.ai.{reason}"),
            ));
        }
        if text.trim().is_empty() {
            return Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.ai.emptyMessage",
            ));
        }
        // Built-ins go on-device when it can (13 §6), unless a model was
        // chosen; otherwise the command's role or model.
        let on_device = model.is_none() && command.on_device && self.ondevice_ready;
        let attempts = if on_device {
            Vec::new()
        } else {
            let wanted = model
                .clone()
                .unwrap_or_else(|| command.role_or_model.clone());
            self.attempts_for(&wanted)
        };
        if !on_device && attempts.is_empty() {
            return Err(MadoError::new(ErrorCode::NotFound, "error.ai.noModel"));
        }
        let id = PromptId::new();
        let prompt = PromptState::AiResult {
            id,
            window_id: window,
            tab_id: tab,
            command_id: command.id.clone(),
            name: command.name.clone(),
            text: String::new(),
            done: false,
            error: None,
            editable,
            model: None,
        };
        let mut prompts = self.state.prompts.clone();
        prompts.insert(id, prompt);
        self.state.tx(rec).set_prompts(prompts);
        let instructions = commands::SYSTEM.to_owned();
        let user = commands::prompt(
            &command,
            &Inputs {
                selection: &text,
                page: "",
                url: &url,
                title: &title,
            },
        );
        self.page_runs.insert(
            id,
            Run {
                tab,
                command,
                text,
                editable,
            },
        );
        let (inputs, secrets) = (self.inputs.clone(), Arc::clone(&self.ai.secrets));
        let effort = mado_agent::chat::effort(
            self.state
                .ai
                .roles
                .get("commands")
                .map_or("auto", |r| r.effort.as_str()),
        );
        tokio::task::spawn_local(async move {
            let send = |e: CommandEvent| {
                let _ = inputs.send(super::Input::Internal(Internal::Ai(AiEvent::Command(e))));
            };
            if on_device {
                let result = tokio::task::spawn_blocking(move || {
                    mado_platform::ondevice::respond(&instructions, &user)
                })
                .await;
                let (text, error) = match result {
                    Ok(Ok(t)) => (t.trim().to_owned(), None),
                    _ => (String::new(), Some("onDevice".to_owned())),
                };
                send(CommandEvent::Done {
                    prompt: id,
                    text,
                    model: Some("onDevice".into()),
                    error,
                });
                return;
            }
            let mut last = None;
            for at in &attempts {
                let req = mado_ai::ModelRequest {
                    model: at.model.clone(),
                    system: instructions.clone(),
                    messages: vec![mado_ai::Message::user(user.clone())],
                    tools: Vec::new(),
                    effort,
                    max_output_tokens: 4096,
                    web_search: false,
                };
                match answer(&secrets, at, &req, |d| {
                    send(CommandEvent::Delta {
                        prompt: id,
                        delta: d.to_owned(),
                    });
                })
                .await
                {
                    Ok(turn) => {
                        send(CommandEvent::Done {
                            prompt: id,
                            text: turn.text.trim().to_owned(),
                            model: Some(format!("{}:{}", at.connection, at.model)),
                            error: None,
                        });
                        return;
                    }
                    Err(e) => {
                        let moves = matches!(e, mado_ai::ClientError::Failed(f) if f.fails_over());
                        last = Some(e.failure().key().to_owned());
                        if !moves {
                            break;
                        }
                    }
                }
            }
            send(CommandEvent::Done {
                prompt: id,
                text: String::new(),
                model: None,
                error: last,
            });
        });
        Ok(id)
    }

    /// A role (`commands`, `fast`…) or a `<connectionId>:<modelId>`.
    pub(super) fn attempts_for(&self, wanted: &str) -> Vec<Attempt> {
        if mado_lists::ROLES.contains(&wanted) {
            return self.ai_attempts(wanted);
        }
        let Some((connection, model)) = wanted.split_once(':') else {
            return self.ai_attempts("commands");
        };
        let catalog = mado_lists::models();
        self.state
            .ai
            .connections
            .get(connection)
            .filter(|c| c.status == mado_types::state::AiConnectionStatus::Ready)
            .and_then(|c| {
                let provider = mado_ai::Provider::from_key(&c.provider)?;
                let info = mado_ai::catalog::known(&catalog, &c.provider, model);
                Some(Attempt {
                    connection: connection.to_owned(),
                    label: c.label.clone(),
                    provider,
                    base: c.base_url.clone(),
                    model: model.to_owned(),
                    web_search: false,
                    vision: info.is_none_or(|i| i.vision),
                })
            })
            .into_iter()
            .collect()
    }

    fn respond_page_command(
        &mut self,
        id: PromptId,
        action: &str,
        model: Option<String>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let Some(PromptState::AiResult { text, done, .. }) = self.state.prompts.get(&id).cloned()
        else {
            return Err(MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"));
        };
        let run = self.page_runs.get(&id).cloned();
        let close = |core: &mut Self, rec: &mut PatchRecorder| {
            let mut prompts = core.state.prompts.clone();
            prompts.remove(&id);
            core.state.tx(rec).set_prompts(prompts);
            core.page_runs.remove(&id);
        };
        match action {
            "copy" => self.ui.copy_text(&text),
            "replace" | "insert" => {
                let run = run
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"))?;
                if !run.editable || !done || text.is_empty() {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.ai.notEditable",
                    ));
                }
                self.outbox.push(EngineOp::Apply(
                    run.tab,
                    TabCommand::PostToPage {
                        world: World::Mado,
                        message: json!({ "type": "aiReplace", "text": text, "mode": action }),
                    },
                ));
                close(self, rec);
            }
            "retry" => {
                let run = run
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"))?;
                close(self, rec);
                let next = self.run_page_command(
                    run.tab,
                    &run.command.id,
                    run.text,
                    run.editable,
                    model,
                    rec,
                )?;
                return Ok(json!({ "promptId": next }));
            }
            "close" => close(self, rec),
            _ => {
                return Err(MadoError::new(
                    ErrorCode::InvalidInput,
                    "error.ai.unknownAction",
                ));
            }
        }
        Ok(Value::Null)
    }

    pub(super) fn command_event(&mut self, event: CommandEvent, rec: &mut PatchRecorder) {
        let update = |core: &mut Self,
                      rec: &mut PatchRecorder,
                      id: PromptId,
                      f: &dyn Fn(&mut PromptState)| {
            let mut prompts = core.state.prompts.clone();
            if let Some(p) = prompts.get_mut(&id) {
                f(p);
                core.state.tx(rec).set_prompts(prompts);
            }
        };
        match event {
            CommandEvent::FromMenu {
                tab,
                command,
                text,
                editable,
            } => {
                if let Err(e) = self.run_page_command(tab, &command, text, editable, None, rec) {
                    tracing::info!(key = %e.message_key, "a page command did not start");
                    let notice = mado_types::state::Notice {
                        id: format!("n{}", self.next_notice),
                        message_key: e.message_key,
                        args: Default::default(),
                        action: None,
                        undo: None,
                        expires_at: None,
                    };
                    self.next_notice += 1;
                    self.add_notice(notice, rec);
                }
            }
            CommandEvent::Delta { prompt, delta } => update(self, rec, prompt, &|p| {
                if let PromptState::AiResult { text, .. } = p {
                    text.push_str(&delta);
                }
            }),
            CommandEvent::Done {
                prompt,
                text: answer,
                model: m,
                error: e,
            } => update(self, rec, prompt, &|p| {
                if let PromptState::AiResult {
                    text,
                    done,
                    error,
                    model,
                    ..
                } = p
                {
                    text.clone_from(&answer);
                    *done = true;
                    error.clone_from(&e);
                    model.clone_from(&m);
                }
            }),
        }
    }
}
