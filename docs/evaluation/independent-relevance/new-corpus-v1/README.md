# Independent relevance corpus acquisition — 2026-08-31

`NEW_CORPUS_STATUS=ADJUDICATION_REQUIRED`.

The campaign identity was frozen before labelling in `corpus-identity.json`.
It used only the operator-approved AMATL providers and recorded every raw
provider response under `acquisition/raw/`. The frozen pre-label snapshot has
368 valid rows; its hash is recorded in `prelabel/prelabel-manifest.json`.

The two completed packets in `prelabel/` have been validated against the
frozen snapshot. The reproducible validation and agreement report is
`adjudication/agreement-report.json`; the human-only disagreement queue is
`adjudication/adjudication-packet.json`. Annotators must not run AMATL
candidates, inspect historical labels, or inspect each other's packet.

```
LABELER_A_STATUS=COMPLETE_VALIDATED
LABELER_B_STATUS=COMPLETE_VALIDATED
AGREEMENT=336/368 (91.304348%)
COHEN_KAPPA=0.684441824
DISAGREEMENTS=32
ADJUDICATION_STATUS=NOT_STARTED
WORK_PACKAGE_STATUS=BLOCKED_WAITING_FOR_ADJUDICATION
```

All 32 disagreements are `MINOR_BOUNDARY` decisions (Relevant ↔
PossiblyRelevant or PossiblyRelevant ↔ NotRelevant). There are no extreme or
Unknown conflicts. This is a meaningful concentration at class boundaries,
so `LABELING_TAXONOMY_RISK=BOUNDARY_AMBIGUITY`; it is an observation, not a
change to the frozen taxonomy.

No final corpus, selection set, or blind holdout exists:

```
SELECTION_STATUS=NOT_CREATED
BLIND_HOLDOUT_STATUS=NOT_CREATED
BLIND_GROUND_TRUTH_ACCESS=NOT_APPLICABLE
```

The next single action is independent human completion of every blank
`adjudicated_label` in `adjudication/adjudication-packet.json`, preserving all
source fields. Only then may the validator create final ground truth and
calculate final class deficits.

Capacity is already provably insufficient for the required natural 100
Relevant examples: 5 are agreed Relevant and only 4 remaining disagreements
could possibly become Relevant, for a hard maximum of 9. Final deficits remain
unknown until adjudication, but the minimum possible Relevant deficit is 91.
At the observed first-campaign Relevant yield of 7/368 (1.90%), acquiring 91
new final Relevant labels alone implies about 4,784 raw observations; a 25%
acquisition margin gives a preliminary `RAW_ACQUISITION_TARGET=5,980` for that
class. The machine-readable specification is
`adjudication/supplemental-acquisition-spec.json`. This is planning evidence
only, not authorization to acquire data.

Rows not selected remain an append-only reserve pool. A separate sealing tool
and focused validation must be completed before any `CLEAN_UNSEEN` claim.
