# Node vs Rust Equivalent Benchmark

`integratedSearch`와 유사한 앱 레이어 로직(fan-out, 후처리, URL 가공, 정렬)을 DB 없이 동일 조건에서 비교하는 로컬 벤치마크 프로젝트입니다.

## 목표
- DB I/O를 제거하고 TypeScript(Node)와 Rust의 순수 처리 성능 차이를 비교
- 동일 Docker CPU/메모리 제약에서 p95/p99, 처리량, 단계별 처리시간을 시각화

## 구성
- `services/node-api`: TypeScript 기준 구현
- `services/rust-api`: Rust 동등 구현 (`tokio` multi-thread)
- `data/seed`: 고정 입력 데이터셋
- `bench/k6`: 부하 시나리오
- `observability/prometheus`, `observability/grafana`: 메트릭 수집/시각화
- `scripts`: 실행 자동화 및 결과 요약

## 빠른 시작
1. 컨테이너 빌드/실행
   - `docker compose up -d --build node-api rust-api prometheus grafana`
2. 공정 비교 자동 실행(권장)
   - `./scripts/run-bench.sh`
3. 반복 횟수 지정 실행(예: 8회)
   - `RUNS=8 ./scripts/run-bench.sh`
4. 단건 수동 실행(디버깅용)
   - `docker compose run --rm k6-bench run /scripts/integrated-search-like.js --env TARGET_URL=http://node-api:3001 --env K6_WARMUP=0 --summary-export /results/debug-node.json`

## 대시보드
- Grafana: [http://localhost:3300](http://localhost:3300)
  - 기본 계정: `admin` / `admin`
- Prometheus: [http://localhost:39090](http://localhost:39090)

## 공정 비교 원칙
- 동일 요청 payload, 동일 데이터셋 seed, 동일 컨테이너 자원
- Node/Rust 모두 캐시 off
- 단발 측정 금지: 워밍업 제외 후 반복 측정(기본 6회, 권장 6~10회)
- 순서 편향 제거: 라운드마다 Node→Rust / Rust→Node 교차 실행
- 요청 분포 고정: k6 랜덤 대신 iteration 기반 순환 payload 사용
- 부하 모델 고정: closed model 대신 arrival-rate 기반 open model 사용
- 최종 비교는 Grafana 시각값이 아닌 `results/*-run-*.json` 집계 기준

## 환경 변수
서버 설정은 서비스별 `.env`에 있습니다. `docker compose`가 `env_file`로 주입합니다.

- Node: `services/node-api/.env`
- Rust: `services/rust-api/.env`

코드에도 기본값이 있어서, 변수를 빼도 서버는 뜹니다. 데이터 규모를 바꿀 때는 아래 두 파일의 `DATASET_MULTIPLIER`를 **같은 값**으로 맞추세요.

| 변수 | Node 기본 | Rust 기본 | 설명 |
|---|---|---|---|
| `PORT` | `3001` | `3002` | HTTP 포트 |
| `SERVICE_NAME` | `node-api` | `rust-api` | 메트릭/로그에 붙는 서비스명 |
| `DATASET_PATH` | `/app/data/seed/integrated-search-like.json` | 동일 | 컨테이너 안 seed JSON 경로 |
| `DATASET_MULTIPLIER` | `50` | `50` | seed 복제 배수. 원본 30행 × 이 값 = 메모리 row 수 |
| `RUST_LOG` | — | `info` | Rust만 사용. 로그 레벨 |

로컬에서 Docker 없이 띄울 때는 `DATASET_PATH`를 저장소 기준 `data/seed/integrated-search-like.json`으로 바꾸면 됩니다.

## 벤치마크 실행 규약
- `scripts/run-bench.sh`는 아래 순서를 자동 수행합니다.
  - 서비스 기동 및 헬스체크 대기
  - Node/Rust 워밍업(결과 제외)
  - 메인 측정 반복 실행(교차 순서)
  - `scripts/summarize_results.py`로 run별 결과 중앙값 집계
- 결과 파일:
  - `results/node-run-XX.json`
  - `results/rust-run-XX.json`
  - `results/warmup-*.json` (참고용, 비교 제외)
