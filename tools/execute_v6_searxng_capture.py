#!/usr/bin/env python3
"""Execute the frozen V6 SearXNG acquisition and terminalise its evidence."""
from __future__ import annotations

import hashlib
import json
import socket
import subprocess
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit, urlunsplit, urlencode
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
V6 = ROOT / "docs/evaluation/independent-relevance/v6"
OUT = V6 / "execution"
ENDPOINT = "http://127.0.0.1:8888/search"
EXPERIMENT = "independent-relevance-confirmatory-v6"
DEPTH = 3
TARGET = 474


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def dump(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def canonical(value: str) -> str:
    raw = (value or "").strip()
    if any(raw[i] == "%" and (i + 2 >= len(raw) or any(c not in "0123456789abcdefABCDEF" for c in raw[i + 1:i + 3])) for i in range(len(raw)) if raw[i] == "%"):
        raise ValueError("INVALID_RESULT")
    p = urlsplit(raw)
    if p.scheme.lower() not in ("http", "https") or not p.netloc or p.hostname is None:
        raise ValueError("INVALID_RESULT")
    host = p.hostname.lower()
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    port = p.port
    if port is not None and not ((p.scheme.lower() == "http" and port == 80) or (p.scheme.lower() == "https" and port == 443)):
        host += f":{port}"
    userinfo = ""
    if p.username is not None:
        userinfo = p.username
        if p.password is not None:
            userinfo += ":" + p.password
        userinfo += "@"
    kept = []
    for segment in p.query.split("&") if p.query else []:
        key = segment.split("=", 1)[0].lower()
        if key.startswith("utm_") or key in {"fbclid", "gclid", "msclkid", "yclid", "_ga", "_gl", "mc_cid", "mc_eid"}:
            continue
        kept.append(segment)
    return urlunsplit((p.scheme.lower(), userinfo + host, p.path or "/", "&".join(kept), ""))


def historical_urls() -> dict[str, set[str]]:
    paths = {
        "V1": [ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/query-set.json"],
        "V2": [ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication/supplemental-query-universe-v1.json", ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication/supplemental-query-universe-v2.json"],
        "V3": [ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication/supplemental-v3-query-universe.json"],
        "V4": [ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/v4/supplemental-v4-query-universe.json"],
        "V5": [V6.parent / "v5/frozen-query-universe.json"],
        "OTHER_HISTORICAL_OVERLAP": [ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/v4/supplemental-v4-raw-evidence.json", V6.parent / "v5/execution/raw-capture.json"],
    }
    found: dict[str, set[str]] = {key: set() for key in paths}
    def walk(value: object, bucket: set[str]) -> None:
        if isinstance(value, dict):
            for key, item in value.items():
                if key in {"canonical_url", "original_url", "url"} and isinstance(item, str):
                    try:
                        bucket.add(canonical(item))
                    except ValueError:
                        pass
                walk(item, bucket)
        elif isinstance(value, list):
            for item in value:
                walk(item, bucket)
    for category, category_paths in paths.items():
        for path in category_paths:
            walk(json.loads(path.read_text(encoding="utf-8")), found[category])
    return found


def fetch(query: str) -> tuple[int, float, bytes]:
    request = Request(ENDPOINT + "?" + urlencode({"q": query, "format": "json", "pageno": 1}), headers={"Accept": "application/json", "User-Agent": "AMATL-V6-controlled-capture/1.0"})
    started = time.monotonic()
    with urlopen(request, timeout=20) as response:
        return response.status, round((time.monotonic() - started) * 1000, 3), response.read()


def main() -> None:
    if OUT.exists():
        raise SystemExit(f"V6_CAPTURE_STATUS=BLOCKED_PREEXISTING_OUTPUT {OUT}")
    design = json.loads((V6 / "design.json").read_text(encoding="utf-8"))
    policy = json.loads((V6 / "execution-policy.json").read_text(encoding="utf-8"))
    universe = json.loads((V6 / "frozen-query-universe.json").read_text(encoding="utf-8"))
    assignments = json.loads((V6 / "pair-assignments.json").read_text(encoding="utf-8"))["assignments"]
    if design["design_status"] != "FROZEN" or design["provider_set"] != ["SearXNG"] or design["result_depth"] != DEPTH:
        raise SystemExit("V6_CAPTURE_STATUS=BLOCKED_DESIGN")
    if len(universe["queries"]) != 4600 or len(assignments) != 4600 or policy["retry_max_per_query"] != 1:
        raise SystemExit("V6_CAPTURE_STATUS=BLOCKED_UNIVERSE_OR_POLICY")
    start_head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    OUT.mkdir(parents=True)
    raw_dir = OUT / "raw"
    raw_dir.mkdir()
    historical = historical_urls()
    accepted: list[dict] = []
    attempts: list[dict] = []
    ledger: list[dict] = []
    current: set[str] = set()
    counts = Counter()
    queries_executed: list[dict] = []
    for assignment in assignments:
        treatment = sum(row["arm"] == "treatment" for row in accepted)
        control = sum(row["arm"] == "control" for row in accepted)
        if treatment >= TARGET and control >= TARGET:
            break
        queries_executed.append(assignment)
        query_id = assignment["query_id"]
        for attempt_number in (1, 2):
            rec = {"experiment_id": EXPERIMENT, "query_id": query_id, "pair_id": assignment["pair_id"], "arm": assignment["arm"], "execution_sequence": len(queries_executed), "provider": "SearXNG", "query": assignment["query"], "attempt": attempt_number, "timestamp": now(), "provenance": {"frozen_query_universe_sha256": sha(V6 / "frozen-query-universe.json"), "randomization_manifest_sha256": sha(V6 / "randomization-manifest.json")}}
            raw_path = raw_dir / f"{len(queries_executed):04d}-attempt-{attempt_number}.json"
            try:
                status, latency, body = fetch(assignment["query"])
                rec.update({"http_status": status, "latency_ms": latency, "payload_sha256": hashlib.sha256(body).hexdigest()})
                payload = json.loads(body.decode("utf-8"))
                rec["raw_result_count"] = len(payload.get("results", []))
                rec["payload"] = payload
                if status != 200:
                    raise RuntimeError(f"HTTP_{status}")
                results = payload.get("results", [])[:DEPTH]
                rec["results"] = []
                for rank, result in enumerate(results, 1):
                    title = result.get("title") or ""
                    snippet = result.get("content") or result.get("snippet") or result.get("description") or ""
                    original_url = result.get("url") or ""
                    reason = None
                    try:
                        canonical_url = canonical(original_url)
                    except ValueError:
                        canonical_url = None
                        reason = "INVALID_RESULT"
                    if not reason and (not title or not snippet):
                        reason = "INVALID_RESULT"
                    if not reason and canonical_url in current:
                        reason = "DUPLICATE_CURRENT"
                    if not reason:
                        for category in ("V1", "V2", "V3", "V4", "V5", "OTHER_HISTORICAL_OVERLAP"):
                            if canonical_url in historical[category]:
                                reason = "OVERLAP_" + category if category != "OTHER_HISTORICAL_OVERLAP" else category
                                break
                    observed = {"rank": rank, "title": title, "snippet": snippet, "original_url": original_url, "canonical_url": canonical_url, "accepted": reason is None, "rejection_reason": reason}
                    rec["results"].append(observed)
                    if reason:
                        counts[reason] += 1
                    else:
                        row = {"row_id": f"v6-{len(queries_executed):04d}-{rank:02d}", "query_id": query_id, "pair_id": assignment["pair_id"], "arm": assignment["arm"], "provider": "SearXNG", "rank": rank, "original_url": original_url, "canonical_url": canonical_url, "title": title, "snippet": snippet, "execution_sequence": len(queries_executed), "raw_capture_path": str(raw_path.relative_to(ROOT)), "provenance": rec["provenance"]}
                        accepted.append(row)
                        current.add(canonical_url)
                        counts["ACCEPTED"] += 1
                dump(raw_path, rec)
                rec.pop("payload", None)
                rec["raw_capture_path"] = str(raw_path.relative_to(ROOT))
                rec["raw_capture_sha256"] = sha(raw_path)
                attempts.append(rec)
                break
            except (HTTPError, URLError, socket.timeout, TimeoutError, RuntimeError, json.JSONDecodeError, ValueError) as exc:
                if not raw_path.exists():
                    dump(raw_path, {**rec, "error": f"{type(exc).__name__}: {exc}"})
                status = rec.get("http_status")
                retryable = isinstance(exc, (URLError, socket.timeout, TimeoutError, RuntimeError)) and (status is None or status == 429 or status >= 500)
                rec.update({"error": f"{type(exc).__name__}: {exc}", "raw_capture_path": str(raw_path.relative_to(ROOT)), "raw_capture_sha256": sha(raw_path), "retryable": retryable})
                attempts.append(rec)
                counts["TIMEOUT"] += isinstance(exc, (socket.timeout, TimeoutError))
                counts["HTTP_FAILURE"] += status is not None and status != 200
                if attempt_number == 1 and retryable:
                    counts["RETRY"] += 1
                    continue
                break
    treatment = sum(row["arm"] == "treatment" for row in accepted)
    control = sum(row["arm"] == "control" for row in accepted)
    target = treatment >= TARGET and control >= TARGET
    raw = {"schema": "amatl.relevance.v6-raw-capture.v1", "experiment_id": EXPERIMENT, "provider": "SearXNG", "result_depth": DEPTH, "starting_head": start_head, "attempts": attempts, "network_requests": len(attempts)}
    raw_path = OUT / "raw-capture.json"
    dump(raw_path, raw)
    ledger_obj = {"schema": "amatl.relevance.v6-rejection-ledger.v1", "experiment_id": EXPERIMENT, "rejection_counts": {k: v for k, v in counts.items() if k != "ACCEPTED"}, "accepted_count": len(accepted), "attempts": attempts}
    ledger_path = OUT / "rejection-ledger.json"
    dump(ledger_path, ledger_obj)
    prelabel = {"schema": "amatl.relevance.v6-prelabel-manifest.v1", "experiment_id": EXPERIMENT, "freeze_status": "FROZEN_PRELABEL" if target else "NOT_APPLICABLE_TARGET_NOT_REACHED", "rows": sorted(accepted, key=lambda row: row["row_id"]), "raw_capture_sha256": sha(raw_path)}
    prelabel_path = OUT / "prelabel-manifest.json"
    dump(prelabel_path, prelabel)
    decision = "INCONCLUSIVE" if not target else "PENDING_LABELING"
    reason = "FROZEN_UNIVERSE_EXHAUSTED_BEFORE_VALID_TARGET" if not target else "TARGET_REACHED_LABELING_REQUIRED"
    report = {"schema": "amatl.relevance.v6-execution-report.v1", "V6_EXECUTION_STATUS": "COMPLETE_INCONCLUSIVE" if not target else "ACQUISITION_COMPLETE_PENDING_LABELING", "STARTING_HEAD": start_head, "PROVIDER": "SearXNG", "RESULT_DEPTH": DEPTH, "QUERIES_EXECUTED": len(queries_executed), "TREATMENT_QUERIES_EXECUTED": sum(a["arm"] == "treatment" for a in queries_executed), "CONTROL_QUERIES_EXECUTED": sum(a["arm"] == "control" for a in queries_executed), "NETWORK_REQUESTS": len(attempts), "HTTP_SUCCESS": sum(a.get("http_status") == 200 for a in attempts), "HTTP_FAILURE": sum(a.get("http_status") is not None and a.get("http_status") != 200 for a in attempts), "TIMEOUTS": counts["TIMEOUT"], "RETRIES": counts["RETRY"], "RAW_RESULTS": sum(a.get("raw_result_count", 0) if a.get("http_status") == 200 else 0 for a in attempts), "TREATMENT_RAW_RESULTS": sum(a.get("raw_result_count", 0) if a.get("http_status") == 200 and a["arm"] == "treatment" else 0 for a in attempts), "CONTROL_RAW_RESULTS": sum(a.get("raw_result_count", 0) if a.get("http_status") == 200 and a["arm"] == "control" else 0 for a in attempts), "VALID_RESULTS": len(accepted), "TREATMENT_VALID_RESULTS": treatment, "CONTROL_VALID_RESULTS": control, "TARGET_VALID_PER_ARM": TARGET, "TARGET_REACHED": target, "REJECTION_COUNTS": {k: v for k, v in counts.items() if k != "ACCEPTED"}, "PRELABEL_FREEZE": prelabel["freeze_status"], "PRELABEL_ROWS": len(accepted), "PRELABEL_SHA256": sha(prelabel_path), "V6_FINAL_DECISION": decision, "V6_FINAL_REASON": reason}
    report_path = OUT / "execution-report.json"
    dump(report_path, report)
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
