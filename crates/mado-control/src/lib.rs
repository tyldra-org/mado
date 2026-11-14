//! Control socket server, client pairing and the MCP server.
//!
//! Specification: `mado-docs/spec/14-control-cli-mcp.md`. Roadmap: M5-T06
//! for the full protocol. M1-T07 adds the endpoint itself and the one
//! method it accepts while control is off: `launch.forward` with the
//! profile's launch secret (spec 01 §1.3, 14 §1).

mod endpoint;
mod forward;
mod secret;

pub use endpoint::{CallHandler, Endpoint, ForwardHandler, TestAccess, serve};
pub use forward::{ForwardError, forward};
pub use secret::LaunchSecret;
