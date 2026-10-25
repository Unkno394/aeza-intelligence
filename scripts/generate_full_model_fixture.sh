#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

output_dir="${1:-scripts/fixtures/full_model_stress}"
python3 scripts/generate.py \
  --accounts 300 --services 900 --transactions 4500 \
  --history-days 120 --id-offset -9700000 \
  --reference 2026-10-25T00:00:00Z --out "$output_dir"

echo "Offline check: cargo run --offline --locked --bin model_smoke -- $output_dir/model_history.json"
echo "Database fixture (test DB only): $output_dir/fixture_insert.sql"
