#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
BIND_ADDR="${BILICRAFT_WEB_BIND:-127.0.0.1:8787}"
STATUS_URL="http://${BIND_ADDR}/api/status"
CONFIG_URL="http://${BIND_ADDR}/api/config"
STATUS_JSON="$(curl --fail --silent --show-error --max-time 10 "$STATUS_URL")"
printf '%s\n' "$STATUS_JSON"
python3 -c 'import json, sys
status = json.loads(sys.stdin.read())
if status.get("state") != "ready":
    raise SystemExit("service state is not ready")
' <<<"$STATUS_JSON"
curl --fail --silent --show-error --max-time 10 "$CONFIG_URL" >/dev/null
printf 'Health check passed: %s\n' "$BIND_ADDR"
