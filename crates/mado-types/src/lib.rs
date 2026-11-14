//! Shared IDs, state document, commands, events and errors, exported to
//! TypeScript.
//!
//! Every wire type lives here. Specification: `mado-docs/spec/00-overview.md`
//! (conventions, error model) and `01-process-and-ipc.md` sections 3 and 4.
//! `cargo test -p mado-types` writes the TypeScript bindings to
//! `ui/src/generated/`; CI fails when the committed files differ.

// Lets `#[derive(Patchable)]` name `::mado_types` inside this crate too.
extern crate self as mado_types;

pub mod command;
pub mod error;
pub mod ids;
pub mod patch;
pub mod state;

pub use command::{Command, CommandFamily, Origin, Request, Response};
pub use error::{ErrorCode, MadoError};
pub use ids::{
    ClientId, DownloadId, ExtensionId, GroupId, ProfileId, PromptId, TabId, TaskId, VaultItemId,
    WindowId,
};
pub use patch::{Patch, PatchOp, PatchRecorder, Patchable};
pub use state::{State, StateSnapshot};
