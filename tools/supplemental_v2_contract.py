#!/usr/bin/env python3
"""The immutable multi-run acceptance gate for S1 supplemental-v2."""
from dataclasses import dataclass, field
from pathlib import Path
import json
from supplemental_pilot_contract import canonical_url

TARGET = 8
STRATUM = "S1_PROCEDURAL_SHALLOW"
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"

def load(name): return json.loads((OUT / name).read_text(encoding="utf-8"))

def multirun_url_sets():
    ground = {canonical_url(r["canonical_url"]) for r in load("v1-ground-truth.json")["rows"]}
    historical = {canonical_url(u) for u in load("supplemental-pilot-historical-canonical-urls.json")}
    pilot = {a["canonical_url"] for a in load("supplemental-pilot-attempts.json")["attempts"] if a["reason_code"] == "ACCEPTED"}
    return ground, historical, {canonical_url(u) for u in pilot}

@dataclass
class V2Accumulator:
    universe: dict
    v1_urls: set
    historical_urls: set
    supplemental_v1_urls: set
    current_urls: set = field(default_factory=set)
    accepted: list = field(default_factory=list)

    def __post_init__(self):
        if self.universe.get("universe_id") != "independent-relevance-supplemental-v2": raise ValueError("V2_UNIVERSE_ID")
        self.queries = {q["query_id"]: q for q in self.universe["queries"]}
        if len(self.queries) != len(self.universe["queries"]): raise ValueError("V2_QUERY_IDS")

    def accept_or_reject(self, query_id, provider, result):
        query, reason = self.queries.get(query_id), None
        if query is None: reason = "OUTSIDE_FROZEN_UNIVERSE"
        elif provider != "searxng": reason = "OUTSIDE_PROVIDER_CONTRACT"
        elif result.get("rank") not in query["permitted_positions"]: reason = "OUTSIDE_ALLOWED_DEPTH"
        else:
            try:
                url = canonical_url(result["original_url"])
                if not result.get("title") or not result.get("snippet"): raise ValueError
            except (KeyError, TypeError, ValueError): reason = "INVALID_RESULT"
            else:
                if url in self.current_urls: reason = "DUPLICATE_CURRENT_V2"
                elif url in self.v1_urls: reason = "OVERLAP_V1_GROUND_TRUTH"
                elif url in self.historical_urls: reason = "OVERLAP_HISTORICAL"
                elif url in self.supplemental_v1_urls: reason = "OVERLAP_SUPPLEMENTAL_V1"
                elif len(self.accepted) >= TARGET: reason = "S1_NEW_VALID_TARGET_FILLED"
        record = {"query_id": query_id, "reason_code": reason or "ACCEPTED"}
        if reason is None:
            self.current_urls.add(url)
            record.update({"canonical_url": url, "stratum": STRATUM, "SOURCE_RUN": "supplemental-v2"})
            self.accepted.append(record)
        return record

def build_accumulator(universe):
    ground, historical, pilot = multirun_url_sets()
    return V2Accumulator(universe, ground, historical, pilot)
