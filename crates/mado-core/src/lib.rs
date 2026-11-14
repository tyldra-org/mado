//! The core actor: browser state, command dispatch, patches and service wiring.
//!
//! Specification: `mado-docs/spec/01-process-and-ipc.md` section 2.
//! Roadmap: M1-T06.
//!
//! [`start`] runs the core on its own thread. Inputs (commands with their
//! origin, window events, internal results) are handled one at a time; a
//! handler changes the state document only through generated setters, which
//! record the patch every UI and overlay webview applies.

mod core;
mod timers;
mod tokens;
mod ui;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use core::{CoreConfig, CoreHandle, Input, Internal, Reply, start, system_clock};
pub use timers::Timer;
pub use ui::UiSink;
