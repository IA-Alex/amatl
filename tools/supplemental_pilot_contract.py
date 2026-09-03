#!/usr/bin/env python3
"""Offline contract primitives for the frozen 180-row supplemental pilot.

This module deliberately contains no HTTP client and no provider fallback.  A
future, separately authorised transport must supply SearXNG results to this
contract; only rows accepted here can enter the pilot corpus.
"""
import hashlib
import json
from dataclasses import dataclass, field
from pathlib import Path
from types import MappingProxyType
from urllib.parse import urlsplit, urlunsplit

STRATA = ("S1_PROCEDURAL_SHALLOW", "S2_INFORMATIONAL_SHALLOW", "S3_MIXED_DEPTH_CONTROL")
TARGET_PER_STRATUM = 60
TOTAL_TARGET = 180
SEARXNG_ONLY = "searxng"


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical_url(value):
    """Apply the frozen URL key used for current, V1, and historical overlap."""
    parts = urlsplit(value.strip())
    if parts.scheme not in ("http", "https") or not parts.netloc:
        raise ValueError("INVALID_RESULT")
    path = parts.path.rstrip("/") or "/"
    return urlunsplit((parts.scheme.lower(), parts.netloc.lower(), path, parts.query, ""))


def load_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def validate_universe(universe):
    if universe.get("schema") != "amatl.relevance.supplemental-query-universe.v1":
        raise ValueError("invalid universe schema")
    if universe.get("provider_requirement") != "SEARXNG_ONLY":
        raise ValueError("pilot provider is not SearXNG-only")
    if universe.get("dynamic_expansion") != "DISABLED":
        raise ValueError("dynamic query expansion must be disabled")
    queries = universe.get("queries", [])
    if not queries:
        raise ValueError("empty universe")
    ids = [query.get("query_id") for query in queries]
    if len(ids) != len(set(ids)) or any(not query_id for query_id in ids):
        raise ValueError("query IDs must be unique")
    if [q["order"] for q in queries] != list(range(1, len(queries) + 1)):
        raise ValueError("universe order is not contiguous and deterministic")
    counts = {stratum: 0 for stratum in STRATA}
    capacity = {stratum: 0 for stratum in STRATA}
    s3_kinds = []
    for query in queries:
        stratum = query.get("assigned_stratum")
        if stratum not in counts:
            raise ValueError("query must belong to exactly one known stratum")
        positions = query.get("permitted_positions")
        if positions != sorted(set(positions)) or not positions:
            raise ValueError("permitted positions must be sorted and unique")
        expected = [1, 2, 3] if stratum != STRATA[2] else [4, 5, 6, 7, 8]
        if positions != expected:
            raise ValueError("query positions violate frozen stratum contract")
        counts[stratum] += 1
        capacity[stratum] += len(positions)
        if stratum == STRATA[2]:
            s3_kinds.append(query.get("semantic_form"))
    if any(capacity[stratum] < TARGET_PER_STRATUM for stratum in STRATA):
        raise ValueError("insufficient structural capacity")
    if s3_kinds.count("procedural") != s3_kinds.count("informational"):
        raise ValueError("S3 must retain its fixed procedural/informational balance")
    return counts, capacity


@dataclass
class PilotAccumulator:
    """Quota and contamination gate; rejected attempts remain in ``attempts``."""
    universe: dict
    v1_urls: set = field(default_factory=set)
    historical_urls: set = field(default_factory=set)
    _queries: object = field(init=False)
    valid_counts: dict = field(init=False)
    current_urls: set = field(default_factory=set)
    attempts: list = field(default_factory=list)

    def __post_init__(self):
        validate_universe(self.universe)
        self._queries = MappingProxyType({q["query_id"]: q for q in self.universe["queries"]})
        self.valid_counts = {stratum: 0 for stratum in STRATA}
        self.v1_urls = {canonical_url(url) for url in self.v1_urls}
        self.historical_urls = {canonical_url(url) for url in self.historical_urls}

    def accept_or_reject(self, query_id, provider, result):
        reason = None
        query = self._queries.get(query_id)
        if query is None:
            reason = "OUTSIDE_FROZEN_UNIVERSE"
        elif provider != SEARXNG_ONLY:
            reason = "OUTSIDE_PROVIDER_CONTRACT"
        elif result.get("rank") not in query["permitted_positions"]:
            reason = "OUTSIDE_ALLOWED_DEPTH"
        else:
            try:
                url = canonical_url(result.get("canonical_url") or result["original_url"])
                if not result.get("title") or not result.get("snippet"):
                    raise ValueError("INVALID_RESULT")
            except (KeyError, TypeError, ValueError):
                reason = "INVALID_RESULT"
            else:
                stratum = query["assigned_stratum"]
                if url in self.current_urls:
                    reason = "DUPLICATE_CURRENT_PILOT"
                elif url in self.v1_urls:
                    reason = "OVERLAP_V1"
                elif url in self.historical_urls:
                    reason = "OVERLAP_HISTORICAL"
                elif self.valid_counts[stratum] >= TARGET_PER_STRATUM or sum(self.valid_counts.values()) >= TOTAL_TARGET:
                    reason = "QUOTA_FILLED"
        attempt = {"query_id": query_id, "provider": provider, "rank": result.get("rank"), "reason_code": reason}
        if reason is None:
            self.current_urls.add(url)
            self.valid_counts[query["assigned_stratum"]] += 1
            attempt.update({"reason_code": "ACCEPTED", "canonical_url": url, "stratum": query["assigned_stratum"]})
        self.attempts.append(attempt)
        return attempt

    def exhausted_status(self):
        if all(count == TARGET_PER_STRATUM for count in self.valid_counts.values()):
            return "COMPLETE"
        return "EXHAUSTED_FROZEN_UNIVERSE"
