//! Address bar input, suggestions, search engines and URL display.
//!
//! Specification: `mado-docs/spec/05-navigation.md`. Roadmap: M2-T06.

mod calc;
mod classify;

pub use calc::evaluate;
pub use classify::{Context, Input, KNOWN_SCHEMES, Refusal, SCOPES, classify};
