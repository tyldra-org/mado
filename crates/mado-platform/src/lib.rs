//! Operating system integration: paths, keychain, authentication,
//! notifications, the Dock.
//!
//! Specification: `mado-docs/spec/00-overview.md` (paths) and the sections
//! that call it. Roadmap: M1-T10 (M1-T07 adds paths and the Dock badge).

pub mod dock;
pub mod paths;

pub use paths::Paths;
