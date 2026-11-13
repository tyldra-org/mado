//! Experiments. Each runs from `mado-proto --exp <name>` on a worker thread,
//! returns JSON, and fails the run when its result says `"ok": false`.

use serde_json::{Value, json};

use crate::shell::Ctx;

#[cfg(target_os = "macos")]
mod agent;
#[cfg(target_os = "macos")]
mod blocking;
#[cfg(target_os = "macos")]
mod memory;
#[cfg(target_os = "macos")]
mod overlay;
#[cfg(target_os = "macos")]
mod session;
#[cfg(target_os = "macos")]
mod shell_check;
#[cfg(target_os = "macos")]
mod sites;

pub const NAMES: &[&str] = &["shell", "overlay", "session", "memory", "agent", "blocking"];

#[cfg(target_os = "macos")]
pub fn run(name: &str, ctx: &Ctx) -> Result<Value, String> {
    match name {
        "shell" => shell_check::run(ctx),
        "overlay" => overlay::run(ctx),
        "session" => session::run(ctx),
        "memory" => memory::run(ctx),
        "agent" => agent::run(ctx),
        "blocking" => blocking::run(ctx),
        _ => Err(format!("no experiment {name}")),
    }
}

#[cfg(windows)]
pub fn run(name: &str, _ctx: &Ctx) -> Result<Value, String> {
    Err(format!(
        "{name}: experiments are scripted on macOS only; follow the Windows run sheet"
    ))
}

/// Named pass/fail checks with what was observed for each.
#[derive(Debug)]
pub struct Checks {
    list: Vec<Value>,
    ok: bool,
}

impl Checks {
    pub fn new() -> Self {
        Checks {
            list: Vec::new(),
            ok: true,
        }
    }

    pub fn check(&mut self, name: &str, pass: bool, observed: Value) {
        self.ok &= pass;
        self.list
            .push(json!({ "check": name, "pass": pass, "observed": observed }));
    }

    pub fn finish(self, observations: Value) -> Value {
        json!({ "ok": self.ok, "checks": self.list, "observations": observations })
    }
}
