//! Chat in the core (spec 13 §5): reading the attached pages, choosing a
//! connection for the `chat` role, streaming the answer to the UI on
//! `mado://ai` and keeping the chat.

use std::sync::Arc;

use mado_agent::chat::{self as agent_chat, PageContext};
use mado_agent::untrusted;
use mado_ai::connection::Provider;
use mado_ai::secrets::Secrets;
use mado_ai::{ClientError, ModelClient};
use mado_engine::{EngineOp, TabCommand, World};
use mado_types::command::{ChatContext, Command, Origin};
use mado_types::state::AiConnectionStatus;
use mado_types::{ErrorCode, MadoError, TabId};
use rusqlite::params;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::ai::AiEvent;
use super::{Core, Internal, Reply};

/// `ai.chatRetention = week` keeps a chat this long after its last message.
const WEEK_MS: i64 = 7 * 24 * 3600 * 1000;
/// The daily retention job.
pub(super) const RETENTION_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// One way to answer: a model on a connection (13 §2.2 failover).
#[derive(Debug, Clone)]
pub(super) struct Attempt {
    pub connection: String,
    pub label: String,
    pub provider: Provider,
    pub base: Option<String>,
    pub model: String,
    pub web_search: bool,
    pub vision: bool,
}

/// What a chat job reports back.
#[derive(Debug)]
pub enum ChatEvent {
    Delta {
        chat: String,
        message: String,
        delta: String,
    },
    Note {
        chat: String,
        message: String,
        note: Value,
    },
    Done {
        chat: String,
        message: String,
        /// The user's message as sent, with its pages.
        user_text: String,
        text: String,
        model: Option<String>,
        sources: Vec<(String, String)>,
        notes: Vec<Value>,
        error: Option<String>,
    },
    Title {
        chat: String,
        title: String,
    },
}

fn chat_error(key: &str) -> MadoError {
    MadoError::new(ErrorCode::InvalidInput, format!("error.ai.{key}"))
}

fn new_id() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}

/// A stored message's content: `{ text, sources?, notes? }`.
fn stored(text: &str, sources: &[(String, String)], notes: &[Value]) -> String {
    let sources: Vec<Value> = sources
        .iter()
        .map(|(url, title)| json!({ "url": url, "title": title }))
        .collect();
    json!({ "text": text, "sources": sources, "notes": notes }).to_string()
}

impl Core {
    /// The connections and models that may answer `role`, in order: the
    /// role's model, the same model on other ready connections by
    /// priority, then the role's fallback (13 §2.2, §4).
    pub(super) fn ai_attempts(&self, role: &str) -> Vec<Attempt> {
        let Some(resolved) = self
            .state
            .ai
            .roles
            .get(role)
            .and_then(|r| r.resolved.clone())
        else {
            return Vec::new();
        };
        let catalog = mado_lists::models();
        let available = self.ai_available();
        let attempt = |connection: &str, model: &str| -> Option<Attempt> {
            let c = self.state.ai.connections.get(connection)?;
            if c.status != AiConnectionStatus::Ready {
                return None;
            }
            let provider = Provider::from_key(&c.provider)?;
            let info = mado_ai::catalog::known(&catalog, &c.provider, model);
            Some(Attempt {
                connection: connection.to_owned(),
                label: c.label.clone(),
                provider,
                base: c.base_url.clone(),
                model: model.to_owned(),
                web_search: info.is_some_and(|i| i.web_search),
                vision: info.is_none_or(|i| i.vision),
            })
        };
        let mut out = Vec::new();
        let mut add = |r: &str| {
            if let Some((c, m)) = r.split_once(':') {
                let mut others: Vec<&mado_ai::catalog::Available> = available
                    .iter()
                    .filter(|a| a.ready && a.model == m)
                    .collect();
                others.sort_by_key(|a| (a.connection != c, a.priority));
                for a in others {
                    if !out
                        .iter()
                        .any(|x: &Attempt| x.connection == a.connection && x.model == a.model)
                        && let Some(at) = attempt(&a.connection, &a.model)
                    {
                        out.push(at);
                    }
                }
            }
        };
        add(&resolved);
        if let Some(f) = self
            .state
            .ai
            .roles
            .get(role)
            .and_then(|r| r.fallback.clone())
        {
            add(&f);
        }
        out
    }

    pub(super) fn chat_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        // 13 §5: private windows have no chat.
        if self.state.profile.is_private {
            return Err(chat_error("noPrivateChat"));
        }
        match cmd {
            Command::AiChatSend {
                chat_id,
                text,
                context,
            } => {
                if !origin.is_user() {
                    return Err(MadoError::new(ErrorCode::Protected, "error.ai.notAllowed"));
                }
                self.chat_send(chat_id, text, context)
            }
            Command::AiChatList => {
                let rows = self
                    .profile_db
                    .readers()
                    .read(|c| {
                        c.prepare("SELECT id, title, updated_at FROM ai_chats ORDER BY updated_at DESC LIMIT 500")?
                            .query_map([], |r| {
                                Ok(json!({ "id": r.get::<_, String>(0)?, "title": r.get::<_, Option<String>>(1)?, "updatedAt": r.get::<_, i64>(2)? }))
                            })?
                            .collect::<rusqlite::Result<Vec<Value>>>()
                    })
                    .map_err(|_| MadoError::internal())?;
                Ok(Value::Array(rows))
            }
            Command::AiChatGet { chat_id } => self.chat_get(&chat_id),
            Command::AiChatDelete { chat_id } => {
                if let Some(job) = self.chat_jobs.remove(&chat_id) {
                    job.abort();
                }
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute("DELETE FROM ai_chats WHERE id = ?1", [chat_id])
                        .map(drop)
                });
                if let Some(reply) = reply_to.take() {
                    tokio::task::spawn_local(async move {
                        let _ = reply.send(
                            pending
                                .await
                                .map(|()| Value::Null)
                                .map_err(|_| MadoError::internal()),
                        );
                    });
                }
                Ok(Value::Null)
            }
            Command::AiChatCancel { chat_id } => {
                if let Some(job) = self.chat_jobs.remove(&chat_id) {
                    job.abort();
                    self.ui
                        .ai(&json!({ "chatId": chat_id, "done": true, "error": "cancelled" }));
                }
                Ok(Value::Null)
            }
            _ => Err(MadoError::new(
                ErrorCode::Unsupported,
                "error.unknownCommand",
            )),
        }
    }

    fn chat_get(&self, chat_id: &str) -> Result<Value, MadoError> {
        let id = chat_id.to_owned();
        self.profile_db
            .readers()
            .read(move |c| {
                use rusqlite::OptionalExtension as _;
                let Some(title) = c
                    .query_row("SELECT title FROM ai_chats WHERE id = ?1", [&id], |r| {
                        r.get::<_, Option<String>>(0)
                    })
                    .optional()?
                else {
                    return Ok(None);
                };
                let messages = c
                    .prepare("SELECT id, role, content, model, created_at FROM ai_messages WHERE chat_id = ?1 ORDER BY created_at, rowid")?
                    .query_map([&id], |r| {
                        let content: Value =
                            serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or(Value::Null);
                        Ok(json!({
                            "id": r.get::<_, String>(0)?,
                            "role": r.get::<_, String>(1)?,
                            "text": content.get("display").or(content.get("text")).cloned().unwrap_or(Value::Null),
                            "sources": content.get("sources").cloned().unwrap_or(json!([])),
                            "notes": content.get("notes").cloned().unwrap_or(json!([])),
                            "model": r.get::<_, Option<String>>(3)?,
                            "createdAt": r.get::<_, i64>(4)?,
                        }))
                    })?
                    .collect::<rusqlite::Result<Vec<Value>>>()?;
                Ok(Some(json!({ "id": id, "title": title, "messages": messages })))
            })
            .map_err(|_| MadoError::internal())?
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.ai.noChat"))
    }

    fn chat_send(
        &mut self,
        chat_id: Option<String>,
        text: String,
        context: ChatContext,
    ) -> Result<Value, MadoError> {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(chat_error("emptyMessage"));
        }
        let attempts = self.ai_attempts("chat");
        if attempts.is_empty() {
            return Err(MadoError::new(ErrorCode::NotFound, "error.ai.noModel"));
        }
        let now = self.now();
        let (chat, history) = match chat_id {
            Some(id) => {
                let id2 = id.clone();
                let history = self
                    .profile_db
                    .readers()
                    .read(move |c| {
                        let exists: bool = c.query_row(
                            "SELECT EXISTS (SELECT 1 FROM ai_chats WHERE id = ?1)",
                            [&id2],
                            |r| r.get(0),
                        )?;
                        if !exists {
                            return Ok(None);
                        }
                        c.prepare("SELECT role, content FROM ai_messages WHERE chat_id = ?1 ORDER BY created_at, rowid")?
                            .query_map([&id2], |r| {
                                let content: Value = serde_json::from_str(&r.get::<_, String>(1)?)
                                    .unwrap_or(Value::Null);
                                Ok(agent_chat::Turn {
                                    role: r.get(0)?,
                                    text: content["text"].as_str().unwrap_or_default().to_owned(),
                                })
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()
                            .map(Some)
                    })
                    .map_err(|_| MadoError::internal())?
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.ai.noChat"))?;
                (id, history)
            }
            None => {
                let id = new_id();
                let expires = self.chat_expiry(now);
                let row = id.clone();
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute(
                        "INSERT INTO ai_chats (id, title, created_at, updated_at, expires_at) VALUES (?1, NULL, ?2, ?2, ?3)",
                        params![row, now, expires],
                    )
                    .map(drop)
                });
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "a chat was not saved");
                    }
                });
                (id, Vec::new())
            }
        };
        if self.chat_jobs.contains_key(&chat) {
            return Err(MadoError::new(ErrorCode::Conflict, "error.ai.chatBusy"));
        }
        let message = new_id();
        // Pages: refused before reading when not web pages or on a
        // sensitive host (13 §9.4); read with `ai-reader` otherwise.
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
        let mut notes: Vec<Value> = Vec::new();
        let mut reads: Vec<(TabId, String, oneshot::Receiver<Option<Value>>)> = Vec::new();
        let mut tabs = context.tab_ids.clone();
        tabs.dedup();
        for tab in tabs {
            let Some(url) = self.model.tabs.get(&tab).map(|t| t.url.clone()) else {
                continue;
            };
            if let Some(reason) = untrusted::refused(&url, &sensitive, &allowed) {
                notes.push(json!({ "kind": reason, "tabId": tab, "url": url }));
                continue;
            }
            let (tx, rx) = oneshot::channel();
            self.outbox.push(EngineOp::Apply(
                tab,
                TabCommand::EvaluateInWorld {
                    world: World::Mado,
                    source: mado_engine::page::ai_text(),
                    reply: Box::new(move |r| {
                        let _ = tx.send(r.ok());
                    }),
                },
            ));
            reads.push((tab, url, rx));
        }
        let selection = context.selection.as_ref().and_then(|s| {
            let url = self.model.tabs.get(&s.tab_id)?.url.clone();
            match untrusted::refused(&url, &sensitive, &allowed) {
                Some(reason) => {
                    notes.push(json!({ "kind": reason, "tabId": s.tab_id, "url": url }));
                    None
                }
                None => Some((url, s.text.clone())),
            }
        });
        let shot = context.screenshot_tab_id.and_then(|tab| {
            let url = self.model.tabs.get(&tab)?.url.clone();
            if let Some(reason) = untrusted::refused(&url, &sensitive, &allowed) {
                notes.push(json!({ "kind": reason, "tabId": tab, "url": url }));
                return None;
            }
            let (tx, rx) = oneshot::channel();
            self.outbox.push(EngineOp::Apply(
                tab,
                TabCommand::Snapshot {
                    full_page: false,
                    reply: Box::new(move |r| {
                        let _ = tx.send(r.ok());
                    }),
                },
            ));
            Some(rx)
        });
        let effort = agent_chat::effort(
            self.state
                .ai
                .roles
                .get("chat")
                .map_or("auto", |r| r.effort.as_str()),
        );
        let fast = self.ai_attempts("fast");
        let titled = !history.is_empty();
        let today = jiff::Timestamp::from_millisecond(now)
            .map(|t| t.strftime("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let (inputs, secrets, files) = (
            self.inputs.clone(),
            Arc::clone(&self.ai.secrets),
            context.files.clone(),
        );
        let (c2, m2) = (chat.clone(), message.clone());
        let job = tokio::task::spawn_local(async move {
            let send = |e: ChatEvent| {
                let _ = inputs.send(super::Input::Internal(Internal::Ai(AiEvent::Chat(e))));
            };
            let mut pages = Vec::new();
            for (tab, url, rx) in reads {
                let v = rx.await.ok().flatten().unwrap_or(Value::Null);
                if v["sensitiveField"] == true && !untrusted::allowed(&url, &allowed) {
                    notes.push(json!({ "kind": "sensitivePage", "tabId": tab, "url": url }));
                    continue;
                }
                let body = v["text"].as_str().unwrap_or_default();
                if body.is_empty() {
                    notes.push(json!({ "kind": "emptyPage", "tabId": tab, "url": url }));
                    continue;
                }
                pages.push(PageContext {
                    url: v["url"].as_str().unwrap_or(&url).to_owned(),
                    title: v["title"].as_str().unwrap_or_default().to_owned(),
                    text: body.to_owned(),
                });
            }
            for path in files {
                let p = std::path::PathBuf::from(&path);
                let read = tokio::task::spawn_blocking(move || {
                    if !p.is_absolute() {
                        return None;
                    }
                    let bytes = std::fs::read(&p).ok()?;
                    String::from_utf8(bytes).ok()
                })
                .await
                .ok()
                .flatten();
                match read {
                    Some(t) => pages.push(PageContext {
                        url: format!("file://{path}"),
                        title: path.rsplit('/').next().unwrap_or_default().to_owned(),
                        text: t,
                    }),
                    None => notes.push(json!({ "kind": "unreadableFile", "path": path })),
                }
            }
            let image = match shot {
                Some(rx) => rx.await.ok().flatten().map(|i| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.encode(i.bytes)
                }),
                None => None,
            };
            let user_text = agent_chat::user_text(
                &text,
                &pages,
                selection.as_ref().map(|(u, s)| (u.as_str(), s.as_str())),
            );
            for n in &notes {
                send(ChatEvent::Note {
                    chat: c2.clone(),
                    message: m2.clone(),
                    note: n.clone(),
                });
            }
            let mut last_error = None;
            for (i, at) in attempts.iter().enumerate() {
                if i > 0 {
                    let note = json!({ "kind": "continuedWith", "label": at.label });
                    notes.push(note.clone());
                    send(ChatEvent::Note {
                        chat: c2.clone(),
                        message: m2.clone(),
                        note,
                    });
                }
                let shot = image.clone().filter(|_| at.vision);
                if image.is_some() && shot.is_none() && i == 0 {
                    notes.push(json!({ "kind": "noVision" }));
                }
                let req = agent_chat::request(
                    &at.model,
                    &today,
                    &history,
                    &user_text,
                    shot,
                    effort,
                    at.web_search,
                );
                let result = answer(&secrets, at, &req, |d| {
                    send(ChatEvent::Delta {
                        chat: c2.clone(),
                        message: m2.clone(),
                        delta: d.to_owned(),
                    });
                })
                .await;
                match result {
                    Ok(turn) => {
                        let model = format!("{}:{}", at.connection, at.model);
                        let answer_text = turn.text.clone();
                        send(ChatEvent::Done {
                            chat: c2.clone(),
                            message: m2.clone(),
                            user_text: user_text.clone(),
                            text: turn.text,
                            model: Some(model),
                            sources: turn.sources,
                            notes: notes.clone(),
                            error: None,
                        });
                        if !titled && let Some(f) = fast.first() {
                            let req = agent_chat::title_request(&f.model, &text, &answer_text);
                            if let Ok(t) = answer(&secrets, f, &req, |_| {}).await {
                                let title = agent_chat::clean_title(&t.text);
                                if !title.is_empty() {
                                    send(ChatEvent::Title {
                                        chat: c2.clone(),
                                        title,
                                    });
                                }
                            }
                        }
                        return;
                    }
                    Err(e) => {
                        let moves_on = matches!(e, ClientError::Failed(f) if f.fails_over());
                        last_error = Some(e.failure().key().to_owned());
                        if !moves_on {
                            break;
                        }
                    }
                }
            }
            send(ChatEvent::Done {
                chat: c2,
                message: m2,
                user_text,
                text: String::new(),
                model: None,
                sources: Vec::new(),
                notes,
                error: last_error,
            });
        });
        self.chat_jobs.insert(chat.clone(), job.abort_handle());
        Ok(json!({ "chatId": chat, "messageId": message }))
    }

    fn chat_expiry(&self, now: i64) -> Option<i64> {
        (self.setting_str("ai.chatRetention") != "forever").then_some(now + WEEK_MS)
    }

    pub(super) fn chat_event(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Delta {
                chat,
                message,
                delta,
            } => self
                .ui
                .ai(&json!({ "chatId": chat, "messageId": message, "delta": delta })),
            ChatEvent::Note {
                chat,
                message,
                note,
            } => self
                .ui
                .ai(&json!({ "chatId": chat, "messageId": message, "note": note })),
            ChatEvent::Done {
                chat,
                message,
                user_text,
                text,
                model,
                sources,
                notes,
                error,
            } => {
                self.chat_jobs.remove(&chat);
                let now = self.now();
                let expires = self.chat_expiry(now);
                let (c, m, ut, t, md, s, n) = (
                    chat.clone(),
                    message.clone(),
                    user_text,
                    text.clone(),
                    model.clone(),
                    sources.clone(),
                    notes.clone(),
                );
                let failed = error.is_some();
                let pending = self.profile_db.writer().write(move |db| {
                    db.execute(
                        "INSERT INTO ai_messages (id, chat_id, role, content, model, created_at) VALUES (?1, ?2, 'user', ?3, NULL, ?4)",
                        params![new_id(), c, json!({ "text": ut }).to_string(), now],
                    )?;
                    if !failed {
                        db.execute(
                            "INSERT INTO ai_messages (id, chat_id, role, content, model, created_at) VALUES (?1, ?2, 'assistant', ?3, ?4, ?5)",
                            params![m, c, stored(&t, &s, &n), md, now + 1],
                        )?;
                    }
                    db.execute(
                        "UPDATE ai_chats SET updated_at = ?2, expires_at = ?3 WHERE id = ?1",
                        params![c, now, expires],
                    )
                    .map(drop)
                });
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "a chat message was not saved");
                    }
                });
                let sources: Vec<Value> = sources
                    .iter()
                    .map(|(url, title)| json!({ "url": url, "title": title }))
                    .collect();
                self.ui.ai(&json!({
                    "chatId": chat,
                    "messageId": message,
                    "done": true,
                    "text": text,
                    "model": model,
                    "sources": sources,
                    "notes": notes,
                    "error": error,
                }));
            }
            ChatEvent::Title { chat, title } => {
                let (c, t) = (chat.clone(), title.clone());
                let pending = self.profile_db.writer().write(move |db| {
                    db.execute(
                        "UPDATE ai_chats SET title = ?2 WHERE id = ?1",
                        params![c, t],
                    )
                    .map(drop)
                });
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "a chat title was not saved");
                    }
                });
                self.ui.ai(&json!({ "chatId": chat, "title": title }));
            }
        }
    }

    /// 13 §5: deletes chats past their expiry; daily.
    pub(super) fn chat_retention(&mut self) {
        self.timers
            .schedule(crate::Timer::AiRetention, RETENTION_EVERY);
        if self.state.profile.is_private {
            return;
        }
        let now = self.now();
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "DELETE FROM ai_chats WHERE expires_at IS NOT NULL AND expires_at < ?1",
                [now],
            )
            .map(drop)
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "expired chats were not deleted");
            }
        });
    }

    /// `ai.chatRetention` changed: every chat's expiry follows.
    pub(super) fn chat_retention_changed(&mut self) {
        if self.state.profile.is_private {
            return;
        }
        let forever = self.setting_str("ai.chatRetention") == "forever";
        let pending = self.profile_db.writer().write(move |c| {
            if forever {
                c.execute("UPDATE ai_chats SET expires_at = NULL", [])
            } else {
                c.execute(
                    "UPDATE ai_chats SET expires_at = updated_at + ?1",
                    [WEEK_MS],
                )
            }
            .map(drop)
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "chat retention was not applied");
            }
        });
    }
}

/// One attempt: the secret from the Keychain, then the streamed answer.
pub(super) async fn answer(
    secrets: &Arc<dyn Secrets>,
    at: &Attempt,
    req: &mado_ai::ModelRequest,
    on_text: impl FnMut(&str),
) -> Result<mado_ai::Turn, ClientError> {
    let (store, id) = (Arc::clone(secrets), at.connection.clone());
    let secret = tokio::task::spawn_blocking(move || store.get(&id).ok().flatten())
        .await
        .ok()
        .flatten();
    let client =
        ModelClient::new(at.provider, at.base.as_deref(), secret).map_err(ClientError::Failed)?;
    let mut stream = client.stream(req).await?;
    mado_ai::collect(&mut stream, on_text).await
}
