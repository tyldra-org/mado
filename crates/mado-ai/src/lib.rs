//! AI connections, model catalog, roles and provider clients.
//!
//! Specification: `mado-docs/spec/13-ai.md` sections 2 to 4. Roadmap: M5.

mod anthropic;
pub mod catalog;
pub mod client;
#[cfg(test)]
mod client_tests;
mod compat;
pub mod connection;
mod gemini;
pub mod http;
pub mod local;
pub mod models;
pub mod oauth;
mod openai;
pub mod secrets;

#[cfg(any(test, feature = "testing"))]
pub mod mock;

pub use client::{
    ClientError, Effort, EventStream, Message, ModelClient, ModelEvent, ModelRequest, Part, Role,
    StopReason, ToolSpec, Turn, collect,
};
pub use connection::{Api, ConnectionError, Kind, Provider};
