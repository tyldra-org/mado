<!--
Title must be a conventional commit: type(scope): description
It becomes the squash-merge commit subject.
-->

## What and why

<!-- What does this change and what problem does it solve? -->

<!-- Required: the issue this resolves. The linked-issue check fails without it. -->
Closes #

## How it was verified

<!-- Exact commands run and their results; screenshots for UI changes.
Say which platforms you ran it on: macOS, Windows or both. -->

```sh
./scripts/check.sh
```

## Checklist

- [ ] The title follows Conventional Commits.
- [ ] Tests cover the change, or the reason they do not is stated above.
- [ ] Documentation and `CHANGELOG.md` are updated for user-visible changes.
- [ ] Roadmap subtasks touched by this change are ticked, or none apply.
- [ ] Breaking changes are marked with `!` and the migration is described.
