# Independent relevance corpus acquisition — 2026-08-31

`NEW_CORPUS_STATUS=GROUND_TRUTH_FROZEN`.

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
ADJUDICATION_STATUS=COMPLETE
V1_ROLE=KNOWN_INDEPENDENT_GROUND_TRUTH
WORK_PACKAGE_STATUS=COMPLETE_V1_WAITING_FOR_SUPPLEMENTAL_AUTHORIZATION
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

The completed human packet is preserved at
`adjudication/adjudication-packet.json`. The frozen reconstruction is
`adjudication/v1-ground-truth.json`, with all source fields, A/B labels,
final-label provenance, hashes and role recorded by
`adjudication/v1-ground-truth-manifest.json`. V1 is known ground truth and is
not, and can never again be claimed as, a blind holdout.

Final V1 distribution is 9 Relevant, 51 PossiblyRelevant, 308 NotRelevant and
0 Unknown. Deficits to the 100/100/100 minimum are 91, 49 and 0 respectively.
The old 5,980 estimate is superseded: final Relevant yield is 9/368 (2.4457%),
so the same unstratified 25% calculation is 4,652 raw rows for 91 Relevant.
It is not an acquisition commitment. Rank and query evidence is in
`adjudication/v1-yield-analysis.json`; it finds 7 of 9 V1 Relevant rows at
ranks 1--3 and none below rank 7, while explicitly not assigning provider
causality from a one-provider campaign.

The designed, pre-label-only supplemental pilot is
`adjudication/supplemental-pilot-design.json`: 180 rows across two shallow
query-type strata and a mixed-depth control stratum. It remains unexecuted
because the existing acquisition specification states
`NOT_AUTHORIZED_BY_THIS_WORK_PACKAGE`. No supplemental packets exist and no
labels have been created. The next single action is explicit authorization for
that 180-row capture; after a contamination check and pre-label freeze, two
identical unlabeled A/B packets must be generated and work must stop for the
independent evaluators.

Rows not selected remain an append-only reserve pool. A separate sealing tool
and focused validation must be completed before any `CLEAN_UNSEEN` claim.
