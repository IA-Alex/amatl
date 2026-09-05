#!/usr/bin/env python3
"""Generate one deterministic, network-free candidate universe for ADR-012."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from pre_execution_novelty_diversity_gate import DEFAULT_THRESHOLDS
from query_similarity import jaccard, tokens

GENERATOR_VERSION = "offline-catalog-v1.1.0"
TARGET_PAIRS = 2300

CONCEPTS = (
    "acoustic ecology", "adaptive optics", "algorithmic auditing", "aquifer recharge",
    "arctic navigation", "battery recycling", "biochar production", "biometric privacy",
    "blue carbon mapping", "botanical phenology", "bridge vibration", "coastal mapping",
    "coral restoration", "crop disease forecasting", "cryogenic cooling", "data provenance",
    "distributed ledgers", "drone photogrammetry", "ecosystem services", "electric aviation",
    "enzymatic catalysis", "fermentation control", "fire weather modeling", "forest acoustics",
    "geothermal reservoirs", "glacier mass balance", "graphene membranes", "habitat corridors",
    "hydrogen storage", "image compression", "indoor air chemistry", "lake stratification",
    "lithium extraction", "low power computing", "marine heatwaves", "microgrid resilience",
    "nanopore sequencing", "ocean alkalinity", "permafrost monitoring", "plant root hydraulics",
    "quantum sensing", "river sediment transport", "robotic inspection", "soil carbon accounting",
    "space weather forecasting", "thermal energy storage", "urban heat mapping", "wildlife telemetry",
)

CONTEXTS = (
    "agricultural engineering", "anthropology", "archaeology", "astronomy", "behavioral science",
    "civil infrastructure", "clinical research", "coastal planning", "computer graphics", "conservation",
    "cultural heritage", "cybersecurity", "disaster management", "ecological economics", "education",
    "electrical systems", "emergency medicine", "energy policy", "environmental chemistry", "epidemiology",
    "food systems", "forest management", "geospatial analysis", "health informatics", "hydrology",
    "industrial design", "information science", "materials engineering", "meteorology", "microbiology",
    "music technology", "oceanography", "operations research", "optical engineering", "public health",
    "rail transportation", "remote sensing", "risk analysis", "robotics", "science education",
    "software reliability", "structural engineering", "supply networks", "sustainable finance", "urban planning",
    "veterinary science", "water management", "workplace safety",
)

TREATMENT_FORMS = (
    "which mechanisms govern {topic}",
    "how can researchers measure {topic}",
    "what evidence explains {topic}",
    "under what conditions does {topic} change",
    "what are the main constraints on {topic}",
    "how do models represent {topic}",
    "what tradeoffs shape {topic}",
    "how is {topic} evaluated in practice",
)

CONTROL_FORMS = (
    "a practical briefing on {topic}",
    "a research overview of {topic}",
    "background material about {topic}",
    "an introductory account of {topic}",
    "a plain language guide to {topic}",
    "key concepts associated with {topic}",
    "a general explanation of {topic}",
    "essential context for {topic}",
)

MAX_ALTERNATIVES_PER_PAIR = len(TREATMENT_FORMS)


def _cross_pair_conflict(candidate_tokens, accepted_pairs, threshold, comparison_counter=None):
    """Return whether either query conflicts with any previously accepted pair."""
    for candidate_token_set in candidate_tokens:
        for accepted_pair in accepted_pairs:
            for accepted_token_set in accepted_pair:
                if comparison_counter is not None:
                    comparison_counter["comparisons"] += 1
                if jaccard(candidate_token_set, accepted_token_set) > threshold:
                    return True
    return False


def _select_strategy_index(topic, base_index, accepted_pairs, comparison_counter=None):
    threshold = DEFAULT_THRESHOLDS["max_cross_pair_similarity"]
    for alternative in range(MAX_ALTERNATIVES_PER_PAIR):
        strategy_index = (base_index + alternative) % len(TREATMENT_FORMS)
        candidate_queries = (
            TREATMENT_FORMS[strategy_index].format(topic=topic),
            CONTROL_FORMS[strategy_index].format(topic=topic),
        )
        if not _cross_pair_conflict(tuple(tokens(query) for query in candidate_queries), accepted_pairs, threshold, comparison_counter):
            return strategy_index, candidate_queries
    raise RuntimeError("INSUFFICIENT_DISTINCT_QUERY_SPACE")


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def build_candidate() -> dict:
    rows = []
    assignments = []
    accepted_pairs = []
    separation_stats = {"comparisons": 0, "generation_failures": 0}
    pair_number = 0
    for concept_index, concept in enumerate(CONCEPTS):
        for context_index, context in enumerate(CONTEXTS):
            if pair_number >= TARGET_PAIRS:
                break
            pair_number += 1
            topic = f"{concept} within {context}"
            pair_id = f"candidate-p-{pair_number:04d}"
            base_index = (concept_index * len(CONTEXTS) + context_index) % len(TREATMENT_FORMS)
            try:
                strategy_index, candidate_queries = _select_strategy_index(
                    topic, base_index, accepted_pairs, separation_stats
                )
            except RuntimeError:
                separation_stats["generation_failures"] += 1
                raise
            accepted_pairs.append(tuple(tokens(query) for query in candidate_queries))
            for arm, forms in (("treatment", TREATMENT_FORMS), ("control", CONTROL_FORMS)):
                query_id = f"candidate-{arm[0]}-{pair_number:04d}"
                query = candidate_queries[0 if arm == "treatment" else 1]
                row = {
                    "query_id": query_id,
                    "query_text": query,
                    "query": query,
                    "normalized_query": " ".join(query.casefold().split()),
                    "arm": arm,
                    "pair_id": pair_id,
                    "query_family": f"{concept} / {context}",
                    "generation_strategy": "NEW_CANDIDATE_MODE:STRATIFIED_CONCEPT_CONTEXT_CATALOG",
                    "generation_provenance": {
                        "generator_version": GENERATOR_VERSION,
                        "concept_index": concept_index,
                        "context_index": context_index,
                        "template_index": strategy_index,
                        "catalog_source": "offline_static_domain_catalog",
                        "network_requests": 0,
                    },
                }
                rows.append(row)
                assignments.append({"query_id": query_id, "pair_id": pair_id, "arm": arm, "query": query})
        if pair_number >= TARGET_PAIRS:
            break
    return {
        "schema": "amatl.relevance.offline-candidate-universe.v1",
        "candidate_mode": "NEW_CANDIDATE_MODE",
        "generator_version": GENERATOR_VERSION,
        "pair_count": TARGET_PAIRS,
        "treatment_query_count": TARGET_PAIRS,
        "control_query_count": TARGET_PAIRS,
        "network_requests": 0,
        "separation_contract": {
            "algorithm": "EXHAUSTIVE_DETERMINISTIC_JACCARD_AGAINST_ACCEPTED_PAIRS",
            "threshold": DEFAULT_THRESHOLDS["max_cross_pair_similarity"],
            "max_alternatives_per_pair": MAX_ALTERNATIVES_PER_PAIR,
            "actual_comparisons": separation_stats["comparisons"],
            "generation_failures": separation_stats["generation_failures"],
            "worst_case_complexity": "O(P^2 * A^2)"
        },
        "queries": rows,
        "assignments": assignments,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(canonical(build_candidate()))


if __name__ == "__main__":
    main()
