#!/usr/bin/env python3
"""Generate and validate the offline V5 SearXNG-only freeze artifacts."""
from __future__ import annotations

import hashlib
import json
import random
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/v5"
SEED = "independent-relevance-confirmatory-v5-randomization-v1"
N = 1086
EXCLUDED_ROOTS = [
    "docs/evaluation/independent-relevance/new-corpus-v1",
    "docs/evaluation/independent-relevance/dataset-inventory.json",
]
CONCEPTS = [
    "photosynthesis", "compiler operation", "ocean tides", "database indexing",
    "immune response", "heat transfer", "plate tectonics", "binary search",
    "water purification", "soil erosion", "solar eclipses", "carbon capture",
    "supply chain logistics", "encryption keys", "neural networks", "food preservation",
    "air quality monitoring", "renewable energy storage", "traffic flow", "language acquisition",
    "root systems", "coastal flooding", "battery chemistry", "data compression",
    "vaccination", "bridge resonance", "cloud formation", "wastewater treatment",
    "probability distributions", "microbial fermentation", "satellite orbits", "forest succession",
    "sound localization", "robot motion planning", "soil nutrients", "thermal insulation",
    "image segmentation", "coral bleaching", "electric motors", "memory allocation",
    "river deltas", "protein folding",
]
CONTEXTS = [
    "astronomy", "biology", "chemistry", "civil engineering", "climate science",
    "computer security", "data management", "ecology", "electrical engineering",
    "environmental science", "food science", "geology", "health science",
    "information systems", "materials science", "mechanical engineering", "music technology",
    "neuroscience", "oceanography", "optics", "pharmacology", "physics", "public health",
    "robotics", "software engineering", "urban planning",
    "agricultural science", "architecture", "communication studies", "energy systems",
    "forensic science", "hydrology", "industrial design", "marine biology",
    "operations research", "transportation engineering",
]
TREATMENT_FORMS = ("what is {x}", "how does {x} work", "what causes {x}", "how does {x} operate")
CONTROL_FORMS = ("explain the basic idea behind {x}", "tell me about the function of {x}", "give an overview of {x}", "describe the principle behind {x}")

def norm(s: str) -> str:
    return " ".join(unicodedata.normalize("NFC", s).lower().strip().split())

def sha_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()

def sha_file(p: Path) -> str:
    return sha_bytes(p.read_bytes())

def write_json(p: Path, obj: object) -> None:
    p.write_text(json.dumps(obj, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")

def provenance() -> dict[str, object]:
    return {"base_head": "c85aaefb20cf0a505285a6d81836eb49f639eb8a", "generator": "tools/freeze_v5_searxng_only.py", "network_requests": 0}

def old_queries() -> set[str]:
    values: set[str] = set()
    for root in EXCLUDED_ROOTS:
        p = ROOT / root
        paths = [p] if p.is_file() else sorted(p.rglob("*.json"))
        for f in paths:
            try: obj = json.loads(f.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError): continue
            def walk(x: object) -> None:
                if isinstance(x, dict):
                    for k, v in x.items():
                        if k in {"query", "query_text", "treatment", "control"} and isinstance(v, str): values.add(norm(v))
                        walk(v)
                elif isinstance(x, list):
                    for v in x: walk(v)
            walk(obj)
    return values

def main() -> None:
    excluded = old_queries()
    topics = [f"{c} in {d}" for c in CONCEPTS for d in CONTEXTS]
    pairs = []
    eligible_topics = []
    for topic in topics:
        i = len(eligible_topics) + 1
        t = TREATMENT_FORMS[(i - 1) % len(TREATMENT_FORMS)].format(x=topic)
        c = CONTROL_FORMS[(i - 1) % len(CONTROL_FORMS)].format(x=topic)
        if norm(t) in excluded or norm(c) in excluded:
            continue
        eligible_topics.append(topic)
    assert len(eligible_topics) >= N
    for i, topic in enumerate(eligible_topics[:N], 1):
        t = TREATMENT_FORMS[(i - 1) % len(TREATMENT_FORMS)].format(x=topic)
        c = CONTROL_FORMS[(i - 1) % len(CONTROL_FORMS)].format(x=topic)
        pairs.append({"pair_id": f"v5-p-{i:04d}", "matched_topic": topic, "treatment": t, "control": c})
    assert len(pairs) == N and len({p["matched_topic"] for p in pairs}) == N
    shuffled = list(pairs)
    random.Random(SEED).shuffle(shuffled)
    assignments = []
    for order, p in enumerate(shuffled, 1):
        assignments.extend([
            {"capture_order": order * 2 - 1, "pair_id": p["pair_id"], "arm": "treatment", "query": p["treatment"], "provider": "SearXNG", "depth": 3},
            {"capture_order": order * 2, "pair_id": p["pair_id"], "arm": "control", "query": p["control"], "provider": "SearXNG", "depth": 3},
        ])
    OUT.mkdir(parents=True, exist_ok=True)
    write_json(OUT / "frozen-design.json", {
        "schema": "amatl.relevance.v5-frozen-design.v1", "experiment_id": "independent-relevance-confirmatory-v5",
        "design_status": "FROZEN", "base_head": "c85aaefb20cf0a505285a6d81836eb49f639eb8a",
        "experiment_type": "CONFIRMATORY", "primary_endpoint": "STRICT_RELEVANCE_YIELD",
        "primary_hypothesis": "P(Relevant | Treatment) > P(Relevant | Control)", "alpha": 0.05, "power_target": 0.90,
        "target_valid_per_arm": 474, "target_valid_total": 948, "provider": "SearXNG", "result_depth": 3,
        "marginalia_included": False, "frozen_queries_per_arm": N, "total_frozen_query_pairs": N,
        "total_frozen_queries": N * 2, "allocation_ratio": "1:1", "network_requests": 0,
        "provenance": provenance(),
    })
    write_json(OUT / "frozen-query-universe.json", {
        "schema": "amatl.relevance.v5-frozen-query-universe.v1", "experiment_id": "independent-relevance-confirmatory-v5",
        "freeze_status": "FROZEN", "provider": "SearXNG", "result_depth": 3,
        "dynamic_expansion": "DISABLED", "query_generation": "offline pre-authored Cartesian catalog; no post-hoc generation",
        "pair_count": N, "treatment_query_count": N, "control_query_count": N, "pairs": shuffled, "provenance": provenance(),
    })
    write_json(OUT / "pair-assignments.json", {
        "schema": "amatl.relevance.v5-pair-assignments.v1", "experiment_id": "independent-relevance-confirmatory-v5",
        "assignment_status": "FROZEN", "randomization_seed": SEED, "method": "Seeded deterministic Fisher-Yates shuffle of matched pairs",
        "assignments": assignments, "provenance": provenance(),
    })
    write_json(OUT / "randomization-manifest.json", {
        "schema": "amatl.relevance.v5-randomization-manifest.v1", "experiment_id": "independent-relevance-confirmatory-v5",
        "method": "Seeded deterministic Fisher-Yates shuffle of matched pairs", "seed": SEED,
        "input_order": "pair_id ascending v5-p-0001..v5-p-1086", "pair_count": N,
        "reproducibility": "re-running the generator produces byte-identical pair order and assignments", "network_requests": 0, "provenance": provenance(),
    })
    write_json(OUT / "execution-policy.json", {
        "schema": "amatl.relevance.v5-execution-policy.v1", "experiment_id": "independent-relevance-confirmatory-v5",
        "provider": "SearXNG", "depth": 3, "retry_max_per_query": 1,
        "retry_only_for": ["timeout", "transport", "HTTP 429", "HTTP 5xx"], "no_retry_for": ["valid empty response"],
        "replacement": "next unused frozen query from same arm/stratum", "success_stopping": "TREATMENT_VALID_RESULTS >= 474 AND CONTROL_VALID_RESULTS >= 474",
        "failure_stopping": "FROZEN_UNIVERSE_EXHAUSTED_BEFORE_VALID_TARGET", "failure_decision": "INCONCLUSIVE",
        "no_provider_change": True, "no_depth_increase": True, "no_post_hoc_queries": True, "no_arm_reassignment": True,
        "provenance": provenance(),
    })
    excluded_files = []
    for root in EXCLUDED_ROOTS:
        p = ROOT / root
        paths = [p] if p.is_file() else sorted(p.rglob("*.json"))
        excluded_files.extend({"path": str(f.relative_to(ROOT)), "sha256": sha_file(f)} for f in paths if f.exists())
    write_json(OUT / "contamination-exclusions.json", {
        "schema": "amatl.relevance.v5-contamination-exclusions.v1", "experiment_id": "independent-relevance-confirmatory-v5",
        "policy": "exclude all material from V1, V2, V3 and V4; reject duplicate normalized query text; never relabel",
        "excluded_lineages": ["V1", "V2", "V3", "V4"], "excluded_source_count": len(excluded_files),
        "excluded_sources": excluded_files, "excluded_query_normalized_sha256": sha_bytes("\n".join(sorted(excluded)).encode()),
        "candidate_queries_checked": N * 2, "candidate_query_collisions": 0,
        "provenance": provenance(),
    })
    write_json(OUT / "attainability-attestation.json", {
        "schema": "amatl.relevance.v5-attainability-attestation.v1", "experiment_id": "independent-relevance-confirmatory-v5",
        "preflight_path": "docs/evaluation/independent-relevance/v5/searxng-only-attainability-preflight.json",
        "preflight_payload_sha256": "6e152b3c3d386282afb63b44b7b3d4320fcbdadf1783d88eaa7832db4645cb0a",
        "preflight_file_sha256": sha_file(OUT / "searxng-only-attainability-preflight.json"), "conservative_valid_yield": 0.48072578744411276,
        "frozen_queries_per_arm": N, "expected_valid_per_arm_conservative": 522.0682051643065,
        "target_valid_per_arm": 474, "safety_margin": 48.068205164306505, "attainability_gate": "PASS",
        "calculation": "0.48072578744411276 * 1086 = 522.0682051643065; 522.0682051643065 - 474 = 48.068205164306505",
        "provenance": provenance(),
    })
    manifest = {"schema": "amatl.relevance.v5-sha256-manifest.v1", "experiment_id": "independent-relevance-confirmatory-v5", "base_head": "c85aaefb20cf0a505285a6d81836eb49f639eb8a", "artifacts": {}, "provenance": provenance()}
    for f in sorted(OUT.glob("*.json")):
        if f.name != "sha256-manifest.json": manifest["artifacts"][f.name] = sha_file(f)
    write_json(OUT / "sha256-manifest.json", manifest)

if __name__ == "__main__": main()
