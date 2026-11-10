# Mado

Instructions for AI coding agents working on Mado. Humans start with [CONTRIBUTING.md](CONTRIBUTING.md); the rules are the same.

This root file covers what applies everywhere. Some folders have their own `AGENTS.md` (listed in [Where code lives](#where-code-lives)); read it before editing there. The closest file wins, and the user's instructions win over all of them.

Mado is a desktop web browser for macOS and Windows. A Rust core owns every piece of browser state. Pages render in the engine the operating system ships: WKWebView on macOS, WebView2 on Windows. The interface is Svelte 5 and TypeScript inside Tauri 2. Each open profile is its own OS process.

Status: pre-alpha. Most crates are empty and say in their `lib.rs` which spec section and roadmap task will fill them.

## What we never compromise on

Every change is judged against these. If a task seems to need breaking one, stop and ask.

1. **Works like Chrome, in our taste.** Shortcuts, menus, tab behaviour, session restore and where settings live follow Chrome. Mado differs only where a decision says so (vertical tabs, the built-in blocker, Mado Passwords, AI). Calm and plain: undo instead of "Are you sure?", no nags.
2. **Web pages are hostile.** Nothing a page sends is trusted. Page content never reaches Tauri commands, and every page message is validated in Rust before Mado acts on it.
3. **Private by default.** No telemetry. Mado contacts only the hosts listed in the privacy spec. Never log URLs from private windows, page content, credentials, tokens or AI prompts.
4. **The user controls everything.** Every behaviour has a setting, every optional feature can be switched off, and a switch the user turned off stays off. Some settings are protected: agents, the CLI and MCP can never change them.
5. **Two platforms, one behaviour.** A feature works on macOS and Windows, or the gap is stated in the spec and shown in the app.
6. **Zero cost.** No paid services, servers or certificates. A feature that needs one is not built.

## Glossary

Use these words the same way in code, comments, commits and replies.

- **you**: the agent reading this file. **user**: the person using Mado. **maintainer**: the people who own this repository.
- **core**: the `mado-core` actor inside one profile process. It is the only owner of state.
- **state document**: the tree the core owns and the UI mirrors. **patch**: a numbered JSON Patch that moves the UI's copy forward one revision.
- **command**: a typed request to the core (`tab.close`, `settings.set`). **origin**: who sent it (UI, overlay, settings window, internal page, control client, agent). The origin comes from the transport, never the payload.
- **engine**: WebKit or WebView2 behind the `Engine` trait. **adapter**: `mado-engine-webkit` or `mado-engine-webview2`.
- **tab view**: the native web view showing one tab. **UI webview**: Tauri webview drawing Mado's own chrome. **overlay**: a second Tauri webview above tab views for menus, prompts and popups.
- **page script**: a script from `inject/` running inside web pages. **internal page**: a `mado://` page such as history or downloads.
- **profile process**: one running Mado process for one profile. **private window**: a temporary profile that leaves nothing behind.
- **agent**: an AI agent run by Mado for the user (not you). **agent tab**: a tab such an agent opens.
- **protected setting**: a setting only the user can change, from the settings window.

## The plan

Mado is fully specified before it is built. The design lives in the `mado-docs` folder next to this repository:

- `decisions/`: what Mado does and why. Only maintainers change these.
- `spec/`: how each part is built and wired. Start at `spec/README.md`.
- `roadmap/`: milestones M0 to M7, with tasks, subtasks and "Done when" checks.

To work on a roadmap task: read its specs fully, implement every subtask, add tests for its acceptance checks, run the checks, tick the subtasks. If a spec is wrong or missing something, do not improvise a design: write the problem and a proposed fix under "Spec issues" in the milestone file and move to a task that does not depend on it.

Until the UI is designed (milestone M6), screens are plain functional components in `ui/src/placeholder/`. Make them correct and accessible; make no visual design decisions.

## Commands

| Purpose | Command |
| --- | --- |
| Install | `bun install --frozen-lockfile` (the Rust toolchain pinned in `rust-toolchain.toml` installs itself) |
| Everything CI runs | `./scripts/check.sh` (stages: `--list`; one stage: `./scripts/check.sh lint`; one language: `--scope rust` or `--scope ts`). Missing tools such as `shellcheck`, `actionlint`, `zizmor` or `cargo-deny` are skipped and named; CI runs `--all`, where a skip fails |
| One crate | `cargo test -p mado-<name>`, `cargo clippy -p mado-<name> --all-targets -- -D warnings` |
| JS and TS tests | `bun test` (plain `*.test.ts`), or `bun test <path>` for one file |
| Svelte component and runes tests | `bun run test:ui` (Vitest, `ui/src/**/*.svelte.test.ts`) |
| Types | `bun run typecheck` |
| Format | `cargo fmt --all && bun run format` |
| Crate dependency rules | `bun run deps:check` |
| UI dev server | `bun run dev:ui` |
| Page scripts | `bun run build:inject` |
| What CI runs for your committed changes | `bun scripts/ci-scope.ts areas origin/main` (jobs), `bun scripts/ci-scope.ts cargo origin/main` (crates) |

While iterating, run the narrowest check for what you touched, never the full suite:

- Rust: `cargo check -p mado-<name>` after each edit, then `cargo nextest run -p mado-<name>` (one test: `-E 'test(<name>)'`; settings in `.config/nextest.toml`, whose `ci` profile CI uses) plus `cargo test -p mado-<name> --doc`. Without nextest, `cargo test -p mado-<name>`.
- JS and TS: `bun test --changed` runs only tests affected by your changes (compare against a branch with `--changed=<branch>`); add `--bail` to stop at the first failure and `-t "<name>"` for one test.
- UI: `bun run --cwd ui test <file>` for one component test, `--changed` for affected ones; `bun run --cwd ui typecheck` for `svelte-check`.

### Tools

Use these when installed; they live on the developer's machine, never in this repository. `bacon` and the samply viewer are interactive, so recommend them to the user and run the plain commands yourself.

| Need | Tool |
| --- | --- |
| Why a Rust test fails | `cargo test -p mado-<name> --no-run` prints the binary; `rust-lldb -- <binary> <test> --exact`; `RUST_LOG=debug` |
| Where CPU time goes (engines, core, from M2) | `samply record --save-only -o /tmp/mado.json.gz -- <binary>`; the user opens it with `samply load` |
| Which code no test reaches | `cargo llvm-cov nextest -p mado-<name> --no-tests=pass --summary-only` |
| Generic code slowing builds | `cargo llvm-lines -p mado-<name> --lib` |
| Unused dependencies | `cargo machete`, then confirm each by building |
| Prove a speed-up | `hyperfine --warmup 1 '<before>' '<after>'` (cold Rust builds: `--prepare 'cargo clean -p mado-<name>'`) |
| Why a Svelte effect reruns | `$inspect.trace()` as its first statement; remove before committing |
| Check a changed component | `bunx @sveltejs/mcp svelte-autofixer <file> --svelte-version <installed>` |
| Debug the UI inside the app (from M1) | Safari Web Inspector on macOS (WKWebView), Edge DevTools on Windows (WebView2) |

Run project tools through `bun run` or `bunx`, never a global install: the global `tsc` may be a different TypeScript than the lockfile's. Installing or upgrading a tool is the user's call; Cargo tools come from `cargo binstall`.

A new kind of file, such as a new top-level folder or config file, goes in a list in `scripts/ci-scope.ts` with a case in `scripts/ci-scope.test.ts`. A path that no list names makes CI run every check.

Run `./scripts/check.sh` once before you say the work is done.

## How it works

The UI sends a command through the single Tauri command `mado_invoke`. The core checks the origin, runs the handler, which changes state only through a recording transaction, persists, emits one patch to every UI webview, and sends engine commands to the main thread. Engine adapters run on the main thread and report events back to the core through a channel. Page scripts talk to the core over a nonce-checked message channel; the core validates each message against a Rust type. The `mado` CLI and MCP server reach the same core over a local socket, through the same origin checks and approvals.

Full detail: `mado-docs/spec/01-process-and-ipc.md` and `02-engine.md`.

## Where code lives

| Path | Contents | Read first |
| --- | --- | --- |
| `app/` | The application binary; becomes the Tauri 2 shell in M1 | |
| `crates/mado-types/` | IDs, state document, commands, events, errors; exported to TypeScript with ts-rs (wired in M1; until then `ui/src/generated/` holds only its README) | |
| `crates/mado-core/` | The core actor | |
| `crates/mado-engine/` | The `Engine` trait and engine events | |
| `crates/mado-engine-webkit/` | macOS adapter (WKWebView, WKWebExtension) | its `AGENTS.md` |
| `crates/mado-engine-webview2/` | Windows adapter (WebView2) | its `AGENTS.md` |
| `crates/mado-<area>/` | storage, settings, safety, permissions, navigation, extensions, blocker, vault, autofill, import, ai, agent, control, platform | the crate's `lib.rs` |
| `crates/mado-cli/` | The `mado` command line | |
| `ui/` | Svelte 5 interface: main window, overlay, settings window, internal pages | `ui/AGENTS.md` |
| `inject/` | Page scripts injected into web pages | `inject/AGENTS.md` |
| `lists/` | Signed data lists the app fetches | |
| `tests/` | Fixtures and the end-to-end harness; engine fixture pages go in `tests/pages/` (created with the first engine task) | |
| `proto/` | The throwaway M0 prototype; never import from it | |
| `scripts/` | `check.sh` and repository checks | |

Crate dependency direction (enforced by `bun run deps:check`): nothing depends on `mado-app`; only `mado-app` depends on an engine adapter; `mado-types` depends on no workspace crate; `mado-engine` and `mado-cli` depend only on `mado-types`.

## Skill routing

Before the first action of any task, find the rows below that match the work and load those skills first, from the global `~/.agents/skills/` installation. Do not start reading code, editing or running commands until they are loaded, even when the task looks familiar. When several rows match, load each. When the work changes mid-task (for example from Rust to UI, or from coding to a commit), load the newly matching skills before continuing. The skills are not vendored here and are not required of human contributors, except `mado-milestone` in `.agents/skills/`, which runs roadmap work when the user names a milestone or task (`m0`, `M2-T05`).

| Work | Skill |
| --- | --- |
| Any non-trivial code change: design, implementation, refactoring, debugging, performance | `engineering-judgment` |
| Rust in `app/` or `crates/`: ownership, traits, async, `unsafe` in the adapters, Cargo, Clippy | `rust-development` |
| TypeScript in `ui/`, `inject/` or `scripts/`: types, boundary validation, tsconfig | `typescript-development` |
| Bun: `bun test`, `bun build`, scripts, workspaces, `bun.lock` | `bun-development` |
| Svelte components, runes, `.svelte.ts` modules and UI tests in `ui/` | `svelte-development` |
| Svelte UI screens, layout, motion, color (from M6; placeholders before that) | `design` |
| Reviewing a diff, branch or pull request | `code-review` plus the stack skill |
| CI workflows, `scripts/check.sh`, `scripts/ci-scope.ts`, README, CONTRIBUTING, SECURITY, releases | `open-source-repo` |
| Only when the user asks to simplify code or for a strict quality review | `code-quality` |
| Any Git command: status, diff, log, staging, commits, branches, worktrees, history | `git` |
| GitHub work: issues, pull requests, reviews, Actions and CI logs, releases, settings | `github` |
| This file or a folder `AGENTS.md` | `agents-md` |
| Commit messages, PR text, docs and comments people read | `unslop` |

## Ways to hurt yourself

1. **Using the user's real browser data.** `~/Library/Application Support/Mado` and `%LOCALAPPDATA%\Mado` hold the user's real profiles, passwords and history. Never run a development build against them, and never open them read-write. Development builds use `.mado-dev/` in the repository (gitignored).
2. **Killing by pattern.** Never `pkill -f mado` or kill a PID found by name: the user may be browsing in Mado while you work. Kill only a PID you started.
3. **Opening a door for web pages.** Never give a tab view Tauri access, never trust an origin or URL taken from a message payload, never inject a script into the page world when the Mado world would do.
4. **Engine calls off the main thread.** WebKit and WebView2 objects are touched only on the main thread. The core never blocks waiting for one.
5. **Leaking secrets.** Never log, print, snapshot or send to a model a password, card number, 2FA secret, API key, token or cookie. Never put secrets in test fixtures.
6. **Editing generated files.** `ui/src/generated/` comes from `mado-types`. Change the Rust type and regenerate.
7. **Silent network calls.** A new outbound request needs its host in the privacy spec's list. Never add analytics or crash reporting that sends without consent.

## Never run

Only when the user asks for it in this conversation:

- Commit, push (any form, including `--force`), rewrite history, create a tag or open a pull request.
- Change GitHub state (`gh repo edit`, `gh label`, `gh release create`, `gh secret`, `gh api -X POST|PUT|PATCH|DELETE`).
- Publish packages or sign releases.

Never, even when asked casually: print or upload secrets, signing keys, certificates or the user's browser data.

## Hit every surface

The most common bug in a browser is a feature that works where you tested it and is missing everywhere else. Before you call a change done, go through this list and say which entries applied:

- **Platforms.** macOS and Windows adapters. If you changed one, the other needs the same behaviour or a stated gap.
- **Windows of every kind.** Normal, private, guest, popup, and agent tabs. Private windows keep nothing; agent tabs never take focus and stay out of history.
- **Profiles.** Each profile is its own process; anything shared (app settings, filter lists, updates) must work with several running at once.
- **Entry points.** A command reachable from the UI is usually also reachable from the menu bar, a keyboard shortcut, the command bar, a context menu, the CLI and MCP. Settings appear in settings search.
- **Reverse states.** Every way in has a way out and a way to see it: close and reopen, hide and unhide, allow and revoke, turn off and back on.
- **Feature switches.** With a feature switched off on the Features page, its code does not run and its scripts are not injected.
- **Strings and access.** New text goes in `ui/src/i18n/en.json`. New controls work by keyboard and have screen reader labels.
- **Persistence.** State that must survive a restart is written; a crash mid-change restores cleanly.

## Code rules

- The core owns state. The UI renders the state document and sends commands; it keeps only view state (hover, focus, open menus).
- Rust: one `thiserror` error enum per crate, converted to `MadoError` at the wire. No `unwrap()` outside tests (Clippy denies it). A command that fails changes nothing.
- `unsafe` only in the engine adapters and `mado-platform`, and every block has a `// SAFETY:` comment.
- Wire types derive `Serialize`, `Deserialize` and `TS`, use `camelCase` fields, and live in `mado-types`.
- Times are Unix milliseconds. IDs are UUIDv7 newtypes.
- TypeScript is strict. No `any`. Validate anything from outside (pages, files, network) at the boundary.
- Comments say why: a constraint, an engine quirk, an invariant. Do not restate the code.
- Keep changes focused. Do not reformat or refactor code you were not asked to touch.

## Verifying and declaring done

Do not say a change is done, ready or working until:

1. The focused tests for what you changed pass, and new behaviour has a test that fails without the change.
2. `./scripts/check.sh` passes.
3. Once the app runs (M1 onward), you started it and exercised the change on its happy path, on the platform you are on.
4. Your reply says which platform you ran it on and which you did not. Windows behaviour you could not run is "untested on Windows", never "done".

Engine behaviour that no document confirms is marked "(verify in M0)" in the specs. Do not guess it: use the M0 report, or ask.

## Git and pull requests

- What needs the user's explicit request is listed in [Never run](#never-run).
- Commits and PR titles use Conventional Commits; the scope is the crate or package name without `mado-`: `fix(navigation): select the opener after closing a tab`.
- PR body: the problem in a sentence or two, how you fixed it, the exact commands you ran and on which platforms. End with the model and tool that did the work.
- UI changes need before and after screenshots; motion needs a short video.
- Pin GitHub Actions to full commit SHAs with the tag in a trailing comment.
- Add a `CHANGELOG.md` entry under `Unreleased` for user-visible changes.

## Documentation

- Code comments explain local reasons. The specs explain cross-component design. When your change makes a spec inaccurate, update the spec in the same change or record it under "Spec issues".
- Do not commit plans, research notes or scratch files. Keep them outside the repository.
