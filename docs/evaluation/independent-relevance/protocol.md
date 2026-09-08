# Independent relevance-evaluation protocol

## Current classification

`STEP4E_ROLE=KNOWN_DEVELOPMENT_BENCHMARK`. Its ground truth, predictions, metrics and error analysis are already visible, so it is immutable evidence but never a blind test again.

`OLD_HOLDOUT_ROLE=HISTORICAL_CONTAMINATED_BENCHMARK`. The 93-row bounded-semantic corpus was opened and used before candidate, thresholds and policy were frozen. Its state is permanently `CONSUMED`.

`INDEPENDENT_PROTOCOL_STATUS=REQUIRES_NEW_LABELLED_DATA`. All relevance-labelled datasets in this repository have been observed or used to make decisions. Repartitioning them cannot make labels unseen.

## Lifecycle

`PROPOSED → DEVELOPMENT → ELIGIBLE_FOR_SELECTION → SELECTED → FROZEN → BLIND_EVALUATED → ACCEPTED|REJECTED`.

Development uses STEP4E and the 172-row corpus for debugging, thresholds and analysis. A separate newly acquired labelled selection set may be opened for comparison; once read it is `KNOWN_TUNED`, never final. A final blind set remains `CLEAN_UNSEEN` until a frozen candidate is evaluated exactly once.

When new labels arrive, `tools/relevance_protocol.py split INPUT SEED OUTPUT` creates the stratified selection/blind row assignment. It records the input hash, seed, fraction and output identity; the same inputs and seed reproduce it, while a changed seed creates a different identity.

Before `FROZEN`, record implementation commit, model ID/hash, backend, parameters, thresholds, preprocessing, evaluator path/hash and artifact path/hash. `tools/relevance_protocol.py freeze-candidate` derives the identity; `verify-candidate` rejects changed evaluator or artifact. Any change creates a new candidate identity.

The policy in `pass-fail-policy.json` is the required pre-opening policy. Its canonical JSON SHA-256 is recorded in the consumption record, so changing a threshold changes policy identity.

## Blind opening and consumption

The evaluator operator first verifies the dataset inventory and frozen candidate. Only then may they open the blind labels and evaluate. `consume-holdout` writes a record containing holdout ID/hash, opening time, candidate ID/commit, model hash, evaluator hash, policy hash and result hash, and marks the event `CONSUMED`. A consumed ID must be changed to `HISTORICAL_ONLY` in the inventory; it cannot return to `CLEAN_UNSEEN`.

## New-data requirement

Acquire no data in this work package. For a viable next campaign, obtain at least 300 new independently labelled rows: 100 Relevant, 100 PossiblyRelevant and 100 NotRelevant, split before development into 150 selection and 150 final blind rows (50/class each). Use a deterministic, recorded seed only for assignment after labels are sealed; stratify by class and, when available in adequate counts, query family/provider. Preserve row IDs, raw inputs, labels, provenance, source hash and annotator/adjudication record. Keep final labels in access controlled material unavailable to developers; publish only its input hash and manifest until candidate/policy/freeze are complete.

The 150-row final minimum supports class-level reporting but remains modest; report uncertainty and do not create small multi-way strata.
