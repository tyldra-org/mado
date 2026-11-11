<!--
The title becomes the commit subject: type(scope): summary, one change,
imperative, under 64 characters. This description stays here for reviewers.
-->

## What and why

<!-- One to three sentences: the problem and what this change does about it.
Put a longer write-up, such as an agent's notes, at the end inside
<details><summary>Notes from (model and tool)</summary> ... </details>. -->

<!-- Required: the issue this resolves. The linked-issue check fails without it. -->
Closes #

## How it was verified

<!-- The commands that matter and their results, and the platforms you ran
them on: macOS, Windows or both. Screenshots for UI changes. -->

```sh
./scripts/check.sh
```

## Checklist

- [ ] The title follows Conventional Commits.
- [ ] Tests cover the change, or the reason they do not is stated above.
- [ ] Documentation and `CHANGELOG.md` are updated for user-visible changes.
- [ ] Roadmap subtasks touched by this change are ticked, or none apply.
- [ ] Breaking changes are marked with `!` and the migration is described.
