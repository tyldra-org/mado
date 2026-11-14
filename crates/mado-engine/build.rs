//! Bundles the page scripts in `inject/src/` (spec 02 §5) into
//! `app/resources/inject/`, which `page_scripts` embeds. Bun runs
//! `inject/build.ts`; it needs no installed packages.

use std::path::Path;
use std::process::Command;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    println!(
        "cargo:rerun-if-changed={}",
        root.join("inject/src").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join("inject/build.ts").display()
    );
    let status = Command::new("bun")
        .arg(root.join("inject/build.ts"))
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("inject/build.ts failed ({s}); fix the page scripts"),
        Err(e) => panic!("bun is needed to bundle the page scripts (inject/build.ts): {e}"),
    }
}
