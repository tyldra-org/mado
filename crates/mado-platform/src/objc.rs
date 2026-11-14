//! Small helpers for frameworks Mado reaches by class name.

use std::sync::mpsc;
use std::time::Duration;

use objc2::runtime::AnyClass;

// Linking loads the classes looked up by name.
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Speech", kind = "framework")]
unsafe extern "C" {}
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "LocalAuthentication", kind = "framework")]
unsafe extern "C" {}
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// Why a platform call did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    #[error("{0} is not available on this Mac")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
    /// The system answered nothing in time.
    #[error("no answer from {0}")]
    NoAnswer(&'static str),
    /// Needs Mado running as an app bundle (notifications, login items).
    #[error("needs Mado.app; this is a bare binary")]
    NotBundled,
}

pub(crate) fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass, PlatformError> {
    AnyClass::get(name)
        .ok_or_else(|| PlatformError::Unavailable(name.to_string_lossy().into_owned()))
}

pub(crate) fn wait<T>(
    rx: &mpsc::Receiver<T>,
    what: &'static str,
    within: Duration,
) -> Result<T, PlatformError> {
    rx.recv_timeout(within)
        .map_err(|_| PlatformError::NoAnswer(what))
}

/// True when this process runs from an `.app` bundle.
pub fn bundled() -> bool {
    std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}
