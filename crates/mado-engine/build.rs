//! Bundles the page scripts in `inject/src/` (spec 02 §5) into
//! `$OUT_DIR/inject/`, which `page_scripts` embeds. Bun runs
//! `inject/build.ts`; it needs no installed packages. The bundles live in
//! the build folder, not the checkout, so they last exactly as long as
//! cargo's record that this script ran: a kept build folder with a fresh
//! checkout (CI on the owner's Mac) still finds them.

use std::path::Path;
use std::process::Command;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Every file, not only the folder: a folder changes only when files
    // come or go, so an edited script would be left unbundled.
    let mut dirs = vec![root.join("inject/src"), root.join("inject/vendor")];
    while let Some(dir) = dirs.pop() {
        println!("cargo:rerun-if-changed={}", dir.display());
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    println!(
        "cargo:rerun-if-changed={}",
        root.join("inject/build.ts").display()
    );
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("inject");
    let status = Command::new("bun")
        .arg(root.join("inject/build.ts"))
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("inject/build.ts failed ({s}); fix the page scripts"),
        Err(e) => panic!("bun is needed to bundle the page scripts (inject/build.ts): {e}"),
    }
}
