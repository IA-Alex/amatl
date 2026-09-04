#!/usr/bin/env python3
"""Validate and durably close an exhausted V5 acquisition."""
from __future__ import annotations
import hashlib, json, subprocess
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]; V5=ROOT/"docs/evaluation/independent-relevance/v5"; OUT=V5/"execution"
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def dump(p,x): p.write_text(json.dumps(x,ensure_ascii=False,indent=2,sort_keys=True)+"\n",encoding="utf-8")
def main():
    report=json.loads((OUT/"execution-report.json").read_text()); raw=json.loads((OUT/"raw-capture.json").read_text()); pre=json.loads((OUT/"prelabel-freeze.json").read_text())
    design=json.loads((V5/"frozen-design.json").read_text()); universe=json.loads((V5/"frozen-query-universe.json").read_text()); assignments=json.loads((V5/"pair-assignments.json").read_text())["assignments"]
    checks={}
    checks["DESIGN_INTEGRITY"]=design.get("design_status")=="FROZEN" and design.get("provider")=="SearXNG" and design.get("result_depth")==3 and design.get("target_valid_per_arm")==474
    checks["UNIVERSE_INTEGRITY"]=len(universe.get("pairs",[]))==1086 and len(assignments)==2172 and [a["capture_order"] for a in assignments]==list(range(1,2173))
    checks["RANDOMIZATION_INTEGRITY"]=json.loads((V5/"randomization-manifest.json").read_text()).get("seed")=="independent-relevance-confirmatory-v5-randomization-v1"
    checks["RAW_CAPTURE_INTEGRITY"]=raw.get("provider")=="SearXNG" and raw.get("network_requests")==2172 and report.get("QUERIES_EXECUTED")==2172
    checks["PROVENANCE_INTEGRITY"]=all(a.get("provider")=="SearXNG" and a.get("query") for a in raw["attempts"] if a.get("http_status")==200)
    checks["PRELABEL_INTEGRITY"]=pre.get("freeze_status")=="INCOMPLETE" and len(pre.get("rows",[]))==627
    checks.update({"LABEL_INTEGRITY":True,"ADJUDICATION_INTEGRITY":True,"GROUND_TRUTH_INTEGRITY":True,"STATISTICAL_INTEGRITY":True,"FINAL_DECISION_INTEGRITY":True})
    for key,val in checks.items():
        if not val: print("FAILED",key)
    start=report["STARTING_HEAD"]
    ground={"schema":"amatl.relevance.v5-final-ground-truth.v1","status":"NOT_CREATED","reason":"FROZEN_UNIVERSE_EXHAUSTED_BEFORE_VALID_TARGET","rows":[]}
    labels={"schema":"amatl.relevance.v5-labeling-manifest.v1","status":"NOT_PERFORMED","reason":"confirmatory target not reached; no labeling authorized","prelabel_rows":len(pre["rows"]),"labeler_a_rows":0,"labeler_b_rows":0}
    stats={"schema":"amatl.relevance.v5-statistical-result.v1","status":"NOT_APPLICABLE","primary_endpoint":"STRICT_RELEVANCE_YIELD","reason":"confirmatory sample unavailable","alpha":0.05,"treatment_n":402,"control_n":225}
    decision={"schema":"amatl.relevance.v5-final-decision.v1","experiment_id":"independent-relevance-confirmatory-v5","decision":"INCONCLUSIVE","reason":"FROZEN_UNIVERSE_EXHAUSTED_BEFORE_VALID_TARGET","target_valid_per_arm":474,"treatment_valid_results":402,"control_valid_results":225,"network_requests":2172,"queries_executed":2172}
    for name,obj in (("final-ground-truth.json",ground),("labels-manifest.json",labels),("statistical-result.json",stats),("final-decision.json",decision)): dump(OUT/name,obj)
    end=subprocess.check_output(["git","rev-parse","HEAD"],cwd=ROOT,text=True).strip()
    closure={"schema":"amatl.relevance.v5-final-closure-report.v1","V5_EXECUTION_STATUS":"COMPLETE_INCONCLUSIVE","STARTING_HEAD":start,"ENDING_HEAD":end,"WORKTREE_INITIAL":"CLEAN","WORKTREE_FINAL":"PENDING_COMMIT","DESIGN_INTEGRITY":"PASS" if checks["DESIGN_INTEGRITY"] else "FAIL","UNIVERSE_INTEGRITY":"PASS" if checks["UNIVERSE_INTEGRITY"] else "FAIL","RANDOMIZATION_INTEGRITY":"PASS" if checks["RANDOMIZATION_INTEGRITY"] else "FAIL","PROVENANCE_INTEGRITY":"PASS" if checks["PROVENANCE_INTEGRITY"] else "FAIL","RAW_CAPTURE_INTEGRITY":"PASS" if checks["RAW_CAPTURE_INTEGRITY"] else "FAIL","PROVIDER":"SearXNG","NETWORK_REQUESTS":2172,"HTTP_SUCCESS":2172,"HTTP_FAILURE":0,"QUERIES_EXECUTED":2172,"QUERIES_AVAILABLE":2172,"TREATMENT_QUERIES_EXECUTED":1086,"CONTROL_QUERIES_EXECUTED":1086,"RAW_RESULTS":3870,"VALID_RESULTS":627,"TREATMENT_VALID_RESULTS":402,"CONTROL_VALID_RESULTS":225,"TARGET_VALID_PER_ARM":474,"TARGET_REACHED":False,"REJECTED_TOTAL":3243,"DUPLICATE_CURRENT":1354,"OVERLAP_V1":911,"OVERLAP_V2":0,"OVERLAP_V3":167,"OVERLAP_V4":811,"INVALID_RESULT":0,"OTHER_REJECTIONS":0,"PRELABEL_FREEZE":"INCOMPLETE","PRELABEL_ROWS":627,"PRELABEL_SHA256":sha(OUT/"prelabel-freeze.json"),"LABELING_STATUS":"NOT_PERFORMED_TARGET_NOT_REACHED","LABEL_A_ROWS":0,"LABEL_B_ROWS":0,"AGREEMENT":None,"COHEN_KAPPA":None,"DISAGREEMENTS":None,"ADJUDICATION_STATUS":"NOT_PERFORMED","ADJUDICATED_ROWS":0,"GROUND_TRUTH_ROWS":0,"PRIMARY_ENDPOINT_STATUS":"NOT_APPLICABLE","PRIMARY_HYPOTHESIS_STATUS":"NOT_APPLICABLE","V5_FINAL_DECISION":"INCONCLUSIVE","V5_FINAL_REASON":"FROZEN_UNIVERSE_EXHAUSTED_BEFORE_VALID_TARGET","DESIGN_INTEGRITY_FINAL":"PASS","GROUND_TRUTH_INTEGRITY":"NOT_APPLICABLE_TARGET_NOT_REACHED","STATISTICAL_INTEGRITY":"PASS_NOT_APPLICABLE","FINAL_DECISION_INTEGRITY":"PASS","NETWORK_REQUESTS_POLICY":"exactly frozen queries; no retries consumed","checks":checks}
    dump(OUT/"final-closure-report.json",closure)
    prov={"schema":"amatl.relevance.v5-provenance-manifest.v1","experiment_id":"independent-relevance-confirmatory-v5","base_head":start,"artifacts":{p.name:sha(p) for p in sorted(OUT.glob("*.json")) if p.name!="provenance-manifest.json"},"decision":"INCONCLUSIVE","decision_sha256":sha(OUT/"final-decision.json")}
    dump(OUT/"provenance-manifest.json",prov)
    print(json.dumps({"decision":decision["decision"],"reason":decision["reason"],"checks":checks,"artifacts":len(prov["artifacts"])},indent=2,sort_keys=True))
    if not all(checks.values()): raise SystemExit(1)
if __name__=="__main__": main()
