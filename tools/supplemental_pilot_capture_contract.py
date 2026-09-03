#!/usr/bin/env python3
"""Closed-contract primitives for the supplemental SearXNG capture transport.

This module deliberately has no dependency on AMATL's product configuration or
provider router.  It only describes the frozen experiment handoff.
"""
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit

from supplemental_pilot_contract import SEARXNG_ONLY, sha256, validate_universe

RAW_SCHEMA = "amatl.relevance.supplemental-pilot-raw-evidence.v1"
CONFIG_SCHEMA = "amatl.relevance.supplemental-pilot-searxng-capture-config.v1"
OPERATOR_ENDPOINT_SOURCE = "EXPLICIT_OPERATOR_INPUT"


def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n"


def load_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def validate_endpoint_binding(config):
    """Validate the sole transport endpoint without any discovery or rewrite."""
    binding = config.get("searxng_endpoint_binding")
    if not isinstance(binding, dict):
        raise ValueError("CAPTURE_ENDPOINT_BINDING_MISSING")
    if set(binding) != {"source", "endpoint", "endpoint_sha256"}:
        raise ValueError("CAPTURE_ENDPOINT_BINDING_INVALID")
    if binding["source"] != OPERATOR_ENDPOINT_SOURCE:
        raise ValueError("CAPTURE_ENDPOINT_SOURCE_NOT_OPERATOR_INPUT")
    endpoint = binding["endpoint"]
    if not isinstance(endpoint, str) or not endpoint or endpoint.startswith("<"):
        raise ValueError("CAPTURE_ENDPOINT_MISSING")
    parsed = urlsplit(endpoint)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ValueError("CAPTURE_ENDPOINT_INVALID_URL")
    if parsed.username is not None or parsed.password is not None:
        raise ValueError("CAPTURE_ENDPOINT_EMBEDDED_CREDENTIALS_FORBIDDEN")
    if parsed.query or parsed.fragment:
        raise ValueError("CAPTURE_ENDPOINT_INVALID_URL")
    if binding["endpoint_sha256"] != hashlib.sha256(endpoint.encode("utf-8")).hexdigest():
        raise ValueError("CAPTURE_ENDPOINT_AUTHORIZED_VALUE_MISMATCH")
    return endpoint


def validate_config(config, universe):
    if config.get("schema") != CONFIG_SCHEMA:
        raise ValueError("CAPTURE_CONFIG_SCHEMA_MISMATCH")
    if "searxng_endpoint_binding" not in config:
        raise ValueError("CAPTURE_ENDPOINT_BINDING_MISSING")
    required = {
        "experiment_id", "universe_path", "query_universe_id", "query_universe_hash", "provider",
        "provider_fallback", "dynamic_provider_selection", "dynamic_query_expansion",
        "quota_policy", "rank_depth_policy", "retry_policy", "raw_evidence", "searxng_endpoint_binding",
    }
    if required - set(config):
        raise ValueError("CAPTURE_CONFIG_MISSING_REQUIRED_FIELD")
    if config["query_universe_id"] != universe.get("universe_id"):
        raise ValueError("CAPTURE_UNIVERSE_ID_MISMATCH")
    if config["query_universe_hash"] != sha256(config["universe_path"]):
        raise ValueError("CAPTURE_UNIVERSE_HASH_MISMATCH")
    if config["provider"] != SEARXNG_ONLY:
        raise ValueError("CAPTURE_PROVIDER_NOT_SEARXNG")
    if config["provider_fallback"] != "none":
        raise ValueError("CAPTURE_PROVIDER_FALLBACK_FORBIDDEN")
    if config["dynamic_provider_selection"] is not False:
        raise ValueError("CAPTURE_DYNAMIC_PROVIDER_SELECTION_FORBIDDEN")
    if config["dynamic_query_expansion"] is not False:
        raise ValueError("CAPTURE_DYNAMIC_QUERY_EXPANSION_FORBIDDEN")
    validate_endpoint_binding(config)
    retry = config["retry_policy"]
    if not isinstance(retry.get("max_attempts"), int) or not 1 <= retry["max_attempts"] <= 3:
        raise ValueError("CAPTURE_RETRY_POLICY_INVALID")
    if config["rank_depth_policy"].get("source") != "frozen_universe_permitted_positions":
        raise ValueError("CAPTURE_RANK_POLICY_INVALID")
    return config


def validate_raw_evidence(evidence, universe, config):
    validate_universe(universe)
    validate_config(config, universe)
    if evidence.get("schema_version") != RAW_SCHEMA:
        raise ValueError("RAW_EVIDENCE_SCHEMA_MISMATCH")
    if evidence.get("experiment_id") != config["experiment_id"]:
        raise ValueError("RAW_EVIDENCE_EXPERIMENT_MISMATCH")
    if evidence.get("query_universe_id") != universe["universe_id"]:
        raise ValueError("RAW_EVIDENCE_UNIVERSE_ID_MISMATCH")
    if evidence.get("query_universe_hash") != config["query_universe_hash"]:
        raise ValueError("RAW_EVIDENCE_UNIVERSE_HASH_MISMATCH")
    if evidence.get("provider") != SEARXNG_ONLY:
        raise ValueError("RAW_EVIDENCE_PROVIDER_NOT_SEARXNG")
    queries = {query["query_id"]: query for query in universe["queries"]}
    previous_sequence = 0
    for attempt in evidence.get("attempts", []):
        required = {"query_id", "query_text", "assigned_stratum", "provider", "request_sequence",
                    "attempt_number", "requested_at", "completed_at", "allowed_rank_range",
                    "http_status", "provider_response_status", "result_count", "results"}
        if required - set(attempt):
            raise ValueError("RAW_EVIDENCE_ATTEMPT_MISSING_REQUIRED_FIELD")
        query = queries.get(attempt["query_id"])
        if query is None or attempt["query_text"] != query["query"] or attempt["assigned_stratum"] != query["assigned_stratum"]:
            raise ValueError("RAW_EVIDENCE_QUERY_NOT_FROZEN")
        if attempt["provider"] != SEARXNG_ONLY:
            raise ValueError("RAW_EVIDENCE_PROVIDER_NOT_SEARXNG")
        if attempt["allowed_rank_range"] != query["permitted_positions"]:
            raise ValueError("RAW_EVIDENCE_RANK_RANGE_MISMATCH")
        if attempt["request_sequence"] <= previous_sequence:
            raise ValueError("RAW_EVIDENCE_SEQUENCE_NOT_DETERMINISTIC")
        previous_sequence = attempt["request_sequence"]
        if attempt["result_count"] != len(attempt["results"]):
            raise ValueError("RAW_EVIDENCE_RESULT_COUNT_MISMATCH")
        for result in attempt["results"]:
            if result.get("rank") not in query["permitted_positions"]:
                raise ValueError("RAW_EVIDENCE_RESULT_OUTSIDE_DEPTH")
            if not result.get("original_url"):
                raise ValueError("RAW_EVIDENCE_RESULT_URL_MISSING")
        if "error" in attempt:
            error = attempt["error"]
            if {"error_class", "retryable", "attempt_number", "query_id", "timestamp"} - set(error):
                raise ValueError("RAW_EVIDENCE_ERROR_MISSING_REQUIRED_FIELD")
    return True


def offline_records(evidence):
    """The narrow, lossless-for-runner projection; quotas remain in its authority."""
    records = []
    for attempt in evidence["attempts"]:
        for result in attempt["results"]:
            records.append({"query_id": attempt["query_id"], "provider": attempt["provider"], "result": result})
    return records


def artifact_sha256(value):
    return hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()
