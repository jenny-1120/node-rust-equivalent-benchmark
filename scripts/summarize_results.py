#!/usr/bin/env python3
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RESULTS = ROOT / "results"


def read_summary(name: str):
    path = RESULTS / name
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


def format_row(name: str, summary: dict):
    p95 = metric(summary, "http_req_duration", "p(95)")
    p99 = metric(summary, "http_req_duration", "p(99)")
    rps = metric(summary, "http_reqs", "rate")
    fail = metric(summary, "http_req_failed", "rate")
    app_p95 = metric(summary, "app_elapsed_ms", "p(95)")
    return {
        "name": name,
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


def main():
    node = read_summary("node-summary.json")
    rust = read_summary("rust-summary.json")

    if not node or not rust:
        print("node-summary.json 또는 rust-summary.json 이 없습니다.")
        return

    node_row = format_row("node", node)
    rust_row = format_row("rust", rust)

    print("=== Node vs Rust Equivalent Result Summary ===")
    print(
        f"Node  : p95={node_row['p95']:.2f}ms p99={node_row['p99']:.2f}ms "
        f"rps={node_row['rps']:.2f} failRate={node_row['fail']:.4f} appP95={node_row['app_p95']:.2f}ms"
    )
    print(
        f"Rust  : p95={rust_row['p95']:.2f}ms p99={rust_row['p99']:.2f}ms "
        f"rps={rust_row['rps']:.2f} failRate={rust_row['fail']:.4f} appP95={rust_row['app_p95']:.2f}ms"
    )

    print("\n=== Rust vs Node Delta ===")
    print(f"p95 delta: {pct_delta(node_row['p95'], rust_row['p95']):.2f}%")
    print(f"p99 delta: {pct_delta(node_row['p99'], rust_row['p99']):.2f}%")
    print(f"rps delta: {pct_delta(node_row['rps'], rust_row['rps']):.2f}%")
    print(f"appP95 delta: {pct_delta(node_row['app_p95'], rust_row['app_p95']):.2f}%")


if __name__ == "__main__":
    main()
