//! Operating system integration: paths, keychain, authentication,
//! notifications, permissions, login items, the default browser, the Dock.
//!
//! Specification: `mado-docs/spec/00-overview.md` (paths) and the sections
//! that call it. Roadmap: M1-T10.

pub mod auth;
pub mod default_browser;
pub mod dock;
pub mod keychain;
pub mod login;
pub mod notify;
mod objc;
pub mod paths;
pub mod permissions;

pub use objc::{PlatformError, bundled};
pub use paths::Paths;
