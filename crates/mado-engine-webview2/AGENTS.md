# mado-engine-webview2

The Windows engine adapter: WebView2 environments, controllers, extensions, request filtering, downloads. Spec: `mado-docs/spec/02-engine.md` section 7, and section 5.2 of `09-extensions.md`. Root rules in [../../AGENTS.md](../../AGENTS.md) apply.

## Rules

- **UI thread only.** WebView2 objects live on the window's UI thread (single-threaded apartment). Event handlers run there; keep them short and hand work to the core.
- **Bindings.** Use `webview2-com` and `windows`. Wrap COM calls so errors become `EngineError` with the `HRESULT` kept.
- **`unsafe`.** Allowed here. Every block has a `// SAFETY:` comment naming the invariant.
- **Deferrals.** Events the core must answer (`NavigationStarting`, `NewWindowRequested`, `PermissionRequested`, `ScriptDialogOpening` and others) take a deferral and complete it exactly once, including on timeout.
- **No isolated worlds.** Page scripts share the page's world. Never assume a message came from Mado's script; the core validates it.
- **Request filtering is hot.** `WebResourceRequested` runs for every request. No allocation-heavy work, no locks held across calls, no logging per request.
- **Map, do not decide.** Policy belongs to the core.

## Testing

This crate builds only on Windows. Fixture pages go in `tests/pages/` (created with the first engine task) and exercise each event; run them on a real Windows machine and say so in the PR.
