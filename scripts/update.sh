#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
"$ROOT_DIR/scripts/backup.sh"
npm --prefix web install --ignore-scripts
npm --prefix web run build
cargo build --release
"$ROOT_DIR/target/release/bilicraft-ai-stock" --root "$ROOT_DIR" status
