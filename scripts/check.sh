#!/usr/bin/env bash
# Single quality entry point for Mado. Developers and CI run the
# same stages, so a green local run predicts a green CI run.
#
# Usage: scripts/check.sh [--all] [--list] [--scope rust|ts]... [stage...]
#
#   stage            Run only the named stages. Default: every stage.
#   --all            Strict mode for CI and releases: a missing tool or an
#                    unconfigured command fails the stage instead of skipping.
#   --scope S        Run only the rust or ts half of the format, lint, build
#                    and test stages. Repeatable. Default: both.
#   --list           Print the stages and their commands, then exit.
#   -h, --help       Show this help.
#
# Stages: format lint build test supply shell workflows.
# Rust tests run with cargo-nextest plus doc tests when it is installed, and
# with cargo test otherwise.
#
# Environment:
#   CHECK_CARGO_PACKAGES      Cargo package selection (default --workspace),
#                             for example "-p mado-core -p mado-app".
#   CHECK_CARGO_DOC_PACKAGES  Selection for doc tests (default: the same);
#                             empty skips them.
#   CHECK_TS_CHANGED_SINCE    Run only the bun and Vitest tests affected by
#                             changes since this commit (--changed).
#   CHECK_<STAGE>_<RUST|TS>_CMD  Replace one half's command, for example
#                             CHECK_TEST_RUST_CMD='cargo test --locked'. An
#                             empty string disables that half.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

CARGO_PACKAGES=${CHECK_CARGO_PACKAGES:---workspace}
CARGO_DOC_PACKAGES=${CHECK_CARGO_DOC_PACKAGES-$CARGO_PACKAGES}

rust_test_default() {
  if cargo nextest --version >/dev/null 2>&1; then
    # nextest does not run doc tests; cargo test --doc covers them.
    printf 'cargo nextest run %s --locked --no-tests=pass' "$CARGO_PACKAGES"
    if [ -n "$CARGO_DOC_PACKAGES" ]; then
      printf ' && cargo test %s --doc --locked' "$CARGO_DOC_PACKAGES"
    fi
  else
    printf 'cargo test %s --locked' "$CARGO_PACKAGES"
  fi
}

ts_test_default() {
  # bun test runs plain *.test.ts files; Vitest in ui/ runs the Svelte
  # component and runes tests (*.svelte.test.ts).
  if [ -n "${CHECK_TS_CHANGED_SINCE:-}" ]; then
    printf 'bun test --changed=%s && bun run --cwd ui test --changed=%s --passWithNoTests' \
      "$CHECK_TS_CHANGED_SINCE" "$CHECK_TS_CHANGED_SINCE"
  else
    printf 'bun test && bun run --cwd ui test'
  fi
}

half_command() {
  # half_command <stage> <scope>: the command for one half of a stage.
  # Defaults are computed lazily, so a TypeScript-only run never starts cargo
  # (which would install the pinned toolchain).
  local var
  var="CHECK_$(printf '%s_%s' "$1" "$2" | tr '[:lower:]' '[:upper:]')_CMD"
  if [ -n "${!var+set}" ]; then
    printf '%s' "${!var}"
    return
  fi
  case "$1:$2" in
    format:rust) printf '%s' 'cargo fmt --all --check' ;;
    format:ts) printf '%s' 'bun run format:check' ;;
    lint:rust) printf 'cargo clippy %s --all-targets --locked -- -D warnings && bun run deps:check' "$CARGO_PACKAGES" ;;
    lint:ts) printf '%s' 'bun run lint' ;;
    build:rust) printf 'cargo build %s --locked' "$CARGO_PACKAGES" ;;
    build:ts) printf '%s' 'bun run build' ;;
    test:rust) rust_test_default ;;
    test:ts) ts_test_default ;;
  esac
}

ALL_STAGES="format lint build test supply shell workflows"
ALL_SCOPES="rust ts"

strict=false
stages=()
scopes=()
results=()
failed=0

usage() {
  sed -n '2,/^set -euo pipefail$/p' "${BASH_SOURCE[0]}" | sed -e '$d' -e 's/^# \{0,1\}//'
}

die() {
  printf 'check.sh: %s\n' "$*" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case $1 in
    --all) strict=true ;;
    --list) list_only=true ;;
    --scope)
      [ $# -ge 2 ] || die "--scope needs a value (scopes: $ALL_SCOPES)"
      case " $ALL_SCOPES " in
        *" $2 "*) scopes+=("$2") ;;
        *) die "unknown scope: $2 (scopes: $ALL_SCOPES)" ;;
      esac
      shift
      ;;
    -h | --help) usage; exit 0 ;;
    -*) die "unknown option: $1 (see --help)" ;;
    *)
      case " $ALL_STAGES " in
        *" $1 "*) stages+=("$1") ;;
        *) die "unknown stage: $1 (stages: $ALL_STAGES)" ;;
      esac
      ;;
  esac
  shift
done

if [ ${#stages[@]} -eq 0 ]; then
  # shellcheck disable=SC2206 # Intentional word splitting of a fixed list.
  stages=($ALL_STAGES)
fi
if [ ${#scopes[@]} -eq 0 ]; then
  # shellcheck disable=SC2206 # Intentional word splitting of a fixed list.
  scopes=($ALL_SCOPES)
fi

if [ "${list_only:-false}" = true ]; then
  for s in $ALL_STAGES; do
    case $s in
      format | lint | build | test)
        for scope in $ALL_SCOPES; do
          printf '%-11s %s\n' "$s/$scope" "$(half_command "$s" "$scope")"
        done
        ;;
      supply) printf '%-11s %s\n' "$s" 'cargo deny check' ;;
      shell) printf '%-11s %s\n' "$s" 'shellcheck <tracked *.sh files>' ;;
      workflows) printf '%-11s %s\n' "$s" 'actionlint' ;;
    esac
  done
  exit 0
fi

group_start() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then
    printf '::group::%s\n' "$1"
  else
    printf '\n==> %s\n' "$1"
  fi
}

group_end() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then printf '::endgroup::\n'; fi
}

record() {
  # record <stage> <status> [detail]
  results+=("$(printf '%-11s %-7s %s' "$1" "$2" "${3:-}")")
  if [ "$2" = FAIL ]; then failed=$((failed + 1)); fi
}

skip_or_fail() {
  # A skip is visible and named; under --all it is a failure.
  if [ "$strict" = true ]; then
    record "$1" FAIL "$2 (required by --all)"
    printf 'error: %s: %s\n' "$1" "$2" >&2
  else
    record "$1" skip "$2"
    printf 'skip: %s: %s\n' "$1" "$2"
  fi
}

run_timed() {
  # run_timed <stage> <display label> <command...>
  local stage=$1 label=$2 start=$SECONDS rc=0
  shift 2
  group_start "$stage: $label"
  "$@" || rc=$?
  group_end
  if [ "$rc" -eq 0 ]; then
    record "$stage" ok "$((SECONDS - start))s"
  else
    record "$stage" FAIL "exit status $rc after $((SECONDS - start))s"
  fi
}

run_command_stage() {
  # Runs the selected halves (rust, ts) of a stage, each recorded on its own.
  local stage=$1 scope name cmd tool
  for scope in "${scopes[@]}"; do
    name="$stage/$scope"
    cmd=$(half_command "$stage" "$scope")
    if [ -z "$cmd" ]; then
      record "$name" off "disabled by empty CHECK_$(printf '%s_%s' "$stage" "$scope" | tr '[:lower:]' '[:upper:]')_CMD"
      continue
    fi
    read -r tool _ <<<"$cmd"
    if ! command -v "$tool" >/dev/null 2>&1; then
      skip_or_fail "$name" "missing tool: $tool"
      continue
    fi
    run_timed "$name" "$cmd" bash -c "set -euo pipefail; $cmd"
  done
}

shell_files() {
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git ls-files -- '*.sh' '*.bash'
  else
    find . -path ./node_modules -prune -o -type f \( -name '*.sh' -o -name '*.bash' \) -print
  fi
}

run_shell_stage() {
  local files=()
  while IFS= read -r f; do
    [ -n "$f" ] && files+=("$f")
  done < <(shell_files)
  if [ ${#files[@]} -eq 0 ]; then
    record shell ok "no shell scripts"
    return
  fi
  if ! command -v shellcheck >/dev/null 2>&1; then
    skip_or_fail shell "missing tool: shellcheck"
    return
  fi
  run_timed shell "shellcheck (${#files[@]} files)" shellcheck --severity=warning "${files[@]}"
}

run_supply_stage() {
  # Licences, advisories, banned crates and sources (deny.toml).
  if ! command -v cargo-deny >/dev/null 2>&1; then
    skip_or_fail supply "missing tool: cargo-deny"
    return
  fi
  run_timed supply "cargo deny check" cargo deny check
}

run_workflows_stage() {
  if [ ! -d .github/workflows ]; then
    record workflows ok "no workflows"
    return
  fi
  if ! command -v actionlint >/dev/null 2>&1; then
    skip_or_fail workflows "missing tool: actionlint"
    return
  fi
  run_timed workflows actionlint actionlint
}

for stage in "${stages[@]}"; do
  case $stage in
    format | lint | build | test) run_command_stage "$stage" ;;
    supply) run_supply_stage ;;
    shell) run_shell_stage ;;
    workflows) run_workflows_stage ;;
  esac
done

printf '\nSummary%s\n' "$([ "$strict" = true ] && printf ' (--all)')"
for line in "${results[@]}"; do
  printf '  %s\n' "$line"
done

if [ "$failed" -gt 0 ]; then
  printf '\n%d stage(s) failed.\n' "$failed" >&2
  exit 1
fi
printf '\nAll checks passed.\n'
