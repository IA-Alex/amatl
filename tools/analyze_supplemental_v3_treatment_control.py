#!/usr/bin/env python3
"""Offline, reproducible analysis of the frozen Supplemental V3 experiment."""
import hashlib
import json
import math
from collections import Counter
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
GT = OUT / "supplemental-v3-ground-truth-v1.json"
MANIFEST = OUT / "supplemental-v3-ground-truth-v1-manifest.json"
ATTESTATION = OUT / "supplemental-v3-ground-truth-v1-attestation.json"
ARTIFACT = OUT / "supplemental-v3-treatment-control-analysis.json"
LABELS = ("Relevant", "PossiblyRelevant", "NotRelevant", "Unknown")
Z = 1.959963984540054


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def canonical_url(value):
    parts = urlsplit(value.strip())
    return urlunsplit((parts.scheme.lower(), parts.netloc.lower(), parts.path.rstrip("/") or "/", parts.query, ""))


def wilson(successes, trials):
    p = successes / trials
    denominator = 1 + Z * Z / trials
    centre = (p + Z * Z / (2 * trials)) / denominator
    half = Z * math.sqrt(p * (1 - p) / trials + Z * Z / (4 * trials * trials)) / denominator
    return [centre - half, centre + half]


def fisher_two_sided(a, b, c, d):
    """Two-sided Fisher p-value by direct fixed-margin enumeration."""
    lower = max(0, (a + b) - (b + d))
    upper = min(a + b, a + c)
    denominator = math.comb(a + b + c + d, a + c)

    def probability(x):
        return math.comb(a + b, x) * math.comb(c + d, a + c - x) / denominator

    observed = probability(a)
    p_value = sum(probability(x) for x in range(lower, upper + 1) if probability(x) <= observed * (1 + 1e-12))
    odds_ratio = None if b * c == 0 else (a * d) / (b * c)
    return odds_ratio, p_value


def distribution(rows):
    counts = {label: sum(row["final_label"] == label for row in rows) for label in LABELS}
    n = len(rows)
    strict = counts["Relevant"]
    broad = strict + counts["PossiblyRelevant"]
    return {
        "rows": n, "counts": counts,
        "strict_yield": strict / n, "broad_yield": broad / n,
        "not_relevant_rate": counts["NotRelevant"] / n,
        "unknown_rate": counts["Unknown"] / n,
        "strict_ci95_wilson": wilson(strict, n),
        "broad_ci95_wilson": wilson(broad, n),
    }


def fisher_for(rows_t, rows_c, broad=False):
    positive = (lambda r: r["final_label"] in ("Relevant", "PossiblyRelevant")) if broad else (lambda r: r["final_label"] == "Relevant")
    a, b = sum(map(positive, rows_t)), sum(not positive(r) for r in rows_t)
    c, d = sum(map(positive, rows_c)), sum(not positive(r) for r in rows_c)
    odds, p_value = fisher_two_sided(a, b, c, d)
    return {"table": [[a, b], [c, d]], "odds_ratio": odds, "p_value_two_sided": p_value}


def main():
    ground = load(GT); manifest = load(MANIFEST); attestation = load(ATTESTATION)
    gt_hash, manifest_hash, attestation_hash = sha(GT), sha(MANIFEST), sha(ATTESTATION)
    integrity = {
        "status": "PASS" if (
            ground["status"] == "FROZEN" and ground["ground_truth_id"] == "independent-relevance-supplemental-v3-ground-truth-v1"
            and gt_hash == manifest["input_hashes"]["ground_truth"] == attestation["ground_truth_sha256"]
            and manifest_hash == attestation["manifest_sha256"] and attestation["integrity_status"] == "PASS"
        ) else "FAIL",
        "ground_truth_hash_valid": gt_hash == "bfbc05e893d93f84a908ffd6faf618b4c5a800f8ce8e40b03fcc1cfc97681952",
        "manifest_hash": manifest_hash, "attestation_hash": attestation_hash,
        "manifest_hash_chain": manifest_hash == "24b5760988c97051c17f7b425eabaf4d1a819c7d8d9b426e1c071bdff959b112",
        "attestation_integrity": attestation["integrity_status"] == "PASS",
    }
    if integrity["status"] != "PASS":
        raise SystemExit("WORK_PACKAGE_STATUS=BLOCKED_V3_ANALYSIS_INPUT_INTEGRITY")
    rows = ground["rows"]
    treatment = [r for r in rows if r["designation"] == "treatment"]
    control = [r for r in rows if r["designation"] == "control"]
    corpora = {
        "v1": {canonical_url(r["canonical_url"]) for r in load(OUT / "v1-ground-truth.json")["rows"]},
        "previous_supplemental": {canonical_url(r["canonical_url"]) for r in load(OUT / "supplemental-ground-truth-v1.json")["rows"]},
        "historical": {canonical_url(u) for u in load(OUT / "supplemental-pilot-historical-canonical-urls.json")},
    }
    current = {canonical_url(r["canonical_url"]) for r in rows}
    overlaps = {name: sorted(current & urls) for name, urls in corpora.items()}
    overlap_status = "PASS" if not any(overlaps.values()) else "FAIL"
    if overlap_status != "PASS":
        raise SystemExit("WORK_PACKAGE_STATUS=BLOCKED_V3_ANALYSIS_OVERLAP_FAILURE")
    agg, td, cd = distribution(rows), distribution(treatment), distribution(control)
    strict_delta = td["strict_yield"] - cd["strict_yield"]
    broad_delta = td["broad_yield"] - cd["broad_yield"]
    strict_fisher = fisher_for(treatment, control)
    broad_fisher = fisher_for(treatment, control, broad=True)
    result = {
        "schema": "amatl.relevance.supplemental-v3-treatment-control-analysis.v1",
        "input": {"ground_truth": str(GT.relative_to(ROOT)), "manifest": str(MANIFEST.relative_to(ROOT)), "attestation": str(ATTESTATION.relative_to(ROOT)), "hashes": {"ground_truth": gt_hash, "manifest": manifest_hash, "attestation": attestation_hash}},
        "input_integrity": integrity,
        "overlap_validation": {"status": overlap_status, "counts": {k: len(v) for k, v in overlaps.items()}, "overlaps": overlaps, "contract": "canonical_url; accepted V3 rows must have zero overlap"},
        "aggregate_distribution": agg, "treatment_distribution": td, "control_distribution": cd,
        "strict_metrics": {"treatment_yield": td["strict_yield"], "control_yield": cd["strict_yield"], "absolute_delta_pp": strict_delta * 100, "relative_lift": td["strict_yield"] / cd["strict_yield"] if cd["strict_yield"] else "UNDEFINED_OR_INFINITE", "fisher": strict_fisher},
        "broad_metrics": {"treatment_yield": td["broad_yield"], "control_yield": cd["broad_yield"], "absolute_delta_pp": broad_delta * 100, "relative_lift": td["broad_yield"] / cd["broad_yield"] if cd["broad_yield"] else "UNDEFINED_OR_INFINITE", "not_relevant_delta_pp": (td["not_relevant_rate"] - cd["not_relevant_rate"]) * 100, "fisher": broad_fisher},
        "expectation_comparison": {"expected_strict_yield": 0.10, "expected_relevant_approx": 6, "worst_case_relevant": 0, "observed_strict_yield": agg["strict_yield"], "observed_relevant": agg["counts"]["Relevant"], "expected_yield_met": agg["strict_yield"] >= 0.10, "expected_relevant_count_met": agg["counts"]["Relevant"] >= 6},
        "historical_descriptive_comparisons": {"references": {"v1_strict_yield": 0.024457, "supplemental_aggregate_strict_yield": 0.055556, "s2_discovery_sample_strict_yield": 0.15}, "v3_vs_v1_descriptive_lift": agg["strict_yield"] / 0.024457, "v3_vs_supplemental_descriptive_lift": agg["strict_yield"] / 0.055556, "treatment_vs_v1_descriptive_lift": td["strict_yield"] / 0.024457, "causal_interpretation": "DESCRIPTIVE_ONLY; datasets are not assumed IID and provider causality is not identifiable"},
        "signal_validation": {"signals": ["TITLE_QUERY_OVERLAP", "SNIPPET_QUERY_OVERLAP"], "feature_presence_analysis": "NOT_AVAILABLE_IN_FROZEN_V3_ROWS", "assessment": "validated only through the predeclared treatment/control comparison; no post-hoc features or label-based query construction"},
        "hypothesis_result": "DIRECTIONALLY_SUPPORTED_BUT_UNDERPOWERED",
        "decision": "RUN_CONFIRMATORY_V4",
        "acquisition_strategy_status": "NOT_VALIDATED_PENDING_CONFIRMATORY_V4",
        "v4_recommendation": {"purpose": "Confirm the observed strict-relevance advantage of S2 treatment over control with adequate power", "primary_endpoint": "STRICT_RELEVANCE_YIELD", "recommended_sample_size": {"treatment": 360, "control": 360, "total": 720, "basis": "approximate 80% power planning from observed 10.26% vs 4.76% rates and 5.5 percentage-point delta; exact-power simulation should precede authorization"}, "treatment_control_ratio": "1:1", "rationale": "V3 has only 4 vs 1 Relevant rows; the point estimate favors treatment but Wilson intervals overlap substantially and Fisher two-sided p=0.6486."},
        "limitations": ["Small and unbalanced V3 arms; uncertainty is substantial.", "Fisher p-values do not establish causality.", "PossiblyRelevant remains separate from Relevant and broad relevance is secondary.", "Historical comparisons are descriptive only.", "No row metadata exposes independent TITLE_QUERY_OVERLAP/SNIPPET_QUERY_OVERLAP feature-presence flags."],
        "next_action": "Do not acquire or rank-change now; authorize only a separately designed confirmatory V4 after exact power planning.",
        "provider_causality_identifiable": False, "network_requests": 0, "frozen_artifacts_changed": False,
    }
    ARTIFACT.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"artifact": str(ARTIFACT.relative_to(ROOT)), "artifact_hash": sha(ARTIFACT), "decision": result["decision"], "strict_fisher": strict_fisher}, indent=2))


if __name__ == "__main__":
    main()
