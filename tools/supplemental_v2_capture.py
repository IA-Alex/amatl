#!/usr/bin/env python3
"""S1-v2 transport entry point; no network is opened without --allow-network."""
import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", default=OUT / "supplemental-v2-searxng-capture-config.json")
    parser.add_argument("--allow-network", action="store_true")
    args = parser.parse_args()
    config = json.loads(Path(args.config).read_text())
    required = {"provider": "searxng", "provider_fallback": "none", "dynamic_provider_selection": False, "dynamic_query_expansion": False, "s1_new_valid_target": 8}
    if any(config.get(k) != v for k, v in required.items()) or config["retry_policy"].get("max_attempts") != 2 or config["retry_policy"].get("same_query_only") is not True:
        raise ValueError("V2_CAPTURE_CONTRACT_FAILURE")
    from supplemental_v2_contract import build_accumulator
    universe = json.loads((ROOT / config["universe_path"]).read_text())
    build_accumulator(universe)  # verifies every multirun dedup input exists
    if not args.allow_network:
        raise ValueError("CAPTURE_NETWORK_NOT_AUTHORIZED")
    # Deliberately no implementation call here: capture requires a separate,
    # explicit authorization work package.  This guard proves v2 is routed
    # through a dedicated SearXNG-only transport contract without fallback.
    raise RuntimeError("V2_CAPTURE_REQUIRES_EXPLICIT_EXECUTION_WORK_PACKAGE")

if __name__ == "__main__": main()
