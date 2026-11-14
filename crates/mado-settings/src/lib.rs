//! Settings: the registry of every setting, stored values, protection, the
//! change log with undo, search, export and import.
//!
//! Specification: `mado-docs/spec/06-settings.md`. Roadmap: M1-T05.
//! Side effects of a change (spec 06 §4) run in the core: [`Change`] tells
//! it which setting changed, from what to what.

pub mod generate;
mod registry;
mod service;
mod setting;
mod strings;
pub mod types;

pub use registry::*;
pub use service::{
    AuthTokens, Change, ImportReport, LogEntry, Outcome, SearchHit, Settings, SettingsError, Write,
    persist,
};
pub use setting::{Migrate, Options, Setting, SettingDef, def, lookup};
pub use strings::Strings;
