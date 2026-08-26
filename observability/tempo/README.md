# Tempo (Optional)

현재 벤치마크 기본 경로는 **Prometheus + Grafana 메트릭 비교**입니다.
트레이싱(Tempo/OTLP)은 아직 기본 `docker-compose.yml` 실행 경로에 포함되지 않습니다.

## 현재 기준(메트릭 비교)
- 벤치 실행: `./scripts/run-bench.sh`
- 반복 실행 수: `RUNS` 환경변수(기본 6)
- 공통 데이터셋 설정: `services/common.env`
- 서비스별 설정: `services/node-api/.env`, `services/rust-api/.env`

## Tempo를 붙일 때 권장
- Node/Rust 모두 동일한 trace 샘플링/전송 정책 적용
- 벤치 구간(warm-up 제외)만 비교 대상으로 필터링
- 메트릭 비교 결과(`results/*-run-*.json`)와 동일한 실행 회차를 trace에서도 매칭
