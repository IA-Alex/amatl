#!/usr/bin/env python3
"""Build the single ADR-012 candidate and record one offline evaluation."""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

from generate_offline_candidate_universe import GENERATOR_VERSION, build_candidate, canonical  # noqa: E402
from pre_execution_novelty_diversity_gate import (  # noqa: E402
    GATE_VERSION,
    NEW_CANDIDATE_MODE,
    PreExecutionNoveltyDiversityGate,
    canonical_json,
    sha256_bytes,
    write_manifest,
)

TARGET_VALID_PER_ARM = 474
TREATMENT_EVIDENCE_YIELD = 402 / 1086
CONTROL_EVIDENCE_YIELD = 225 / 1086
CONSERVATIVE_GATE_YIELD = min(TREATMENT_EVIDENCE_YIELD, CONTROL_EVIDENCE_YIELD)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_bytes(canonical(value))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    out = args.output_dir
    out.mkdir(parents=True, exist_ok=True)

    candidate = build_candidate()
    candidate_path = out / "candidate-universe.json"
    write_json(candidate_path, candidate)

    history_root = ROOT / "docs/evaluation/independent-relevance"
    historical_paths = [
        history_root / "new-corpus-v1/query-set.json",
        history_root / "new-corpus-v1/adjudication/supplemental-query-universe-v1.json",
        history_root / "new-corpus-v1/adjudication/supplemental-query-universe-v2.json",
        history_root / "new-corpus-v1/adjudication/supplemental-v3-query-universe.json",
        history_root / "new-corpus-v1/v4/supplemental-v4-query-universe.json",
        history_root / "v5/frozen-query-universe.json",
    ]
    result_paths = [
        history_root / "new-corpus-v1/v4/supplemental-v4-raw-evidence.json",
        history_root / "v5/execution/raw-capture.json",
    ]
    historical = [json.loads(path.read_text()) for path in historical_paths]
    results = [json.loads(path.read_text()) for path in result_paths]
    assignments = candidate["assignments"]
    decision = PreExecutionNoveltyDiversityGate().evaluate(
        candidate,
        historical_query_universes=historical,
        historical_result_manifests=results,
        arm_assignments=assignments,
        target_valid_per_arm=TARGET_VALID_PER_ARM,
        conservative_valid_per_query=CONSERVATIVE_GATE_YIELD,
        provenance={
            "timestamp": "OFFLINE_DETERMINISTIC",
            "source": "ADR-012",
            "generator_version": GENERATOR_VERSION,
            "mode": NEW_CANDIDATE_MODE,
            "network_requests": 0,
        },
        mode=NEW_CANDIDATE_MODE,
    )

    manifest_path = out / "candidate-universe-manifest.json"
    manifest = write_manifest(manifest_path, decision, candidate_path, historical_paths)
    decision_doc = {
        "schema": "amatl.relevance.offline-gate-decision.v1",
        "candidate_universe_sha256": digest(candidate_path),
        "candidate_universe_manifest_sha256": digest(manifest_path),
        "gate_version": GATE_VERSION,
        "generator_version": GENERATOR_VERSION,
        "network_requests": 0,
        "decision": decision.decision,
        "subgate_decisions": decision.subgate_decisions,
        "reasons": list(decision.reasons),
    }
    write_json(out / "gate-decision.json", decision_doc)

    t_count = decision.metrics["TREATMENT_QUERY_COUNT"]
    c_count = decision.metrics["CONTROL_QUERY_COUNT"]
    evaluation = {
        "schema": "amatl.relevance.offline-novelty-diversity-evaluation.v1",
        "evaluation_mode": NEW_CANDIDATE_MODE,
        "generator_version": GENERATOR_VERSION,
        "gate_version": GATE_VERSION,
        "network_requests": 0,
        "candidate_universe_sha256": digest(candidate_path),
        "historical_source_hashes": {str(p.relative_to(ROOT)): digest(p) for p in historical_paths},
        "historical_result_manifest_hashes": {str(p.relative_to(ROOT)): digest(p) for p in result_paths},
        "thresholds": decision.thresholds,
        "threshold_classification": {k: "POLICY_DEFAULT" for k in decision.thresholds},
        "metrics": decision.metrics,
        "risk_classifications": decision.metrics["RISK_CLASSIFICATIONS"],
        "capacity_model": {
            "description": "conservative arm-specific observed V5 yield; gate uses weaker arm for both arms",
            "target_valid_per_arm": TARGET_VALID_PER_ARM,
            "conservative_valid_per_query": CONSERVATIVE_GATE_YIELD,
            "conservative_valid_per_query_treatment": TREATMENT_EVIDENCE_YIELD,
            "conservative_valid_per_query_control": CONTROL_EVIDENCE_YIELD,
            "expected_valid_treatment": t_count * TREATMENT_EVIDENCE_YIELD,
            "expected_valid_control": c_count * CONTROL_EVIDENCE_YIELD,
            "capacity_margin_treatment": t_count * TREATMENT_EVIDENCE_YIELD - TARGET_VALID_PER_ARM,
            "capacity_margin_control": c_count * CONTROL_EVIDENCE_YIELD - TARGET_VALID_PER_ARM,
            "treatment_capacity_ratio": t_count * TREATMENT_EVIDENCE_YIELD / TARGET_VALID_PER_ARM,
            "control_capacity_ratio": c_count * CONTROL_EVIDENCE_YIELD / TARGET_VALID_PER_ARM,
        },
        "gate_decision": decision.as_dict(),
        "artifact_hashes": {},
    }
    evaluation_path = out / "novelty-diversity-evaluation.json"
    write_json(evaluation_path, evaluation)
    evaluation["artifact_hashes"] = {
        "candidate_universe": digest(candidate_path),
        "candidate_universe_manifest": digest(manifest_path),
        "gate_decision": digest(out / "gate-decision.json"),
    }
    write_json(evaluation_path, evaluation)
    print(json.dumps({"decision": decision.decision, "candidate": str(candidate_path), "manifest": str(manifest_path)}, sort_keys=True))
    raise SystemExit(0 if decision.decision == "PASS" else 1)


if __name__ == "__main__":
    main()
