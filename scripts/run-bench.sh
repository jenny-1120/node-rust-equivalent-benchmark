#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

mkdir -p results

echo "[1/4] Starting core services..."
docker compose up -d --build node-api rust-api prometheus grafana

echo "[2/4] Running k6 for Node..."
docker compose run --rm k6-node

echo "[3/4] Running k6 for Rust..."
docker compose run --rm k6-rust

echo "[4/4] Summarizing results..."
python3 scripts/summarize_results.py

echo "Done. Check Grafana at http://localhost:3000 and summaries in ./results"
