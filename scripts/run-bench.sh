#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

mkdir -p results

RUNS="${RUNS:-6}"
if (( RUNS < 2 )); then
  echo "RUNS must be >= 2"
  exit 1
fi

# Near-saturation defaults for Node/Rust 4-core comparison.
K6_STAGE1_TARGET="${K6_STAGE1_TARGET:-80}"
K6_STAGE2_TARGET="${K6_STAGE2_TARGET:-140}"
K6_STAGE3_TARGET="${K6_STAGE3_TARGET:-200}"
K6_PRE_ALLOCATED_VUS="${K6_PRE_ALLOCATED_VUS:-250}"
K6_MAX_VUS="${K6_MAX_VUS:-500}"
K6_THINK_TIME_MS="${K6_THINK_TIME_MS:-0}"

wait_for_health() {
  local name="$1"
  local url="$2"
  local retries=120
  local delay=1
  local i
  for ((i=1; i<=retries; i++)); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      echo "  - $name healthy"
      return 0
    fi
    sleep "$delay"
  done
  echo "  - $name failed health check: $url"
  return 1
}

restart_target() {
  local target="$1"
  echo "  - restarting $target for clean run state"
  docker compose restart "$target" >/dev/null
  if [[ "$target" == "node-api" ]]; then
    wait_for_health "node-api" "http://localhost:3101/health"
  else
    wait_for_health "rust-api" "http://localhost:3102/health"
  fi
}

run_k6() {
  set +e
  docker compose run --rm \
    k6-bench \
    run /scripts/integrated-search-like.js \
    --env K6_STAGE1_TARGET="$K6_STAGE1_TARGET" \
    --env K6_STAGE2_TARGET="$K6_STAGE2_TARGET" \
    --env K6_STAGE3_TARGET="$K6_STAGE3_TARGET" \
    --env K6_PRE_ALLOCATED_VUS="$K6_PRE_ALLOCATED_VUS" \
    --env K6_MAX_VUS="$K6_MAX_VUS" \
    --env K6_THINK_TIME_MS="$K6_THINK_TIME_MS" \
    "$@"
  local status=$?
  set -e
  if (( status == 0 )); then
    return 0
  fi
  if (( status == 99 )); then
    echo "  - warning: k6 thresholds missed (exit 99); continuing"
    return 0
  fi
  echo "  - k6 failed with exit ${status}"
  return "$status"
}

run_once() {
  local service="$1"
  local run_id="$2"
  local out_file="/results/${service}-run-${run_id}.json"
  local target_url

  if [[ "$service" == "node" ]]; then
    target_url="http://node-api:3001"
    restart_target "node-api"
  else
    target_url="http://rust-api:3002"
    restart_target "rust-api"
  fi

  echo "  - benchmark $service run ${run_id}"
  run_k6 \
    --env TARGET_URL="$target_url" \
    --env K6_WARMUP=0 \
    --summary-export "$out_file"
}

echo "[1/5] Starting core services..."
docker compose up -d --build node-api rust-api cadvisor prometheus grafana

echo "k6 profile: stage=${K6_STAGE1_TARGET}/${K6_STAGE2_TARGET}/${K6_STAGE3_TARGET} rps, preVUs=${K6_PRE_ALLOCATED_VUS}, maxVUs=${K6_MAX_VUS}, thinkTimeMs=${K6_THINK_TIME_MS}"

echo "[2/5] Waiting for API readiness..."
wait_for_health "node-api" "http://localhost:3101/health"
wait_for_health "rust-api" "http://localhost:3102/health"

echo "[3/5] Warm-up (excluded from summary)..."
run_k6 \
  --env TARGET_URL="http://node-api:3001" \
  --env K6_WARMUP=1 \
  --summary-export /results/warmup-node.json >/dev/null
run_k6 \
  --env TARGET_URL="http://rust-api:3002" \
  --env K6_WARMUP=1 \
  --summary-export /results/warmup-rust.json >/dev/null

echo "[4/5] Main benchmark runs (alternating order)..."
for idx in $(seq 1 "$RUNS"); do
  run_tag="$(printf "%02d" "$idx")"
  if (( idx % 2 == 1 )); then
    echo "- Round $idx/$RUNS: node -> rust"
    run_once "node" "$run_tag"
    run_once "rust" "$run_tag"
  else
    echo "- Round $idx/$RUNS: rust -> node"
    run_once "rust" "$run_tag"
    run_once "node" "$run_tag"
  fi
done

echo "[5/5] Summarizing results..."
python3 scripts/summarize_results.py

echo "Done. Check Grafana at http://localhost:3300 and run summaries in ./results"
