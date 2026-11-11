# Review and ship

Load after verification passes.

## Self-review

Load `code-review` and read the whole branch diff, `git diff
origin/main...HEAD`, as a senior reviewer who did not write it. Read every
line, tests and generated files included.

- Every change traces to a row in the acceptance matrix or a wiring step.
  Anything else is scope creep: remove it or move it to a follow-up issue.
- A later task will not need to rewrite this contract.
- Names, errors and comments use the `AGENTS.md` glossary.
- Rust: no `unwrap()` outside tests, a `// SAFETY:` comment on every
  `unsafe` block, no lock held across `.await`, bounded queues, minimal
  `pub`.
- TypeScript: strict, no `any`, outside data validated at the boundary.
- The code agrees with every spec sentence it implements; where it does not,
  update the spec (backed up first) or record a Spec issue.

Fix each blocking finding, rerun the focused checks, and review again until
none remain.

## Pull request

- Title: Conventional Commit with the task ID, for example
  `feat(settings): registry and settings commands (M1-T05)`.
- Body, in the repository template's sections:
  - the problem and the approach in two or three sentences;
  - a link to the design note, and any deviation from it with the reason;
  - the acceptance matrix with evidence;
  - the wiring trace and the surface list;
  - risks and how to roll back;
  - platforms run and not run;
  - Spec issues and follow-up issues raised;
  - last line: the model and tool that did the work.
- `Closes #<issue>`: the required `linked-issue` check fails a PR that
  closes no issue. When the PR holds the whole task on both platforms, it
  closes the task's issue. Otherwise, before opening the PR, add sub-issues to
  the task's issue: one for this PR's part, and one for a pending Windows
  run. The PR closes its own sub-issue, and the PR that finishes the task
  also closes the task's issue.

## CI

Watch with `gh pr checks <n> --watch` as a background job, never a sleep
loop. On failure, read the failed step (`gh run view <id> --log-failed`),
reproduce it locally, fix the cause and push a new commit. No amend or force
push after pushing. Three root-cause attempts at most, then stop the task.

## Merge

1. Arm auto-merge as soon as the PR is open:
   `gh pr merge <n> --auto --squash --delete-branch --match-head-commit <sha>`.
   GitHub merges once the required checks pass. Never `--admin`, never a
   bypass.
2. `git switch main && git pull --ff-only`, then rerun the task's focused
   tests on `main`.
3. If `main` broke, revert first (a revert PR), then fix forward in a new
   PR.
4. Tick the finished subtasks in the milestone file. When the task's issue
   stays open, comment on it with what remains.

## Milestone exit

After the last task merges:

1. On fresh `main`, run `./scripts/check.sh --all` and every task's "Done
   when" check together; tasks can break each other.
2. Check each sentence of the milestone's "Goal" against evidence.
3. Set the status line to `done`, or `done on macOS (Windows pending)` with
   the Windows run sheet.
4. Under "Spec issues", record what this milestone showed to be wrong in
   later milestones' specs, and list open follow-ups.
5. Send the report described in `SKILL.md`.
