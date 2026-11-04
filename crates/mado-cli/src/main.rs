//! The mado command line.
//!
//! Talks to a running Mado profile only through its control endpoint.
//! Specification: `mado-docs/spec/14-control-cli-mcp.md`. Roadmap: M5-T07.

fn main() {
    println!(
        "mado {} (the command line arrives in milestone M5)",
        env!("CARGO_PKG_VERSION")
    );
}
