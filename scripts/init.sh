#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
mkdir -p data backups
if [[ ! -f config.toml ]]; then cp config.example.toml config.toml; fi
chmod 700 data backups
if [[ ! -f data/secrets.json ]]; then printf '{}\n' > data/secrets.json; chmod 600 data/secrets.json; fi
./target/release/bilicraft-ai-stock init 2>/dev/null || true
printf 'Initialized %s/config.toml and protected data directory.\n' "$ROOT_DIR"
