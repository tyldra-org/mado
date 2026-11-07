# Design

Load at the start of every task, before writing code.

## Understand

- Read every spec section the task names, fully, and the sections they link
  to for the types, commands and tables you will touch.
- Read the decisions those sections cite. They are read-only.
- Read the code the task fills: the crate's `lib.rs` header (it names the
  spec and task), neighboring modules, existing tests and the merged PRs of
  the tasks this one depends on.
- Read the tasks that depend on this one. Note what they will expect from
  your contract. Design for them; do not build them.
- Resolve every "(verify in M0)" from `../mado-docs/research/m0-report.md`.
- List your assumptions. Confirm each from code or spec, or treat it as an
  open question. An open question that changes the outcome is a stop
  condition: record a Spec issue.

## Design note

The issue body holds it. Summarize the spec in your own words and cite
sections by number (`01 §4.2`); never paste spec text, which is private.

~~~markdown
## Context
What this task delivers and why the milestone needs it, in two or three
sentences. Spec: 01 §3, 06 §8. Depends on: #12 (M1-T03).

## Acceptance
| # | Requirement (subtask or "Done when") | Proof | Platform |
| --- | --- | --- | --- |
| 1 | Registry holds every key in 06 §8 | `cargo test -p mado-settings registry_has_every_key` | both |

## Design
- Owner: the crate, and the actor or module, that owns each new rule and value.
- Contract: new types, commands, events, settings and tables; who calls whom.
- Data flow: entry point, origin check, handler, transaction, persistence,
  patch, UI or engine.
- Failure: what can fail, what the user sees, what state remains (none for a
  failed command).
- Threads and bounds: main-thread engine calls, actor messages, queue and
  cache limits.
- Security and privacy: each new input and where it is validated; hosts
  contacted; what is never logged.
- Alternatives: one or two, and why they lost.

## Wiring
Surfaces from wiring.md that apply, and those owned by a named later task.

## Test plan
Test levels and names, what CI's macOS and Windows jobs run, and the manual
check on this Mac.

## Risks and rollback
Top risks from the pre-mortem and how each is covered. How to undo the change,
including any migration.

## Out of scope
What this task does not do, and the task that does.
~~~

## Pre-mortem

Before building, assume the task shipped and caused a bug six months later.
Write the three likeliest causes under "Risks" and cover each with a test or a
design change. Common Mado answers:

- works on macOS, missing or different on Windows;
- works in a normal window, leaks data in a private window or agent tab;
- runs while its feature switch is off;
- state lost or half-written after a crash;
- a page message or payload origin trusted;
- reachable from the UI but not the menu, shortcut, CLI or MCP;
- a later task has to rewrite the contract.

## Size

When the change will pass roughly 400 lines of hand-written code, or mixes
concerns, split it into several pull requests under the same issue. Each is
green and mergeable alone, in dependency order (types, then core, then UI).
All but the last say `Refs #<issue>`.

## Gate

- Every subtask and the "Done when" line is a row in the acceptance matrix.
- Every row has a planned proof.
- No open question changes the outcome.
