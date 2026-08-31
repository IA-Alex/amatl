#!/usr/bin/env python3
"""Small, file-backed controls for independent relevance evaluation.

The tool deliberately does not run a candidate.  It validates the frozen
evidence around one, so a final holdout is opened only after the candidate and
the acceptance policy have identities that can be checked again.
"""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

BLIND_STATES = {"CLEAN_UNSEEN"}
KNOWN_STATES = {"KNOWN_NOT_TUNED", "KNOWN_TUNED", "CONTAMINATED", "HISTORICAL_ONLY"}
IMMUTABLE_HISTORY = {
    "step4e-v1": ("KNOWN_TUNED", "db7eea3026f05adc0b59c6868e78c710928163912e009456cef43785169dc198"),
    "bounded-semantic-historical-holdout-v1": ("HISTORICAL_ONLY", "c9124992e4eba406ae58f60149c1ef87522c0c2bd97d9b3ba77dd802f2f001b5"),
}


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical_hash(value):
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def load_json(path):
    return json.loads(Path(path).read_text())


def deterministic_stratified_split(rows, seed, selection_fraction=0.5):
    """Assign labelled rows reproducibly, preserving each label's proportions."""
    if not 0 < selection_fraction < 1:
        raise ValueError("selection_fraction must be between zero and one")
    groups = {}
    for row in rows:
        groups.setdefault(row["label"], []).append(row)
    output = {"selection": [], "blind": []}
    for label, group in groups.items():
        ordered = sorted(group, key=lambda row: hashlib.sha256(
            f"{seed}:{label}:{row['row_id']}".encode()).hexdigest())
        selection_count = round(len(ordered) * selection_fraction)
        output["selection"].extend(row["row_id"] for row in ordered[:selection_count])
        output["blind"].extend(row["row_id"] for row in ordered[selection_count:])
    return {key: sorted(value) for key, value in output.items()}


def verify_inventory(path):
    inventory = load_json(path)
    if inventory.get("schema") != "amatl.relevance.dataset-inventory.v1":
        raise ValueError("unsupported dataset inventory schema")
    for item in inventory["datasets"]:
        state = item["contamination_status"]
        if state not in BLIND_STATES | KNOWN_STATES:
            raise ValueError(f"{item['dataset_id']}: invalid contamination state")
        historical = IMMUTABLE_HISTORY.get(item["dataset_id"])
        if historical and (state != historical[0] or item["hash"] != historical[1]):
            raise ValueError(f"{item['dataset_id']}: historical classification is immutable")
        if item.get("holdout_state") == "CONSUMED" and state in BLIND_STATES:
            raise ValueError(f"{item['dataset_id']}: consumed holdout cannot be CLEAN_UNSEEN")
        relative = Path(item["path"])
        dataset_path = Path(path).parent / relative
        if not dataset_path.exists():
            dataset_path = ROOT / relative
        actual = sha256_file(dataset_path)
        if actual != item["hash"]:
            raise ValueError(f"{item['dataset_id']}: dataset hash mismatch")
    return inventory


def freeze_candidate(candidate):
    required = ("candidate_id", "implementation_commit", "model_id", "model_hash",
                "backend", "parameters", "thresholds", "preprocessing",
                "evaluator_path", "artifact_path")
    missing = [key for key in required if key not in candidate]
    if missing:
        raise ValueError(f"candidate freeze missing: {', '.join(missing)}")
    frozen = dict(candidate)
    frozen["state"] = "FROZEN"
    frozen["evaluator_hash"] = sha256_file(frozen["evaluator_path"])
    frozen["artifact_hash"] = sha256_file(frozen["artifact_path"])
    frozen["identity_hash"] = canonical_hash({k: v for k, v in frozen.items() if k != "identity_hash"})
    return frozen


def verify_frozen_candidate(frozen):
    if frozen.get("state") != "FROZEN":
        raise ValueError("candidate is not FROZEN")
    expected = canonical_hash({k: v for k, v in frozen.items() if k != "identity_hash"})
    if frozen.get("identity_hash") != expected:
        raise ValueError("candidate freeze identity mismatch")
    for field, path_field in (("evaluator_hash", "evaluator_path"), ("artifact_hash", "artifact_path")):
        if sha256_file(frozen[path_field]) != frozen[field]:
            raise ValueError(f"candidate modified after freeze: {path_field}")
    return frozen


def consume_holdout(inventory, holdout_id, frozen, policy, opened_at, result_hash):
    verify_frozen_candidate(frozen)
    item = next((d for d in inventory["datasets"] if d["dataset_id"] == holdout_id), None)
    if not item:
        raise ValueError("unknown holdout_id")
    if item["contamination_status"] != "CLEAN_UNSEEN" or item.get("holdout_state") == "CONSUMED":
        raise ValueError("holdout is not an unconsumed CLEAN_UNSEEN dataset")
    return {
        "schema": "amatl.relevance.holdout-consumption.v1",
        "holdout_id": holdout_id, "holdout_hash": item["hash"], "opened_at": opened_at,
        "candidate_id": frozen["candidate_id"], "candidate_commit": frozen["implementation_commit"],
        "model_hash": frozen["model_hash"], "evaluator_hash": frozen["evaluator_hash"],
        "pass_fail_policy_hash": canonical_hash(policy), "result_hash": result_hash,
        "holdout_state": "CONSUMED",
    }


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("verify-inventory"); p.add_argument("inventory")
    p = sub.add_parser("split"); p.add_argument("input"); p.add_argument("seed", type=int); p.add_argument("output"); p.add_argument("--selection-fraction", type=float, default=0.5)
    p = sub.add_parser("freeze-candidate"); p.add_argument("input"); p.add_argument("output")
    p = sub.add_parser("verify-candidate"); p.add_argument("candidate")
    p = sub.add_parser("consume-holdout")
    p.add_argument("inventory"); p.add_argument("holdout_id"); p.add_argument("candidate"); p.add_argument("policy"); p.add_argument("opened_at"); p.add_argument("result_hash"); p.add_argument("output")
    args = parser.parse_args()
    if args.command == "verify-inventory":
        verify_inventory(args.inventory)
        print("INVENTORY_VALID=YES")
    elif args.command == "split":
        rows = load_json(args.input)["rows"]
        output = {"schema": "amatl.relevance.split.v1", "input_hash": sha256_file(args.input),
                  "seed": args.seed, "selection_fraction": args.selection_fraction,
                  "assignments": deterministic_stratified_split(rows, args.seed, args.selection_fraction)}
        output["identity_hash"] = canonical_hash(output)
        Path(args.output).write_text(json.dumps(output, indent=2) + "\n")
    elif args.command == "freeze-candidate":
        Path(args.output).write_text(json.dumps(freeze_candidate(load_json(args.input)), indent=2) + "\n")
    elif args.command == "verify-candidate":
        verify_frozen_candidate(load_json(args.candidate)); print("CANDIDATE_FREEZE_VALID=YES")
    else:
        inventory = verify_inventory(args.inventory)
        record = consume_holdout(inventory, args.holdout_id, load_json(args.candidate), load_json(args.policy), args.opened_at, args.result_hash)
        Path(args.output).write_text(json.dumps(record, indent=2) + "\n")


if __name__ == "__main__":
    main()
