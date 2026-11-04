# ui/

The Svelte 5 interface: the main window, the overlay, the settings window and `mado://` internal pages. Root rules in [../AGENTS.md](../AGENTS.md) apply.

Before editing anything here, load `svelte-development` together with `typescript-development` (see Skill routing in the root file). Load `design` only for look-and-feel work from M6 on.

## Rules

- **The UI is a projection.** It renders the state document from the core and sends commands. It never holds browser state of its own; only view state (hover, focus, which menu is open, scroll position).
- **One way to talk to the core.** Use the typed `invoke()` client and the patch store. Do not call Tauri APIs directly from components.
- **Generated types.** `src/generated/` comes from `mado-types` (`cargo test -p mado-types`). Never edit it by hand.
- **Strings.** Every user-facing string is a key in `src/i18n/en.json` (ICU MessageFormat), rendered with `t()`. No literal text in markup.
- **Placeholders until M6.** New screens go in `src/placeholder/` as plain functional components: correct data, commands, keyboard access and labels, no visual design decisions. Topic 9 decides the look.
- **Accessibility.** Every control is reachable by keyboard, has a visible focus state and an accessible name. Respect reduced motion.
- **Performance.** Apply patches without re-rendering whole lists; no continuously repainting animations.
- **Settings rows** come from the settings registry, not hand-written forms.
- **Runes only.** New code uses `$state`, `$derived` and `$props`, `onclick`-style events and snippets; no `export let`, `$:`, `on:` or `<slot>`. Compute with `$derived`, never by writing state inside `$effect`. Key every `{#each}` by a stable id.
- **No SvelteKit.** This is plain Svelte with Vite inside Tauri: no SSR, no `+page` files, no `$app/*` imports.

## Commands

- `bun run dev:ui` from the repository root starts Vite.
- `bun run --cwd ui typecheck` runs `svelte-check`; add `--watch` while iterating.
- `npx @sveltejs/mcp svelte-autofixer <file> --svelte-version <installed>` checks a component you wrote or changed (escape `$` as `\$` for inline code).
- `bun test ui` runs UI unit tests.
- `.svelte` files are formatted by Prettier (`bun run format`); everything else by Biome.
