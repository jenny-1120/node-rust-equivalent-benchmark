#!/usr/bin/env python3
import json
import statistics
from glob import glob
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RESULTS = ROOT / "results"

def read_summary(path: Path):
    if not path.exists():
        return None
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def metric(summary: dict, key: str, field: str, default: float = 0.0):
    try:
        metric_obj = summary["metrics"][key]
        if field in metric_obj:
            return float(metric_obj[field])
        if "values" in metric_obj and field in metric_obj["values"]:
            return float(metric_obj["values"][field])
        return default
    except Exception:
        return default


def format_row(name: str, summary: dict, run_id: str):
    p95 = metric(summary, "http_req_duration", "p(95)")
    p99 = metric(summary, "http_req_duration", "p(99)")
    rps = metric(summary, "http_reqs", "rate")
    fail = metric(summary, "http_req_failed", "rate")
    app_p95 = metric(summary, "app_elapsed_ms", "p(95)")
    return {
        "name": name,
        "run_id": run_id,
        "p95": p95,
        "p99": p99,
        "rps": rps,
        "fail": fail,
        "app_p95": app_p95,
    }


def pct_delta(base: float, target: float):
    if base == 0:
        return 0.0
    return (target - base) / base * 100


def iqr(values):
    if len(values) < 2:
        return 0.0
    q1, q3 = statistics.quantiles(values, n=4, method="inclusive")[0], statistics.quantiles(
        values, n=4, method="inclusive"
    )[2]
    return q3 - q1


def cv(values):
    if len(values) < 2:
        return 0.0
    mean = statistics.mean(values)
    if mean == 0:
        return 0.0
    return statistics.pstdev(values) / mean


def aggregate(rows):
    metrics = ["p95", "p99", "rps", "fail", "app_p95"]
    agg = {}
    for key in metrics:
        values = [row[key] for row in rows]
        agg[key] = {
            "median": statistics.median(values),
            "mean": statistics.mean(values),
            "iqr": iqr(values),
            "cv": cv(values),
        }
    return agg


def load_runs(service: str):
    paths = sorted(Path(x) for x in glob(str(RESULTS / f"{service}-run-*.json")))
    rows = []
    for path in paths:
        summary = read_summary(path)
        if summary is None:
            continue
        run_id = path.stem.replace(f"{service}-run-", "")
        rows.append(format_row(service, summary, run_id))

    # Backward compatibility for old single-run outputs.
    if not rows:
        legacy_path = RESULTS / f"{service}-summary.json"
        legacy = read_summary(legacy_path)
        if legacy is not None:
            rows.append(format_row(service, legacy, "legacy"))
    return rows


def main():
    node_rows = load_runs("node")
    rust_rows = load_runs("rust")

    if not node_rows or not rust_rows:
        print("node-run-*.json 또는 rust-run-*.json 결과가 없습니다.")
        return

    node_agg = aggregate(node_rows)
    rust_agg = aggregate(rust_rows)

    print("=== Node vs Rust Equivalent Result Summary (Median of Runs) ===")
    print(f"runs: node={len(node_rows)}, rust={len(rust_rows)}")
    print(
        f"Node  : p95={node_agg['p95']['median']:.2f}ms p99={node_agg['p99']['median']:.2f}ms "
        f"rps={node_agg['rps']['median']:.2f} failRate={node_agg['fail']['median']:.4f} "
        f"appP95={node_agg['app_p95']['median']:.2f}ms"
    )
    print(
        f"Rust  : p95={rust_agg['p95']['median']:.2f}ms p99={rust_agg['p99']['median']:.2f}ms "
        f"rps={rust_agg['rps']['median']:.2f} failRate={rust_agg['fail']['median']:.4f} "
        f"appP95={rust_agg['app_p95']['median']:.2f}ms"
    )

    print("\n=== Rust vs Node Delta ===")
    print(f"p95 delta: {pct_delta(node_agg['p95']['median'], rust_agg['p95']['median']):.2f}%")
    print(f"p99 delta: {pct_delta(node_agg['p99']['median'], rust_agg['p99']['median']):.2f}%")
    print(f"rps delta: {pct_delta(node_agg['rps']['median'], rust_agg['rps']['median']):.2f}%")
    print(
        f"appP95 delta: "
        f"{pct_delta(node_agg['app_p95']['median'], rust_agg['app_p95']['median']):.2f}%"
    )

    print("\n=== Run Variability (lower is better) ===")
    print(
        f"Node  : p95 IQR={node_agg['p95']['iqr']:.2f}ms, "
        f"rps CV={node_agg['rps']['cv'] * 100:.2f}%"
    )
    print(
        f"Rust  : p95 IQR={rust_agg['p95']['iqr']:.2f}ms, "
        f"rps CV={rust_agg['rps']['cv'] * 100:.2f}%"
    )


if __name__ == "__main__":
    main()
