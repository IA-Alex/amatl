#!/usr/bin/env python3
"""Capture real AMATL search observations for the independent relevance campaign.

This tool never labels, ranks for relevance, or reads a blind label artefact.
It uses the already-approved local AMATL configuration and records every raw
response needed to reconstruct accepted rows.
"""
import argparse
import datetime as dt
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("query_set")
    parser.add_argument("identity")
    parser.add_argument("output")
    parser.add_argument("--amatl", default=str(ROOT / "target" / "debug" / "amatl"))
    args = parser.parse_args()
    query_set, identity = (json.loads(Path(p).read_text()) for p in (args.query_set, args.identity))
    if identity["label_observation_status"] != "NOT_STARTED":
        raise ValueError("identity must be created before labels are observed")
    if identity["query_set_id"] != query_set["query_set_id"]:
        raise ValueError("query-set identity mismatch")
    out = Path(args.output); raw = out / "raw"
    raw.mkdir(parents=True, exist_ok=False)
    observations = []
    started = dt.datetime.now(dt.timezone.utc).isoformat()
    for item in query_set["queries"]:
        completed = subprocess.run([args.amatl, "search", item["query"], "--json"], cwd=ROOT, text=True, capture_output=True)
        if completed.returncode not in (0, 1):
            raise RuntimeError(f"AMATL failed for {item['query_id']}: {completed.returncode}")
        response = json.loads(completed.stdout)
        raw_path = raw / f"{item['query_id']}.json"
        raw_path.write_text(json.dumps(response, indent=2) + "\n")
        captured_at = dt.datetime.now(dt.timezone.utc).isoformat()
        for result in response.get("results", []):
            observations.append({
                "row_id": f"{item['query_id']}-{result['rank']:03d}", "query_id": item["query_id"],
                "query": item["query"], "provider_or_source": result.get("providers", []),
                "original_url": result["original_url"], "canonical_url": result["canonical_url"],
                "domain": result["domain"], "title": result["title"], "snippet": result["snippet"],
                "rank": result["rank"], "result_status": result["status"], "published_at": result.get("published_at"),
                "acquired_at": captured_at, "acquisition_run_id": identity["corpus_id"],
                "raw_response_path": str(raw_path.resolve().relative_to(ROOT)), "raw_response_sha256": sha(raw_path),
            })
    capture = {"schema":"amatl.relevance.raw-acquisition.v1", "corpus_id":identity["corpus_id"],
               "started_at":started, "completed_at":dt.datetime.now(dt.timezone.utc).isoformat(),
               "query_set_hash":sha(args.query_set), "identity_hash":hashlib.sha256(canonical(identity)).hexdigest(),
               "raw_rows":len(observations), "rows":observations}
    (out / "raw-observations.json").write_text(json.dumps(capture, indent=2) + "\n")

if __name__ == "__main__": main()
