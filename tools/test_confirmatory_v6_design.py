#!/usr/bin/env python3
"""Offline structural tests for the frozen V6 confirmatory design."""
import hashlib
import json
import random
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/v6"
SOURCE = ROOT / "docs/evaluation/independent-relevance/candidates/candidate-v2/candidate-universe.json"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(name):
    return json.loads((OUT / name).read_text(encoding="utf-8"))


class V6FreezeTests(unittest.TestCase):
    def test_source_identity_and_counts(self):
        d = load("design.json")
        q = load("frozen-query-universe.json")
        self.assertEqual(sha(SOURCE), d["source_candidate_sha256"])
        self.assertEqual(SOURCE.read_bytes(), (OUT / "frozen-query-universe.json").read_bytes())
        self.assertEqual(len(q["queries"]), 4600)
        self.assertEqual((q["treatment_query_count"], q["control_query_count"]), (2300, 2300))
        self.assertEqual(q["network_requests"], 0)

    def test_query_identity_and_pairs(self):
        q = load("frozen-query-universe.json")
        queries = q["queries"]
        self.assertEqual(len({x["query_id"] for x in queries}), 4600)
        self.assertEqual(len({x["pair_id"] for x in queries}), 2300)
        by_pair = {}
        for x in queries:
            by_pair.setdefault(x["pair_id"], []).append(x)
        self.assertTrue(all({x["arm"] for x in xs} == {"treatment", "control"} and len(xs) == 2 for xs in by_pair.values()))
        self.assertTrue(all(x["query_family"] for x in queries))
        self.assertTrue(all(x["generation_provenance"]["network_requests"] == 0 for x in queries))

    def test_randomization_is_new_and_reproducible(self):
        r = load("randomization-manifest.json")
        self.assertEqual(r["seed"], "independent-relevance-confirmatory-v6-randomization-v1")
        self.assertEqual(r["pair_count"], 2300)
        pairs = [f"candidate-p-{i:04d}" for i in range(1, 2301)]
        shuffled = pairs[:]
        random.Random(r["seed"]).shuffle(shuffled)
        self.assertNotEqual(shuffled, pairs)
        assignments = load("pair-assignments.json")["assignments"]
        self.assertEqual([a["arm"] for a in assignments[:2]], ["treatment", "control"])
        actual = "\n".join(f'{a["pair_id"]}\t{a["arm"]}\t{a["query_id"]}' for a in assignments).encode()
        self.assertEqual(hashlib.sha256(actual).hexdigest(), r["assignment_sha256"])
        self.assertEqual(len(assignments), 4600)

    def test_policy_and_exclusion_contract(self):
        d, x, e = load("design.json"), load("decision-policy.json"), load("execution-policy.json")
        h = load("historical-exclusion-manifest.json")
        self.assertEqual(h["historical_exclusion_set"], ["V1", "V2", "V3", "V4", "V5", "historical evaluation corpus"])
        self.assertEqual(d["capacity"]["headroom"], "THIN")
        self.assertEqual(e["success_stop"], "TREATMENT_VALID_RESULTS >= 474 AND CONTROL_VALID_RESULTS >= 474")
        self.assertIn("INCONCLUSIVE", h["policy"] + d["capacity_failure_policy"] + x["inconclusive"])
        self.assertEqual(d["labeling_started"], "NO")
        self.assertEqual(d["network_requests"], 0)
        self.assertEqual(x["multiplicity_adjustment"], "NOT_REQUIRED")

    def test_root_hash_manifest(self):
        p = load("provenance-manifest.json")
        self.assertEqual(p["source_commit"], "c0d5e49ae9eb89a9a26a620e8c0d97fbe3b50a8f")
        self.assertEqual(p["network_requests"], 0)
        for name, expected in p["artifacts"].items():
            self.assertEqual(sha(OUT / name), expected, name)


if __name__ == "__main__":
    unittest.main()
