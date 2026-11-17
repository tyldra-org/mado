//! Provider clients (spec 13 §3): one request type, one event stream, and
//! each provider's wire format behind them. Requests carry no cookies; a
//! request is retried only before its first byte.

use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::connection::{Api, Failure, Provider, base_url};
use crate::models::RemoteModel;

/// Who said a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
}

/// One piece of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Part {
    Text {
        text: String,
    },
    /// Base64 image data.
    #[serde(rename_all = "camelCase")]
    Image {
        media_type: String,
        data: String,
    },
    /// A tool call the model made; `extra` is provider data that must go
    /// back with it (Gemini's `thoughtSignature`).
    ToolCall {
        id: String,
        name: String,
        args: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra: Option<Value>,
    },
    /// A tool's answer to call `id`.
    #[serde(rename_all = "camelCase")]
    ToolResult {
        id: String,
        name: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// The model's reasoning as the provider returned it (Anthropic
    /// thinking blocks, OpenAI reasoning items). It goes back unchanged in
    /// the next request of the same conversation, to the same provider.
    Reasoning {
        data: Value,
    },
}

impl Part {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::text(text)],
        }
    }
}

/// A tool the model may call; `parameters` is a JSON Schema object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// How hard the model thinks (13 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Effort {
    #[default]
    Auto,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    /// Anthropic's thinking budget, in tokens.
    pub(crate) fn anthropic_budget(self) -> Option<u32> {
        Some(match self {
            Self::Auto => return None,
            Self::Low => 2_048,
            Self::Medium => 8_192,
            Self::High => 16_384,
            Self::Xhigh => 32_768,
            Self::Max => 63_999,
        })
    }

    /// Gemini's thinking budget, in tokens.
    pub(crate) fn gemini_budget(self) -> Option<u32> {
        Some(match self {
            Self::Auto => return None,
            Self::Low => 1_024,
            Self::Medium => 8_192,
            Self::High => 24_576,
            Self::Xhigh | Self::Max => 32_768,
        })
    }

    /// OpenAI's `reasoning.effort` (and OpenRouter's).
    pub(crate) fn openai(self) -> Option<&'static str> {
        Some(match self {
            Self::Auto => return None,
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High | Self::Xhigh | Self::Max => "high",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub effort: Effort,
    pub max_output_tokens: u32,
    /// Let the provider search the web with its own tool (13 §5); only for
    /// models the catalogue marks `webSearch`.
    #[serde(default)]
    pub web_search: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// The provider's safety system stopped the answer.
    Refusal,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelEvent {
    TextDelta {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        args: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra: Option<Value>,
    },
    /// Reasoning to keep in the assistant message (see `Part::Reasoning`).
    Reasoning {
        data: Value,
    },
    Usage {
        input: u64,
        output: u64,
        cached: u64,
    },
    /// A web page the answer cites (web search).
    Source {
        url: String,
        title: String,
    },
    Stop {
        reason: StopReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// The request failed before any answer (after retries).
    #[error("the request failed: {}", .0.key())]
    Failed(Failure),
    /// The answer broke off or the provider reported an error in it.
    #[error("the answer broke off: {0}")]
    Stream(String),
}

impl ClientError {
    pub fn failure(&self) -> Failure {
        match self {
            Self::Failed(f) => *f,
            Self::Stream(_) => Failure::Network,
        }
    }
}

/// A connection's client.
#[derive(Debug, Clone)]
pub struct ModelClient {
    provider: Provider,
    api: Api,
    base: Url,
    secret: Option<String>,
    http: reqwest::Client,
    /// The waits before the two retries (13 §3: 1 s and 4 s).
    pub backoff: [Duration; 2],
}

/// The longest `retry-after` Mado honours before giving up on a retry.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

impl ModelClient {
    pub fn new(
        provider: Provider,
        base: Option<&str>,
        secret: Option<String>,
    ) -> Result<Self, Failure> {
        let api = provider.api();
        if api == Api::None {
            return Err(Failure::Invalid);
        }
        let base = base_url(provider, base).map_err(|_| Failure::Invalid)?;
        let http = crate::http::client().map_err(|_| Failure::Network)?;
        Ok(Self {
            provider,
            api,
            base,
            secret: secret.filter(|s| !s.is_empty()),
            http,
            backoff: [Duration::from_secs(1), Duration::from_secs(4)],
        })
    }

    pub async fn list_models(&self) -> Result<Vec<RemoteModel>, Failure> {
        crate::models::list(&self.http, self.api, &self.base, self.secret.as_deref()).await
    }

    /// Starts a streamed answer.
    pub async fn stream(&self, req: &ModelRequest) -> Result<EventStream, ClientError> {
        let (url, body) = match self.api {
            Api::Anthropic => (
                crate::models::endpoint(&self.base, "v1/messages"),
                crate::anthropic::body(req),
            ),
            Api::OpenAiResponses => (
                crate::models::endpoint(&self.base, "responses"),
                crate::openai::body(req),
            ),
            Api::Gemini => (
                crate::models::endpoint(
                    &self.base,
                    &format!("v1beta/models/{}:streamGenerateContent?alt=sse", req.model),
                ),
                crate::gemini::body(req),
            ),
            Api::OpenAiCompatible => (
                crate::models::endpoint(&self.base, "chat/completions"),
                crate::compat::body(req, self.provider),
            ),
            Api::None => return Err(ClientError::Failed(Failure::Invalid)),
        };
        let response = self.send(&url, &body).await?;
        Ok(EventStream {
            response,
            buf: Vec::new(),
            parser: Parser::new(self.api),
            pending: VecDeque::new(),
            done: false,
        })
    }

    fn request(&self, url: &str, body: &Value) -> reqwest::RequestBuilder {
        let r = self
            .http
            .post(url)
            .header("accept", "text/event-stream")
            .json(body);
        let secret = self.secret.as_deref().unwrap_or_default();
        match self.api {
            Api::Anthropic => r
                .header("x-api-key", secret)
                .header("anthropic-version", "2023-06-01"),
            Api::Gemini => r.header("x-goog-api-key", secret),
            _ if secret.is_empty() => r,
            _ => r.bearer_auth(secret),
        }
    }

    /// Sends, retrying twice before the first byte on network errors,
    /// rate limits and server errors (13 §3).
    async fn send(&self, url: &str, body: &Value) -> Result<reqwest::Response, ClientError> {
        let mut attempt = 0;
        loop {
            let (failure, retry_after) = match self.request(url, body).send().await {
                Ok(r) if r.status().is_success() => return Ok(r),
                Ok(r) => {
                    let status = r.status().as_u16();
                    let after = r
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map(Duration::from_secs);
                    let text = r.text().await.unwrap_or_default();
                    tracing::debug!(status, "a model request failed");
                    let failure = Failure::from_status(status, &text);
                    let retryable = failure == Failure::RateLimited || status >= 500;
                    if !retryable {
                        return Err(ClientError::Failed(failure));
                    }
                    (
                        if status >= 500 {
                            Failure::Network
                        } else {
                            failure
                        },
                        after,
                    )
                }
                Err(e) => {
                    tracing::debug!(error = %e, "a model request did not connect");
                    (Failure::Network, None)
                }
            };
            let Some(wait) = self.backoff.get(attempt).copied() else {
                return Err(ClientError::Failed(failure));
            };
            let wait = match retry_after {
                Some(after) if after > MAX_RETRY_AFTER => return Err(ClientError::Failed(failure)),
                Some(after) => after.max(wait),
                None => wait,
            };
            tokio::time::sleep(wait).await;
            attempt += 1;
        }
    }
}

/// A streamed answer. `next` yields events until `Stop`, then `None`.
#[derive(Debug)]
pub struct EventStream {
    response: reqwest::Response,
    buf: Vec<u8>,
    parser: Parser,
    pending: VecDeque<ModelEvent>,
    done: bool,
}

impl EventStream {
    pub async fn next(&mut self) -> Option<Result<ModelEvent, ClientError>> {
        loop {
            if let Some(e) = self.pending.pop_front() {
                if matches!(e, ModelEvent::Stop { .. }) {
                    self.done = true;
                }
                return Some(Ok(e));
            }
            if self.done {
                return None;
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => {
                    self.buf.extend_from_slice(&chunk);
                    if let Err(e) = self.drain(false) {
                        self.done = true;
                        return Some(Err(e));
                    }
                }
                Ok(None) => {
                    if let Err(e) = self.drain(true) {
                        self.done = true;
                        return Some(Err(e));
                    }
                    self.parser.finish(&mut self.pending);
                    if self.pending.is_empty() {
                        self.done = true;
                        return Some(Err(ClientError::Stream("the answer ended early".into())));
                    }
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(ClientError::Stream(e.to_string())));
                }
            }
        }
    }

    /// Reads every complete server-sent event in the buffer.
    fn drain(&mut self, end: bool) -> Result<(), ClientError> {
        loop {
            let text = String::from_utf8_lossy(&self.buf).replace("\r\n", "\n");
            let Some(at) = text
                .find("\n\n")
                .or_else(|| (end && !text.trim().is_empty()).then_some(text.len()))
            else {
                return Ok(());
            };
            let block = text[..at].to_owned();
            let rest = text.get(at + 2..).unwrap_or("").to_owned();
            self.buf = rest.into_bytes();
            let mut event = None;
            let mut data = Vec::new();
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    event = Some(v.trim().to_owned());
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push(v.strip_prefix(' ').unwrap_or(v).to_owned());
                }
            }
            if !data.is_empty() {
                self.parser
                    .event(event.as_deref(), &data.join("\n"), &mut self.pending)
                    .map_err(ClientError::Stream)?;
            }
            if self.buf.is_empty() {
                return Ok(());
            }
        }
    }
}

/// Each API's stream state.
#[derive(Debug)]
enum Parser {
    Anthropic(crate::anthropic::State),
    OpenAi(crate::openai::State),
    Gemini(crate::gemini::State),
    Compat(crate::compat::State),
}

impl Parser {
    fn new(api: Api) -> Self {
        match api {
            Api::Anthropic => Self::Anthropic(Default::default()),
            Api::OpenAiResponses => Self::OpenAi(Default::default()),
            Api::Gemini => Self::Gemini(Default::default()),
            _ => Self::Compat(Default::default()),
        }
    }

    fn event(
        &mut self,
        name: Option<&str>,
        data: &str,
        out: &mut VecDeque<ModelEvent>,
    ) -> Result<(), String> {
        if data == "[DONE]" {
            if let Self::Compat(s) = self {
                s.finish(out);
            }
            return Ok(());
        }
        let value: Value = serde_json::from_str(data).map_err(|e| format!("not JSON: {e}"))?;
        match self {
            Self::Anthropic(s) => s.event(name, &value, out),
            Self::OpenAi(s) => s.event(&value, out),
            Self::Gemini(s) => s.event(&value, out),
            Self::Compat(s) => s.event(&value, out),
        }
    }

    /// The stream ended without its own end marker.
    fn finish(&mut self, out: &mut VecDeque<ModelEvent>) {
        match self {
            Self::Gemini(s) => s.finish(out),
            Self::Compat(s) => s.finish(out),
            _ => {}
        }
    }
}

/// A whole answer, read from a stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// The assistant message to put back in the conversation: reasoning,
    /// text and tool calls in the order they came.
    pub message: Message,
    pub text: String,
    /// `(input, output, cached)` tokens.
    pub usage: (u64, u64, u64),
    pub stop: StopReason,
    /// Web pages the answer cites, `(url, title)`, each once, in order.
    pub sources: Vec<(String, String)>,
}

impl Turn {
    /// The tool calls the model made, `(id, name, args)`.
    pub fn tool_calls(&self) -> Vec<(&str, &str, &Value)> {
        self.message
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall { id, name, args, .. } => Some((id.as_str(), name.as_str(), args)),
                _ => None,
            })
            .collect()
    }
}

/// Reads a stream to its end; `on_text` sees each text delta as it comes.
pub async fn collect(
    stream: &mut EventStream,
    mut on_text: impl FnMut(&str),
) -> Result<Turn, ClientError> {
    let mut parts: Vec<Part> = Vec::new();
    let mut text = String::new();
    let mut usage = (0, 0, 0);
    let mut stop = StopReason::Other;
    let mut sources: Vec<(String, String)> = Vec::new();
    while let Some(event) = stream.next().await {
        match event? {
            ModelEvent::TextDelta { text: t } => {
                on_text(&t);
                text.push_str(&t);
                match parts.last_mut() {
                    Some(Part::Text { text: last }) => last.push_str(&t),
                    _ => parts.push(Part::text(t)),
                }
            }
            ModelEvent::ToolCall {
                id,
                name,
                args,
                extra,
            } => parts.push(Part::ToolCall {
                id,
                name,
                args,
                extra,
            }),
            ModelEvent::Reasoning { data } => parts.push(Part::Reasoning { data }),
            ModelEvent::Usage {
                input,
                output,
                cached,
            } => usage = (input, output, cached),
            ModelEvent::Source { url, title } => {
                if !sources.iter().any(|(u, _)| *u == url) {
                    sources.push((url, title));
                }
            }
            ModelEvent::Stop { reason } => stop = reason,
        }
    }
    Ok(Turn {
        message: Message {
            role: Role::Assistant,
            parts,
        },
        text,
        usage,
        stop,
        sources,
    })
}

/// Arguments as JSON, from a string the model wrote.
pub(crate) fn parse_args(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}
