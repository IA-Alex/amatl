#!/usr/bin/env python3
"""Protocol tests for the frozen unanimous supplemental ground truth.

Mirrors the integrity gates implemented in
tools/validate_supplemental_ground_truth.py so that the tests and the
read-only validator stay in lock-step.  The freeze tool is idempotent and
deterministic, so setUpClass simply re-runs it and then inspects the frozen
artefacts.
"""
import hashlib
import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"

LABELS = ("Relevant", "PossiblyRelevant", "NotRelevant", "Unknown")
ORIGINAL_FIELDS = (
    "SOURCE_RUN", "acquired_at", "acquisition_run_id", "canonical_url", "domain",
    "original_url", "provider_or_source", "published_at", "query", "query_id",
    "rank", "raw_response_path", "raw_response_sha256", "result_status",
    "row_id", "snippet", "stratum", "title",
)
EXPECTED_CLASS = {"Relevant": 10, "PossiblyRelevant": 72, "NotRelevant": 98, "Unknown": 0}
EXPECTED_STRATA = {"S1_PROCEDURAL_SHALLOW": 60, "S2_INFORMATIONAL_SHALLOW": 60, "S3_MIXED_DEPTH_CONTROL": 60}
EXPECTED_SOURCE = {"supplemental-v1": 172, "supplemental-v2": 8}
GT_ID = "independent-relevance-supplemental-ground-truth-v1"
LABEL_SOURCE = "UNANIMOUS_A_B_AGREEMENT"


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical_hash(value):
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    ).hexdigest()


class SupplementalGroundTruthTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run([sys.executable, str(ROOT / "tools/freeze_supplemental_ground_truth.py")],
                       cwd=ROOT, check=True)
        cls.ground = json.loads((OUT / "supplemental-ground-truth-v1.json").read_text())
        cls.manifest = json.loads((OUT / "supplemental-ground-truth-v1-manifest.json").read_text())
        cls.attestation = json.loads((OUT / "supplemental-ground-truth-v1-attestation.json").read_text())
        cls.prelabel = json.loads((OUT / "supplemental-final-prelabel-corpus.json").read_text())
        cls.a_doc = json.loads((OUT / "supplemental-labeler-a-labeled.json").read_text())
        cls.b_doc = json.loads((OUT / "supplemental-labeler-b-labeled.json").read_text())
        cls.v1 = json.loads((OUT / "v1-ground-truth.json").read_text())

    def test_identity_and_unanimity(self):
        self.assertEqual(self.ground["schema"], "amatl.relevance.supplemental-ground-truth.v1")
        self.assertEqual(self.ground["ground_truth_id"], GT_ID)
        self.assertEqual(self.ground["status"], "FROZEN")
        self.assertIs(self.ground["adjudication_used"], False)
        self.assertEqual(self.ground["label_source"], LABEL_SOURCE)
        rows = self.ground["rows"]
        self.assertEqual(len(rows), 180)
        self.assertEqual(len({r["row_id"] for r in rows}), 180)
        self.assertTrue(all(r["final_label"] == r["label_a"] == r["label_b"] for r in rows))
        self.assertTrue(all(r["final_label_source"] == LABEL_SOURCE for r in rows))
        self.assertTrue(all(r["final_label"] in LABELS for r in rows))

    def test_distributions(self):
        rows = self.ground["rows"]
        self.assertEqual(
            {label: sum(r["final_label"] == label for r in rows) for label in LABELS},
            EXPECTED_CLASS)
        self.assertEqual({s: sum(r["stratum"] == s for r in rows) for s in EXPECTED_STRATA},
                         EXPECTED_STRATA)
        self.assertEqual({s: sum(r["SOURCE_RUN"] == s for r in rows) for s in EXPECTED_SOURCE},
                         EXPECTED_SOURCE)
        self.assertEqual(self.manifest["SUPPLEMENTAL_GT_CLASS_DISTRIBUTION"], EXPECTED_CLASS)
        self.assertEqual(self.manifest["SUPPLEMENTAL_GT_STRATUM_DISTRIBUTION"], EXPECTED_STRATA)
        self.assertEqual(self.manifest["SUPPLEMENTAL_GT_SOURCE_RUN_DISTRIBUTION"], EXPECTED_SOURCE)

    def test_original_fields_survive_byte_for_byte(self):
        gt_by_id = {r["row_id"]: r for r in self.ground["rows"]}
        for doc in (self.prelabel, self.a_doc, self.b_doc):
            for row in doc["rows"]:
                self.assertEqual({f: gt_by_id[row["row_id"]][f] for f in ORIGINAL_FIELDS},
                                 {f: row[f] for f in ORIGINAL_FIELDS})
        self.assertEqual([r["row_id"] for r in self.ground["rows"]],
                         [r["row_id"] for r in self.prelabel["rows"]])

    def test_no_overlap_with_v1(self):
        v1_canonical = {r["canonical_url"] for r in self.v1["rows"]}
        gt_canonical = {r["canonical_url"] for r in self.ground["rows"]}
        self.assertEqual(len(gt_canonical), 180)
        self.assertEqual(v1_canonical & gt_canonical, set())
    def test_hash_chain_and_reproducibility(self):
        raw = (OUT / "supplemental-ground-truth-v1.json").read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), self.manifest["GROUND_TRUTH_HASH"])
        self.assertEqual(self.attestation["ground_truth_hash"], self.manifest["GROUND_TRUTH_HASH"])
        self.assertEqual(self.attestation["manifest_hash"],
                         sha256_file(OUT / "supplemental-ground-truth-v1-manifest.json"))
        self.assertEqual(self.manifest["GROUND_TRUTH_CANONICAL_HASH"],
                         canonical_hash(self.ground))
        self.assertEqual(self.attestation["ground_truth_id"], GT_ID)
        self.assertEqual(self.attestation["status"], "FROZEN")
        self.assertEqual(self.attestation["adjudication_used"], "NO")
        self.assertEqual(self.attestation["ground_truth_label_source"], LABEL_SOURCE)

    def test_canonical_hash_is_sensitive_to_labels_and_original_fields(self):
        changed_label = json.loads((OUT / "supplemental-ground-truth-v1.json").read_text())
        changed_label["rows"][0]["final_label"] = "Unknown"
        changed_source = json.loads((OUT / "supplemental-ground-truth-v1.json").read_text())
        changed_source["rows"][0]["title"] += " changed"
        self.assertNotEqual(canonical_hash(changed_label), self.manifest["GROUND_TRUTH_CANONICAL_HASH"])
        self.assertNotEqual(canonical_hash(changed_source), self.manifest["GROUND_TRUTH_CANONICAL_HASH"])

    def test_manifest_source_hashes_match_disk(self):
        expected = {
            "SUPPLEMENTAL_PRELABEL_CORPUS_HASH": "supplemental-final-prelabel-corpus.json",
            "SUPPLEMENTAL_LABELER_A_HASH": "supplemental-labeler-a-labeled.json",
            "SUPPLEMENTAL_LABELER_B_HASH": "supplemental-labeler-b-labeled.json",
            "SUPPLEMENTAL_AGREEMENT_REPORT_HASH": "supplemental-agreement-report.json",
            "SUPPLEMENTAL_FINAL_FREEZE_MANIFEST_HASH": "supplemental-final-freeze-manifest.json",
            "SUPPLEMENTAL_MULTIRUN_PROVENANCE_MANIFEST_HASH": "supplemental-multirun-provenance-manifest.json",
            "SUPPLEMENTAL_PACKET_MANIFEST_HASH": "supplemental-packet-manifest.json",
            "V1_GROUND_TRUTH_HASH": "v1-ground-truth.json",
        }
        for key, name in expected.items():
            self.assertEqual(self.manifest[key], sha256_file(OUT / name), key)

    def test_manifest_has_no_self_hash(self):
        self.assertNotIn("GROUND_TRUTH_MANIFEST_HASH", self.manifest)
        self.assertNotIn("hashes", self.manifest)

    def test_rerun_with_different_source_commit_preserves_manifest_and_hashes(self):
        paths = [OUT / name for name in ("supplemental-ground-truth-v1.json",
                                         "supplemental-ground-truth-v1-manifest.json",
                                         "supplemental-ground-truth-v1-attestation.json")]
        before = {path: path.read_bytes() for path in paths}
        subprocess.run([sys.executable, str(ROOT / "tools/freeze_supplemental_ground_truth.py"),
                        "--source-commit", "2" * 40], cwd=ROOT, check=True)
        self.assertEqual({path: path.read_bytes() for path in paths}, before)
        self.assertEqual(self.manifest["SOURCE_COMMIT"], "b0efd57af5291c1fb2e030ab938b9377957084e7")


if __name__ == "__main__":
    unittest.main()
