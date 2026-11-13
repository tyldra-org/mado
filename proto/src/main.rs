//! Mado M0 prototype: a throwaway Tauri app that answers the engine questions
//! the specs mark "(verify in M0)". Nothing outside `proto/` imports it, and
//! the folder is deleted when M0 ends. See `proto/README.md`.

mod exp;
#[cfg(target_os = "macos")]
mod fixtures;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod procs;
mod shell;
#[cfg(windows)]
mod windows_tab;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args = match shell::Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("mado-proto: {message}\n\n{}", shell::USAGE);
            return ExitCode::from(2);
        }
    };
    shell::run(args)
}
