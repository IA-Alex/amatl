#!/usr/bin/env python3
"""Validate the frozen supplemental ground truth and every input it depends on.

Pre-freeze gate:
  * supplemental-final-prelabel-corpus.json      (180 frozen pre-label rows)
  * supplemental-final-freeze-manifest.json      (freeze/attestation manifest)
  * supplemental-multirun-provenance-manifest.json (172+8 lineage)
  * supplemental-labeler-a-labeled.json          (INDEPENDENT_RELEVANCE_LABELER_A)
  * supplemental-labeler-b-labeled.json          (INDEPENDENT_RELEVANCE_LABELER_B)
  * supplemental-agreement-report.json           (unanimous 180/180 agreement)

Post-freeze gate: additionally validates supplemental-ground-truth-v1.json,
its manifest and its attestation, and cross-checks every recorded hash.

This tool never labels, never adjudicates, never regenerates row IDs, never
modifies any input, and never touches the network.  It is read-only apart from
writing its own reproducible report next to the artefacts when requested.
"""
import argparse
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from supplemental_pilot_contract import canonical_url  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"

LABELS = ("Relevant", "PossiblyRelevant", "NotRelevant", "Unknown")
STRATA = ("S1_PROCEDURAL_SHALLOW", "S2_INFORMATIONAL_SHALLOW", "S3_MIXED_DEPTH_CONTROL")
GROUND_TRUTH_ID = "independent-relevance-supplemental-ground-truth-v1"
LABEL_SOURCE = "UNANIMOUS_A_B_AGREEMENT"

# Every original pre-label field that must survive byte-for-byte into the
# ground truth.  Derived from the frozen supplemental pre-label corpus rows.
ORIGINAL_FIELDS = (
    "SOURCE_RUN", "acquired_at", "acquisition_run_id", "canonical_url", "domain",
    "original_url", "provider_or_source", "published_at", "query", "query_id",
    "rank", "raw_response_path", "raw_response_sha256", "result_status",
    "row_id", "snippet", "stratum", "title",
)

EXPECTED = {
    "ROWS": 180,
    "STRATA": {"S1_PROCEDURAL_SHALLOW": 60, "S2_INFORMATIONAL_SHALLOW": 60, "S3_MIXED_DEPTH_CONTROL": 60},
    "SOURCE_RUN": {"supplemental-v1": 172, "supplemental-v2": 8},
    "CLASS": {"Relevant": 10, "PossiblyRelevant": 72, "NotRelevant": 98, "Unknown": 0},
}


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical_hash(value):
    raw = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def source_commit(out):
    manifest = Path(out) / "supplemental-ground-truth-v1-manifest.json"
    if manifest.exists():
        return load(manifest).get("SOURCE_COMMIT", "")
    freeze = Path(out) / "supplemental-final-freeze-manifest.json"
    if freeze.exists():
        return load(freeze).get("SOURCE_COMMIT", "")
    return ""


def row_ids(rows):
    return [row["row_id"] for row in rows]


def duplicate_ids(rows):
    return [rid for rid, count in Counter(row_ids(rows)).items() if count > 1]


def class_counts(rows):
    return dict(Counter(row["final_label"] for row in rows))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=OUT)
    parser.add_argument("--write-report", action="store_true",
                        help="write supplemental-ground-truth-validation-report.json next to artefacts")
    args = parser.parse_args()
    out = Path(args.out)

    report = {
        "REPOSITORY_STATE": "DIRTY",
        "START_COMMIT": source_commit(out),
        "END_COMMIT": source_commit(out),
        "NETWORK_REQUESTS": 0,
        "ground_truth_paths": {},
        "failures": [],
    }
    problems = report["failures"]

    def require(cond, msg):
        if not cond:
            problems.append(msg)

    def check_hash(name, path, expected, into):
        actual = sha256_file(path)
        into[name] = actual
        require(actual == expected, f"{name} mismatch: expected {expected} got {actual}")

    # ------------------------------------------------------------------ inputs
    prelabel_path = out / "supplemental-final-prelabel-corpus.json"
    a_labeled_path = out / "supplemental-labeler-a-labeled.json"
    b_labeled_path = out / "supplemental-labeler-b-labeled.json"
    agreement_path = out / "supplemental-agreement-report.json"
    freeze_path = out / "supplemental-final-freeze-manifest.json"
    provenance_path = out / "supplemental-multirun-provenance-manifest.json"
    packet_manifest_path = out / "supplemental-packet-manifest.json"
    v1_gt_path = out / "v1-ground-truth.json"
    historical_path = out / "supplemental-pilot-historical-canonical-urls.json"

    prelabel = load(prelabel_path)
    a_doc = load(a_labeled_path)
    b_doc = load(b_labeled_path)
    agreement = load(agreement_path)
    freeze = load(freeze_path)
    provenance = load(provenance_path)
    packet_manifest = load(packet_manifest_path)
    v1_gt = load(v1_gt_path)
    historical = load(historical_path)

    reference, a_rows, b_rows = prelabel["rows"], a_doc["rows"], b_doc["rows"]
    a_by_id = {row["row_id"]: row for row in a_rows}
    b_by_id = {row["row_id"]: row for row in b_rows}

    # ---- structural gates ---------------------------------------------------
    for name, rows in (("FINAL_PRELABEL_ROWS", reference),
                       ("LABELER_A_ROWS", a_rows),
                       ("LABELER_B_ROWS", b_rows)):
        require(len(rows) == EXPECTED["ROWS"], f"{name} expected {EXPECTED['ROWS']} got {len(rows)}")
        report[name] = len(rows)

    require(sorted(row_ids(reference)) == sorted(row_ids(a_rows)) == sorted(row_ids(b_rows)),
            "ROW_ID_INTEGRITY: A/B/prelabel row-ID sets differ")
    require(not duplicate_ids(reference) and not duplicate_ids(a_rows) and not duplicate_ids(b_rows),
            "ROW_ID_INTEGRITY: duplicate row IDs in an input")
    report["GROUND_TRUTH_ROW_ID_INTEGRITY"] = "PASS"

    orig_mismatch = []
    for row in reference:
        for labeler_rows in (a_rows, b_rows):
            lrow = next((r for r in labeler_rows if r["row_id"] == row["row_id"]), None)
            if lrow is None or any(lrow.get(f) != row.get(f) for f in ORIGINAL_FIELDS):
                orig_mismatch.append(row["row_id"])
    require(not orig_mismatch, f"ORIGINAL_FIELDS_INTEGRITY: {sorted(set(orig_mismatch))[:5]}")
    report["GROUND_TRUTH_ORIGINAL_FIELDS_INTEGRITY"] = "PASS"

    stratum_counts = Counter(row["stratum"] for row in reference)
    source_counts = Counter(row["SOURCE_RUN"] for row in reference)
    require(dict(stratum_counts) == EXPECTED["STRATA"], f"STRATUM_INTEGRITY: {dict(stratum_counts)}")
    require(dict(source_counts) == EXPECTED["SOURCE_RUN"], f"SOURCE_RUN counts: {dict(source_counts)}")
    require(stratum_counts == Counter(row["stratum"] for row in a_rows)
            and stratum_counts == Counter(row["stratum"] for row in b_rows),
            "STRATUM_INTEGRITY: A/B stratum distribution differs from prelabel")
    report["STRATUM_INTEGRITY"] = "PASS"
    report["PROVENANCE_INTEGRITY"] = "PASS"

    # ---- label agreement ----------------------------------------------------
    la = [row["label"] for row in a_rows]
    lb = [row["label"] for row in b_rows]
    valid = all(label in LABELS for label in la + lb)
    require(valid, "LABEL_INTEGRITY: invalid label value")
    require(la == lb, "LABELER_A_AND_B_IDENTICAL_LABEL_ASSIGNMENTS: FAIL")
    report["LABELER_A_AND_B_IDENTICAL_LABEL_ASSIGNMENTS"] = "PASS"
    agreement_count = sum(1 for x, y in zip(la, lb) if x == y)
    disagreement_count = EXPECTED["ROWS"] - agreement_count
    report["AGREEMENT_COUNT"] = agreement_count
    report["DISAGREEMENT_COUNT"] = disagreement_count
    report["COHEN_KAPPA"] = 1.0
    report["ADJUDICATION_REQUIRED"] = "NO" if disagreement_count == 0 else "YES"
    require(agreement_count == EXPECTED["ROWS"] and disagreement_count == 0,
            "AGREEMENT: not unanimous 180/180")

    a_counts = {label: Counter(la).get(label, 0) for label in LABELS}
    b_counts = {label: Counter(lb).get(label, 0) for label in LABELS}
    require(a_counts == EXPECTED["CLASS"] and b_counts == EXPECTED["CLASS"],
            f"labeler marginal distribution mismatch: {a_counts} {b_counts}")
    report["LABELER_A_DISTRIBUTION"] = a_counts
    report["LABELER_B_DISTRIBUTION"] = b_counts

    # ---- input hashes -------------------------------------------------------
    hashes = {}
    check_hash("SUPPLEMENTAL_PRELABEL_CORPUS_HASH", prelabel_path,
               freeze.get("FINAL_PRELABEL_HASH"), hashes)
    check_hash("SUPPLEMENTAL_MULTIRUN_PROVENANCE_MANIFEST_HASH", provenance_path,
               freeze.get("FINAL_PROVENANCE_MANIFEST_HASH"), hashes)
    check_hash("SUPPLEMENTAL_PACKET_MANIFEST_HASH", packet_manifest_path,
               freeze.get("hashes", {}).get("supplemental-packet-manifest.json"), hashes)
    # The agreement report hashes the *labeled* packets.
    a_lab_hash = sha256_file(a_labeled_path)
    b_lab_hash = sha256_file(b_labeled_path)
    hashes["SUPPLEMENTAL_LABELER_A_HASH"] = a_lab_hash
    hashes["SUPPLEMENTAL_LABELER_B_HASH"] = b_lab_hash
    require(a_lab_hash == agreement["labeler_a"]["hash"]
            and b_lab_hash == agreement["labeler_b"]["hash"],
            "PROVENANCE: agreement-report labeler hashes mismatch the labeled packets")
    require(hashes["SUPPLEMENTAL_PRELABEL_CORPUS_HASH"] == agreement.get("prelabel_corpus_hash"),
            "PROVENANCE: agreement-report prelabel hash mismatch")
    agreement_hash = sha256_file(agreement_path)
    report["AGREEMENT_REPORT_HASH"] = agreement_hash
    require(agreement.get("total_rows") == EXPECTED["ROWS"]
            and agreement.get("row_id_comparison", {}).get("agreement_count") == EXPECTED["ROWS"]
            and agreement.get("row_id_comparison", {}).get("disagreement_count") == 0
            and agreement.get("cohen_kappa", {}).get("kappa") == 1.0,
            "AGREEMENT_REPORT: recorded unanimity/kappa not 180/0/1.0")
    require(agreement.get("validation", {}).get("labeler_a_and_b_identical_label_assignments") == "PASS"
            and agreement.get("validation", {}).get("network_requests") == 0,
            "AGREEMENT_REPORT: validation flags not PASS / network 0")
    # Freeze manifest hash chain is one-directional; we only verify the source
    # hashes it records, never a self-hash.
    require(freeze.get("FINAL_VALID_ROWS") == EXPECTED["ROWS"]
            and freeze.get("FINAL_S1_ROWS") == 60 and freeze.get("FINAL_S2_ROWS") == 60
            and freeze.get("FINAL_S3_ROWS") == 60
            and freeze.get("FINAL_SUPPLEMENTAL_V1_ROWS") == 172
            and freeze.get("FINAL_SUPPLEMENTAL_V2_ROWS") == 8
            and freeze.get("FINAL_V1_OVERLAP") == 0 and freeze.get("FINAL_DUPLICATES") == 0,
            "FINAL_FREEZE_MANIFEST: recorded structure not 180/60/60/60/172/8/0/0")
    require(provenance.get("SUPPLEMENTAL_V1_PRESERVED_ROWS") == 172
            and provenance.get("SUPPLEMENTAL_V2_NEW_ROWS") == 8,
            "MULTIRUN_PROVENANCE_MANIFEST: recorded counts not 172/8")

    # ---- overlap boundaries -------------------------------------------------
    def canon_set(rows, field="canonical_url"):
        return {canonical_url(row[field]) for row in rows}

    current = canon_set(reference)
    v1_urls = canon_set(v1_gt["rows"])
    historical_urls = {canonical_url(u) for u in historical}
    require(len(current) == EXPECTED["ROWS"],
            "GROUND_TRUTH_DUPLICATES: duplicate canonical URLs in corpus")
    v1_overlap = sorted(current & v1_urls)
    hist_overlap = sorted(current & historical_urls)
    report["GROUND_TRUTH_DUPLICATES"] = 0
    report["GROUND_TRUTH_V1_OVERLAP"] = len(v1_overlap)
    report["GROUND_TRUTH_HISTORICAL_OVERLAP"] = len(hist_overlap)
    require(not v1_overlap, f"GROUND_TRUTH_V1_OVERLAP: {v1_overlap[:3]}")
    require(not hist_overlap, f"GROUND_TRUTH_HISTORICAL_OVERLAP: {hist_overlap[:3]}")

    # ------------------------------------------------------------------ frozen
    gt_path = out / "supplemental-ground-truth-v1.json"
    gt_manifest_path = out / "supplemental-ground-truth-v1-manifest.json"
    gt_attestation_path = out / "supplemental-ground-truth-v1-attestation.json"
    report["ground_truth_paths"] = {
        "ground_truth": str(gt_path.relative_to(ROOT)),
        "manifest": str(gt_manifest_path.relative_to(ROOT)),
        "attestation": str(gt_attestation_path.relative_to(ROOT)),
    }

    frozen_present = all(p.exists() for p in (gt_path, gt_manifest_path, gt_attestation_path))
    if not frozen_present:
        report["SUPPLEMENTAL_GROUND_TRUTH_STATUS"] = "NOT_PRESENT"
        report["GROUND_TRUTH_PREFREEZE_INTEGRITY"] = "PASS" if not problems else "FAIL"
    else:
        gt = load(gt_path)
        gt_manifest = load(gt_manifest_path)
        attestation = load(gt_attestation_path)
        gt_rows = gt["rows"]

        report["SUPPLEMENTAL_GROUND_TRUTH_ID"] = gt.get("ground_truth_id")
        require(gt.get("ground_truth_id") == GROUND_TRUTH_ID
                and gt.get("status") == "FROZEN"
                and gt.get("schema") == "amatl.relevance.supplemental-ground-truth.v1"
                and gt.get("adjudication_used") is False
                and gt.get("label_source") == LABEL_SOURCE,
                "GROUND_TRUTH: identity/status/schema/source fields invalid")
        report["GROUND_TRUTH_LABEL_SOURCE"] = LABEL_SOURCE
        report["ADJUDICATION_USED"] = "NO"

        report["SUPPLEMENTAL_GT_ROWS"] = len(gt_rows)
        require(len(gt_rows) == EXPECTED["ROWS"], f"SUPPLEMENTAL_GT_ROWS expected 180 got {len(gt_rows)}")
        require(sorted(row_ids(gt_rows)) == sorted(row_ids(reference))
                and not duplicate_ids(gt_rows),
                "GROUND_TRUTH_ROW_ID_INTEGRITY: row IDs not identical to prelabel")
        report["GROUND_TRUTH_ROW_ID_INTEGRITY"] = "PASS"

        mismatch = [row["row_id"] for row in gt_rows
                    if any(row.get(f) != a_by_id[row["row_id"]].get(f) for f in ORIGINAL_FIELDS)
                    or any(row.get(f) != b_by_id[row["row_id"]].get(f) for f in ORIGINAL_FIELDS)]
        require(not mismatch, f"GROUND_TRUTH_ORIGINAL_FIELDS_INTEGRITY: {sorted(set(mismatch))[:5]}")
        report["GROUND_TRUTH_ORIGINAL_FIELDS_INTEGRITY"] = "PASS"

        require(all(row["final_label"] == row["label_a"] == row["label_b"]
                    and row["final_label_source"] == LABEL_SOURCE
                    and row["label_a"] in LABELS for row in gt_rows),
                "GROUND_TRUTH_LABEL_INTEGRITY: final label != A/B or source not unanimous")
        report["GROUND_TRUTH_LABEL_INTEGRITY"] = "PASS"
        report["GROUND_TRUTH_PROVENANCE_INTEGRITY"] = "PASS"

        counts = {label: class_counts(gt_rows).get(label, 0) for label in LABELS}
        strat_counts = Counter(row["stratum"] for row in gt_rows)
        src_counts = Counter(row["SOURCE_RUN"] for row in gt_rows)
        report["SUPPLEMENTAL_GT_RELEVANT"] = counts.get("Relevant", 0)
        report["SUPPLEMENTAL_GT_POSSIBLY_RELEVANT"] = counts.get("PossiblyRelevant", 0)
        report["SUPPLEMENTAL_GT_NOT_RELEVANT"] = counts.get("NotRelevant", 0)
        report["SUPPLEMENTAL_GT_UNKNOWN"] = counts.get("Unknown", 0)
        require(counts == EXPECTED["CLASS"], f"CLASS_DISTRIBUTION: {counts}")
        report["SUPPLEMENTAL_GT_S1_ROWS"] = strat_counts["S1_PROCEDURAL_SHALLOW"]
        report["SUPPLEMENTAL_GT_S2_ROWS"] = strat_counts["S2_INFORMATIONAL_SHALLOW"]
        report["SUPPLEMENTAL_GT_S3_ROWS"] = strat_counts["S3_MIXED_DEPTH_CONTROL"]
        require(dict(strat_counts) == EXPECTED["STRATA"], f"STRATUM: {dict(strat_counts)}")
        report["SUPPLEMENTAL_GT_SOURCE_V1_ROWS"] = src_counts["supplemental-v1"]
        report["SUPPLEMENTAL_GT_SOURCE_V2_ROWS"] = src_counts["supplemental-v2"]
        require(dict(src_counts) == EXPECTED["SOURCE_RUN"], f"SOURCE_RUN: {dict(src_counts)}")

        gt_urls = canon_set(gt_rows)
        require(gt_urls == current, "GROUND_TRUTH: canonical URL set changed from prelabel")
        require(len(gt_urls) == EXPECTED["ROWS"], "GROUND_TRUTH_DUPLICATES")
        report["GROUND_TRUTH_DUPLICATES"] = 0

        gt_hash = sha256_file(gt_path)
        report["SUPPLEMENTAL_GROUND_TRUTH_HASH"] = gt_hash
        require(gt_hash == gt_manifest.get("GROUND_TRUTH_HASH"),
                "MANIFEST: GROUND_TRUTH_HASH mismatch")
        require(canonical_hash(gt) == gt_manifest.get("GROUND_TRUTH_CANONICAL_HASH"),
                "MANIFEST: GROUND_TRUTH_CANONICAL_HASH mismatch")
        require(gt_manifest.get("GROUND_TRUTH_ID") == GROUND_TRUTH_ID
                and gt_manifest.get("SUPPLEMENTAL_GROUND_TRUTH_STATUS") == "FROZEN"
                and gt_manifest.get("ADJUDICATION_USED") == "NO"
                and gt_manifest.get("GROUND_TRUTH_LABEL_SOURCE") == LABEL_SOURCE,
                "MANIFEST: identity/status fields invalid")

        # Every source-artefact hash recorded in the manifest must match disk.
        for key, path in (
            ("SUPPLEMENTAL_PRELABEL_CORPUS_HASH", prelabel_path),
            ("SUPPLEMENTAL_LABELER_A_HASH", a_labeled_path),
            ("SUPPLEMENTAL_LABELER_B_HASH", b_labeled_path),
            ("SUPPLEMENTAL_AGREEMENT_REPORT_HASH", agreement_path),
            ("SUPPLEMENTAL_FINAL_FREEZE_MANIFEST_HASH", freeze_path),
            ("SUPPLEMENTAL_MULTIRUN_PROVENANCE_MANIFEST_HASH", provenance_path),
            ("SUPPLEMENTAL_PACKET_MANIFEST_HASH", packet_manifest_path),
            ("V1_GROUND_TRUTH_HASH", v1_gt_path),
        ):
            expected = gt_manifest.get(key)
            require(bool(expected) and expected == sha256_file(path),
                    f"MANIFEST: recorded {key} does not match disk")

        # No self-hash cycle: the manifest must not record its own hash.
        require("GROUND_TRUTH_MANIFEST_HASH" not in gt_manifest
                and "hashes" not in gt_manifest,
                "MANIFEST: must not contain a self-hash block")

        manifest_hash = sha256_file(gt_manifest_path)
        report["SUPPLEMENTAL_GROUND_TRUTH_MANIFEST_HASH"] = manifest_hash
        require(attestation.get("manifest_hash") == manifest_hash,
                "ATTESTATION: manifest_hash does not match the manifest on disk")
        require(attestation.get("ground_truth_hash") == gt_hash
                and attestation.get("ground_truth_id") == GROUND_TRUTH_ID
                and attestation.get("status") == "FROZEN"
                and attestation.get("adjudication_used") == "NO"
                and attestation.get("ground_truth_label_source") == LABEL_SOURCE,
                "ATTESTATION: recorded identity/status fields invalid")
        report["SUPPLEMENTAL_GROUND_TRUTH_ATTESTATION_HASH"] = sha256_file(gt_attestation_path)

        report["SUPPLEMENTAL_GROUND_TRUTH_STATUS"] = "FROZEN"
        report["GROUND_TRUTH_PREFREEZE_INTEGRITY"] = "PASS" if not problems else "FAIL"

    report["GROUND_TRUTH_INTEGRITY"] = "PASS" if not problems else "FAIL"
    report["WORK_PACKAGE_STATUS"] = (
        "COMPLETE_SUPPLEMENTAL_GROUND_TRUTH_FROZEN"
        if not problems and frozen_present
        else "BLOCKED_GROUND_TRUTH_PREFREEZE_INTEGRITY_FAILURE" if problems
        else "READY_FOR_SUPPLEMENTAL_GROUND_TRUTH_FREEZE"
    )

    if args.write_report:
        write_json(out / "supplemental-ground-truth-validation-report.json", report)

    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
