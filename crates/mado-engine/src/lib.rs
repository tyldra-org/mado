//! The Engine trait, engine events and capabilities shared by every adapter.
//!
//! Specification: `mado-docs/spec/02-engine.md` sections 1 to 3. Roadmap:
//! M1-T03 adds `WindowCommand`, which the app shell needs in M1-T07; M2-T01
//! adds the `Engine` trait, `TabCommand`, `EngineEvent` and capabilities.

pub mod window;

pub use window::{WindowCommand, WindowEngine, WindowEvent};
