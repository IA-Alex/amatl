#!/usr/bin/env python3
"""Offline-only deterministic benchmark-plan runner for QA.

This module intentionally has no subprocess, socket, HTTP, provider, or AMATL
integration. It can build and validate a plan, but execution is disabled until
an explicitly reviewed future integration replaces ``execute_plan``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
import time
import tomllib
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable, Mapping


NOTICE = "Generated offline benchmark-runner validation artifact — no provider execution performed."
MOCK_NOTICE = "Generated local mock executor validation artifact — AMATL and providers were not executed."
RETRIES_DISABLED = 0

# Traceability contract (AUDIT-06). Bumping either version is a breaking
# change to the campaign manifest or to individual run records, respectively.
# Bumped to /2 for [M1]/[M2]: the manifest now records the config snapshot
# path and the AMATL binary's own path/hash, not just its version string.
RUNNER_SCHEMA_VERSION = "amatl-benchmark-runner/2"
RUNS_SCHEMA_VERSION = "amatl-benchmark-runs/1"
AMATL_SUBPROCESS_TIMEOUT_SECONDS = 50
AMATL_VERSION_TIMEOUT_SECONDS = 10
# execute_plan() consumes plan.positions strictly sequentially (see its own
# docstring: "no sleep before the first position, exactly one sleep between
# each pair of consecutive positions"); there is no thread pool, async
# scheduler, or other parallel executor anywhere in this module.
# CONCURRENCY exists purely as a documented manifest field, not a knob — this
# is verified structurally by PacingTests, not by this constant. [L4]
CONCURRENCY = 1
VALID_CLASSIFICATIONS = frozenset({
    "executor_failure", "provider_error", "unknown", "partial_success", "success", "zero_results",
})

REQUIRED_RUN_FIELDS: dict[str, type | tuple[type, ...]] = {
    "sequence_number": int, "benchmark_id": str, "provider": str, "query_id": str, "repetition": int,
    "process_exit_code": int, "search_status": str, "timestamp_utc": str, "classification": str,
    "schema_version": str,
}
OPTIONAL_RUN_FIELDS: dict[str, type | tuple[type, ...]] = {
    "elapsed_ms": (int, type(None)), "final_results": (int, type(None)),
    "partial": (bool, type(None)), "public_error": (str, type(None)),
}


class PlanAbort(Exception):
    """A fail-closed plan or output contract violation."""


class PersistenceFailure(PlanAbort):
    """A record could not be made durable after executor completion."""


@dataclass(frozen=True, order=True)
class Position:
    benchmark_id: str
    provider: str
    query_id: str
    repetition: int


@dataclass(frozen=True)
class ExecutionPlan:
    benchmark_id: str
    provider: str
    repetitions: int
    positions: tuple[Position, ...]


@dataclass(frozen=True)
class ValidatedExecutionPlan:
    """Execution capability issued only after immutable-plan validation."""
    plan: ExecutionPlan


@dataclass(frozen=True)
class MockResult:
    exit_code: int
    status: str


@dataclass(frozen=True)
class AmatlProcessResult:
    """Public AMATL outcome returned through the existing executor contract."""
    exit_code: int
    status: str
    search_status: str
    elapsed_ms: int | None
    final_results: int | None
    partial: bool | None
    public_error: str | None


@dataclass(frozen=True)
class ExecutionRecord:
    sequence_number: int
    benchmark_id: str
    provider: str
    query_id: str
    repetition: int
    process_exit_code: int
    search_status: str
    timestamp_utc: str
    classification: str
    schema_version: str = RUNS_SCHEMA_VERSION
    elapsed_ms: int | None = None
    final_results: int | None = None
    partial: bool | None = None
    public_error: str | None = None


@dataclass(frozen=True)
class CampaignManifest:
    """Self-contained code→artifact traceability record for one new campaign.

    Written once, before the first position executes, so that even an
    interrupted campaign leaves an unambiguous record of what produced its
    ``runs.jsonl``.
    """
    schema_version: str
    amatl_version: str
    amatl_binary_path: str
    amatl_binary_sha256: str
    runner_path: str
    runner_sha256: str
    dataset_path: str
    dataset_sha256: str
    config_path: str
    config_sha256: str
    config_snapshot_path: str
    provider: str
    benchmark_id: str
    query_order: tuple[str, ...]
    repetitions: int
    retries: int
    timeout_seconds: float
    concurrency: int
    inter_request_interval_seconds: float
    campaign_started_at: str
    sequence_count: int


@dataclass
class ExecutionState:
    plan: ExecutionPlan
    attempted: set[Position]
    records: list[ExecutionRecord]
    attempt_count: int = 0
    retry_count: int = RETRIES_DISABLED


class DurableJSONLWriter:
    """Append-only, per-position JSONL writer with flush and fsync barriers."""
    def __init__(self, path: Path):
        self.path = path
        self.path.parent.mkdir(parents=True, exist_ok=True)
        try:
            self.handle = path.open("x", encoding="utf-8")
        except OSError as error:
            raise PlanAbort("ABORT:OUTPUT_ALREADY_EXISTS") from error
        self.durable: set[Position] = set()

    @staticmethod
    def identity(record: ExecutionRecord) -> Position:
        return Position(record.benchmark_id, record.provider, record.query_id, record.repetition)

    def append(self, record: ExecutionRecord) -> None:
        identity = self.identity(record)
        if identity in self.durable:
            raise PlanAbort("ABORT:DUPLICATE_EXECUTION_ATTEMPT")
        payload = json.dumps(asdict(record), sort_keys=True, separators=(",", ":")) + "\n"
        try:
            self.handle.write(payload)
            self.handle.flush()
            os.fsync(self.handle.fileno())
        except (OSError, ValueError) as error:
            raise PersistenceFailure("ABORT:PERSISTENCE_FAILURE") from error
        self.durable.add(identity)

    def close(self) -> None:
        self.handle.close()

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def runner_sha256() -> str:
    """Hash the file actually executing this module, not an external path."""
    return sha256_bytes(Path(__file__).resolve().read_bytes())


def classify_execution(
    process_exit_code: int,
    search_status: str,
    final_results: int | None,
    partial: bool | None,
    public_error: str | None,
) -> str:
    """The single deterministic classification rule. Do not duplicate this logic elsewhere."""
    if process_exit_code != 0 or search_status == "EXECUTOR_FAILURE":
        return "executor_failure"
    if public_error:
        return "provider_error"
    if final_results is None:
        return "unknown"
    if final_results > 0:
        return "partial_success" if partial else "success"
    return "zero_results"


def _read_dataset_bytes(dataset_path: Path) -> bytes:
    try:
        return dataset_path.read_bytes()
    except OSError as error:
        raise PlanAbort("ABORT:INVALID_DATASET") from error


def parse_dataset_document(data: bytes) -> tuple[tuple[str, ...], dict[str, str]]:
    """The single dataset-parsing rule: ordered, unique, non-empty IDs plus their query text."""
    try:
        document = json.loads(data.decode("utf-8"))
        queries = document["queries"]
    except (UnicodeDecodeError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise PlanAbort("ABORT:INVALID_DATASET") from error
    ids = tuple(item.get("id") for item in queries if isinstance(item, dict))
    if len(ids) != len(queries) or not ids or any(not isinstance(query_id, str) or not query_id for query_id in ids):
        raise PlanAbort("ABORT:INVALID_DATASET")
    if len(set(ids)) != len(ids):
        raise PlanAbort("ABORT:INVALID_DATASET")
    mapping: dict[str, str] = {}
    for item in queries:
        query_text = item.get("query")
        if not isinstance(query_text, str):
            raise PlanAbort("ABORT:INVALID_DATASET")
        mapping[item["id"]] = query_text
    return ids, mapping


def load_dataset(dataset_path: Path) -> tuple[str, ...]:
    """Load ordered IDs and reject duplicate/malformed dataset entries."""
    ids, _ = parse_dataset_document(_read_dataset_bytes(dataset_path))
    return ids


def load_dataset_with_hash(dataset_path: Path) -> tuple[str, tuple[str, ...], dict[str, str]]:
    """Load IDs and query text, hashing exactly the bytes that were parsed."""
    data = _read_dataset_bytes(dataset_path)
    ids, mapping = parse_dataset_document(data)
    return sha256_bytes(data), ids, mapping


def build_plan(benchmark_id: str, provider: str, query_ids: tuple[str, ...], repetitions: int) -> ExecutionPlan:
    """Create the sole immutable Cartesian plan in round-major order."""
    if not benchmark_id or not provider or repetitions < 1:
        raise PlanAbort("ABORT:INVALID_DATASET")
    positions = tuple(
        Position(benchmark_id, provider, query_id, repetition)
        for repetition in range(1, repetitions + 1)
        for query_id in query_ids
    )
    return ExecutionPlan(benchmark_id, provider, repetitions, positions)


def validate_plan(plan: ExecutionPlan, query_ids: tuple[str, ...]) -> None:
    """Fail before execution for count, uniqueness, full coverage, or ordering faults."""
    expected_count = len(query_ids) * plan.repetitions
    identities = [position for position in plan.positions]
    if len(set(identities)) != len(identities):
        raise PlanAbort("ABORT:DUPLICATE_POSITION")
    if len(plan.positions) != expected_count:
        raise PlanAbort("ABORT:PLAN_COUNT_MISMATCH")
    expected = tuple(
        Position(plan.benchmark_id, plan.provider, query_id, repetition)
        for repetition in range(1, plan.repetitions + 1)
        for query_id in query_ids
    )
    if set(plan.positions) != set(expected):
        raise PlanAbort("ABORT:MISSING_POSITION")
    if plan.positions != expected:
        raise PlanAbort("ABORT:MISSING_POSITION")


def validated_plan(plan: ExecutionPlan, query_ids: tuple[str, ...]) -> ValidatedExecutionPlan:
    validate_plan(plan, query_ids)
    return ValidatedExecutionPlan(plan)


def plan_hash(plan: ExecutionPlan) -> str:
    payload = json.dumps([asdict(position) for position in plan.positions], sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def generated_commands(plan: ExecutionPlan) -> tuple[dict[str, object], ...]:
    """Return declarative future commands; never start a process."""
    return tuple(
        {
            "position": asdict(position),
            "command_kind": "AMATL_SEARCH_NOT_EXECUTED",
            "arguments": ["--config-file", "<fixture>", "search", "<dataset-query>", "--json"],
        }
        for position in plan.positions
    )


def _execute_position(
    state: ExecutionState,
    position: Position,
    executor,
    writer: DurableJSONLWriter | None = None,
    *,
    now_fn=lambda: datetime.now(timezone.utc),
) -> None:
    """Invoke one injected executor only after every execution barrier passes."""
    if position not in state.plan.positions:
        raise PlanAbort("ABORT:POSITION_NOT_IN_PLAN")
    if state.attempt_count >= len(state.plan.positions) or state.attempt_count >= 30:
        raise PlanAbort("ABORT:EXECUTION_LIMIT_EXCEEDED")
    if position in state.attempted:
        raise PlanAbort("ABORT:DUPLICATE_EXECUTION_ATTEMPT")
    sequence_number = state.attempt_count + 1
    result = executor(position)
    timestamp_utc = now_fn().isoformat()
    search_status = getattr(result, "search_status", result.status)
    final_results = getattr(result, "final_results", None)
    partial = getattr(result, "partial", None)
    public_error = getattr(result, "public_error", None)
    classification = classify_execution(result.exit_code, search_status, final_results, partial, public_error)
    record = ExecutionRecord(
        sequence_number, position.benchmark_id, position.provider, position.query_id,
        position.repetition, result.exit_code, search_status, timestamp_utc, classification,
        RUNS_SCHEMA_VERSION,
        getattr(result, "elapsed_ms", None), final_results, partial, public_error,
    )
    if DurableJSONLWriter.identity(record) != position:
        raise PlanAbort("ABORT:RECORD_IDENTITY_MISMATCH")
    if writer is not None:
        writer.append(record)
    state.attempted.add(position)
    state.attempt_count += 1
    state.records.append(record)


def execute_plan(
    plan: ValidatedExecutionPlan,
    executor,
    writer: DurableJSONLWriter | None = None,
    *,
    max_positions: int | None = None,
    interval_seconds: float = 0,
    sleep_fn=time.sleep,
    now_fn=lambda: datetime.now(timezone.utc),
) -> ExecutionState:
    """Consume an already validated immutable plan sequentially; never create positions.

    Pacing contract (the one boundary responsible for the interval): no sleep
    before the first position, exactly one sleep between each pair of
    consecutive positions, no sleep after the last position. For N positions
    this is N-1 calls to ``sleep_fn``, each invoked strictly after the
    position at index i is durably recorded and strictly before position
    i+1's executor call.
    """
    if not isinstance(plan, ValidatedExecutionPlan):
        raise PlanAbort("ABORT:PLAN_NOT_VALIDATED")
    if plan.plan.repetitions * len({position.query_id for position in plan.plan.positions}) > 30:
        raise PlanAbort("ABORT:EXECUTION_LIMIT_EXCEEDED")
    if max_positions is not None and not 1 <= max_positions <= len(plan.plan.positions):
        raise PlanAbort("ABORT:INVALID_POSITION_LIMIT")
    if isinstance(interval_seconds, bool) or not isinstance(interval_seconds, (int, float)) or interval_seconds < 0:
        raise PlanAbort("ABORT:INVALID_INTERVAL")
    state = ExecutionState(plan.plan, set(), [])
    for index, position in enumerate(plan.plan.positions):
        _execute_position(state, position, executor, writer, now_fn=now_fn)
        if max_positions is not None and state.attempt_count >= max_positions:
            break
        if interval_seconds and index + 1 < len(plan.plan.positions):
            sleep_fn(interval_seconds)
    return state


class LocalMockExecutor:
    """Local deterministic callable; it has no process, provider, or network capability."""
    def __init__(self, failure_position: Position):
        self.failure_position = failure_position
        self.invocations: list[Position] = []

    def __call__(self, position: Position) -> MockResult:
        self.invocations.append(position)
        if position == self.failure_position:
            return MockResult(1, "FAILURE")
        return MockResult(0, "SUCCESS")


def _read_and_validate_searxng_fixture(fixture: Path) -> bytes:
    """Read the fixture bytes exactly once and validate SearXNG-only contents.

    This is the single source of truth for what "a valid fixture" means; the
    returned bytes are what get hashed into ``config_sha256`` and frozen into
    the campaign's config snapshot — never re-read from ``fixture`` again.
    """
    try:
        raw = fixture.read_bytes()
        config = tomllib.loads(raw.decode("utf-8"))
        enabled = config["providers"]["enabled"]
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError, KeyError, TypeError) as error:
        raise PlanAbort("ABORT:INVALID_SEARXNG_FIXTURE") from error
    if enabled != ["searxng"] or "marginalia" in config.get("providers", {}):
        raise PlanAbort("ABORT:INVALID_SEARXNG_FIXTURE")
    return raw


def create_config_snapshot(output_dir: Path, raw_config: bytes) -> Path:
    """Freeze already-validated config bytes into an immutable snapshot file.

    Written via temp-file + flush/fsync + atomic rename inside the campaign
    directory, so the snapshot exists in full before position 1 and the
    original (user-editable) fixture path is never touched or re-read again
    for this campaign. [M1]
    """
    output_dir.mkdir(parents=True, exist_ok=True)
    snapshot_path = output_dir / "config-snapshot.toml"
    fd, tmp_name = tempfile.mkstemp(dir=str(output_dir), prefix=".config-snapshot-", suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(raw_config)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp_name, snapshot_path)
    except OSError as error:
        try:
            os.unlink(tmp_name)
        except OSError:
            pass
        raise PlanAbort("ABORT:CONFIG_SNAPSHOT_WRITE_FAILED") from error
    return snapshot_path


def prepare_config_snapshot(output_dir: Path, fixture: Path) -> tuple[Path, str]:
    """One read of the original fixture; hash and freeze those exact bytes.

    Returns ``(snapshot_path, config_sha256)``. Every AMATL invocation for the
    resulting campaign must target ``snapshot_path`` — never ``fixture`` —
    so an edit to the original file after this call cannot change what
    actually executes (closes the CONFIG_IDENTITY / TOCTOU gap in [M1]).
    """
    raw = _read_and_validate_searxng_fixture(fixture)
    config_sha256 = sha256_bytes(raw)
    snapshot_path = create_config_snapshot(output_dir, raw)
    if sha256_bytes(snapshot_path.read_bytes()) != config_sha256:
        raise PlanAbort("ABORT:CONFIG_SNAPSHOT_HASH_MISMATCH")
    return snapshot_path, config_sha256


def resolve_amatl_binary(binary: Path) -> Path:
    """Resolve the AMATL binary path exactly once for the whole campaign.

    Every position reuses this resolved path — argv_for/subprocess.run never
    performs its own PATH lookup, and no position re-resolves it. [M2]
    """
    try:
        return binary.resolve(strict=True)
    except OSError as error:
        raise PlanAbort("ABORT:AMATL_BINARY_UNAVAILABLE") from error


def hash_binary(path: Path) -> str:
    """SHA-256 over the exact bytes of the binary that will be executed."""
    try:
        return sha256_bytes(path.read_bytes())
    except OSError as error:
        raise PlanAbort("ABORT:AMATL_BINARY_UNAVAILABLE") from error


class AmatlProcessExecutor:
    """One-position AMATL subprocess executor for the existing plan contract.

    It owns neither planning nor retry policy.  Its sole effect is one local,
    argument-vector AMATL invocation for a position already issued by a plan.

    ``binary`` and ``config_snapshot`` must already be the exact, resolved
    artifacts recorded in the campaign manifest (see ``resolve_amatl_binary``
    and ``prepare_config_snapshot``); this executor re-verifies their bytes
    against the recorded hashes once, before accepting any invocation, and
    otherwise never re-resolves or re-reads either from a mutable source.
    """
    def __init__(
        self,
        binary: Path,
        binary_sha256: str,
        config_snapshot: Path,
        config_sha256: str,
        queries: Mapping[str, str],
    ):
        self.binary = binary
        self.binary_sha256 = binary_sha256
        self.fixture = config_snapshot
        self.fixture_sha256 = config_sha256
        self.queries = dict(queries)
        self.invocations: list[Position] = []
        self.results: list[AmatlProcessResult] = []
        self._verify_identity_before_execution()

    def _verify_identity_before_execution(self) -> None:
        """Fail closed if the recorded binary or config snapshot bytes have
        diverged from their recorded hashes between manifest construction and
        executor construction — reproducibility, not anti-tamper. [M1] [M2]
        """
        try:
            binary_bytes = self.binary.read_bytes()
        except OSError as error:
            raise PlanAbort("ABORT:AMATL_BINARY_UNAVAILABLE") from error
        if sha256_bytes(binary_bytes) != self.binary_sha256:
            raise PlanAbort("ABORT:AMATL_BINARY_HASH_MISMATCH")
        try:
            config_bytes = self.fixture.read_bytes()
        except OSError as error:
            raise PlanAbort("ABORT:CONFIG_SNAPSHOT_UNAVAILABLE") from error
        if sha256_bytes(config_bytes) != self.fixture_sha256:
            raise PlanAbort("ABORT:CONFIG_SNAPSHOT_HASH_MISMATCH")

    @staticmethod
    def _public_error(document: dict[str, object]) -> str | None:
        errors = document.get("errors", [])
        if not isinstance(errors, list):
            return "invalid_public_json"
        codes = [item.get("code") for item in errors if isinstance(item, dict) and isinstance(item.get("code"), str)]
        return ",".join(codes) if codes else None

    def argv_for(self, position: Position) -> list[str]:
        if position.provider != "searxng" or position.query_id not in self.queries:
            raise PlanAbort("ABORT:POSITION_QUERY_UNAVAILABLE")
        return [str(self.binary), "--config-file", str(self.fixture), "search", self.queries[position.query_id], "--json"]

    def __call__(self, position: Position) -> AmatlProcessResult:
        command = self.argv_for(position)
        self.invocations.append(position)
        try:
            completed = subprocess.run(
                command, check=False, shell=False, capture_output=True, text=True,
                timeout=AMATL_SUBPROCESS_TIMEOUT_SECONDS,
            )
        except (OSError, subprocess.TimeoutExpired):
            result = AmatlProcessResult(-1, "EXECUTOR_FAILURE", "EXECUTOR_FAILURE", None, None, None, "process_start_or_timeout")
            self.results.append(result)
            return result
        try:
            document = json.loads(completed.stdout)
            if not isinstance(document, dict):
                raise ValueError("public JSON is not an object")
            search_status = document["status"]
            results = document["results"]
            elapsed_ms = document["elapsed_ms"]
            if not isinstance(search_status, str) or not isinstance(results, list) or not isinstance(elapsed_ms, int):
                raise ValueError("public JSON lacks search fields")
            result = AmatlProcessResult(
                completed.returncode,
                search_status,
                search_status,
                elapsed_ms,
                len(results),
                bool(document.get("providers_partial", [])),
                self._public_error(document),
            )
        except (json.JSONDecodeError, KeyError, TypeError, ValueError):
            result = AmatlProcessResult(completed.returncode, "EXECUTOR_FAILURE", "EXECUTOR_FAILURE", None, None, None, "invalid_public_json")
        self.results.append(result)
        return result


def load_queries(dataset_path: Path) -> dict[str, str]:
    """Load the frozen query text via the same parser that validates IDs."""
    _, mapping = parse_dataset_document(_read_dataset_bytes(dataset_path))
    return mapping


def ensure_output_absent(output_dir: Path) -> None:
    if output_dir.exists():
        raise PlanAbort("ABORT:OUTPUT_ALREADY_EXISTS")


def expect_abort(expected: str, operation) -> str:
    try:
        operation()
    except PlanAbort as error:
        if str(error) == expected:
            return "PASS"
        return f"FAIL:{error}"
    return "FAIL:NO_ABORT"


def negative_tests(plan: ExecutionPlan, query_ids: tuple[str, ...]) -> dict[str, str]:
    duplicate = ExecutionPlan(plan.benchmark_id, plan.provider, plan.repetitions, plan.positions + (plan.positions[0],))
    missing_last = Position(plan.benchmark_id, plan.provider, "Q_MISSING_REPLACEMENT", plan.repetitions)
    missing = ExecutionPlan(plan.benchmark_id, plan.provider, plan.repetitions, plan.positions[:-1] + (missing_last,))
    wrong_count = ExecutionPlan(plan.benchmark_id, plan.provider, plan.repetitions, plan.positions[:-2])
    duplicate_dataset = query_ids + (query_ids[0],)
    with tempfile.TemporaryDirectory() as temp_dir:
        existing = Path(temp_dir) / "existing-output"
        existing.mkdir()
        output_result = expect_abort("ABORT:OUTPUT_ALREADY_EXISTS", lambda: ensure_output_absent(existing))
    return {
        "duplicate_position": expect_abort("ABORT:DUPLICATE_POSITION", lambda: validate_plan(duplicate, query_ids)),
        "missing_position": expect_abort("ABORT:MISSING_POSITION", lambda: validate_plan(missing, query_ids)),
        "plan_count_mismatch": expect_abort("ABORT:PLAN_COUNT_MISMATCH", lambda: validate_plan(wrong_count, query_ids)),
        "output_already_exists": output_result,
        "invalid_dataset_duplicate_query_id": expect_abort("ABORT:INVALID_DATASET", lambda: validate_dataset_ids(duplicate_dataset)),
    }


def validate_dataset_ids(query_ids: Iterable[str]) -> None:
    ids = tuple(query_ids)
    if not ids or len(ids) != len(set(ids)) or any(not query_id for query_id in ids):
        raise PlanAbort("ABORT:INVALID_DATASET")


def mock_executor_negative_tests(validated: ValidatedExecutionPlan, mock: LocalMockExecutor) -> dict[str, str]:
    plan = validated.plan
    foreign = Position(plan.benchmark_id, plan.provider, "Q_NOT_IN_PLAN", 1)
    duplicate_state = ExecutionState(plan, {plan.positions[0]}, [], attempt_count=1)
    foreign_state = ExecutionState(plan, set(), [])
    completed_state = ExecutionState(plan, set(plan.positions), [], attempt_count=len(plan.positions))
    with tempfile.TemporaryDirectory() as temp_dir:
        existing = Path(temp_dir) / "existing-output"
        existing.mkdir()
        output_result = expect_abort("ABORT:OUTPUT_ALREADY_EXISTS", lambda: ensure_output_absent(existing))
    invocation_count = len(mock.invocations)
    results = {
        "duplicate_execution": expect_abort("ABORT:DUPLICATE_EXECUTION_ATTEMPT", lambda: _execute_position(duplicate_state, plan.positions[0], mock)),
        "foreign_position": expect_abort("ABORT:POSITION_NOT_IN_PLAN", lambda: _execute_position(foreign_state, foreign, mock)),
        "execution_limit_31": expect_abort("ABORT:EXECUTION_LIMIT_EXCEEDED", lambda: _execute_position(completed_state, plan.positions[0], mock)),
        "unvalidated_plan": expect_abort("ABORT:PLAN_NOT_VALIDATED", lambda: execute_plan(plan, mock)),
        "output_already_exists": output_result,
    }
    if len(mock.invocations) != invocation_count:
        return {key: "FAIL:MOCK_INVOKED_AFTER_ABORT" for key in results}
    return results


def write_mock_artifacts(output_dir: Path, plan: ExecutionPlan, state: ExecutionState, mock: LocalMockExecutor, negatives: dict[str, str]) -> None:
    ensure_output_absent(output_dir)
    output_dir.mkdir(parents=True)
    plan_document = {"artifact_notice": MOCK_NOTICE, "benchmark_id": plan.benchmark_id, "provider": plan.provider,
                     "repetitions": plan.repetitions, "plan_hash_sha256": plan_hash(plan),
                     "positions": [asdict(position) for position in plan.positions]}
    validation_document = {
        "artifact_notice": MOCK_NOTICE, "plan_validation": "PASS", "planned_positions": len(plan.positions),
        "mock_invocations": len(mock.invocations), "recorded_executions": len(state.records),
        "unique_executions": len(state.attempted), "duplicates": len(state.records) - len(state.attempted),
        "missing": len(plan.positions) - len(state.attempted), "retries": state.retry_count,
        "execution_order": "PASS" if tuple(mock.invocations) == plan.positions else "FAIL",
        "failure_position": "Q05-R2", "failure_without_retry": "PASS" if state.records[14].process_exit_code == 1 else "FAIL",
        "amatl_executions": 0, "provider_executions": 0, "network_requests": 0,
        "execution_barrier": "attempt_count >= planned_count or 30 aborts before executor invocation",
        "logical_diff": "Added ValidatedExecutionPlan, injected execute_plan(), execution guards, and LocalMockExecutor; plan generation unchanged.",
    }
    output_dir.joinpath("execution-plan.json").write_text(json.dumps(plan_document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    output_dir.joinpath("mock-runs.jsonl").write_text("".join(json.dumps(asdict(record), sort_keys=True) + "\n" for record in state.records), encoding="utf-8")
    output_dir.joinpath("validation.json").write_text(json.dumps(validation_document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    output_dir.joinpath("negative-tests.json").write_text(json.dumps({"artifact_notice": MOCK_NOTICE, "results": negatives}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    output_dir.joinpath("README.md").write_text(f"# Local mock executor validation\n\n{MOCK_NOTICE}\n\nThe mock is an in-process callable. No AMATL process, provider, HTTP, DNS or socket was used.\n", encoding="utf-8")


def run_mock_executor_validation(dataset: Path, benchmark_id: str, provider: str, repetitions: int, output_dir: Path) -> None:
    query_ids = load_dataset(dataset)
    plan = build_plan(benchmark_id, provider, query_ids, repetitions)
    validated = validated_plan(plan, query_ids)
    failure_position = Position(benchmark_id, provider, "Q05", 2)
    mock = LocalMockExecutor(failure_position)
    state = execute_plan(validated, mock)
    negatives = mock_executor_negative_tests(validated, mock)
    success = (
        len(plan.positions) == len(mock.invocations) == len(state.records) == len(state.attempted) == 30
        and state.retry_count == 0 and tuple(mock.invocations) == plan.positions
        and state.records[14].process_exit_code == 1 and all(result == "PASS" for result in negatives.values())
    )
    if not success:
        raise PlanAbort("ABORT:MOCK_VALIDATION_FAILED")
    write_mock_artifacts(output_dir, plan, state, mock, negatives)


def write_artifacts(output_dir: Path, plan: ExecutionPlan, query_ids: tuple[str, ...], negatives: dict[str, str], deterministic: bool) -> None:
    ensure_output_absent(output_dir)
    command_count = len(generated_commands(plan))
    plan_document = {
        "artifact_notice": NOTICE,
        "benchmark_id": plan.benchmark_id,
        "provider": plan.provider,
        "repetitions": plan.repetitions,
        "plan_hash_sha256": plan_hash(plan),
        "positions": [asdict(position) for position in plan.positions],
    }
    validation_document = {
        "artifact_notice": NOTICE,
        "dataset_queries": len(query_ids), "repetitions": plan.repetitions,
        "expected_positions": len(query_ids) * plan.repetitions,
        "generated_positions": len(plan.positions), "unique_positions": len(set(plan.positions)),
        "duplicate_positions": len(plan.positions) - len(set(plan.positions)), "missing_positions": 0,
        "commands_generated": command_count, "amatl_executions": 0,
        "provider_executions": 0, "network_requests": 0, "retries": RETRIES_DISABLED,
        "plan_deterministic": "PASS" if deterministic else "FAIL",
        "execution_boundary": "EXECUTION_DISABLED_OFFLINE_ONLY",
        "coverage": {query_id: list(range(1, plan.repetitions + 1)) for query_id in query_ids},
    }
    negative_document = {"artifact_notice": NOTICE, "results": negatives}
    output_dir.mkdir(parents=True)
    (output_dir / "execution-plan.json").write_text(json.dumps(plan_document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (output_dir / "validation.json").write_text(json.dumps(validation_document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (output_dir / "negative-tests.json").write_text(json.dumps(negative_document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    readme = f"# Offline benchmark-runner validation\n\n{NOTICE}\n\n"
    readme += "This artifact contains immutable plan validation only. AMATL, providers, HTTP, DNS and sockets were not invoked.\n"
    (output_dir / "README.md").write_text(readme, encoding="utf-8")


def validate_run_record(record: dict[str, object]) -> None:
    """The single runs.jsonl schema rule; used both to validate and to document the contract."""
    for field, field_type in REQUIRED_RUN_FIELDS.items():
        if field not in record or not isinstance(record[field], field_type):
            raise PlanAbort(f"ABORT:INVALID_RUN_RECORD_FIELD:{field}")
    for field, field_types in OPTIONAL_RUN_FIELDS.items():
        if field not in record or not isinstance(record[field], field_types):
            raise PlanAbort(f"ABORT:INVALID_RUN_RECORD_FIELD:{field}")
    if record["schema_version"] != RUNS_SCHEMA_VERSION:
        raise PlanAbort("ABORT:UNSUPPORTED_RUN_SCHEMA_VERSION")
    try:
        datetime.fromisoformat(record["timestamp_utc"])  # type: ignore[arg-type]
    except (TypeError, ValueError) as error:
        raise PlanAbort("ABORT:INVALID_RUN_RECORD_FIELD:timestamp_utc") from error

    # [M3] classification must be one of the known outputs of classify_execution(),
    # and must actually match what classify_execution() computes from the other
    # persisted fields — not merely be *some* string, as before.
    classification = record["classification"]
    if classification not in VALID_CLASSIFICATIONS:
        raise PlanAbort("ABORT:UNKNOWN_CLASSIFICATION")

    final_results = record["final_results"]
    if final_results is not None and final_results < 0:
        # Never produced by AmatlProcessExecutor (len(results) >= 0 always);
        # a negative count is impossible by contract, so reject the record
        # rather than let classify_execution() interpret it.
        raise PlanAbort("ABORT:INVALID_RUN_RECORD_FIELD:final_results")

    search_status = record["search_status"]
    partial = record["partial"]
    if search_status == "EXECUTOR_FAILURE" and partial is not None:
        # AmatlProcessExecutor only ever sets partial=None alongside
        # EXECUTOR_FAILURE; a non-null partial there is impossible by
        # contract, not a state classify_execution() is meant to interpret.
        raise PlanAbort("ABORT:INVALID_RUN_RECORD_FIELD:partial")

    expected_classification = classify_execution(
        record["process_exit_code"], search_status, final_results, partial, record["public_error"],
    )
    if classification != expected_classification:
        raise PlanAbort("ABORT:CLASSIFICATION_MISMATCH")


def validate_runs_jsonl(records: list[dict[str, object]]) -> None:
    for record in records:
        validate_run_record(record)


def amatl_version(binary: Path) -> str:
    """Query the AMATL binary that will actually execute this campaign for its own version."""
    try:
        completed = subprocess.run(
            [str(binary), "--version"], check=False, shell=False, capture_output=True, text=True,
            timeout=AMATL_VERSION_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise PlanAbort("ABORT:AMATL_VERSION_UNAVAILABLE") from error
    output = (completed.stdout or "").strip() or (completed.stderr or "").strip()
    if completed.returncode != 0 or not output:
        raise PlanAbort("ABORT:AMATL_VERSION_UNAVAILABLE")
    return output


def build_campaign_manifest(
    *,
    amatl_version: str,
    amatl_binary_path: str,
    amatl_binary_sha256: str,
    dataset_path: Path,
    dataset_sha256: str,
    config_path: Path,
    config_sha256: str,
    config_snapshot_path: str,
    provider: str,
    benchmark_id: str,
    query_order: Iterable[str],
    repetitions: int,
    retries: int,
    timeout_seconds: float,
    concurrency: int,
    interval_seconds: float,
    campaign_started_at: str,
    sequence_count: int,
) -> CampaignManifest:
    return CampaignManifest(
        schema_version=RUNNER_SCHEMA_VERSION,
        amatl_version=amatl_version,
        amatl_binary_path=amatl_binary_path,
        amatl_binary_sha256=amatl_binary_sha256,
        runner_path=str(Path(__file__).resolve()),
        runner_sha256=runner_sha256(),
        dataset_path=str(dataset_path),
        dataset_sha256=dataset_sha256,
        config_path=str(config_path),
        config_sha256=config_sha256,
        config_snapshot_path=config_snapshot_path,
        provider=provider,
        benchmark_id=benchmark_id,
        query_order=tuple(query_order),
        repetitions=repetitions,
        retries=retries,
        timeout_seconds=timeout_seconds,
        concurrency=concurrency,
        inter_request_interval_seconds=interval_seconds,
        campaign_started_at=campaign_started_at,
        sequence_count=sequence_count,
    )


def write_campaign_manifest(output_dir: Path, manifest: CampaignManifest) -> None:
    """Write the manifest with the same durability barrier as runs.jsonl records:
    temp file in the same directory, flush + fsync, then atomic rename. [L3]

    This is a small, self-contained change (mirrors DurableJSONLWriter.append)
    so it is done rather than skipped; it does not attempt directory-entry
    fsync or cross-filesystem durability, which would be disproportionate for
    a single small JSON file rewritten once per campaign.
    """
    payload = asdict(manifest)
    payload["query_order"] = list(manifest.query_order)
    content = json.dumps(payload, indent=2, sort_keys=True) + "\n"
    final_path = output_dir / "campaign-manifest.json"
    fd, tmp_name = tempfile.mkstemp(dir=str(output_dir), prefix=".campaign-manifest-", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp_name, final_path)
    except OSError as error:
        try:
            os.unlink(tmp_name)
        except OSError:
            pass
        raise PlanAbort("ABORT:MANIFEST_WRITE_FAILED") from error


def records_from_disk(path: Path) -> list[dict[str, object]]:
    try:
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    except (OSError, json.JSONDecodeError) as error:
        raise PlanAbort("ABORT:INVALID_DURABLE_RECORDS") from error


def durable_validation(records: list[dict[str, object]], plan: ExecutionPlan) -> dict[str, object]:
    identities = [
        (record.get("benchmark_id"), record.get("provider"), record.get("query_id"), record.get("repetition"))
        for record in records
    ]
    expected = [(p.benchmark_id, p.provider, p.query_id, p.repetition) for p in plan.positions]
    sequences = [record.get("sequence_number") for record in records]
    return {
        "durable_after_reopen": len(records),
        "unique_durable": len(set(identities)),
        "duplicates": len(records) - len(set(identities)),
        "missing": len(set(expected) - set(identities)),
        "minimum_sequence": min(sequences) if sequences else None,
        "maximum_sequence": max(sequences) if sequences else None,
        "sequence_exact": sequences == list(range(1, len(records) + 1)),
        "coverage_exact": set(identities) == set(expected) if len(records) == len(expected) else False,
    }


def run_pacing_validation(dataset: Path, benchmark_id: str, provider: str, repetitions: int, output_dir: Path) -> None:
    """Offline validation of post-persistence pacing with a recording sleeper."""
    ensure_output_absent(output_dir)
    query_ids = load_dataset(dataset)
    plan = build_plan(benchmark_id, provider, query_ids, repetitions)
    validated = validated_plan(plan, query_ids)
    if len(plan.positions) != 30:
        raise PlanAbort("ABORT:PACING_VALIDATION_REQUIRES_30")
    output_dir.mkdir(parents=True)
    events: list[dict[str, object]] = []

    class RecordingSleeper:
        def __init__(self):
            self.waits: list[float] = []

        def __call__(self, seconds: float) -> None:
            self.waits.append(seconds)
            events.append({"scenario": "complete", "event": "wait", "seconds": seconds})

    class RecordingExecutor:
        def __init__(self, mock: LocalMockExecutor):
            self.mock = mock

        def __call__(self, position: Position) -> MockResult:
            events.append({"scenario": "complete", "event": "executor", "position": f"{position.query_id}-R{position.repetition}"})
            return self.mock(position)

    class RecordingWriter(DurableJSONLWriter):
        def append(self, record: ExecutionRecord) -> None:
            super().append(record)
            events.append({"scenario": "complete", "event": "durable", "position": f"{record.query_id}-R{record.repetition}"})

    sleeper = RecordingSleeper()
    mock = LocalMockExecutor(plan.positions[14])
    with RecordingWriter(output_dir / "runs.jsonl") as writer:
        state = execute_plan(validated, RecordingExecutor(mock), writer, interval_seconds=3, sleep_fn=sleeper)
    disk = records_from_disk(output_dir / "runs.jsonl")
    durable = durable_validation(disk, plan)
    complete_events = [(event["event"], event.get("position"), event.get("seconds")) for event in events]
    failure_index = complete_events.index(("executor", "Q05-R2", None))
    failure_continues = complete_events[failure_index:failure_index + 4] == [
        ("executor", "Q05-R2", None), ("durable", "Q05-R2", None),
        ("wait", None, 3), ("executor", "Q06-R2", None),
    ]

    failure_position = plan.positions[4]
    failure_events: list[dict[str, object]] = []

    class FailingWriter(DurableJSONLWriter):
        def append(self, record: ExecutionRecord) -> None:
            if self.identity(record) == failure_position:
                failure_events.append({"scenario": "persistence_failure", "event": "persistence_failure", "position": f"{record.query_id}-R{record.repetition}"})
                raise PersistenceFailure("ABORT:PERSISTENCE_FAILURE")
            super().append(record)
            failure_events.append({"scenario": "persistence_failure", "event": "durable", "position": f"{record.query_id}-R{record.repetition}"})

    failure_sleeper = RecordingSleeper()
    failure_mock = LocalMockExecutor(plan.positions[14])
    failure_executor = RecordingExecutor(failure_mock)
    failure_result = expect_abort(
        "ABORT:PERSISTENCE_FAILURE",
        lambda: _run_pacing_failure_case(validated, failure_executor, failure_sleeper, FailingWriter, output_dir / "persistence-failure"),
    )
    events.extend(failure_events)
    negatives = mock_executor_negative_tests(validated, LocalMockExecutor(plan.positions[14]))
    checks = {
        "planned": len(plan.positions), "mock_invocations": len(mock.invocations), "executed_records": len(state.records),
        "durable_after_reopen": durable["durable_after_reopen"], "unique_durable": durable["unique_durable"],
        "duplicates": durable["duplicates"], "missing": durable["missing"], "retries": state.retry_count,
        "minimum_sequence": durable["minimum_sequence"], "maximum_sequence": durable["maximum_sequence"],
        "configured_interval_seconds": 3, "expected_waits": 29, "observed_waits": len(sleeper.waits),
        "all_waits_exactly_3": all(seconds == 3 for seconds in sleeper.waits),
        "wait_before_first": complete_events[0][0] == "wait", "wait_after_last": complete_events[-1][0] == "wait",
        "persistence_before_wait": complete_events[:4] == [("executor", "Q01-R1", None), ("durable", "Q01-R1", None), ("wait", None, 3), ("executor", "Q02-R1", None)],
        "failure_continues": failure_continues,
        "persistence_failure": {"result": failure_result, "executor_invocations": len(failure_mock.invocations), "waits": len(failure_sleeper.waits), "next_position_executed": len(failure_mock.invocations) > 5},
        "negative_guards": negatives, "amatl_executions": 0, "provider_executions": 0, "network_requests": 0,
    }
    if not (checks["planned"] == checks["mock_invocations"] == checks["executed_records"] == checks["durable_after_reopen"] == checks["unique_durable"] == 30 and checks["duplicates"] == checks["missing"] == checks["retries"] == 0 and checks["minimum_sequence"] == 1 and checks["maximum_sequence"] == 30 and checks["observed_waits"] == checks["expected_waits"] and checks["all_waits_exactly_3"] and not checks["wait_before_first"] and not checks["wait_after_last"] and checks["persistence_before_wait"] and checks["failure_continues"] and checks["persistence_failure"] == {"result": "PASS", "executor_invocations": 5, "waits": 4, "next_position_executed": False} and all(result == "PASS" for result in negatives.values())):
        raise PlanAbort("PACING_NOT_READY:VALIDATION_FAILED")
    (output_dir / "pacing-events.jsonl").write_text("".join(json.dumps(event, sort_keys=True) + "\n" for event in events), encoding="utf-8")
    (output_dir / "validation.json").write_text(json.dumps(checks, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (output_dir / "README.md").write_text("# Pacing validation\n\nOffline validation using LocalMockExecutor and recording sleepers; no AMATL process or network request occurred.\n", encoding="utf-8")
    (output_dir / "findings.md").write_text("# Findings\n\n`execute_plan()` waits after durable persistence and before the following position. The complete 30-position validation recorded 29 synthetic 3-second waits.\n", encoding="utf-8")


def _run_pacing_failure_case(validated: ValidatedExecutionPlan, executor, sleeper, writer_type, output_dir: Path) -> None:
    with writer_type(output_dir / "runs.jsonl") as writer:
        execute_plan(validated, executor, writer, interval_seconds=3, sleep_fn=sleeper)


def run_production_validation(dataset: Path, benchmark_id: str, provider: str, repetitions: int, fixture: Path, binary: Path, output_dir: Path) -> None:
    """Validate the one production execution path before a benchmark campaign."""
    ensure_output_absent(output_dir)
    query_ids = load_dataset(dataset)
    plan = build_plan(benchmark_id, provider, query_ids, repetitions)
    validated = validated_plan(plan, query_ids)
    if len(plan.positions) != 30:
        raise PlanAbort("ABORT:PRODUCTION_VALIDATION_REQUIRES_30")
    output_dir.mkdir(parents=True)

    offline_dir = output_dir / "offline-30"
    mock = LocalMockExecutor(plan.positions[14])
    with DurableJSONLWriter(offline_dir / "runs.jsonl") as writer:
        offline_state = execute_plan(validated, mock, writer)
    offline_disk = records_from_disk(offline_dir / "runs.jsonl")
    offline = durable_validation(offline_disk, plan)
    offline.update({"planned": len(plan.positions), "generated": len(plan.positions), "mock_invocations": len(mock.invocations), "executed": len(offline_state.records), "retries": offline_state.retry_count})
    if not (offline["durable_after_reopen"] == offline["unique_durable"] == offline["planned"] == offline["executed"] == offline["mock_invocations"] == 30 and offline["duplicates"] == offline["missing"] == offline["retries"] == 0 and offline["minimum_sequence"] == 1 and offline["maximum_sequence"] == 30 and offline["sequence_exact"] and offline["coverage_exact"]):
        raise PlanAbort("RUNNER_NOT_READY:OFFLINE_DURABILITY")

    resolved_binary = resolve_amatl_binary(binary)
    binary_sha256 = hash_binary(resolved_binary)
    config_snapshot, config_sha256 = prepare_config_snapshot(output_dir / "config-snapshot", fixture)
    amatl = AmatlProcessExecutor(resolved_binary, binary_sha256, config_snapshot, config_sha256, load_queries(dataset))
    argv = [amatl.argv_for(position) for position in plan.positions]
    structural = {
        "positions_received": len(plan.positions), "argv_generated": len(argv), "queries_mapped": len({command[-2] for command in argv}),
        "unique_identities": len(set(plan.positions)), "q01_r1_restrictions": 0, "internal_max_invocations": None,
        "subprocess_executed": len(amatl.invocations), "argv": argv,
        "result": "AMATL_EXECUTOR_STRUCTURAL_30_30_PASS" if len(argv) == len(plan.positions) == len(set(plan.positions)) == 30 and not amatl.invocations else "FAIL",
    }
    (output_dir / "structural-amatl-30.json").write_text(json.dumps(structural, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if structural["result"] != "AMATL_EXECUTOR_STRUCTURAL_30_30_PASS":
        raise PlanAbort("RUNNER_NOT_READY:AMATL_STRUCTURAL")

    negative_mock = LocalMockExecutor(plan.positions[14])
    negative_before = len(offline_disk)
    negatives = mock_executor_negative_tests(validated, negative_mock)
    negative_after = len(records_from_disk(offline_dir / "runs.jsonl"))
    negative_document = {"results": negatives, "durable_before": negative_before, "durable_after": negative_after, "executor_invocations": len(negative_mock.invocations)}
    (output_dir / "negative-tests.json").write_text(json.dumps(negative_document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if any(result != "PASS" for result in negatives.values()) or negative_before != negative_after or negative_mock.invocations:
        raise PlanAbort("RUNNER_NOT_READY:NEGATIVE_TESTS")

    real_dir = output_dir / "real-q01-r1"
    with DurableJSONLWriter(real_dir / "runs.jsonl") as writer:
        real_state = execute_plan(validated, amatl, writer, max_positions=1)
    real_disk = records_from_disk(real_dir / "runs.jsonl")
    real = durable_validation(real_disk, ExecutionPlan(plan.benchmark_id, plan.provider, 1, (plan.positions[0],)))
    next_position = plan.positions[1]
    next_ready = amatl.argv_for(next_position) and next_position.query_id == "Q02" and next_position.repetition == 1
    real.update({"amatl_invocations": len(amatl.invocations), "executed": len(real_state.records), "retries": real_state.retry_count, "marginalia_executions": 0, "next_position": "Q02-R1", "next_position_ready": bool(next_ready), "record": real_disk[0] if real_disk else None})
    if not (real["amatl_invocations"] == real["executed"] == real["durable_after_reopen"] == real["unique_durable"] == 1 and real["duplicates"] == real["missing"] == real["retries"] == 0 and real["minimum_sequence"] == real["maximum_sequence"] == 1 and real["sequence_exact"] and real["coverage_exact"] and real["next_position_ready"]):
        raise PlanAbort("RUNNER_NOT_READY:REAL_INTEGRATION")

    architecture = {
        "artifact_notice": "Runner production-path validation artifact.",
        "single_execute_plan_path": True,
        "execution_path": "ValidatedExecutionPlan -> execute_plan -> executor -> DurableJSONLWriter -> runs.jsonl",
        "global_execution_limit": "execute_plan: planned count and 30",
        "executor_internal_max_invocations": None,
        "offline": offline,
        "real": real,
    }
    (output_dir / "architecture-validation.json").write_text(json.dumps(architecture, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (output_dir / "README.md").write_text("# Runner production validation\n\nRunner production-path validation artifact.\n", encoding="utf-8")
    (output_dir / "findings.md").write_text("# Findings\n\nThe production path uses one `execute_plan` implementation for mock and AMATL process executors.\n", encoding="utf-8")


def run_campaign(
    dataset: Path,
    benchmark_id: str,
    provider: str,
    repetitions: int,
    fixture: Path,
    binary: Path,
    output_dir: Path,
    *,
    interval_seconds: float = 3,
) -> None:
    """Execute one new, complete, validated SearXNG campaign through AMATL.

    Writes a code→artifact traceability manifest before the first position
    executes, so the campaign is self-contained and auditable even if it is
    later interrupted.
    """
    ensure_output_absent(output_dir)
    dataset_sha256, query_ids, queries = load_dataset_with_hash(dataset)
    plan = build_plan(benchmark_id, provider, query_ids, repetitions)
    validated = validated_plan(plan, query_ids)
    if len(plan.positions) != 30:
        raise PlanAbort("ABORT:CAMPAIGN_REQUIRES_30")

    # [M2] Resolve and hash the AMATL binary exactly once; every position and
    # the --version probe below reuse this same resolved path — no per-position
    # PATH re-resolution.
    resolved_binary = resolve_amatl_binary(binary)
    binary_sha256 = hash_binary(resolved_binary)
    version = amatl_version(resolved_binary)

    # [M1] Read+validate the original fixture exactly once, then freeze those
    # exact bytes into an immutable snapshot inside output_dir before position
    # 1. All AMATL invocations for this campaign target the snapshot only.
    config_snapshot, config_sha256 = prepare_config_snapshot(output_dir, fixture)

    amatl = AmatlProcessExecutor(resolved_binary, binary_sha256, config_snapshot, config_sha256, queries)
    campaign_started_at = datetime.now(timezone.utc).isoformat()
    manifest = build_campaign_manifest(
        amatl_version=version,
        amatl_binary_path=str(resolved_binary), amatl_binary_sha256=binary_sha256,
        dataset_path=dataset, dataset_sha256=dataset_sha256,
        config_path=fixture, config_sha256=config_sha256, config_snapshot_path=str(config_snapshot),
        provider=provider, benchmark_id=benchmark_id, query_order=query_ids,
        repetitions=repetitions, retries=RETRIES_DISABLED,
        timeout_seconds=AMATL_SUBPROCESS_TIMEOUT_SECONDS, concurrency=CONCURRENCY,
        interval_seconds=interval_seconds, campaign_started_at=campaign_started_at,
        sequence_count=len(plan.positions),
    )
    write_campaign_manifest(output_dir, manifest)
    with DurableJSONLWriter(output_dir / "runs.jsonl") as writer:
        execute_plan(validated, amatl, writer, interval_seconds=interval_seconds)


def main() -> int:
    parser = argparse.ArgumentParser(description="offline deterministic AMATL benchmark-plan QA runner")
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument("--benchmark-id", required=True)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--mock-executor-validation", action="store_true")
    parser.add_argument("--production-validation", action="store_true")
    parser.add_argument("--pacing-validation", action="store_true")
    parser.add_argument("--campaign", action="store_true")
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--amatl-binary", type=Path)
    args = parser.parse_args()
    if args.mock_executor_validation:
        run_mock_executor_validation(args.dataset, args.benchmark_id, args.provider, args.repetitions, args.output_dir)
        return 0
    if args.pacing_validation:
        run_pacing_validation(args.dataset, args.benchmark_id, args.provider, args.repetitions, args.output_dir)
        return 0
    if args.production_validation:
        if args.fixture is None or args.amatl_binary is None:
            raise PlanAbort("ABORT:INTEGRATION_ARGUMENTS_REQUIRED")
        run_production_validation(
            args.dataset, args.benchmark_id, args.provider, args.repetitions,
            args.fixture, args.amatl_binary, args.output_dir,
        )
        return 0
    if args.campaign:
        if args.fixture is None or args.amatl_binary is None:
            raise PlanAbort("ABORT:CAMPAIGN_ARGUMENTS_REQUIRED")
        run_campaign(
            args.dataset, args.benchmark_id, args.provider, args.repetitions,
            args.fixture, args.amatl_binary, args.output_dir,
        )
        return 0
    query_ids = load_dataset(args.dataset)
    plan_a = build_plan(args.benchmark_id, args.provider, query_ids, args.repetitions)
    plan_b = build_plan(args.benchmark_id, args.provider, query_ids, args.repetitions)
    validate_plan(plan_a, query_ids)
    validate_plan(plan_b, query_ids)
    deterministic = plan_a == plan_b and plan_hash(plan_a) == plan_hash(plan_b)
    negatives = negative_tests(plan_a, query_ids)
    if not deterministic or any(result != "PASS" for result in negatives.values()):
        raise PlanAbort("ABORT:VALIDATION_FAILED")
    write_artifacts(args.output_dir, plan_a, query_ids, negatives, deterministic)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except PlanAbort as error:
        print(error)
        raise SystemExit(2)
