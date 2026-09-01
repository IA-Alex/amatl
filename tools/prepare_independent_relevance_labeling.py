#!/usr/bin/env python3
"""Validate and freeze the unlabelled independent-relevance annotation pool."""
import argparse
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = ("row_id", "query", "provider_or_source", "original_url", "canonical_url",
            "domain", "title", "snippet", "acquired_at", "acquisition_run_id",
            "raw_response_path", "raw_response_sha256")

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def norm(value): return re.sub(r"\s+", " ", str(value or "").strip().casefold())
def key(row): return (norm(row.get("query")), norm(row.get("canonical_url")), norm(row.get("title")), norm(row.get("snippet")))

def historical_keys():
    paths = [ROOT / "crates/amatl-core/tests/fixtures/relevance/corpus.json",
             ROOT / "crates/amatl-core/tests/fixtures/relevance/holdout.json",
             ROOT / "crates/amatl-core/tests/fixtures/relevance/holdout_labels_independent.json",
             ROOT / "docs/evaluation/step4e/step4e_ground_truth.json"]
    values = set()
    for path in paths:
        if not path.exists(): continue
        doc = json.loads(path.read_text())
        for row in doc.get("samples", doc.get("rows", [])):
            values.add(key(row))
    return values

def main():
    p = argparse.ArgumentParser(); p.add_argument("raw"); p.add_argument("identity"); p.add_argument("output")
    args = p.parse_args()
    raw, identity = (json.loads(Path(x).read_text()) for x in (args.raw, args.identity))
    known, seen, accepted = historical_keys(), set(), []
    invalid = exact_duplicates = historical_duplicates = 0
    for row in raw["rows"]:
        if any(not row.get(field) for field in REQUIRED): invalid += 1; continue
        if not (ROOT / row["raw_response_path"]).is_file(): invalid += 1; continue
        if sha(ROOT / row["raw_response_path"]) != row["raw_response_sha256"]: invalid += 1; continue
        identity_key = key(row)
        if identity_key in seen: exact_duplicates += 1; continue
        seen.add(identity_key)
        if identity_key in known: historical_duplicates += 1; continue
        accepted.append(row)
    accepted.sort(key=lambda row: row["row_id"])
    out = Path(args.output); out.mkdir(parents=True, exist_ok=False)
    prelabel = {"schema":"amatl.relevance.prelabel-snapshot.v1", "corpus_id":identity["corpus_id"], "raw_acquisition_hash":sha(args.raw), "rows":accepted}
    prelabel_path = out / "prelabel-snapshot.json"; prelabel_path.write_text(json.dumps(prelabel, indent=2) + "\n")
    rubric = (ROOT / "docs/evaluation/step4e/annotation_rubric.md").read_text()
    for packet in ("A", "B"):
        rows = [{k:v for k,v in row.items() if k not in {"final_label", "label_a", "label_b"}} | {"label":"", "annotation_note":""} for row in accepted]
        (out / f"annotation-packet-{packet}.json").write_text(json.dumps({"schema":"amatl.relevance.annotation-packet.v1", "packet_id":packet, "rubric":rubric, "prelabel_snapshot_hash":sha(prelabel_path), "rows":rows}, indent=2) + "\n")
    manifest = {"schema":"amatl.relevance.prelabel-manifest.v1", "corpus_id":identity["corpus_id"], "RAW_ROWS":len(raw["rows"]), "VALID_ROWS":len(accepted), "DUPLICATES_REMOVED":exact_duplicates + historical_duplicates, "EXACT_DUPLICATES_REMOVED":exact_duplicates, "HISTORICAL_DUPLICATES_REMOVED":historical_duplicates, "INVALID_ROWS":invalid, "FINAL_PRELABEL_ROWS":len(accepted), "prelabel_snapshot_hash":sha(prelabel_path), "annotation_packet_a_hash":sha(out / "annotation-packet-A.json"), "annotation_packet_b_hash":sha(out / "annotation-packet-B.json"), "label_observation_status":"NOT_STARTED"}
    (out / "prelabel-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

if __name__ == "__main__": main()
