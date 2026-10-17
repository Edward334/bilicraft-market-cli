#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="$ROOT_DIR/target/release/bilicraft-ai-stock"
BACKUP="$ROOT_DIR/backups"
MODE="${1:-release}"
SERVICE="${BILICRAFT_SERVICE:-bilicraft-ai-stock}"

mkdir -p "$BACKUP" "$(dirname "$BINARY")"
TMP="$(mktemp -d "$ROOT_DIR/.update.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
"$ROOT_DIR/scripts/backup.sh"

case "$MODE" in
  release)
    command -v curl >/dev/null || { echo 'curl is required' >&2; exit 1; }
    command -v tar >/dev/null || { echo 'tar is required' >&2; exit 1; }
    BASE="${BILICRAFT_RELEASE_BASE:-https://github.com/Edward334/bilicraft-market-cli/releases/download/latest}"
    PACKAGE="bilicraft-ai-stock-linux-x86_64.tar.gz"
    curl --fail --location --silent --show-error --retry 3 --connect-timeout 10 --max-time 180 \
      "$BASE/$PACKAGE" -o "$TMP/$PACKAGE"
    curl --fail --location --silent --show-error --retry 3 --connect-timeout 10 --max-time 30 \
      "$BASE/$PACKAGE.sha256" -o "$TMP/$PACKAGE.sha256"
    (cd "$TMP" && sha256sum --check "$PACKAGE.sha256")
    tar -xzf "$TMP/$PACKAGE" -C "$TMP"
    install -m 0755 "$TMP/bilicraft-ai-stock" "$TMP/bilicraft-ai-stock.new"
    "$TMP/bilicraft-ai-stock.new" --help >/dev/null
    ;;
  source)
    command -v cargo >/dev/null || { echo 'Cargo is required for source builds' >&2; exit 1; }
    command -v node >/dev/null || { echo 'Node.js is required for source builds' >&2; exit 1; }
    npm --prefix "$ROOT_DIR/web" ci --ignore-scripts
    npm --prefix "$ROOT_DIR/web" run build
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --release --locked
    install -m 0755 "$ROOT_DIR/target/release/bilicraft-ai-stock" "$ROOT_DIR/target/release/bilicraft-ai-stock.new"
    ;;
  *)
    echo "Usage: $0 [release|source]" >&2
    exit 2
    ;;
esac

OLD="$BACKUP/bilicraft-ai-stock.previous"
HAD_OLD=0
if [[ -x "$BINARY" ]]; then
  cp -p "$BINARY" "$TMP/bilicraft-ai-stock.old"
  install -m 0755 "$TMP/bilicraft-ai-stock.old" "$OLD"
  HAD_OLD=1
fi

systemctl_run() {
  if (( EUID == 0 )); then systemctl "$@"; else sudo systemctl "$@"; fi
}

WAS_ACTIVE=0
if command -v systemctl >/dev/null && systemctl is-active --quiet "$SERVICE"; then
  WAS_ACTIVE=1
  systemctl_run stop "$SERVICE"
fi

install -m 0755 "$TMP/bilicraft-ai-stock.new" "$BINARY"
if ! "$BINARY" --root "$ROOT_DIR" status >/dev/null; then
  echo 'New binary failed its startup/status check; restoring previous version.' >&2
  if (( HAD_OLD )); then install -m 0755 "$OLD" "$BINARY"; else rm -f "$BINARY"; fi
  if (( WAS_ACTIVE )); then systemctl_run start "$SERVICE"; fi
  exit 1
fi

if (( WAS_ACTIVE )); then
  if ! systemctl_run start "$SERVICE"; then
    echo 'Service failed to start with the new binary; restoring previous version.' >&2
    if (( HAD_OLD )); then install -m 0755 "$OLD" "$BINARY"; else rm -f "$BINARY"; fi
    systemctl_run start "$SERVICE" || true
    exit 1
  fi
  sleep 2
  if ! systemctl is-active --quiet "$SERVICE"; then
    echo 'Service is not healthy after update; restoring previous version.' >&2
    systemctl_run stop "$SERVICE" || true
    if (( HAD_OLD )); then install -m 0755 "$OLD" "$BINARY"; else rm -f "$BINARY"; fi
    systemctl_run start "$SERVICE" || true
    exit 1
  fi
fi

printf 'Updated binary successfully: %s\n' "$BINARY"
if (( ! WAS_ACTIVE )); then
  printf 'Systemd service was not active; start it with: sudo systemctl start %s\n' "$SERVICE"
fi
