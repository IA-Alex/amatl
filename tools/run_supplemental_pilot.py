#!/usr/bin/env python3
"""Apply the frozen pilot contract to externally supplied SearXNG responses.

This runner intentionally performs no acquisition itself.  The separately
authorised capture transport writes response records, then this command is the
only route by which those records may count toward the 180-row pilot.
"""
import argparse
import json
from pathlib import Path

from supplemental_pilot_contract import PilotAccumulator, load_json
from supplemental_pilot_capture_contract import offline_records, validate_raw_evidence

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--offline-results", help="JSON list of {query_id, provider, result}; never fetched by this tool")
    parser.add_argument("--raw-evidence", help="validated output from supplemental_pilot_capture.py; never fetched by this tool")
    parser.add_argument("--capture-config", default=OUT / "supplemental-pilot-searxng-capture-config.json",
                        help="the same isolated, endpoint-bound capture contract used for acquisition")
    parser.add_argument("--historical-canonical-urls", help="JSON list of prior canonical URLs")
    parser.add_argument("--output", help="write auditable accepted and rejected attempts")
    args = parser.parse_args()
    universe = load_json(OUT / "supplemental-query-universe-v1.json")
    v1 = load_json(OUT / "v1-ground-truth.json")["rows"]
    history = load_json(args.historical_canonical_urls) if args.historical_canonical_urls else []
    gate = PilotAccumulator(universe, {row["canonical_url"] for row in v1}, set(history))
    if args.offline_results and args.raw_evidence:
        parser.error("choose exactly one of --offline-results or --raw-evidence")
    if args.raw_evidence:
        config = load_json(args.capture_config)
        evidence = load_json(args.raw_evidence)
        validate_raw_evidence(evidence, universe, config)
        records = offline_records(evidence)
    else:
        records = load_json(args.offline_results) if args.offline_results else []
    for record in records:
        gate.accept_or_reject(record.get("query_id"), record.get("provider"), record.get("result", {}))
    output = {"schema": "amatl.relevance.supplemental-pilot-attempt.v1", "network_requests": 0,
              "source": "offline-results only", "valid_counts": gate.valid_counts,
              "total_valid": sum(gate.valid_counts.values()), "status": gate.exhausted_status(), "attempts": gate.attempts}
    rendered = json.dumps(output, indent=2) + "\n"
    if args.output: Path(args.output).write_text(rendered, encoding="utf-8")
    else: print(rendered, end="")


if __name__ == "__main__": main()
