//! Navigation requests, the core's decisions on them (spec 02 §3) and
//! network errors (spec 05 §6).

use serde::{Deserialize, Serialize};

use crate::tab::{Modifiers, NavKind};

/// Which phase of a navigation asks for a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavPhase {
    /// Before the request starts.
    Action {
        modifiers: Modifiers,
        /// HTTP method, upper case.
        method: String,
    },
    /// The response arrived; the engine asks whether to show or download it.
    Response {
        mime: String,
        /// `Content-Disposition: attachment`.
        attachment: bool,
        /// Whether the engine can show this MIME type.
        can_show: bool,
    },
}

/// A navigation waiting for `DecidePolicy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavRequest {
    pub url: String,
    /// The URL of the document that started it; `None` when Mado did.
    pub source_url: Option<String>,
    pub is_main_frame: bool,
    pub user_gesture: bool,
    /// Started by Mado itself (address bar, bookmark, session), not a page.
    pub initiated_by_mado: bool,
    pub kind: NavKind,
    pub phase: NavPhase,
}

/// Why a navigation leaves Mado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalReason {
    /// An external scheme: the app-link flow (spec 07 §6).
    AppLink,
    /// A per-site "Open in Safari" rule (spec 05 §11).
    EngineRule,
}

/// The core's answer to `DecidePolicy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavDecision {
    Allow,
    /// Safe default.
    Cancel,
    /// Cancel here and hand `url` to the system.
    OpenExternal {
        url: String,
        reason: ExternalReason,
    },
    /// Load `url` instead (HTTPS-first upgrade).
    Redirect {
        url: String,
    },
    /// Turn the response into a download.
    Download,
}

/// Engine errors as error-page kinds (spec 05 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NetError {
    Offline,
    DnsFailed,
    TimedOut,
    Refused,
    Reset,
    Certificate,
    UnsupportedScheme,
    TooManyRedirects,
    /// The navigation was cancelled (a new one replaced it, or Stop).
    Cancelled,
    /// Any other engine error, with its code.
    Other(i64),
}

impl NetError {
    /// Maps an `NSURLErrorDomain` code (spec 05 §6).
    pub fn from_nsurl_code(code: i64) -> Self {
        match code {
            -1009 => Self::Offline,
            -1003 => Self::DnsFailed,
            -1001 => Self::TimedOut,
            -1004 => Self::Refused,
            -1005 => Self::Reset,
            -1206..=-1201 => Self::Certificate,
            -1002 => Self::UnsupportedScheme,
            -1007 => Self::TooManyRedirects,
            -999 => Self::Cancelled,
            other => Self::Other(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nsurl_codes_map_to_error_kinds() {
        let table = [
            (-1009, NetError::Offline),
            (-1003, NetError::DnsFailed),
            (-1001, NetError::TimedOut),
            (-1004, NetError::Refused),
            (-1005, NetError::Reset),
            (-1201, NetError::Certificate),
            (-1206, NetError::Certificate),
            (-1002, NetError::UnsupportedScheme),
            (-1007, NetError::TooManyRedirects),
            (-999, NetError::Cancelled),
            (-1100, NetError::Other(-1100)),
        ];
        for (code, kind) in table {
            assert_eq!(NetError::from_nsurl_code(code), kind, "{code}");
        }
    }
}
