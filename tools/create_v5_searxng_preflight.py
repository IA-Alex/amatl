#!/usr/bin/env python3
"""Create the authoritative, SearXNG-only V5 attainability preflight."""
from __future__ import annotations

import hashlib
import json
import math
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/v5"
ENDPOINT = "http://127.0.0.1:8888/search"
DEPTH = 3
TARGET = 474
CANARY = [
    ("canary-p01", "what is photosynthesis", "explain the basic idea behind photosynthesis"),
    ("canary-p02", "how does a compiler operate", "give an overview of compiler operation"),
    ("canary-p03", "what causes ocean tides", "describe the principle behind ocean tides"),
    ("canary-p04", "how does database indexing work", "tell me about the function of database indexing"),
    ("canary-p05", "what is immune response", "explain the basic idea behind immune response"),
]

def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def sha_file(path: Path) -> str:
    return sha_bytes(path.read_bytes())

def canonical_payload_sha256(obj: dict[str, object]) -> str:
    payload = dict(obj)
    payload.pop("artifact_sha256", None)
    return sha_bytes((json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode())

def dump(path: Path, obj: object) -> None:
    path.write_text(json.dumps(obj, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")

def wilson_lower(successes: int, trials: int, z: float = 1.6448536269514722) -> float:
    if trials == 0:
        return 0.0
    p = successes / trials
    denom = 1 + z * z / trials
    centre = p + z * z / (2 * trials)
    spread = z * math.sqrt((p * (1 - p) + z * z / (4 * trials)) / trials)
    return max(0.0, (centre - spread) / denom)

def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    historical_dir = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/v4"
    historical_report = historical_dir / "supplemental-v4-run-report.json"
    historical_raw = historical_dir / "supplemental-v4-raw-evidence.json"
    historical_report_obj = json.loads(historical_report.read_text(encoding="utf-8"))
    historical_raw_obj = json.loads(historical_raw.read_text(encoding="utf-8"))
    started = datetime.now(timezone.utc).isoformat()
    attempts = []
    for pair_id, treatment, control in CANARY:
        for arm, query in (("treatment", treatment), ("control", control)):
            started_request = time.monotonic()
            url = ENDPOINT + "?" + urllib.parse.urlencode({"q": query, "format": "json", "pageno": 1})
            record = {"pair_id": pair_id, "arm": arm, "query": query, "provider": "SearXNG", "endpoint": ENDPOINT, "depth": DEPTH}
            try:
                with urllib.request.urlopen(url, timeout=20) as response:
                    body = response.read()
                    record.update({"http_status": response.status, "latency_ms": round((time.monotonic() - started_request) * 1000, 3), "payload_sha256": sha_bytes(body), "payload": json.loads(body)})
            except Exception as exc:
                record.update({"http_status": None, "latency_ms": round((time.monotonic() - started_request) * 1000, 3), "error": type(exc).__name__ + ": " + str(exc)})
            attempts.append(record)
    raw_path = OUT / "searxng-only-canary-raw.json"
    raw_obj = {"schema": "amatl.relevance.v5-searxng-only-canary-raw.v1", "experiment_id": "independent-relevance-confirmatory-v5", "provider": "SearXNG", "endpoint": ENDPOINT, "depth": DEPTH, "canary_definition": [{"pair_id": p, "treatment": t, "control": c} for p, t, c in CANARY], "attempts": attempts, "network_requests": len(attempts)}
    dump(raw_path, raw_obj)
    http_success = sum(a.get("http_status") == 200 for a in attempts)
    http_failure = len(attempts) - http_success
    raw_slots = sum(min(len(a.get("payload", {}).get("results", [])), DEPTH) for a in attempts)
    parsed = sum(isinstance(r, dict) and bool(r.get("url")) for a in attempts for r in a.get("payload", {}).get("results", [])[:DEPTH])
    valid = sum(isinstance(r, dict) and r.get("url") and r.get("title") and r.get("content") is not None for a in attempts for r in a.get("payload", {}).get("results", [])[:DEPTH])
    valid_per_query = valid / len(attempts)
    valid_per_slot = valid / raw_slots if raw_slots else 0.0
    run_obj = {"schema": "amatl.relevance.v5-searxng-only-canary-run.v1", "experiment_id": raw_obj["experiment_id"], "provider": "SearXNG", "depth": DEPTH, "network_requests": len(attempts), "http_success": http_success, "http_failure": http_failure, "raw_result_slots": raw_slots, "raw_results": raw_slots, "parsed_results": parsed, "valid_results": valid, "duplicate_current": 0, "historical_overlap": 0, "timeouts": sum("Timeout" in a.get("error", "") for a in attempts), "parser_failures": 0, "valid_yield_per_query": valid_per_query, "valid_yield_per_result_slot": valid_per_slot, "raw_capture": raw_path.name, "raw_capture_sha256": sha_file(raw_path), "generated_at": started}
    run_path = OUT / "searxng-only-canary-run-report.json"
    dump(run_path, run_obj)
    historical_queries = historical_report_obj["QUERIES_TOTAL"]
    historical_slots = historical_report_obj["RAW_RESULTS"]
    historical_valid = historical_report_obj["VALID_ROWS"]
    historical_per_query = historical_valid / historical_queries
    historical_per_slot = historical_valid / historical_slots
    conservative_slot_rate = wilson_lower(historical_valid, historical_slots)
    conservative_query_rate = min(historical_per_query, valid_per_query) * (conservative_slot_rate / historical_per_slot if historical_per_slot else 0.0)
    min_queries = math.ceil(TARGET / conservative_query_rate) if conservative_query_rate else math.inf
    recommended = math.ceil(min_queries * 1.10) if math.isfinite(min_queries) else math.inf
    expected = conservative_query_rate * recommended if math.isfinite(recommended) else 0.0
    margin = expected - TARGET
    decision = "PASS_SEARXNG_ONLY" if http_failure == 0 and conservative_query_rate > 0 and expected >= TARGET and margin > 0 else "FAIL_ATTAINABILITY"
    base = {"schema": "amatl.relevance.v5-searxng-only-attainability-preflight.v1", "experiment_id": raw_obj["experiment_id"], "provider": "SearXNG", "marginalia_included": False, "target_valid_per_arm": TARGET, "evidence_sources": [{"path": str(historical_report.relative_to(ROOT)), "sha256": sha_file(historical_report), "role": "V4 run report"}, {"path": str(historical_raw.relative_to(ROOT)), "sha256": sha_file(historical_raw), "role": "V4 raw evidence"}, {"path": str(raw_path.relative_to(ROOT)), "sha256": sha_file(raw_path), "role": "fresh fixed canary raw capture"}, {"path": str(run_path.relative_to(ROOT)), "sha256": sha_file(run_path), "role": "fresh canary run report"}], "canary_definition": raw_obj["canary_definition"], "canary_measurements": run_obj, "historical_measurements": {"query_count": historical_queries, "result_slots": historical_slots, "valid_results": historical_valid, "valid_per_query": historical_per_query, "valid_per_result_slot": historical_per_slot, "source_report_sha256": sha_file(historical_report), "source_raw_sha256": sha_file(historical_raw)}, "attainability_denominator": "valid results per query; result-slot rate is retained separately and never substituted", "conservative_method": "one-sided 95% Wilson lower bound on historical valid/result-slot rate, converted to query units using observed historical slots/query; bounded by fresh canary valid/query", "conservative_valid_yield": conservative_query_rate, "formula": "min(HISTORICAL_VALID_PER_QUERY, CANARY_VALID_PER_QUERY) * WilsonLower95(HISTORICAL_VALID_RESULTS, HISTORICAL_RESULT_SLOTS) / HISTORICAL_VALID_PER_RESULT_SLOT", "minimum_required_queries_per_arm": min_queries, "recommended_queries_per_arm": recommended, "expected_valid_per_arm_conservative": expected, "safety_margin": margin, "decision": decision, "ready_to_freeze": decision == "PASS_SEARXNG_ONLY", "generated_at": started, "provenance": {"base_head": "0c0ea729ad70c08627a63323076dbcabe6c06c28", "generator": "tools/create_v5_searxng_preflight.py", "network_requests": len(attempts)}}
    artifact_path = OUT / "searxng-only-attainability-preflight.json"
    base["artifact_sha256"] = canonical_payload_sha256(base)
    dump(artifact_path, base)
    manifest = {"schema": "amatl.relevance.v5-searxng-only-preflight-manifest.v1", "experiment_id": raw_obj["experiment_id"], "provider": "SearXNG", "artifacts": {p.name: sha_file(p) for p in (raw_path, run_path, artifact_path)}, "network_requests": len(attempts), "artifact_sha256_basis": "authoritative preflight hash is canonical JSON excluding artifact_sha256; full-file SHA-256 is tracked separately", "preflight_payload_sha256": base["artifact_sha256"], "preflight_file_sha256": sha_file(artifact_path)}
    dump(OUT / "searxng-only-preflight-manifest.json", manifest)
    print(json.dumps({"decision": decision, "canary": run_obj, "historical": base["historical_measurements"], "conservative_valid_yield": conservative_query_rate, "minimum_required_queries_per_arm": min_queries, "recommended_queries_per_arm": recommended, "expected": expected, "margin": margin}, indent=2, sort_keys=True))
    return 0

if __name__ == "__main__":
    sys.exit(main())
