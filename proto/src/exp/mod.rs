//! Experiments. Each runs from `mado-proto --exp <name>` on a worker thread,
//! returns JSON, and fails the run when its result says `"ok": false`.

use serde_json::Value;
#[cfg(target_os = "macos")]
use serde_json::json;

use crate::shell::Ctx;

#[cfg(target_os = "macos")]
mod agent;
#[cfg(target_os = "macos")]
mod blocking;
#[cfg(target_os = "macos")]
mod downloads;
#[cfg(target_os = "macos")]
mod drm;
#[cfg(target_os = "macos")]
mod extensions;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "macos")]
mod media;
#[cfg(target_os = "macos")]
mod memory;
#[cfg(target_os = "macos")]
mod network;
#[cfg(target_os = "macos")]
mod overlay;
#[cfg(target_os = "macos")]
mod passwords;
#[cfg(target_os = "macos")]
mod permissions;
#[cfg(target_os = "macos")]
mod platform;
#[cfg(target_os = "macos")]
mod session;
#[cfg(target_os = "macos")]
mod shell_check;
#[cfg(target_os = "macos")]
mod sites;
#[cfg(target_os = "macos")]
mod update;

pub const NAMES: &[&str] = &[
    "shell",
    "overlay",
    "session",
    "memory",
    "agent",
    "blocking",
    "platform",
    "network",
    "extensions",
    "drm",
    "media",
    "downloads",
    "permissions",
    "keychain",
    "update",
    "passwords",
];

#[cfg(target_os = "macos")]
pub fn run(name: &str, ctx: &Ctx) -> Result<Value, String> {
    match name {
        "shell" => shell_check::run(ctx),
        "overlay" => overlay::run(ctx),
        "session" => session::run(ctx),
        "memory" => memory::run(ctx),
        "agent" => agent::run(ctx),
        "blocking" => blocking::run(ctx),
        "platform" => platform::run(ctx),
        "network" => network::run(ctx),
        "extensions" => extensions::run(ctx),
        "drm" => drm::run(ctx),
        "media" => media::run(ctx),
        "downloads" => downloads::run(ctx),
        "permissions" => permissions::run(ctx),
        "keychain" => keychain::run(ctx),
        "update" => update::run(ctx),
        "passwords" => passwords::run(ctx),
        _ => Err(format!("no experiment {name}")),
    }
}

/// Named pass/fail checks with what was observed for each.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct Checks {
    list: Vec<Value>,
    ok: bool,
}

#[cfg(target_os = "macos")]
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
