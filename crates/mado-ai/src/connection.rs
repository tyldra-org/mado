//! Connections (spec 13 §2): who Mado talks to, where, and what a failure
//! means for the next request.

use serde::{Deserialize, Serialize};
use url::Url;

/// How Mado reaches a provider (13 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    ApiKey,
    Oauth,
    Local,
    SubscriptionCli,
    OnDevice,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::ApiKey => "apiKey",
            Self::Oauth => "oauth",
            Self::Local => "local",
            Self::SubscriptionCli => "subscriptionCli",
            Self::OnDevice => "onDevice",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [
            Self::ApiKey,
            Self::Oauth,
            Self::Local,
            Self::SubscriptionCli,
            Self::OnDevice,
        ]
        .into_iter()
        .find(|k| k.key() == key)
    }
}

/// The wire format a provider speaks (13 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    Anthropic,
    OpenAiResponses,
    Gemini,
    OpenAiCompatible,
    /// Not HTTP: ACP over stdio (13 §2.3) or the on-device helper.
    None,
}

/// Every provider of 13 §2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Anthropic,
    Openai,
    Google,
    Xai,
    Mistral,
    Deepseek,
    Groq,
    Openrouter,
    Cloudflare,
    /// A custom OpenAI-compatible server.
    OpenaiCompatible,
    Ollama,
    LmStudio,
    ClaudeCode,
    Codex,
    GeminiCli,
    CopilotCli,
    CursorCli,
    Apple,
}

impl Provider {
    pub const ALL: [Self; 18] = [
        Self::Anthropic,
        Self::Openai,
        Self::Google,
        Self::Xai,
        Self::Mistral,
        Self::Deepseek,
        Self::Groq,
        Self::Openrouter,
        Self::Cloudflare,
        Self::OpenaiCompatible,
        Self::Ollama,
        Self::LmStudio,
        Self::ClaudeCode,
        Self::Codex,
        Self::GeminiCli,
        Self::CopilotCli,
        Self::CursorCli,
        Self::Apple,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Google => "google",
            Self::Xai => "xai",
            Self::Mistral => "mistral",
            Self::Deepseek => "deepseek",
            Self::Groq => "groq",
            Self::Openrouter => "openrouter",
            Self::Cloudflare => "cloudflare",
            Self::OpenaiCompatible => "openaiCompatible",
            Self::Ollama => "ollama",
            Self::LmStudio => "lmStudio",
            Self::ClaudeCode => "claudeCode",
            Self::Codex => "codex",
            Self::GeminiCli => "geminiCli",
            Self::CopilotCli => "copilotCli",
            Self::CursorCli => "cursorCli",
            Self::Apple => "apple",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    /// The name people know it by, for labels.
    pub fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::Openai => "OpenAI",
            Self::Google => "Google Gemini",
            Self::Xai => "xAI",
            Self::Mistral => "Mistral",
            Self::Deepseek => "DeepSeek",
            Self::Groq => "Groq",
            Self::Openrouter => "OpenRouter",
            Self::Cloudflare => "Cloudflare AI Gateway",
            Self::OpenaiCompatible => "OpenAI-compatible server",
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::GeminiCli => "Gemini CLI",
            Self::CopilotCli => "Copilot CLI",
            Self::CursorCli => "Cursor CLI",
            Self::Apple => "Apple Intelligence",
        }
    }

    /// The connection kinds this provider takes (13 §2.1).
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Self::Openrouter => &[Kind::ApiKey, Kind::Oauth],
            Self::Ollama | Self::LmStudio => &[Kind::Local],
            Self::OpenaiCompatible => &[Kind::ApiKey, Kind::Local],
            Self::ClaudeCode
            | Self::Codex
            | Self::GeminiCli
            | Self::CopilotCli
            | Self::CursorCli => &[Kind::SubscriptionCli],
            Self::Apple => &[Kind::OnDevice],
            _ => &[Kind::ApiKey],
        }
    }

    pub fn api(self) -> Api {
        match self {
            Self::Anthropic => Api::Anthropic,
            Self::Openai => Api::OpenAiResponses,
            Self::Google => Api::Gemini,
            Self::Xai
            | Self::Mistral
            | Self::Deepseek
            | Self::Groq
            | Self::Openrouter
            | Self::Cloudflare
            | Self::OpenaiCompatible
            | Self::Ollama
            | Self::LmStudio => Api::OpenAiCompatible,
            _ => Api::None,
        }
    }

    /// The base URL when the connection names none (13 §3).
    pub fn default_base(self) -> Option<&'static str> {
        Some(match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::Openai => "https://api.openai.com/v1",
            Self::Google => "https://generativelanguage.googleapis.com",
            Self::Xai => "https://api.x.ai/v1",
            Self::Mistral => "https://api.mistral.ai/v1",
            Self::Deepseek => "https://api.deepseek.com",
            Self::Groq => "https://api.groq.com/openai/v1",
            Self::Openrouter => "https://openrouter.ai/api/v1",
            Self::Ollama => "http://localhost:11434/v1",
            Self::LmStudio => "http://localhost:1234/v1",
            _ => return None,
        })
    }

    /// Whether the connection needs a secret to work.
    pub fn needs_secret(self, kind: Kind) -> bool {
        matches!(kind, Kind::ApiKey | Kind::Oauth)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionError {
    #[error("{0} does not connect as {1}")]
    WrongKind(&'static str, &'static str),
    #[error("the server address is missing")]
    NoBaseUrl,
    #[error("the server address is not a valid http or https URL")]
    BadUrl,
    /// 13 §2.2: remote servers must use https.
    #[error("a server that is not on this Mac must use https")]
    NotHttps,
    #[error("an API key is needed")]
    NoSecret,
}

impl ConnectionError {
    /// The `error.ai.*` key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::WrongKind(..) => "error.ai.wrongKind",
            Self::NoBaseUrl => "error.ai.noBaseUrl",
            Self::BadUrl => "error.ai.badUrl",
            Self::NotHttps => "error.ai.notHttps",
            Self::NoSecret => "error.ai.noSecret",
        }
    }
}

/// Whether `host` is this Mac (13 §2.2).
pub fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// The base URL a connection talks to: its own, else the provider's. Any
/// URL that is not on this Mac must be https (13 §2.2).
pub fn base_url(provider: Provider, base: Option<&str>) -> Result<Url, ConnectionError> {
    let raw = base
        .filter(|b| !b.trim().is_empty())
        .or(provider.default_base())
        .ok_or(ConnectionError::NoBaseUrl)?;
    let url = Url::parse(raw.trim()).map_err(|_| ConnectionError::BadUrl)?;
    let host = url.host_str().ok_or(ConnectionError::BadUrl)?;
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(host) => Ok(url),
        "http" => Err(ConnectionError::NotHttps),
        _ => Err(ConnectionError::BadUrl),
    }
}

/// Checks a new connection's kind and address before it is stored.
pub fn validate(
    provider: Provider,
    kind: Kind,
    base: Option<&str>,
    has_secret: bool,
) -> Result<(), ConnectionError> {
    if !provider.kinds().contains(&kind) {
        return Err(ConnectionError::WrongKind(provider.name(), kind.key()));
    }
    if provider.api() != Api::None {
        base_url(provider, base)?;
    }
    if kind == Kind::ApiKey && !has_secret {
        return Err(ConnectionError::NoSecret);
    }
    Ok(())
}

/// Why a request failed, as stored in a connection's status (13 §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Failure {
    Unauthorized,
    Network,
    NotFound,
    RateLimited,
    QuotaExceeded,
    /// The server answered something Mado does not understand.
    Invalid,
}

impl Failure {
    pub fn key(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Network => "network",
            Self::NotFound => "notFound",
            Self::RateLimited => "rateLimited",
            Self::QuotaExceeded => "quotaExceeded",
            Self::Invalid => "invalid",
        }
    }

    /// An HTTP error status and its body. Quota answers come as 429 with
    /// a billing message, or 402 (OpenRouter), so the body decides.
    pub fn from_status(status: u16, body: &str) -> Self {
        let quota = || {
            let b = body.to_ascii_lowercase();
            ["insufficient_quota", "quota", "credit", "billing"]
                .iter()
                .any(|w| b.contains(w))
        };
        match status {
            401 | 403 => Self::Unauthorized,
            402 => Self::QuotaExceeded,
            404 => Self::NotFound,
            429 if quota() => Self::QuotaExceeded,
            429 => Self::RateLimited,
            _ => Self::Invalid,
        }
    }

    /// 13 §2.2: these move a request to the next connection.
    pub fn fails_over(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::QuotaExceeded | Self::Unauthorized
        )
    }
}

/// What failover needs to know about a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub id: &'a str,
    pub priority: i64,
    pub ready: bool,
    /// Model IDs it serves.
    pub models: &'a [String],
}

/// 13 §2.2: after `failure` on connection `from` for `model`, the next
/// ready connection by priority that serves the same model, if the failure
/// moves on at all. `tried` are the connections already used for this
/// request.
pub fn failover<'a>(
    candidates: &[Candidate<'a>],
    model: &str,
    failure: Failure,
    tried: &[&str],
) -> Option<&'a str> {
    if !failure.fails_over() {
        return None;
    }
    let mut order: Vec<&Candidate<'a>> = candidates
        .iter()
        .filter(|c| c.ready && !tried.contains(&c.id) && c.models.iter().any(|m| m == model))
        .collect();
    order.sort_by_key(|c| (c.priority, c.id));
    order.first().map(|c| c.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_servers_must_use_https() {
        assert_eq!(
            base_url(Provider::OpenaiCompatible, Some("http://example.com/v1")),
            Err(ConnectionError::NotHttps)
        );
        for ok in [
            "http://localhost:8080/v1",
            "http://127.0.0.1:1/v1",
            "http://[::1]:9/v1",
            "https://example.com/v1",
        ] {
            assert!(
                base_url(Provider::OpenaiCompatible, Some(ok)).is_ok(),
                "{ok}"
            );
        }
        assert_eq!(
            base_url(Provider::OpenaiCompatible, None),
            Err(ConnectionError::NoBaseUrl)
        );
        assert_eq!(
            base_url(Provider::Anthropic, None).map(|u| u.to_string()),
            Ok("https://api.anthropic.com/".to_owned())
        );
        assert_eq!(
            base_url(Provider::Openai, Some("ftp://x")),
            Err(ConnectionError::BadUrl)
        );
    }

    #[test]
    fn a_connection_needs_the_right_kind_and_a_key() {
        assert!(validate(Provider::Anthropic, Kind::ApiKey, None, true).is_ok());
        assert_eq!(
            validate(Provider::Anthropic, Kind::ApiKey, None, false),
            Err(ConnectionError::NoSecret)
        );
        assert!(matches!(
            validate(Provider::Anthropic, Kind::Oauth, None, false),
            Err(ConnectionError::WrongKind(..))
        ));
        assert!(validate(Provider::Openrouter, Kind::Oauth, None, false).is_ok());
        assert!(validate(Provider::Ollama, Kind::Local, None, false).is_ok());
        assert!(validate(Provider::ClaudeCode, Kind::SubscriptionCli, None, false).is_ok());
        for p in Provider::ALL {
            assert_eq!(Provider::from_key(p.key()), Some(p));
        }
    }

    #[test]
    fn errors_are_classified_from_status_and_body() {
        assert_eq!(Failure::from_status(401, ""), Failure::Unauthorized);
        assert_eq!(Failure::from_status(429, "slow down"), Failure::RateLimited);
        assert_eq!(
            Failure::from_status(429, r#"{"error":{"code":"insufficient_quota"}}"#),
            Failure::QuotaExceeded
        );
        assert_eq!(Failure::from_status(402, ""), Failure::QuotaExceeded);
        assert_eq!(Failure::from_status(404, ""), Failure::NotFound);
        assert_eq!(Failure::from_status(500, ""), Failure::Invalid);
    }

    #[test]
    fn failover_takes_the_next_connection_that_serves_the_model() {
        let both = vec!["m".to_owned()];
        let other = vec!["x".to_owned()];
        let c = [
            Candidate {
                id: "a",
                priority: 0,
                ready: true,
                models: &both,
            },
            Candidate {
                id: "b",
                priority: 2,
                ready: true,
                models: &both,
            },
            Candidate {
                id: "c",
                priority: 1,
                ready: true,
                models: &other,
            },
            Candidate {
                id: "d",
                priority: 1,
                ready: false,
                models: &both,
            },
            Candidate {
                id: "e",
                priority: 3,
                ready: true,
                models: &both,
            },
        ];
        assert_eq!(failover(&c, "m", Failure::RateLimited, &["a"]), Some("b"));
        assert_eq!(
            failover(&c, "m", Failure::QuotaExceeded, &["a", "b"]),
            Some("e")
        );
        assert_eq!(
            failover(&c, "m", Failure::Unauthorized, &["a", "b", "e"]),
            None
        );
        assert_eq!(
            failover(&c, "m", Failure::Network, &["a"]),
            None,
            "network errors stay"
        );
    }
}
