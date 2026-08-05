#!/usr/bin/env bash
#
# Build brain-server and push it to a registry the NAS can pull from.
#
#   BRAIN_REGISTRY=nas:5000 ./packaging/deploy-server.sh
#   BRAIN_REGISTRY=nas:5000 BRAIN_TAG=2026-08-04 ./packaging/deploy-server.sh
#
# The registry address is not in the repo because it is a property of one
# person's network rather than of this software. Put it in the environment, or
# in a `.env` beside this script that is gitignored.
#
# Both the NAS and this machine are x86_64, so this is an ordinary build with
# no cross-compilation. If that ever stops being true, this is where a
# `--platform linux/arm64` and a qemu step would go.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -f packaging/.env ]]; then
  # shellcheck disable=SC1091
  source packaging/.env
fi

: "${BRAIN_REGISTRY:?set BRAIN_REGISTRY to the registry the NAS pulls from, e.g. nas:5000}"
tag="${BRAIN_TAG:-$(date +%Y-%m-%d-%H%M)}"
image="${BRAIN_REGISTRY}/brain-server"

# `|| true` so `set -e` does not kill the script before the message below can
# explain what is missing.
engine="$(command -v podman || command -v docker || true)"
[[ -n "$engine" ]] || { echo "no podman or docker on PATH" >&2; exit 1; }

# --format docker because OCI has no HEALTHCHECK and podman writes OCI by
# default. The compose file declares one too, but an image that carries its own
# is one less thing to get wrong.
format=()
[[ "$engine" == *podman ]] && format=(--format docker)

echo "==> checking the workspace first"
# Add --headless when deploying over SSH, where there is no display for the
# widget tests: BRAIN_TEST_ARGS=--headless ./packaging/deploy-server.sh
./test.sh ${BRAIN_TEST_ARGS:-}

echo "==> building ${image}:${tag}"
"$engine" build "${format[@]}" -f server/Containerfile \
  -t "${image}:${tag}" -t "${image}:latest" .

echo "==> smoke test before anything leaves this machine"
data="$(mktemp -d)"
chmod 777 "$data"
name="brain-server-smoke-$$"
cleanup() {
  "$engine" rm -f "$name" >/dev/null 2>&1 || :
  # Rootless podman remaps the uid, so the files are not this user's to delete.
  [[ "$engine" == *podman ]] && "$engine" unshare rm -rf "$data" || rm -rf "$data"
}
trap cleanup EXIT

"$engine" run -d --name "$name" --read-only \
  -e BRAIN_VECTORS_TOKEN=0123456789abcdef0123456789abcdef \
  -v "$data:/var/lib/brain-vectors" -p 18102:8082 "${image}:${tag}" >/dev/null

for _ in $(seq 50); do
  curl -sf http://127.0.0.1:18102/health >/dev/null 2>&1 && break
  sleep 0.1
done
curl -sf http://127.0.0.1:18102/health >/dev/null \
  || { echo "the image does not serve /health" >&2; "$engine" logs "$name" >&2; exit 1; }
# Unauthenticated writes must not work, and this is the cheapest place to find
# out that they do.
code="$(curl -s -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:18102/notes/put -d '{"id":"A.md","text":"x"}')"
[[ "$code" == "401" ]] || { echo "an unauthenticated write got $code, not 401" >&2; exit 1; }
echo "    serves, and refuses an unauthenticated write"

# A self-hosted registry is usually plain HTTP, and both engines refuse that by
# default. Opting out is a flag rather than something this works out for
# itself: "the TLS did not work so I sent it unencrypted" is not a decision a
# script should make on someone's behalf, even on their own network.
insecure=()
if [[ "${BRAIN_REGISTRY_INSECURE:-}" == "1" ]]; then
  if [[ "$engine" == *podman ]]; then
    insecure=(--tls-verify=false)
  else
    echo "    note: docker needs ${BRAIN_REGISTRY} in daemon.json's insecure-registries"
  fi
elif ! curl -sfk --max-time 5 "https://${BRAIN_REGISTRY}/v2/" >/dev/null 2>&1 \
  && curl -sf --max-time 5 "http://${BRAIN_REGISTRY}/v2/" >/dev/null 2>&1; then
  cat >&2 <<EOF

${BRAIN_REGISTRY} answers on HTTP but not HTTPS, so the push will be refused.

If that registry is only reachable over your tailnet — where WireGuard is
already encrypting — that is reasonable, and this says so explicitly:

    BRAIN_REGISTRY_INSECURE=1 BRAIN_REGISTRY=${BRAIN_REGISTRY} $0

The machine doing the *pulling* needs to accept HTTP too. If that is the same
machine the registry runs on, it already does — Docker treats localhost as
insecure by default — so referring to the image as localhost:PORT/brain-server
in the compose file needs no configuration anywhere. See server/README.md.

If it is reachable from anywhere else, put a certificate on it instead.
EOF
  exit 1
fi

echo "==> pushing"
"$engine" push "${insecure[@]}" "${image}:${tag}"
"$engine" push "${insecure[@]}" "${image}:latest"

cat <<EOF

Pushed:
    ${image}:${tag}
    ${image}:latest

On the NAS, in the Container Manager project's .env:
    BRAIN_SERVER_IMAGE=localhost:${BRAIN_REGISTRY##*:}/brain-server:${tag}

Note "localhost", not ${BRAIN_REGISTRY%%:*}. A registry stores repositories by
name and not by hostname, so this is the same image you just pushed — and a
registry reached over localhost is one Docker accepts on HTTP without being
configured to. If the NAS pulled it by its own network name instead, its
daemon would need an insecure-registries entry and a restart.

Pinning the dated tag rather than :latest means a restart cannot quietly
change what is holding your notes. See server/README.md.
EOF
