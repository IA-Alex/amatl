#!/usr/bin/env python3
"""Create the immutable, pre-label supplemental query universe and manifest."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"

PROCEDURAL = [
    "how to replace a smoke detector battery", "how to unclog a kitchen sink", "how to reset a Wi-Fi router",
    "how to patch a small hole in drywall", "how to sharpen kitchen knives", "how to calculate a tip",
    "how to create a household budget", "how to change a bicycle tire", "how to plant basil seeds",
    "how to clean a cast iron pan", "how to make a doctor appointment", "how to file a change of address",
    "how to prepare for a job interview", "how to use a multimeter safely", "how to wash a down jacket",
    "how to replace a faucet washer", "how to store fresh herbs", "how to set up two factor authentication",
    "how to remove coffee stains", "how to compost kitchen scraps", "how to test a car battery",
    "how to sew on a button", "how to create a shared calendar", "how to pack for a camping trip",
    "how to clean a computer keyboard", "how to start a vegetable garden", "how to read an electricity bill",
    "how to make cold brew coffee", "how to measure a room for paint", "how to update emergency contacts",
]
INFORMATIONAL = [
    "what is a credit score", "what causes seasonal allergies", "how does solar power work",
    "what is cloud storage", "what causes food poisoning", "how does a thermostat work",
    "what is a deductible", "what causes a migraine", "how does a water filter work",
    "what is a password manager", "what causes rust", "how does a GPS work",
    "what is renters insurance", "what causes jet lag", "how does a compost bin work",
    "what is a heat index", "what causes static electricity", "how does a credit card work",
    "what is a phishing email", "what causes a sore throat", "how does a library card work",
    "what is a food allergy", "what causes a power outage", "how does an electric car work",
    "what is a firewall", "what causes mold growth", "how does a savings account work",
    "what is a VPN", "what causes a flat tire", "how does recycling work",
]
S3_PROCEDURAL = [
    "how to check a fire extinguisher", "how to clean a shower head", "how to make a backup of a laptop",
    "how to replace a light switch cover", "how to organize tax documents", "how to prepare rice in a rice cooker",
    "how to stop a leaking toilet", "how to choose a bike helmet", "how to clean window screens",
    "how to label moving boxes", "how to calibrate a kitchen scale", "how to fix a loose cabinet handle",
    "how to check tire pressure", "how to dry a wet phone", "how to make an ice pack",
    "how to recycle cardboard", "how to clean a reusable water bottle", "how to set a digital alarm clock",
    "how to replace a vacuum bag", "how to prepare a first aid kit",
]
S3_INFORMATIONAL = [
    "what is a surge protector", "what causes condensation", "how does a septic system work",
    "what is a carbon monoxide detector", "what causes a circuit breaker to trip", "how does a dishwasher work",
    "what is a meal plan", "what causes dry skin", "how does a dehumidifier work",
    "what is a postal code", "what causes a battery to drain", "how does a water meter work",
    "what is a filing cabinet", "what causes an eclipse", "how does a thermostat sensor work",
    "what is a utility bill", "what causes a sink to smell", "how does a smoke alarm work",
    "what is a rain barrel", "what causes wind",
]

def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def render(value): return json.dumps(value, indent=2) + "\n"

def main():
    queries, order = [], 1
    def append(query_id, text, stratum, positions, semantic_form):
        nonlocal order
        queries.append({"order": order, "query_id": query_id, "query": text, "assigned_stratum": stratum,
                        "assignment_rationale": "Fixed before capture from the pilot design's query-form and rank-band rule; never inferred from results or labels.",
                        "semantic_form": semantic_form, "permitted_positions": positions,
                        "provider_requirement": "SEARXNG_ONLY"})
        order += 1
    for index, text in enumerate(PROCEDURAL, 1): append(f"sup-s1-{index:03d}", text, "S1_PROCEDURAL_SHALLOW", [1,2,3], "procedural")
    for index, text in enumerate(INFORMATIONAL, 1): append(f"sup-s2-{index:03d}", text, "S2_INFORMATIONAL_SHALLOW", [1,2,3], "informational")
    for index, (procedural, informational) in enumerate(zip(S3_PROCEDURAL, S3_INFORMATIONAL), 1):
        append(f"sup-s3-{2 * index - 1:03d}", procedural, "S3_MIXED_DEPTH_CONTROL", [4,5,6,7,8], "procedural")
        append(f"sup-s3-{2 * index:03d}", informational, "S3_MIXED_DEPTH_CONTROL", [4,5,6,7,8], "informational")
    universe = {"schema": "amatl.relevance.supplemental-query-universe.v1", "universe_id": "independent-relevance-supplemental-v1",
                "version": 1, "freeze_status": "FROZEN", "created_from": ["supplemental-pilot-design.json", "pre-capture authored topic list in tools/freeze_supplemental_query_universe.py"],
                "provider_requirement": "SEARXNG_ONLY", "dynamic_expansion": "DISABLED", "deterministic_order": "ascending order; S3 alternates procedural then informational query IDs",
                "stratum_definitions_reference": "supplemental-pilot-design.json", "normalization_rules": {"query": "Unicode NFC, trim, collapse ASCII whitespace, case preserved", "url": "lowercase scheme/host; remove fragment; preserve query; normalize empty path to slash"},
                "exclusion_rules": ["DUPLICATE_CURRENT_PILOT", "OVERLAP_V1", "OVERLAP_HISTORICAL", "INVALID_RESULT", "OUTSIDE_ALLOWED_DEPTH", "OUTSIDE_STRATUM_CONTRACT", "OUTSIDE_PROVIDER_CONTRACT"], "queries": queries}
    universe_path = OUT / "supplemental-query-universe-v1.json"
    payload = render(universe)
    if universe_path.exists() and universe_path.read_text(encoding="utf-8") != payload: raise ValueError("refusing to alter frozen universe")
    universe_path.write_text(payload, encoding="utf-8")
    counts = {"S1_PROCEDURAL_SHALLOW": 30, "S2_INFORMATIONAL_SHALLOW": 30, "S3_MIXED_DEPTH_CONTROL": 40}
    manifest = {"schema": "amatl.relevance.supplemental-query-universe-manifest.v1", "universe_id": universe["universe_id"], "version": 1,
                "freeze_status": "FROZEN", "query_count": len(queries), "query_counts_by_stratum": counts,
                "structural_capacity_by_stratum": {"S1_PROCEDURAL_SHALLOW": 90, "S2_INFORMATIONAL_SHALLOW": 90, "S3_MIXED_DEPTH_CONTROL": 200, "S3_MIXED_DEPTH_CONTROL_PROCEDURAL_HALF": 100, "S3_MIXED_DEPTH_CONTROL_INFORMATIONAL_HALF": 100},
                "universe_sha256": digest(universe_path), "contract_input_sha256": {"supplemental_pilot_design": digest(OUT / "supplemental-pilot-design.json"), "v1_ground_truth_manifest": digest(OUT / "v1-ground-truth-manifest.json"), "v1_ground_truth": digest(OUT / "v1-ground-truth.json")},
                "freeze_rule": "Any byte change to the universe or a contractual input requires a new version and fresh authorization; runtime expansion is prohibited."}
    manifest_path = OUT / "supplemental-query-universe-v1-manifest.json"
    manifest_payload = render(manifest)
    if manifest_path.exists() and manifest_path.read_text(encoding="utf-8") != manifest_payload: raise ValueError("refusing to alter frozen universe manifest")
    manifest_path.write_text(manifest_payload, encoding="utf-8")
    procedure = {"schema": "amatl.relevance.supplemental-pilot-acquisition-procedure.v1", "status": "FROZEN_READY_FOR_AUTHORIZED_CAPTURE",
                 "provider_contract": "SEARXNG_ONLY", "provider_fallback": "DISABLED", "marginalia": "UNREACHABLE_FROM_THIS_PATH",
                 "universe_path": str(universe_path.relative_to(ROOT)), "universe_sha256": manifest["universe_sha256"],
                 "execution_order": ["read frozen universe", "execute only listed query IDs against SearXNG", "retain raw response evidence", "supply result to run_supplemental_pilot.py", "canonicalize", "reject duplicate/overlap/invalid rows with reason code", "count accepted row only in assigned stratum", "stop at 60 per stratum and 180 total"],
                 "forbidden": ["query expansion", "provider switching", "rank expansion", "quota borrowing", "discarding rejected attempt evidence"],
                 "terminal_statuses": ["COMPLETE", "EXHAUSTED_FROZEN_UNIVERSE"], "network_execution": "NOT_PERFORMED_BY_THIS_WORK_PACKAGE"}
    procedure_path = OUT / "supplemental-pilot-acquisition-procedure.json"
    procedure_payload = render(procedure)
    if procedure_path.exists() and procedure_path.read_text(encoding="utf-8") != procedure_payload: raise ValueError("refusing to alter frozen acquisition procedure")
    procedure_path.write_text(procedure_payload, encoding="utf-8")
    print(json.dumps(manifest, indent=2))

if __name__ == "__main__": main()
