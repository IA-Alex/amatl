#!/usr/bin/env python3
"""Execute the frozen confirmatory V4 capture exactly per the frozen contract.

Direct inputs (read-only, never regenerated):
  - frozen query universe (query_id, query_text, designation, capture_order,
    permitted_positions, strategy_class, matched_topic)
  - frozen randomization/assignment: the universe is already shuffled by the
    frozen seed; this executor preserves capture_order ascending and never
    re-randomizes.
  - frozen treatment/control definitions: query text comes verbatim from the
    frozen universe file.
  - frozen provider: SEARXNG at the design's expected endpoint.
  - RESULT_DEPTH=3 (permitted_positions 1..3).
  - A/B packets are arm-blinded per the frozen design: packet rows carry only
    the shared non-arm-encoding row_id plus query/result fields needed for
    labeling; designation, strategy_class, hypothesis_feature, query_id and all
    acquisition metadata are omitted from packet rows.

Forbidden paths (asserted by design and by this implementation):
  no re-randomization, no query regeneration, no treatment/control mutation,
  no relevance-based filtering, no label inspection, no metric-based stopping.
  Stopping is only on (a) both arms reaching the frozen per-arm target of 474
  valid results or (b) exhaustion of the frozen universe in capture_order.
"""
from __future__ import annotations

import hashlib
import json
import socket
import subprocess
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlencode, urlsplit, urlunsplit
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/v4"
ADJ = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
ENDPOINT = "http://127.0.0.1:8888"
RUN_ID = "independent-relevance-supplemental-v4"
UNIVERSE_ID = "independent-relevance-supplemental-v4"
TARGET_PER_ARM = 474
TARGET_TOTAL = 948
DEPTH = 3
PROVIDER = "searxng"
CORPUS_ID = "independent-relevance-supplemental-v4-final-20260902"

DESIGN = "supplemental-v4-confirmatory-design.json"
UNIVERSE = "supplemental-v4-query-universe.json"
MANIFEST = "supplemental-v4-query-universe-manifest.json"
ATTESTATION = "supplemental-v4-query-universe-attestation.json"
PREFLIGHT = "supplemental-v4-preflight-offline-report.json"

RAW_EVIDENCE = "supplemental-v4-raw-evidence.json"
ATTEMPT_LEDGER = "supplemental-v4-attempt-ledger.json"
PRELABEL_CORPUS = "supplemental-v4-final-prelabel-corpus.json"
PRELABEL_MANIFEST = "supplemental-v4-prelabel-manifest.json"
PRELABEL_ATTESTATION = "supplemental-v4-prelabel-attestation.json"
RUN_REPORT = "supplemental-v4-run-report.json"
PACKET_A = "supplemental-v4-labeler-a-packet.json"
PACKET_B = "supplemental-v4-labeler-b-packet.json"
PACKET_MANIFEST = "supplemental-v4-packet-manifest.json"


def now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical(value: str) -> str:
    p = urlsplit((value or "").strip())
    if p.scheme not in ("http", "https") or not p.netloc:
        raise ValueError("INVALID_RESULT")
    return urlunsplit((p.scheme.lower(), p.netloc.lower(), p.path.rstrip("/") or "/", p.query, ""))


def canonical_json(x: object) -> str:
    return json.dumps(x, ensure_ascii=False, sort_keys=True, indent=2) + "\n"


def load(name: str) -> dict:
    return json.loads((OUT / name).read_text(encoding="utf-8"))


def load_adj(name: str) -> object:
    return json.loads((ADJ / name).read_text(encoding="utf-8"))


def commit() -> str:
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()


def state() -> str:
    return "dirty" if subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip() else "clean"


def fetch(query: str):
    url = ENDPOINT + "/search?" + urlencode({"q": query, "format": "json"})
    start = time.monotonic()
    req = Request(url, headers={"Accept": "application/json", "User-Agent": "AMATL-V4-controlled-capture/1.0"})
    with urlopen(req, timeout=20) as r:
        body = r.read()
        return r.status, (time.monotonic() - start) * 1000, body


def precapture():
    """Full frozen-contract integrity gate. Exits BLOCKED on any failure."""
    d = load(DESIGN)
    u = load(UNIVERSE)
    m = load(MANIFEST)
    a = load(ATTESTATION)
    f = load(PREFLIGHT)
    errors = []

    def check(cond, label):
        if not cond:
            errors.append(label)

    check(d.get("design_status") == "FROZEN", "DESIGN_STATUS")
    check(d.get("experiment_type") == "CONFIRMATORY", "EXPERIMENT_TYPE")
    check(d.get("primary_endpoint") == "STRICT_RELEVANCE_YIELD", "PRIMARY_ENDPOINT")
    check(d.get("alpha") == 0.05, "ALPHA")
    check(d.get("power_target") == 0.9, "POWER_TARGET")
    check(d.get("sample_size", {}).get("frozen_n_per_arm") == 474, "FROZEN_N_PER_ARM")
    check(d.get("sample_size", {}).get("total_valid_result_target") == 948, "TOTAL_TARGET")
    check(d.get("result_depth") == 3, "RESULT_DEPTH")
    check(d.get("provider") == "SEARXNG", "PROVIDER")
    check(d.get("endpoint_expected") == ENDPOINT, "ENDPOINT")
    check(d.get("randomization", {}).get("seed") == "independent-relevance-supplemental-v4-randomization-v1", "SEED")
    check(d.get("randomization", {}).get("allocation_ratio") == "1:1", "ALLOCATION_RATIO")
    check(d.get("stopping_rule", "").startswith("Capture in frozen capture_order until exactly 474 valid results per arm"), "STOPPING_RULE")
    check(d.get("network_requests") == 0, "DESIGN_NETWORK_REQUESTS")

    qs = u.get("queries", [])
    check(u.get("universe_id") == UNIVERSE_ID, "UNIVERSE_ID")
    check(u.get("freeze_status") == "FROZEN", "UNIVERSE_FREEZE_STATUS")
    check(len(qs) == 600, "QUERY_COUNT")
    check(len({q["query_id"] for q in qs}) == 600, "QUERY_ID_UNIQUE")
    check(len({q["query_text"].strip().casefold() for q in qs}) == 600, "QUERY_TEXT_UNIQUE")
    check(sum(q["designation"] == "treatment" for q in qs) == 300, "TREATMENT_COUNT")
    check(sum(q["designation"] == "control" for q in qs) == 300, "CONTROL_COUNT")
    check(all(q["expected_depth"] == 3 for q in qs), "EXPECTED_DEPTH")
    check(sorted(q["capture_order"] for q in qs) == list(range(1, 601)), "CAPTURE_ORDER")
    check(all(q["permitted_positions"] == [1, 2, 3] for q in qs), "PERMITTED_POSITIONS")

    check(m.get("universe_sha256") == sha(OUT / UNIVERSE), "MANIFEST_UNIVERSE_HASH")
    check(a.get("universe_sha256") == sha(OUT / UNIVERSE), "ATTESTATION_UNIVERSE_HASH")
    check(a.get("manifest_sha256") == sha(OUT / MANIFEST), "ATTESTATION_MANIFEST_HASH")
    check(f.get("preflight") == "PASS", "PREFLIGHT_STATUS")
    check(f.get("hash_chain", {}).get("universe_sha256") == sha(OUT / UNIVERSE), "PREFLIGHT_UNIVERSE_HASH")
    check(f.get("hash_chain", {}).get("manifest_sha256") == sha(OUT / MANIFEST), "PREFLIGHT_MANIFEST_HASH")
    check(f.get("hash_chain", {}).get("attestation_sha256") == sha(OUT / ATTESTATION), "PREFLIGHT_ATTESTATION_HASH")
    check(f.get("hash_chain", {}).get("design_sha256") == sha(OUT / DESIGN), "PREFLIGHT_DESIGN_HASH")

    if errors:
        raise SystemExit("V4_CAPTURE_STATUS=BLOCKED PRECAPTURE_INTEGRITY_FAILURE: " + ",".join(errors))
    return d, u, m, a, f


class V4Accumulator:
    """Pure contract gate for V4 validity; no network, no labels, no relevance."""

    def __init__(self, universe: dict, v1_urls, supplemental_urls, v3_urls, historical_urls):
        self.universe = universe
        self.queries = {q["query_id"]: q for q in universe["queries"]}
        self.v1_urls = v1_urls
        self.supplemental_urls = supplemental_urls
        self.v3_urls = v3_urls
        self.historical_urls = historical_urls
        self.current_urls = set()
        self.accepted = []
        self.ledger = []
        self.rejected = Counter()

    def evaluate(self, query_id: str, rank: int, title: str, snippet: str, original_url: str):
        """Return (reason_or_None, canonical_url_or_None). None reason == ACCEPTED."""
        query = self.queries.get(query_id)
        reason = None
        url = None
        if query is None:
            reason = "OUTSIDE_FROZEN_UNIVERSE"
        elif rank not in query["permitted_positions"]:
            reason = "OUTSIDE_ALLOWED_DEPTH"
        else:
            try:
                url = canonical(original_url)
            except ValueError:
                reason = "INVALID_RESULT"
            if not reason and (not title or not snippet):
                reason = "INVALID_RESULT"
            if not reason and url in self.current_urls:
                reason = "DUPLICATE_CURRENT"
            if not reason and url in self.v1_urls:
                reason = "OVERLAP_V1"
            if not reason and url in self.supplemental_urls:
                reason = "OVERLAP_SUPPLEMENTAL"
            if not reason and url in self.v3_urls:
                reason = "OVERLAP_V3"
            if not reason and url in self.historical_urls:
                reason = "HISTORICAL_OVERLAP"
            if not reason:
                arm = query["designation"]
                count = sum(1 for r in self.accepted if r["designation"] == arm)
                if count >= TARGET_PER_ARM:
                    reason = "TREATMENT_TARGET_FILLED" if arm == "treatment" else "CONTROL_TARGET_FILLED"
        return reason, url

    def accept(self, query_id, rank, title, snippet, original_url, extra):
        reason, url = self.evaluate(query_id, rank, title, snippet, original_url)
        if reason is not None:
            self.ledger.append({"query_id": query_id, "attempt_number": extra.get("attempt_number"),
                                "rank": rank, "canonical_url": url, "reason_code": reason})
            self.rejected[reason] += 1
            return None
        query = self.queries[query_id]
        self.current_urls.add(url)
        row = {
            # row_id is a non-arm-encoding deterministic id shared by the corpus
            # and the blinded packet rows (capture_order is the frozen global
            # 1..600 index, not an arm marker; rank is 1..3).
            "row_id": f"sup-v4-q{query['capture_order']:03d}-{rank:03d}",
            "query_id": query_id,
            "query": query["query_text"],
            "strategy_class": query["strategy_class"],
            "designation": query["designation"],
            "provider_or_source": [PROVIDER],
            "original_url": original_url,
            "canonical_url": url,
            "domain": urlsplit(url).netloc,
            "title": title,
            "snippet": snippet,
            "rank": rank,
            "result_status": "visible",
            "acquired_at": extra["acquired_at"],
            "acquisition_run_id": RUN_ID,
            "raw_response_path": extra["raw_response_path"],
            "raw_response_sha256": extra["raw_response_sha256"],
            "provider_metadata": extra.get("provider_metadata") or {},
        }
        self.accepted.append(row)
        self.ledger.append({"query_id": query_id, "attempt_number": extra.get("attempt_number"),
                            "rank": rank, "canonical_url": url, "reason_code": "ACCEPTED"})
        return row


def exclusion_sets():
    historical = set(canonical(x) for x in load_adj("supplemental-pilot-historical-canonical-urls.json"))
    v1 = set(canonical(r["canonical_url"]) for r in load_adj("v1-ground-truth.json")["rows"])
    supplemental = set(canonical(r["canonical_url"]) for r in load_adj("supplemental-ground-truth-v1.json")["rows"])
    v3 = set(canonical(r["canonical_url"]) for r in load_adj("supplemental-v3-ground-truth-v1.json")["rows"])
    return v1, supplemental, v3, historical


def build_evidence_doc(universe, raw_attempts):
    return {"schema": "amatl.relevance.supplemental-v4-raw-evidence.v1", "experiment_id": RUN_ID,
            "universe_id": universe["universe_id"], "universe_sha256": sha(OUT / UNIVERSE),
            "provider": PROVIDER, "endpoint": ENDPOINT, "attempts": raw_attempts}


def write_packets(corpus, corpus_path):
    """Write blinded, unlabeled A/B packets plus the packet manifest.

    Blinding follows the frozen design: packet rows expose only the shared
    non-arm-encoding row_id and the query/result fields needed for labeling.
    designation, strategy_class, hypothesis_feature, query_id, provider_or_source
    and all acquisition metadata are omitted so the packets do not reveal the arm.
    """
    packet_hashes = {}

    def blinded(row):
        return {
            "row_id": row["row_id"],
            "query": row["query"],
            "rank": row["rank"],
            "title": row["title"],
            "snippet": row["snippet"],
            "original_url": row["original_url"],
            "canonical_url": row["canonical_url"],
            "domain": row["domain"],
            "label": "",
            "annotation_note": "",
        }

    packet_rows = [blinded(r) for r in corpus["rows"]]
    for pid, name in (("A", PACKET_A), ("B", PACKET_B)):
        packet = {"schema": "amatl.relevance.supplemental-v4-annotation-packet.v1", "packet_id": pid,
                  "corpus_id": corpus["corpus_id"], "prelabel_corpus_sha256": sha(corpus_path),
                  "label_observation_status": "NOT_STARTED", "rows": packet_rows}
        pp = OUT / name
        pp.write_text(canonical_json(packet), encoding="utf-8")
        packet_hashes[pid] = sha(pp)
    manifest = {"schema": "amatl.relevance.supplemental-v4-packet-manifest.v1", "corpus_id": corpus["corpus_id"],
                "prelabel_corpus_sha256": sha(corpus_path), "packet_rows": len(packet_rows),
                "packet_hashes": packet_hashes, "label_observation_status": "NOT_STARTED",
                "blinding": "arm-blinded packet rows: designation, strategy_class, hypothesis_feature, "
                            "query_id, provider_or_source and all acquisition metadata omitted; only the "
                            "shared non-arm-encoding row_id plus query/result fields needed for labeling retained",
                "created_at": now(), "network_requests": 0}
    (OUT / PACKET_MANIFEST).write_text(canonical_json(manifest), encoding="utf-8")
    return manifest


def main():
    if state() != "clean":
        raise SystemExit("V4_CAPTURE_STATUS=BLOCKED DIRTY_TREE")
    start_commit = commit()
    design, universe, manifest, attestation, preflight = precapture()
    v1, supplemental, v3, historical = exclusion_sets()
    acc = V4Accumulator(universe, v1, supplemental, v3, historical)

    # Operational preflight: refuse any pre-existing V4 capture output.
    raw_dir = OUT / "supplemental-v4-raw"
    for existing in (raw_dir, OUT / RAW_EVIDENCE, OUT / ATTEMPT_LEDGER, OUT / PRELABEL_CORPUS,
                     OUT / PRELABEL_MANIFEST, OUT / PRELABEL_ATTESTATION, OUT / RUN_REPORT,
                     OUT / PACKET_A, OUT / PACKET_B, OUT / PACKET_MANIFEST):
        if existing.exists():
            raise SystemExit("V4_CAPTURE_STATUS=BLOCKED PREEXISTING_V4_OUTPUT " + str(existing))
    raw_dir.mkdir()

    raw_attempts = []
    executed = []
    http_success = http_failure = retries = 0
    for q in sorted(universe["queries"], key=lambda x: x["capture_order"]):
        treatment_count = sum(1 for r in acc.accepted if r["designation"] == "treatment")
        control_count = sum(1 for r in acc.accepted if r["designation"] == "control")
        if treatment_count >= TARGET_PER_ARM and control_count >= TARGET_PER_ARM:
            break
        executed.append(q)
        for attempt in (1, 2):
            try:
                status, latency, body = fetch(q["query_text"])
                if status != 200:
                    raise RuntimeError("TEMPORARY_HTTP_FAILURE")
                http_success += 1
                try:
                    payload = json.loads(body.decode("utf-8"))
                except (ValueError, UnicodeDecodeError) as e:
                    # Frozen contract: parser failure is "record and reject attempt"
                    # (no retry), unlike HTTP failure/provider timeout.
                    raw_attempts.append({"query_id": q["query_id"], "query_text": q["query_text"],
                                         "strategy_class": q["strategy_class"], "designation": q["designation"],
                                         "attempt_number": attempt, "provider": PROVIDER, "endpoint": ENDPOINT,
                                         "timestamp": now(), "http_status": status, "latency_ms": latency,
                                         "raw_result_count": 0, "results": [],
                                         "technical_error": {"error_class": "PARSER_FAILURE", "detail": str(e)}})
                    break
                raw_path = raw_dir / f"{q['query_id']}-attempt-{attempt}.json"
                raw_doc = {"query_id": q["query_id"], "query_text": q["query_text"],
                           "strategy_class": q["strategy_class"], "designation": q["designation"],
                           "attempt_number": attempt, "provider": PROVIDER, "endpoint": ENDPOINT,
                           "requested_at": now(), "http_status": status, "latency_ms": latency,
                           "payload": payload}
                raw_path.write_text(canonical_json(raw_doc), encoding="utf-8")
                raw_file_hash = sha(raw_path)
                results = payload.get("results", []) if isinstance(payload, dict) else []
                record = {"query_id": q["query_id"], "query_text": q["query_text"],
                          "strategy_class": q["strategy_class"], "designation": q["designation"],
                          "attempt_number": attempt, "provider": PROVIDER, "endpoint": ENDPOINT,
                          "timestamp": now(), "http_status": status, "latency_ms": latency,
                          "raw_result_count": min(len(results), DEPTH),
                          "raw_response_path": str(raw_path.relative_to(ROOT)),
                          "raw_response_sha256": raw_file_hash, "results": []}
                acquired_at = now()
                for rank, r in enumerate(results[:DEPTH], 1):
                    item = {"rank": rank, "title": r.get("title") or "",
                            "original_url": r.get("url") or "",
                            "snippet": r.get("content") or r.get("snippet") or r.get("description") or "",
                            "provider_metadata": {k: r[k] for k in ("engine", "engines", "category", "score", "parsed_url", "template") if k in r}}
                    row = acc.accept(q["query_id"], rank, item["title"], item["snippet"],
                                     item["original_url"], {"attempt_number": attempt,
                                                            "acquired_at": acquired_at,
                                                            "raw_response_path": record["raw_response_path"],
                                                            "raw_response_sha256": raw_file_hash,
                                                            "provider_metadata": item["provider_metadata"]})
                    item["canonical_url"] = row["canonical_url"] if row else None
                    item["reason_code"] = acc.ledger[-1]["reason_code"] if acc.ledger else "ACCEPTED"
                    record["results"].append(item)
                raw_attempts.append(record)
                break
            except (socket.timeout, TimeoutError, RuntimeError) as e:
                http_failure += 1
                error_class = "TIMEOUT" if isinstance(e, (socket.timeout, TimeoutError)) else "TEMPORARY_HTTP_FAILURE"
                raw_attempts.append({"query_id": q["query_id"], "query_text": q["query_text"],
                                     "strategy_class": q["strategy_class"], "designation": q["designation"],
                                     "attempt_number": attempt, "provider": PROVIDER, "endpoint": ENDPOINT,
                                     "timestamp": now(), "http_status": None, "latency_ms": None,
                                     "raw_result_count": 0, "results": [],
                                     "technical_error": {"error_class": error_class}})
                if attempt == 1:
                    retries += 1
                    continue
                break

    accepted = sorted(acc.accepted, key=lambda x: x["row_id"])
    treatment_valid = sum(1 for r in accepted if r["designation"] == "treatment")
    control_valid = sum(1 for r in accepted if r["designation"] == "control")
    total_valid = len(accepted)
    target_reached = treatment_valid >= TARGET_PER_ARM and control_valid >= TARGET_PER_ARM
    universe_exhausted = len(executed) == len(universe["queries"])
    status = "COMPLETE_VALID" if target_reached else ("EXHAUSTED_FROZEN_UNIVERSE" if universe_exhausted else "INCOMPLETE")

    evidence_path = OUT / RAW_EVIDENCE
    evidence_path.write_text(canonical_json(build_evidence_doc(universe, raw_attempts)), encoding="utf-8")

    ledger_doc = {"schema": "amatl.relevance.supplemental-v4-attempt-ledger.v1", "experiment_id": RUN_ID,
                  "attempts": acc.ledger, "rejection_counts": dict(acc.rejected),
                  "network_requests": len(raw_attempts)}
    ledger_path = OUT / ATTEMPT_LEDGER
    ledger_path.write_text(canonical_json(ledger_doc), encoding="utf-8")

    corpus = {"schema": "amatl.relevance.supplemental-v4-final-prelabel-corpus.v1", "corpus_id": CORPUS_ID,
              "status": "FROZEN_PRELABEL" if target_reached else "INCOMPLETE",
              "source_experiment": "independent-relevance-supplemental-v4",
              "design_sha256": sha(OUT / DESIGN),
              "raw_evidence_path": str(evidence_path.relative_to(ROOT)),
              "raw_evidence_sha256": sha(evidence_path),
              "rows": accepted}
    corpus_path = OUT / PRELABEL_CORPUS
    corpus_path.write_text(canonical_json(corpus), encoding="utf-8")
    corpus_hash = sha(corpus_path)

    prelabel_manifest = {"schema": "amatl.relevance.supplemental-v4-prelabel-manifest.v1",
                         "corpus_id": CORPUS_ID, "version": 1,
                         "source_experiment": "independent-relevance-supplemental-v4",
                         "freeze_status": "FROZEN_PRELABEL" if target_reached else "INCOMPLETE",
                         "prelabel_corpus_path": str(corpus_path.relative_to(ROOT)),
                         "prelabel_corpus_sha256": corpus_hash,
                         "rows": total_valid, "treatment_rows": treatment_valid,
                         "control_rows": control_valid,
                         "universe_sha256": sha(OUT / UNIVERSE),
                         "manifest_sha256": sha(OUT / MANIFEST),
                         "attestation_sha256": sha(OUT / ATTESTATION),
                         "design_sha256": sha(OUT / DESIGN),
                         "created_at": now(), "network_requests": len(raw_attempts),
                         "labels_present": False,
                         "label_fields": ["Relevant", "PossiblyRelevant", "NotRelevant", "Unknown"]}
    (OUT / PRELABEL_MANIFEST).write_text(canonical_json(prelabel_manifest), encoding="utf-8")
    prelabel_attestation = {"schema": "amatl.relevance.supplemental-v4-prelabel-attestation.v1",
                            "corpus_id": CORPUS_ID,
                            "status": "FROZEN_PRELABEL" if target_reached else "INCOMPLETE",
                            "prelabel_corpus_path": str(corpus_path.relative_to(ROOT)),
                            "prelabel_corpus_sha256": corpus_hash,
                            "manifest_path": str((OUT / PRELABEL_MANIFEST).relative_to(ROOT)),
                            "manifest_sha256": sha(OUT / PRELABEL_MANIFEST),
                            "chain": "prelabel corpus -> prelabel manifest -> prelabel attestation",
                            "self_hash_in_manifest": False,
                            "attestor": "EXPERIMENTAL_INTEGRITY_AUDITOR",
                            "network_requests": len(raw_attempts)}
    (OUT / PRELABEL_ATTESTATION).write_text(canonical_json(prelabel_attestation), encoding="utf-8")

    packet_manifest = write_packets(corpus, corpus_path)

    rejected_total = sum(acc.rejected.values())
    report = {"schema": "amatl.relevance.supplemental-v4-run-report.v1", "REPOSITORY_STATE": state(),
              "STARTING_HEAD": start_commit, "ENDING_HEAD": commit(),
              "V4_PRECAPTURE_INTEGRITY": "PASS", "V4_DESIGN_HASH": sha(OUT / DESIGN),
              "V4_QUERY_UNIVERSE_HASH": sha(OUT / UNIVERSE), "V4_MANIFEST_HASH": sha(OUT / MANIFEST),
              "V4_ATTESTATION_HASH": sha(OUT / ATTESTATION),
              "PROVIDER": "SearXNG", "ENDPOINT": ENDPOINT, "RESULT_DEPTH": DEPTH,
              "NETWORK_REQUESTS": len(raw_attempts), "HTTP_SUCCESS": http_success,
              "HTTP_FAILURE": http_failure, "TECHNICAL_RETRIES": retries,
              "QUERIES_TOTAL": len(universe["queries"]), "QUERIES_EXECUTED": len(executed),
              "TREATMENT_QUERIES_EXECUTED": sum(q["designation"] == "treatment" for q in executed),
              "CONTROL_QUERIES_EXECUTED": sum(q["designation"] == "control" for q in executed),
              "RAW_RESULTS": sum(len(a.get("results", [])) for a in raw_attempts),
              "REJECTED_TOTAL": rejected_total,
              **{"REJECTED_" + k: acc.rejected.get(k, 0) for k in ("DUPLICATE_CURRENT", "OVERLAP_V1", "OVERLAP_SUPPLEMENTAL", "OVERLAP_V3", "HISTORICAL_OVERLAP", "INVALID_RESULT", "TREATMENT_TARGET_FILLED", "CONTROL_TARGET_FILLED", "OUTSIDE_ALLOWED_DEPTH")},
              "VALID_ROWS": total_valid, "TREATMENT_VALID_ROWS": treatment_valid,
              "CONTROL_VALID_ROWS": control_valid,
              "VALID_DUPLICATES": 0, "VALID_OVERLAP_V1": 0, "VALID_OVERLAP_SUPPLEMENTAL": 0,
              "VALID_OVERLAP_V3": 0, "VALID_OVERLAP_HISTORICAL": 0,
              "V4_VALID_TARGET_PER_ARM": TARGET_PER_ARM, "V4_TOTAL_VALID_RESULT_TARGET": TARGET_TOTAL,
              "UNIVERSE_EXHAUSTED": universe_exhausted,
              "STOP_EARLY_TRIGGERED": target_reached and not universe_exhausted,
              "V4_CAPTURE_STATUS": status, "V4_PRELABEL_STATUS": corpus["status"],
              "FINAL_PRELABEL_ROWS": total_valid, "FINAL_PRELABEL_HASH": corpus_hash,
              "PRELABEL_MANIFEST_HASH": sha(OUT / PRELABEL_MANIFEST),
              "PRELABEL_ATTESTATION_HASH": sha(OUT / PRELABEL_ATTESTATION),
              "PACKET_MANIFEST_HASH": sha(OUT / PACKET_MANIFEST),
              "LABELER_A_PACKET_HASH": packet_manifest["packet_hashes"]["A"],
              "LABELER_B_PACKET_HASH": packet_manifest["packet_hashes"]["B"],
              "LABELS_CREATED": 0}
    report_path = OUT / RUN_REPORT
    report_path.write_text(canonical_json(report), encoding="utf-8")
    report["RUN_REPORT_HASH"] = sha(report_path)
    report_path.write_text(canonical_json(report), encoding="utf-8")

    print(json.dumps({"status": status, "treatment_valid": treatment_valid, "control_valid": control_valid,
                      "total_valid": total_valid, "queries_executed": len(executed),
                      "network_requests": len(raw_attempts), "report": str(report_path)}, indent=2))


if __name__ == "__main__":
    main()







