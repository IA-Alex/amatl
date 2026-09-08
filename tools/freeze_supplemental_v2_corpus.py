#!/usr/bin/env python3
"""Merge 172 preserved supplemental-v1 rows with 8 accepted supplemental-v2
rows into the frozen 180-row pre-label corpus and emit the A/B labeler
packets plus the freeze and multirun provenance manifests.

This tool never labels, never reads a label artefact, and never touches the
368-row V1 ground truth beyond using its canonical URLs as a contamination
boundary.  It only freezes pre-label rows.  It stops after A/B packet
validation (STOP boundary); it never runs labelers or adjudication.
"""
import argparse
import hashlib
import json
import subprocess
import sys
from collections import Counter, OrderedDict
from pathlib import Path
from urllib.parse import urlsplit

from supplemental_pilot_contract import canonical_url

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"

CORPUS_SCHEMA = "amatl.relevance.supplemental-final-prelabel-corpus.v1"
FREEZE_SCHEMA = "amatl.relevance.supplemental-final-freeze-manifest.v1"
PROVENANCE_SCHEMA = "amatl.relevance.supplemental-multirun-provenance-manifest.v1"
PACKET_SCHEMA = "amatl.relevance.supplemental-annotation-packet.v1"
PACKET_MANIFEST_SCHEMA = "amatl.relevance.supplemental-packet-manifest.v1"

V1_EVIDENCE = "supplemental-pilot-raw-evidence.json"
V1_LEDGER = "supplemental-pilot-attempts.json"
V2_EVIDENCE = "supplemental-v2-raw-evidence.json"
V2_LEDGER = "supplemental-v2-attempts.json"

V1_RUN_ID = "independent-relevance-supplemental-v1"
V2_RUN_ID = "independent-relevance-supplemental-v2"
STRATUM_ORDER = ("S1_PROCEDURAL_SHALLOW", "S2_INFORMATIONAL_SHALLOW", "S3_MIXED_DEPTH_CONTROL")


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n"


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def source_commit(manifest_path, explicit=None):
    existing = load(manifest_path) if Path(manifest_path).exists() else {}
    return existing.get("SOURCE_COMMIT") or explicit or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()


def domain_of(url):
    return urlsplit(url).netloc


def reconstruct_rows(evidence_path, accepted, run_id, source_run, evidence_relpath):
    """Rebuild full pre-label rows from raw evidence + accepted ledger rows."""
    evidence = load(evidence_path)
    evidence_hash = evidence.get("artifact_sha256") or sha256_file(evidence_path)
    by_query = {}
    for attempt in evidence["attempts"]:
        bucket = {canonical_url(result["original_url"]): result for result in attempt["results"]}
        by_query[attempt["query_id"]] = (attempt["query_text"], attempt.get("requested_at"), bucket)
    rows = []
    for item in accepted:
        query_id = item["query_id"]
        canonical = item["canonical_url"]
        query_text, requested_at, bucket = by_query[query_id]
        result = bucket[canonical]
        rank = result["rank"]
        rows.append({
            "row_id": f"{query_id}-{rank:03d}",
            "query_id": query_id,
            "query": query_text,
            "provider_or_source": ["searxng"],
            "original_url": result["original_url"],
            "canonical_url": canonical,
            "domain": domain_of(result["original_url"]),
            "title": result["title"],
            "snippet": result["snippet"],
            "rank": rank,
            "result_status": "visible",
            "published_at": None,
            "acquired_at": requested_at,
            "acquisition_run_id": run_id,
            "raw_response_path": str(evidence_relpath),
            "raw_response_sha256": evidence_hash,
            "SOURCE_RUN": source_run,
            "stratum": item["stratum"],
        })
    return rows


def load_v1_rows():
    ledger = load(OUT / V1_LEDGER)
    accepted = [a for a in ledger["attempts"] if a["reason_code"] == "ACCEPTED"]
    rows = reconstruct_rows(OUT / V1_EVIDENCE, accepted, V1_RUN_ID, "supplemental-v1",
                            Path("docs/evaluation/independent-relevance/new-corpus-v1/adjudication") / V1_EVIDENCE)
    if len(rows) != 172:
        raise ValueError(f"V1_RECONSTRUCTION_COUNT_EXPECTED_172_GOT_{len(rows)}")
    return rows


def load_v2_rows(v2_dir):
    ledger = load(Path(v2_dir) / V2_LEDGER)
    accepted = [a for a in ledger["attempts"] if a["reason_code"] == "ACCEPTED"]
    rows = reconstruct_rows(Path(v2_dir) / V2_EVIDENCE, accepted, V2_RUN_ID, "supplemental-v2",
                            Path("docs/evaluation/independent-relevance/new-corpus-v1/adjudication") / V2_EVIDENCE)
    return rows


def validate_multirun(rows):
    checks = {}
    checks["FINAL_VALID_ROWS"] = len(rows)
    row_ids = [r["row_id"] for r in rows]
    checks["FINAL_DUPLICATES"] = len(row_ids) - len(set(row_ids)) + len(rows) - len({r["canonical_url"] for r in rows})
    if len(row_ids) != len(set(row_ids)):
        raise ValueError("DUPLICATE_ROW_ID")
    if len({r["canonical_url"] for r in rows}) != len(rows):
        raise ValueError("DUPLICATE_CANONICAL_URL")
    ground = {canonical_url(r["canonical_url"]) for r in load(OUT / "v1-ground-truth.json")["rows"]}
    historical = {canonical_url(u) for u in load(OUT / "supplemental-pilot-historical-canonical-urls.json")}
    corpus_urls = {r["canonical_url"] for r in rows}
    checks["FINAL_V1_OVERLAP"] = len(corpus_urls & ground)
    checks["FINAL_HISTORICAL_OVERLAP"] = len(corpus_urls & historical)
    if checks["FINAL_V1_OVERLAP"] or checks["FINAL_HISTORICAL_OVERLAP"]:
        raise ValueError("CORPUS_CONTAMINATION")
    by_stratum = Counter(r["stratum"] for r in rows)
    by_source = Counter(r["SOURCE_RUN"] for r in rows)
    checks["FINAL_S1_ROWS"] = by_stratum["S1_PROCEDURAL_SHALLOW"]
    checks["FINAL_S2_ROWS"] = by_stratum["S2_INFORMATIONAL_SHALLOW"]
    checks["FINAL_S3_ROWS"] = by_stratum["S3_MIXED_DEPTH_CONTROL"]
    checks["FINAL_SUPPLEMENTAL_V1_ROWS"] = by_source["supplemental-v1"]
    checks["FINAL_SUPPLEMENTAL_V2_ROWS"] = by_source["supplemental-v2"]
    if (checks["FINAL_S1_ROWS"], checks["FINAL_S2_ROWS"], checks["FINAL_S3_ROWS"]) != (60, 60, 60):
        raise ValueError("STRATUM_COUNTS_VIOLATION")
    if (checks["FINAL_SUPPLEMENTAL_V1_ROWS"], checks["FINAL_SUPPLEMENTAL_V2_ROWS"]) != (172, 8):
        raise ValueError("SOURCE_RUN_COUNTS_VIOLATION")
    return checks


def freeze(v2_dir, output_dir, explicit_source_commit=None):
    v1_rows = load_v1_rows()
    v2_rows = load_v2_rows(v2_dir)
    if len(v2_rows) != 8:
        raise ValueError(f"V2_ROWS_EXPECTED_8_GOT_{len(v2_rows)}")
    rows = v1_rows + v2_rows
    rows.sort(key=lambda r: (STRATUM_ORDER.index(r["stratum"]), r["query_id"], r["rank"]))
    checks = validate_multirun(rows)

    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    corpus = {"schema": CORPUS_SCHEMA, "corpus_id": "independent-relevance-supplemental-final-20260902",
              "total_rows": len(rows), "status": "FROZEN_PRELABEL", "rows": rows}
    corpus_path = output_dir / "supplemental-final-prelabel-corpus.json"
    corpus_path.write_text(canonical_json(corpus), encoding="utf-8")
    corpus_hash = sha256_file(corpus_path)

    v1_evidence_hash = sha256_file(OUT / V1_EVIDENCE)
    v2_evidence_hash = sha256_file(Path(v2_dir) / V2_EVIDENCE)
    v2_ledger_hash = sha256_file(Path(v2_dir) / V2_LEDGER)
    universe1_hash = sha256_file(OUT / "supplemental-query-universe-v1.json")
    manifest1_hash = sha256_file(OUT / "supplemental-query-universe-v1-manifest.json")
    universe2_hash = sha256_file(OUT / "supplemental-query-universe-v2.json")
    manifest2_hash = sha256_file(OUT / "supplemental-query-universe-v2-manifest.json")

    freeze_manifest = {
        "schema": FREEZE_SCHEMA,
        "corpus_id": corpus["corpus_id"],
        "freeze_status": "FROZEN",
        "FINAL_VALID_ROWS": checks["FINAL_VALID_ROWS"],
        "FINAL_S1_ROWS": checks["FINAL_S1_ROWS"],
        "FINAL_S2_ROWS": checks["FINAL_S2_ROWS"],
        "FINAL_S3_ROWS": checks["FINAL_S3_ROWS"],
        "FINAL_SUPPLEMENTAL_V1_ROWS": checks["FINAL_SUPPLEMENTAL_V1_ROWS"],
        "FINAL_SUPPLEMENTAL_V2_ROWS": checks["FINAL_SUPPLEMENTAL_V2_ROWS"],
        "FINAL_DUPLICATES": checks["FINAL_DUPLICATES"],
        "FINAL_V1_OVERLAP": checks["FINAL_V1_OVERLAP"],
        "FINAL_HISTORICAL_OVERLAP": checks["FINAL_HISTORICAL_OVERLAP"],
        "hashes": {
            "supplemental-final-prelabel-corpus.json": corpus_hash,
            "supplemental-pilot-raw-evidence.json": v1_evidence_hash,
            "supplemental-query-universe-v1.json": universe1_hash,
            "supplemental-query-universe-v1-manifest.json": manifest1_hash,
            "supplemental-v2-raw-evidence.json": v2_evidence_hash,
            "supplemental-v2-attempts.json": v2_ledger_hash,
            "supplemental-query-universe-v2.json": universe2_hash,
            "supplemental-query-universe-v2-manifest.json": manifest2_hash,
            "v1-ground-truth.json": sha256_file(OUT / "v1-ground-truth.json"),
            "v1-ground-truth-manifest.json": sha256_file(OUT / "v1-ground-truth-manifest.json"),
        },
        "SOURCE_COMMIT": source_commit(output_dir / "supplemental-final-freeze-manifest.json",
                                        explicit_source_commit),
        "CREATED_AT": "2026-09-02T00:00:00Z",
    }
    freeze_path = output_dir / "supplemental-final-freeze-manifest.json"
    freeze_path.write_text(canonical_json(freeze_manifest), encoding="utf-8")

    provenance = {
        "schema": PROVENANCE_SCHEMA,
        "SUPPLEMENTAL_V1_PRESERVED_ROWS": checks["FINAL_SUPPLEMENTAL_V1_ROWS"],
        "SUPPLEMENTAL_V2_NEW_ROWS": checks["FINAL_SUPPLEMENTAL_V2_ROWS"],
        "preserved_source_run": "supplemental-v1",
        "new_source_run": "supplemental-v2",
        "row_identity_rule": "Preserve existing supplemental-v1 row IDs exactly; assign SOURCE_RUN without regenerating them. Supplemental-v1 row identity is the deterministic {query_id}-{rank:03d} derived from the frozen acquisition ledger and raw evidence; it is preserved byte-for-byte in the final corpus.",
        "lineage": {
            "supplemental-v1": {"rows": 172, "raw_evidence": V1_EVIDENCE, "ledger": V1_LEDGER, "evidence_sha256": v1_evidence_hash, "universe": "independent-relevance-supplemental-v1"},
            "supplemental-v2": {"rows": 8, "raw_evidence": V2_EVIDENCE, "ledger": V2_LEDGER, "evidence_sha256": v2_evidence_hash, "universe": "independent-relevance-supplemental-v2"},
        },
        "dedup_boundaries": ["v1-ground-truth", "historical-canonical-urls", "supplemental-v1-valid", "accepted-supplemental-v2"],
    }
    provenance_path = output_dir / "supplemental-multirun-provenance-manifest.json"
    provenance_path.write_text(canonical_json(provenance), encoding="utf-8")

    packet_rows = [dict(r, label="", annotation_note="") for r in rows]
    packets = {}
    for packet_id in ("A", "B"):
        packet = {"schema": PACKET_SCHEMA, "packet_id": packet_id, "corpus_id": corpus["corpus_id"],
                  "prelabel_corpus_hash": corpus_hash, "label_observation_status": "NOT_STARTED",
                  "rows": [dict(r) for r in packet_rows]}
        packets[packet_id] = packet
    a_path = output_dir / "supplemental-labeler-a-packet.json"
    b_path = output_dir / "supplemental-labeler-b-packet.json"
    a_path.write_text(canonical_json(packets["A"]), encoding="utf-8")
    b_path.write_text(canonical_json(packets["B"]), encoding="utf-8")
    a_hash = sha256_file(a_path)
    b_hash = sha256_file(b_path)

    def packet_fingerprint(packet):
        return canonical_json([{k: v for k, v in r.items() if k not in ("label", "annotation_note")} for r in packet["rows"]])
    universe_match = packet_fingerprint(packets["A"]) == packet_fingerprint(packets["B"])
    labels_empty = all(r["label"] == "" and r["annotation_note"] == "" for r in packets["A"]["rows"]) and \
        all(r["label"] == "" and r["annotation_note"] == "" for r in packets["B"]["rows"])
    packet_manifest = {
        "schema": PACKET_MANIFEST_SCHEMA,
        "corpus_id": corpus["corpus_id"],
        "rows_per_packet": len(packet_rows),
        "SUPPLEMENTAL_PACKETS_UNIVERSE_MATCH": "PASS" if universe_match else "FAIL",
        "SUPPLEMENTAL_PACKETS_LABELS_EMPTY": "PASS" if labels_empty else "FAIL",
        "SUPPLEMENTAL_LABELER_A_PACKET": str(a_path.relative_to(ROOT)),
        "SUPPLEMENTAL_LABELER_A_HASH": a_hash,
        "SUPPLEMENTAL_LABELER_B_PACKET": str(b_path.relative_to(ROOT)),
        "SUPPLEMENTAL_LABELER_B_HASH": b_hash,
    }
    if not universe_match or not labels_empty:
        raise ValueError("PACKET_VALIDATION_FAILURE")
    packet_manifest_path = output_dir / "supplemental-packet-manifest.json"
    packet_manifest_path.write_text(canonical_json(packet_manifest), encoding="utf-8")
    packet_manifest_hash = sha256_file(packet_manifest_path)

    freeze_manifest["hashes"]["supplemental-final-freeze-manifest.json"] = sha256_file(freeze_path)
    freeze_manifest["hashes"]["supplemental-multirun-provenance-manifest.json"] = sha256_file(provenance_path)
    freeze_manifest["hashes"]["supplemental-packet-manifest.json"] = packet_manifest_hash
    freeze_manifest["SUPPLEMENTAL_PACKETS_UNIVERSE_MATCH"] = packet_manifest["SUPPLEMENTAL_PACKETS_UNIVERSE_MATCH"]
    freeze_manifest["SUPPLEMENTAL_PACKETS_LABELS_EMPTY"] = packet_manifest["SUPPLEMENTAL_PACKETS_LABELS_EMPTY"]
    freeze_manifest["SUPPLEMENTAL_LABELER_A_HASH"] = a_hash
    freeze_manifest["SUPPLEMENTAL_LABELER_B_HASH"] = b_hash
    freeze_manifest["FINAL_PRELABEL_HASH"] = corpus_hash
    freeze_manifest["FINAL_MANIFEST_HASH"] = sha256_file(freeze_path)
    freeze_manifest["FINAL_PROVENANCE_MANIFEST_HASH"] = sha256_file(provenance_path)
    freeze_path.write_text(canonical_json(freeze_manifest), encoding="utf-8")

    return {"checks": checks, "corpus_hash": corpus_hash, "freeze_hash": sha256_file(freeze_path),
            "provenance_hash": sha256_file(provenance_path), "packet_manifest_hash": packet_manifest_hash,
            "a_hash": a_hash, "b_hash": b_hash}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--v2-artifacts-dir", default=OUT)
    parser.add_argument("--output-dir", default=OUT)
    parser.add_argument("--source-commit", help="source commit for an initial freeze")
    args = parser.parse_args()
    result = freeze(args.v2_artifacts_dir, args.output_dir, args.source_commit)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
