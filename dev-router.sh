#!/usr/bin/env bash

set -euo pipefail

TARGET="armv7-unknown-linux-musleabihf"
ROUTER_HOST="${ROUTER_HOST:-router}"
REMOTE_ROOT="${REMOTE_ROOT:-/jffs/camofy}"
REMOTE_BIN="${REMOTE_ROOT}/camofy"

TITLE_COLOR="\033[1;38;2;234;0;94m"
RESET_COLOR="\033[0m"

title() {
  printf "%b==> %s%b\n" "${TITLE_COLOR}" "$*" "${RESET_COLOR}"
}

title "Building frontend (web)..."
(cd web && npm run build)

title "Building camofy for ${TARGET}..."
cross build --release --target "${TARGET}"

LOCAL_BIN="target/${TARGET}/release/camofy"
if [ ! -f "${LOCAL_BIN}" ]; then
  echo "Build failed: ${LOCAL_BIN} not found" >&2
  exit 1
fi

title "Ensuring remote directories on ${ROUTER_HOST}..."
ssh "${ROUTER_HOST}" "mkdir -p '${REMOTE_ROOT}' '${REMOTE_ROOT}/log' '${REMOTE_ROOT}/config' '${REMOTE_ROOT}/core' '${REMOTE_ROOT}/tmp'"

title "Uploading binary to ${ROUTER_HOST}:${REMOTE_BIN}..."
cat "${LOCAL_BIN}" | ssh "${ROUTER_HOST}" "cat >'/tmp/camofy' && chmod +x '/tmp/camofy' && mv '/tmp/camofy' '${REMOTE_BIN}'"

title "Starting camofy in background on router..."
ssh "${ROUTER_HOST}" "killall camofy 2>/dev/null || true; CAMOFY_ROOT='${REMOTE_ROOT}' CAMOFY_HOST='0.0.0.0' CAMOFY_PORT='3000' nohup '${REMOTE_BIN}' >>'${REMOTE_ROOT}/log/dev.log' 2>&1 &"

echo "Done. camofy should now be running on the router."
