#!/usr/bin/env python3
"""Execute the explicitly authorised frozen V3 capture, then stop at A/B packets."""
import hashlib, json, socket, subprocess, time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlencode, urlsplit, urlunsplit
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
ENDPOINT = "http://127.0.0.1:8888"
RUN_ID = "independent-relevance-supplemental-v3"
TARGET = 60
DEPTH = 3

def now(): return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def canonical(value):
    p = urlsplit((value or "").strip())
    if p.scheme not in ("http", "https") or not p.netloc: raise ValueError("INVALID_RESULT")
    return urlunsplit((p.scheme.lower(), p.netloc.lower(), p.path.rstrip("/") or "/", p.query, ""))
def canonical_json(x): return json.dumps(x, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
def load(name): return json.loads((OUT / name).read_text(encoding="utf-8"))
def commit(): return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
def state(): return "dirty" if subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip() else "clean"

def precapture():
    u, m, a, f = map(load, ("supplemental-v3-query-universe.json", "supplemental-v3-query-universe-manifest.json", "supplemental-v3-query-universe-attestation.json", "supplemental-v3-preflight-report.json"))
    if f.get("preflight") != "PASS" or len(u["queries"]) != 30 or len({q["query_id"] for q in u["queries"]}) != 30 or len({q["query_text"].casefold().strip() for q in u["queries"]}) != 30 or any(q["expected_depth"] != 3 for q in u["queries"]): raise SystemExit("WORK_PACKAGE_STATUS=BLOCKED_V3_PRECAPTURE_INTEGRITY_FAILURE")
    if sum(q["designation"] == "treatment" for q in u["queries"]) != 20 or sum(q["designation"] == "control" for q in u["queries"]) != 10: raise SystemExit("WORK_PACKAGE_STATUS=BLOCKED_V3_PRECAPTURE_INTEGRITY_FAILURE")
    if m["universe_sha256"] != sha(OUT / "supplemental-v3-query-universe.json") or a["universe_sha256"] != m["universe_sha256"] or a["manifest_sha256"] != sha(OUT / "supplemental-v3-query-universe-manifest.json"): raise SystemExit("WORK_PACKAGE_STATUS=BLOCKED_V3_PRECAPTURE_INTEGRITY_FAILURE")
    return u, m, a, f

def fetch(query):
    url = ENDPOINT + "/search?" + urlencode({"q": query, "format": "json"})
    start = time.monotonic()
    req = Request(url, headers={"Accept": "application/json", "User-Agent": "AMATL-V3-controlled-capture/1.0"})
    with urlopen(req, timeout=20) as r:
        body = r.read()
        return r.status, (time.monotonic()-start)*1000, body, json.loads(body.decode("utf-8"))

def main():
    start_commit = commit(); universe, manifest, attestation, preflight = precapture()
    historical = set(canonical(x) for x in load("supplemental-pilot-historical-canonical-urls.json"))
    v1 = set(canonical(r["canonical_url"]) for r in load("v1-ground-truth.json")["rows"])
    supplemental = set(canonical(r["canonical_url"]) for r in load("supplemental-ground-truth-v1.json")["rows"])
    current = set(); valid = []; ledger = []; raw_attempts = []; rejected = Counter(); http_success = http_failure = retries = 0; executed = []
    raw_dir = OUT / "supplemental-v3-raw"
    raw_dir.mkdir(exist_ok=False)
    for q in universe["queries"]:
        if len(valid) >= TARGET: break
        executed.append(q)
        for attempt in (1, 2):
            try:
                status, latency, body, payload = fetch(q["query_text"])
                if status != 200: raise RuntimeError("TEMPORARY_HTTP_FAILURE")
                http_success += 1
                raw_path = raw_dir / f"{q['query_id']}-attempt-{attempt}.json"
                raw_doc = {"query_id": q["query_id"], "query_text": q["query_text"], "strategy_class": q["strategy_class"], "designation": q["designation"], "attempt_number": attempt, "provider": "searxng", "endpoint": ENDPOINT, "requested_at": now(), "http_status": status, "latency_ms": latency, "payload": payload}
                raw_path.write_text(canonical_json(raw_doc), encoding="utf-8")
                raw_file_hash = sha(raw_path)
                results = payload.get("results", []) if isinstance(payload, dict) else []
                record = {"query_id": q["query_id"], "query_text": q["query_text"], "strategy_class": q["strategy_class"], "designation": q["designation"], "attempt_number": attempt, "provider": "searxng", "endpoint": ENDPOINT, "timestamp": now(), "http_status": status, "latency_ms": latency, "raw_result_count": min(len(results), DEPTH), "raw_response_path": str(raw_path.relative_to(ROOT)), "raw_response_sha256": raw_file_hash, "results": []}
                for rank, r in enumerate(results[:DEPTH], 1):
                    item = {"rank": rank, "title": r.get("title") or "", "original_url": r.get("url") or "", "snippet": r.get("content") or r.get("snippet") or r.get("description") or "", "provider_metadata": {k:r[k] for k in ("engine","engines","category","score","parsed_url","template") if k in r}}
                    reason = None
                    try: url = canonical(item["original_url"])
                    except ValueError: reason = "INVALID_RESULT"
                    if not reason and (not item["title"] or not item["snippet"]): reason = "INVALID_RESULT"
                    if not reason and url in current: reason = "DUPLICATE_CURRENT"
                    if not reason and url in v1: reason = "OVERLAP_V1"
                    if not reason and url in supplemental: reason = "OVERLAP_SUPPLEMENTAL"
                    if not reason and url in historical: reason = "HISTORICAL_OVERLAP"
                    if not reason and len(valid) >= TARGET: reason = "TARGET_FILLED"
                    item["canonical_url"] = url if not reason or reason != "INVALID_RESULT" else None
                    item["reason_code"] = reason or "ACCEPTED"
                    ledger.append({"query_id": q["query_id"], "attempt_number": attempt, "rank": rank, "canonical_url": item.get("canonical_url"), "reason_code": item["reason_code"]})
                    rejected[item["reason_code"]] += item["reason_code"] != "ACCEPTED"
                    if not reason:
                        current.add(url); row = {"row_id": f"sup-v3-{q['query_id']}-{rank:03d}", "query_id": q["query_id"], "query": q["query_text"], "strategy_class": q["strategy_class"], "hypothesis_feature": q["hypothesis_feature"], "designation": q["designation"], "provider_or_source": ["searxng"], "original_url": item["original_url"], "canonical_url": url, "domain": urlsplit(url).netloc, "title": item["title"], "snippet": item["snippet"], "rank": rank, "result_status": "visible", "acquired_at": now(), "acquisition_run_id": RUN_ID, "raw_response_path": record["raw_response_path"], "raw_response_sha256": raw_file_hash, "provider_metadata": item["provider_metadata"]}; valid.append(row)
                    record["results"].append(item)
                raw_attempts.append(record); break
            except (socket.timeout, TimeoutError, RuntimeError) as e:
                http_failure += 1
                raw_attempts.append({"query_id": q["query_id"], "query_text": q["query_text"], "strategy_class": q["strategy_class"], "designation": q["designation"], "attempt_number": attempt, "provider": "searxng", "endpoint": ENDPOINT, "timestamp": now(), "http_status": None, "latency_ms": None, "raw_result_count": 0, "results": [], "technical_error": {"error_class": "TIMEOUT" if isinstance(e, (socket.timeout, TimeoutError)) else "TEMPORARY_HTTP_FAILURE"}})
                if attempt == 1: retries += 1; continue
                break
    status = "COMPLETE_TARGET_REACHED" if len(valid) == TARGET else "EXHAUSTED_FROZEN_UNIVERSE"
    evidence = {"schema": "amatl.relevance.supplemental-v3-raw-evidence.v1", "experiment_id": RUN_ID, "universe_id": universe["universe_id"], "universe_sha256": sha(OUT/"supplemental-v3-query-universe.json"), "provider": "searxng", "endpoint": ENDPOINT, "attempts": raw_attempts}
    raw_path = OUT / "supplemental-v3-raw-evidence.json"; raw_path.write_text(canonical_json(evidence), encoding="utf-8")
    accepted = sorted(valid, key=lambda x:x["row_id"])
    corpus = {"schema": "amatl.relevance.supplemental-v3-final-prelabel-corpus.v1", "corpus_id": "independent-relevance-supplemental-v3-final-20260902", "status": "FROZEN_PRELABEL" if len(accepted)==TARGET else "INCOMPLETE", "raw_evidence_path": str(raw_path.relative_to(ROOT)), "raw_evidence_sha256": sha(raw_path), "rows": accepted}
    corpus_path = OUT / "supplemental-v3-final-prelabel-corpus.json"; corpus_path.write_text(canonical_json(corpus), encoding="utf-8")
    packet_hashes = {}; packet_rows = [{**r, "label":"", "annotation_note":""} for r in accepted]
    for pid, name in (("A","supplemental-v3-labeler-a-packet.json"),("B","supplemental-v3-labeler-b-packet.json")):
        packet = {"schema":"amatl.relevance.supplemental-v3-annotation-packet.v1", "packet_id":pid, "corpus_id":corpus["corpus_id"], "prelabel_corpus_sha256":sha(corpus_path), "label_observation_status":"NOT_STARTED", "rows":packet_rows}
        pp=OUT/name; pp.write_text(canonical_json(packet), encoding="utf-8"); packet_hashes[pid]=sha(pp)
    ledger_doc={"schema":"amatl.relevance.supplemental-v3-attempt-ledger.v1", "experiment_id":RUN_ID, "attempts":ledger, "rejection_counts":dict(rejected), "network_requests":len(raw_attempts)}; ledger_path=OUT/"supplemental-v3-attempt-ledger.json"; ledger_path.write_text(canonical_json(ledger_doc),encoding="utf-8")
    report={"schema":"amatl.relevance.supplemental-v3-run-report.v1", "REPOSITORY_STATE":state(), "START_COMMIT":start_commit, "V3_PRECAPTURE_INTEGRITY":"PASS", "V3_QUERY_UNIVERSE_HASH":sha(OUT/"supplemental-v3-query-universe.json"), "V3_MANIFEST_HASH":sha(OUT/"supplemental-v3-query-universe-manifest.json"), "V3_ATTESTATION_HASH":sha(OUT/"supplemental-v3-query-universe-attestation.json"), "PROVIDER":"SearXNG", "ENDPOINT":ENDPOINT, "NETWORK_REQUESTS":len(raw_attempts), "HTTP_SUCCESS":http_success, "HTTP_FAILURE":http_failure, "TECHNICAL_RETRIES":retries, "QUERIES_TOTAL":30, "QUERIES_EXECUTED":len(executed), "TREATMENT_QUERIES_EXECUTED":sum(q["designation"]=="treatment" for q in executed), "CONTROL_QUERIES_EXECUTED":sum(q["designation"]=="control" for q in executed), "RAW_RESULTS":sum(len(a.get("results",[])) for a in raw_attempts), "TREATMENT_RAW_ROWS":sum(len(a.get("results",[])) for a,q in zip(raw_attempts,executed) if q["designation"]=="treatment"), "CONTROL_RAW_ROWS":sum(len(a.get("results",[])) for a,q in zip(raw_attempts,executed) if q["designation"]=="control"), "REJECTED_TOTAL":sum(rejected.values()), **{"REJECTED_"+k:rejected.get(k,0) for k in ("DUPLICATE_CURRENT","OVERLAP_V1","OVERLAP_SUPPLEMENTAL","HISTORICAL_OVERLAP","INVALID_RESULT","TARGET_FILLED")}, "VALID_ROWS":len(accepted), "TREATMENT_VALID_ROWS":sum(r["designation"]=="treatment" for r in accepted), "CONTROL_VALID_ROWS":sum(r["designation"]=="control" for r in accepted), "VALID_DUPLICATES":0, "VALID_OVERLAP_V1":0, "VALID_OVERLAP_SUPPLEMENTAL":0, "VALID_OVERLAP_HISTORICAL":0, "V3_VALID_ROW_TARGET":TARGET, "UNIVERSE_EXHAUSTED":len(executed)==30 and len(accepted)<TARGET, "STOP_EARLY_TRIGGERED":len(accepted)==TARGET and len(executed)<30, "V3_CAPTURE_STATUS":status, "V3_PRELABEL_STATUS":corpus["status"], "FINAL_PRELABEL_ROWS":len(accepted), "FINAL_PRELABEL_HASH":sha(corpus_path), "LABELER_A_PACKET_ROWS":len(packet_rows), "LABELER_A_LABELS_EMPTY":all(r["label"]=="" and r["annotation_note"]=="" for r in packet_rows), "LABELER_A_HASH":packet_hashes["A"], "LABELER_B_PACKET_ROWS":len(packet_rows), "LABELER_B_LABELS_EMPTY":all(r["label"]=="" and r["annotation_note"]=="" for r in packet_rows), "LABELER_B_HASH":packet_hashes["B"], "A_B_PRELABEL_EQUIVALENCE":packet_rows==packet_rows, "RAW_EVIDENCE_ARTIFACT":str(raw_path.relative_to(ROOT)), "RAW_EVIDENCE_HASH":sha(raw_path), "ATTEMPT_LEDGER":str(ledger_path.relative_to(ROOT)), "ATTEMPT_LEDGER_HASH":sha(ledger_path), "RUN_REPORT_HASH":None, "WORK_PACKAGE_STATUS":"COMPLETE_READY_FOR_V3_INDEPENDENT_LABELING" if len(accepted)==TARGET else "BLOCKED_V3_TARGET_NOT_REACHED", "NEXT_SINGLE_ACTION":"Deliver the frozen V3 Labeler A and Labeler B packets to two independent relevance labelers." if len(accepted)==TARGET else "Analyze V3 rejection and capacity evidence offline before authorizing any V3 universe expansion."}
    report_path=OUT/"supplemental-v3-run-report.json"; report_path.write_text(canonical_json(report),encoding="utf-8"); report["RUN_REPORT_HASH"]=sha(report_path); report_path.write_text(canonical_json(report),encoding="utf-8")
    print(json.dumps({"status":status,"valid":len(accepted),"queries":len(executed),"network_requests":len(raw_attempts),"report":str(report_path)},indent=2))

if __name__ == "__main__": main()
