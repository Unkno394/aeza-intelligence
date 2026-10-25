#!/usr/bin/env bash
set -euo pipefail

# Opt-in longer soak on an already running disposable Compose stack.
: "${DATABASE_URL:?Set DATABASE_URL to the disposable Compose test database}"
: "${PUBLIC_URL:?Set PUBLIC_URL to the disposable Compose application URL}"
: "${E2E_USERNAME:?Set E2E_USERNAME to an active test operator}"

export LOAD_DURATION_SECONDS="${SOAK_DURATION_SECONDS:-600}"
export LOAD_CONCURRENCY="${SOAK_CONCURRENCY:-16}"
export LOAD_REQUEST_TIMEOUT_MS="${SOAK_REQUEST_TIMEOUT_MS:-10000}"
export LOAD_P95_LIMIT_MS="${SOAK_P95_LIMIT_MS:-5000}"
export API_MEMORY_GROWTH_LIMIT_MB="${SOAK_MEMORY_GROWTH_LIMIT_MB:-128}"
export LOAD_SESSION_MINUTES="$(((LOAD_DURATION_SECONDS + 359) / 60 + 5))"

exec bash scripts/check_performance.sh
