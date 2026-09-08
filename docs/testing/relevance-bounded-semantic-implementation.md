# STEP 3B — Bounded Semantic Relevance — Implementation (frozen, pre-holdout)

**Status:** `BOUNDED_SEMANTIC_IMPLEMENTATION_FROZEN`

The bounded-semantic relevance layer is implemented and evaluated **only against
the existing 172-row development corpus**. The independent held-out corpus
([`crates/amatl-core/tests/fixtures/relevance/holdout.json`](../../crates/amatl-core/tests/fixtures/relevance/holdout.json))
was frozen **unlabeled** at commit `82fd8e2f052976134dd4d7567aca2c6e903d8d38`
before any rule, alias, intent pattern, or threshold in this layer was designed.
Its `expected_label` fields are empty. **The final held-out campaign has not been
run** — it is blocked on independent ground-truth labels.

- **Baseline under test:** `13e9aa980555355c9f30b66310ba04c61a8bad0a`
- **Holdout freeze:** `82fd8e2f052976134dd4d7567aca2c6e903d8d38` (unlabeled)
- **Branch:** `fix/audit-repository-hygiene`
- **AdaptiveRouter / routing / telemetry / production thresholds:** unchanged.

---

## 1. Architecture

The purely lexical STEP 2E heuristic is **extended, not replaced**. Every
decision keeps its components on `ResultRelevanceAssessment`; there is no opaque
semantic score.

```
LEXICAL EVIDENCE
  + BOUNDED SEMANTIC EVIDENCE (morphology, alias/concept map)
  + INTENT / ENTITY / SUBJECT CONSISTENCY
  + ITEMISED NEGATIVE EVIDENCE
        -> EXPLAINABLE RELEVANCE DECISION
```

New module: [`crates/amatl-core/src/relevance_semantics.rs`]. New feature flags
on `RelevanceThresholds` (`semantic_morphology`, `semantic_aliases`,
`semantic_intent`), all `true` in production; `RelevanceThresholds::lexical_only()`
reproduces the STEP 2E baseline byte-for-byte.

### 1.1 Morphology (`MORPHOLOGY_MODEL = internal explicit rules`)

A ~30-rule ordered suffix stripper (`-ing`, `-ed`, `-s/-es`, `-ation/-ution`,
`-er/-or`, `-ly`, `-ment`, `-ness`, `-ability/-ibility`, …), English only,
`MIN_STEM_LEN = 5`, non-ascii / non-lowercase tokens skipped, doubled-consonant
collapse (`programming -> programm -> program`), plus a 5-char stem-prefix
fallback so `distributed`/`distribution` link. **No dependency added.** Original
tokens are always retained; stems are additional. `stemmed_term_matches` on the
assessment records exactly which query term stemmed into a match.

### 1.2 Concept / alias map (`ALIAS_MODEL = ConceptAliasSet table`)

`ALIAS_SETS` — **14 concept groups** (`documentation`, `repository`,
`asynchronous`, `database`, `kubernetes`, `configuration`, `standard library`,
`tutorial`, `javascript`, `postgresql`, `performance`, `authentication`,
`autoscaling`, `specification`). Each group is a general vocabulary
relationship; the doc comment carries the rationale + generalization reason.
`alias_map_has_no_fixture_specific_terms` rejects sample-id shapes, URLs, and
over-long entries. An `alias_match` requires BOTH the query and the result text
to evidence the same canonical concept.

### 1.3 Query intent (`INTENT_MODEL = QueryIntent enum, 6 variants`)

`QueryIntent`: `FactualCapital{entity}`, `FactualAttribution{work}`,
`Documentation{subject}`, `Tutorial{subject}`, `Repository{subject}`,
`Definition{subject}`. **~30 high-confidence surface patterns**
(`capital of X`, `X documentation`, `documentation for X`, `who wrote X`,
`inventor of X`, `X github`, `X tutorial`, `getting started with X`,
`what is X`, …). Only fires on unquoted, filter-free queries; quoted-phrase
queries keep the pure lexical path.

### 1.4 Entity / subject consistency + negative evidence

`NegativeRelevanceEvidence { subject_mismatch, entity_mismatch,
weak_generic_match, alias_conflict }` — four **named** booleans, never one
penalty.

- `entity_mismatch` — e.g. `capital of Canada` vs a page asserting "capital of
  the province of Ontario". Detected by a *competing* "capital of the
  province/state" / "is the capital" assertion when the queried entity is not
  the one tied to "capital".
- `subject_mismatch` — the intent subject's terms are absent and a different
  `KNOWN_SUBJECTS` name dominates the title (`SQLite WAL documentation` -> a
  PostgreSQL WAL page). `KNOWN_SUBJECTS` is a closed list of ~40 product names
  carrying **no facts** — only "this string names a distinct thing".
- `weak_generic_match` — the only overlap was a URL-path token or a generic
  word (`capital`, `guide`, `overview`, …).
- `alias_conflict` — query and result name different members of the same
  product family (`ALIAS_CONFLICT_FAMILIES`).

`is_blocking()` = `subject_mismatch || entity_mismatch`.

### 1.5 Decision model

Unchanged: `EXACT_PHRASE -> Relevant`, `!had_sufficient_text -> Unknown`.

Added, in order:
1. `lexical_strong && blocking_negative` -> **downgraded** to
   PossiblyRelevant / Unknown (never Relevant). This is the `fact-002` fix.
2. `lexical_strong` -> Relevant (unchanged).
3. `lexical_moderate && strong_semantic_rescue` -> Relevant, where
   `strong_semantic_rescue = !blocking && subject_match && (alias_match ||
   entity_match || semantic_cov >= 0.75)`.
4. `intent_active && intent_rescue_qualifies(...)` -> Relevant. Attribution
   intents require a genuine `entity_match` (a page that merely names the work
   is not the answer — the `fact-008` guard); subject intents require
   `subject_match` + corroboration.
5. Weak-lexical + semantic evidence -> PossiblyRelevant, unless blocking /
   `weak_generic_match` demotes it.
6. `not_relevant_ceiling` / snippet rules -> unchanged.

**`alias_match` alone never reaches Relevant. `stemmed_match` alone never
reaches Relevant.** Enforced by `alias_alone_does_not_force_relevant` and
`stemming_alone_does_not_force_relevant`.

---

## 2. Ablation study — development corpus (172)

`cargo test -p amatl-core --test relevance_empirical_validation
step3b_ablation_study_dev_corpus -- --nocapture --test-threads=1`

| arm | RELEVANT_P | RELEVANT_R | MACRO_F1 | URE_P | URE_R | NR→R |
|---|---|---|---|---|---|---|
| A. LEXICAL_BASELINE | 0.9492 | 0.6154 | 0.6805 | 0.7000 | 1.0000 | 0.0263 |
| B. + MORPHOLOGY | 0.9492 | 0.6154 | 0.6874 | 0.7000 | 1.0000 | 0.0263 |
| C. + ALIASES | 0.9492 | 0.6154 | 0.6846 | 0.7000 | 1.0000 | 0.0263 |
| **D. + INTENT / NEGATIVE (FULL)** | **0.9672** | **0.6484** | **0.6857** | **0.7778** | **1.0000** | **0.0000** |

**Reading:**

- **Morphology and aliases alone are near-neutral** on headline metrics
  (MACRO_F1 ±0.007, no recall change). They do not independently justify their
  complexity — their value is as *corroborating inputs* to the intent layer
  (`subject_match` uses stems; `strong_semantic_rescue` uses `alias_match`).
- **The full signal improves every headline metric and drives the
  safety-critical `NOT_RELEVANT → RELEVANT` rate to zero.** RELEVANT precision
  +1.8pp, RELEVANT recall +3.3pp, URE precision +7.8pp, RELEVANT false-positive
  rate 0.051 -> 0.033.
- **MACRO_F1 (0.686) is still below the 0.70 gate on the dev corpus** — the
  fuzzy POSSIBLY_RELEVANT class is still the ceiling, as in STEP 3.

### Regression check (routes that already worked)

| direction | baseline | full 3B |
|---|---|---|
| `UNKNOWN → RELEVANT` | 0.0000 | 0.0000 (held) |
| `NOT_RELEVANT → RELEVANT` | 0.0263 | 0.0000 (improved) |
| EXACT_PHRASE path | Relevant | Relevant (test-enforced, `exact_phrase_behavior_is_preserved`) |
| UNKNOWN precision | 1.0000 | 1.0000 |
| UNKNOWN recall | 0.9375 | 0.8750 (1 `insuf` row -> POSSIBLY, **not** RELEVANT) |

The single UNKNOWN-recall slip moves a row into POSSIBLY, never toward RELEVANT;
the safe direction is intact.

### Errors remaining on dev

- **`gen-014`** (HIGH): `how to stay focused while working from home` vs
  "Remote work productivity: 12 tips" — **zero** lexical, stem, alias, or intent
  overlap. This is the irreducible bag-of-words ceiling; it is the case that
  would justify a local embedding.
- POSSIBLY_RELEVANT precision/recall remain the MACRO_F1 drag.

---

## 3. Complexity guardrails

| metric | value |
|---|---|
| `ALIAS_COUNT` (concept groups) | 14 |
| alias surface forms (total) | ~90 |
| `INTENT_RULE_COUNT` (surface patterns) | ~30 across 6 variants |
| `KNOWN_SUBJECTS` | ~40 (names only, no facts) |
| `NEW_DEPENDENCIES` | 0 |
| stemmer rules | ~30 |

The layer is pure set/string work over already-normalized tokens; no allocation
beyond small `BTreeSet`s. It runs before ranking and is orders of magnitude
cheaper than any provider fetch. (No artificial microbenchmark is claimed.)

---

## 4. What is NOT done

- **No held-out evaluation.** `holdout.json` labels are empty by design.
- **No embeddings, ML, LLM, external service, adaptive telemetry, routing
  change.**
- **No production threshold change.** `RelevanceThresholds` numeric fields are
  the STEP 2E `INITIAL_HEURISTIC` values, untouched.
- **Telemetry remains blocked.**

`NEXT_ACTION = WAIT_FOR_INDEPENDENT_HOLDOUT_LABELS`, then run
`step3b` + the (to-be-added) final held-out campaign and evaluate the full gate
set on the held-out set only.

---

## Appendix — tests added

`relevance_semantics.rs`: `stem_collapses_inflection`,
`stem_leaves_short_and_proper_tokens_alone`, `morphological_variant_can_match`,
`alias_match_is_auditable`, `intent_capital_of_detected`,
`intent_documentation_detected`, `entity_mismatch_on_wrong_capital`,
`subject_mismatch_on_wrong_product`, `paraphrase_subject_still_matches_docs`,
`weak_generic_match_flagged_for_url_only_overlap`,
`identical_inputs_produce_identical_semantic_assessment`,
`alias_map_has_no_fixture_specific_terms`.

`relevance.rs`: `exact_phrase_behavior_is_preserved`,
`insufficient_evidence_behavior_is_preserved`,
`alias_alone_does_not_force_relevant`,
`stemming_alone_does_not_force_relevant`,
`entity_mismatch_blocks_false_relevant`,
`intent_mismatch_does_not_gain_relevance`,
`semantic_evidence_can_rescue_paraphrase`, `negative_evidence_is_explainable`,
`new_relevance_fields_are_serialization_compatible`.

`relevance_empirical_validation.rs`: `step3b_ablation_study_dev_corpus`,
`step3b_full_report_dev_corpus`, `step3b_ure_false_positive_detail_dev_corpus`,
`step3b_ablation_baseline_is_reproducible`,
`step3b_new_signal_does_not_change_routing`,
`step3b_legacy_mode_remains_compatible`.
