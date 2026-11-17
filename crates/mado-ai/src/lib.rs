//! AI connections, model catalog, roles and provider clients.
//!
//! Specification: `mado-docs/spec/13-ai.md` sections 2 to 4. Roadmap: M5.

pub mod catalog;
pub mod connection;
pub mod http;
pub mod local;
pub mod models;
pub mod oauth;
pub mod secrets;

#[cfg(any(test, feature = "testing"))]
pub mod mock;

pub use connection::{Api, ConnectionError, Kind, Provider};
