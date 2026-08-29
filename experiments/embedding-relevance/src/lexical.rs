//! Mode A — lexical baseline. A deliberately simple token-overlap classifier
//! standing in for "STEP 2E lexical baseline". This is a diagnostic reference
//! point, not a reproduction of amatl-core's production `assess_result`.

use crate::{token_overlap, Label};

/// Classify by query-token coverage of the document text.
///
/// Thresholds here are diagnostic knobs, tuned only on the CONSUMED corpora.
pub fn classify(query: &str, doc: &str) -> Label {
    let ov = token_overlap(query, doc);
    if doc.trim().is_empty() {
        return Label::Unknown;
    }
    if ov >= 0.75 {
        Label::Relevant
    } else if ov >= 0.4 {
        Label::PossiblyRelevant
    } else {
        Label::NotRelevant
    }
}
