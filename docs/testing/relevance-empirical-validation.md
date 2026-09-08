# STEP 3 — Relevance Empirical Validation

**Status:** `RELEVANCE_HEURISTIC_INSUFFICIENT` (for the RELEVANT-recall / `MACRO_F1`
dimension) — the safety-critical direction (`NOT_RELEVANT → RELEVANT`) is strong,
but the errors that block the gates are **structural** (lexical matching cannot
recognise semantic paraphrase or wrong-entity precision), not correctable by
moving thresholds. A threshold grid that scored perfectly on the tuning split
collapsed on the held-out validation split, confirming the ceiling is not a
tuning problem.

- **Commit under test:** `a97d9320e200212fbc44d41c021a87ea4048405a`
  (`feat(search): add deterministic relevance assessment`)
- **Branch:** `fix/audit-repository-hygiene`
- **Corpus:** [`crates/amatl-core/tests/fixtures/relevance/corpus.json`](../../crates/amatl-core/tests/fixtures/relevance/corpus.json)
- **Evaluator:** [`crates/amatl-core/tests/relevance_empirical_validation.rs`](../../crates/amatl-core/tests/relevance_empirical_validation.rs)
- **Campaign command:**
  ```
  cargo test -p amatl-core --test relevance_empirical_validation -- --nocapture --test-threads=1
  ```
- **AdaptiveRouter:** not changed. **Telemetry:** not changed. **Production
  thresholds:** not changed.

---

## 1. Current thresholds (baseline, unchanged)

`RelevanceThresholds::default()` — marked `INITIAL_HEURISTIC` in `relevance.rs`:

| field | value |
|---|---|
| `relevant_title_coverage` | 0.75 |
| `relevant_combined_coverage` | 1.0 |
| `relevant_snippet_coverage` | 0.90 |
| `possibly_relevant_coverage` | 0.40 |
| `not_relevant_ceiling` | 0.25 |
| `min_title_tokens_without_snippet` | 2 |

The heuristic is deterministic, local (no LLM / embeddings / network), computed
from title/snippet/url text + provider-original rank only, and does not read the
final ranking. Tokenisation is the shared `crate::text::{normalized_text,
tokens}`; significant query terms = query tokens minus a fixed 28-word stop list,
plus every quoted-phrase token.

---

## 2. Corpus composition

**172 observations.** Heterogeneous across the 10 required families; built
without network access.

### Provenance

| provenance | count | meaning |
|---|---|---|
| `DERIVED_REAL` | 132 | title/snippet paraphrased or trimmed from the real shape of results for that query (canonical docs pages, Wikipedia lede sentences, project homepages) |
| `SYNTHETIC_EDGE_CASE` | 40 | constructed only to probe a boundary (lexical collisions, empty snippets, generic titles) |
| `REAL` (verbatim from repo fixtures) | 0 | the repo carries no captured SearXNG/Marginalia result corpus to draw verbatim rows from; the phase-2e fixtures are themselves hand-built. Documented as a corpus limitation. |

### Ground-truth label distribution

| label | count | share |
|---|---|---|
| RELEVANT | 91 | 52.9% |
| NOT_RELEVANT | 38 | 22.1% |
| POSSIBLY_RELEVANT | 27 | 15.7% |
| UNKNOWN | 16 | 9.3% |

> The corpus is **RELEVANT-heavy** vs. the 25–35% suggested target. This reflects
> the realistic top-of-results mix for well-formed queries but it inflates the
> RELEVANT-precision denominator and should be rebalanced (more PARTIAL_RELEVANCE
> and NOISE) before any gate is treated as statistically firm. Reported, not
> silently corrected.

### Query-class distribution

`DOCUMENTATION` 34, `NOISE` 23, `TECHNICAL` 21, `FACTUAL` 21, `GENERAL` 18,
`PARTIAL_RELEVANCE` 15, `NAVIGATION` 13, `INSUFFICIENT_EVIDENCE` 10,
`RARE_TERM` 9, `EXACT_PHRASE` 8.

### Provider role

`PRIMARY` 87, `EXPANSION` 85. The **Unique-Relevant-Expansion (URE)** subset =
EXPANSION role with `confirmed_overlap = false` = **85 rows**, of which **7** are
human-labelled RELEVANT.

### Ground-truth method

Each row carries a one-line `rationale` describing **semantic utility** ("would a
user asking this be helped by this result?"), never lexical overlap. Labels were
assigned before running the heuristic and are not derived from its output. The
evaluator invokes the production `assess_result` directly — no re-implementation.

---

## 3. Baseline results — full corpus, default thresholds

```
TOTAL_SAMPLES = 172
ACCURACY      = 0.6395
MACRO_F1      = 0.6735
```

### Class distribution: expected vs predicted

| class | expected | predicted |
|---|---|---|
| RELEVANT | 91 | 59 |
| POSSIBLY_RELEVANT | 27 | 60 |
| NOT_RELEVANT | 38 | 38 |
| UNKNOWN | 16 | 15 |

The heuristic **under-calls RELEVANT (59 vs 91)** and **over-calls
POSSIBLY_RELEVANT (60 vs 27)**.

### Per-class precision / recall / F1

| class | precision | recall | F1 |
|---|---|---|---|
| RELEVANT | **0.9492** | 0.6154 | 0.7467 |
| POSSIBLY_RELEVANT | 0.2333 | 0.5185 | 0.3218 |
| NOT_RELEVANT | 0.6579 | 0.6579 | 0.6579 |
| UNKNOWN | 1.0000 | 0.9375 | 0.9677 |

### Confusion matrix (rows = expected, cols = predicted)

| | RELEVANT | POSSIBLY | NOT_REL | UNKNOWN | total |
|---|---|---|---|---|---|
| **RELEVANT** | 56 | 34 | 1 | 0 | 91 |
| **POSSIBLY_RELEVANT** | 2 | 14 | 11 | 0 | 27 |
| **NOT_RELEVANT** | 1 | 12 | 25 | 0 | 38 |
| **UNKNOWN** | 0 | 0 | 1 | 15 | 16 |

### AMATL-critical rates

| metric | value | reading |
|---|---|---|
| `RELEVANT_FALSE_POSITIVE_RATE` | 0.0508 | 3 of 59 RELEVANT calls are wrong — low |
| `NOT_RELEVANT_TO_RELEVANT_ERROR_RATE` | **0.0263** | 1 of 38 — well under the 0.10 blocker line |
| `UNKNOWN_TO_RELEVANT_ERROR_RATE` | **0.0000** | 0 of 16 |
| `POSSIBLE_TO_RELEVANT_RATE` | 0.0741 | 2 of 27 |
| `NOT_RELEVANT_FALSE_NEGATIVE_RATE` | 0.3421 | 13 of 38 real noise not called noise (mostly → POSSIBLY) |

### Unique-Relevant-Expansion (URE) subset

```
URE_SUBSET_SIZE         = 85
URE_TP = 7   URE_FP = 3   URE_FN = 0
URE_PRECISION           = 0.7000
URE_RECALL              = 1.0000
URE_FALSE_POSITIVE_RATE = 0.0385
```

URE recall is perfect (every genuinely-unique-relevant expansion result was
caught) but precision is 0.70 on only 10 positive predictions — **3 false
positives** are the decisive weakness for the future telemetry signal, because a
URE false positive turns exclusive noise into `UNIQUE_RELEVANT_EXPANSION`.

---

## 4. Acceptance gates

`INITIAL_ACCEPTANCE_GATES` (proposed in the campaign brief):

| gate | threshold | value | result |
|---|---|---|---|
| `RELEVANT_PRECISION` | ≥ 0.85 | 0.9492 | **PASS** |
| `URE_PRECISION` | ≥ 0.85 | 0.7000 | **FAIL** |
| `NOT_RELEVANT_TO_RELEVANT_ERROR_RATE` | ≤ 0.10 | 0.0263 | **PASS** |
| `UNKNOWN_TO_RELEVANT_ERROR_RATE` | ≤ 0.10 | 0.0000 | **PASS** |
| `MACRO_F1` | ≥ 0.70 | 0.6735 | **FAIL** |

`CURRENT_THRESHOLDS_PASS_GATES = false` (2 of 5 fail).

**Are the failing gates appropriate?**

- `MACRO_F1 ≥ 0.70` is appropriate in principle; it fails here because
  POSSIBLY_RELEVANT is a genuinely fuzzy class that the heuristic uses as a
  catch-all. `MACRO_F1` is the right signal that the *middle* of the scale is not
  yet trustworthy.
- `URE_PRECISION ≥ 0.85` is being measured on **10 positive predictions**. The
  gate is not wrong, but with this corpus it is **under-powered** — one
  additional false positive swings it by 0.10. It should be re-evaluated once the
  corpus carries ≥ 30 URE positives.

Neither gate was altered.

---

## 5. Error analysis

### Confusion-cost tally (severity model is diagnostic only)

| severity | count | sample IDs |
|---|---|---|
| CRITICAL (`NOT_RELEVANT → RELEVANT`) | **1** | `fact-002` |
| HIGH (`RELEVANT → NOT_RELEVANT`, `UNKNOWN → RELEVANT`) | **1** | `gen-014` |
| MEDIUM | 26 | mostly `RELEVANT/POSSIBLY/NOT` boundary and NOISE→POSSIBLY |
| LOW | 34 | `RELEVANT → POSSIBLY_RELEVANT` (the dominant error) |

### The single CRITICAL error — `fact-002`

```
query     = "capital of Canada"
title     = "Toronto - Wikipedia"
snippet   = "Toronto is the most populous city in Canada and the capital of the province of Ontario."
expected  = NOT_RELEVANT   predicted = RELEVANT
title_coverage=0.00  snippet_coverage=1.00  url_coverage=0.00  exact_phrase=false
```

Root cause: **`QUERY_INTENT_MISMATCH` / `LEXICAL_FALSE_POSITIVE`.** The snippet
contains every significant query term ("capital", "canada") because it discusses
Toronto being the *provincial* capital. `snippet_coverage = 1.0` with ≥ 2 matched
terms triggers `strong_snippet` → RELEVANT. No lexical rule can tell "capital of
the province of Ontario" from "capital of Canada" — this needs entity/relation
understanding.

### The single HIGH error — `gen-014`

```
query     = "how to stay focused while working from home"
title     = "Remote work productivity: 12 tips"
snippet   = "Set a dedicated workspace, keep regular hours, use time-blocking, and take real breaks away from screens."
expected  = RELEVANT   predicted = NOT_RELEVANT
title_coverage=0.00  snippet_coverage=0.00  url_coverage=0.00
```

Root cause: **`LEXICAL_FALSE_NEGATIVE` / `MISSING_CONTEXT`.** A perfect answer
that shares *zero* tokens with the query ("focused"→"productivity",
"working from home"→"remote work"). Combined coverage 0, snippet present →
NotRelevant. This is the structural ceiling of bag-of-words matching.

### Root-cause distribution across CRITICAL + HIGH + MEDIUM errors

| root-cause class | approx. share | examples |
|---|---|---|
| `LEXICAL_FALSE_NEGATIVE` (paraphrase, synonymy) | ~40% | `gen-014`, `tech-013`, `fact-007`, `fact-011`, `rare-001`, most `RELEVANT → POSSIBLY` |
| `LEXICAL_FALSE_POSITIVE` / `QUERY_INTENT_MISMATCH` | ~20% | `fact-002`, `noise-007` (right concept, wrong language), `noise-009` (wrong sense of "capital"), `doc-006` (WAL, wrong DB) |
| `GENERIC_TITLE` / `MISSING_CONTEXT` (NOISE → POSSIBLY not NOT_RELEVANT) | ~25% | `noise-001..006` — lexical-collision rows land in POSSIBLY because a stray token matches the URL or a shared word |
| `THRESHOLD_BOUNDARY` | ~15% | `fact-013`, `fact-018`, `fact-022` — partial-evidence rows near `possibly_relevant_coverage` |

The dominant failure mode (**~65% combined**) is that lexical overlap is a poor
proxy for semantic utility in **both** directions. Only ~15% of the errors look
like they sit on a threshold that could be nudged.

### Per-family observations

- **FACTUAL** — RELEVANT rows where the snippet states the answer in different
  words than the query score POSSIBLY, not RELEVANT (`fact-007`, `fact-011`,
  `fact-019..025`). Factual queries systematically under-score.
- **NOISE** — lexical-collision rows (`rust` rust-on-a-car, `python` the snake,
  `kafka` the author) are correctly kept out of RELEVANT but land in
  POSSIBLY_RELEVANT rather than NOT_RELEVANT, because a URL token or a shared
  incidental word gives non-zero coverage. This is why
  `NOT_RELEVANT_FALSE_NEGATIVE_RATE` is 0.34.
- **RARE_TERM** — works when the rare term appears verbatim in the title
  (`phason`, `borophene`, `skyrmion`); a rare term that the page describes
  without naming would be missed. Small sample (9).
- **EXACT_PHRASE** — the strongest family: the quoted-phrase exact-match path is
  precise (`phrase-001,002,004,006,007,008` all correct). `phrase-003` (shares
  "zero"/"copy", phrase absent) correctly NOT_RELEVANT.
- **NAVIGATION** — homepages with a real snippet score RELEVANT; bare homepages
  with an empty snippet score UNKNOWN (`tech-010`, `insuf-005`), which is the
  intended conservative behaviour.
- **INSUFFICIENT_EVIDENCE** — 15/16 UNKNOWN rows correctly UNKNOWN; UNKNOWN
  precision 1.00, recall 0.94. This part of the heuristic is solid.

---

## 6. Threshold sensitivity

Deterministic stratified 70/30 split by FNV-1a hash of the sample id, fixed seed
`0x544533505f33` → **tuning = 124, validation = 48**. Grid:

- `relevant_title_coverage ∈ {0.60, 0.70, 0.75, 0.80, 0.90}`
- `possibly_relevant_coverage ∈ {0.30, 0.40, 0.50}`
- `not_relevant_ceiling ∈ {0.15, 0.25, 0.35}`
- `relevant_combined_coverage ∈ {0.90, 1.0, 1.10}`

Selection metric (tuning only): `0.5·RELEVANT_P + 0.3·URE_P + 0.2·MACRO_F1`,
gated on `nr2r ≤ 0.10`.

| config | split | RELEVANT_P | URE_P | MACRO_F1 | nr2r |
|---|---|---|---|---|---|
| default | tuning | 0.974 | 0.857 | 0.699 | 0.000 |
| default | validation | 0.900 | 0.333 | 0.582 | — |
| **best-on-tuning** `title 0.80 / possibly 0.40 / ceiling 0.15 / combined 0.90` | tuning | 1.000 | 1.000 | 0.720 | 0.000 |
| **same config** | **validation** | 0.895 | **0.333** | **0.621** | 0.100 |

**Result: `THRESHOLD_SENSITIVITY_RESULT = no generalising improvement found.`**
The grid point that looks perfect on the tuning split (URE_P 1.00, MACRO_F1 0.72)
performs no better than the default on the untouched validation split (URE_P
0.33, MACRO_F1 0.62) and pushes `nr2r` up to the 0.10 line. This is textbook
overfitting to a small corpus — the improvement is noise, not signal.

No production threshold change is defensible from this data.
`RECOMMENDED_THRESHOLDS = (unchanged)` — see §8.

---

## 7. Conclusion

Answering the campaign's closing questions with evidence:

1. **How many false positives does RELEVANT produce?** 3 of 59 calls
   (`RELEVANT_FALSE_POSITIVE_RATE = 0.051`); 1 of those is CRITICAL
   (`NOT_RELEVANT → RELEVANT`, rate 0.026). **Low, and below the blocker line.**
2. **How many genuinely relevant results does it lose?** RELEVANT recall 0.62 —
   **35 of 91** real RELEVANT results are down-graded, almost all to
   POSSIBLY_RELEVANT (34) rather than lost to NOT_RELEVANT (1). Under-confident,
   not unsafe.
3. **How does UNKNOWN behave?** Very well — precision 1.00, recall 0.94, zero
   `UNKNOWN → RELEVANT`. The insufficient-evidence path is trustworthy.
4. **Is URE precision sufficient?** **No** — 0.70 on the full corpus, and the
   candidate tuning collapses to 0.33 on held-out data. But the sample is only 10
   positive predictions; the gate is under-powered here.
5. **Which query families produce errors?** FACTUAL and GENERAL (paraphrase →
   under-scored to POSSIBLY); NOISE (lexical collisions land in POSSIBLY instead
   of NOT_RELEVANT). EXACT_PHRASE and INSUFFICIENT_EVIDENCE are strong.
6. **Threshold or structural?** **Structural.** ~65% of errors are lexical-vs-
   semantic mismatch in both directions; the threshold grid produced no
   improvement that survived the validation split.
7. **Reproducible?** Yes — `empirical_baseline_is_reproducible` and
   `fixed_seed_split_is_reproducible` pass; the evaluator is deterministic and
   offline.

**The `NOT_RELEVANT → RELEVANT` proportion is 0.026 — below the "significant
proportion" blocker.** The heuristic is *safe* to observe. It is not *accurate*
enough (RELEVANT recall, `MACRO_F1`, URE precision) to be a quality signal
without a non-lexical component.

### Decision

```
STATUS = RELEVANCE_HEURISTIC_INSUFFICIENT
```

Rationale: the architecture (deterministic, explainable, non-circular,
role-aware) is sound and the safety-critical direction passes, but the gates that
fail (`MACRO_F1`, `URE_PRECISION`) fail for **structural** reasons — bag-of-words
coverage cannot recognise paraphrase (`gen-014`) or wrong-entity precision
(`fact-002`) — and a controlled threshold search on a train/validation split
produced **no** change that generalised. Moving thresholds would be overfitting.

This is not `RELEVANCE_THRESHOLD_TUNING_REQUIRED` because tuning was tried and
did not work. It is not `EMPIRICAL_DATASET_INSUFFICIENT` because 172 samples give
a defensible read on the *structural* limitation (even if the URE gate
specifically wants a bigger sample).

---

## 8. Next action

**Do not proceed to relevance-aware telemetry on this signal as-is.**

Recommended sequence:

1. **Keep the heuristic and its thresholds unchanged** (`INITIAL_HEURISTIC`
   stays). `RECOMMENDED_THRESHOLDS = unchanged`.
2. **Redesign the signal to add a bounded non-lexical component** before
   telemetry — options, cheapest first, all still local:
   - title/snippet **stemming + a synonym/alias map** for common query
     reformulations (remote↔home, capital-city entity typing);
   - a small **local embedding** (still offline, no remote inference) used *only*
     to rescue `RELEVANT → POSSIBLY` under-scoring and to demote NOISE that
     currently lands in POSSIBLY;
   - keep the exact-phrase and insufficient-evidence paths exactly as they are —
     they already work.
3. **Rebalance and grow the corpus** to ~250–300 rows: raise NOISE and
   PARTIAL_RELEVANCE, and specifically add ≥ 30 URE-positive rows so the
   `URE_PRECISION` gate becomes powered. Add `REAL` verbatim rows once a
   SearXNG/Marginalia capture fixture exists.
4. **Re-run this campaign** against the redesigned signal. Only on
   `STATUS = RELEVANCE_EMPIRICALLY_VALIDATED` does
   `STRUCTURAL COMPLEMENTARITY → DETERMINISTIC RELEVANCE → EMPIRICAL VALIDATION →
   TELEMETRY → ADAPTIVE ROUTING` advance to the telemetry step.

The evaluator, corpus, split, and gates in this iteration are the reusable
harness for that re-run.

---

## Appendix — tests added

`crates/amatl-core/tests/relevance_empirical_validation.rs`:

| test | purpose |
|---|---|
| `empirical_baseline_full_corpus` | runs the baseline, prints the full report, checks + reports gates (hard-fails only under `AMATL_RELEVANCE_ENFORCE_GATES=1`) |
| `threshold_sensitivity_grid_on_tuning_split` | grid search on the tuning split, honest re-evaluation on the validation split |
| `corpus_schema_is_valid` | ≥ 80 samples, unique ids, valid urls/labels/roles/provenance, parseable queries |
| `all_samples_have_ground_truth` | every row has a non-empty `expected_label` |
| `all_samples_have_rationale` | every row has a rationale that is not a lexical-only phrase |
| `empirical_baseline_is_reproducible` | two runs produce identical metrics |
| `confusion_matrix_totals_match_sample_count` | matrix sums to N, per-class totals sum to N |
| `metrics_have_safe_denominators` | empty run → finite metrics, no NaN |
| `unique_relevant_expansion_subset_is_measured` | URE subset is non-trivial (≥ 10) |
| `fixed_seed_split_is_reproducible` | the 70/30 split is deterministic and ~stratified |

The acceptance gates are **not** wired to fail the functional test suite; they
fail only the dedicated empirical-validation job (env flag), so development is
not blocked while the signal is redesigned.
