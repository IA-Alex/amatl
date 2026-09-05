#!/usr/bin/env python3
"""Offline, deterministic pre-freeze novelty/diversity gate.

This module deliberately has no provider, HTTP, URL canonicalization, or label
dependencies.  It is the single precondition a new candidate universe must
pass before a freeze writer may emit ``freeze_status=FROZEN``.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import unicodedata
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

GATE_NAME = "PRE_EXECUTION_NOVELTY_DIVERSITY_GATE"
GATE_VERSION = "1.0.0"
ARMS = ("treatment", "control")
DEFAULT_THRESHOLDS = {
    "max_normalized_query_overlap_rate": 0.20,
    "max_query_duplicate_rate": 0.20,
    "max_query_near_duplicate_rate": 0.20,
    "min_effective_query_diversity": 0.50,
    "max_arm_novelty_delta": 0.10,
    "max_arm_diversity_delta": 0.10,
    "max_pair_family_concentration": 0.20,
    "max_cross_pair_similarity": 0.80,
    "min_capacity_margin": 0.0,
    "min_arm_capacity_ratio": 0.90,
}


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def normalize_query(value: str) -> str:
    return " ".join(unicodedata.normalize("NFC", value).casefold().strip().split())


def tokens(value: str) -> frozenset[str]:
    return frozenset(re.findall(r"[\w]+", normalize_query(value), flags=re.UNICODE))


def jaccard(left: frozenset[str], right: frozenset[str]) -> float:
    union = left | right
    return len(left & right) / len(union) if union else 1.0


def _queries(universe: Any) -> list[dict[str, Any]]:
    if isinstance(universe, dict):
        if isinstance(universe.get("queries"), list):
            return universe["queries"]
        if isinstance(universe.get("assignments"), list):
            return universe["assignments"]
        if isinstance(universe.get("pairs"), list):
            out = []
            for pair in universe["pairs"]:
                for arm in ARMS:
                    if isinstance(pair.get(arm), str):
                        out.append({"pair_id": pair.get("pair_id"), "arm": arm, "query": pair[arm], "matched_topic": pair.get("matched_topic")})
            return out
    if isinstance(universe, list):
        return universe
    raise ValueError("CANDIDATE_UNIVERSE_QUERY_LIST_MISSING")


def _text(row: dict[str, Any]) -> str:
    value = row.get("query", row.get("query_text"))
    if not isinstance(value, str) or not value.strip():
        raise ValueError("QUERY_TEXT_MISSING")
    return value


def _arm(row: dict[str, Any]) -> str:
    value = str(row.get("arm", row.get("designation", ""))).casefold()
    if value not in ARMS:
        raise ValueError("QUERY_ARM_INVALID")
    return value


def _family(row: dict[str, Any], normalized: str) -> str:
    explicit = row.get("matched_topic", row.get("family"))
    if isinstance(explicit, str) and explicit.strip():
        return normalize_query(explicit)
    return " ".join(sorted(tokens(normalized)))


def _historical_queries(sources: Iterable[Any]) -> set[str]:
    result: set[str] = set()
    def walk(value: Any) -> None:
        if isinstance(value, dict):
            for key, item in value.items():
                if key in {"query", "query_text"} and isinstance(item, str):
                    result.add(item)
                walk(item)
        elif isinstance(value, list):
            for item in value:
                walk(item)
    for source in sources:
        walk(source)
    return result


def _result_urls(sources: Iterable[Any]) -> tuple[set[str], Counter[str]]:
    urls: set[str] = set()
    domains: Counter[str] = Counter()
    def walk(value: Any) -> None:
        if isinstance(value, dict):
            for key, item in value.items():
                if key in {"canonical_url", "original_url", "url"} and isinstance(item, str) and item.startswith(("http://", "https://")):
                    urls.add(item.casefold().split("#", 1)[0].rstrip("/"))
                if key == "domain" and isinstance(item, str):
                    domains[item.casefold()] += 1
                walk(item)
        elif isinstance(value, list):
            for item in value:
                walk(item)
    for source in sources:
        walk(source)
    return urls, domains


@dataclass(frozen=True)
class GateDecision:
    decision: str
    reasons: tuple[str, ...]
    metrics: dict[str, Any]
    thresholds: dict[str, Any]
    provenance: dict[str, Any]

    def as_dict(self) -> dict[str, Any]:
        return {"gate_name": GATE_NAME, "gate_version": GATE_VERSION, "decision": self.decision,
                "freeze_allowed": self.decision == "PASS", "reasons": list(self.reasons),
                "metrics": self.metrics, "thresholds": self.thresholds, "provenance": self.provenance}


class PreExecutionNoveltyDiversityGate:
    def __init__(self, thresholds: dict[str, Any] | None = None):
        self.thresholds = {**DEFAULT_THRESHOLDS, **(thresholds or {})}

    def evaluate(self, candidate_universe: Any, historical_query_universes: Iterable[Any] = (),
                 historical_result_manifests: Iterable[Any] = (), arm_assignments: Iterable[dict[str, Any]] = (),
                 target_valid_per_arm: int = 0, conservative_valid_per_query: float = 0.0,
                 provenance: dict[str, Any] | None = None) -> GateDecision:
        rows = _queries(candidate_universe)
        reasons: list[str] = []
        try:
            prepared = [{"original": _text(row), "normalized": normalize_query(_text(row)), "arm": _arm(row),
                         "family": _family(row, normalize_query(_text(row))), "pair": row.get("pair_id"),
                         "tokens": tokens(_text(row))} for row in rows]
        except ValueError as exc:
            return GateDecision("FAIL_INTEGRITY", (str(exc),), {}, self.thresholds, provenance or {"timestamp": "OFFLINE_DETERMINISTIC"})
        normalized_values = [x["normalized"] for x in prepared]
        historical_raw = _historical_queries(historical_query_universes)
        historical = {normalize_query(value) for value in historical_raw}
        exact_count = len(set(x["original"] for x in prepared) & historical_raw)
        normalized_count = len(set(normalized_values) & historical)
        duplicate_count = len(normalized_values) - len(set(normalized_values))
        near_pairs = sum(jaccard(a["tokens"], b["tokens"]) >= 0.80 for i, a in enumerate(prepared) for b in prepared[i + 1:])
        near_count = near_pairs * 2
        family_counts = Counter(x["family"] for x in prepared)
        unique_families = len(family_counts)
        effective = unique_families / len(prepared) if prepared else 0.0
        pair_rows: dict[str, dict[str, frozenset[str]]] = {}
        for x in prepared:
            if x["pair"]:
                pair_rows.setdefault(str(x["pair"]), {})[x["arm"]] = x["tokens"]
        pair_families = Counter(" ".join(sorted(set().union(*pair.values())))
                                for pair in pair_rows.values() if len(pair) == 2)
        pair_count = len(pair_rows)
        pair_family_count = len(pair_families)
        pair_concentration = max(pair_families.values(), default=0) / pair_count if pair_count else 0.0
        pair_values = list(pair_rows.values())
        cross_sim = max((jaccard(a, b) for i, left in enumerate(pair_values) for right in pair_values[i + 1:] for a in left.values() for b in right.values()), default=0.0)
        arms = {arm: [x for x in prepared if x["arm"] == arm] for arm in ARMS}
        arm_metrics: dict[str, Any] = {}
        for arm, values in arms.items():
            arm_families = len(set(x["family"] for x in values))
            arm_metrics[arm] = {"QUERY_COUNT": len(values), "NOVELTY_RATE": (len(set(x["normalized"]) - historical) / len(values) if values else 0.0),
                               "EFFECTIVE_DIVERSITY": arm_families / len(values) if values else 0.0}
        novelty_rate = (len(set(normalized_values) - historical) / len(prepared)) if prepared else 0.0
        result_urls, domains = _result_urls(historical_result_manifests)
        result_status = "ESTIMATED" if result_urls or domains else "INSUFFICIENT_EVIDENCE"
        historical_risk = (len(result_urls) / max(1, len(result_urls) + len(prepared))) if result_urls else None
        metrics = {"TOTAL_CANDIDATE_QUERIES": len(prepared), "EXACT_QUERY_OVERLAP_COUNT": exact_count,
                   "EXACT_QUERY_OVERLAP_RATE": exact_count / len(prepared) if prepared else 0.0,
                   "NORMALIZED_QUERY_OVERLAP_COUNT": normalized_count, "NORMALIZED_QUERY_OVERLAP_RATE": normalized_count / len(prepared) if prepared else 0.0,
                   "NEW_QUERY_COUNT": len(set(normalized_values) - historical), "QUERY_NOVELTY_RATE": novelty_rate,
                   "UNIQUE_QUERY_COUNT": len(set(normalized_values)), "QUERY_DUPLICATE_RATE": duplicate_count / len(prepared) if prepared else 0.0,
                   "QUERY_NEAR_DUPLICATE_RATE": min(1.0, near_count / len(prepared)) if prepared else 0.0,
                   "QUERY_FAMILY_COUNT": unique_families, "EFFECTIVE_QUERY_DIVERSITY": effective,
                   "PAIR_COUNT": pair_count, "UNIQUE_PAIR_FAMILY_COUNT": pair_family_count, "PAIR_FAMILY_CONCENTRATION": pair_concentration,
                   "CROSS_PAIR_SIMILARITY": cross_sim, "TREATMENT_QUERY_COUNT": len(arms["treatment"]), "CONTROL_QUERY_COUNT": len(arms["control"]),
                   "TREATMENT_NOVELTY_RATE": arm_metrics["treatment"]["NOVELTY_RATE"], "CONTROL_NOVELTY_RATE": arm_metrics["control"]["NOVELTY_RATE"],
                   "TREATMENT_EFFECTIVE_DIVERSITY": arm_metrics["treatment"]["EFFECTIVE_DIVERSITY"], "CONTROL_EFFECTIVE_DIVERSITY": arm_metrics["control"]["EFFECTIVE_DIVERSITY"],
                   "ARM_NOVELTY_DELTA": abs(arm_metrics["treatment"]["NOVELTY_RATE"] - arm_metrics["control"]["NOVELTY_RATE"]),
                   "ARM_DIVERSITY_DELTA": abs(arm_metrics["treatment"]["EFFECTIVE_DIVERSITY"] - arm_metrics["control"]["EFFECTIVE_DIVERSITY"]),
                   "EXPECTED_VALID_TREATMENT_CONSERVATIVE": len(arms["treatment"]) * conservative_valid_per_query,
                   "EXPECTED_VALID_CONTROL_CONSERVATIVE": len(arms["control"]) * conservative_valid_per_query,
                   "CAPACITY_MARGIN_TREATMENT": len(arms["treatment"]) * conservative_valid_per_query - target_valid_per_arm,
                   "CAPACITY_MARGIN_CONTROL": len(arms["control"]) * conservative_valid_per_query - target_valid_per_arm,
                   "RESULT_NOVELTY_ESTIMATION": result_status, "HISTORICAL_RESULT_OVERLAP_RISK": historical_risk,
                   "HISTORICAL_DOMAIN_CONCENTRATION": (max(domains.values()) / sum(domains.values()) if domains else None),
                   "NOVELTY_RISK_SCORE": (1.0 - novelty_rate) if result_status == "INSUFFICIENT_EVIDENCE" else (1.0 - novelty_rate + (historical_risk or 0.0)) / 2}
        if not prepared or len(arms["treatment"]) != len(arms["control"]): reasons.append("QUERY_INTEGRITY_ARM_BALANCE")
        if metrics["NORMALIZED_QUERY_OVERLAP_RATE"] > self.thresholds["max_normalized_query_overlap_rate"]: reasons.append("NOVELTY_HISTORICAL_OVERLAP")
        if metrics["QUERY_DUPLICATE_RATE"] > self.thresholds["max_query_duplicate_rate"] or metrics["QUERY_NEAR_DUPLICATE_RATE"] > self.thresholds["max_query_near_duplicate_rate"] or metrics["EFFECTIVE_QUERY_DIVERSITY"] < self.thresholds["min_effective_query_diversity"] or metrics["PAIR_FAMILY_CONCENTRATION"] > self.thresholds["max_pair_family_concentration"] or metrics["CROSS_PAIR_SIMILARITY"] > self.thresholds["max_cross_pair_similarity"]: reasons.append("DIVERSITY_POLICY")
        if metrics["ARM_NOVELTY_DELTA"] > self.thresholds["max_arm_novelty_delta"] or metrics["ARM_DIVERSITY_DELTA"] > self.thresholds["max_arm_diversity_delta"]: reasons.append("ARM_BALANCE_POLICY")
        if metrics["CAPACITY_MARGIN_TREATMENT"] <= self.thresholds["min_capacity_margin"] or metrics["CAPACITY_MARGIN_CONTROL"] <= self.thresholds["min_capacity_margin"]: reasons.append("CAPACITY_MARGIN_NON_POSITIVE")
        decision = "PASS" if not reasons else ("FAIL_NOVELTY" if any("NOVELTY" in x for x in reasons) else "FAIL_DIVERSITY" if any("DIVERSITY" in x for x in reasons) else "FAIL_BALANCE" if any("BALANCE" in x for x in reasons) else "FAIL_CAPACITY" if any("CAPACITY" in x for x in reasons) else "FAIL_INTEGRITY")
        return GateDecision(decision, tuple(reasons), metrics, self.thresholds, provenance or {"timestamp": "OFFLINE_DETERMINISTIC", "network_requests": 0})


def write_manifest(path: Path, decision: GateDecision, candidate_path: Path, historical_paths: list[Path]) -> dict[str, Any]:
    doc = decision.as_dict()
    doc["candidate_universe_sha256"] = sha256_file(candidate_path)
    doc["historical_sources"] = [{"path": str(p), "sha256": sha256_file(p)} for p in historical_paths]
    doc["historical_source_sha256"] = sha256_bytes(canonical_json(doc["historical_sources"]))
    doc["artifact_sha256"] = sha256_bytes(canonical_json(doc))
    path.write_bytes(canonical_json(doc))
    return doc


def freeze_candidate_universe(candidate: Any, output_path: Path, **kwargs: Any) -> GateDecision:
    """Real freeze boundary: FROZEN is emitted only after a PASS."""
    decision = PreExecutionNoveltyDiversityGate(kwargs.pop("thresholds", None)).evaluate(candidate, **kwargs)
    if decision.decision != "PASS":
        raise RuntimeError(f"FREEZE_BLOCKED_BY_{GATE_NAME}:{decision.decision}")
    output_path.write_bytes(canonical_json({**candidate, "freeze_status": "FROZEN", "gate": decision.as_dict()}))
    return decision


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--historical", type=Path, action="append", default=[])
    parser.add_argument("--results", type=Path, action="append", default=[])
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--target-valid-per-arm", type=int, default=0)
    parser.add_argument("--conservative-valid-per-query", type=float, default=0.0)
    args = parser.parse_args()
    candidate = json.loads(args.candidate.read_text(encoding="utf-8"))
    historical = [json.loads(p.read_text(encoding="utf-8")) for p in args.historical]
    results = [json.loads(p.read_text(encoding="utf-8")) for p in args.results]
    decision = PreExecutionNoveltyDiversityGate().evaluate(candidate, historical, results, target_valid_per_arm=args.target_valid_per_arm, conservative_valid_per_query=args.conservative_valid_per_query)
    write_manifest(args.manifest, decision, args.candidate, args.historical)
    print(json.dumps(decision.as_dict(), ensure_ascii=False, sort_keys=True))
    raise SystemExit(0 if decision.decision == "PASS" else 1)


if __name__ == "__main__":
    main()
