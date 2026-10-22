#!/usr/bin/env bash
#
# Bumps every sandbox project with the vump in this working tree, then asks
# each ecosystem's own toolchain what version it ends up building.
#
# This is the one thing `cargo test` cannot answer. The test suite proves vump
# writes the bytes it meant to; only dotnet, uv, npm and cargo can say whether
# those bytes reach a running program. It is deliberately not part of `cargo
# test`: it needs four toolchains and a network, which would make every CI job
# slower and flakier in order to test somebody else's tool.
#
# Every project is reverted afterwards, including on failure. An ecosystem
# whose toolchain is missing is reported and skipped, not failed.
set -uo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
VUMP="$ROOT/target/debug/vump"
FAILED=()
SKIPPED=()

# Bumping the sandbox dirties it, so the script reverts what it did on the way
# out. That is a destructive git command in a real repository, so it refuses to
# start where there is anything there it might throw away.
if [ -n "$(git status --porcelain sandbox/)" ]; then
  echo "sandbox/ has uncommitted changes, and this script reverts sandbox/ when"
  echo "it finishes. Commit or stash them first:"
  git status --short sandbox/ | sed 's/^/  /'
  exit 1
fi

cleanup() {
  git checkout -- sandbox 2>/dev/null
}
trap cleanup EXIT

echo "building vump"
cargo build -q || { echo "cannot build vump"; exit 1; }

# check <tool> <project> <wanted> <command...>
#
# Skips when the toolchain is absent. Otherwise bumps the project and runs the
# command in it, failing when either step fails or the version does not appear.
check() {
  local tool=$1 project=$2 wanted=$3
  shift 3

  echo
  echo "$project"
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "  skip ($tool not installed)"
    SKIPPED+=("$project")
    return
  fi

  local dir="$ROOT/sandbox/$project"
  if ! ( cd "$dir" && "$VUMP" minor --allow-nested >/dev/null ); then
    echo "  FAIL the bump itself failed"
    FAILED+=("$project (bump)")
    return
  fi

  local output
  if ! output=$( cd "$dir" && "$@" 2>&1 ); then
    echo "  FAIL $tool could not build or run it:"
    printf '%s\n' "$output" | sed 's/^/       /'
    FAILED+=("$project ($tool)")
    return
  fi

  if printf '%s' "$output" | grep -qF "$wanted"; then
    echo "  ok   $tool reports $wanted"
  else
    echo "  FAIL $tool does not report $wanted:"
    printf '%s\n' "$output" | sed 's/^/       /'
    FAILED+=("$project (version)")
  fi
}

check node    npm/single-project 1.1.0 node index.js
check dotnet  cs/single-project  1.1.0 dotnet run --project Demo
check dotnet  cs/shared-version  1.1.0 dotnet run --project App
check uv      py/single-project  1.1.0 uv run --quiet python -m demo
# --locked is the assertion: cargo refuses a lock it would have rewritten, so a
# passing build proves vump's edit matches what cargo expects.
check cargo   rs/workspace       1.1.0 cargo run -q --locked --bin sandbox-cli

echo
if [ ${#SKIPPED[@]} -gt 0 ]; then
  echo "skipped: ${SKIPPED[*]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
  echo "FAILED: ${FAILED[*]}"
  exit 1
fi
echo "every sandbox reports the version vump wrote"
