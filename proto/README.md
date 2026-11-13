# M0 prototype

Throwaway code that answers platform questions on real macOS and Windows machines before the real implementation starts: native tab views under Tauri, extensions, blocking cost, session restore, DRM playback, Keychain behaviour with a self-signed build, off-screen agent tabs and more.

Nothing here ships, and the folder is deleted when milestone M0 ends. The deliverable is the M0 report.

The prototype is its own Cargo workspace with its own lockfile. The main workspace, its lints and CI never build it; Biome and shellcheck still read its scripts.

## macOS

```sh
proto/signing.sh create     # once: the "Mado Self-Signed (M0)" identity in .mado-dev/signing/
proto/build-app.sh          # builds and signs proto/target/MadoProto.app
proto/target/MadoProto.app/Contents/MacOS/mado-proto            # the shell
proto/target/MadoProto.app/Contents/MacOS/mado-proto --smoke example.org \
  --out proto/out/t01-smoke.json --screenshot proto/out/t01-shell.png
```

`--smoke` types the URL into the UI's address field, waits for the tab view to load it, prints the result, the view stack and the OS and WebKit versions as JSON, and quits. `--screenshot` needs the Screen Recording permission of the terminal that runs it.

The M0 identity signs prototype builds only. The release identity in spec 15 §3 and its CI secret belong to the maintainer.

## Windows

Untested. `src/windows_tab.rs` was written on a Mac and never compiled. The Windows run sheet in `mado-docs/research/m0-report.md` lists the steps.
