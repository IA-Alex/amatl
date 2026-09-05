#!/usr/bin/env python3
"""Generate one deterministic, network-free candidate universe for ADR-012."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

GENERATOR_VERSION = "offline-catalog-v1.0.0"
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


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def build_candidate() -> dict:
    rows = []
    assignments = []
    pair_number = 0
    for concept_index, concept in enumerate(CONCEPTS):
        for context_index, context in enumerate(CONTEXTS):
            if pair_number >= TARGET_PAIRS:
                break
            pair_number += 1
            topic = f"{concept} within {context}"
            pair_id = f"candidate-p-{pair_number:04d}"
            strategy_index = (concept_index * len(CONTEXTS) + context_index) % len(TREATMENT_FORMS)
            for arm, forms in (("treatment", TREATMENT_FORMS), ("control", CONTROL_FORMS)):
                query_id = f"candidate-{arm[0]}-{pair_number:04d}"
                query = forms[strategy_index].format(topic=topic)
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
