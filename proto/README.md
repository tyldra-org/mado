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

`--exp <name>` runs one experiment, prints its checks, observations and the OS and WebKit versions as JSON, and exits non-zero when a check fails. An experiment never interrupts whoever is using the Mac: the app takes no focus and shows no Dock icon, its window is transparent, ignores the mouse and floats so nothing covers it (a covered or off-screen window stops rendering), and synthesised clicks and keys go to the window's views, never through the real pointer or keyboard. Because the window is transparent, `--screenshot` captures nothing in experiment runs; it also needs the Screen Recording permission of the terminal. Run one prototype at a time: `memory` counts the WebKit processes started from the same terminal.

`memory` and `blocking` take several minutes and use the network: `memory` loads 50 public sites, `blocking` downloads the default filter lists of spec 10 §2 into a temp cache once a day. `platform` and `network` load `example.com` and Google's Safe Browsing test page. `extensions` downloads four extensions from the Chrome Web Store (cached a day) and loads Google's ad script once without a blocker; `drm` asks `example.com`'s page which DRM key systems exist. `downloads` serves a file locally, cancels its download with resume data and resumes it in a second app process it starts. `swift proto/macos/foundation-models.swift` asks Apple's on-device model for one word. Experiments that would show a macOS permission or Keychain prompt are not scripted: they need someone at the Mac.

The M0 identity signs prototype builds only. The release identity in spec 15 §3 and its CI secret belong to the maintainer.

## Windows

Not run on Windows yet. `proto/check-windows.sh` type-checks the Windows build from a Mac (Clippy, `-D warnings`, target `x86_64-pc-windows-msvc`), so the Windows code compiles; whether it works is for the Windows run sheet in `mado-docs/research/m0-report.md`. Scripted experiments (`--exp`) are macOS-only; on Windows the run sheet lists the manual steps.
