# M0 prototype

Throwaway code that answers platform questions on a real Mac before the real implementation starts: native tab views under Tauri, extensions, blocking cost, session restore, DRM playback, Keychain behaviour with a self-signed build, off-screen agent tabs and more.

Nothing here ships. The deliverable is the M0 report. M0 is done for macOS, but the folder stays until `proto/supervised.sh` has run the checks that moved to M2-T06, M2-T07, M4-T06 and M4-T07. Delete it after those.

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

`memory` and `blocking` take several minutes and use the network: `memory` loads 50 public sites, `blocking` downloads the default filter lists of spec 10 §2 into a temp cache once a day. `platform` and `network` load `example.com` and Google's Safe Browsing test page. `extensions` downloads four extensions from the Chrome Web Store (cached a day) and loads Google's ad script once without a blocker; `drm` asks `example.com`'s page which DRM key systems exist. `downloads` serves a file locally, cancels its download with resume data and resumes it in a second app process it starts. `swift proto/macos/foundation-models.swift` asks Apple's on-device model for one word.

`keychain` and `update` need two more builds next to the first: `proto/build-app.sh MadoProto2 2` (same identity, its own binary) and `proto/build-app.sh MadoProtoAdhoc 3 adhoc` (ad hoc signature). `keychain` disables Keychain user interaction, so a call that would prompt fails instead. `permissions` denies every permission request in its own UI delegate, and posts one notification with provisional authorization, which goes quietly to Notification Center and is removed; afterwards Mado Proto is listed under System Settings › Notifications. `passwords` calls the `op` and `bw` CLIs if they are installed.

The steps that need someone at the Mac (speech recognition, 1Password's approval prompt, `bw serve` after `bw login`, DRM playback with real accounts) run from `proto/supervised.sh`, one prompt at a time. Don't run `--exp permissions` with `MADO_PROTO_SUPERVISED` set from a terminal: macOS then asks on the terminal's behalf, and ends the app when the terminal has no speech recognition usage text.

The M0 identity signs prototype builds only. The release identity in spec 15 §3 and its CI secret belong to the maintainer.
