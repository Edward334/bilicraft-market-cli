#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$ROOT_DIR/backups/$STAMP"
cp -a "$ROOT_DIR/config.toml" "$ROOT_DIR/data" "$ROOT_DIR/backups/$STAMP/"
chmod -R go-rwx "$ROOT_DIR/backups/$STAMP"
ARCHIVE="$ROOT_DIR/backups/bilicraft-$STAMP.tar.gz"
tar -C "$ROOT_DIR/backups" -czf "$ARCHIVE" "$STAMP"
chmod 600 "$ARCHIVE"
rm -rf "$ROOT_DIR/backups/$STAMP"
printf 'Backup: backups/bilicraft-%s.tar.gz\n' "$STAMP"
