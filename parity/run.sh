#!/usr/bin/env bash
# Parity test: the Rust reader must produce the same graph and excerpts as the web
# version's parser.js.
#
#   parity/run.sh <path-to-obsidian-live-wallpaper> [extra vault [ignore...]]
#
# Always checks the generated edge-case fixture (with and without unresolved links).
# An extra vault, such as your real one, is checked too when given.
set -euo pipefail
web=${1:?usage: parity/run.sh <obsidian-live-wallpaper checkout> [vault [ignore...]]}
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
parser="$web/parser.js"
[ -f "$parser" ] || { echo "no parser.js in $web"; exit 2; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cargo build -q --release -p vw-core --bin vw-dump --manifest-path "$root/Cargo.toml"
dump="$root/target/release/vw-dump"
fail=0

check() { # label vault args...
  local label=$1 vault=$2; shift 2
  node "$here/dump-js.js" "$parser" "$vault" "$@" > "$work/js.json" 2>/dev/null
  "$dump" "$vault" "$@" > "$work/rs.json" 2>/dev/null
  printf '%-32s ' "$label"; node "$here/compare.js" "$work/js.json" "$work/rs.json" || fail=1
  node "$here/dump-js.js" "$parser" "$vault" "$@" --excerpts > "$work/jsx.json" 2>/dev/null
  "$dump" "$vault" "$@" --excerpts > "$work/rsx.json" 2>/dev/null
  printf '%-32s ' "$label excerpts"; node "$here/compare.js" "$work/jsx.json" "$work/rsx.json" excerpts || fail=1
}

node "$here/make-fixture.js" "$work/fixture" > /dev/null
check "fixture" "$work/fixture" --ignore templates --ignore archive/old
check "fixture, no unresolved" "$work/fixture" --ignore templates --ignore archive/old --no-unresolved
check "fixture, no ignores" "$work/fixture"

if [ $# -ge 2 ]; then
  vault=$2; shift 2
  args=()
  for p in "$@"; do args+=(--ignore "$p"); done
  check "$(basename "$vault")" "$vault" "${args[@]+"${args[@]}"}"
fi
exit $fail
