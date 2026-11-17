//! Agent runtime, tools, approvals, audit, memory, skills and routines.
//!
//! Specification: `mado-docs/spec/13-ai.md` sections 5 to 12. Roadmap: M5.

pub mod chat;
pub mod commands;
pub mod untrusted;

/// The command list and the tool table live in mado-types, so the CLI can
/// read them without the agent.
pub use mado_types::{catalog, tools};
