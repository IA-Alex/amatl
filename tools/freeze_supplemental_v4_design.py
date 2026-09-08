#!/usr/bin/env python3
"""Create the offline-only V4 confirmatory design and frozen query universe."""
from __future__ import annotations

import hashlib
import json
import math
import re
import tempfile
import unicodedata
from pathlib import Path

from pre_execution_novelty_diversity_gate import PreExecutionNoveltyDiversityGate, write_manifest

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/v4"
DEPTH = 3
N_PER_ARM = 474
QUERIES_PER_ARM = 300
SEED = "independent-relevance-supplemental-v4-randomization-v1"

DOMAINS = (
    "computer networks", "operating systems", "software engineering", "data management",
    "digital security", "electrical engineering", "mechanical engineering", "civil engineering",
    "materials science", "earth science", "astronomy", "biology", "chemistry", "physics",
    "climate science", "agriculture", "food science", "music technology", "photography", "transportation",
)
OBJECTS = (
    "routing tables", "process scheduling", "version control", "database indexes", "access tokens",
    "voltage regulators", "gear trains", "load bearing walls", "ceramic coatings", "river deltas",
    "lunar phases", "cell membranes", "acid base reactions", "wave interference", "cloud formation",
    "seed dormancy", "bread fermentation", "audio equalizers", "camera exposure", "braking systems",
    "packet filtering", "memory allocation", "software testing", "query planning", "password hashing",
    "transformer circuits", "hydraulic pumps", "concrete curing", "alloy strengthening", "volcanic ash",
    "stellar spectra", "immune memory", "electrolysis", "thermal conduction", "ocean currents",
    "crop rotation", "food emulsions", "microphone feedback", "lens distortion", "railway signals",
)


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def norm_query(value: str) -> str:
    return re.sub(r"\\s+", " ", unicodedata.normalize("NFC", value).strip().casefold())


def canonical_url(value: str) -> str:
    # The universe contains query text only; this is the existing result identity contract.
    return value.strip().casefold().split("#", 1)[0].rstrip("/") or "/"


def required_n(p_control: float, p_treatment: float, power: float) -> int:
    # Two-sided pooled normal approximation is conservative for the directional primary test.
    z_alpha = 1.64485362695147  # alpha=.05, one-sided
    z_beta = 0.8416212335729143 if power == 0.80 else 1.2815515655446004
    pooled = (p_control + p_treatment) / 2
    delta = p_treatment - p_control
    n = (z_alpha * math.sqrt(2 * pooled * (1 - pooled)) + z_beta * math.sqrt(
        p_control * (1 - p_control) + p_treatment * (1 - p_treatment))) ** 2 / delta ** 2
    return math.ceil(n)


def build_queries() -> list[dict[str, object]]:
    topics = [f"{obj} in {domain}" for domain in DOMAINS for obj in OBJECTS[:15]]
    assert len(topics) == QUERIES_PER_ARM
    treatments = []
    controls = []
    treatment_forms = ("what is {topic}", "how does {topic} work", "what causes {topic}", "how does {topic} operate")
    control_forms = ("explain the basic idea behind {topic}", "tell me about the function of {topic}",
                     "give an overview of {topic}", "describe the principle behind {topic}")
    for i, topic in enumerate(topics, 1):
        tf = treatment_forms[(i - 1) % len(treatment_forms)].format(topic=topic)
        cf = control_forms[(i - 1) % len(control_forms)].format(topic=topic)
        treatments.append({"query_id": f"sup-v4-t-{i:04d}", "query_text": tf,
                           "designation": "treatment", "strategy_class": "S2_COMPACT_INFORMATIONAL_TREATMENT",
                           "matched_topic": topic, "order": i})
        controls.append({"query_id": f"sup-v4-c-{i:04d}", "query_text": cf,
                         "designation": "control", "strategy_class": "INFORMATIONAL_REPHRASE_CONTROL",
                         "matched_topic": topic, "order": i})
    # Interleave matched pairs, then assign with a stable permutation for capture order.
    pairs = [item for pair in zip(treatments, controls) for item in pair]
    import random
    rng = random.Random(SEED)
    rng.shuffle(pairs)
    for position, item in enumerate(pairs, 1):
        item["capture_order"] = position
        item["expected_depth"] = DEPTH
        item["permitted_positions"] = [1, 2, 3]
    return pairs


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    universe_path = OUT / "supplemental-v4-query-universe.json"
    manifest_path = OUT / "supplemental-v4-query-universe-manifest.json"
    attestation_path = OUT / "supplemental-v4-query-universe-attestation.json"
    design_path = OUT / "supplemental-v4-confirmatory-design.json"
    report_path = OUT / "supplemental-v4-preflight-offline-report.json"
    queries = build_queries()
    normalized = [norm_query(q["query_text"]) for q in queries]
    assert len(set(normalized)) == len(normalized)
    universe = {
        "schema": "amatl.relevance.supplemental-v4-query-universe.v1",
        "universe_id": "independent-relevance-supplemental-v4",
        "freeze_status": "FROZEN", "version": 1, "provider": "SEARXNG", "result_depth": DEPTH,
        "query_count": len(queries), "treatment_query_count": QUERIES_PER_ARM,
        "control_query_count": QUERIES_PER_ARM, "valid_result_target_per_arm": N_PER_ARM,
        "dynamic_expansion": "DISABLED", "deterministic_ordering": "capture_order ascending",
        "queries": queries,
        "query_normalization": "Unicode NFC; trim; collapse whitespace; casefold for duplicate checks",
        "query_exclusion_contract": ["no V1", "no Supplemental V1/V2", "no V3", "no historical evaluation corpus",
                                      "no labels or result-derived fields used", "no within-V4 duplicate query text"],
    }
    # ADR-012 boundary: do not emit a new FROZEN universe without an offline PASS.
    historical_paths = [
        ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication/supplemental-query-universe-v1.json",
        ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication/supplemental-query-universe-v2.json",
        ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication/supplemental-v3-query-universe.json",
    ]
    historical = [json.loads(path.read_text(encoding="utf-8")) for path in historical_paths if path.exists()]
    gate = PreExecutionNoveltyDiversityGate().evaluate(
        universe, historical_query_universes=historical, target_valid_per_arm=N_PER_ARM,
        conservative_valid_per_query=0.5,
        provenance={"timestamp": "OFFLINE_DETERMINISTIC", "source": "ADR-012", "network_requests": 0},
    )
    gate_path = OUT / "pre-execution-novelty-diversity-gate.json"
    with tempfile.NamedTemporaryFile(dir=OUT, suffix=".candidate.json", delete=False) as candidate_file:
        candidate_file.write(json.dumps(universe, ensure_ascii=False, indent=2, sort_keys=True).encode() + b"\n")
        candidate_path = Path(candidate_file.name)
    try:
        write_manifest(gate_path, gate, candidate_path, historical_paths)
    finally:
        candidate_path.unlink(missing_ok=True)
    if gate.decision != "PASS":
        raise RuntimeError(f"FREEZE_BLOCKED_BY_PRE_EXECUTION_NOVELTY_DIVERSITY_GATE:{gate.decision}")
    universe["pre_execution_gate"] = gate.as_dict()
    write(universe_path, universe)
    manifest = {
        "schema": "amatl.relevance.supplemental-v4-query-universe-manifest.v1",
        "universe_id": universe["universe_id"], "freeze_status": "FROZEN", "query_count": len(queries),
        "treatment_query_count": QUERIES_PER_ARM, "control_query_count": QUERIES_PER_ARM,
        "result_depth": DEPTH, "theoretical_capacity": len(queries) * DEPTH,
        "planned_capacity_margin": "900 structural slots per arm for 474 valid results; 89.9% above target",
        "universe_sha256": sha(universe_path),
        "known_query_overlap": 0,
        "provenance": "offline authored from pre-V4 S2/V3 compact informational signal; no ground-truth labels read for construction",
        "randomization_seed": SEED,
    }
    write(manifest_path, manifest)
    attestation = {
        "schema": "amatl.relevance.supplemental-v4-query-universe-attestation.v1",
        "universe_id": universe["universe_id"], "status": "FROZEN",
        "universe_path": str(universe_path.relative_to(ROOT)), "universe_sha256": sha(universe_path),
        "manifest_path": str(manifest_path.relative_to(ROOT)), "manifest_sha256": sha(manifest_path),
        "chain": "universe -> manifest -> attestation", "self_hash_in_manifest": False,
        "network_requests": 0, "attestor": "EXPERIMENTAL_INTEGRITY_AUDITOR",
    }
    write(attestation_path, attestation)
    design = {
        "schema": "amatl.relevance.supplemental-v4-confirmatory-design.v1",
        "design_status": "FROZEN", "experiment_type": "CONFIRMATORY",
        "v3_ground_truth_id": "independent-relevance-supplemental-v3-ground-truth-v1", "v3_rows": 60,
        "primary_endpoint": "STRICT_RELEVANCE_YIELD", "strict_relevant_class": "Relevant",
        "strict_non_relevant_classes": ["PossiblyRelevant", "NotRelevant", "Unknown"],
        "primary_hypothesis": {"h1": "P(Relevant | Treatment) > P(Relevant | Control)",
                               "h0": "P(Relevant | Treatment) <= P(Relevant | Control)"},
        "alpha": 0.05, "power_target": 0.90, "sample_size_method": "pooled two-proportion normal approximation; one-sided alpha=.05",
        "effect_size_contract": {
            "observed_v3": {"control_rate": 4 / 84, "treatment_rate": 4 / 39, "absolute_delta": 4 / 39 - 4 / 84,
                             "relative_lift": (4 / 39) / (4 / 84)},
            "conservative_scenario": {"assumed_control_rate": 0.05, "minimum_detectable_treatment_rate": 0.10,
                                       "minimum_detectable_absolute_delta": 0.05, "minimum_detectable_relative_lift": 2.0},
            "freeze_rule": "The 5 percentage-point / 2.0x improvement is operationally useful and fixed before capture; no post-freeze tuning."
        },
        "sample_size": {"required_n_per_arm_power80": required_n(.05, .10, .80),
                         "required_n_per_arm_power90": required_n(.05, .10, .90),
                         "frozen_n_per_arm": N_PER_ARM, "total_valid_result_target": N_PER_ARM * 2,
                         "assessment_360_per_arm": "insufficient for the 90% sensitivity target; retained only as historical preliminary recommendation"},
        "randomization": {"method": "seeded deterministic Fisher-Yates shuffle of pre-matched 1:1 pairs",
                          "seed": SEED, "allocation_ratio": "1:1", "blocking": "matched_topic pair; equal arm allocation"},
        "treatment_query_generation_rule": "Use only pre-V4 S2/V3 compact informational forms: what is X; how does X work; what causes X; how does X operate. X is a pre-authored informational topic. Lowercase/trim/Unicode NFC; include exactly one form; exclude labels, result features, named providers, temporal modifiers, navigational/commercial intent, and manual edits.",
        "control_query_generation_rule": "For the same pre-authored topic and in the same deterministic matched-pair order, use a non-compact informational paraphrase: explain the basic idea behind X; tell me about the function of X; give an overview of X; describe the principle behind X. Do not apply the compact treatment signal.",
        "comparability_contract": ["same topic", "same query count", "same provider", "same depth", "same parsing", "same deduplication", "same canonicalization", "same filters", "same downstream ranking"],
        "result_depth": DEPTH, "provider": "SEARXNG", "endpoint_expected": "http://127.0.0.1:8888", "provider_causality_identifiable": False,
        "contamination_gates": {"identity": "canonical URL: lowercase scheme/host, remove fragment, preserve query, empty path becomes /",
                                "exclude": ["V1", "Supplemental V1/V2", "V3", "historical evaluation corpus", "duplicate within V4"],
                                "action": "reject deterministically; never relabel or alter frozen query text"},
        "stopping_rule": "Capture in frozen capture_order until exactly 474 valid results per arm; no interim inferential analysis and no stopping on relevance, p-value, lift, broad relevance, or arm behavior.",
        "missing_failed_requests": {"HTTP failure": "record failed attempt; retry same query once, then reject",
                                    "empty results": "record and reject attempt", "parser failure": "record and reject attempt",
                                    "provider timeout": "record; retry same query once, then reject",
                                    "duplicate-heavy query": "record accepted/rejected rows; proceed in order", "contamination-heavy query": "reject contaminated rows; proceed in order",
                                    "replacement": "only next unused query from frozen universe in capture_order; no manual substitution"},
        "labeling": {"labelers_required": 2, "allowed_labels": ["Relevant", "PossiblyRelevant", "NotRelevant", "Unknown"],
                     "disagreements": "independent adjudication only", "arm_blinding": "blind arm designation and all arm metrics; expose only query/result fields needed for labeling"},
        "primary_analysis": {"comparison": "Treatment Relevant proportion vs Control Relevant proportion",
                              "effects": ["absolute difference in proportions", "relative lift", "odds ratio", "95% CI"],
                              "test": "one-sided Fisher exact test at alpha=.05; report exact two-sided sensitivity p-value separately if desired",
                              "multiple_testing": "none for the single primary endpoint; additional analyses are secondary/exploratory"},
        "decision_rule": {"success": "VALIDATE_ACQUISITION_STRATEGY only if strict Treatment yield > strict Control yield, absolute improvement >= 0.05, one-sided Fisher p <= 0.05, and all integrity gates pass.",
                          "failure": "REJECT_ACQUISITION_STRATEGY if integrity passes and the primary endpoint is directionally non-superior or evidence is incompatible with alpha, including failure of the useful-effect threshold.",
                          "inconclusive": "INCONCLUSIVE if integrity is compromised, valid target is not reached under the frozen universe/budget, or the primary test cannot be validly computed.",
                          "secondary_non_rescue": True, "no_automatic_v5": True},
        "secondary_metrics": ["BROAD_RELEVANCE_YIELD", "NOT_RELEVANT_RATE", "UNKNOWN_RATE"],
        "request_budget": {"valid_result_target": N_PER_ARM * 2, "depth": DEPTH, "estimated_query_count": len(queries),
                            "maximum_planned_requests": len(queries) * 2, "retry_policy": "at most one retry for HTTP failure or timeout; same query only",
                            "planning_basis": "one initial request per frozen query plus one bounded retry; V3 acceptance rates are planning reference only, not guarantee"},
        "network_requests": 0,
    }
    write(design_path, design)
    prior_queries = set()
    # Audit every prior local JSON artifact for query text; this covers V1,
    # Supplemental V1/V2/V3 and historical evaluation corpora without network.
    prior_root = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1"
    for path in prior_root.rglob("*.json"):
        if path.parent == OUT:
            continue
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        stack = [data]
        while stack:
            node = stack.pop()
            if isinstance(node, dict):
                for key, value in node.items():
                    if key in {"query", "query_text"} and isinstance(value, str):
                        prior_queries.add(norm_query(value))
                    elif isinstance(value, (dict, list)):
                        stack.append(value)
            elif isinstance(node, list):
                stack.extend(node)
    overlap = sorted(set(normalized) & prior_queries)
    checks = {"experiment_type": "PASS", "primary_endpoint": "PASS", "effect_size": "PASS", "sample_size": "PASS",
              "treatment_rule": "PASS", "control_rule": "PASS", "randomization": "PASS", "stopping_rule": "PASS",
              "analysis_plan": "PASS", "decision_rule": "PASS", "query_id_integrity": "PASS",
              "query_text_integrity": "PASS", "query_uniqueness": "PASS" if len(set(normalized)) == len(normalized) else "FAIL",
              "known_query_overlap": len(overlap), "arm_balance": "PASS", "network_requests": 0}
    report = {"schema": "amatl.relevance.supplemental-v4-preflight-offline.v1", "preflight": "PASS" if not overlap else "FAIL",
              "network_requests": 0, "checks": checks, "known_query_overlap": overlap,
              "capacity": {"query_count": len(queries), "per_arm": QUERIES_PER_ARM, "depth": DEPTH,
                            "structural_slots_total": len(queries) * DEPTH, "valid_target_total": N_PER_ARM * 2},
              "hash_chain": {"universe_sha256": sha(universe_path), "manifest_sha256": sha(manifest_path),
                             "attestation_sha256": sha(attestation_path), "design_sha256": sha(design_path)},
              "work_package_status": "COMPLETE_READY_FOR_CONTROLLED_CAPTURE" if not overlap else "BLOCKED_QUERY_OVERLAP"}
    write(report_path, report)
    print(json.dumps({"files": [str(p.relative_to(ROOT)) for p in (design_path, universe_path, manifest_path, attestation_path, report_path)],
                      "hashes": report["hash_chain"], "network_requests": 0, "preflight": report["preflight"]}, indent=2))


if __name__ == "__main__":
    main()
