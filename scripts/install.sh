#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-release}"
mkdir -p "$ROOT_DIR/data" "$ROOT_DIR/backups" "$ROOT_DIR/target/release"
if [[ -d "$ROOT_DIR/.git" ]] && [[ -z "$(git -C "$ROOT_DIR" status --porcelain 2>/dev/null || true)" ]]; then
  git -C "$ROOT_DIR" pull --ff-only origin main
fi
if [[ ! -f "$ROOT_DIR/config.toml" ]]; then cp "$ROOT_DIR/config.example.toml" "$ROOT_DIR/config.toml"; fi
if [[ ! -f "$ROOT_DIR/data/secrets.json" ]]; then printf '{}\n' > "$ROOT_DIR/data/secrets.json"; fi
chmod 700 "$ROOT_DIR/data" "$ROOT_DIR/backups"
chmod 600 "$ROOT_DIR/data/secrets.json"
case "$MODE" in
  release) "$ROOT_DIR/scripts/update.sh" release ;;
  source)
    command -v cargo >/dev/null || { echo 'Rust/Cargo is required for source builds: https://rustup.rs' >&2; exit 1; }
    command -v node >/dev/null || { echo 'Node.js >=18 is required to build the embedded Web UI' >&2; exit 1; }
    npm --prefix "$ROOT_DIR/web" ci --ignore-scripts
    npm --prefix "$ROOT_DIR/web" run build
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --release --locked
    ;;
  *) echo "Usage: $0 [release|source]" >&2; exit 2 ;;
esac
"$ROOT_DIR/scripts/init.sh"
cp "$ROOT_DIR/systemd/bilicraft-ai-stock.service.example" "$ROOT_DIR/systemd/bilicraft-ai-stock.service"
printf 'Installed %s. Configure Microsoft login with ./scripts/login.sh, then start with ./scripts/start.sh.\n' "$ROOT_DIR/target/release/bilicraft-ai-stock"
