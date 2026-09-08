#!/usr/bin/env python3
"""Offline tests for the benchmark-plan runner's traceability contract (AUDIT-06).

No subprocess, socket, HTTP, provider, or AMATL integration is exercised.
Every test uses LocalMockExecutor / RecordingSleeper style in-process fakes.

Run from the repository root with:
    (cd tools && python3 -m unittest test_benchmark_plan_runner -v)

(This module imports its subject as a top-level `benchmark_plan_runner`, so
`tools/` must be the working directory — or on PYTHONPATH — for the import
to resolve; `python3 -m unittest tools/test_benchmark_plan_runner.py` from
the repo root fails with ModuleNotFoundError. [M4])
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

import benchmark_plan_runner as runner


DATASET_DOCUMENT = {
    "queries": [{"id": f"Q{i:02d}", "query": f"query {i}"} for i in range(1, 11)]
}


def write_dataset(path: Path, document: dict = DATASET_DOCUMENT) -> Path:
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


def make_validated_plan(repetitions: int, query_ids=tuple(f"Q{i:02d}" for i in range(1, 11))):
    plan = runner.build_plan("bench", "searxng", query_ids, repetitions)
    return runner.validated_plan(plan, query_ids)


class RecordingSleeper:
    def __init__(self):
        self.calls: list[float] = []

    def __call__(self, seconds: float) -> None:
        self.calls.append(seconds)


class OrderRecordingExecutor:
    """Wraps LocalMockExecutor and records executor-vs-sleep interleaving."""
    def __init__(self, mock: runner.LocalMockExecutor, events: list[str]):
        self.mock = mock
        self.events = events

    def __call__(self, position: runner.Position):
        self.events.append(f"executor:{position.query_id}-R{position.repetition}")
        return self.mock(position)


def sleeper_recording_into(events: list[str]):
    def sleep_fn(seconds: float) -> None:
        events.append(f"sleep:{seconds}")
    return sleep_fn


class PacingTests(unittest.TestCase):
    def test_a_n30_sleep_count_is_29(self):
        validated = make_validated_plan(3)  # 10 queries * 3 reps = 30
        mock = runner.LocalMockExecutor(runner.Position("bench", "searxng", "Q99", 99))
        sleeper = RecordingSleeper()
        runner.execute_plan(validated, mock, interval_seconds=3, sleep_fn=sleeper)
        self.assertEqual(len(sleeper.calls), 29)
        self.assertTrue(all(seconds == 3 for seconds in sleeper.calls))

    def test_b_n1_sleep_count_is_0(self):
        validated = make_validated_plan(1, query_ids=("Q01",))
        mock = runner.LocalMockExecutor(runner.Position("bench", "searxng", "Q99", 99))
        sleeper = RecordingSleeper()
        runner.execute_plan(validated, mock, interval_seconds=3, sleep_fn=sleeper)
        self.assertEqual(len(sleeper.calls), 0)

    def test_c_sleep_between_positions_never_before_or_after(self):
        validated = make_validated_plan(3)
        mock = runner.LocalMockExecutor(runner.Position("bench", "searxng", "Q99", 99))
        events: list[str] = []
        runner.execute_plan(
            validated, OrderRecordingExecutor(mock, events), interval_seconds=3,
            sleep_fn=sleeper_recording_into(events),
        )
        self.assertTrue(events[0].startswith("executor:"), "no sleep before the first position")
        self.assertTrue(events[-1].startswith("executor:"), "no sleep after the last position")
        # Each sleep must be immediately preceded and followed by an executor call.
        for index, event in enumerate(events):
            if event.startswith("sleep:"):
                self.assertTrue(events[index - 1].startswith("executor:"))
                self.assertTrue(events[index + 1].startswith("executor:"))
        self.assertEqual(sum(1 for event in events if event.startswith("sleep:")), 29)


class SequenceAndClassificationTests(unittest.TestCase):
    def test_d_sequence_number_is_deterministic(self):
        validated = make_validated_plan(3)
        mock1 = runner.LocalMockExecutor(runner.Position("bench", "searxng", "Q05", 2))
        mock2 = runner.LocalMockExecutor(runner.Position("bench", "searxng", "Q05", 2))
        state1 = runner.execute_plan(validated, mock1)
        state2 = runner.execute_plan(validated, mock2)
        sequence1 = [record.sequence_number for record in state1.records]
        sequence2 = [record.sequence_number for record in state2.records]
        self.assertEqual(sequence1, sequence2)
        self.assertEqual(sequence1, list(range(1, 31)))

    def test_e_classification_is_deterministic(self):
        cases = [
            (0, "SUCCESS", 10, False, None, "success"),
            (0, "SUCCESS", 10, True, None, "partial_success"),
            (0, "SUCCESS", 0, False, None, "zero_results"),
            (1, "FAILURE", None, None, None, "executor_failure"),
            (0, "EXECUTOR_FAILURE", None, None, None, "executor_failure"),
            (0, "SUCCESS", 5, False, "provider_rate_limit", "provider_error"),
            (0, "SUCCESS", None, None, None, "unknown"),
        ]
        for exit_code, status, results, partial, error, expected in cases:
            with self.subTest(status=status, results=results, partial=partial, error=error):
                first = runner.classify_execution(exit_code, status, results, partial, error)
                second = runner.classify_execution(exit_code, status, results, partial, error)
                self.assertEqual(first, expected)
                self.assertEqual(first, second)


class ManifestHashTests(unittest.TestCase):
    def test_f_manifest_contains_real_reproducible_hashes(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            dataset_path = write_dataset(temp / "dataset.json")
            config_path = temp / "fixture.toml"
            config_path.write_text('[providers]\nenabled = ["searxng"]\n', encoding="utf-8")

            dataset_sha256_a, query_ids, _ = runner.load_dataset_with_hash(dataset_path)
            dataset_sha256_b, _, _ = runner.load_dataset_with_hash(dataset_path)
            config_sha256_a = runner.sha256_bytes(config_path.read_bytes())
            config_sha256_b = runner.sha256_bytes(config_path.read_bytes())
            expected_dataset_sha256 = runner.sha256_bytes(dataset_path.read_bytes())
            expected_config_sha256 = runner.sha256_bytes(config_path.read_bytes())

            self.assertEqual(dataset_sha256_a, dataset_sha256_b)
            self.assertEqual(dataset_sha256_a, expected_dataset_sha256)
            self.assertEqual(config_sha256_a, config_sha256_b)
            self.assertEqual(config_sha256_a, expected_config_sha256)

            binary_path = temp / "fake-amatl"
            binary_path.write_bytes(b"#!/bin/sh\necho 'amatl 0.0.0-test'\n")
            binary_sha256 = runner.hash_binary(binary_path)

            manifest = runner.build_campaign_manifest(
                amatl_version="amatl 0.0.0-test",
                amatl_binary_path=str(binary_path), amatl_binary_sha256=binary_sha256,
                dataset_path=dataset_path, dataset_sha256=dataset_sha256_a,
                config_path=config_path, config_sha256=config_sha256_a,
                config_snapshot_path=str(temp / "campaign" / "config-snapshot.toml"),
                provider="searxng", benchmark_id="bench", query_order=query_ids,
                repetitions=3, retries=runner.RETRIES_DISABLED,
                timeout_seconds=runner.AMATL_SUBPROCESS_TIMEOUT_SECONDS,
                concurrency=runner.CONCURRENCY, interval_seconds=3,
                campaign_started_at=datetime.now(timezone.utc).isoformat(),
                sequence_count=30,
            )
            self.assertEqual(manifest.dataset_sha256, expected_dataset_sha256)
            self.assertEqual(manifest.config_sha256, expected_config_sha256)
            self.assertEqual(manifest.amatl_binary_sha256, binary_sha256)
            self.assertEqual(manifest.amatl_binary_path, str(binary_path))
            self.assertEqual(manifest.runner_sha256, runner.runner_sha256())
            self.assertEqual(manifest.runner_path, str(Path(runner.__file__).resolve()))
            self.assertEqual(manifest.schema_version, runner.RUNNER_SCHEMA_VERSION)

            output_dir = temp / "campaign"
            output_dir.mkdir()
            runner.write_campaign_manifest(output_dir, manifest)
            written = json.loads((output_dir / "campaign-manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(written["dataset_sha256"], expected_dataset_sha256)
            self.assertEqual(written["config_sha256"], expected_config_sha256)
            self.assertEqual(written["amatl_binary_sha256"], binary_sha256)
            self.assertEqual(written["amatl_binary_path"], str(binary_path))
            self.assertEqual(written["config_snapshot_path"], manifest.config_snapshot_path)
            self.assertEqual(written["runner_sha256"], runner.runner_sha256())
            self.assertEqual(written["query_order"], list(query_ids))

    def test_g_dataset_modification_changes_dataset_sha256(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            dataset_path = write_dataset(Path(temp_dir) / "dataset.json")
            before, _, _ = runner.load_dataset_with_hash(dataset_path)
            mutated = dict(DATASET_DOCUMENT)
            mutated["queries"] = list(DATASET_DOCUMENT["queries"]) + [{"id": "Q11", "query": "extra"}]
            write_dataset(dataset_path, mutated)
            after, _, _ = runner.load_dataset_with_hash(dataset_path)
            self.assertNotEqual(before, after)

    def test_h_config_modification_changes_config_sha256(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            config_path = Path(temp_dir) / "fixture.toml"
            config_path.write_text('[providers]\nenabled = ["searxng"]\n', encoding="utf-8")
            before = runner.sha256_bytes(config_path.read_bytes())
            config_path.write_text('[providers]\nenabled = ["searxng"]\n# comment\n', encoding="utf-8")
            after = runner.sha256_bytes(config_path.read_bytes())
            self.assertNotEqual(before, after)

    def test_i_simulated_runner_modification_changes_runner_sha256(self):
        original_bytes = Path(runner.__file__).read_bytes()
        baseline = runner.sha256_bytes(original_bytes)
        mutated_bytes = original_bytes + b"\n# AUDIT-06 offline-test perturbation, never written to disk\n"
        mutated = runner.sha256_bytes(mutated_bytes)
        self.assertNotEqual(baseline, mutated)
        # runner_sha256() itself must track whatever bytes are actually on disk.
        self.assertEqual(runner.runner_sha256(), baseline)


class RunsJsonlSchemaTests(unittest.TestCase):
    def test_j_runs_jsonl_readable_and_schema_valid(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            output_path = Path(temp_dir) / "runs.jsonl"
            validated = make_validated_plan(3)
            failure_position = runner.Position("bench", "searxng", "Q05", 2)
            mock = runner.LocalMockExecutor(failure_position)
            with runner.DurableJSONLWriter(output_path) as writer:
                runner.execute_plan(validated, mock, writer)
            records = runner.records_from_disk(output_path)
            self.assertEqual(len(records), 30)
            runner.validate_runs_jsonl(records)  # must not raise
            for record in records:
                self.assertEqual(record["schema_version"], runner.RUNS_SCHEMA_VERSION)
                datetime.fromisoformat(record["timestamp_utc"])  # must parse

    def test_j_invalid_record_rejected(self):
        good = {
            "sequence_number": 1, "benchmark_id": "b", "provider": "searxng", "query_id": "Q01",
            "repetition": 1, "process_exit_code": 0, "search_status": "SUCCESS",
            "timestamp_utc": datetime.now(timezone.utc).isoformat(), "classification": "unknown",
            "schema_version": runner.RUNS_SCHEMA_VERSION, "elapsed_ms": None, "final_results": None,
            "partial": None, "public_error": None,
        }
        runner.validate_run_record(good)  # sanity: valid record passes
        for broken in (
            {**good, "timestamp_utc": "not-a-timestamp"},
            {**good, "schema_version": "amatl-benchmark-runs/999"},
            {k: v for k, v in good.items() if k != "classification"},
            {**good, "sequence_number": "1"},
        ):
            with self.subTest(broken=broken):
                with self.assertRaises(runner.PlanAbort):
                    runner.validate_run_record(broken)


class ConfigSnapshotTests(unittest.TestCase):
    """[M1] config-file must be an immutable snapshot, not the mutable fixture."""

    def test_snapshot_immune_to_fixture_mutation_after_creation(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            fixture_path = temp / "fixture.toml"
            fixture_path.write_text('[providers]\nenabled = ["searxng"]\n', encoding="utf-8")
            output_dir = temp / "campaign"

            snapshot_path, config_sha256 = runner.prepare_config_snapshot(output_dir, fixture_path)
            original_snapshot_bytes = snapshot_path.read_bytes()

            binary_path = Path(sys.executable)
            executor = runner.AmatlProcessExecutor(
                binary=binary_path, binary_sha256=runner.hash_binary(binary_path),
                config_snapshot=snapshot_path, config_sha256=config_sha256,
                queries={"Q01": "hello world"},
            )
            argv_before = executor.argv_for(runner.Position("bench", "searxng", "Q01", 1))

            # Modify the ORIGINAL fixture *after* the snapshot/executor exist.
            fixture_path.write_text('[providers]\nenabled = ["searxng"]\n# tampered after snapshot\n', encoding="utf-8")

            # config_sha256 does not change (it was computed before the mutation
            # and is never recomputed from the mutable fixture path).
            self.assertEqual(runner.sha256_bytes(snapshot_path.read_bytes()), config_sha256)
            # the snapshot itself still holds exactly the originally-hashed bytes.
            self.assertEqual(snapshot_path.read_bytes(), original_snapshot_bytes)
            self.assertNotEqual(snapshot_path.read_bytes(), fixture_path.read_bytes())
            # argv_for still points at the snapshot, never the mutated fixture.
            argv_after = executor.argv_for(runner.Position("bench", "searxng", "Q01", 1))
            self.assertEqual(argv_before, argv_after)
            self.assertIn(str(snapshot_path), argv_after)
            self.assertNotIn(str(fixture_path), argv_after)

    def test_snapshot_exists_before_any_execution(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            fixture_path = temp / "fixture.toml"
            fixture_path.write_text('[providers]\nenabled = ["searxng"]\n', encoding="utf-8")
            snapshot_path, _ = runner.prepare_config_snapshot(temp / "campaign", fixture_path)
            self.assertTrue(snapshot_path.is_file())

    def test_invalid_fixture_rejected_before_snapshot(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            fixture_path = temp / "fixture.toml"
            fixture_path.write_text('[providers]\nenabled = ["searxng", "marginalia"]\n', encoding="utf-8")
            with self.assertRaises(runner.PlanAbort):
                runner.prepare_config_snapshot(temp / "campaign", fixture_path)


class BinaryIdentityTests(unittest.TestCase):
    """[M2] the manifest and the executor must agree on one hashed, resolved binary."""

    def test_a_temp_binary_expected_hash(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            binary_path = Path(temp_dir) / "fake-amatl"
            binary_path.write_bytes(b"#!/bin/sh\necho 'amatl 1.2.3'\n")
            expected = runner.sha256_bytes(binary_path.read_bytes())
            self.assertEqual(runner.hash_binary(binary_path), expected)

    def test_b_same_version_string_different_bytes_different_hash(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            binary_a = temp / "amatl-a"
            binary_b = temp / "amatl-b"
            binary_a.write_bytes(b"#!/bin/sh\necho 'amatl 1.2.3'\n")
            binary_b.write_bytes(b"#!/bin/sh\necho 'amatl 1.2.3'\n# padding byte differs the hash\n")
            self.assertNotEqual(runner.hash_binary(binary_a), runner.hash_binary(binary_b))

    def test_c_executor_uses_exactly_registered_path(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            binary_path = temp / "fake-amatl"
            binary_path.write_bytes(b"fake binary bytes")
            fixture_path = temp / "fixture.toml"
            fixture_path.write_text('[providers]\nenabled = ["searxng"]\n', encoding="utf-8")
            snapshot_path, config_sha256 = runner.prepare_config_snapshot(temp / "campaign", fixture_path)
            executor = runner.AmatlProcessExecutor(
                binary=binary_path, binary_sha256=runner.hash_binary(binary_path),
                config_snapshot=snapshot_path, config_sha256=config_sha256,
                queries={"Q01": "hello world"},
            )
            argv = executor.argv_for(runner.Position("bench", "searxng", "Q01", 1))
            self.assertEqual(argv[0], str(binary_path))

    def test_d_binary_hash_mismatch_fails_closed(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp = Path(temp_dir)
            binary_path = temp / "fake-amatl"
            binary_path.write_bytes(b"version-A-bytes")
            stale_hash = runner.hash_binary(binary_path)
            # Binary changed underneath us between manifest construction and
            # executor construction: fail closed rather than silently execute
            # different bytes than what was hashed into the manifest.
            binary_path.write_bytes(b"version-B-different-bytes")
            fixture_path = temp / "fixture.toml"
            fixture_path.write_text('[providers]\nenabled = ["searxng"]\n', encoding="utf-8")
            snapshot_path, config_sha256 = runner.prepare_config_snapshot(temp / "campaign", fixture_path)
            with self.assertRaises(runner.PlanAbort):
                runner.AmatlProcessExecutor(
                    binary=binary_path, binary_sha256=stale_hash,
                    config_snapshot=snapshot_path, config_sha256=config_sha256, queries={},
                )

    def test_e_missing_binary_fails_closed(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            missing = Path(temp_dir) / "does-not-exist"
            with self.assertRaises(runner.PlanAbort):
                runner.resolve_amatl_binary(missing)


class ClassificationSelfVerificationTests(unittest.TestCase):
    """[M3] validate_run_record must reject unknown/mismatched classifications
    by recomputing them through classify_execution() — not merely check type.
    """

    @staticmethod
    def _record(**overrides) -> dict[str, object]:
        base = {
            "sequence_number": 1, "benchmark_id": "b", "provider": "searxng", "query_id": "Q01",
            "repetition": 1, "process_exit_code": 0, "search_status": "SUCCESS",
            "timestamp_utc": datetime.now(timezone.utc).isoformat(), "classification": "success",
            "schema_version": runner.RUNS_SCHEMA_VERSION, "elapsed_ms": 10, "final_results": 5,
            "partial": False, "public_error": None,
        }
        base.update(overrides)
        return base

    def test_correct_classification_accepted(self):
        runner.validate_run_record(self._record())  # must not raise

    def test_unknown_classification_rejected(self):
        with self.assertRaises(runner.PlanAbort):
            runner.validate_run_record(self._record(classification="totally_unknown"))

    def test_known_but_incorrect_classification_rejected(self):
        # final_results=5, partial=False, exit 0 -> actually "success", not "zero_results".
        with self.assertRaises(runner.PlanAbort):
            runner.validate_run_record(self._record(classification="zero_results"))

    def test_negative_final_results_rejected(self):
        with self.assertRaises(runner.PlanAbort):
            runner.validate_run_record(self._record(final_results=-1, classification="success"))

    def test_executor_failure_with_non_null_partial_rejected(self):
        with self.assertRaises(runner.PlanAbort):
            runner.validate_run_record(self._record(
                process_exit_code=-1, search_status="EXECUTOR_FAILURE", final_results=None,
                partial=False, public_error="process_start_or_timeout", classification="executor_failure",
            ))

    def test_classification_recomputed_via_classify_execution(self):
        # Exercise every branch of classify_execution() through validate_run_record
        # to demonstrate it is driving the check, not a second copy of the rule.
        cases = [
            (0, "SUCCESS", 10, False, None, "success"),
            (0, "SUCCESS", 10, True, None, "partial_success"),
            (0, "SUCCESS", 0, False, None, "zero_results"),
            (1, "FAILURE", None, None, None, "executor_failure"),
            (0, "SUCCESS", 5, False, "provider_rate_limit", "provider_error"),
            (0, "SUCCESS", None, None, None, "unknown"),
        ]
        for exit_code, status, results, partial, error, expected in cases:
            with self.subTest(expected=expected):
                record = self._record(
                    process_exit_code=exit_code, search_status=status, final_results=results,
                    partial=partial, public_error=error, classification=expected,
                )
                runner.validate_run_record(record)  # must not raise
                with self.assertRaises(runner.PlanAbort):
                    runner.validate_run_record({**record, "classification": "unknown" if expected != "unknown" else "success"})


class PersistenceFailureHaltsExecutionTests(unittest.TestCase):
    """[L1] persistence failure must stop pacing and further execution, as a
    standalone regression test rather than only a script-level assertion.
    """

    def test_persistence_failure_stops_before_sleep_and_next_execution(self):
        validated = make_validated_plan(3)
        failure_position = validated.plan.positions[4]  # Q05-R1
        events: list[str] = []

        class FailingWriter(runner.DurableJSONLWriter):
            def append(self, record):
                if self.identity(record) == failure_position:
                    raise runner.PersistenceFailure("ABORT:PERSISTENCE_FAILURE")
                super().append(record)

        def sleep_fn(seconds: float) -> None:
            events.append(f"sleep:{seconds}")

        mock = runner.LocalMockExecutor(runner.Position("bench", "searxng", "Q99", 99))

        class RecordingExecutor:
            def __call__(self, position: runner.Position):
                events.append(f"executor:{position.query_id}-R{position.repetition}")
                return mock(position)

        with tempfile.TemporaryDirectory() as temp_dir:
            with FailingWriter(Path(temp_dir) / "runs.jsonl") as writer:
                with self.assertRaises(runner.PersistenceFailure):
                    runner.execute_plan(
                        validated, RecordingExecutor(), writer, interval_seconds=3, sleep_fn=sleep_fn,
                    )

        # The last thing that happened is the failing position's executor call:
        # no sleep followed it, and no further position was ever attempted.
        self.assertEqual(events[-1], "executor:Q05-R1")
        self.assertEqual(len(mock.invocations), 5)


class NoNetworkGuardTests(unittest.TestCase):
    def test_no_socket_module_used_for_execution(self):
        # LocalMockExecutor and execute_plan are pure in-process callables; this
        # test documents that assumption rather than intercepting sockets.
        validated = make_validated_plan(1, query_ids=("Q01",))
        mock = runner.LocalMockExecutor(runner.Position("bench", "searxng", "NONE", 0))
        state = runner.execute_plan(validated, mock)
        self.assertEqual(len(state.records), 1)
        self.assertEqual(mock.invocations, [validated.plan.positions[0]])


if __name__ == "__main__":
    unittest.main()
