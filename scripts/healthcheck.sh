#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
BIND_ADDR="${BILICRAFT_WEB_BIND:-127.0.0.1:8787}"
STATUS_URL="http://${BIND_ADDR}/api/status"
AUTH_URL="http://${BIND_ADDR}/api/web-auth/status"
AUTH_JSON="$(curl --fail --silent --show-error --max-time 10 "$AUTH_URL")"
printf '%s\n' "$AUTH_JSON"
python3 -c 'import json, sys
state = json.loads(sys.stdin.read()).get("state")
if state not in {"setup_required", "login_required", "authenticated"}:
    raise SystemExit("web authentication endpoint is not ready")
' <<<"$AUTH_JSON"
printf 'Health check passed: %s\n' "$BIND_ADDR"
