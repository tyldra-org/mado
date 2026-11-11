---
name: mado-milestone
description: "Use when the user names a Mado roadmap milestone or task: 'm0', 'M1', 'milestone 2', 'M2-T05', 'goal m3', 'next task', 'continue the milestone'. Delivers roadmap work end to end at staff-engineer standard: a design note in the GitHub issue, an implementation wired through every surface, a test for every acceptance check, self-review, pull request, squash merge, and a milestone exit review."
---

# Mado milestone

## Scope

Owns how Mado roadmap work is delivered: turning `m0` or `M2-T05` into
designed, fully wired, verified and merged changes, one task at a time, with a
written trail a reviewer can audit later.

Does not own, and loads alongside:

- Repository rules: root and folder `AGENTS.md`. They win on any conflict.
- Engineering method: `engineering-judgment`, plus the language skill for
  the code touched (`rust-development`, `typescript-development`,
  `bun-development`, `svelte-development`).
- Review method: `code-review`. Git: `git`. GitHub: `github`. Prose in
  issues, PRs and docs: `unslop`.
- What to build: the specs and decisions in `../mado-docs`. This skill never
  invents a design the specs do not describe.

## Standard

Work as the senior engineer who will own this code in production for the next
ten years, in a team where every change is reviewed by someone stricter.

- Understand before writing: the spec, the code around the change, the tasks
  that depend on it and the ways it can fail.
- Write the design down and check it against the spec before building.
- A task is done when a user reaches it from every entry point that should
  exist, it survives failure and restart, and a test proves each acceptance
  check. Compiling is not done. Passing on one platform is not done.
- Leave nothing a reviewer would find: no stub, `TODO`, dead path, missing
  way out, unmapped error, unwired setting or unstated platform gap.
- Small reviewable pull requests. Evidence over assertion. Never report a
  result you did not see.

## Resolve the request

| Request | Scope |
| --- | --- |
| `m0` ... `m7`, "milestone N" | Every unfinished task in `../mado-docs/roadmap/M<N>-*.md`, in dependency order, then the milestone exit review |
| `M2-T05` | That task only |
| "next task", "continue" | The first unfinished task in the lowest milestone in progress |

Naming a milestone or task is the user's request, in this conversation, to
open its issues, push branches, open pull requests and squash-merge them after
the required checks (`ci`, `linked-issue`) pass. It does not cover pushing to `main`, force-pushing, tags,
releases, labels or GitHub settings.

## Preflight, once per run

1. Read `AGENTS.md`, `../mado-docs/README.md`, `../mado-docs/spec/README.md`,
   `../mado-docs/spec/00-overview.md`, `../mado-docs/roadmap/README.md` and
   the whole milestone file.
2. Plan the milestone: list its tasks with their "Depends on", mark which are
   done, and note contracts several tasks share (types, commands, tables,
   settings) so the first task that defines one shapes it for the rest.
3. `gh auth status`; the remote is `tyldra-org/mado`.
4. `git fetch origin`, `git switch main`, `git pull --ff-only`. Leave changes
   you did not make alone: never stash, reset or commit them; stage only your
   own paths.
5. `../mado-docs` has no git. Before the first edit there, copy each file you
   will change to `${TMPDIR:-/tmp}/mado-docs-backup-<date>/`.
6. Set the milestone status line to `in progress` if it says `not started`.

## Lifecycle per task

Each phase ends at a gate. Do not start the next phase until the gate holds.

| Phase | Do | Gate | Load |
| --- | --- | --- | --- |
| 1. Pick | Skip tasks whose subtasks are all ticked; skip a task whose dependencies are unmerged and say why | Dependencies merged on `main` | |
| 2. Design | Read the specs, write the design note into the issue | Every subtask and the "Done when" line is a row in the acceptance matrix with a planned proof | [design.md](references/design.md) |
| 3. Build | Branch `<type>/<task-id>-<slug>` from fresh `main`; implement in small commits, acceptance tests first where practical | Focused checks green | language skill |
| 4. Wire | Trace the path end to end and walk every surface | Trace and surface lists written; gap scan clean | [wiring.md](references/wiring.md) |
| 5. Verify | Prove each matrix row, run the full checks, exercise the app | Every row has evidence and a grade | [verification.md](references/verification.md) |
| 6. Review | Review your own diff as a stricter engineer would | No blocking findings | [review-and-ship.md](references/review-and-ship.md), `code-review` |
| 7. Ship | PR, CI, squash merge, check `main`, tick the roadmap | Merged, `main` green, issue closed or its remainder stated | [review-and-ship.md](references/review-and-ship.md) |

After the last task: the milestone exit review in
[review-and-ship.md](references/review-and-ship.md#milestone-exit). For M0,
also load [m0.md](references/m0.md); it replaces the wiring phase.

## Engineering bar

1. The spec is the contract. Implement it as written. When it is wrong or
   silent, record a Spec issue with a proposed fix; never improvise.
2. Design for the next tasks without building them: read the tasks that
   depend on this one and make the contract they will need.
3. One owner per rule and per piece of state. The core owns state; the UI
   mirrors it and holds only view state.
4. A failed command changes nothing. Every error path is designed, mapped to
   `MadoError` at the wire and tested.
5. Every input from a page, file, network or socket is hostile until a Rust
   type has validated it. Every new outbound host is in the privacy spec.
6. Both platforms behave the same, or the gap is in the spec and shown in the
   app. Write the Windows code with the macOS code; CI's Windows job proves it
   builds and its tests pass.
7. Placeholders only where the roadmap allows them (UI screens before M6).
   No `todo!()`, `unimplemented!()` or `TODO`; deferred work belongs to a
   named later task or a linked issue.
8. Never weaken a check to get green: no skipped tests, loosened lints,
   `#[ignore]`, `@ts-ignore` or disabled CI steps.
9. Keep the docs true: when the code makes a spec sentence wrong, update the
   spec in the same change or record a Spec issue.
10. Scope is the task. Refactors, cleanups and ideas outside it become
    follow-up issues, not extra lines in the diff.

## When to stop a task

Leave its pull request open with a comment saying why, and move to the next
independent task, when:

- a spec is wrong or missing: write the problem and a proposed fix under
  "Spec issues" in the milestone file;
- a decision would need to change (`../mado-docs/decisions/` belongs to the
  maintainer);
- the required check still fails after three root-cause fixes;
- it needs hardware or accounts this Mac does not have.

Stop the whole run on anything destructive, a rejected push or a ruleset
error. Never retry with force, `--admin` or a bypass.

## Report

End with one table: task, issue, pull request, merge commit, grade, status
(done; done on macOS, Windows pending; blocked with the reason). Then list
everything graded `untested`, new Spec issues and follow-up issues opened.

## Done means, per task

- [ ] Design note in the issue; acceptance matrix covers every subtask and the
      "Done when" line.
- [ ] Every row has a test that fails without the change, or recorded evidence.
- [ ] Path traced and surfaces walked; each surface wired, not applicable with
      a reason, or owned by a named later task.
- [ ] Gap scan clean; no stub, dead code, unmapped error or weakened check.
- [ ] `./scripts/check.sh --all` and CI green; the app exercised on this Mac
      from M1 on.
- [ ] Self-review done; specs, `CHANGELOG.md` and crate docs match the code.
- [ ] Squash-merged, `main` green, roadmap subtasks ticked, issue closed or
      its remainder stated.
