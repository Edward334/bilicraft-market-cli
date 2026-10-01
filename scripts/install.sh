#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v cargo >/dev/null || { echo 'Rust/Cargo is required: https://rustup.rs' >&2; exit 1; }
cd "$ROOT_DIR"
command -v node >/dev/null || { echo 'Node.js >=18 is required to build the embedded Web UI' >&2; exit 1; }
npm --prefix web install --ignore-scripts
npm --prefix web run build
cargo build --release
"$ROOT_DIR/scripts/init.sh"
cp "$ROOT_DIR/systemd/bilicraft-ai-stock.service.example" "$ROOT_DIR/systemd/bilicraft-ai-stock.service"
printf 'Built target/release/bilicraft-ai-stock. Run ./scripts/start.sh.\n'
