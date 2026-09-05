"""Directed, network-free tests for ADR-012's pre-freeze gate."""
import json
from pathlib import Path

import pytest

from pre_execution_novelty_diversity_gate import (
    DEFAULT_THRESHOLDS, PreExecutionNoveltyDiversityGate, canonical_json,
    freeze_candidate_universe, sha256_bytes, write_manifest,
)


def rows(items, arm="treatment", pair_prefix="p"):
    return [{"query_id": f"{arm}-{i}", "query": q, "arm": arm,
             "pair_id": f"{pair_prefix}-{i}", "matched_topic": q}
            for i, q in enumerate(items, 1)]


def candidate(treatment, control):
    return {"schema": "test", "queries": rows(treatment) + rows(control, "control")}


def passing_candidate(n=10):
    return candidate([f"treatment topic {i}" for i in range(n)], [f"control topic {i}" for i in range(n)])


def evaluate(value, historical=(), **kwargs):
    return PreExecutionNoveltyDiversityGate({"min_effective_query_diversity": 0.1}).evaluate(value, historical, **kwargs)


def test_exact_and_normalized_overlap():
    d = evaluate(candidate(["Fresh alpha"], ["Fresh beta"]), [{"queries": [{"query": "Fresh alpha"}, {"query": " fresh   beta "}]}])
    assert d.metrics["EXACT_QUERY_OVERLAP_COUNT"] == 1
    assert d.metrics["NORMALIZED_QUERY_OVERLAP_COUNT"] == 2
    assert d.metrics["NEW_QUERY_COUNT"] == 0


def test_near_duplicates_and_reproducible_families():
    c = candidate(["what is solar energy", "what is solar energy today"], ["explain database indexing", "explain database indexing clearly"])
    a, b = evaluate(c), evaluate(c)
    assert a.metrics == b.metrics
    assert a.metrics["QUERY_NEAR_DUPLICATE_RATE"] > 0
    assert a.metrics["QUERY_FAMILY_COUNT"] == 4


def test_arm_balance_pair_diversity_and_capacity():
    d = evaluate(passing_candidate(), target_valid_per_arm=4, conservative_valid_per_query=.5)
    assert d.metrics["TREATMENT_QUERY_COUNT"] == d.metrics["CONTROL_QUERY_COUNT"] == 10
    assert d.metrics["ARM_NOVELTY_DELTA"] == 0
    assert d.metrics["PAIR_COUNT"] == 10
    assert d.metrics["UNIQUE_PAIR_FAMILY_COUNT"] == 10
    assert d.metrics["CAPACITY_MARGIN_TREATMENT"] == 1


def test_denominator_and_threshold_boundary():
    d = evaluate(passing_candidate(10), target_valid_per_arm=1, conservative_valid_per_query=.5)
    assert d.metrics["QUERY_DUPLICATE_RATE"] == 0
    assert d.metrics["QUERY_NOVELTY_RATE"] == 1
    assert d.decision == "PASS"


def test_manifest_hash_and_freeze_boundary(tmp_path):
    path = tmp_path / "candidate.json"
    path.write_bytes(canonical_json(passing_candidate()))
    manifest = tmp_path / "gate.json"
    d = evaluate(json.loads(path.read_text()), target_valid_per_arm=1, conservative_valid_per_query=.5)
    doc = write_manifest(manifest, d, path, [])
    assert doc["artifact_sha256"] == sha256_bytes(canonical_json({k: v for k, v in doc.items() if k != "artifact_sha256"}))
    frozen = tmp_path / "frozen.json"
    freeze_candidate_universe(json.loads(path.read_text()), frozen, target_valid_per_arm=1, conservative_valid_per_query=.5,
                              thresholds={"min_effective_query_diversity": .1})
    assert json.loads(frozen.read_text())["freeze_status"] == "FROZEN"


def test_freeze_denied_on_fail(tmp_path):
    with pytest.raises(RuntimeError, match="FREEZE_BLOCKED"):
        freeze_candidate_universe(candidate(["same", "same"], ["same", "same"]), tmp_path / "no.json",
                                  target_valid_per_arm=1, conservative_valid_per_query=.5)


def test_v5_negative_replay_and_positive_control():
    root = Path(__file__).resolve().parents[1] / "docs/evaluation/independent-relevance"
    v5 = json.loads((root / "v5/frozen-query-universe.json").read_text())
    historical = [json.loads((root / "new-corpus-v1/adjudication" / name).read_text()) for name in (
        "supplemental-query-universe-v1.json", "supplemental-query-universe-v2.json", "supplemental-v3-query-universe.json")]
    historical.append(json.loads((root / "new-corpus-v1/v4/supplemental-v4-query-universe.json").read_text()))
    d = PreExecutionNoveltyDiversityGate().evaluate(v5, historical, target_valid_per_arm=474, conservative_valid_per_query=.2887)
    assert d.decision in {"FAIL_NOVELTY", "FAIL_DIVERSITY", "FAIL_BALANCE", "FAIL_CAPACITY"}
    assert "DIVERSITY_POLICY" in d.reasons or "NOVELTY_HISTORICAL_OVERLAP" in d.reasons
    assert PreExecutionNoveltyDiversityGate().evaluate(
        passing_candidate(), target_valid_per_arm=1, conservative_valid_per_query=.5
    ).decision == "PASS"
