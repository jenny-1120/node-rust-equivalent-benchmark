# Node vs Rust Equivalent Benchmark Report (Sample Run)

## 실행 정보
- Scope: DB 제외, `integratedSearch` 유사 앱 로직 비교
- Runtime:
  - Node: TypeScript + Express
  - Rust: Axum + Tokio multi-thread(4 workers)
- Resource limit:
  - Node: `cpus=2.0`, `memory=2g`
  - Rust: `cpus=2.0`, `memory=2g`
- Dataset: `data/seed/integrated-search-like.json` x `DATASET_MULTIPLIER=50` (총 1,500 rows)
- Load:
  - k6 smoke: `vus=10`, `duration=5s` (동일 시나리오)

## 결과 요약 (smoke)
- Node
  - p95: `2.04ms`
  - p99: `5.58ms`
  - RPS: `49.58`
  - failRate: `0.0000`
  - app elapsed p95: `1.00ms`
- Rust
  - p95: `1.44ms`
  - p99: `2.84ms`
  - RPS: `49.68`
  - failRate: `0.0000`
  - app elapsed p95: `0.98ms`

## Delta (Rust vs Node)
- p95: `-29.64%`
- p99: `-49.16%`
- RPS: `+0.20%`
- app elapsed p95: `-2.40%`

## 단계별 비교 확인 방법
- Grafana 대시보드: `Node vs Rust Equivalent`
- 핵심 패널
  - Request p95/p99
  - Request Throughput
  - Stage Avg Duration (`fanOutFilter`, `postProcess`, `imageUrlBuild`, `mergeSort`)

## 1차 판정
- 현재 smoke 결과는 Rust가 지연 지표에서 우세하므로 **Go 후보**
- 단, 최종 판정은 계획된 full 시나리오(램프/반복 실행 3회 이상) 결과로 확정 필요
