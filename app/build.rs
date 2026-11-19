fn main() {
    // generate_context! embeds the UI build. CI's Rust jobs build the app
    // without building the UI first, so give them an empty page; release
    // builds run `bun run build` before `cargo build` (spec 15 §2).
    let ui = std::path::Path::new("resources/ui");
    // Rerun when the UI goes missing too: CI on the owner's Mac keeps the
    // build folder but starts each run from a clean checkout.
    println!("cargo:rerun-if-changed=resources/ui/window.html");
    if !ui.join("window.html").exists() {
        let _ = std::fs::create_dir_all(ui);
        for page in [
            "window",
            "overlay",
            "settings",
            "picker",
            "internal",
            "dev-platform",
        ] {
            let path = ui.join(format!("{page}.html"));
            let _ = std::fs::write(
                &path,
                "<!doctype html><title>Mado</title><p>The UI is not built. Run bun run build.",
            );
            // Dated before this run, so cargo doesn't count the stub as a
            // change and rerun this script on every build.
            let _ = std::fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|f| f.set_modified(std::time::SystemTime::UNIX_EPOCH));
        }
        println!("cargo:warning=app/resources/ui is not built; using empty pages");
    }
    let manifest = tauri_build::AppManifest::new().commands(&["mado_invoke", "mado_dev_platform"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("tauri-build");
}
