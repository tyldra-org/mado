fn main() {
    // generate_context! embeds the UI build. CI's Rust jobs build the app
    // without building the UI first, so give them an empty page; release
    // builds run `bun run build` before `cargo build` (spec 15 §2).
    let ui = std::path::Path::new("resources/ui");
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
            let _ = std::fs::write(
                ui.join(format!("{page}.html")),
                "<!doctype html><title>Mado</title><p>The UI is not built. Run bun run build.",
            );
        }
        println!("cargo:warning=app/resources/ui is not built; using empty pages");
    }
    let manifest = tauri_build::AppManifest::new().commands(&["mado_invoke", "mado_dev_platform"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("tauri-build");
}
