//! Operating system integration: paths, keychain, authentication,
//! notifications, permissions, login items, the default browser, the Dock, memory pressure and power.
//!
//! Specification: `mado-docs/spec/00-overview.md` (paths) and the sections
//! that call it. Roadmap: M1-T10.

pub mod appearance;
pub mod applinks;
pub mod auth;
pub mod default_browser;
pub mod dock;
pub mod enclave;
pub mod files;
pub mod keychain;
pub mod login;
pub mod native_messaging;
pub mod notify;
mod objc;
#[cfg(target_os = "macos")]
pub mod ondevice;
pub mod paths;
pub mod permissions;
pub mod power;

pub use objc::{PlatformError, bundled};
pub use paths::Paths;
