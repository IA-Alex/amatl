# STEP4E Evaluation Report
Frozen blind evaluation of **ARM_A** (production deterministic relevance) vs **ARM_B** (production + Candle semantic advisory) on the 150-row STEP4E holdout.
- Rows: **150** (50 direct / 50 collision / 50 limited)
- Model: `BAAI/bge-small-en-v1.5` (load 3015 ms)
- Advisory: `SemanticEvaluator` over `bge-small-en-v1.5`, paraphrase threshold 0.72, bounded-semantic corroboration required.
- Bootstrap CIs: 10,000 resamples, seed 42 (deterministic).

## Headline result
**ARM_B == ARM_A byte-for-byte.** The advisory changed **0** of 150 rows; both artifacts share an identical SHA-256. The Candle semantic pass is **inert** on this holdout.

## Primary metrics (vs ground truth)
| metric | ARM_A | ARM_B | Δ (B−A) |
|---|---|---|---|
| accuracy | 0.2267 [0.1600, 0.2933] | 0.2267 [0.1600, 0.2933] | +0.0000 [0.0000, 0.0000] |
| macro_f1 | 0.2601 [0.1879, 0.3264] | 0.2601 [0.1879, 0.3264] | +0.0000 [0.0000, 0.0000] |
| rescue_recall | 0.3000 [0.1754, 0.4314] | 0.3000 [0.1754, 0.4314] | +0.0000 [0.0000, 0.0000] |
| rescue_precision | 1.0000 [1.0000, 1.0000] | 1.0000 [1.0000, 1.0000] | +0.0000 [0.0000, 0.0000] |
| protection_specificity | 0.1400 [0.0488, 0.2444] | 0.1400 [0.0488, 0.2444] | +0.0000 [0.0000, 0.0000] |
| false_promotion_count | 0.0000 [0.0000, 0.0000] | 0.0000 [0.0000, 0.0000] | +0.0000 [0.0000, 0.0000] |

### Per-class (ARM_A == ARM_B)
| class | precision | recall | F1 |
|---|---|---|---|
| NotRelevant | 0.156 | 0.140 | 0.147 |
| PossiblyRelevant | 0.133 | 0.240 | 0.171 |
| Relevant | 1.000 | 0.300 | 0.462 |

## Stratified by holdout stratum
| stratum | n | GT | ARM_A acc | ARM_B acc | Δ acc |
|---|---|---|---|---|---|
| direct | 50 | Relevant:50 | 0.300 | 0.300 | +0.000 |
| collision | 50 | NotRelevant:49, PossiblyRelevant:1 | 0.160 | 0.160 | +0.000 |
| limited | 50 | PossiblyRelevant:49, NotRelevant:1 | 0.220 | 0.220 | +0.000 |

## Semantic-change analysis
- Candidates (PossiblyRelevant rows embedded): **90**
- Embedding agreed (cosine ≥ 0.72): **38**
- Bounded-semantic corroboration (`bounded_strong_rescue`): **0**
- Suggestions emitted: **0**; rows changed: **0**
- **Binding gate: `bounded_strong_rescue`.** The embedding agrees on 38/90 candidates, but the lexical corroboration layer never fires, so the advisory is a no-op.
- Counterfactual (embedding-only, ignoring the bounded gate): would promote **38** rows — 32 correct (GT Relevant), 5 incorrect (GT NotRelevant), 1 GT PossiblyRelevant.

## Root cause
The bounded gate requires `subject_match && (alias_matches || entity_match)` or `alias_matches >= 2`. On this synthetic holdout the topics (bicycle chains, composting, smoke detectors, …) are **outside the ALIAS_SETS concept vocabulary** (async, kubernetes, postgresql, …) and the imperative query shapes do not match `detect_intent` patterns, so `subject_match`/`entity_match` are always false. The embedding itself is not the limiter — it agrees on 42% of candidates — the lexical corroboration layer is structurally inert here.

## Performance
- Model load: 3015 ms
- ARM_A (deterministic): 26 ms
- ARM_B (semantic): 44500 ms (incremental 44474 ms)
- CPU time: 490050.0 ms (CPU_TIME_READABLE)
- Peak RSS delta: 264.5546875 MB
- Label counts — ARM_A {'NotRelevant': 45, 'PossiblyRelevant': 90, 'Relevant': 15}, ARM_B {'NotRelevant': 45, 'PossiblyRelevant': 90, 'Relevant': 15}

## Conclusion
The STEP4E holdout does not exercise the semantic advisory: ARM_B is identical to ARM_A. The advisory is safe (zero false promotions) but also provides zero rescues on this data. A future holdout whose topics overlap the concept vocabulary, or a relaxation of the bounded gate, would be needed to observe advisory behavior.
