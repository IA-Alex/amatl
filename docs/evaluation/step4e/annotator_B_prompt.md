# STEP4E annotator B instructions

Annotate `step4e_annotation_packet_B.json` independently. Do not run AMATL; do not ask for or inspect predictions; and do not inspect the other annotation file. Return exactly one label per row, preserve `row_id`, do not alter dataset fields, and do not omit difficult rows—use `Unknown` where required.

# STEP4E blind relevance annotation rubric

Use one label for each row. Judge only the supplied query, title, snippet, and URL.

- **Relevant**: directly satisfies the query intent or provides strongly useful information for the requested subject.
- **PossiblyRelevant**: related and potentially useful, but incomplete, indirect, ambiguous, or only partially satisfies intent.
- **NotRelevant**: does not satisfy the query intent despite possible lexical or topical similarity.
- **Unknown**: the supplied title/snippet/URL information is insufficient for a defensible decision.

Do not infer AMATL behavior, optimize labels for anticipated model performance, consult historical AMATL labels, or use information outside the supplied row. Annotate rows independently. Use Unknown when evidence is insufficient.
