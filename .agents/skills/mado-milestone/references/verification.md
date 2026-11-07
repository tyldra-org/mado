# Verification

Load when writing tests and before calling a task done.

## Prove each acceptance row

- Give every row in the acceptance matrix at least one test, named after the
  behaviour it proves.
- Watch each new behaviour test fail without the change: run it against the
  old code, or break the line under test, see it fail, restore. Say so in
  the PR.
- Test the failure path of each row, not only success: invalid input,
  forbidden origin, storage error, engine error, cancellation, restart.
- Match the test level to the risk: unit tests for pure logic, property tests
  for invariants (patches replay to the same state, migrations round-trip),
  cross-crate tests for command to state to patch, the end-to-end harness in
  `tests/` once it exists, fixture pages in `tests/pages/` for engine work.
- Keep tests deterministic: no sleeps, no network, fixed clocks and IDs where
  values matter, data only under `.mado-dev/` or a temp directory, never the
  user's real profile.

## Run

1. While iterating: the focused commands in `AGENTS.md`.
2. Run new and changed tests several times in a row. One flaky pass is a
   failure; find the cause.
3. Before review: `./scripts/check.sh --all`. A skipped stage fails there;
   install the missing tool the way `CONTRIBUTING.md` says, or report it.
4. From M1 on: start the development build, exercise the happy path and one
   failure path on this Mac, and stop only the PID you started. Take
   screenshots of UI changes.
5. The PR's `ci` check runs the macOS and Windows jobs.

## Grade

- `verified`: run on this Mac through the real entry point.
- `tested`: automated tests pass for that platform (CI's Windows job counts
  for Windows), but nobody exercised it by hand.
- `untested`: neither. Windows behaviour nobody ran or tested is "untested on
  Windows", never "done".

## Evidence in the PR

- The acceptance matrix, each row with its proof, result and grade.
- Commands exactly as run, with pass counts.
- Which tests were seen failing before the change.
- Platforms run and not run; screenshot paths for UI.

Never write a result you did not see.

## Gate

Every row has evidence, `./scripts/check.sh --all` is green, and no claim goes
beyond its grade.
