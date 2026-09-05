#!/usr/bin/env python3
"""Deterministic, offline ADR-012 gate and future freeze boundary."""
from __future__ import annotations
import argparse, hashlib, json, re, unicodedata, math
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

GATE_NAME = "PRE_EXECUTION_NOVELTY_DIVERSITY_GATE"
GATE_VERSION = "1.1.0"
ARMS = ("treatment", "control")
LEGACY_REPLAY_MODE = "LEGACY_REPLAY_MODE"
NEW_CANDIDATE_MODE = "NEW_CANDIDATE_MODE"
DEFAULT_THRESHOLDS = {"max_normalized_query_overlap_rate": .20, "max_query_duplicate_rate": .20, "max_query_near_duplicate_rate": .20, "min_effective_query_diversity": .50, "max_arm_novelty_delta": .10, "max_arm_diversity_delta": .10, "max_pair_family_concentration": .20, "max_cross_pair_similarity": .80, "min_capacity_margin": 0.0, "min_arm_capacity_ratio": .90, "max_historical_result_overlap_risk": .50, "max_domain_concentration_risk": .50, "max_query_family_saturation_risk": .50}
THRESHOLD_CLASSIFICATION = {k: "POLICY_DEFAULT" for k in DEFAULT_THRESHOLDS}

def canonical_json(value: Any) -> bytes: return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()
def sha256_bytes(value: bytes) -> str: return hashlib.sha256(value).hexdigest()
def sha256_file(path: Path) -> str: return sha256_bytes(path.read_bytes())
def normalize_query(value: str) -> str: return " ".join(unicodedata.normalize("NFC", value).casefold().strip().split())
def tokens(value: str) -> frozenset[str]: return frozenset(re.findall(r"[\w]+", normalize_query(value), flags=re.UNICODE))
def jaccard(left: frozenset[str], right: frozenset[str]) -> float:
    union = left | right
    return len(left & right) / len(union) if union else 1.0

def _queries(universe: Any) -> list[dict[str, Any]]:
    if isinstance(universe, dict):
        if isinstance(universe.get("queries"), list): return universe["queries"]
        if isinstance(universe.get("assignments"), list): return universe["assignments"]
        if isinstance(universe.get("pairs"), list):
            return [{"query_id": f"{p.get('pair_id')}:{a}", "pair_id": p.get("pair_id"), "arm": a, "query": p[a], "matched_topic": p.get("matched_topic")} for p in universe["pairs"] for a in ARMS if isinstance(p.get(a), str)]
    if isinstance(universe, list): return universe
    raise ValueError("CANDIDATE_UNIVERSE_QUERY_LIST_MISSING")
def _text(row):
    value = row.get("query", row.get("query_text"))
    if not isinstance(value, str) or not value.strip(): raise ValueError("QUERY_TEXT_MISSING")
    return value
def _arm(row):
    value = str(row.get("arm", row.get("designation", ""))).casefold()
    if value not in ARMS: raise ValueError("QUERY_ARM_INVALID")
    return value
def _family(row, normalized):
    value = row.get("query_family", row.get("matched_topic", row.get("family")))
    return normalize_query(value) if isinstance(value, str) and value.strip() else " ".join(sorted(tokens(normalized)))
def _walk(sources: Iterable[Any], keys: set[str]):
    out=[]
    def visit(v):
        if isinstance(v, dict):
            for k,x in v.items():
                if k in keys and isinstance(x,str): out.append(x)
                visit(x)
        elif isinstance(v,list):
            for x in v: visit(x)
    for source in sources: visit(source)
    return out
def _historical_queries(sources): return set(_walk(sources, {"query","query_text","treatment","control"}))
def _result_urls(sources):
    urls={x.casefold().split("#",1)[0].rstrip("/") for x in _walk(sources,{"canonical_url","original_url","url"}) if x.startswith(("http://","https://"))}
    return urls, Counter(x.casefold() for x in _walk(sources,{"domain"}))
def _stats(values):
    if not values: return {k: None for k in ("MEAN","MEDIAN","P90","P95","MAX")}
    v=sorted(values)
    def percentile(q): return v[min(len(v)-1,max(0,math.ceil(q*len(v))-1))]
    return {"MEAN":sum(v)/len(v),"MEDIAN":percentile(.5),"P90":percentile(.9),"P95":percentile(.95),"MAX":v[-1]}
def _risk(value, threshold, evidence):
    if not evidence or value is None: return "INSUFFICIENT_EVIDENCE"
    if value > threshold: return "SEVERE" if value > min(1,threshold*2) else "HIGH"
    return "MODERATE" if value > threshold*.5 else "LOW"

@dataclass(frozen=True)
class GateDecision:
    decision: str; reasons: tuple[str,...]; metrics: dict[str,Any]; thresholds: dict[str,Any]; provenance: dict[str,Any]; mode: str=LEGACY_REPLAY_MODE; subgate_decisions: dict[str,str]|None=None
    def as_dict(self): return {"gate_name":GATE_NAME,"gate_version":GATE_VERSION,"mode":self.mode,"freeze_allowed":self.decision=="PASS","reasons":list(self.reasons),"metrics":self.metrics,"thresholds":self.thresholds,"threshold_classification":THRESHOLD_CLASSIFICATION,"subgate_decisions":self.subgate_decisions or {},"decision":self.decision,"freeze_boundary_integration":"COMPLETE","FREEZE_BOUNDARY_INTEGRATION":"COMPLETE","provenance":{**self.provenance,"network_requests":0}}

class PreExecutionNoveltyDiversityGate:
    def __init__(self, thresholds=None): self.thresholds={**DEFAULT_THRESHOLDS,**(thresholds or {})}
    @staticmethod
    def _pairs(values):
        out={}
        for x in values:
            if x["pair"]: out.setdefault(str(x["pair"]),{})[x["arm"]]=x["tokens"]
        return out
    @staticmethod
    def _top_share(values):
        c=Counter(x["family"] for x in values); return max(c.values(),default=0)/len(values) if values else 0
    def evaluate(self, candidate_universe, historical_query_universes=(), historical_result_manifests=(), arm_assignments=(), target_valid_per_arm=0, conservative_valid_per_query=0.0, provenance=None, mode=LEGACY_REPLAY_MODE):
        rows=_queries(candidate_universe); reasons=[]; counts={k:0 for k in ("duplicates","missing","unknown","mismatch")}; provenance_missing=0; prepared=[]; seen=set()
        if isinstance(arm_assignments, dict): arm_assignments=arm_assignments.get("assignments", [])
        try:
            for i,row in enumerate(rows):
                text=_text(row); qid=str(row.get("query_id",f"row-{i+1}")); norm=normalize_query(text); prepared.append({"id":qid,"original":text,"normalized":norm,"arm":_arm(row),"family":_family(row,norm),"pair":row.get("pair_id"),"tokens":tokens(text),"row":row})
                if qid in seen: counts["duplicates"]+=1
                seen.add(qid)
                family_present=isinstance(row.get("query_family",row.get("matched_topic",row.get("family"))),str) and row.get("query_family",row.get("matched_topic",row.get("family"))).strip()
                generation_ok=isinstance(row.get("generation_strategy"),str) and bool(row["generation_strategy"].strip()) and row.get("generation_provenance") not in (None,"",{})
                if mode==NEW_CANDIDATE_MODE and (not all(isinstance(row.get(k),str) and row[k].strip() for k in ("query_id","query","arm","pair_id")) or not family_present or not generation_ok): counts["missing"]+=1; provenance_missing+=1
        except ValueError as exc: return GateDecision("FAIL_INTEGRITY",(str(exc),),{},self.thresholds,provenance or {},mode,{"INTEGRITY_GATE":"FAIL"})
        byid={x["id"]:x for x in prepared}; bytext={x["normalized"]:x for x in prepared}; assigned=Counter(); assignment_pairs={}
        for a in list(arm_assignments):
            text=a.get("query",a.get("query_text")); match=byid.get(str(a.get("query_id"))) if a.get("query_id") is not None else (bytext.get(normalize_query(text)) if isinstance(text,str) else None)
            if not match: counts["unknown"]+=1; continue
            assigned[match["id"]]+=1
            if _arm(a)!=match["arm"]: counts["mismatch"]+=1
            if match["pair"]: assignment_pairs.setdefault(str(match["pair"]),set()).add(match["arm"])
        if arm_assignments or mode==NEW_CANDIDATE_MODE: counts["missing"]+=sum(assigned[x["id"]]==0 for x in prepared)
        counts["duplicates"]+=sum(max(0,n-1) for n in assigned.values())
        incomplete=0
        candidate_pair_counts={}
        for x in prepared:
            if x["pair"]: candidate_pair_counts.setdefault(str(x["pair"]),Counter())[x["arm"]]+=1
        if arm_assignments:
            incomplete=sum(arms!=set(ARMS) for arms in assignment_pairs.values())
            incomplete+=sum(set(c)!=set(ARMS) or any(c[a]!=1 for a in ARMS) for c in candidate_pair_counts.values())
        integrity_fail=any(counts.values()) or incomplete or (mode==NEW_CANDIDATE_MODE and not arm_assignments)
        if integrity_fail: reasons.append("FAIL_INTEGRITY")
        historical_raw=_historical_queries(historical_query_universes); historical={normalize_query(x) for x in historical_raw}; htokens=[tokens(x) for x in historical_raw]; cats=Counter()
        for x in prepared:
            if x["original"] in historical_raw: cats["EXACT"]+=1
            elif x["normalized"] in historical: cats["NORMALIZED"]+=1
            elif any(jaccard(x["tokens"],t)>=.80 for t in htokens): cats["NEAR_DUPLICATE"]+=1
            else: cats["NOVEL"]+=1
        n=len(prepared); normvals=[x["normalized"] for x in prepared]; dup=len(normvals)-len(set(normvals)); pairs=self._pairs(prepared); pv=list(pairs.values()); internal=[jaccard(a["tokens"],b["tokens"]) for i,a in enumerate(prepared) for b in prepared[i+1:]]; cross=[jaccard(a,b) for i,p in enumerate(pv) for q in pv[i+1:] for a in p.values() for b in q.values()]
        family=Counter(x["family"] for x in prepared); top=sorted(family.items(),key=lambda z:(-z[1],z[0])); arms={a:[x for x in prepared if x["arm"]==a] for a in ARMS}; urls,domains=_result_urls(historical_result_manifests); hrisk=len(urls)/(len(urls)+n) if urls else None; drisk=max(domains.values())/sum(domains.values()) if domains else None; rt=len(arms["treatment"])*conservative_valid_per_query/target_valid_per_arm if target_valid_per_arm else None; rc=len(arms["control"])*conservative_valid_per_query/target_valid_per_arm if target_valid_per_arm else None
        tnov=sum(1 for x in arms["treatment"] if x["normalized"] not in historical)/len(arms["treatment"]) if arms["treatment"] else 0; cnov=sum(1 for x in arms["control"] if x["normalized"] not in historical)/len(arms["control"]) if arms["control"] else 0
        tdiv=len({x["family"] for x in arms["treatment"]})/len(arms["treatment"]) if arms["treatment"] else 0; cdiv=len({x["family"] for x in arms["control"]})/len(arms["control"]) if arms["control"] else 0
        pair_family=Counter(" ".join(sorted(set().union(*p.values()))) for p in pairs.values() if len(p)==2)
        internal_stats=_stats(internal); cross_stats=_stats(cross); risks={"HISTORICAL_RESULT_OVERLAP_RISK":_risk(hrisk,self.thresholds["max_historical_result_overlap_risk"],bool(urls)),"DOMAIN_CONCENTRATION_RISK":_risk(drisk,self.thresholds["max_domain_concentration_risk"],bool(domains)),"QUERY_FAMILY_SATURATION_RISK":_risk(self._top_share(prepared),self.thresholds["max_query_family_saturation_risk"],bool(prepared))}
        metrics={"TOTAL_CANDIDATE_QUERIES":n,"EXACT_QUERY_OVERLAP_COUNT":cats["EXACT"],"NORMALIZED_QUERY_OVERLAP_COUNT":cats["EXACT"]+cats["NORMALIZED"],"NEAR_DUPLICATE_HISTORICAL_COUNT":cats["NEAR_DUPLICATE"],"NEAR_DUPLICATE_HISTORICAL_OVERLAP_RATE":cats["NEAR_DUPLICATE"]/n if n else 0,"HISTORICAL_OVERLAP_CATEGORIES":dict(cats),"NEW_QUERY_COUNT":cats["NOVEL"],"QUERY_NOVELTY_RATE":cats["NOVEL"]/n if n else 0,"QUERY_DUPLICATE_RATE":dup/n if n else 0,"QUERY_NEAR_DUPLICATE_RATE":sum(jaccard(a["tokens"],b["tokens"])>=.80 for i,a in enumerate(prepared) for b in prepared[i+1:])*2/n if n else 0,"QUERY_FAMILY_COUNT":len(family),"QUERY_FAMILY_DISTRIBUTION":{k:family[k] for k in sorted(family)},"TOP_QUERY_FAMILIES":top[:10],"TOP_QUERY_FAMILY_SHARE":top[0][1]/n if top else 0,"FAMILY_CONCENTRATION":top[0][1]/n if top else 0,"PAIR_COUNT":len(pairs),"UNIQUE_PAIR_FAMILY_COUNT":len(pair_family),"PAIR_FAMILY_CONCENTRATION":max(pair_family.values(),default=0)/len(pairs) if pairs else 0,"INTERNAL_QUERY_SIMILARITY":internal_stats,"CROSS_PAIR_SIMILARITY":max(cross,default=0),"CROSS_PAIR_SIMILARITY_STATS":cross_stats,"INTERNAL_QUERY_SIMILARITY_MEAN":internal_stats["MEAN"],"INTERNAL_QUERY_SIMILARITY_MEDIAN":internal_stats["MEDIAN"],"INTERNAL_QUERY_SIMILARITY_P90":internal_stats["P90"],"INTERNAL_QUERY_SIMILARITY_P95":internal_stats["P95"],"INTERNAL_QUERY_SIMILARITY_MAX":internal_stats["MAX"],"CROSS_PAIR_SIMILARITY_MEAN":cross_stats["MEAN"],"CROSS_PAIR_SIMILARITY_MEDIAN":cross_stats["MEDIAN"],"CROSS_PAIR_SIMILARITY_P90":cross_stats["P90"],"CROSS_PAIR_SIMILARITY_P95":cross_stats["P95"],"CROSS_PAIR_SIMILARITY_MAX":cross_stats["MAX"],"TREATMENT_QUERY_FAMILY_COUNT":len({x["family"] for x in arms["treatment"]}),"CONTROL_QUERY_FAMILY_COUNT":len({x["family"] for x in arms["control"]}),"TREATMENT_TOP_FAMILY_SHARE":self._top_share(arms["treatment"]),"CONTROL_TOP_FAMILY_SHARE":self._top_share(arms["control"]),"TREATMENT_QUERY_COUNT":len(arms["treatment"]),"CONTROL_QUERY_COUNT":len(arms["control"]),"TREATMENT_NOVELTY_RATE":tnov,"CONTROL_NOVELTY_RATE":cnov,"ARM_NOVELTY_DELTA":abs(tnov-cnov),"TREATMENT_EFFECTIVE_DIVERSITY":tdiv,"CONTROL_EFFECTIVE_DIVERSITY":cdiv,"ARM_DIVERSITY_DELTA":abs(tdiv-cdiv),"EXPECTED_VALID_TREATMENT":len(arms["treatment"])*conservative_valid_per_query,"EXPECTED_VALID_CONTROL":len(arms["control"])*conservative_valid_per_query,"EXPECTED_VALID_TREATMENT_CONSERVATIVE":len(arms["treatment"])*conservative_valid_per_query,"EXPECTED_VALID_CONTROL_CONSERVATIVE":len(arms["control"])*conservative_valid_per_query,"TARGET_VALID_PER_ARM":target_valid_per_arm,"TREATMENT_CAPACITY_RATIO":rt,"CONTROL_CAPACITY_RATIO":rc,"CAPACITY_MARGIN_TREATMENT":len(arms["treatment"])*conservative_valid_per_query-target_valid_per_arm,"CAPACITY_MARGIN_CONTROL":len(arms["control"])*conservative_valid_per_query-target_valid_per_arm,"CAPACITY_MARGIN_POLICY":self.thresholds["min_capacity_margin"],"CAPACITY_RATIO_POLICY":self.thresholds["min_arm_capacity_ratio"],"HISTORICAL_RESULT_OVERLAP_RISK":risks["HISTORICAL_RESULT_OVERLAP_RISK"],"HISTORICAL_RESULT_OVERLAP_RISK_RAW":hrisk,"DOMAIN_CONCENTRATION_RISK":risks["DOMAIN_CONCENTRATION_RISK"],"DOMAIN_CONCENTRATION_RISK_RAW":drisk,"QUERY_FAMILY_SATURATION_RISK":risks["QUERY_FAMILY_SATURATION_RISK"],"QUERY_FAMILY_SATURATION_RISK_RAW":self._top_share(prepared),"RISK_CLASSIFICATIONS":risks,"PROVENANCE_INTEGRITY":"FAIL" if provenance_missing and mode==NEW_CANDIDATE_MODE else "PASS","MISSING_PROVENANCE_ROWS":provenance_missing if mode==NEW_CANDIDATE_MODE else 0,"PAIR_ASSIGNMENT_INTEGRITY":"FAIL" if integrity_fail else "PASS","ASSIGNMENT_QUERY_COUNT":sum(assigned.values()),"ASSIGNMENT_DUPLICATES":counts["duplicates"],"ASSIGNMENT_MISSING_QUERIES":counts["missing"],"ASSIGNMENT_UNKNOWN_QUERIES":counts["unknown"],"ASSIGNMENT_ARM_MISMATCHES":counts["mismatch"],"INCOMPLETE_PAIRS":incomplete,"SIMILARITY_STRATEGY":"EXHAUSTIVE_DETERMINISTIC_JACCARD"}
        sub={"INTEGRITY_GATE":"FAIL" if integrity_fail else "PASS","NOVELTY_GATE":"PASS","DIVERSITY_GATE":"PASS","ARM_BALANCE_GATE":"PASS","CAPACITY_GATE":"PASS"}; overlap=(cats["EXACT"]+cats["NORMALIZED"])/n if n else 0
        if overlap>self.thresholds["max_normalized_query_overlap_rate"]: sub["NOVELTY_GATE"]="FAIL"; reasons.append("NOVELTY_HISTORICAL_OVERLAP")
        if (dup/n if n else 0)>self.thresholds["max_query_duplicate_rate"] or (sum(jaccard(a["tokens"],b["tokens"])>=.80 for i,a in enumerate(prepared) for b in prepared[i+1:])*2/n if n else 0)>self.thresholds["max_query_near_duplicate_rate"] or (top[0][1]/n if top else 0) > self.thresholds["max_query_family_saturation_risk"] or (len(family)/n if n else 0)<self.thresholds["min_effective_query_diversity"] or metrics["PAIR_FAMILY_CONCENTRATION"]>self.thresholds["max_pair_family_concentration"] or metrics["CROSS_PAIR_SIMILARITY"]>self.thresholds["max_cross_pair_similarity"]: sub["DIVERSITY_GATE"]="FAIL"; reasons.append("DIVERSITY_POLICY")
        if metrics["ARM_NOVELTY_DELTA"]>self.thresholds["max_arm_novelty_delta"] or metrics["ARM_DIVERSITY_DELTA"]>self.thresholds["max_arm_diversity_delta"]: sub["ARM_BALANCE_GATE"]="FAIL"; reasons.append("ARM_BALANCE_POLICY")
        if len(arms["treatment"])!=len(arms["control"]): sub["ARM_BALANCE_GATE"]="FAIL"; reasons.append("ARM_BALANCE_POLICY")
        if any(metrics[k]<=self.thresholds["min_capacity_margin"] for k in ("CAPACITY_MARGIN_TREATMENT","CAPACITY_MARGIN_CONTROL")) or any(r is not None and r<self.thresholds["min_arm_capacity_ratio"] for r in (rt,rc)): sub["CAPACITY_GATE"]="FAIL"; reasons.append("CAPACITY_POLICY")
        decision="PASS" if not reasons else ("FAIL_INTEGRITY" if sub["INTEGRITY_GATE"]=="FAIL" else "FAIL_NOVELTY" if sub["NOVELTY_GATE"]=="FAIL" else "FAIL_DIVERSITY" if sub["DIVERSITY_GATE"]=="FAIL" else "FAIL_CAPACITY" if sub["CAPACITY_GATE"]=="FAIL" else "FAIL_BALANCE")
        return GateDecision(decision,tuple(dict.fromkeys(reasons)),metrics,self.thresholds,provenance or {"timestamp":"OFFLINE_DETERMINISTIC"},mode,sub)

def write_manifest(path,decision,candidate_path,historical_paths):
    doc=decision.as_dict(); doc["candidate_universe_sha256"]=sha256_file(candidate_path); doc["historical_sources"]=[{"path":str(p),"sha256":sha256_file(p)} for p in historical_paths]; doc["historical_source_sha256"]=sha256_bytes(canonical_json(doc["historical_sources"])); doc["artifact_sha256"]=sha256_bytes(canonical_json({k:v for k,v in doc.items() if k!="artifact_sha256"})); path.write_bytes(canonical_json(doc)); return doc
def freeze_candidate_universe(candidate,output_path,**kwargs):
    decision=PreExecutionNoveltyDiversityGate(kwargs.pop("thresholds",None)).evaluate(candidate,**kwargs)
    if decision.decision!="PASS": raise RuntimeError(f"FREEZE_BLOCKED_BY_{GATE_NAME}:{decision.decision}")
    output_path.write_bytes(canonical_json({**candidate,"freeze_status":"FROZEN","gate":decision.as_dict()})); return decision
def main():
    p=argparse.ArgumentParser(); p.add_argument("--candidate",type=Path,required=True); p.add_argument("--historical",type=Path,action="append",default=[]); p.add_argument("--results",type=Path,action="append",default=[]); p.add_argument("--manifest",type=Path,required=True); p.add_argument("--target-valid-per-arm",type=int,default=0); p.add_argument("--conservative-valid-per-query",type=float,default=0.0); a=p.parse_args(); c=json.loads(a.candidate.read_text()); d=PreExecutionNoveltyDiversityGate().evaluate(c,[json.loads(x.read_text()) for x in a.historical],[json.loads(x.read_text()) for x in a.results],target_valid_per_arm=a.target_valid_per_arm,conservative_valid_per_query=a.conservative_valid_per_query); write_manifest(a.manifest,d,a.candidate,a.historical); print(json.dumps(d.as_dict(),ensure_ascii=False,sort_keys=True)); raise SystemExit(0 if d.decision=="PASS" else 1)
if __name__=="__main__": main()
