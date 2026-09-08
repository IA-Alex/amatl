#!/usr/bin/env python3
"""Isolated, SearXNG-only capture transport for the frozen supplemental pilot.

Normal use requires ``--allow-network`` and an explicit SearXNG endpoint.  The
fixture mode exists solely for offline contract validation and never opens a
socket.  This command neither reads ``amatl.toml`` nor imports AMATL clients.
"""
import argparse
import json
import socket
import sys
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlencode
from urllib.parse import parse_qsl, urlsplit
from urllib.request import urlopen

from supplemental_pilot_capture_contract import (RAW_SCHEMA, artifact_sha256, canonical_json,
    load_json, validate_config, validate_endpoint_binding, validate_raw_evidence)
from supplemental_pilot_contract import sha256

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication/supplemental-pilot-searxng-capture-config.json"


def now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


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


def capture(config, universe, query_ids, fixture_responses=None, allow_network=False):
    validate_config(config, universe)
    endpoint = validate_endpoint_binding(config)
    queries = [q for q in universe["queries"] if not query_ids or q["query_id"] in query_ids]
    if query_ids and len(queries) != len(set(query_ids)):
        raise ValueError("CAPTURE_QUERY_OUTSIDE_FROZEN_UNIVERSE")
    attempts, sequence = [], 0
    for query in queries:
        source_attempts = (fixture_responses or {}).get(query["query_id"], [None])
        for attempt_number in range(1, config["retry_policy"]["max_attempts"] + 1):
            sequence += 1
            requested_at = now()
            record = {"query_id": query["query_id"], "query_text": query["query"], "assigned_stratum": query["assigned_stratum"],
                      "provider": "searxng", "request_sequence": sequence, "attempt_number": attempt_number,
                      "requested_at": requested_at, "allowed_rank_range": query["permitted_positions"]}
            try:
                if fixture_responses is not None:
                    payload = source_attempts[attempt_number - 1]
                    if isinstance(payload, dict) and "raise" in payload:
                        raise TimeoutError(payload["raise"])
                    http_status, response = 200, payload or {"results": []}
                else:
                    if not allow_network:
                        raise ValueError("CAPTURE_NETWORK_NOT_AUTHORIZED")
                    http_status, response = network_response(endpoint, query["query"], 20)
                all_results = response.get("results", [])
                record.update({"completed_at": now(), "http_status": http_status, "provider_response_status": "SUCCESS",
                               "result_count": 0, "results": []})
                for rank, result in enumerate(all_results, start=1):
                    if rank in query["permitted_positions"]:
                        record["results"].append(normalise_result(result, rank))
                record["result_count"] = len(record["results"])
                attempts.append(record)
                break
            except (TimeoutError, socket.timeout) as error:
                retryable = attempt_number < config["retry_policy"]["max_attempts"]
                record.update({"completed_at": now(), "http_status": None, "provider_response_status": "ERROR", "result_count": 0, "results": [],
                               "error": {"error_class": "TIMEOUT", "retryable": retryable, "attempt_number": attempt_number,
                                         "query_id": query["query_id"], "timestamp": now()}})
                attempts.append(record)
                if not retryable: break
            except Exception as error:
                if isinstance(error, ValueError) and str(error) == "CAPTURE_NETWORK_NOT_AUTHORIZED":
                    raise
                record.update({"completed_at": now(), "http_status": None, "provider_response_status": "ERROR", "result_count": 0, "results": [],
                               "error": {"error_class": "TRANSPORT_FAILURE", "retryable": False, "attempt_number": attempt_number,
                                         "query_id": query["query_id"], "timestamp": now()}})
                attempts.append(record)
                break
    evidence = {"schema_version": RAW_SCHEMA, "experiment_id": config["experiment_id"], "query_universe_id": universe["universe_id"],
                "query_universe_hash": config["query_universe_hash"], "provider": "searxng", "attempts": attempts}
    validate_raw_evidence(evidence, universe, config)
    evidence["artifact_sha256"] = artifact_sha256(evidence)
    return evidence


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", default=DEFAULT_CONFIG)
    parser.add_argument("--query-id", action="append", default=[])
    parser.add_argument("--fixture-responses", help="offline JSON mapping query_id to response-attempt lists")
    parser.add_argument("--output", required=True)
    parser.add_argument("--allow-network", action="store_true")
    args = parser.parse_args()
    config = load_json(args.config)
    # Keep the established transport command as the routing entry point while
    # making the revision explicit.  The v2 handler has its own S1-only quota
    # and multi-run dedup contract; it never falls back to the v1 contract.
    if config.get("schema") == "amatl.relevance.supplemental-v2-searxng-capture-config.v1":
        from supplemental_v2_capture import main as v2_main
        return v2_main()
    universe = load_json(ROOT / config["universe_path"])
    evidence = capture(config, universe, args.query_id, load_json(args.fixture_responses) if args.fixture_responses else None,
                       args.allow_network)
    Path(args.output).write_text(canonical_json(evidence), encoding="utf-8")
    print(json.dumps({"status": "CAPTURE_COMPLETE", "network_requests": int(args.allow_network), "artifact_sha256": evidence["artifact_sha256"]}))


if __name__ == "__main__": main()
