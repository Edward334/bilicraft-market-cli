#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVICE_NAME="${BILICRAFT_SERVICE:-bilicraft-ai-stock}"
UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"

if (( EUID != 0 )); then
  echo "Run with sudo: sudo $0" >&2
  exit 1
fi
if [[ ! -x "$ROOT_DIR/target/release/bilicraft-ai-stock" ]]; then
  echo "Release binary missing; run ./scripts/install.sh first." >&2
  exit 1
fi
if [[ ! -f "$ROOT_DIR/config.toml" || ! -f "$ROOT_DIR/data/secrets.json" ]]; then
  echo "Configuration missing; run ./scripts/install.sh first." >&2
  exit 1
fi

install -d -m 0755 /etc/systemd/system
sed "s|/opt/bilicraft-ai-stock|$ROOT_DIR|g" \
  "$ROOT_DIR/systemd/bilicraft-ai-stock.service.example" > "$UNIT_PATH"
systemctl daemon-reload
systemctl enable --now "$SERVICE_NAME"
systemctl --no-pager --full status "$SERVICE_NAME"
printf '\nService enabled: %s\n' "$SERVICE_NAME"
printf 'Check local health with: %s/scripts/healthcheck.sh\n' "$ROOT_DIR"
