#!/bin/sh
# Fails in CI when ui/src/generated/ differs from what mado-types generates
# (spec 00, conventions). Run after `cargo test -p mado-types`, which rewrites
# the files. Locally it only reports, since regenerated files wait to be
# committed with the Rust change.
set -eu

cd "$(dirname "$0")/.."
changes=$(git status --porcelain -- ui/src/generated)
[ -z "$changes" ] && exit 0
if [ -n "${CI:-}" ]; then
  printf 'ui/src/generated/ differs from mado-types. Run cargo test -p mado-types and commit:\n%s\n' "$changes" >&2
  git --no-pager diff -- ui/src/generated >&2
  exit 1
fi
printf 'note: ui/src/generated/ changed; commit it with the Rust change:\n%s\n' "$changes"
