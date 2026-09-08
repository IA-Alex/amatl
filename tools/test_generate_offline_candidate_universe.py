import pytest

from generate_offline_candidate_universe import (
    CONTROL_FORMS,
    MAX_ALTERNATIVES_PER_PAIR,
    TREATMENT_FORMS,
    _cross_pair_conflict,
    _select_strategy_index,
)
from pre_execution_novelty_diversity_gate import DEFAULT_THRESHOLDS
from query_similarity import tokens


def _accepted(*queries):
    return (tuple(tokens(query) for query in queries),)


def test_non_conflicting_pair_is_accepted():
    index, queries = _select_strategy_index("coastal mapping within urban planning", 0, [])
    assert index == 0
    assert queries[0].startswith("which mechanisms")


def test_cross_pair_conflict_is_rejected_and_alternative_selected():
    topic = "coastal mapping within urban planning"
    conflicting = [TREATMENT_FORMS[0].format(topic=topic), CONTROL_FORMS[0].format(topic=topic)]
    index, _ = _select_strategy_index(topic, 0, _accepted(*conflicting))
    assert index == 1


def test_selection_is_deterministic_with_multiple_alternatives():
    topic = "coastal mapping within urban planning"
    prior = _accepted(
        TREATMENT_FORMS[0].format(topic=topic), CONTROL_FORMS[0].format(topic=topic)
    )
    assert _select_strategy_index(topic, 0, prior) == _select_strategy_index(topic, 0, prior)


def test_exhaustion_is_explicit():
    topic = "coastal mapping within urban planning"
    all_alternatives = tuple(
        _accepted(TREATMENT_FORMS[i].format(topic=topic), CONTROL_FORMS[i].format(topic=topic))[0]
        for i in range(MAX_ALTERNATIVES_PER_PAIR)
    )
    with pytest.raises(RuntimeError, match="INSUFFICIENT_DISTINCT_QUERY_SPACE"):
        _select_strategy_index(topic, 0, all_alternatives)


def test_pair_matching_and_boundary_semantics_are_preserved():
    topic = "coastal mapping within urban planning"
    index, queries = _select_strategy_index(topic, 0, [])
    assert queries == (
        TREATMENT_FORMS[index].format(topic=topic),
        CONTROL_FORMS[index].format(topic=topic),
    )
    assert DEFAULT_THRESHOLDS["max_cross_pair_similarity"] == 0.80


def test_threshold_boundary_is_accepted_but_value_above_is_rejected():
    candidate = (frozenset({"a", "b", "c", "d"}),)
    at_threshold = (frozenset({"a", "b", "c", "d", "e"}),)
    above_candidate = (frozenset({"a", "b", "c", "d", "e"}),)
    above_threshold = (frozenset({"a", "b", "c", "d", "e", "f"}),)
    assert not _cross_pair_conflict(candidate, (at_threshold,), 0.80)
    assert _cross_pair_conflict(above_candidate, (above_threshold,), 0.80)
