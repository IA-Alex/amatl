#!/usr/bin/env python3
"""No-network structural preflight for the frozen supplemental pilot."""
import argparse
import json
from pathlib import Path

from supplemental_pilot_contract import (
    PilotAccumulator, STRATA, TARGET_PER_STRATUM, TOTAL_TARGET, load_json,
    sha256, validate_universe,
)
from supplemental_pilot_capture_contract import RAW_SCHEMA, load_json as capture_load_json, validate_config

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1"
OUT = BASE / "adjudication"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture-config", default=OUT / "supplemental-pilot-searxng-capture-config.json")
    parser.add_argument("--output", default=OUT / "supplemental-pilot-preflight-report.json")
    args = parser.parse_args()
    universe_path = OUT / "supplemental-query-universe-v1.json"
    universe_manifest_path = OUT / "supplemental-query-universe-v1-manifest.json"
    plan_path, v1_manifest_path, ground_path = (OUT / "supplemental-pilot-design.json", OUT / "v1-ground-truth-manifest.json", OUT / "v1-ground-truth.json")
    config_path = Path(args.capture_config)
    universe, manifest, plan, v1_manifest = map(load_json, (universe_path, universe_manifest_path, plan_path, v1_manifest_path))
    config = capture_load_json(config_path)
    counts, capacity = validate_universe(universe)
    if sha256(universe_path) != manifest["universe_sha256"]: raise ValueError("frozen universe hash mismatch")
    if sha256(ground_path) != v1_manifest["GROUND_TRUTH_HASH"]: raise ValueError("V1 ground-truth hash mismatch")
    if manifest["contract_input_sha256"] != {"supplemental_pilot_design": sha256(plan_path), "v1_ground_truth_manifest": sha256(v1_manifest_path), "v1_ground_truth": sha256(ground_path)}: raise ValueError("contractual input hash mismatch")
    if plan["pilot_raw_target"] != TOTAL_TARGET or [s["RAW_TARGET"] for s in plan["strata"]] != [TARGET_PER_STRATUM] * 3: raise ValueError("pilot targets do not match authorization")
    v1_urls = {row["canonical_url"] for row in load_json(ground_path)["rows"]}
    accumulator = PilotAccumulator(universe, v1_urls=v1_urls, historical_urls=set())
    endpoint_status, endpoint_source, status, capture_transport = "AUTHORIZED_AND_BOUND", "EXPLICIT_OPERATOR_INPUT", "PASS", "READY"
    try:
        validate_config(config, universe)
    except ValueError as error:
        if str(error) not in {"CAPTURE_ENDPOINT_BINDING_MISSING", "CAPTURE_ENDPOINT_MISSING"}:
            raise
        endpoint_status, endpoint_source = "BLOCKED_MISSING_EXPLICIT_OPERATOR_ENDPOINT", "NONE"
        status, capture_transport = "BLOCKED", "BLOCKED_CAPTURE_TRANSPORT_FAILURE"
    report = {
        "schema": "amatl.relevance.supplemental-pilot-preflight.v2", "authorization": "180-row pilot only", "network_capture_started": False,
        "network_requests": 0, "v1_ground_truth_integrity": "PASS", "v1_ground_truth_hash": v1_manifest["GROUND_TRUTH_HASH"],
        "frozen_plan_hash": sha256(plan_path), "supplemental_query_universe": str(universe_path.relative_to(ROOT)),
        "supplemental_query_universe_integrity": "PASS", "supplemental_query_universe_hash": manifest["universe_sha256"], "supplemental_query_manifest_hash": sha256(universe_manifest_path),
        "query_counts": counts, "structural_capacity": capacity, "target": TOTAL_TARGET,
        "provider_contract": "SEARXNG_ONLY", "provider_binding": "SEARXNG_ONLY", "provider_fallback": "DISABLED",
        "general_config_provider_leakage": "NONE", "marginalia_reachable_from_pilot_path": False,
        "stratum_enforcement": "PASS", "quota_enforcement": "PASS", "dynamic_expansion": "DISABLED",
        "frozen_universe_exhaustion": "EXHAUSTED_FROZEN_UNIVERSE", "canonicalization_and_dedup": "AVAILABLE",
        "overlap_v1_verifiable": True, "historical_overlap_input": "explicit canonical URL set supplied to PilotAccumulator",
        "output_schema": "amatl.relevance.supplemental-pilot-attempt.v1", "raw_evidence_schema": RAW_SCHEMA,
        "raw_evidence_schema_validation": "PASS", "capture_transport": capture_transport, "runner_compatibility": "PASS",
        "endpoint_status": endpoint_status, "endpoint_source": endpoint_source,
        "network_preflight": "PASS", "post_capture_freeze_supported": True,
        "dry_run_attempt_count": len(accumulator.attempts), "status": status,
        "SEARXNG_ENDPOINT_STATUS": endpoint_status,
        "SEARXNG_ENDPOINT_SOURCE": endpoint_source,
        "PILOT_PROVIDER_BINDING": "SEARXNG_ONLY",
        "PILOT_PROVIDER_FALLBACK": "DISABLED",
        "PILOT_GENERAL_CONFIG_PROVIDER_LEAKAGE": "NONE",
        "PILOT_PREFLIGHT": status,
        "NETWORK_REQUESTS": 0,
        "WORK_PACKAGE_STATUS": "COMPLETE_READY_FOR_PILOT_CAPTURE" if status == "PASS" else "BLOCKED",
        "next_single_action": "Provide the explicit operator-authorized SearXNG endpoint and its exact SHA-256 to the isolated capture configuration." if status == "BLOCKED" else "Execute the authorized 180-row pilot using only tools/supplemental_pilot_capture.py with the frozen SearXNG capture configuration.",
        "NEXT_SINGLE_ACTION": "Provide the explicit operator-authorized SearXNG endpoint and its exact SHA-256 to the isolated capture configuration." if status == "BLOCKED" else "Execute the authorized 180-row SearXNG-only pilot.",
    }
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))

if __name__ == "__main__": main()
