//! Agent runtime, tools, approvals, audit, memory, skills and routines.
//!
//! Specification: `mado-docs/spec/13-ai.md` sections 5 to 12. Roadmap: M5.

pub mod acp;
pub mod chat;
pub mod cli;
pub mod commands;
pub mod memory;
pub mod safety;
pub mod schedule;
pub mod skills;
pub mod untrusted;

/// The command list and the tool table live in mado-types, so the CLI can
/// read them without the agent.
pub use mado_types::{catalog, tools};
