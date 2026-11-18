//! Casefold / NFKC normalisation used by the deterministic fixture embedder.
//!
//! Copied from `amatl-core`'s `text::normalized_text`. That function is
//! `pub(crate)` in production: it is deliberately *not* part of the published
//! API, and this excluded experiment must not widen it. The boundary therefore
//! carries its own copy.
//!
//! It only feeds the deterministic [`FixtureBackend`](crate::FixtureBackend)
//! used by the latency harness and the failure-mode tests. The real
//! [`CandleBackend`](crate::CandleBackend) never calls it: it tokenises with the
//! model package's own WordPiece tokenizer.

use unicode_normalization::UnicodeNormalization;

/// NFKC + full casefold + whitespace collapse, matching the production
/// behaviour byte-for-byte so a fixture measurement stays comparable.
pub(crate) fn normalized_text(value: &str) -> String {
    let mut casefolded = String::new();
    for character in value.nfkc() {
        match character {
            'ß' | 'ẞ' => casefolded.push_str("ss"),
            'ς' => casefolded.push('σ'),
            _ => casefolded.extend(character.to_lowercase()),
        }
    }
    casefolded.split_whitespace().collect::<Vec<_>>().join(" ")
}
