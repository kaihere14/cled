#!/usr/bin/env bash
# End-to-end test of clipboard sync through a real relay: starts the relay with a stand-in Clerk
# instance (no credentials needed), then runs the ignored `relay_e2e` test in the desktop crate
# against it. Usage: scripts/relay-e2e.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'kill "$relay_pid" 2>/dev/null; wait "$relay_pid" 2>/dev/null; rm -rf "$work"' EXIT

cd "$root/apps/relay"
node src/testing/e2e-relay.ts "$work/transcript.bin" "$work/relay.log" >"$work/relay.json" &
relay_pid=$!
for _ in $(seq 1 100); do
  [ -s "$work/relay.json" ] && break
  sleep 0.1
done
[ -s "$work/relay.json" ] || { echo "the relay didn't start" >&2; exit 1; }

read_field() { node -e "console.log(JSON.parse(require('fs').readFileSync('$work/relay.json','utf8'))$1)"; }
export CLED_E2E_RELAY_URL="$(read_field .url)"
export CLED_E2E_TOKEN_A="$(read_field .tokens.user_A)"
export CLED_E2E_TOKEN_B="$(read_field .tokens.user_B)"
export CLED_E2E_TRANSCRIPT="$work/transcript.bin"
export CLED_E2E_LOG="$work/relay.log"

cd "$root"
cargo test -p cled-desktop --lib relay_e2e -- --ignored --nocapture
echo "relay transcript: $(wc -c <"$work/transcript.bin") bytes of ciphertext; log: $(wc -l <"$work/relay.log") lines"
