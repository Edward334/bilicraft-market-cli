#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
[[ -x target/release/bilicraft-ai-stock ]] || { echo 'Binary missing; run ./scripts/install.sh (or ./scripts/install.sh source)' >&2; exit 1; }
exec "$ROOT_DIR/target/release/bilicraft-ai-stock" --root "$ROOT_DIR" serve
