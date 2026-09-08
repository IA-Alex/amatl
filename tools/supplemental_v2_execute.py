#!/usr/bin/env python3
"""Authorized execution work package for the frozen S1-only supplemental-v2 run.

This is the explicit operator-authorized execution referenced by
``supplemental_v2_capture.py``.  It runs the 12-query frozen S1 universe
against the operator-bound SearXNG endpoint, applies the immutable multi-run
acceptance gate (``supplemental_v2_contract.V2Accumulator``), stops
deterministically when ``S1_NEW_VALID_TARGET=8`` is reached, and emits the
versioned v2 artifacts:

  * ``supplemental-v2-raw-evidence.json``  (raw transport evidence)
  * ``supplemental-v2-attempts.json``      (attempt / rejection ledger)
  * ``supplemental-v2-run-report.json``    (run report with hashes)

Network is opened only with ``--allow-network``; the default fixture mode is
offline and exists for reproducible contract validation.
"""
import argparse
import hashlib
import json
import socket
import subprocess
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import parse_qsl, urlencode, urlsplit
from urllib.request import urlopen

from supplemental_pilot_capture_contract import validate_endpoint_binding
from supplemental_pilot_contract import canonical_url
from supplemental_v2_contract import TARGET, STRATUM, build_accumulator

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
DEFAULT_CONFIG = OUT / "supplemental-v2-searxng-capture-config.json"

RAW_SCHEMA = "amatl.relevance.supplemental-v2-raw-evidence.v1"
LEDGER_SCHEMA = "amatl.relevance.supplemental-v2-attempt.v1"
REPORT_SCHEMA = "amatl.relevance.supplemental-v2-run-report.v1"
EXPERIMENT_ID = "independent-relevance-supplemental-v2"
CONFIG_SCHEMA = "amatl.relevance.supplemental-v2-searxng-capture-config.v1"

V2_CONFIG_REQUIRED = {
    "provider": "searxng",
    "provider_fallback": "none",
    "dynamic_provider_selection": False,
    "dynamic_query_expansion": False,
    "s1_new_valid_target": TARGET,
}
V2_RETRY = {
    "max_attempts": 2,
    "same_query_only": True,
    "retryable_error_classes": ["TIMEOUT", "TEMPORARY_HTTP_FAILURE"],
}

# Map the contract's internal reason codes onto the reporting taxonomy in the
# authorization (section 4/11).  Rejection counts are acquisition aliases and
# are reported separately from final-corpus integrity counts.
REJECTION_MAP = {
    "OVERLAP_V1_GROUND_TRUTH": "OVERLAP_V1",
    "OVERLAP_HISTORICAL": "OVERLAP_HISTORICAL",
    "OVERLAP_SUPPLEMENTAL_V1": "OVERLAP_SUPPLEMENTAL_V1",
    "DUPLICATE_CURRENT_V2": "DUPLICATE_CURRENT_V2",
    "INVALID_RESULT": "INVALID_RESULT",
    "OUTSIDE_ALLOWED_DEPTH": "OUTSIDE_ALLOWED_DEPTH",
    "OUTSIDE_PROVIDER_CONTRACT": "PROVIDER_CONTRACT_VIOLATION",
    "OUTSIDE_STRATUM_CONTRACT": "OUTSIDE_STRATUM_CONTRACT",
    "S1_NEW_VALID_TARGET_FILLED": "OTHER",
    "OUTSIDE_FROZEN_UNIVERSE": "OTHER",
}
OTHER_REJECTION_KEYS = {
    "OTHER", "PROVIDER_CONTRACT_VIOLATION", "INVALID_RESULT",
    "OUTSIDE_ALLOWED_DEPTH", "OUTSIDE_STRATUM_CONTRACT",
}


def now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n"


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def head_commit():
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()


def repository_state():
    state = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    return "dirty" if state else "clean"


class TemporaryHTTPFailure(Exception):
    pass


def normalise_result(result, rank):
    original_url = result.get("url") or result.get("original_url") or ""
    parts = urlsplit(original_url)
    sensitive = {"token", "key", "api_key", "apikey", "password", "secret", "cookie", "authorization"}
    if parts.username or parts.password or any(key.lower() in sensitive for key, _ in parse_qsl(parts.query, keep_blank_values=True)):
        raise ValueError("CAPTURE_RESULT_URL_CONTAINS_SECRET")
    audit_fields = {key: result[key] for key in ("engine", "engines", "category", "score", "parsed_url", "template") if key in result}
    return {"rank": rank, "title": result.get("title") or "", "original_url": original_url,
            "snippet": result.get("content") or result.get("snippet") or result.get("description") or "",
            "provider_specific_identifier": result.get("id"), "raw_fields": audit_fields}


def network_response(endpoint, query, timeout_seconds):
    url = endpoint.rstrip("/") + "/search?" + urlencode({"q": query, "format": "json"})
    with urlopen(url, timeout=timeout_seconds) as response:  # nosec B310: explicit operator endpoint, only after --allow-network
        return response.status, json.loads(response.read().decode("utf-8"))


def validate_config(config, universe):
    if config.get("schema") != CONFIG_SCHEMA:
        raise ValueError("V2_CAPTURE_CONFIG_SCHEMA_MISMATCH")
    if config.get("query_universe_id") != universe.get("universe_id"):
        raise ValueError("V2_UNIVERSE_ID_MISMATCH")
    if config.get("query_universe_hash") != hashlib.sha256((ROOT / config["universe_path"]).read_bytes()).hexdigest():
        raise ValueError("V2_UNIVERSE_HASH_MISMATCH")
    if any(config.get(key) != value for key, value in V2_CONFIG_REQUIRED.items()):
        raise ValueError("V2_TRANSPORT_CONTRACT_FAILURE")
    if config["retry_policy"] != V2_RETRY:
        raise ValueError("V2_RETRY_POLICY_FAILURE")
    if config["rank_depth_policy"].get("source") != "frozen_universe_permitted_positions":
        raise ValueError("V2_RANK_POLICY_FAILURE")
    if config.get("authorized_provider") != "SEARXNG_ONLY":
        raise ValueError("V2_AUTHORIZED_PROVIDER_FAILURE")
    validate_endpoint_binding(config)
    return config


def execute(config, universe, fixture_responses=None, allow_network=False):
    """Capture the frozen S1 universe and apply the immutable v2 gate.

    Stops fetching new queries deterministically once ``TARGET`` accepted rows
    are accumulated (STOP EARLY).  Results already captured for the stopping
    query are still processed by the gate (they become ``S1_NEW_VALID_TARGET_FILLED``
    rejections, never accepted overflow).
    """
    validate_config(config, universe)
    endpoint = validate_endpoint_binding(config)
    gate = build_accumulator(universe)
    gate_records = []
    attempts = []
    sequence = 0
    network_requests = 0
    successful_responses = 0
    failed_responses = 0
    technical_retries = 0
    retry_policy = config["retry_policy"]
    for query in universe["queries"]:
        if len(gate.accepted) >= TARGET:
            break  # STOP EARLY at S1_NEW_VALID_TARGET=8; no overflow acquisition.
        source_attempts = (fixture_responses or {}).get(query["query_id"], [None])
        for attempt_number in range(1, retry_policy["max_attempts"] + 1):
            sequence += 1
            requested_at = now()
            record = {"query_id": query["query_id"], "query_text": query["query"],
                      "assigned_stratum": query["assigned_stratum"], "provider": "searxng",
                      "request_sequence": sequence, "attempt_number": attempt_number,
                      "requested_at": requested_at, "allowed_rank_range": query["permitted_positions"]}
            try:
                if fixture_responses is not None:
                    payload = source_attempts[attempt_number - 1]
                    if isinstance(payload, dict) and "raise" in payload:
                        raise TimeoutError(payload["raise"])
                    http_status, response = 200, payload
                else:
                    if not allow_network:
                        raise ValueError("CAPTURE_NETWORK_NOT_AUTHORIZED")
                    network_requests += 1
                    http_status, response = network_response(endpoint, query["query"], 20)
                if http_status != 200:
                    raise TemporaryHTTPFailure(f"HTTP {http_status}")
                successful_responses += 1
                all_results = response.get("results", []) if isinstance(response, dict) else []
                results = [normalise_result(result, rank)
                           for rank, result in enumerate(all_results, start=1)
                           if rank in query["permitted_positions"]]
                record.update({"completed_at": now(), "http_status": http_status,
                               "provider_response_status": "SUCCESS",
                               "result_count": len(results), "results": results})
                attempts.append(record)
                for result in results:
                    gate_records.append(gate.accept_or_reject(query["query_id"], "searxng", result))
                break
            except (TimeoutError, socket.timeout) as error:
                retryable = attempt_number < retry_policy["max_attempts"]
                if not retryable:
                    failed_responses += 1
                record.update({"completed_at": now(), "http_status": None,
                               "provider_response_status": "ERROR", "result_count": 0, "results": [],
                               "error": {"error_class": "TIMEOUT", "retryable": retryable,
                                         "attempt_number": attempt_number, "query_id": query["query_id"],
                                         "timestamp": now()}})
                attempts.append(record)
                if retryable:
                    technical_retries += 1
                else:
                    break
            except TemporaryHTTPFailure as error:
                retryable = attempt_number < retry_policy["max_attempts"]
                if not retryable:
                    failed_responses += 1
                record.update({"completed_at": now(), "http_status": 502,
                               "provider_response_status": "ERROR", "result_count": 0, "results": [],
                               "error": {"error_class": "TEMPORARY_HTTP_FAILURE", "retryable": retryable,
                                         "attempt_number": attempt_number, "query_id": query["query_id"],
                                         "timestamp": now()}})
                attempts.append(record)
                if retryable:
                    technical_retries += 1
                else:
                    break
            except Exception as error:
                if isinstance(error, ValueError) and str(error) == "CAPTURE_NETWORK_NOT_AUTHORIZED":
                    raise
                failed_responses += 1
                record.update({"completed_at": now(), "http_status": None,
                               "provider_response_status": "ERROR", "result_count": 0, "results": [],
                               "error": {"error_class": "TRANSPORT_FAILURE", "retryable": False,
                                         "attempt_number": attempt_number, "query_id": query["query_id"],
                                         "timestamp": now()}})
                attempts.append(record)
                break
    return gate, gate_records, attempts, {
        "network_requests": network_requests,
        "successful_responses": successful_responses,
        "failed_responses": failed_responses,
        "technical_retries": technical_retries,
    }


def build_artifacts(universe, config, gate, gate_records, attempts, counters, output_dir, start_commit):
    evidence = {"schema_version": RAW_SCHEMA, "experiment_id": EXPERIMENT_ID,
                "query_universe_id": universe["universe_id"],
                "query_universe_hash": config["query_universe_hash"],
                "provider": "searxng", "attempts": attempts}
    evidence_hash = hashlib.sha256(canonical_json(evidence).encode("utf-8")).hexdigest()
    evidence["artifact_sha256"] = evidence_hash

    valid = len(gate.accepted)
    ledger = {"schema": LEDGER_SCHEMA, "experiment_id": EXPERIMENT_ID,
              "network_requests": counters["network_requests"],
              "source": "authorized S1-only v2 SearXNG execution; contract applied by V2Accumulator",
              "valid_counts": {"S1_PROCEDURAL_SHALLOW": valid},
              "total_valid": valid,
              "status": "COMPLETE" if valid >= TARGET else "EXHAUSTED_FROZEN_UNIVERSE",
              "attempts": gate_records}
    ledger_hash = hashlib.sha256(canonical_json(ledger).encode("utf-8")).hexdigest()

    reasons = Counter(a["reason_code"] for a in gate_records)
    rejected = [a for a in gate_records if a["reason_code"] != "ACCEPTED"]
    mapped = Counter(REJECTION_MAP.get(a["reason_code"], "OTHER") for a in rejected)
    other = sum(v for k, v in mapped.items() if k in OTHER_REJECTION_KEYS)

    manifest_hash = hashlib.sha256((OUT / "supplemental-query-universe-v2-manifest.json").read_bytes()).hexdigest()
    report = {
        "schema": REPORT_SCHEMA,
        "REPOSITORY_STATE": repository_state(),
        "START_COMMIT": start_commit,
        "SEARXNG_ENDPOINT_STATUS": "AUTHORIZED_AND_BOUND",
        "SEARXNG_ENDPOINT_SOURCE": "EXPLICIT_OPERATOR_INPUT",
        "SUPPLEMENTAL_V2_UNIVERSE_HASH": config["query_universe_hash"],
        "SUPPLEMENTAL_V2_MANIFEST_HASH": manifest_hash,
        "SUPPLEMENTAL_V2_EXECUTION_STATUS": "COMPLETE" if valid >= TARGET else "EXHAUSTED_FROZEN_UNIVERSE",
        "SUPPLEMENTAL_V2_PROVIDER_EFFECTIVE": "searxng",
        "SUPPLEMENTAL_V2_RAW_ATTEMPTS": len(attempts),
        "SUPPLEMENTAL_V2_NETWORK_REQUESTS": counters["network_requests"],
        "SUPPLEMENTAL_V2_TECHNICAL_RETRIES": counters["technical_retries"],
        "SUPPLEMENTAL_V2_SUCCESSFUL_RESPONSES": counters["successful_responses"],
        "SUPPLEMENTAL_V2_FAILED_RESPONSES": counters["failed_responses"],
        "SUPPLEMENTAL_V2_VALID_ROWS": valid,
        "SUPPLEMENTAL_V2_REJECTED_ROWS": len(rejected),
        "SUPPLEMENTAL_V2_OVERLAP_V1_REJECTIONS": mapped.get("OVERLAP_V1", 0),
        "SUPPLEMENTAL_V2_OVERLAP_HISTORICAL_REJECTIONS": mapped.get("OVERLAP_HISTORICAL", 0),
        "SUPPLEMENTAL_V2_OVERLAP_SUPPLEMENTAL_V1_REJECTIONS": mapped.get("OVERLAP_SUPPLEMENTAL_V1", 0),
        "SUPPLEMENTAL_V2_DUPLICATE_REJECTIONS": mapped.get("DUPLICATE_CURRENT_V2", 0),
        "SUPPLEMENTAL_V2_OTHER_REJECTIONS": other,
        "SUPPLEMENTAL_V2_REJECTION_BREAKDOWN": {k: v for k, v in sorted(reasons.items())},
        "SUPPLEMENTAL_V2_RAW_HASH": evidence_hash,
        "SUPPLEMENTAL_V2_LEDGER_HASH": ledger_hash,
        "SUPPLEMENTAL_V2_RUN_REPORT_HASH": "computed-after-write",
        "reference_hashes": {
            "supplemental-query-universe-v2.json": config["query_universe_hash"],
            "supplemental-query-universe-v2-manifest.json": manifest_hash,
            "supplemental-pilot-raw-evidence.json": hashlib.sha256((OUT / "supplemental-pilot-raw-evidence.json").read_bytes()).hexdigest(),
            "supplemental-pilot-attempts.json": hashlib.sha256((OUT / "supplemental-pilot-attempts.json").read_bytes()).hexdigest(),
            "v1-ground-truth.json": hashlib.sha256((OUT / "v1-ground-truth.json").read_bytes()).hexdigest(),
        },
        "WORK_PACKAGE_STATUS": "COMPLETE_READY_FOR_MERGE" if valid >= TARGET else "COMPLETE_EXHAUSTED_FROZEN_UNIVERSE",
        "NEXT_SINGLE_ACTION": "Build the 180-row merged corpus and A/B packets." if valid >= TARGET else "Conserve evidence and terminate; no universe expansion permitted.",
    }
    report["SUPPLEMENTAL_V2_RUN_REPORT_HASH"] = hashlib.sha256(canonical_json(report).encode("utf-8")).hexdigest()

    evidence_path = output_dir / "supplemental-v2-raw-evidence.json"
    ledger_path = output_dir / "supplemental-v2-attempts.json"
    report_path = output_dir / "supplemental-v2-run-report.json"
    evidence_path.write_text(canonical_json(evidence), encoding="utf-8")
    ledger_path.write_text(canonical_json(ledger), encoding="utf-8")
    report_path.write_text(canonical_json(report), encoding="utf-8")
    return report, evidence_path, ledger_path, report_path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", default=DEFAULT_CONFIG)
    parser.add_argument("--fixture-responses", help="offline JSON mapping query_id to response-attempt lists")
    parser.add_argument("--output-dir", default=OUT)
    parser.add_argument("--allow-network", action="store_true")
    args = parser.parse_args()
    config = load(args.config)
    universe = load(ROOT / config["universe_path"])
    start_commit = head_commit()
    gate, gate_records, attempts, counters = execute(config, universe,
                                       load(args.fixture_responses) if args.fixture_responses else None,
                                       args.allow_network)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    report, evidence_path, ledger_path, report_path = build_artifacts(
        universe, config, gate, gate_records, attempts, counters, output_dir, start_commit)
    print(json.dumps({
        "status": report["SUPPLEMENTAL_V2_EXECUTION_STATUS"],
        "valid_rows": report["SUPPLEMENTAL_V2_VALID_ROWS"],
        "network_requests": report["SUPPLEMENTAL_V2_NETWORK_REQUESTS"],
        "raw_hash": report["SUPPLEMENTAL_V2_RAW_HASH"],
        "ledger_hash": report["SUPPLEMENTAL_V2_LEDGER_HASH"],
        "run_report_hash": report["SUPPLEMENTAL_V2_RUN_REPORT_HASH"],
        "raw_evidence": str(evidence_path),
        "ledger": str(ledger_path),
        "run_report": str(report_path),
    }, indent=2))
    if report["SUPPLEMENTAL_V2_EXECUTION_STATUS"] == "EXHAUSTED_FROZEN_UNIVERSE":
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
