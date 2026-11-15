//! Address bar input, suggestions, search engines and URL display.
//!
//! Specification: `mado-docs/spec/05-navigation.md`. Roadmap: M2-T06.

mod calc;
mod classify;
mod display;
pub mod engines;
mod idn;
pub mod suggest;

pub use calc::evaluate;
pub use classify::{Context, Input, KNOWN_SCHEMES, Refusal, SCOPES, classify};
pub use display::{Display, display};
pub use idn::{TopDomains, host_for_display};
