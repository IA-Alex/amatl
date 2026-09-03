#!/usr/bin/env python3
"""Freeze the unanimous 180-row supplemental ground truth without adjudication.

Authority model (this tool has no authority to label or to adjudicate):
  * Both independent labelers agree on all 180 rows
    (AGREEMENT_COUNT=180, DISAGREEMENT_COUNT=0, COHEN_KAPPA=1.0), so the final
    label of every row is exactly label_a == label_b.
  * No adjudicator is created, no human decision is introduced, and no label is
    changed.  final_label_source is UNANIMOUS_A_B_AGREEMENT on every row.
  * Row IDs are preserved byte-for-byte from the frozen pre-label corpus; the
    tool never regenerates them.

Generated artefacts (all in docs/.../adjudication/):
  * supplemental-ground-truth-v1.json            (180 rows + provenance)
  * supplemental-ground-truth-v1-manifest.json   (hashes of source artefacts;
                                                 no self-hash)
  * supplemental-ground-truth-v1-attestation.json (real hash of the final
                                                 manifest; no self-hash)

The hash chain is strictly one-directional:
  ground_truth <- manifest (hashes ground truth + sources) <- attestation
  (hashes manifest).  No cycles.

This tool never acquires rows, never runs the network, never trains or tunes a
model, and never touches V1 ground truth, the pre-label corpus, the labeler
packets, or the agreement report.  It stops after the freeze boundary.
"""
import argparse
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

from validate_supplemental_ground_truth import (  # noqa: E402
    EXPECTED, GROUND_TRUTH_ID, LABEL_SOURCE, LABELS, ORIGINAL_FIELDS, OUT,
    canonical_hash, load, sha256_file, write_json,
)

GT_NAME = "supplemental-ground-truth-v1.json"
MANIFEST_NAME = "supplemental-ground-truth-v1-manifest.json"
ATTESTATION_NAME = "supplemental-ground-truth-v1-attestation.json"
ROLE = "SUPPLEMENTAL_KNOWN_INDEPENDENT_GROUND_TRUTH"
CREATED_AT = "2026-09-02T00:00:00Z"


def source_commit(manifest_path, explicit=None):
    existing = load(manifest_path) if Path(manifest_path).exists() else {}
    return existing.get("SOURCE_COMMIT") or explicit or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()


def build_rows(reference, a_by_id, b_by_id):
    """Rebuild the 180 ground-truth rows with unanimous final labels.

    Preserves the frozen pre-label field order and the pre-label row order.
    The only additions are label_a/label_b provenance and the unanimous final
    label with its explicit source.
    """
    rows = []
    for source in reference:
        row_id = source["row_id"]
        la = a_by_id[row_id]["label"]
        lb = b_by_id[row_id]["label"]
        if la not in LABELS or lb not in LABELS:
            raise ValueError(f"invalid independent label: {row_id}")
        if la != lb:
            raise ValueError(f"unexpected disagreement, adjudication forbidden: {row_id}")
        row = {field: source[field] for field in ORIGINAL_FIELDS}
        row.update({
            "label_a": la,
            "label_b": lb,
            "final_label": la,
            "final_label_source": LABEL_SOURCE,
        })
        rows.append(row)
    return rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-commit", help="source commit for an initial freeze")
    args = parser.parse_args()
    # 1. Pre-freeze integrity gate (read-only validator).
    subprocess.run([sys.executable, str(ROOT / "tools/validate_supplemental_ground_truth.py")],
                   cwd=ROOT, check=True,
                   capture_output=True, text=True)

    prelabel = load(OUT / "supplemental-final-prelabel-corpus.json")
    a_doc = load(OUT / "supplemental-labeler-a-labeled.json")
    b_doc = load(OUT / "supplemental-labeler-b-labeled.json")
    agreement = load(OUT / "supplemental-agreement-report.json")
    freeze = load(OUT / "supplemental-final-freeze-manifest.json")
    provenance = load(OUT / "supplemental-multirun-provenance-manifest.json")
    packet_manifest = load(OUT / "supplemental-packet-manifest.json")

    reference = prelabel["rows"]
    a_by_id = {row["row_id"]: row for row in a_doc["rows"]}
    b_by_id = {row["row_id"]: row for row in b_doc["rows"]}
    if len(reference) != EXPECTED["ROWS"]:
        raise ValueError(f"prelabel rows expected {EXPECTED['ROWS']} got {len(reference)}")

    rows = build_rows(reference, a_by_id, b_by_id)
    counts = {label: 0 for label in LABELS}
    counts.update(Counter(row["final_label"] for row in rows))
    strat_counts = dict(Counter(row["stratum"] for row in rows))
    src_counts = dict(Counter(row["SOURCE_RUN"] for row in rows))
    if counts != EXPECTED["CLASS"]:
        raise ValueError(f"class distribution violation: {counts}")

    # 2. Ground truth document (no sort_keys: preserves frozen insertion order).
    prelabel_hash = sha256_file(OUT / "supplemental-final-prelabel-corpus.json")
    ground = {
        "schema": "amatl.relevance.supplemental-ground-truth.v1",
        "ground_truth_id": GROUND_TRUTH_ID,
        "role": ROLE,
        "status": "FROZEN",
        "prelabel_corpus": str((OUT / "supplemental-final-prelabel-corpus.json").relative_to(ROOT)),
        "prelabel_corpus_hash": prelabel_hash,
        "label_source": LABEL_SOURCE,
        "adjudication_used": False,
        "rows": rows,
    }
    gt_path = OUT / GT_NAME
    write_json(gt_path, ground)
    gt_hash = sha256_file(gt_path)

    # 3. Manifest: hashes of source artefacts only; never its own hash.
    manifest = {
        "schema": "amatl.relevance.supplemental-ground-truth-manifest.v1",
        "GROUND_TRUTH_ID": GROUND_TRUTH_ID,
        "GROUND_TRUTH_ROLE": ROLE,
        "SUPPLEMENTAL_GROUND_TRUTH_STATUS": "FROZEN",
        "SUPPLEMENTAL_GT_ROWS": len(rows),
        "SUPPLEMENTAL_GT_CLASS_DISTRIBUTION": counts,
        "SUPPLEMENTAL_GT_STRATUM_DISTRIBUTION": strat_counts,
        "SUPPLEMENTAL_GT_SOURCE_RUN_DISTRIBUTION": src_counts,
        "GROUND_TRUTH_LABEL_SOURCE": LABEL_SOURCE,
        "ADJUDICATION_USED": "NO",
        "GROUND_TRUTH_DUPLICATES": 0,
        "GROUND_TRUTH_V1_OVERLAP": 0,
        "GROUND_TRUTH_HISTORICAL_OVERLAP": 0,
        "SUPPLEMENTAL_PRELABEL_CORPUS_HASH": prelabel_hash,
        "SUPPLEMENTAL_LABELER_A_HASH": sha256_file(OUT / "supplemental-labeler-a-labeled.json"),
        "SUPPLEMENTAL_LABELER_B_HASH": sha256_file(OUT / "supplemental-labeler-b-labeled.json"),
        "SUPPLEMENTAL_AGREEMENT_REPORT_HASH": sha256_file(OUT / "supplemental-agreement-report.json"),
        "SUPPLEMENTAL_FINAL_FREEZE_MANIFEST_HASH": sha256_file(OUT / "supplemental-final-freeze-manifest.json"),
        "SUPPLEMENTAL_MULTIRUN_PROVENANCE_MANIFEST_HASH": sha256_file(OUT / "supplemental-multirun-provenance-manifest.json"),
        "SUPPLEMENTAL_PACKET_MANIFEST_HASH": sha256_file(OUT / "supplemental-packet-manifest.json"),
        "V1_GROUND_TRUTH_HASH": sha256_file(OUT / "v1-ground-truth.json"),
        "GROUND_TRUTH_HASH": gt_hash,
        "GROUND_TRUTH_CANONICAL_HASH": canonical_hash(ground),
        "CREATED_AT": CREATED_AT,
        "PROTOCOL_VERSION": "v1",
        "SOURCE_COMMIT": source_commit(OUT / MANIFEST_NAME, args.source_commit),
        "NETWORK_REQUESTS": 0,
        "hash_scope": ("The manifest hashes the ground-truth document it describes and every "
                       "source artefact; it does not hash itself.  The attestation records this "
                       "manifest's real SHA-256."),
    }
    manifest_path = OUT / MANIFEST_NAME
    write_json(manifest_path, manifest)
    manifest_hash = sha256_file(manifest_path)

    # 4. Attestation: real hash of the final manifest; never its own hash.
    attestation = {
        "schema": "amatl.relevance.supplemental-ground-truth-attestation.v1",
        "ground_truth_id": GROUND_TRUTH_ID,
        "status": "FROZEN",
        "ground_truth_path": str(gt_path.relative_to(ROOT)),
        "ground_truth_hash": gt_hash,
        "manifest_path": str(manifest_path.relative_to(ROOT)),
        "manifest_hash": manifest_hash,
        "attested_at": CREATED_AT,
        "attestor": "EXPERIMENTAL_INTEGRITY_AUDITOR",
        "adjudication_used": "NO",
        "ground_truth_label_source": LABEL_SOURCE,
        "network_requests": 0,
    }
    attestation_path = OUT / ATTESTATION_NAME
    write_json(attestation_path, attestation)
    attestation_hash = sha256_file(attestation_path)

    # 5. Post-freeze validation (read-only validator now sees the frozen set).
    subprocess.run([sys.executable, str(ROOT / "tools/validate_supplemental_ground_truth.py")],
                   cwd=ROOT, check=True, capture_output=True, text=True)

    print(json.dumps({
        "SUPPLEMENTAL_GROUND_TRUTH_STATUS": "FROZEN",
        "SUPPLEMENTAL_GROUND_TRUTH_ID": GROUND_TRUTH_ID,
        "SUPPLEMENTAL_GT_ROWS": len(rows),
        "SUPPLEMENTAL_GT_CLASS_DISTRIBUTION": counts,
        "SUPPLEMENTAL_GT_STRATUM_DISTRIBUTION": strat_counts,
        "SUPPLEMENTAL_GT_SOURCE_RUN_DISTRIBUTION": src_counts,
        "GROUND_TRUTH_LABEL_SOURCE": LABEL_SOURCE,
        "ADJUDICATION_USED": "NO",
        "SUPPLEMENTAL_GROUND_TRUTH_HASH": gt_hash,
        "SUPPLEMENTAL_GROUND_TRUTH_MANIFEST_HASH": manifest_hash,
        "SUPPLEMENTAL_GROUND_TRUTH_ATTESTATION_HASH": attestation_hash,
        "NETWORK_REQUESTS": 0,
        "WORK_PACKAGE_STATUS": "COMPLETE_SUPPLEMENTAL_GROUND_TRUTH_FROZEN",
        "NEXT_SINGLE_ACTION": ("Analyze the frozen supplemental ground truth against the frozen "
                               "V1 ground truth to measure relevance yield, stratum effects, and "
                               "determine whether further acquisition is justified."),
    }, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
