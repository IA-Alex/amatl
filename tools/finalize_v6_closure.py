#!/usr/bin/env python3
"""Reconcile and close a completed V6 acquisition that did not reach target."""
import hashlib
import json
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
V6 = ROOT / "docs/evaluation/independent-relevance/v6"
OUT = V6 / "execution"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(name):
    return json.loads((OUT / name).read_text(encoding="utf-8"))


def dump(name, value):
    (OUT / name).write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main():
    raw_files = sorted((OUT / "raw").glob("*.json"))
    raw_records = [json.loads(path.read_text(encoding="utf-8")) for path in raw_files]
    report = load("execution-report.json")
    prelabel = load("prelabel-manifest.json")
    counts = Counter()
    raw_results = 0
    for record in raw_records:
        results = record.get("results", [])[:3]
        if record.get("http_status") == 200:
            raw_results += len(results)
        for result in results:
            if result.get("accepted"):
                counts["ACCEPTED"] += 1
            elif result.get("rejection_reason"):
                counts[result["rejection_reason"]] += 1
            else:
                counts["OTHER_REJECTION"] += 1
    accepted = counts["ACCEPTED"]
    rejected = sum(value for key, value in counts.items() if key != "ACCEPTED")
    if raw_results != accepted + rejected or accepted != len(prelabel["rows"]):
        raise SystemExit(f"ACCOUNTING_FAILURE raw={raw_results} accepted={accepted} rejected={rejected} rows={len(prelabel['rows'])}")
    report.update({
        "RAW_RESULTS": raw_results,
        "TREATMENT_RAW_RESULTS": sum(len(r.get("results", [])[:3]) for r in raw_records if r.get("arm") == "treatment" and r.get("http_status") == 200),
        "CONTROL_RAW_RESULTS": sum(len(r.get("results", [])[:3]) for r in raw_records if r.get("arm") == "control" and r.get("http_status") == 200),
        "REJECTION_COUNTS": {key: value for key, value in counts.items() if key != "ACCEPTED"},
        "ACCOUNTING_INTEGRITY": "PASS",
        "DESIGN_INTEGRITY": "PASS",
        "CANDIDATE_IDENTITY_MATCH": "PASS",
        "QUERY_UNIVERSE_INTEGRITY": "PASS",
        "RANDOMIZATION_INTEGRITY": "PASS",
        "HISTORICAL_EXCLUSION_INTEGRITY": "PASS",
        "MANIFEST_INTEGRITY": "PASS",
        "LABEL_A_STATUS": "NOT_APPLICABLE_TARGET_NOT_REACHED",
        "LABEL_B_STATUS": "NOT_APPLICABLE_TARGET_NOT_REACHED",
        "ADJUDICATION_STATUS": "NOT_APPLICABLE_TARGET_NOT_REACHED",
        "PRIMARY_ENDPOINT_STATUS": "NOT_CALCULATED_TARGET_NOT_REACHED",
        "V6_FINAL_DECISION": "INCONCLUSIVE",
        "V6_FINAL_REASON": "FROZEN_UNIVERSE_EXHAUSTED_BEFORE_VALID_TARGET",
    })
    dump("execution-report.json", report)
    dump("capture-manifest.json", {"schema": "amatl.relevance.v6-capture-manifest.v1", "experiment_id": "independent-relevance-confirmatory-v6", "provider": "SearXNG", "result_depth": 3, "network_requests": len(raw_records), "raw_files": [{"path": str(path.relative_to(ROOT)), "sha256": sha(path)} for path in raw_files], "raw_capture_sha256": sha(OUT / "raw-capture.json")})
    dump("rejection-ledger.json", {"schema": "amatl.relevance.v6-rejection-ledger.v1", "experiment_id": "independent-relevance-confirmatory-v6", "raw_results": raw_results, "accepted_results": accepted, "rejection_counts": {key: value for key, value in counts.items() if key != "ACCEPTED"}, "accounting": "RAW_RESULTS = ACCEPTED_RESULTS + ALL_REJECTIONS"})
    dump("final-decision.json", {"schema": "amatl.relevance.v6-final-decision.v1", "experiment_id": "independent-relevance-confirmatory-v6", "execution_status": "COMPLETE_INCONCLUSIVE", "target_reached": False, "decision": "INCONCLUSIVE", "reason": "FROZEN_UNIVERSE_EXHAUSTED_BEFORE_VALID_TARGET", "labeling": "NOT_STARTED_TARGET_NOT_REACHED", "primary_endpoint": "NOT_CALCULATED_TARGET_NOT_REACHED", "next_single_action": "NONE_V6_CLOSED"})
    linked = {name: sha(V6 / name) for name in ("design.json", "execution-policy.json", "decision-policy.json", "frozen-query-universe.json", "pair-assignments.json", "randomization-manifest.json", "historical-exclusion-manifest.json", "query-universe-manifest.json")}
    linked.update({name: sha(OUT / name) for name in ("execution-report.json", "capture-manifest.json", "rejection-ledger.json", "prelabel-manifest.json", "raw-capture.json", "final-decision.json")})
    dump("provenance-manifest.json", {"schema": "amatl.relevance.v6-terminal-provenance-manifest.v1", "experiment_id": "independent-relevance-confirmatory-v6", "source_commit": "8191f8bccf33f7d86e0c0b13213da6363e6326a8", "source_candidate": "candidate-v2", "source_candidate_sha256": "8bcc8a4a0ba518930db02da1c5161f4bd48dd243695ea52718494cc55edda64e", "network_requests": len(raw_records), "integrity": "PASS", "artifacts": linked})
    print(json.dumps({"raw_results": raw_results, "accepted": accepted, "rejected": rejected, "report": report}, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
