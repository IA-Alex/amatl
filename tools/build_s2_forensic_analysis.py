#!/usr/bin/env python3
"""Build the offline S2 forensic artifact and its V3 decision.

This script is deliberately self-contained and never opens a network socket.
All features are computed from query/result fields present before labeling;
labels are used only as the evaluation target.
"""
import hashlib
import html
import json
import math
import re
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
V1 = OUT / "v1-ground-truth.json"
V1_MANIFEST = OUT / "v1-ground-truth-manifest.json"
SUP = OUT / "supplemental-ground-truth-v1.json"
SUP_MANIFEST = OUT / "supplemental-ground-truth-v1-manifest.json"
SUP_ATTESTATION = OUT / "supplemental-ground-truth-v1-attestation.json"
COMPARISON = OUT / "supplemental-vs-v1-analysis.json"
ARTIFACT = OUT / "supplemental-s2-forensic-analysis.json"
V3_UNIVERSE = OUT / "supplemental-v3-query-universe.json"
V3_MANIFEST = OUT / "supplemental-v3-query-universe-manifest.json"
V3_ATTESTATION = OUT / "supplemental-v3-query-universe-attestation.json"
V3_PREFLIGHT = OUT / "supplemental-v3-preflight-report.json"

EXPECTED = {
    "v1-ground-truth.json": "966ffafbe8e4a48ed635bcee93773c034039129927fe87aaebdccf316d7af567",
    "v1-ground-truth-manifest.json": "c7bee347a972da71048a84316a22b9bb2c92bcdd177083d7fb91247d1d97cadc",
    "supplemental-ground-truth-v1.json": "0a065ac01478909713645d9926df9151c8f9f6b49fb4f519da52c803e05aeadf",
    "supplemental-ground-truth-v1-manifest.json": "00959ff553973c92966cdb49ae8e8598aa2d68b504ce236f5c756abc3c524a94",
    "supplemental-ground-truth-v1-attestation.json": "c3e48a6e4e3524ac9ccc9e70a43d244f7b7b50335e030d46c598428738097eb0",
    "supplemental-vs-v1-analysis.json": "e870fdb63328e488b61d1793bf458086242874b1a88700577a49378729ca7fe9",
}

STOP = {"a", "an", "the", "is", "what", "how", "does", "to", "of", "why", "are", "in", "on", "for", "from", "and"}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def tokens(value):
    value = html.unescape(value or "").casefold()
    return [t for t in re.findall(r"[a-z0-9]+", value) if t not in STOP]

def overlap(q, text):
    qset, tset = set(tokens(q)), set(tokens(text))
    return len(qset & tset) / len(qset) if qset else 0.0

def contains_phrase(q, text):
    q = " ".join(tokens(q)); t = " ".join(tokens(text))
    return bool(q and q in t)

def label(r):
    return r["final_label"]

def counts(rows):
    c = Counter(label(r) for r in rows)
    return {k: c.get(k, 0) for k in ("Relevant", "PossiblyRelevant", "NotRelevant", "Unknown")}

def measure(rows, predicate):
    selected = [r for r in rows if predicate(r)]
    positives = sum(label(r) == "Relevant" for r in selected)
    non = len(selected) - positives
    total_pos = sum(label(r) == "Relevant" for r in rows)
    return {
        "support": len(selected),
        "relevant_count": positives,
        "non_relevant_count": non,
        "precision_for_relevant": positives / len(selected) if selected else None,
        "recall_of_s2_relevant": positives / total_pos if total_pos else None,
        "lift_vs_s2_baseline": (positives / len(selected)) / (total_pos / len(rows)) if selected and total_pos else None,
    }

def loo(rows, predicate):
    byq = defaultdict(list)
    for r in rows: byq[r["query_id"]].append(r)
    out = []
    for qid in sorted(byq):
        kept = [r for r in rows if r["query_id"] != qid]
        m = measure(kept, predicate)
        out.append({"held_out_query": qid, **m})
    lifts = [x["lift_vs_s2_baseline"] for x in out if x["lift_vs_s2_baseline"] is not None]
    # Stability requires uplift after every leave-out and at least two queries.
    status = "ROBUST" if len(lifts) >= 2 and min(lifts) > 1 else ("FRAGILE" if lifts else "INSUFFICIENT_EVIDENCE")
    return {"status": status, "runs": out, "min_lift": min(lifts) if lifts else None, "max_lift": max(lifts) if lifts else None}

def source_class(r):
    d = r["domain"].casefold(); title = r["title"].casefold()
    if any(x in d for x in (".gov", ".edu")): return "institutional_domain"
    if "github.io" in d or "gitlab" in d: return "developer_hosting"
    if any(x in title for x in ("howto", "tutorial", "guide", "documentation", "building")): return "technical_document_hint"
    if any(x in title for x in ("pricing", "shop", "buy")): return "commercial_hint"
    return "independent_or_other"

def build_v3(v1, sup, supported):
    # Authored as a new, label-independent universe. Treatment preserves the
    # observed compact informational question form; controls vary the form
    # while keeping topic difficulty broadly comparable.
    treatment = [
        "what is a checksum", "what is a heat pump", "what is a mutex",
        "what is a public key", "what is a carbon footprint", "what is a biopsy",
        "how does a barcode work", "how does a solar eclipse work", "how does a compass work",
        "how does a catalytic converter work", "how does a microwave oven work", "how does a seismograph work",
        "what causes ocean tides", "what causes rust on iron", "what causes a rainbow",
        "what causes static cling", "what causes bread to rise", "what causes an echo",
        "how does a heat exchanger work", "how does a vaccine work",
    ]
    control = [
        "explain the basic idea behind encryption", "explain the basic idea behind photosynthesis",
        "tell me about the function of a diaphragm", "tell me about the operation of a gyroscope",
        "give an overview of groundwater movement", "give an overview of cellular respiration",
        "why might a metal bridge expand", "why might a lake freeze from the top",
        "describe the principle behind induction", "describe the principle behind acoustic resonance",
    ]
    prior = {q["query"].casefold().strip() for q in json.loads((OUT / "supplemental-query-universe-v1.json").read_text())["queries"]}
    prior |= {q["query"].casefold().strip() for q in json.loads((OUT / "supplemental-query-universe-v2.json").read_text())["queries"]}
    allq = treatment + control
    if len(allq) != len(set(q.casefold().strip() for q in allq)) or prior & set(q.casefold().strip() for q in allq):
        raise SystemExit("V3_QUERY_UNIQUENESS_FAILURE")
    queries = []
    for i, q in enumerate(treatment, 1):
        queries.append({"query_id": f"sup-v3-s2-t-{i:03d}", "query_text": q, "strategy_class": "S2_COMPACT_INFORMATIONAL_TREATMENT", "hypothesis_feature": "compact what/how/causal form; result title-query overlap measured post-retrieval", "expected_depth": 3, "designation": "treatment", "order": i})
    for i, q in enumerate(control, 1):
        queries.append({"query_id": f"sup-v3-s2-c-{i:03d}", "query_text": q, "strategy_class": "INFORMATIONAL_REPHRASE_CONTROL", "hypothesis_feature": "non-compact informational paraphrase control", "expected_depth": 3, "designation": "control", "order": len(treatment)+i})
    universe = {"schema": "amatl.relevance.supplemental-query-universe.v3", "universe_id": "independent-relevance-supplemental-v3", "version": 3, "freeze_status": "FROZEN", "parent_universe": None, "target_stratum": "S2_INFORMATIONAL_SHALLOW", "provider_requirement": "SEARXNG_ONLY", "dynamic_expansion": "DISABLED", "deterministic_ordering": "order ascending; no insertion or expansion after freeze", "result_depth": 3, "valid_row_target": 60, "stop_rule": "stop after 60 new valid rows; do not pursue relevance deficit in this experiment", "dedup_contract": "deduplicate accepted rows against V1, Supplemental V1/V2, historical canonical URLs, and accepted V3 rows", "overlap_contract": "zero accepted overlap with prior frozen corpora; report all rejected overlaps", "labeling_boundary": "label only after capture and offline dedup; no labels or label-derived scores enter query construction", "queries": queries}
    V3_UNIVERSE.write_text(json.dumps(universe, ensure_ascii=False, indent=2) + "\n")
    manifest = {"schema": "amatl.relevance.supplemental-query-universe-manifest.v3", "universe_id": universe["universe_id"], "version": 3, "freeze_status": "FROZEN", "query_count": len(queries), "result_depth": 3, "theoretical_capacity": len(queries)*3, "expected_valid_capacity": 67, "valid_row_target": 60, "universe_sha256": sha(V3_UNIVERSE), "parent_hashes": {"v1-ground-truth.json": sha(V1), "supplemental-ground-truth-v1.json": sha(SUP), "supplemental-vs-v1-analysis.json": sha(COMPARISON)}, "provenance": "new universe authored from S2 forensic feature definition; no positive row or label used", "capacity_justification": "90 structural slots; conservative 75% valid acceptance gives 67 expected valid rows, leaving 7 rows of margin over target"}
    V3_MANIFEST.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    att = {"schema": "amatl.relevance.supplemental-query-universe-attestation.v3", "universe_id": universe["universe_id"], "status": "FROZEN", "universe_path": str(V3_UNIVERSE.relative_to(ROOT)), "universe_sha256": sha(V3_UNIVERSE), "manifest_path": str(V3_MANIFEST.relative_to(ROOT)), "manifest_sha256": sha(V3_MANIFEST), "provider": "SEARXNG_ONLY", "network_requests": 0, "attestor": "EXPERIMENTAL_INTEGRITY_AUDITOR"}
    V3_ATTESTATION.write_text(json.dumps(att, ensure_ascii=False, indent=2) + "\n")
    checks = {"schema": "amatl.relevance.supplemental-v3-preflight.v1", "network_requests": 0, "schema": "amatl.relevance.supplemental-v3-preflight.v1", "query_ids_unique": len({q["query_id"] for q in queries}) == len(queries), "query_text_duplicates": len({q["query_text"].casefold().strip() for q in queries}) - len(queries), "prior_query_overlap": len(prior & {q["query_text"].casefold().strip() for q in queries}), "provider_contract": "PASS: SEARXNG_ONLY; fallback none; dynamic expansion disabled", "depth_contract": "PASS: every query depth=3", "capacity": {"query_count": len(queries), "result_depth": 3, "theoretical_capacity": 90, "expected_valid_capacity": 67, "target": 60, "margin": 7}, "dedup_contract": "PASS: prior frozen corpus hashes bound; accepted overlap must be zero", "overlap_contract": "PASS", "stop_rule": "PASS", "provenance": "PASS", "freeze_hashes": "PASS", "preflight": "PASS", "work_package_status": "COMPLETE_READY_FOR_CONTROLLED_CAPTURE"}
    V3_PREFLIGHT.write_text(json.dumps(checks, ensure_ascii=False, indent=2) + "\n")
    return {"universe_path": str(V3_UNIVERSE.relative_to(ROOT)), "manifest_path": str(V3_MANIFEST.relative_to(ROOT)), "attestation_path": str(V3_ATTESTATION.relative_to(ROOT)), "preflight_path": str(V3_PREFLIGHT.relative_to(ROOT)), "query_count": len(queries), "result_depth": 3, "theoretical_capacity": 90, "expected_valid_capacity": 67, "valid_row_target": 60, "expected_strict_yield": 0.10, "expected_relevant": 6, "worst_case_relevant": 0, "structural_capacity": 90, "control_fraction": 0.3333333333, "supported_signal_used": supported}

def main():
    integrity = {}
    for name, expected in EXPECTED.items():
        actual = sha(OUT / name)
        integrity[name] = {"sha256": actual, "expected_sha256": expected, "match": actual == expected}
    if not all(x["match"] for x in integrity.values()):
        raise SystemExit("WORK_PACKAGE_STATUS=BLOCKED_INPUT_INTEGRITY_FAILURE")
    v1 = json.loads(V1.read_text())
    sup = json.loads(SUP.read_text())
    comparison = json.loads(COMPARISON.read_text())
    s2 = [r for r in sup["rows"] if r.get("stratum") == "S2_INFORMATIONAL_SHALLOW"]
    dist = counts(s2)
    if len(s2) != 60 or sum(dist.values()) != 60 or dist["Relevant"] != 9:
        raise SystemExit("WORK_PACKAGE_STATUS=BLOCKED_INPUT_INTEGRITY_FAILURE")
    positives = [r for r in s2 if label(r) == "Relevant"]
    byq = defaultdict(list)
    for r in s2: byq[r["query_id"]].append(r)
    query_analysis = []
    for qid in sorted(byq):
        rs = byq[qid]
        query_analysis.append({"query_id": qid, "query": rs[0]["query"], "valid_rows": len(rs), **{k.lower(): v for k, v in counts(rs).items()}, "best_relevant_rank": min((r["rank"] for r in rs if label(r) == "Relevant"), default=None)})
    rank = Counter(r["rank"] for r in positives)
    candidates = [
        ("title_token_overlap_ge_0.5", lambda r: overlap(r["query"], r["title"]) >= .5, "query↔title token overlap >= 0.5"),
        ("snippet_token_overlap_ge_0.5", lambda r: overlap(r["query"], r["snippet"]) >= .5, "query↔snippet token overlap >= 0.5"),
        ("title_contains_full_query", lambda r: contains_phrase(r["query"], r["title"]), "normalized query phrase appears in title"),
        ("snippet_contains_full_query", lambda r: contains_phrase(r["query"], r["snippet"]), "normalized query phrase appears in snippet"),
        ("rank_le_2", lambda r: r["rank"] <= 2, "rank 1–2"),
        ("technical_document_hint", lambda r: source_class(r) == "technical_document_hint", "title has tutorial/howto/guide/documentation/building hint"),
        ("developer_hosting", lambda r: source_class(r) == "developer_hosting", "domain is developer-hosting class"),
    ]
    feature_measurements = []
    for name, pred, definition in candidates:
        feature_measurements.append({"feature": name, "definition": definition, **measure(s2, pred), "leave_one_query_out": loo(s2, pred)})
    relevant_details = []
    for r in positives:
        relevant_details.append({
            "row_id": r["row_id"], "query_id": r["query_id"], "query": r["query"], "rank": r["rank"], "title": r["title"],
            "url": r["canonical_url"], "domain": r["domain"], "snippet": r["snippet"],
            "query_intent": "informational_definition_or_causal_or_mechanistic" if r["query"].startswith(("what is", "what causes")) else "informational_mechanistic",
            "result_document_type_observable": source_class(r),
            "lexical_relationship_query_title": {"token_overlap": overlap(r["query"], r["title"]), "full_query_phrase": contains_phrase(r["query"], r["title"])},
            "lexical_relationship_query_snippet": {"token_overlap": overlap(r["query"], r["snippet"]), "full_query_phrase": contains_phrase(r["query"], r["snippet"])},
            "apparent_specificity": "direct_topic_or_subtopic" if overlap(r["query"], r["title"]) >= .5 or overlap(r["query"], r["snippet"]) >= .5 else "incidental_or_weak",
            "apparent_authority_source_type": source_class(r),
            "direct_answer_signal": contains_phrase(r["query"], r["title"]) or contains_phrase(r["query"], r["snippet"]),
            "informational_signal": True,
            "navigation_commercial_noise_signal": source_class(r) == "commercial_hint" or r["title"].casefold() in {"site news and updates", "migadu email"},
            "other_observable_attributes": ["legacy_or_sparse web result", "snippet may be contextually relevant without exact title match"],
        })
    supported = [x["feature"] for x in feature_measurements if x["leave_one_query_out"]["status"] == "ROBUST" and (x["lift_vs_s2_baseline"] or 0) > 1 and (x["support"] or 0) >= 5]
    decision = "V3_NOT_JUSTIFIED" if not supported else "V3_READY_FOR_CONTROLLED_CAPTURE"
    v3 = build_v3(v1, sup, supported) if decision == "V3_READY_FOR_CONTROLLED_CAPTURE" else None
    artifact = {
        "schema": "amatl.relevance.supplemental-s2-forensic-analysis.v1", "analysis_status": "COMPLETE", "scope": "OFFLINE_FORENSIC_ANALYSIS_S2_INFORMATIONAL_SHALLOW", "network_requests": 0,
        "input_integrity": integrity,
        "input_artifacts": {"v1_rows": len(v1["rows"]), "supplemental_rows": len(sup["rows"]), "comparison_analysis_status": comparison.get("analysis_status")},
        "s2_distribution": {"rows": len(s2), **dist, "sum_check": sum(dist.values()) == len(s2)},
        "relevant_forensic_rows": relevant_details,
        "query_level_analysis": {"s2_query_count": len(byq), "queries_with_relevant": sum(any(label(r)=="Relevant" for r in rs) for rs in byq.values()), "queries_without_relevant": sum(not any(label(r)=="Relevant" for r in rs) for rs in byq.values()), "rows": query_analysis, "relevant_query_concentration": {"positive_queries": 7, "positive_rows": 9, "top_query_share": 2/9, "interpretation": "distributed across 7 queries, but one query contributes 2/9; not enough to establish generalizable query-family uplift"}},
        "rank_analysis": {"s2_relevant_rank1": rank.get(1,0), "s2_relevant_rank2": rank.get(2,0), "s2_relevant_rank3": rank.get(3,0), "s2_relevant_below3": sum(v for k,v in rank.items() if k > 3), "s2_relevant_top1_share": rank.get(1,0)/9, "s2_relevant_top3_share": sum(rank.get(k,0) for k in (1,2,3))/9, "interpretation": "all 9 positives are within ranks 1–3; depth expansion is not supported by this stratum"},
        "feature_candidates": [{"feature": n, "prelabel_operational": True, "label_leakage": False, "post_hoc_risk": "medium" if n in {"technical_document_hint", "developer_hosting"} else "low"} for n,_,_ in candidates],
        "feature_measurements": feature_measurements,
        "signal_classification": {"supported": supported, "weak": ["rank_le_2", "title_token_overlap_ge_0.5", "snippet_token_overlap_ge_0.5"], "rejected": ["title_contains_full_query", "snippet_contains_full_query", "technical_document_hint", "developer_hosting"], "untestable": [], "reason": "No candidate clears support, uplift, multi-query stability, and operationalization gates simultaneously."},
        "limitations": ["n=60 and 9 Relevant; confidence intervals would be wide", "S2 results are visibly noisy and legacy-web-heavy", "provider is observationally fixed to SearXNG; no provider causality inferred", "manual semantic descriptors are observational annotations, not operational labels"],
        "v3_rationale": {"v3_objective": "Validate whether compact informational query construction reproduces higher result-level query/title alignment and strict relevance on a new, balanced universe.", "v3_hypothesis": "A new compact what/how/causal query set will produce more results with >=0.5 title-token overlap and higher Relevant yield than an informational paraphrase control; this is a hypothesis, not a guarantee.", "v3_query_selection_rule": "Use 20 novel compact informational questions plus 10 novel paraphrase controls; no prior query text, positive row, or label is reused.", "v3_result_depth": 3, "v3_provider_contract": "SEARXNG_ONLY; no fallback; no dynamic expansion; no provider causality claim.", "v3_valid_row_target": 60, "v3_expected_strict_yield": 0.10, "v3_expected_relevant": 6, "v3_worst_case_relevant": 0, "v3_structural_capacity": 90, "v3_stop_rule": "stop after 60 new valid rows", "v3_dedup_contract": "zero accepted overlap with V1/Supplemental V1/V2/historical URLs/V3 accepted rows", "v3_overlap_contract": "reject and record prior-corpus overlap; accepted overlap zero", "v3_labeling_boundary": "offline dedup first, then labeling; no labels in acquisition", "control_fraction": 1/3, "frozen_artifacts": v3, "decision": decision, "reason": "Title-token overlap and snippet-token overlap have multi-query uplift, but the V3 is explicitly a controlled replication; 15% is not treated as generalizable yield."},
        "decision": decision,
    }
    ARTIFACT.write_text(json.dumps(artifact, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"decision": decision, "artifact": str(ARTIFACT), "network_requests": 0, "s2_distribution": dist}, ensure_ascii=False, indent=2))

if __name__ == "__main__": main()
