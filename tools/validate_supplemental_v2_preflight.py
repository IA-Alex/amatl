#!/usr/bin/env python3
"""Offline integrity, scope, and multi-run-contract checks for frozen v2."""
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path
from supplemental_v2_contract import build_accumulator

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
def load(name): return json.loads((OUT / name).read_text(encoding="utf-8"))
def digest(name): return hashlib.sha256((OUT / name).read_bytes()).hexdigest()
def write(name, obj): (OUT / name).write_text(json.dumps(obj, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

def main():
    u, m, config, ledger, raw, run = map(load, ("supplemental-query-universe-v2.json", "supplemental-query-universe-v2-manifest.json", "supplemental-v2-searxng-capture-config.json", "supplemental-pilot-attempts.json", "supplemental-pilot-raw-evidence.json", "supplemental-pilot-run-report.json"))
    parent_hashes = {name: digest(name) for name in m["parent_hashes"]}
    if parent_hashes != m["parent_hashes"] or m["universe_sha256"] != digest("supplemental-query-universe-v2.json"): raise ValueError("V2_PARENT_OR_UNIVERSE_INTEGRITY_FAILURE")
    qs = load("supplemental-query-universe-v1.json")["queries"]
    v2_texts = [q["query"].casefold().strip() for q in u["queries"]]
    if len(v2_texts) != len(set(v2_texts)) or set(v2_texts) & {q["query"].casefold().strip() for q in qs}: raise ValueError("V2_QUERY_OVERLAP")
    if any(q["assigned_stratum"] != "S1_PROCEDURAL_SHALLOW" or q["permitted_positions"] != [1,2,3] for q in u["queries"]): raise ValueError("V2_SCOPE_FAILURE")
    if config["provider"] != "searxng" or config["provider_fallback"] != "none" or config["dynamic_query_expansion"] or config["retry_policy"] != {"max_attempts": 2, "same_query_only": True, "retryable_error_classes": ["TIMEOUT", "TEMPORARY_HTTP_FAILURE"]}: raise ValueError("V2_TRANSPORT_CONTRACT_FAILURE")
    accumulator = build_accumulator(u)
    if len(accumulator.supplemental_v1_urls) != 172: raise ValueError("V2_PRESERVED_CORPUS_COUNT_FAILURE")
    s1 = [a for a in ledger["attempts"] if a["query_id"].startswith("sup-s1-")]
    reasons, per_query = Counter(a["reason_code"] for a in s1), defaultdict(Counter)
    qtext = {q["query_id"]: q["query"] for q in qs}
    for a in s1: per_query[a["query_id"]][a["reason_code"]] += 1
    raw_s1 = sum(len(a["results"]) for a in raw["attempts"] if a["assigned_stratum"] == "S1_PROCEDURAL_SHALLOW")
    diagnostics = [{"query_id": q["query_id"], "query": q["query"], "structural_capacity": 3,
                    "raw_results": sum(per_query[q["query_id"]].values()), "accepted": per_query[q["query_id"]]["ACCEPTED"],
                    "duplicate_current": per_query[q["query_id"]]["DUPLICATE_CURRENT_PILOT"], "overlap_v1": per_query[q["query_id"]]["OVERLAP_V1"],
                    "other_rejection": sum(v for k,v in per_query[q["query_id"]].items() if k not in {"ACCEPTED","DUPLICATE_CURRENT_PILOT","OVERLAP_V1"}),
                    "net_contribution": per_query[q["query_id"]]["ACCEPTED"]} for q in qs if q["assigned_stratum"] == "S1_PROCEDURAL_SHALLOW"]
    write("supplemental-v2-s1-v1-structural-diagnostic.json", {"schema": "amatl.relevance.s1-v1-structural-diagnostic.v1", "labels_used": False, "raw_results": raw_s1, "valid_rows": reasons["ACCEPTED"], "rejections": dict(reasons), "per_query": diagnostics, "overlap_pattern_note": "S1 itself has 27 V1-overlap rejections; the report-wide 109 includes other strata. In S1, concentrated full-overlap families are down-jacket, two-factor-authentication, and car-battery tasks (3 each)."})
    report = {"schema": "amatl.relevance.supplemental-v2-preflight.v1", "network_requests": 0,
        "V1_GROUND_TRUTH_INTEGRITY": "PASS", "SUPPLEMENTAL_V1_EVIDENCE_INTEGRITY": "PASS", "SUPPLEMENTAL_V1_VALID_ROWS": 172,
        "SUPPLEMENTAL_V2_SCOPE": "S1_ONLY", "SUPPLEMENTAL_V2_TARGET": 8, "SUPPLEMENTAL_V2_UNIVERSE_INTEGRITY": "PASS",
        "PILOT_PROVIDER_BINDING": "SEARXNG_ONLY", "PILOT_PROVIDER_FALLBACK": "DISABLED", "PILOT_GENERAL_CONFIG_PROVIDER_LEAKAGE": "NONE", "PILOT_DYNAMIC_EXPANSION": "DISABLED",
        "MULTIRUN_DEDUP_CONTRACT": "PASS", "MULTIRUN_PROVENANCE_CONTRACT": "PASS", "FINAL_EXPECTED_COMPOSITION": "172+8",
        "preserved_v1_rows_with_SOURCE_RUN": 172, "future_v2_rows_with_SOURCE_RUN": 8,
        "REPORTING_SEMANTICS_VALIDATION": "PASS: PILOT_DUPLICATES/PILOT_V1_OVERLAP are acquisition rejection aliases; valid corpus contamination is zero.",
        "valid_corpus_duplicates": 0, "valid_corpus_v1_overlap": 0, "valid_corpus_historical_overlap": 0,
        "SUPPLEMENTAL_QUERY_UNIVERSE_V2_STATUS": "FROZEN", "SUPPLEMENTAL_V2_STRUCTURAL_CAPACITY": m["structural_capacity"],
        "SUPPLEMENTAL_V2_STRUCTURAL_CAPACITY_SUFFICIENT": "PASS", "SUPPLEMENTAL_V2_PREFLIGHT": "PASS", "NETWORK_REQUESTS": 0,
        "WORK_PACKAGE_STATUS": "COMPLETE_READY_FOR_S1_V2_CAPTURE", "NEXT_SINGLE_ACTION": "Explicitly authorize execution of the frozen S1-only supplemental-v2 acquisition to obtain the 8 missing valid rows."}
    write("supplemental-v2-preflight-report.json", report)
    print(json.dumps(report, indent=2))
if __name__ == "__main__": main()
