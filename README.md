<div align="center">

# Mado

**A private, open-source web browser for Mac, built on the engine your system already ships.**

[Platforms](#platforms) · [Privacy](#privacy) · [Roadmap](#roadmap) · [FAQ](#faq) · [Build from source](#build-from-source) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md)

[![CI](https://github.com/tyldra-org/mado/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/tyldra-org/mado/actions/workflows/ci.yml)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)

</div>

> [!IMPORTANT]
> **Mado is pre-alpha.** The design and the repository scaffold are in place; building starts with the engine prototype. There is nothing to install yet. Watch this repository's releases to hear when the first build is out.

Mado is an open-source web browser for macOS, written in Rust. It works like Chrome where your habits live: the same shortcuts, menus, tab behaviour and settings layout. It differs where it should: vertical tabs down the side, a built-in ad blocker, its own password manager, and optional AI that works with the subscriptions or keys you already have. Pages render in the engine your operating system already ships, WebKit on macOS, so there is no second browser engine to download and the engine gets security fixes with your OS updates.

## What Mado will do

- **Feel familiar.** Chrome's shortcuts, menus, tab groups and session restore, so nothing has to be relearned.
- **Block ads and trackers by default.** A built-in blocker that reads uBlock Origin's filter lists, and room for the ad blocker extensions you already use.
- **Run your Chrome extensions.** Install from the Chrome Web Store: 1Password, Bitwarden, uBlock Origin Lite, Dark Reader and more.
- **Keep your passwords.** Mado Passwords stores passwords, 2FA codes, passkeys, cards and addresses on your device, and also fills from 1Password or Bitwarden.
- **Keep one profile per process.** Each profile is its own app instance with its own sign-ins, history and extensions.
- **Use AI only if you want to.** Bring your own API keys, a local model, or a subscription through its official CLI. Agents work in tabs of their own and ask before they send, pay or delete. Everything is off until you turn it on.

## Privacy

- **No telemetry.** Mado never reports how you use it.
- **No account needed** to browse.
- **Known destinations only.** Mado itself contacts only a published list of hosts, such as filter-list and update servers.
- **Private windows leave nothing behind**, and Mado never logs page content, credentials or AI prompts.
- **You stay in control.** Every optional feature has a switch, and a switch you turn off stays off.

## Platforms

| Platform                        | Engine             | Minimum    |
| ------------------------------- | ------------------ | ---------- |
| macOS (Apple silicon and Intel) | WebKit (WKWebView) | macOS 15.4 |

## How it works

A Rust core owns every piece of browser state: tabs, history, settings, passwords and permissions. Each profile runs as its own process. The interface is Svelte 5 and TypeScript inside Tauri 2, and it only renders what the core sends it. Web pages are treated as hostile: nothing a page sends reaches the interface, and every message from a page is validated in Rust before Mado acts on it.

## Roadmap

| Milestone                 | Release | Delivers                                                    |
| ------------------------- | ------- | ----------------------------------------------------------- |
| M0 Prototype              |         | Answers open engine questions on real WebKit (macOS)        |
| M1 Foundation             |         | Core, storage, settings, app shell, release pipeline        |
| M2 Browsing core          | v0.1    | A browser you can use every day                             |
| M3 Extensions and blocker | v0.2    | Chrome Web Store extensions and the built-in blocker        |
| M4 Passwords              | v0.3    | Mado Passwords, autofill, passkeys, 1Password and Bitwarden |
| M5 AI and control         | v0.4    | Chat, agents, CLI, MCP                                      |
| M6 Design pass            | v0.5    | The final look across every screen                          |
| M7 1.0                    | v1.0    | Performance, accessibility and distribution on macOS        |

No milestone is complete yet. Until the design pass in M6, screens are plain functional placeholders.

## FAQ

### Can I download Mado yet?

Not yet. Mado is pre-alpha, and v0.1 is the first build meant for everyday browsing (see [Roadmap](#roadmap)). Watch this repository's releases to hear when it is out.

### Which browser engine does Mado use?

The one your system already ships. On macOS that is WebKit, the engine Safari uses. Mado bundles no engine of its own, so engine security fixes arrive with your system's updates.

### Will Mado run Chrome extensions?

That is the plan for v0.2: install extensions from the Chrome Web Store, with 1Password, Bitwarden, uBlock Origin Lite and Dark Reader among the first tested. Before installing, Mado warns you when an extension may not fully work on your system.

### Will Mado block ads?

Yes, by default, from v0.2. The built-in blocker reads uBlock Origin's filter lists, and you can still add the ad blocker extension you already use.

### How is Mado different from Chrome, Safari or Arc?

- **Chrome:** Mado keeps Chrome's shortcuts, menus and tab groups, with no telemetry and no bundled engine.
- **Safari:** Mado uses Safari's engine on macOS, and adds Chrome Web Store extensions.
- **Arc:** tabs sit down the side, as in Arc. Mado is open source under the MPL 2.0 and needs no account.

### Does Mado collect my data?

No. Mado has no telemetry and needs no account to browse. Mado itself contacts only a published list of hosts, such as filter-list and update servers.

### Do I have to use the AI features?

No. AI is off until you turn it on. Then it works with your own API keys, a local model, or a subscription through its official CLI.

### Will Mado run on Windows or Linux?

Not now. Mado is a macOS browser.

## Build from source

Built with Rust, Tauri 2, Svelte 5 and TypeScript, using Bun for the JavaScript side.

Prerequisites:

- [Rust](https://rustup.rs) through `rustup`. The toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml) installs itself.
- [Bun](https://bun.com), at the version in [`.bun-version`](.bun-version).
- Xcode command line tools with the macOS 15 SDK or newer.

```sh
git clone https://github.com/tyldra-org/mado.git
cd mado
bun install --frozen-lockfile
./scripts/check.sh
```

`./scripts/check.sh` is the same entry point CI runs. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full workflow and [AGENTS.md](AGENTS.md) for the repository layout.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md). Do not open public issues for security problems.

## License

Licensed under the Mozilla Public License 2.0; see [LICENSE](LICENSE).
Copyright 2026 Yogesh Prasad.
