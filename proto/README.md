# M0 prototype

Throwaway code that answers platform questions on real macOS and Windows machines before the real implementation starts: native tab views under Tauri, extensions, blocking cost, session restore, DRM playback, Keychain behaviour with a self-signed build, off-screen agent tabs and more.

Nothing here ships, and the folder is deleted when milestone M0 ends. The deliverable is the M0 report.

The prototype is its own Cargo workspace with its own lockfile. The main workspace, its lints and CI never build it; Biome and shellcheck still read its scripts.

## macOS

```sh
proto/signing.sh create     # once: the "Mado Self-Signed (M0)" identity in .mado-dev/signing/
proto/build-app.sh          # builds and signs proto/target/MadoProto.app
proto/target/MadoProto.app/Contents/MacOS/mado-proto            # the shell, by hand
proto/target/MadoProto.app/Contents/MacOS/mado-proto --exp list # the experiments
proto/target/MadoProto.app/Contents/MacOS/mado-proto --exp overlay \
  --out proto/out/exp01.json --screenshot proto/out/exp01.png
```

`--exp <name>` runs one experiment, prints its checks, observations and the OS and WebKit versions as JSON, and exits non-zero when a check fails. An experiment never interrupts whoever is using the Mac: its window opens off-screen, the app takes no focus and shows no Dock icon, and synthesised clicks go to the window's views, never through the real pointer. `--screenshot` needs the Screen Recording permission of the terminal that runs it.

The M0 identity signs prototype builds only. The release identity in spec 15 §3 and its CI secret belong to the maintainer.

## Windows

Untested. `src/windows_tab.rs` was written on a Mac and never compiled. The Windows run sheet in `mado-docs/research/m0-report.md` lists the steps.
