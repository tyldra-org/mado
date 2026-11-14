//! The wire error (spec 00, error model). Each crate has its own `thiserror`
//! enum and converts it to `MadoError` at the wire.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// What went wrong, as a closed set the UI and CLI can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    PermissionDenied,
    /// A protected setting or command, asked for by a control client or an agent.
    Protected,
    EngineUnavailable,
    NetworkFailed,
    Conflict,
    Locked,
    Unsupported,
    Internal,
}

/// An error as the UI, overlay, CLI and MCP see it. `message_key` is an
/// `en.json` key; Rust never sends sentences (spec 00, conventions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MadoError {
    pub code: ErrorCode,
    pub message_key: String,
    #[ts(type = "Record<string, unknown>")]
    pub args: BTreeMap<String, Value>,
    pub retryable: bool,
}

impl MadoError {
    pub fn new(code: ErrorCode, message_key: impl Into<String>) -> Self {
        Self {
            code,
            message_key: message_key.into(),
            args: BTreeMap::new(),
            retryable: false,
        }
    }

    pub fn arg(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.args.insert(name.into(), value.into());
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    /// A bug: the core caught a panic or broke an invariant.
    pub fn internal() -> Self {
        Self::new(ErrorCode::Internal, "error.internal")
    }
}

impl fmt::Display for MadoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message_key)
    }
}

impl std::error::Error for MadoError {}
