# Wiring

Load while building anything that crosses a crate, thread or process
boundary, and before opening the pull request.

A feature is wired when a real user action reaches it through every intended
path, its effect is stored and shown, and every way in has a way out. The
most common Mado bug is a feature that works where it was tested and is
missing everywhere else.

## Trace the path

For each new command or feature, write the path into the PR, one line per
step with the file. Mark a step that does not apply and say why.

1. **Types.** Wire types live in `mado-types`, derive `Serialize`,
   `Deserialize` and `TS`, and use `camelCase`. Regenerate
   `ui/src/generated/`; never edit it by hand. The drift check passes.
2. **Command.** A variant in its family, with the origins allowed to send it.
   Protected settings accept only `Origin::Settings`. The origin comes from
   the transport, never the payload.
3. **Handler.** In `mado-core`, state changes only inside a recording
   transaction. A failure changes nothing and maps to `MadoError` with an
   `ErrorCode`.
4. **Persistence.** New tables or columns arrive by migration with
   `user_version`. Writes go through the single writer. State survives a
   restart, and a crash mid-change recovers cleanly.
5. **Patch.** One patch reaches every UI webview; the UI's copy updates; the
   UI keeps only view state.
6. **Engine.** Engine commands run on the main thread, in both adapters, or
   the gap is in the spec and shown in the app. Engine events come back
   through the channel; the core never blocks on the engine.
7. **Page scripts.** Messages are nonce-checked and validated against a Rust
   type. Scripts run in the Mado world unless the page world is required.
8. **UI.** Placeholder components in `ui/src/placeholder/` until M6: correct,
   keyboard reachable, labelled for screen readers, strings in
   `ui/src/i18n/en.json`.

## Walk the surfaces

Go through "Hit every surface" in `AGENTS.md` and record each entry as wired,
not applicable (with the reason), or owned by a later task (with its ID).
Never skip one silently.

- **Platforms:** macOS and Windows adapters.
- **Window kinds:** normal, private, guest, popup, agent tab.
- **Profiles:** several profile processes running at once.
- **Entry points:** UI, menu bar, Chrome's shortcut, command bar, context
  menu, CLI, MCP; settings appear in settings search.
- **Reverse states:** close and reopen, hide and unhide, allow and revoke,
  off and on again.
- **Feature switch:** switched off, its code does not run and its scripts are
  not injected.
- **Strings and access:** `en.json`, keyboard, screen reader labels.
- **Persistence:** restart and crash.
- **Privacy:** hosts in the privacy spec; nothing private logged.
- **Docs:** spec still accurate; `CHANGELOG.md` under `Unreleased` for
  user-visible changes; the crate's `lib.rs` doc no longer says the task
  will fill it.

A surface that does not exist yet (CLI and MCP before M5, designed UI before
M6) is "owned by a later task" only when that task lists it. Otherwise add a
Spec issue, so the gap cannot be lost.

## Gap scan

Run on the branch before review:

~~~sh
git diff --stat origin/main...HEAD
git diff origin/main...HEAD | grep -nE '^\+.*(TODO|FIXME|XXX|todo!|unimplemented!|dbg!|\.only\(|#\[ignore\]|@ts-ignore|@ts-expect-error|as any|console\.log)'
bun run deps:check
~~~

Remove each hit or justify it in the PR. Then confirm:

- every new public item has a caller, or is a contract a named later task
  consumes;
- every new error variant has a test that reaches it;
- every new setting is in the registry, `en.json` and settings search;
- every new command has a test from each allowed origin and a rejection test
  from a forbidden one;
- every started task, listener, timer and queue has a bound and a stopper.

## Gate

The trace and surface lists are in the PR, and the gap scan is clean or every
hit is justified.
