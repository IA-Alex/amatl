#!/usr/bin/env python3
"""Freeze the S1-only, pre-label supplemental universe revision."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"

# Authored before capture; these are deliberately different tasks from v1's
# household/consumer-service query families.  Selection uses no labels.
QUERIES = (
    "how to tune a guitar by ear",
    "how to calibrate a digital kitchen scale",
    "how to bleed air from a home radiator",
    "how to program a universal remote control",
    "how to repot an orchid",
    "how to create a bootable USB installer",
    "how to level a washing machine",
    "how to make a backup of an Android phone",
    "how to season a carbon steel wok",
    "how to replace a watch battery",
    "how to scan a document with an iPhone",
    "how to adjust bicycle disc brakes",
)

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def write_json(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

def main():
    v1 = OUT / "supplemental-query-universe-v1.json"
    parents = {
        name: digest(OUT / name) for name in (
            "supplemental-pilot-raw-evidence.json", "supplemental-pilot-attempts.json",
            "supplemental-pilot-historical-canonical-urls.json", "supplemental-pilot-run-report.json",
            "supplemental-query-universe-v1.json", "supplemental-query-universe-v1-manifest.json",
            "v1-ground-truth.json", "v1-ground-truth-manifest.json",
        )
    }
    v1_queries = {q["query"].casefold().strip() for q in json.loads(v1.read_text())["queries"]}
    if len(QUERIES) != len(set(q.casefold().strip() for q in QUERIES)) or v1_queries & {q.casefold().strip() for q in QUERIES}:
        raise ValueError("V2_QUERY_PREFLIGHT_DUPLICATE")
    queries = [{"query_id": f"sup-v2-s1-{i:03d}", "query": q,
                "assigned_stratum": "S1_PROCEDURAL_SHALLOW", "permitted_positions": [1, 2, 3],
                "semantic_form": "procedural", "order": i}
               for i, q in enumerate(QUERIES, 1)]
    universe = {
        "schema": "amatl.relevance.supplemental-query-universe.v2", "universe_id": "independent-relevance-supplemental-v2",
        "version": 2, "freeze_status": "FROZEN", "parent_universe": "independent-relevance-supplemental-v1",
        "reason_for_revision": "S1 deficit of eight valid rows after v1 frozen-universe exhaustion; S2/S3 remain preserved and complete.",
        "target_stratum": "S1_PROCEDURAL_SHALLOW", "valid_deficit": 8, "provider_requirement": "SEARXNG_ONLY",
        "dynamic_expansion": "DISABLED", "deterministic_ordering": "order ascending; no insertion or expansion after freeze",
        "normalization_rules": {"query": "Unicode NFC, trim, collapse ASCII whitespace, case-insensitive duplicate check",
                                "url": "lowercase scheme/host; remove fragment; preserve query; normalize empty path to slash"},
        "exclusion_rules": ["S1 only; ranks 1..3 only", "no v1 query text", "no label-informed selection",
                            "deduplicate accepted v2 rows against v1 ground truth, historical URLs, supplemental-v1 valid rows, and accepted v2 rows"],
        "queries": queries,
    }
    universe_path = OUT / "supplemental-query-universe-v2.json"
    write_json(universe_path, universe)
    manifest = {
        "schema": "amatl.relevance.supplemental-query-universe-manifest.v2", "universe_id": universe["universe_id"],
        "version": 2, "freeze_status": "FROZEN", "query_count": len(queries), "structural_capacity": len(queries) * 3,
        "target_stratum": universe["target_stratum"], "valid_deficit": 8, "universe_sha256": digest(universe_path),
        "parent_hashes": parents, "provenance": {"run_report": "supplemental-pilot-run-report.json",
            "run_status": "EXHAUSTED_FROZEN_UNIVERSE", "preserved_valid_rows": 172, "preserved_s1_rows": 52,
            "preserved_s2_rows": 60, "preserved_s3_rows": 60},
        "capacity_justification": {"v1_s1_raw_results": 81, "v1_s1_valid_rows": 52,
            "observed_valid_rate": 52 / 81, "observed_attrition_rate": 29 / 81,
            "v2_capacity": len(queries) * 3, "margin_vs_deficit": (len(queries) * 3) / 8,
            "rationale": "36 slots are 4.5x the deficit; at one half of the observed structural valid rate they yield 11.56 rows. This is a capacity margin, not a relevance estimate."},
    }
    write_json(OUT / "supplemental-query-universe-v2-manifest.json", manifest)
    config = {"schema": "amatl.relevance.supplemental-v2-searxng-capture-config.v1", "experiment_id": "independent-relevance-supplemental-v2",
        "universe_path": str(universe_path.relative_to(ROOT)), "query_universe_id": universe["universe_id"], "query_universe_hash": digest(universe_path),
        "provider": "searxng", "provider_fallback": "none", "dynamic_provider_selection": False, "dynamic_query_expansion": False,
        "authorization_source": "EXPLICIT_OPERATOR_INPUT", "authorized_scope": "S1-only supplemental-v2 acquisition; stop after 8 new valid rows",
        "authorized_provider": "SEARXNG_ONLY", "searxng_endpoint_binding": {"source": "EXPLICIT_OPERATOR_INPUT", "endpoint": "http://127.0.0.1:8888", "endpoint_sha256": hashlib.sha256(b"http://127.0.0.1:8888").hexdigest()},
        "retry_policy": {"max_attempts": 2, "same_query_only": True, "retryable_error_classes": ["TIMEOUT", "TEMPORARY_HTTP_FAILURE"]},
        "rank_depth_policy": {"source": "frozen_universe_permitted_positions"}, "s1_new_valid_target": 8,
        "dedup_inputs": ["v1-ground-truth.json", "supplemental-pilot-historical-canonical-urls.json", "supplemental-pilot-attempts.json", "accepted-v2-rows"],
    }
    write_json(OUT / "supplemental-v2-searxng-capture-config.json", config)
    write_json(OUT / "supplemental-v2-multirun-provenance-contract.json", {
        "schema": "amatl.relevance.supplemental-v2-multirun-provenance-contract.v1",
        "status": "FROZEN", "preserved_source_run": "supplemental-v1", "preserved_rows": 172,
        "new_source_run": "supplemental-v2", "new_rows_required": 8, "final_rows_expected": 180,
        "row_identity_rule": "Preserve existing supplemental-v1 row IDs exactly; assign SOURCE_RUN without regenerating them.",
        "dedup_order": ["v1-ground-truth", "historical", "supplemental-v1-valid", "accepted-supplemental-v2"],
    })

if __name__ == "__main__": main()
