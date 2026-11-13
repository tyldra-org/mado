# Contributing to Mado

Thank you for your interest in improving Mado. This guide explains how to propose a change and what makes it easy to accept.

Mado is pre-alpha. Most work right now follows the milestone plan, so the best way to help is to pick up an issue labelled `good first issue` or `help wanted`, or to open an issue before starting anything larger.

## Before you start

- Read the [Code of Conduct](CODE_OF_CONDUCT.md). It applies everywhere the project operates.
- Search [existing issues](https://github.com/tyldra-org/mado/issues) first.
- Every pull request starts from an issue. Find one or open one; for anything larger than a small fix, agree on the approach there before writing code. Product decisions (what Mado does) are made in issues first, then implemented.
- Report security problems privately through [SECURITY.md](SECURITY.md), never in a public issue.

## Set up

You need Rust through `rustup` (the pinned toolchain installs itself), Bun at the version in `.bun-version`, and the platform build tools listed in the [README](README.md#build-from-source).

```sh
git clone https://github.com/tyldra-org/mado.git
cd mado
bun install --frozen-lockfile
```

## Make a change

1. Fork the repository and create a branch from `main`.
2. Keep the change focused on one purpose. Split unrelated work into separate pull requests.
3. Add or update tests that prove the change.
4. Update documentation and `CHANGELOG.md` under `Unreleased` when behavior visible to users changes.
5. Run the full local check, which is the same entry point CI runs:

   ```sh
   ./scripts/check.sh
   ```

   Individual stages:

   | Stage | Rust half | TypeScript half |
   | --- | --- | --- |
   | Format | `cargo fmt --all --check` | `bun run format:check` |
   | Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings && bun run deps:check` | `bun run lint` |
   | Build | `cargo build --workspace --locked` | `bun run build` |
   | Test | `cargo nextest run --workspace` plus `cargo test --doc`, or `cargo test --workspace` without nextest | `bun test`, then Vitest in `ui/` (`bun run test:ui`) |
   | Supply chain | `cargo deny check` | |
   | Shell and workflows | `shellcheck`, `actionlint`, `zizmor` | |

   `bun run format` and `cargo fmt --all` fix formatting.

   `./scripts/check.sh --list` prints the stages, and `./scripts/check.sh <stage>` runs one, for example `./scripts/check.sh lint`. `--scope rust` or `--scope ts` runs only that half, for example `./scripts/check.sh --scope ts lint test`. Locally, a stage whose tool is missing (`shellcheck`, `actionlint`, `cargo-deny`) is skipped and named in the summary. CI runs `./scripts/check.sh --all`, where a skipped stage fails, so install those tools to match CI.

   While iterating, run the narrowest check: `cargo test -p mado-<name>` for one crate, `bun test <file>` or `bun test --changed` for JavaScript and TypeScript, `bun run --cwd ui test <file>` for a Svelte component. Component and runes tests are named `*.svelte.test.ts` and run under Vitest; other tests are `*.test.ts` and run under `bun test`.

6. Mado targets macOS first; Windows (M8) and Linux (M9) come after the UI design pass. If your change touches platform code, say which platforms you ran it on. CI also builds the Windows code so it keeps compiling.

   CI runs only what your change needs (`scripts/ci-scope.ts` decides). A docs-only pull request skips every check but the required `ci` result. A Rust change checks the changed crates and every crate that depends on them, on macOS and Windows. A TypeScript change runs format, lint and typecheck once and the tests it reaches on Linux and Windows. Dependency, toolchain or CI changes check everything, and so does every push to `main`. So does a file in none of the lists in `scripts/ci-scope.ts`: when you add a new kind of file, add it to the list for what reads it, with a case in `scripts/ci-scope.test.ts`.

## Optional local tools

None of these are required; each makes a local loop faster or shows something the checks do not. Install the Cargo tools with `cargo install --locked <name>` (or `cargo binstall <name>`).

| Tool | Use it for | Command |
| --- | --- | --- |
| `cargo-nextest` | Faster Rust tests; `check.sh` uses it when present | `cargo nextest run -p mado-<name>`, one test with `-E 'test(<name>)'`. It skips doc tests: `cargo test -p mado-<name> --doc` |
| `bacon` | Clippy or tests rerun on every save, with Mado's jobs from `bacon.toml` | `bacon` (Clippy as CI runs it), `bacon nextest`, `bacon doc` |
| `sccache` | Reuses compiled crates across branches and checkouts | Set `rustc-wrapper` in your own `~/.cargo/config.toml`, never in this repository |
| `cargo-llvm-cov` | Coverage, to see which behaviour no test reaches | `rustup component add llvm-tools`, then `cargo llvm-cov nextest -p mado-<name> --no-tests=pass --html` |
| `samply` | CPU profiles of the app or a benchmark | `samply record <binary>` opens the profile in the Firefox Profiler |
| `cargo-llvm-lines` | Generic code that bloats compile time | `cargo llvm-lines -p mado-<name>` |
| `cargo-machete` | Unused dependencies | `cargo machete` |
| `hyperfine` | Before and after timings for a speed-up | `hyperfine --warmup 1 '<before>' '<after>'` |

Debug Rust with `rust-lldb` or your editor's LLDB integration on the test or app binary, and `RUST_LOG=debug` for log output. Debug the UI with the Web Inspector of the platform's web view: Safari's Develop menu for WKWebView on macOS, Edge DevTools for WebView2 on Windows. On macOS the system linker is already fast; no linker setup is needed.

## Code rules

- The Rust core owns all browser state. The UI only sends commands and renders what the core sends back.
- Engine code stays inside `mado-engine-webkit` and `mado-engine-webview2`; nothing else calls WebKit or WebView2 directly. `bun run deps:check` enforces the crate dependency direction.
- Web pages are untrusted. Anything a page sends is validated in Rust before Mado acts on it.
- User-facing text goes in `ui/src/i18n/en.json`, never in code.
- `unsafe` is allowed only in the engine adapters and `mado-platform`, with a `// SAFETY:` comment on every block.
- No telemetry, and no network request that is not listed in the privacy rules.

## Commit messages and pull request titles

This project uses [Conventional Commits](https://www.conventionalcommits.org/). Pull requests are squash merged and the pull request title becomes the commit subject, so the title should follow the format (a maintainer may edit it before merging):

```text
type(optional-scope): short imperative description
```

Allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`. Scopes are crate or package names without the `mado-` prefix, for example `fix(navigation): keep the opener selected after closing a tab`. Mark breaking changes with `!` after the type or scope and explain the migration in the body.

## Labels and trust

Issues and pull requests carry an `area:` label for the part of Mado they touch, such as `area: engine` or `area: passwords`, and a `platform:` label when they concern only macOS or Windows; pull requests get theirs from the files they change. Anyone may open a pull request. People the maintainer trusts, after a merged pull request or a roadmap task agreed on its issue, are listed in [`.github/VOUCHED.td`](.github/VOUCHED.td), and their pull requests are marked `contributor: vouched`. Issues and pull requests from people denounced there close automatically.

## Files only maintainers change

Only the maintainers in [`.github/MAINTAINERS`](.github/MAINTAINERS) change these. A pull request from anyone else that touches one fails the `protected-files` check and cannot merge, whatever its review says. Open an issue describing the change instead.

- `.github/`: workflows, issue forms, the pull request template and Dependabot.
- `LICENSE`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md` and other policy files.
- `AGENTS.md` files, at the root and in folders.
- `scripts/check.sh`, `scripts/ci-scope.ts` and `scripts/check-deps.ts`, which decide what CI checks, and `deny.toml`, the dependency and license policy.
- `lists/`, the signed lists Mado ships.

## Pull requests

- Open an issue first, so the change is agreed before you write it, and name it in the description: `Closes #<number>`. The `linked-issue` check fails without one, unless the pull request changes Markdown files only, such as a typo fix. Pull requests from the maintainers in [`.github/MAINTAINERS`](.github/MAINTAINERS) and from Dependabot need none.
- Fill in the pull request template, including how you verified the change.
- Keep pull requests small; they are reviewed faster and more thoroughly.
- All required checks must pass before merge.
- A maintainer will review, usually within a few working days. If there is no response after a week, a polite ping on the pull request is welcome.
- Respond to review comments with new commits; the history is squashed on merge.

## Licensing

Contributions are licensed under the project license, the Mozilla Public License 2.0 (see [LICENSE](LICENSE)), without additional terms. Submit only work you have the right to license that way.

## AI-assisted contributions

AI tools are welcome. You remain responsible for every line you submit: read it, test it, and be able to explain it. Agents should follow [AGENTS.md](AGENTS.md).
