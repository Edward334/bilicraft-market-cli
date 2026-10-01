#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
[[ -x target/release/bilicraft-ai-stock ]] || cargo build --release
exec "$ROOT_DIR/target/release/bilicraft-ai-stock" --root "$ROOT_DIR" login
