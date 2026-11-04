# inject/

Page scripts: TypeScript that runs inside web pages, bundled by `build.ts` into one self-contained IIFE per file in `src/`. Root rules in [../AGENTS.md](../AGENTS.md) apply.

These scripts run next to hostile code. Treat every page as an attacker.

## Rules

- **Isolated world by default.** On WebKit, scripts run in the Mado content world unless they must replace a page-visible API (`webauthn`, `notifications`, `geolocation`, `scriptlets`). WebView2 has no isolated worlds, so never rely on isolation for safety: the core validates everything.
- **Capture before the page runs.** The `bridge` script captures the message channel at document start and keeps it in a closure. Never expose Mado objects, functions or IDs on `window`.
- **Never trust the page's environment.** Pages can override prototypes and globals. Capture built-ins you need at document start; do not call methods on page-owned objects you have not checked.
- **Every message is declared.** A new message type needs an entry in the page message table in `mado-docs/spec/02-engine.md` section 5.0 and a matching Rust type that validates it. The core drops anything else.
- **No secrets in the page.** Values such as passwords reach a page script only to fill a field in the same frame, and are never stored or sent anywhere else.
- **No network.** Page scripts never call `fetch` or open connections; they ask the core.
- **Small and fast.** Each script runs on every page load. Keep bundles small, do work lazily, never block the main thread.

## Commands

- `bun run build:inject` from the repository root builds every script into `app/resources/inject/`.
- `bun run --cwd inject typecheck` type-checks.
- Test behaviour with fixture pages in `tests/pages/` (created with the first engine task) on both engines.
