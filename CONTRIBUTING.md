# Contributing to Mado

Thank you for your interest in improving Mado. This guide explains how to propose a change and what makes it easy to accept.

Mado is pre-alpha. Most work right now follows the milestone plan, so the best way to help is to pick up an issue labelled `good first issue` or `help wanted`, or to open an issue before starting anything larger.

## Before you start

- Read the [Code of Conduct](CODE_OF_CONDUCT.md). It applies everywhere the project operates.
- Search [existing issues](https://github.com/tyldra-org/mado/issues) first.
- For anything larger than a small fix, open an issue to agree on the approach before writing code. Product decisions (what Mado does) are made in issues first, then implemented.
- Report security problems privately through [SECURITY.md](SECURITY.md), never in a public issue.

## Set up

You need Rust through `rustup` (the pinned toolchain installs itself), Bun at the version in `.bun-version`, and the platform build tools listed in the [README](README.md#development).

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

   | Stage | Command |
   | --- | --- |
   | Format | `cargo fmt --all --check && bun run format:check` |
   | Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings && bun run lint && bun run deps:check` |
   | Build | `cargo build --workspace --locked && bun run build` |
   | Test | `cargo test --workspace --locked && bun test` |
   | Supply chain | `cargo deny check` |
   | Shell and workflows | `shellcheck`, `actionlint` |

   `bun run format` and `cargo fmt --all` fix formatting.

   `./scripts/check.sh --list` prints the stages, and `./scripts/check.sh <stage>` runs one, for example `./scripts/check.sh lint`. Locally, a stage whose tool is missing (`shellcheck`, `actionlint`, `cargo-deny`) is skipped and named in the summary. CI runs `./scripts/check.sh --all`, where a skipped stage fails, so install those tools to match CI.

   While iterating, run the narrowest check: `cargo test -p mado-<name>` for one crate, `bun test <file>` or `bun test --changed` for JavaScript and TypeScript.

6. Mado ships on macOS and Windows. If your change touches platform code, say which platforms you ran it on. CI runs both.

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

## Pull requests

- Fill in the pull request template, including how you verified the change.
- Keep pull requests small; they are reviewed faster and more thoroughly.
- All required checks must pass before merge.
- A maintainer will review, usually within a few working days. If there is no response after a week, a polite ping on the pull request is welcome.
- Respond to review comments with new commits; the history is squashed on merge.

## Licensing

Contributions are licensed under the project license, the Mozilla Public License 2.0 (see [LICENSE](LICENSE)), without additional terms. Submit only work you have the right to license that way.

## AI-assisted contributions

AI tools are welcome. You remain responsible for every line you submit: read it, test it, and be able to explain it. Agents should follow [AGENTS.md](AGENTS.md).
