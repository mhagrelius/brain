#!/usr/bin/env bash
#
# Prove the sync path end to end, against a real server, on this machine.
#
#   ./sync-check.sh
#
# Starts a throwaway `brain-server` on a spare port with its own temporary data
# directory, drives it with the real client through `examples/sync_check`, and
# takes it down again. Nothing it touches is anywhere near a real vault: the
# server is given an empty directory and both ends of the check use temporary
# ones of their own.
#
# `./test.sh` is still the gate. This is the thing test.sh cannot do — each
# side's wire format has a test pinning its exact bytes, which catches drift,
# but only running both against each other catches the case where the two are
# self-consistent and disagree.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

port="${BRAIN_SYNC_CHECK_PORT:-18099}"
data="$(mktemp -d)"
# 32 characters is the shortest the server will accept, and this one lives for
# the length of this script on the loopback address.
token="0123456789abcdef0123456789abcdef"
server=""

cleanup() {
  local status=$?
  [[ -n "$server" ]] && kill "$server" 2>/dev/null || :
  rm -rf "$data"
  exit $status
}
trap cleanup EXIT

echo "==> building"
cargo build --release --locked --package brain-server
cargo build --release --example sync_check

echo "==> starting brain-server on 127.0.0.1:$port"
BRAIN_VECTORS_TOKEN="$token" \
BRAIN_VECTORS_DATA="$data" \
BRAIN_VECTORS_ADDR="127.0.0.1:$port" \
  ./target/release/brain-server >"$data/server.log" 2>&1 &
server=$!

# Wait for it rather than sleeping at it: a fixed sleep is either too short on a
# cold cache or wasted every other time.
for _ in $(seq 50); do
  if curl -sf "http://127.0.0.1:$port/health" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$server" 2>/dev/null; then
    echo "the server exited before it listened:" >&2
    cat "$data/server.log" >&2
    exit 1
  fi
  sleep 0.1
done

echo "==> driving it with the real client"
./target/release/examples/sync_check "http://127.0.0.1:$port" "$token"

echo
echo "==> server said"
sed 's/^/    /' "$data/server.log"
