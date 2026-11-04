//! The Mado application: the only binary users launch.
//!
//! Today this is a placeholder so the workspace builds. Milestone M1-T07 turns
//! it into the Tauri 2 shell described in `mado-docs/spec/01-process-and-ipc.md`.

fn main() {
    println!(
        "Mado {} (the browser shell arrives in milestone M1)",
        env!("CARGO_PKG_VERSION")
    );
}
