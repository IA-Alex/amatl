# STEP 3B — Bounded Semantic Relevance — Final Held-Out Campaign

**Status:** `BOUNDED_SEMANTIC_RELEVANCE_IMPROVED_BUT_INSUFFICIENT`
**Telemetry:** remains **blocked** (`TELEMETRY_UNLOCKED = NO`).

- **Implementation under test:** `66598c95e4ffcc22cc8bf551d4a2cc08cd37399a`
  (`feat(search): add bounded semantic relevance signals`) — run verbatim,
  `RelevanceThresholds::default()`, no tuning before or after.
- **Held-out freeze:** `82fd8e2f052976134dd4d7567aca2c6e903d8d38` — 93 samples,
  inputs frozen **before** any bounded-semantic rule/alias/intent design.
- **Ground truth:** assigned by an independent party after the freeze
  (`holdout_labels_independent.json`), merged into `holdout.json` (only
  `expected_label` / `rationale`; every input field byte-value identical to the
  freeze — verified structurally; the file was additionally pretty-printed).
- **Campaign runs:** exactly **1**.
- **Harness:** [`crates/amatl-core/tests/relevance_holdout_final.rs`] — loads the
  corpus, calls production `assess_result`, computes metrics. No heuristic logic
  re-implemented.

---

## 1. Held-out label distribution

| label | count | share |
|---|---|---|
| RELEVANT | 59 | 63.4% |
| NOT_RELEVANT | 20 | 21.5% |
| POSSIBLY_RELEVANT | 10 | 10.8% |
| UNKNOWN | 4 | 4.3% |

URE subset (EXPANSION, `confirmed_overlap = false`) = **47**; URE-positive
(labelled RELEVANT) = **16**.

> The independent labeller applied a **more generous RELEVANT standard** than
> the development corpus: results that merely *support* an answer without
> stating it were labelled RELEVANT (`ho-ent-005` "Mount Kenya is second after
> Kilimanjaro" → RELEVANT; `ho-ent-012` "it is not the capital; that is Albany"
> → RELEVANT), and every paraphrased how-to was RELEVANT regardless of shared
> vocabulary. This is a legitimate semantic-utility judgement and is exactly the
> regime a bounded lexical signal is weakest in.

---

## 2. Results — production signal, single run

```
FINAL_HOLDOUT_SAMPLE_COUNT = 93
FINAL_HOLDOUT_ACCURACY     = 0.3978
FINAL_HOLDOUT_MACRO_F1     = 0.5012
```

### Per-class

| class | precision | recall | F1 |
|---|---|---|---|
| RELEVANT | 0.8400 | 0.3559 | 0.5000 |
| POSSIBLY_RELEVANT | 0.1569 | 0.8000 | 0.2623 |
| NOT_RELEVANT | 0.3077 | 0.2000 | 0.2424 |
| UNKNOWN | 1.0000 | 1.0000 | 1.0000 |

### Confusion matrix (rows = expected, cols = predicted)

| | RELEVANT | POSSIBLY | NOT_REL | UNKNOWN | total |
|---|---|---|---|---|---|
| **RELEVANT** | 21 | 30 | 8 | 0 | 59 |
| **POSSIBLY_RELEVANT** | 1 | 8 | 1 | 0 | 10 |
| **NOT_RELEVANT** | 3 | 13 | 4 | 0 | 20 |
| **UNKNOWN** | 0 | 0 | 0 | 4 | 4 |

### AMATL-critical

```
FINAL_HOLDOUT_URE_SUBSET = 47   URE_TP=4  URE_FP=4  URE_FN=12
FINAL_HOLDOUT_URE_PRECISION = 0.5000
FINAL_HOLDOUT_URE_RECALL    = 0.2500
FINAL_HOLDOUT_NR_TO_R_ERROR_RATE      = 0.1500   (3 / 20)
FINAL_HOLDOUT_UNKNOWN_TO_R_ERROR_RATE = 0.0000   (0 / 4)
FINAL_HOLDOUT_RELEVANT_FALSE_POSITIVE_RATE = 0.1600   (4 / 25)
```

---

## 3. Acceptance gates

| gate | threshold | value | result |
|---|---|---|---|
| RELEVANT_PRECISION | ≥ 0.85 | 0.8400 | **FAIL** |
| RELEVANT_RECALL | ≥ 0.70 | 0.3559 | **FAIL** |
| URE_PRECISION | ≥ 0.85 | 0.5000 | **FAIL** |
| MACRO_F1 | ≥ 0.70 | 0.5012 | **FAIL** |
| NR_TO_R_ERROR_RATE | ≤ 0.10 | 0.1500 | **FAIL** |
| UNKNOWN_TO_R_ERROR_RATE | ≤ 0.10 | 0.0000 | **PASS** |

`ALL_GATES_PASS = false` (5 of 6 fail).

---

## 4. Error analysis (56 errors, implementation unchanged)

| direction | count | share |
|---|---|---|
| `RELEVANT → POSSIBLY_RELEVANT` | 30 | 54% |
| `NOT_RELEVANT → POSSIBLY_RELEVANT` | 13 | 23% |
| `RELEVANT → NOT_RELEVANT` | 8 | 14% |
| `NOT_RELEVANT → RELEVANT` (critical) | 3 | 5% |
| `POSSIBLY_RELEVANT → RELEVANT` | 1 | 2% |
| `POSSIBLY_RELEVANT → NOT_RELEVANT` | 1 | 2% |

**Dominant failure — RELEVANT under-called, 38 / 56 (68%):**

- **Zero / low lexical-overlap paraphrase & synonymy** — the answer is correct
  but worded unlike the query. `ho-para-001` "prevent burnout" ↔ "job-related
  exhaustion"; `ho-para-002` "make a website load faster" ↔ "front-end
  performance optimization"; `ho-para-005` "reduce cloud computing costs" ↔
  "cutting your AWS bill"; `ho-gen-005` "fix a leaky faucet" ↔ "repairing a
  dripping tap"; `ho-para-008` "speed up python code" ↔ "optimizing Python
  performance". The morphology + alias layer narrows some of this but cannot
  bridge `burnout`↔`exhaustion`, `faucet`↔`tap`, `website`↔`front-end`,
  `costs`↔`bill`.
- **Support-not-statement factual answers** — `ho-ent-005`, `ho-ent-004`,
  `ho-partial-007`, `ho-rare-*` where the labeller judged a result that implies
  or contextualises the answer as RELEVANT. No bounded rule performs that
  inference.
- **`X documentation` intent cases still POSSIBLY** — `ho-doc-001/003/005`,
  `ho-syn-002`, `ho-intent-001/008`. The intent-rescue path exists but its
  "subject_match + corroboration" bar is not cleared by these specific
  phrasings (`typescript generics documentation` → title "Documentation -
  Generics"; `kubectl cheat sheet` → "kubectl Quick Reference"). This is the
  one band where **rule coverage**, not paraphrase, is the limiter.

**Secondary — NOISE not demoted, 13 / 56 (23%):** lexical-collision rows
(`java` coffee, `python` snake, `mercury` planet, `go` board game, `kafka` the
novelist) reach POSSIBLY_RELEVANT via incidental URL/word overlap instead of
NOT_RELEVANT. Same structural issue documented in STEP 3; the `weak_generic_match`
and `alias_conflict` signals do not fire because the collision term is a real
title token, not a URL-only or generic one.

**Critical — `NOT_RELEVANT → RELEVANT`, 3:** all FACTUAL entity near-misses.
`ho-ent-001` ("Sydney … capital city of the state of New South Wales" — the
entity-consistency layer catches "capital of the province" but not "capital
city of the state" phrasing); `ho-ent-008` (Edison, telephone — attribution
cue present, wrong person); `ho-ent-009` ("largest freshwater lake … by surface
area" vs unqualified "largest lake"). These are **rule-coverage gaps** in the
entity/attribution logic, not paraphrase.

### Verdict on dominant cause

The failure is **dominated by zero-overlap paraphrase, synonymy, and
conceptual/semantic equivalence** (≈ 38 of 56 errors, 68%), which no bounded
lexical rule set can resolve. Intent/entity/alias rule defects account for a
minority (≈ 6–9 errors: the 3 critical entity near-misses + several
`X documentation` misses). `NR_TO_R = 0.15` also breaches its gate and is
partly addressable by widening the entity-consistency patterns, but fixing it
would not move the four gates that fail because of paraphrase recall.

```
LOCAL_EMBEDDING_JUSTIFIED = YES
```

---

## 5. Comparison to development corpus

| metric | dev (172) | held-out (93) |
|---|---|---|
| RELEVANT precision | 0.9672 | 0.8400 |
| RELEVANT recall | 0.6484 | 0.3559 |
| MACRO_F1 | 0.6857 | 0.5012 |
| URE precision | 0.7778 | 0.5000 |
| NR → R | 0.0000 | 0.1500 |

The dev-corpus gains did **not** generalise. The held-out set's more generous,
inference-tolerant RELEVANT standard collapses RELEVANT recall (0.65 → 0.36),
and the entity-consistency rules that zeroed `NR → R` on dev miss the held-out
phrasing variants. This is the same "tuning did not generalise" signal STEP 3
saw with thresholds — now confirmed for bounded semantic rules on an
independent set.

---

## 6. Decision

```
STATUS = BOUNDED_SEMANTIC_RELEVANCE_IMPROVED_BUT_INSUFFICIENT
TELEMETRY_UNLOCKED = NO
LOCAL_EMBEDDING_JUSTIFIED = YES
NEXT_ACTION = LOCAL_EMBEDDING_EXPERIMENT
```

The bounded semantic layer is a **real, safe improvement over the pure lexical
baseline on the development corpus** (NR→R 0.026→0.000, RELEVANT recall
+0.033, URE precision +0.078, no regression) and it is worth keeping as a
component: it is deterministic, explainable, adds no dependency, and the
`UNKNOWN` / `EXACT_PHRASE` paths remain solid (UNKNOWN F1 = 1.00 on held-out).

But it **does not clear the acceptance gates on independent data**, and the
gap is structural: recognising that "dripping tap" answers "leaky faucet"
requires distributional/semantic similarity that discrete lexical rules cannot
encode. The next iteration should evaluate a **small local embedding** (offline,
no remote inference) as a *bounded auxiliary* signal — used to rescue
`RELEVANT → POSSIBLY` paraphrase under-scoring and to demote NOISE that reaches
POSSIBLY — layered on top of, not replacing, the current lexical + bounded
semantic model.

The bounded-semantic implementation stays committed (it improves the dev
baseline and is a prerequisite substrate); telemetry stays blocked; no
production threshold or rule is changed as a result of this campaign.
