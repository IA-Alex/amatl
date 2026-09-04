#!/usr/bin/env python3
"""Execute the frozen V5 SearXNG acquisition and freeze its prelabel pool."""
from __future__ import annotations

import hashlib, json, socket, time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit, urlunsplit, urlencode
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
V5 = ROOT / "docs/evaluation/independent-relevance/v5"
OUT = V5 / "execution"
ENDPOINT = "http://127.0.0.1:8888/search"
TARGET = 474
DEPTH = 3

def now(): return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def dump(p, x): p.write_text(json.dumps(x, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
def canonical(value):
    p = urlsplit((value or "").strip())
    if p.scheme not in ("http", "https") or not p.netloc: raise ValueError("INVALID_RESULT")
    return urlunsplit((p.scheme.lower(), p.netloc.lower(), p.path.rstrip("/") or "/", p.query, ""))

def historical_urls():
    groups = {"V1": [], "V2": [], "V3": [], "V4": []}
    base = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1"
    for path in base.rglob("*.json"):
        n = path.name.lower()
        if "/v4/" in str(path): group = "V4"
        elif "supplemental-v3" in n or "/v3/" in str(path): group = "V3"
        elif "supplemental-v2" in n or "/v2/" in str(path): group = "V2"
        else: group = "V1"
        try: obj = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError): continue
        def walk(x):
            if isinstance(x, dict):
                for k, v in x.items():
                    if k in {"canonical_url", "original_url"} and isinstance(v, str):
                        try: groups[group].append(canonical(v))
                        except ValueError: pass
                    walk(v)
            elif isinstance(x, list):
                for v in x: walk(v)
        walk(obj)
    return {k: set(v) for k, v in groups.items()}

def fetch(query):
    request = Request(ENDPOINT + "?" + urlencode({"q": query, "format": "json", "pageno": 1}),
                      headers={"Accept": "application/json", "User-Agent": "AMATL-V5-controlled-capture/1.0"})
    start = time.monotonic()
    with urlopen(request, timeout=20) as response:
        body = response.read()
        return response.status, round((time.monotonic() - start) * 1000, 3), body

def main():
    if OUT.exists(): raise SystemExit(f"V5_CAPTURE_STATUS=BLOCKED_PREEXISTING_OUTPUT {OUT}")
    design = json.loads((V5 / "frozen-design.json").read_text())
    universe = json.loads((V5 / "frozen-query-universe.json").read_text())
    assignments = json.loads((V5 / "pair-assignments.json").read_text())["assignments"]
    if design["design_status"] != "FROZEN" or design["provider"] != "SearXNG" or design["result_depth"] != DEPTH: raise SystemExit("V5_CAPTURE_STATUS=BLOCKED_DESIGN")
    if len(assignments) != 2172 or len(universe["pairs"]) != 1086: raise SystemExit("V5_CAPTURE_STATUS=BLOCKED_UNIVERSE")
    if [a["capture_order"] for a in assignments] != list(range(1, 2173)): raise SystemExit("V5_CAPTURE_STATUS=BLOCKED_ORDER")
    historical = historical_urls()
    OUT.mkdir(parents=True)
    raw_dir = OUT / "raw"; raw_dir.mkdir()
    accepted, ledger, attempts = [], [], []
    current = set(); counts = Counter(); queries_executed = []
    start_head = __import__("subprocess").check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    for assignment in assignments:
        if sum(r["arm"] == "treatment" for r in accepted) >= TARGET and sum(r["arm"] == "control" for r in accepted) >= TARGET: break
        queries_executed.append(assignment)
        query_id = f"{assignment['pair_id']}-{assignment['arm']}"
        for attempt in (1, 2):
            rec = {"pair_id": assignment["pair_id"], "arm": assignment["arm"], "query_id": query_id,
                   "query": assignment["query"], "provider": "SearXNG", "endpoint": ENDPOINT, "depth": DEPTH,
                   "attempt": attempt, "timestamp": now()}
            try:
                status, latency, body = fetch(assignment["query"])
                rec.update({"http_status": status, "latency_ms": latency, "payload_sha256": hashlib.sha256(body).hexdigest()})
                if status != 200: raise RuntimeError(f"HTTP_{status}")
                payload = json.loads(body.decode("utf-8")); rec["raw_result_count"] = min(len(payload.get("results", [])), DEPTH)
                raw_path = raw_dir / f"{assignment['capture_order']:04d}-attempt-{attempt}.json"
                dump(raw_path, {**rec, "payload": payload})
                rec["raw_response_path"] = str(raw_path.relative_to(ROOT)); rec["raw_response_sha256"] = sha(raw_path)
                rec["results"] = []
                for rank, result in enumerate(payload.get("results", [])[:DEPTH], 1):
                    item = {"rank": rank, "title": result.get("title") or "", "content": result.get("content") or result.get("snippet") or result.get("description") or "", "original_url": result.get("url") or ""}
                    reason = None; url = None
                    try: url = canonical(item["original_url"])
                    except ValueError: reason = "INVALID_RESULT"
                    if not reason and (not item["title"] or not item["content"]): reason = "INVALID_RESULT"
                    if not reason and url in current: reason = "DUPLICATE_CURRENT"
                    if not reason:
                        for label in ("V1", "V2", "V3", "V4"):
                            if url in historical[label]: reason = "OVERLAP_" + label; break
                    if not reason and url in {r["canonical_url"] for r in accepted if r["arm"] != assignment["arm"]}: reason = "CROSS_ARM_CONTAMINATION"
                    if reason: counts[reason] += 1
                    else:
                        row = {"row_id": f"v5-{assignment['capture_order']:04d}-{rank:02d}", "pair_id": assignment["pair_id"], "arm": assignment["arm"], "query_id": query_id, "query": assignment["query"], "provider": "SearXNG", "provider_or_source": ["SearXNG"], "original_url": item["original_url"], "canonical_url": url, "domain": urlsplit(url).netloc, "title": item["title"], "snippet": item["content"], "rank": rank, "result_status": "visible", "acquired_at": now(), "acquisition_run_id": "independent-relevance-confirmatory-v5", "raw_response_path": rec["raw_response_path"], "raw_response_sha256": rec["raw_response_sha256"]}
                        accepted.append(row); current.add(url); counts["ACCEPTED"] += 1
                    rec["results"].append({**item, "canonical_url": url, "rejection_reason": reason})
                attempts.append(rec); break
            except (HTTPError, URLError, socket.timeout, TimeoutError, RuntimeError, json.JSONDecodeError) as exc:
                status = getattr(exc, "code", None); retryable = isinstance(exc, (URLError, socket.timeout, TimeoutError, RuntimeError)) and (status is None or status == 429 or status >= 500)
                rec.update({"http_status": status, "error": type(exc).__name__ + ": " + str(exc), "results": []}); attempts.append(rec)
                counts["HTTP_FAILURE"] += 1
                if attempt == 1 and retryable: counts["RETRY"] += 1; continue
                break
    treatment = sum(r["arm"] == "treatment" for r in accepted); control = sum(r["arm"] == "control" for r in accepted)
    raw = {"schema":"amatl.relevance.v5-raw-capture.v1", "experiment_id":"independent-relevance-confirmatory-v5", "provider":"SearXNG", "endpoint":ENDPOINT, "depth":DEPTH, "starting_head":start_head, "attempts":attempts, "network_requests":len(attempts)}
    raw_path = OUT / "raw-capture.json"; dump(raw_path, raw)
    prelabel = {"schema":"amatl.relevance.v5-prelabel-freeze.v1", "experiment_id":"independent-relevance-confirmatory-v5", "freeze_status":"FROZEN_PRELABEL" if treatment >= TARGET and control >= TARGET else "INCOMPLETE", "rows":sorted(accepted, key=lambda r:r["row_id"]), "raw_capture_sha256":sha(raw_path)}
    prelabel_path = OUT / "prelabel-freeze.json"; dump(prelabel_path, prelabel)
    packet_rows = [{k:r[k] for k in ("row_id","query","original_url","canonical_url","domain","title","snippet","rank")} | {"label":"","annotation_note":""} for r in prelabel["rows"]]
    for labeler in ("A", "B"): dump(OUT / f"annotation-packet-{labeler}.json", {"schema":"amatl.relevance.v5-annotation-packet.v1", "packet_id":labeler, "prelabel_sha256":sha(prelabel_path), "label_observation_status":"NOT_STARTED", "rubric":"docs/evaluation/step4e/annotation_rubric.md", "rows":packet_rows})
    dump(OUT / "attempt-ledger.json", {"schema":"amatl.relevance.v5-attempt-ledger.v1", "attempts":[{k:v for k,v in a.items() if k not in {"payload"}} for a in attempts], "rejection_counts":dict(counts)})
    report = {"schema":"amatl.relevance.v5-execution-report.v1", "STARTING_HEAD":start_head, "PROVIDER":"SearXNG", "NETWORK_REQUESTS":len(attempts), "HTTP_SUCCESS":sum(a.get("http_status")==200 for a in attempts), "HTTP_FAILURE":sum(a.get("http_status")!=200 for a in attempts), "QUERIES_EXECUTED":len(queries_executed), "QUERIES_AVAILABLE":2172, "TREATMENT_QUERIES_EXECUTED":sum(a["arm"]=="treatment" for a in queries_executed), "CONTROL_QUERIES_EXECUTED":sum(a["arm"]=="control" for a in queries_executed), "RAW_RESULTS":sum(len(a.get("results",[])) for a in attempts), "VALID_RESULTS":len(accepted), "TREATMENT_VALID_RESULTS":treatment, "CONTROL_VALID_RESULTS":control, "TARGET_REACHED":treatment>=TARGET and control>=TARGET, "TARGET_VALID_PER_ARM":TARGET, "REJECTION_COUNTS":dict(counts), "PRELABEL_FREEZE":prelabel["freeze_status"], "PRELABEL_ROWS":len(accepted), "PRELABEL_SHA256":sha(prelabel_path), "LABELING_STATUS":"WAITING_FOR_INDEPENDENT_HUMAN_LABELS"}
    dump(OUT / "execution-report.json", report)
    print(json.dumps(report, indent=2, sort_keys=True))

if __name__ == "__main__": main()
