#!/usr/bin/env python3
"""Freeze the V6 confirmatory design from the accepted candidate-v2 universe.

This tool is intentionally offline-only. It copies the accepted universe byte
for byte, derives deterministic pair order from a new seed, and emits only
design/provenance artifacts. It never contacts a provider or creates labels.
"""
from __future__ import annotations

import hashlib
import json
import random
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "docs/evaluation/independent-relevance"
SOURCE = BASE / "candidates/candidate-v2"
OUT = BASE / "v6"
EXPERIMENT_ID = "independent-relevance-confirmatory-v6"
SEED = "independent-relevance-confirmatory-v6-randomization-v1"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def dump(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")


def load(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    source_universe = SOURCE / "candidate-universe.json"
    source_manifest = SOURCE / "candidate-universe-manifest.json"
    source_gate = SOURCE / "gate-decision.json"
    source_eval = SOURCE / "novelty-diversity-evaluation.json"

    candidate = load(source_universe)
    source_sha = digest(source_universe)
    assert source_sha == "8bcc8a4a0ba518930db02da1c5161f4bd48dd243695ea52718494cc55edda64e"
    assert candidate["pair_count"] == 2300
    assert candidate["treatment_query_count"] == 2300
    assert candidate["control_query_count"] == 2300
    assert len(candidate["queries"]) == len(candidate["assignments"]) == 4600
    assert candidate["network_requests"] == 0

    # This is a byte-for-byte frozen copy; no query text or provenance is edited.
    frozen_universe = OUT / "frozen-query-universe.json"
    shutil.copyfile(source_universe, frozen_universe)

    queries = candidate["queries"]
    assignments = candidate["assignments"]
    pair_ids = sorted({q["pair_id"] for q in queries})
    assert len(pair_ids) == 2300
    rng = random.Random(SEED)
    shuffled = pair_ids[:]
    rng.shuffle(shuffled)
    order = {pair_id: i + 1 for i, pair_id in enumerate(shuffled)}
    randomized = sorted(assignments, key=lambda a: (order[a["pair_id"]], {"treatment": 0, "control": 1}[a["arm"]]))
    pair_digest = hashlib.sha256(
        "\n".join(f'{a["pair_id"]}\t{a["arm"]}\t{a["query_id"]}' for a in randomized).encode()
    ).hexdigest()
    random_manifest = {
        "schema": "amatl.relevance.v6-randomization-manifest.v1",
        "experiment_id": EXPERIMENT_ID,
        "method": "Seeded deterministic Fisher-Yates shuffle of pre-matched pair IDs; arm assignment within each pair preserved",
        "seed": SEED,
        "allocation_ratio": "1:1",
        "pair_count": 2300,
        "input_order": "pair_id ascending candidate-p-0001..candidate-p-2300",
        "assignment_order": "randomized pair order; treatment/control adjacent by pair",
        "assignment_sha256": pair_digest,
        "network_requests": 0,
        "reproducibility": "same frozen universe and seed produce byte-identical order and assignment hash",
    }
    dump(OUT / "randomization-manifest.json", random_manifest)
    dump(OUT / "pair-assignments.json", {
        "schema": "amatl.relevance.v6-pair-assignments.v1",
        "experiment_id": EXPERIMENT_ID,
        "assignment_status": "FROZEN",
        "method": random_manifest["method"],
        "randomization_seed": SEED,
        "assignments": randomized,
        "network_requests": 0,
    })

    candidate_manifest = load(source_manifest)
    gate = load(source_gate)
    evaluation = load(source_eval)
    assert candidate_manifest["candidate_universe_sha256"] == source_sha
    assert gate["candidate_universe_sha256"] == source_sha
    assert gate["decision"] == "PASS"
    assert evaluation["candidate_universe_sha256"] == source_sha
    source_refs = {
        "candidate_universe": {"path": str(source_universe.relative_to(ROOT)), "sha256": source_sha},
        "candidate_universe_manifest": {"path": str(source_manifest.relative_to(ROOT)), "sha256": digest(source_manifest)},
        "gate_decision": {"path": str(source_gate.relative_to(ROOT)), "sha256": digest(source_gate)},
        "novelty_diversity_evaluation": {"path": str(source_eval.relative_to(ROOT)), "sha256": digest(source_eval)},
    }
    dump(OUT / "query-universe-manifest.json", {
        "schema": "amatl.relevance.v6-query-universe-manifest.v1",
        "experiment_id": EXPERIMENT_ID,
        "freeze_status": "FROZEN",
        "source_candidate": "candidate-v2",
        "source_candidate_sha256": source_sha,
        "source_generator_version": candidate["generator_version"],
        "source_gate_decision": gate["decision"],
        "frozen_query_count": 4600,
        "frozen_query_count_treatment": 2300,
        "frozen_query_count_control": 2300,
        "pair_count": 2300,
        "query_ids_and_pair_structure": "copied byte-for-byte from frozen-query-universe.json",
        "pair_assignment_integrity": "PASS",
        "network_requests": 0,
        "source_artifacts": source_refs,
    })

    # The accepted gate's historical inputs are carried as immutable references.
    historical = []
    for item in candidate_manifest["historical_sources"]:
        historical.append({"lineage": "V1-V5 query universe", "path": item["path"], "sha256": item["sha256"]})
    for path, sha in evaluation["historical_result_manifest_hashes"].items():
        historical.append({"lineage": "historical result corpus", "path": path, "sha256": sha})
    dump(OUT / "historical-exclusion-manifest.json", {
        "schema": "amatl.relevance.v6-historical-exclusion-manifest.v1",
        "experiment_id": EXPERIMENT_ID,
        "freeze_status": "FROZEN",
        "historical_exclusion_set": ["V1", "V2", "V3", "V4", "V5", "historical evaluation corpus"],
        "policy": "Before acceptance, canonicalize with the frozen AMATL canonicalizer; reject any canonical URL in this set, then reject duplicate-current URLs. Historical overlap is rejected before labeling.",
        "canonicalization_policy": "lowercase scheme/host; remove tracking parameters and fragments; preserve non-tracking query; empty path becomes /; malformed URLs are invalid",
        "duplicate_current_policy": "reject a canonical URL already accepted in the current experiment, across arms",
        "invalid_result_policy": "reject missing/invalid URL, title, snippet, rank, or provider payload",
        "historical_overlap_policy": "reject canonical URL present in any listed V1-V5 or historical corpus",
        "historical_sources": historical,
        "historical_source_count": len(historical),
        "integrity_basis": "full-file SHA-256 values recorded from accepted candidate-v2 provenance; no historical artifact modified",
        "network_requests": 0,
    })

    canon = ROOT / "crates/amatl-core/src/canonical.rs"
    dedupe = ROOT / "crates/amatl-core/src/dedupe.rs"
    design = {
        "schema": "amatl.relevance.v6-confirmatory-design.v1",
        "design_status": "FROZEN",
        "experiment_id": EXPERIMENT_ID,
        "experiment_version": "6.0.0",
        "experiment_type": "CONFIRMATORY",
        "source_candidate": "candidate-v2",
        "source_candidate_sha256": source_sha,
        "source_generator_version": "offline-catalog-v1.1.0",
        "source_gate_decision": "PASS",
        "source_gate_version": "1.1.0",
        "source_commit": "c0d5e49ae9eb89a9a26a620e8c0d97fbe3b50a8f",
        "candidate_identity_match": "PASS",
        "primary_hypothesis": "P(Relevant | Treatment) > P(Relevant | Control)",
        "primary_endpoint": "STRICT_RELEVANCE_YIELD",
        "alpha": 0.05,
        "power_target": 0.90,
        "target_valid_per_arm": 474,
        "target_valid_total": 948,
        "capacity": {"expected_valid_per_arm": 476.5193, "margin_per_arm": 2.5193, "ratio_per_arm": 1.005315, "headroom": "THIN", "justification": "Only 0.5315% above target; small deviations in observed valid yield can exhaust the frozen universe before target."},
        "capacity_failure_policy": "If either arm exhausts its frozen queries before 474 valid results, FINAL_DECISION=INCONCLUSIVE; no rescue sampling, added queries, reallocation, depth increase, or provider switch.",
        "universe": {"frozen_query_count": 4600, "treatment": 2300, "control": 2300, "pair_count": 2300, "pair_structure": "matched topic; one query per arm"},
        "randomization": {"method": random_manifest["method"], "seed": SEED, "allocation_ratio": "1:1", "manifest": "randomization-manifest.json"},
        "provider_set": ["SearXNG"],
        "result_depth": 3,
        "depth_justification": "Preserves the historical V4/V5 shallow result contract; depth is not increased to compensate for thin capacity.",
        "canonicalization_implementation": {"path": str(canon.relative_to(ROOT)), "sha256": digest(canon)},
        "deduplication_implementation": {"path": str(dedupe.relative_to(ROOT)), "sha256": digest(dedupe)},
        "historical_exclusion_manifest": "historical-exclusion-manifest.json",
        "valid_result_definition": "A provider result at rank 1..3 with valid URL, title, and snippet, accepted after canonicalization and rejection of current duplicates and historical overlap. Query, raw result, result slot, canonical result, valid result, and relevant result are distinct; the denominator is accepted valid results only.",
        "execution_policy": "execution-policy.json",
        "prelabel_freeze_policy": "Capture raw evidence and accepted rows first; freeze query/result provenance, canonical URL, arm, pair, provider, and exclusion evidence in PRELABEL_DATASET and PRELABEL_MANIFEST with SHA-256. LABELING_STARTED=NO until complete.",
        "labeling_policy": "Two independent blinded labelers use Relevant, PossiblyRelevant, NotRelevant, Unknown; labels are not created in this work package.",
        "labeler_a": "INDEPENDENT_RELEVANCE_LABELER_A_V6",
        "labeler_b": "INDEPENDENT_RELEVANCE_LABELER_B_V6",
        "adjudication_policy": "Independent adjudication of disagreements; no labels or arm metrics exposed during initial labeling.",
        "primary_endpoint_formula": {"treatment": "Relevant_Treatment / Valid_Treatment", "control": "Relevant_Control / Valid_Control", "effect_estimate": "Treatment yield - Control yield; report relative lift and odds ratio as secondary effect summaries"},
        "statistical_test": "one-sided Fisher exact test on Relevant versus non-Relevant by arm, alpha=0.05",
        "confidence_interval_method": "two-sided 95% Wilson score interval per arm and Newcombe-Wilson interval for the Treatment-minus-Control difference",
        "multiplicity_adjustment": "NOT_REQUIRED: one primary endpoint; all other metrics SECONDARY/EXPLORATORY and cannot change primary decision.",
        "decision_rule": "VALIDATE_ACQUISITION_STRATEGY iff both arm targets and integrity gates pass, Treatment strict yield > Control strict yield, absolute difference >= 0.05, and one-sided Fisher p <= 0.05. REJECT_ACQUISITION_STRATEGY iff target/integrity pass and any primary condition fails. INCONCLUSIVE iff target is not reached, universe/provider/execution integrity is compromised, or the primary test cannot be validly computed.",
        "network_requests": 0,
        "labeling_started": "NO",
        "no_post_freeze_tuning": True,
    }
    dump(OUT / "design.json", design)

    dump(OUT / "execution-policy.json", {
        "schema": "amatl.relevance.v6-execution-policy.v1", "experiment_id": EXPERIMENT_ID, "provider_set": ["SearXNG"], "result_depth": 3,
        "capture_order": "randomization-manifest pair order; treatment then control within pair", "retry_max_per_query": 1,
        "retry_only_for": ["timeout", "transport failure", "HTTP 429", "HTTP 5xx"], "no_retry_for": ["valid empty response", "malformed response", "invalid result", "duplicate", "historical overlap"],
        "failure_handling": "Record every attempt and reason; after bounded retry mark query unavailable. Never replace a failed query with a new query, reallocate arms, switch provider, or increase depth.",
        "request_accounting": "One request per frozen query plus at most one same-query retry; count and hash each raw response or failure record.",
        "provenance": "Every request/attempt must carry experiment_id, query_id, pair_id, arm, provider, capture ordinal, and frozen universe hash.",
        "network_execution_policy": "Future execution only; no network requests in design work package. SearXNG-only; Marginalia excluded; no preflight here.", "success_stop": "TREATMENT_VALID_RESULTS >= 474 AND CONTROL_VALID_RESULTS >= 474",
        "universe_exhaustion_stop": "Any arm exhausted before target => INCONCLUSIVE", "no_rescue_sampling": True, "network_requests": 0,
    })
    dump(OUT / "decision-policy.json", {"schema": "amatl.relevance.v6-decision-policy.v1", "experiment_id": EXPERIMENT_ID, "primary_endpoint": "STRICT_RELEVANCE_YIELD", "alpha": 0.05, "success": "VALIDATE_ACQUISITION_STRATEGY iff frozen target/integrity, strict Treatment yield > Control yield, absolute delta >= 0.05 and one-sided Fisher p <= 0.05", "failure": "REJECT_ACQUISITION_STRATEGY iff target and integrity pass but any success condition fails", "inconclusive": "INCONCLUSIVE on target shortfall, universe exhaustion, provider/execution integrity failure, or invalid primary test", "secondary_non_rescue": True, "multiplicity_adjustment": "NOT_REQUIRED", "network_requests": 0})

    artifacts = ["design.json", "query-universe-manifest.json", "frozen-query-universe.json", "pair-assignments.json", "randomization-manifest.json", "historical-exclusion-manifest.json", "execution-policy.json", "decision-policy.json"]
    dump(OUT / "provenance-manifest.json", {"schema": "amatl.relevance.v6-provenance-manifest.v1", "experiment_id": EXPERIMENT_ID, "freeze_status": "FROZEN", "source_candidate": "candidate-v2", "source_candidate_sha256": source_sha, "source_generator_version": "offline-catalog-v1.1.0", "source_gate_decision": "PASS", "source_commit": "c0d5e49ae9eb89a9a26a620e8c0d97fbe3b50a8f", "network_requests": 0, "hash_policy": "full-file SHA-256 for every listed artifact; this root manifest is excluded from its own artifact set", "artifacts": {name: digest(OUT / name) for name in artifacts}})

    # Idempotence check: all source values and all frozen invariants still hold.
    assert digest(frozen_universe) == source_sha
    assert load(OUT / "query-universe-manifest.json")["source_candidate_sha256"] == source_sha
    assert load(OUT / "design.json")["network_requests"] == 0
    print(json.dumps({"status": "FROZEN", "experiment_id": EXPERIMENT_ID, "network_requests": 0, "source_candidate_sha256": source_sha}, sort_keys=True))


if __name__ == "__main__":
    main()
