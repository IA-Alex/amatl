#!/usr/bin/env python3
import json
import tempfile
import unittest
from pathlib import Path

import relevance_protocol as protocol


class ProtocolTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.data = self.root / "data.json"; self.data.write_text('{"rows":[1,2,3]}\n')
        self.artifact = self.root / "model.bin"; self.artifact.write_text("model-a\n")
        self.evaluator = self.root / "evaluator.py"; self.evaluator.write_text("# evaluator\n")
        self.candidate = {"candidate_id":"candidate-a", "implementation_commit":"abc", "model_id":"local/a", "model_hash":"model-hash", "backend":"fixture", "parameters":{}, "thresholds":{}, "preprocessing":"none", "artifact_path":str(self.artifact), "evaluator_path":str(self.evaluator)}

    def tearDown(self): self.temp.cleanup()

    def inventory(self, seed=7, state="CLEAN_UNSEEN", holdout_state="UNOPENED"):
        return {"schema":"amatl.relevance.dataset-inventory.v1", "split_seed":seed, "datasets":[{"dataset_id":"future", "path":"data.json", "hash":protocol.sha256_file(self.data), "contamination_status":state, "holdout_state":holdout_state}]}

    def test_same_seed_has_same_split_identity_and_changed_seed_does_not(self):
        rows = [{"row_id": f"r{i}", "label": "Relevant" if i < 4 else "NotRelevant"} for i in range(8)]
        first = protocol.deterministic_stratified_split(rows, 7)
        self.assertEqual(first, protocol.deterministic_stratified_split(rows, 7))
        self.assertNotEqual(first, protocol.deterministic_stratified_split(rows, 8))
        self.assertEqual(len(first["selection"]), 4)

    def test_dataset_hash_detects_modification(self):
        manifest = self.inventory(); manifest_path = self.root / "inventory.json"; manifest_path.write_text(json.dumps(manifest))
        self.assertIsNotNone(protocol.verify_inventory(manifest_path))
        self.data.write_text('{"rows":[9]}\n')
        with self.assertRaisesRegex(ValueError, "hash mismatch"): protocol.verify_inventory(manifest_path)

    def test_consumed_holdout_cannot_be_clean_unseen(self):
        manifest = self.inventory(state="CLEAN_UNSEEN", holdout_state="CONSUMED"); manifest_path = self.root / "inventory.json"; manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "consumed holdout"): protocol.verify_inventory(manifest_path)

    def test_candidate_modified_after_freeze_is_invalid(self):
        frozen = protocol.freeze_candidate(self.candidate); protocol.verify_frozen_candidate(frozen)
        self.artifact.write_text("model-b\n")
        with self.assertRaisesRegex(ValueError, "modified after freeze"): protocol.verify_frozen_candidate(frozen)

    def test_policy_change_changes_hash(self):
        self.assertNotEqual(protocol.canonical_hash({"macro_f1":0.30}), protocol.canonical_hash({"macro_f1":0.31}))

    def test_step4e_and_historical_holdout_are_not_blind(self):
        for state in ("KNOWN_TUNED", "HISTORICAL_ONLY"):
            with self.assertRaisesRegex(ValueError, "not an unconsumed"):
                protocol.consume_holdout({"datasets":[{"dataset_id":"x", "hash":"h", "contamination_status":state}]}, "x", protocol.freeze_candidate(self.candidate), {}, "2026-01-01T00:00:00Z", "result")

    def test_step4e_historical_identity_cannot_be_reclassified(self):
        manifest = self.inventory()
        manifest["datasets"][0]["dataset_id"] = "step4e-v1"
        path = self.root / "inventory.json"; path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "historical classification"):
            protocol.verify_inventory(path)


if __name__ == "__main__": unittest.main()
