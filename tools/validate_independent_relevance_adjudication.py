#!/usr/bin/env python3
"""Validate independent labels and prepare a human-only adjudication packet.

This tool deliberately has no dependency on AMATL predictions, ranking, scores,
or historical labels.  It can only construct final ground truth after a complete
human adjudication packet is supplied.
"""
import argparse
import hashlib
import json
import math
from collections import Counter
from pathlib import Path

LABELS = ("Relevant", "PossiblyRelevant", "NotRelevant", "Unknown")
ORIGINAL_FIELDS = (
    "row_id", "query_id", "query", "provider_or_source", "original_url",
    "canonical_url", "domain", "title", "snippet", "rank", "result_status",
    "published_at", "acquired_at", "acquisition_run_id", "raw_response_path",
    "raw_response_sha256",
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def validation(rows, reference):
    ids = [row.get("row_id") for row in rows]
    ref_ids = [row.get("row_id") for row in reference]
    id_set, ref_set = set(ids), set(ref_ids)
    invalid = [row.get("row_id") for row in rows if row.get("label") not in LABELS]
    empty = [row.get("row_id") for row in rows if not row.get("label")]
    ref_by_id = {row["row_id"]: row for row in reference}
    mismatch = []
    for row in rows:
        ref = ref_by_id.get(row.get("row_id"))
        if ref is None or any(row.get(field) != ref.get(field) for field in ORIGINAL_FIELDS):
            mismatch.append(row.get("row_id"))
    return {
        "TOTAL_ROWS": len(rows), "VALID_ROWS": len(rows) - len(invalid) - len(empty),
        "MISSING_ROWS": sorted(ref_set - id_set), "EXTRA_ROWS": sorted(id_set - ref_set),
        "DUPLICATE_ROW_IDS": sorted(row_id for row_id, count in Counter(ids).items() if count > 1),
        "INVALID_LABELS": invalid, "EMPTY_LABELS": empty,
        "ORIGINAL_FIELD_MISMATCH": sorted(mismatch),
        "ROW_ID_SET_HASH": canonical_hash(sorted(id_set)),
    }


def valid(result):
    return (result["TOTAL_ROWS"] == 368 and result["VALID_ROWS"] == 368 and
            not any(result[key] for key in ("MISSING_ROWS", "EXTRA_ROWS", "DUPLICATE_ROW_IDS", "INVALID_LABELS", "EMPTY_LABELS", "ORIGINAL_FIELD_MISMATCH")))


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def main():
    root = Path(__file__).resolve().parents[1]
    default = root / "docs/evaluation/independent-relevance/new-corpus-v1/prelabel"
    parser = argparse.ArgumentParser()
    parser.add_argument("--prelabel", type=Path, default=default / "prelabel-snapshot.json")
    parser.add_argument("--labeler-a", type=Path, default=default / "annotation-packet-A-labeled.json")
    parser.add_argument("--labeler-b", type=Path, default=default / "annotation-packet-B-labeled.json")
    parser.add_argument("--output-dir", type=Path, default=default.parent / "adjudication")
    parser.add_argument("--adjudication", type=Path)
    args = parser.parse_args()

    prelabel, a_doc, b_doc = load(args.prelabel), load(args.labeler_a), load(args.labeler_b)
    reference, a_rows, b_rows = prelabel["rows"], a_doc["rows"], b_doc["rows"]
    a_validation, b_validation = validation(a_rows, reference), validation(b_rows, reference)
    report = {
        "schema": "amatl.relevance.agreement-report.v1",
        "PRELABEL_HASH": digest(args.prelabel), "RAW_ACQUISITION_HASH": prelabel.get("raw_acquisition_hash"),
        "LABELER_A_HASH": digest(args.labeler_a), "LABELER_B_HASH": digest(args.labeler_b),
        "LABELER_A_VALIDATION": "PASS" if valid(a_validation) else "FAIL",
        "LABELER_B_VALIDATION": "PASS" if valid(b_validation) else "FAIL",
        "labeler_a_validation": a_validation, "labeler_b_validation": b_validation,
    }
    if not (valid(a_validation) and valid(b_validation)):
        report["WORK_PACKAGE_STATUS"] = "BLOCKED_LABEL_INPUT_INVALID"
        args.output_dir.mkdir(parents=True, exist_ok=True)
        write_json(args.output_dir / "agreement-report.json", report)
        raise SystemExit("invalid label input")

    a_by_id, b_by_id = ({row["row_id"]: row for row in rows} for rows in (a_rows, b_rows))
    matrix = {a: {b: 0 for b in LABELS} for a in LABELS}
    disagreements, agreements = [], 0
    for row in reference:
        row_id, la, lb = row["row_id"], a_by_id[row["row_id"]]["label"], b_by_id[row["row_id"]]["label"]
        matrix[la][lb] += 1
        if la == lb:
            agreements += 1
        else:
            severity = "UNKNOWN_CONFLICT" if "Unknown" in (la, lb) else ("EXTREME" if {la, lb} == {"Relevant", "NotRelevant"} else "MINOR_BOUNDARY")
            disagreements.append({field: row[field] for field in ORIGINAL_FIELDS} | {"label_a": la, "label_b": lb, "adjudicated_label": "", "adjudication_note": "", "disagreement_severity": severity})
    total = len(reference)
    observed = agreements / total
    a_counts, b_counts = Counter(row["label"] for row in a_rows), Counter(row["label"] for row in b_rows)
    expected = sum(a_counts[label] * b_counts[label] for label in LABELS) / total**2
    kappa = (observed - expected) / (1 - expected) if expected != 1 else 1.0
    per_class = {label: {"agreements": matrix[label][label], "labeler_a_total": a_counts[label], "labeler_b_total": b_counts[label]} for label in LABELS}
    packet = {"schema": "amatl.relevance.adjudication-packet.v1", "prelabel_snapshot_hash": digest(args.prelabel), "rows": disagreements}
    args.output_dir.mkdir(parents=True, exist_ok=True)
    packet_path = args.output_dir / "adjudication-packet.json"
    # The packet is a human-owned record once it exists.  Regenerating it on
    # every validation run would silently discard adjudications, so only create
    # it when absent.  An existing packet must still be exactly the immutable
    # disagreement queue reconstructed from the two labeler packets.
    if packet_path.exists():
        existing_packet = load(packet_path)
        expected_rows = {row["row_id"]: row for row in disagreements}
        existing_rows = existing_packet.get("rows", [])
        packet_is_intact = (
            existing_packet.get("schema") == packet["schema"]
            and existing_packet.get("prelabel_snapshot_hash") == packet["prelabel_snapshot_hash"]
            and len(existing_rows) == len(expected_rows)
            and {row.get("row_id") for row in existing_rows} == set(expected_rows)
            and all(
                all(row.get(field) == expected_rows[row["row_id"]].get(field)
                    for field in ORIGINAL_FIELDS + ("label_a", "label_b", "disagreement_severity"))
                for row in existing_rows if row.get("row_id") in expected_rows
            )
        )
        if not packet_is_intact:
            raise ValueError("existing adjudication packet differs from the frozen disagreement queue")
        packet = existing_packet
    else:
        write_json(packet_path, packet)
    by_severity = {name: [row["row_id"] for row in disagreements if row["disagreement_severity"] == name] for name in ("MINOR_BOUNDARY", "EXTREME", "UNKNOWN_CONFLICT")}
    report.update({
        "TOTAL_COMPARABLE_ROWS": total, "AGREEMENTS": agreements, "DISAGREEMENTS": len(disagreements),
        "PERCENT_AGREEMENT": round(observed * 100, 6), "COHEN_KAPPA": round(kappa, 9),
        "PER_CLASS_AGREEMENT": per_class, "DISAGREEMENT_MATRIX": matrix,
        "MINOR_BOUNDARY_COUNT": len(by_severity["MINOR_BOUNDARY"]), "EXTREME_COUNT": len(by_severity["EXTREME"]),
        "UNKNOWN_CONFLICT_COUNT": len(by_severity["UNKNOWN_CONFLICT"]), "DISAGREEMENT_ROW_IDS": by_severity,
        "ADJUDICATION_PACKET_ROWS": len(disagreements), "ADJUDICATION_PACKET_HASH": digest(packet_path),
        "ADJUDICATION_STATUS": "NOT_STARTED", "SELECTION_STATUS": "NOT_CREATED", "BLIND_HOLDOUT_STATUS": "NOT_CREATED",
    })
    # A pending adjudication prevents exact final deficits, but it can still be
    # mathematically impossible for this fixed corpus to meet a quota.  Record
    # that fact without assigning any disputed label.
    agreed_counts = Counter(row["label"] for row in a_rows if row["label"] == b_by_id[row["row_id"]]["label"])
    relevant_upper_bound = agreed_counts["Relevant"] + sum("Relevant" in (row["label_a"], row["label_b"]) for row in disagreements)
    if relevant_upper_bound < 100:
        minimum_deficit = 100 - relevant_upper_bound
        observed_relevant = max(a_counts["Relevant"], b_counts["Relevant"])
        raw_baseline = math.ceil(minimum_deficit * total / observed_relevant)
        spec = {
            "schema": "amatl.relevance.supplemental-acquisition-spec.v1",
            "status": "PRELIMINARY_PENDING_ADJUDICATION",
            "basis": "Relevant quota is impossible even if every Relevant disagreement is adjudicated Relevant.",
            "TARGET_CLASS": "Relevant", "MAXIMUM_CURRENT_FINAL_LABELS": relevant_upper_bound,
            "CLASS_DEFICIT": minimum_deficit, "MINIMUM_NEW_FINAL_LABELS_REQUIRED": minimum_deficit,
            "observed_first_campaign_yield": {"relevant_labels": observed_relevant, "total_rows": total},
            "RAW_ACQUISITION_TARGET_BEFORE_MARGIN": raw_baseline,
            "acquisition_margin": 0.25, "RAW_ACQUISITION_TARGET": math.ceil(raw_baseline * 1.25),
            "authorization": "NOT_AUTHORIZED_BY_THIS_WORK_PACKAGE",
        }
        spec_path = args.output_dir / "supplemental-acquisition-spec.json"
        write_json(spec_path, spec)
        report["SUPPLEMENTAL_ACQUISITION_SPEC_HASH"] = digest(spec_path)
    adjudication_path = args.adjudication or packet_path
    if not adjudication_path.exists():
        report["WORK_PACKAGE_STATUS"] = "BLOCKED_WAITING_FOR_ADJUDICATION"
    else:
        adjudication = load(adjudication_path)
        adjudication_rows = adjudication.get("rows", [])
        adjudication_ids = [row.get("row_id") for row in adjudication_rows]
        expected_ids = [row["row_id"] for row in disagreements]
        complete = (Counter(adjudication_ids) == Counter(expected_ids) and all(row.get("adjudicated_label") in LABELS for row in adjudication_rows))
        report["ADJUDICATION_RESULT_HASH"] = digest(adjudication_path)
        if not complete:
            empty = all(not row.get("adjudicated_label") for row in adjudication_rows)
            report["ADJUDICATION_STATUS"] = "WAITING_FOR_HUMAN" if empty else "INVALID"
            report["WORK_PACKAGE_STATUS"] = ("BLOCKED_WAITING_FOR_ADJUDICATION" if empty
                                             else "BLOCKED_ADJUDICATION_INVALID")
        else:
            final = []
            adjudicated = {row["row_id"]: row["adjudicated_label"] for row in adjudication_rows}
            for row in reference:
                final.append({field: row[field] for field in ORIGINAL_FIELDS} | {"final_label": adjudicated.get(row["row_id"], a_by_id[row["row_id"]]["label"])})
            final_doc = {"schema": "amatl.relevance.final-ground-truth.v1", "prelabel_snapshot_hash": digest(args.prelabel), "rows": final}
            final_path = args.output_dir / "final-ground-truth.json"; write_json(final_path, final_doc)
            counts = Counter(row["final_label"] for row in final)
            deficits = {label: max(0, 100 - counts[label]) for label in ("Relevant", "PossiblyRelevant", "NotRelevant")}
            report.update({"ADJUDICATION_STATUS": "COMPLETE", "FINAL_GROUND_TRUTH_HASH": digest(final_path), "FINAL_LABEL_DISTRIBUTION": dict(counts), "CLASS_DEFICITS": deficits, "CORPUS_BALANCE_STATUS": "SUFFICIENT" if not any(deficits.values()) else "INSUFFICIENT", "WORK_PACKAGE_STATUS": "COMPLETE"})
    write_json(args.output_dir / "agreement-report.json", report)


if __name__ == "__main__":
    main()
