#!/usr/bin/env python3
"""Freeze the adjudicated V1 corpus and its pre-label supplemental pilot plan.

This command never acquires or labels data.  Its only authority is to turn the
two independent label packets plus the human adjudication record into a
traceable, known (therefore non-blind) ground truth.
"""
import hashlib
import json
import argparse
import subprocess
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1"
PRELABEL = BASE / "prelabel"
OUT = BASE / "adjudication"
LABELS = ("Relevant", "PossiblyRelevant", "NotRelevant", "Unknown")
ORIGINAL_FIELDS = (
    "row_id", "query_id", "query", "provider_or_source", "original_url",
    "canonical_url", "domain", "title", "snippet", "rank", "result_status",
    "published_at", "acquired_at", "acquisition_run_id", "raw_response_path",
    "raw_response_sha256",
)


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical_sha(value):
    raw = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def source_commit(manifest_path, explicit=None):
    existing = load(manifest_path) if Path(manifest_path).exists() else {}
    return existing.get("SOURCE_COMMIT") or explicit or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()


def query_type(query):
    """A declared, text-only rule available before any future label exists."""
    if query.startswith("how to "):
        return "procedural"
    if query.startswith("what is ") or query.startswith("what causes ") or query.startswith("how does "):
        return "informational"
    return "other"


def rates(rows, field):
    grouped = defaultdict(Counter)
    for row in rows:
        key = row[field]
        if isinstance(key, list):
            key = "+".join(key)
        grouped[str(key)][row["final_label"]] += 1
    return {
        key: {"total": sum(c.values()), "Relevant": c["Relevant"],
              "PossiblyRelevant": c["PossiblyRelevant"], "NotRelevant": c["NotRelevant"],
              "Unknown": c["Unknown"], "relevant_rate": c["Relevant"] / sum(c.values()),
              "possibly_relevant_rate": c["PossiblyRelevant"] / sum(c.values()),
              "not_relevant_rate": c["NotRelevant"] / sum(c.values())}
        for key, c in sorted(grouped.items(), key=lambda item: int(item[0]) if item[0].isdigit() else item[0])
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-commit", help="source commit for an initial freeze")
    args = parser.parse_args()
    # Re-run the pre-existing input/adjudication validator first.  It preserves
    # the human-owned packet and rejects an altered disagreement queue.
    subprocess.run(["python3", str(ROOT / "tools/validate_independent_relevance_adjudication.py")], cwd=ROOT, check=True)
    prelabel = load(PRELABEL / "prelabel-snapshot.json")
    a_doc, b_doc = (load(PRELABEL / name) for name in ("annotation-packet-A-labeled.json", "annotation-packet-B-labeled.json"))
    adjudication = load(OUT / "adjudication-packet.json")
    reference, a_rows, b_rows = prelabel["rows"], a_doc["rows"], b_doc["rows"]
    a_by_id = {row["row_id"]: row for row in a_rows}
    b_by_id = {row["row_id"]: row for row in b_rows}
    adjudicated = {row.get("row_id"): row for row in adjudication.get("rows", [])}
    disagreements = {row_id for row_id in a_by_id if a_by_id[row_id]["label"] != b_by_id[row_id]["label"]}
    if len(reference) != 368 or len({row["row_id"] for row in reference}) != 368:
        raise ValueError("prelabel must contain exactly 368 unique rows")
    if len(disagreements) != 32 or set(adjudicated) != disagreements:
        raise ValueError("adjudication row IDs are not exactly the 32 disagreements")

    rows = []
    for source in reference:
        row_id = source["row_id"]
        if any(a_by_id[row_id].get(f) != source.get(f) or b_by_id[row_id].get(f) != source.get(f) for f in ORIGINAL_FIELDS):
            raise ValueError(f"label packet source-field integrity failed: {row_id}")
        la, lb = a_by_id[row_id]["label"], b_by_id[row_id]["label"]
        if la not in LABELS or lb not in LABELS:
            raise ValueError(f"invalid independent label: {row_id}")
        if la == lb:
            final_label, source_kind = la, "AGREEMENT"
        else:
            adj = adjudicated[row_id]
            if any(adj.get(f) != source.get(f) for f in ORIGINAL_FIELDS):
                raise ValueError(f"adjudication source-field integrity failed: {row_id}")
            if adj.get("label_a") != la or adj.get("label_b") != lb or adj.get("adjudicated_label") not in LABELS:
                raise ValueError(f"invalid human adjudication: {row_id}")
            final_label, source_kind = adj["adjudicated_label"], "ADJUDICATION"
        rows.append({field: source[field] for field in ORIGINAL_FIELDS} | {
            "label_a": la, "label_b": lb, "final_label": final_label, "final_label_source": source_kind,
        })
    if sum(row["final_label_source"] == "AGREEMENT" for row in rows) != 336:
        raise ValueError("agreement provenance count is not 336")
    if sum(row["final_label_source"] == "ADJUDICATION" for row in rows) != 32:
        raise ValueError("adjudication provenance count is not 32")

    ground = {
        "schema": "amatl.relevance.ground-truth.v1", "ground_truth_id": "independent-relevance-ground-truth-v1",
        "role": "KNOWN_INDEPENDENT_GROUND_TRUTH", "prelabel_snapshot_hash": sha(PRELABEL / "prelabel-snapshot.json"),
        "rows": rows,
    }
    ground_path = OUT / "v1-ground-truth.json"
    write(ground_path, ground)
    counts = Counter(row["final_label"] for row in rows)
    rank_rates = rates(rows, "rank")
    analysis = {
        "schema": "amatl.relevance.v1-yield-analysis.v1", "ground_truth_hash": sha(ground_path),
        "total_rows": len(rows), "class_distribution": dict(counts),
        "global_relevant_yield": counts["Relevant"] / len(rows), "by_rank": rank_rates,
        "by_query": rates(rows, "query_id"), "by_provider": rates(rows, "provider_or_source"),
        "by_query_type": rates([{**r, "query_type": query_type(r["query"])} for r in rows], "query_type"),
        "concentration": {
            "relevant_in_ranks_1_to_3": sum(r["final_label"] == "Relevant" and r["rank"] <= 3 for r in rows),
            "possibly_relevant_in_ranks_1_to_3": sum(r["final_label"] == "PossiblyRelevant" and r["rank"] <= 3 for r in rows),
            "relevant_query_ids": sorted({r["query_id"] for r in rows if r["final_label"] == "Relevant"}),
        },
        "finding": "Seven of nine Relevant rows occur at ranks 1-3; the other two occur at ranks 6 and 7, and none occur below rank 7. PossiblyRelevant falls from 22/102 at ranks 1-3 to 29/266 afterwards. With one provider only, provider causality is not identifiable. The evidence supports a combination of shallow-rank concentration and query mix, not a causal claim about provider behavior.",
    }
    analysis_path = OUT / "v1-yield-analysis.json"
    write(analysis_path, analysis)
    deficits = {label: max(0, 100 - counts[label]) for label in ("Relevant", "PossiblyRelevant", "NotRelevant")}
    # Pilot strata are solely rules over future, pre-label metadata.  Sampling
    # rank bands together avoids encoding label as position.
    strata = [
        {"STRATUM_ID": "S1_PROCEDURAL_SHALLOW", "RATIONALE": "V1 positives concentrate in ranks 1-3; procedural queries retain natural semantic variation.", "QUERY_SELECTION_RULE": "query begins 'how to '", "RESULT_SELECTION_RULE": "rank in 1..3; retain all visible results selected by deterministic rotating query order", "RAW_TARGET": 60, "EXPECTED_PURPOSE": "measure shallow procedural yield"},
        {"STRATUM_ID": "S2_INFORMATIONAL_SHALLOW", "RATIONALE": "Tests whether shallow concentration generalizes beyond procedural wording.", "QUERY_SELECTION_RULE": "query begins 'what is ', 'what causes ', or 'how does '", "RESULT_SELECTION_RULE": "rank in 1..3; retain all visible results selected by deterministic rotating query order", "RAW_TARGET": 60, "EXPECTED_PURPOSE": "measure shallow informational yield"},
        {"STRATUM_ID": "S3_MIXED_DEPTH_CONTROL", "RATIONALE": "Preserves boundary and negative examples so position cannot trivially encode class.", "QUERY_SELECTION_RULE": "one half procedural and one half informational using deterministic query-id order", "RESULT_SELECTION_RULE": "rank in 4..8; retain all visible results selected by deterministic rotating query order", "RAW_TARGET": 60, "EXPECTED_PURPOSE": "estimate depth contrast and maintain semantic discrimination"},
    ]
    plan = {
        "schema": "amatl.relevance.supplemental-pilot-design.v1", "status": "READY", "pilot_required": "YES",
        "pilot_raw_target": 180, "pilot_justification": "The materially changed, rank-stratified strategy has no pre-label yield evidence. 60 observations per stratum gives a 95% zero-event upper bound of about 4.9% per stratum and produces a useful directional estimate before any thousand-row commitment; it is intentionally below the 200-600 default range.",
        "sampling_seed": "independent-relevance-supplemental-pilot-v1", "labeling_rule": "Independent A/B labeling only; strata never determine labels.",
        "execution_status": "BLOCKED", "blocker": "supplemental-acquisition-spec.json records authorization NOT_AUTHORIZED_BY_THIS_WORK_PACKAGE", "next_single_action": "Obtain explicit project authorization for the 180-row pre-label pilot capture.",
        "strata": strata,
    }
    plan_path = OUT / "supplemental-pilot-design.json"
    write(plan_path, plan)
    manifest_path = OUT / "v1-ground-truth-manifest.json"
    commit = source_commit(manifest_path, args.source_commit)
    manifest = {
        "schema": "amatl.relevance.ground-truth-manifest.v1", "GROUND_TRUTH_ID": ground["ground_truth_id"], "V1_ROLE": ground["role"],
        "TOTAL_ROWS": len(rows), "CLASS_DISTRIBUTION": dict(counts), "SOURCE_PRELABEL_HASH": sha(PRELABEL / "prelabel-snapshot.json"),
        "LABELER_A_HASH": sha(PRELABEL / "annotation-packet-A-labeled.json"), "LABELER_B_HASH": sha(PRELABEL / "annotation-packet-B-labeled.json"),
        "AGREEMENT_REPORT_HASH": sha(OUT / "agreement-report.json"), "ADJUDICATION_PACKET_HASH": sha(OUT / "adjudication-packet.json"),
        "GROUND_TRUTH_HASH": sha(ground_path), "GROUND_TRUTH_CANONICAL_HASH": canonical_sha(ground), "YIELD_ANALYSIS_HASH": sha(analysis_path),
        "SUPPLEMENTAL_PILOT_DESIGN_HASH": sha(plan_path), "CREATED_AT": "2026-09-02T00:00:00Z", "PROTOCOL_VERSION": "v1", "SOURCE_COMMIT": commit,
        "hash_scope": "The canonical ground-truth document includes every original field, A/B labels, final label, and final-label provenance.",
    }
    write(manifest_path, manifest)
    print(json.dumps({"V1_GROUND_TRUTH_HASH": manifest["GROUND_TRUTH_HASH"], "distribution": dict(counts), "deficits": deficits, "PILOT_EXECUTION_STATUS": "BLOCKED"}, indent=2))


if __name__ == "__main__":
    main()
