use crate::model::{
    Document, DocumentStatus, Evidence, EvidenceFragment, EvidenceProvenance, EvidenceScoreBasis,
    EvidenceSignal, EvidenceStatus, EvidenceV2, FetchMethod, Query, RankingScore, SCHEMA_VERSION,
};
use crate::text::tokens;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const EVIDENCE_V2_VERSION: &str = "v2";
pub const EVIDENCE_V2_MAX_FRAGMENTS: usize = 8;
pub const EVIDENCE_V2_FRAGMENT_BYTES: usize = 512;

pub fn analyze_evidence(documents: &[Document]) -> Vec<Evidence> {
    let occurrences = documents
        .iter()
        .fold(BTreeMap::new(), |mut counts, document| {
            *counts
                .entry(document.content_hash.as_str())
                .or_insert(0_u32) += 1;
            counts
        });
    documents
        .iter()
        .map(|document| analyze_document(document, occurrences[document.content_hash.as_str()]))
        .collect()
}

pub fn analyze_evidence_v2(query: &Query, documents: &[Document]) -> Vec<EvidenceV2> {
    let evidence = analyze_evidence(documents);
    build_evidence_v2(query, documents, &evidence)
}

pub fn analyze_evidence_bundle(
    query: &Query,
    documents: &[Document],
) -> (Vec<Evidence>, Vec<EvidenceV2>) {
    let evidence = analyze_evidence(documents);
    let evidence_v2 = build_evidence_v2(query, documents, &evidence);
    (evidence, evidence_v2)
}

fn build_evidence_v2(
    query: &Query,
    documents: &[Document],
    evidence: &[Evidence],
) -> Vec<EvidenceV2> {
    let query_tokens = tokens(&query.normalized_query);
    documents
        .iter()
        .zip(evidence)
        .map(|(document, baseline)| {
            let content = document.content.as_deref();
            let extracted_content_hash = content.map(|value| hex_digest(value.as_bytes()));
            let provenance_id = provenance_id(document, extracted_content_hash.as_deref());
            let provenance = EvidenceProvenance {
                schema_version: SCHEMA_VERSION.into(),
                provenance_id: provenance_id.clone(),
                document_id: document.search_result_id.clone(),
                original_url: document.original_url.clone(),
                canonical_url: document.canonical_url.clone(),
                final_url: document.final_url.clone(),
                source_content_hash: document.content_hash.clone(),
                extracted_content_hash,
                fetch_method: document.fetch_method.clone(),
                extractor_used: document.extractor_used.clone(),
                retrieved_at: document.retrieved_at.clone(),
                published_at: document.published_at.clone(),
            };
            let fragments = content.map_or_else(Vec::new, |content| {
                evidence_fragments(content, &query_tokens, &provenance_id)
            });
            EvidenceV2 {
                schema_version: SCHEMA_VERSION.into(),
                evidence_version: EVIDENCE_V2_VERSION.into(),
                document_id: document.search_result_id.clone(),
                status: baseline.status.clone(),
                provenance,
                fragments,
                score_basis: EvidenceScoreBasis {
                    schema_version: SCHEMA_VERSION.into(),
                    fact_density: baseline.fact_density,
                    verified_date: baseline.verified_date,
                    metadata_quality: baseline.metadata_quality,
                    citation_count: baseline.citation_count,
                    citation_span: baseline.citation_span,
                    freshness: baseline.freshness,
                    originality: baseline.originality,
                },
                evidence_score: baseline.evidence_score,
            }
        })
        .collect()
}

#[derive(Debug)]
struct FragmentCandidate {
    start: usize,
    end: usize,
    priority: usize,
    matched_terms: Vec<String>,
    signals: Vec<EvidenceSignal>,
}

fn evidence_fragments(
    content: &str,
    query_tokens: &std::collections::BTreeSet<String>,
    provenance_id: &str,
) -> Vec<EvidenceFragment> {
    let mut candidates = bounded_content_ranges(content)
        .into_iter()
        .map(|(start, end)| {
            let text = &content[start..end];
            let fragment_tokens = tokens(text);
            let matched_terms = query_tokens
                .intersection(&fragment_tokens)
                .cloned()
                .collect::<Vec<_>>();
            let citation = contains_citation(text);
            let temporal = contains_iso_shaped_date(text);
            let numeric = text.bytes().any(|value| value.is_ascii_digit());
            let mut signals = Vec::new();
            if !matched_terms.is_empty() {
                signals.push(EvidenceSignal::QueryMatch);
            }
            if citation {
                signals.push(EvidenceSignal::Citation);
            }
            if temporal {
                signals.push(EvidenceSignal::Temporal);
            }
            if numeric {
                signals.push(EvidenceSignal::Numeric);
            }
            let priority = matched_terms.len() * 16
                + usize::from(citation) * 8
                + usize::from(temporal) * 4
                + usize::from(numeric) * 2
                + fragment_tokens.len().min(50) / 10;
            FragmentCandidate {
                start,
                end,
                priority,
                matched_terms,
                signals,
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.start.cmp(&right.start))
    });
    candidates.truncate(EVIDENCE_V2_MAX_FRAGMENTS);
    candidates.sort_by_key(|candidate| candidate.start);
    candidates
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| {
            let text = content[candidate.start..candidate.end].to_owned();
            let fragment_hash = hex_digest(text.as_bytes());
            let identity = format!(
                "evidence-fragment-v2\0{provenance_id}\0{}\0{}\0{fragment_hash}",
                candidate.start, candidate.end
            );
            EvidenceFragment {
                schema_version: SCHEMA_VERSION.into(),
                fragment_id: hex_digest(identity.as_bytes()),
                provenance_id: provenance_id.into(),
                ordinal: u32::try_from(index + 1).unwrap_or(u32::MAX),
                text,
                start_byte: u64::try_from(candidate.start).unwrap_or(u64::MAX),
                end_byte: u64::try_from(candidate.end).unwrap_or(u64::MAX),
                fragment_hash,
                matched_terms: candidate.matched_terms,
                signals: candidate.signals,
            }
        })
        .collect()
}

fn bounded_content_ranges(content: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut paragraph_start = 0;
    for (index, character) in content.char_indices() {
        if character == '\n' {
            append_bounded_range(content, paragraph_start, index, &mut ranges);
            paragraph_start = index + character.len_utf8();
        }
    }
    append_bounded_range(content, paragraph_start, content.len(), &mut ranges);
    ranges
}

fn append_bounded_range(content: &str, start: usize, end: usize, ranges: &mut Vec<(usize, usize)>) {
    let Some((mut cursor, end)) = trim_range(content, start, end) else {
        return;
    };
    while cursor < end {
        let hard_end = floor_char_boundary(
            content,
            cursor.saturating_add(EVIDENCE_V2_FRAGMENT_BYTES).min(end),
        );
        let split = if hard_end < end {
            preferred_split(content, cursor, hard_end)
        } else {
            end
        };
        if let Some(range) = trim_range(content, cursor, split) {
            ranges.push(range);
        }
        cursor = split.max(cursor + 1);
        while cursor < end && !content.is_char_boundary(cursor) {
            cursor += 1;
        }
    }
}

fn floor_char_boundary(content: &str, mut index: usize) -> usize {
    while index > 0 && !content.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn preferred_split(content: &str, start: usize, hard_end: usize) -> usize {
    let minimum = start + (hard_end - start) / 2;
    let mut sentence = None;
    let mut whitespace = None;
    for (relative, character) in content[start..hard_end].char_indices() {
        let after = start + relative + character.len_utf8();
        if after < minimum {
            continue;
        }
        if matches!(character, '.' | '!' | '?') {
            sentence = Some(after);
        } else if character.is_whitespace() {
            whitespace = Some(after);
        }
    }
    sentence.or(whitespace).unwrap_or(hard_end)
}

fn trim_range(content: &str, mut start: usize, mut end: usize) -> Option<(usize, usize)> {
    while start < end {
        let character = content[start..end].chars().next()?;
        if !character.is_whitespace() {
            break;
        }
        start += character.len_utf8();
    }
    while start < end {
        let character = content[start..end].chars().next_back()?;
        if !character.is_whitespace() {
            break;
        }
        end -= character.len_utf8();
    }
    (start < end).then_some((start, end))
}

fn contains_citation(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("http://") || value.contains("https://")
}

fn contains_iso_shaped_date(value: &str) -> bool {
    value.as_bytes().windows(10).any(|window| {
        window[0..4].iter().all(u8::is_ascii_digit)
            && window[4] == b'-'
            && window[5..7].iter().all(u8::is_ascii_digit)
            && window[7] == b'-'
            && window[8..10].iter().all(u8::is_ascii_digit)
    })
}

fn provenance_id(document: &Document, extracted_content_hash: Option<&str>) -> String {
    let identity = format!(
        "evidence-provenance-v2\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
        document.search_result_id,
        document.original_url.0.as_str(),
        document.canonical_url.0.as_str(),
        document.final_url.0.as_str(),
        document.content_hash,
        extracted_content_hash.unwrap_or_default(),
        fetch_method(&document.fetch_method),
        document.extractor_used.as_deref().unwrap_or_default(),
        document.retrieved_at,
        document.published_at.as_deref().unwrap_or_default(),
    );
    hex_digest(identity.as_bytes())
}

fn fetch_method(value: &FetchMethod) -> &'static str {
    match value {
        FetchMethod::Http => "http",
        FetchMethod::Rendered => "rendered",
    }
}

fn hex_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn analyze_document(document: &Document, duplicate_count: u32) -> Evidence {
    let content = document.content.as_deref().unwrap_or_default();
    let content_tokens = tokens(content);
    let token_count = content_tokens.len().max(1) as f64;
    let numeric = content
        .split_whitespace()
        .filter(|token| token.chars().any(|character| character.is_ascii_digit()))
        .count() as f64;
    let fact_density = (numeric / token_count * 12.0).clamp(0.0, 1.0);
    let citation_count = count_occurrences(content, "http://")
        .saturating_add(count_occurrences(content, "https://")) as u32;
    let citation_span = (f64::from(citation_count) / (token_count / 100.0).max(1.0)).min(1.0);
    let metadata_fields = [
        document.title.is_some(),
        document.author.is_some(),
        document.published_at.is_some(),
        document.content_type.is_some(),
        document.extractor_used.is_some(),
        document.content.is_some(),
    ];
    let metadata_quality = metadata_fields.iter().filter(|value| **value).count() as f64
        / metadata_fields.len() as f64;
    let published = document
        .published_at
        .as_deref()
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok());
    let retrieved = OffsetDateTime::parse(&document.retrieved_at, &Rfc3339).ok();
    let verified_date = published
        .zip(retrieved)
        .is_some_and(|(published, retrieved)| published <= retrieved);
    let freshness = published
        .zip(retrieved)
        .map_or(0.0, |(published, retrieved)| {
            let age_days = (retrieved - published).whole_seconds().max(0) as f64 / 86_400.0;
            2_f64.powf(-age_days / 180.0)
        });
    let originality = 1.0 / f64::from(duplicate_count.max(1));
    let evidence_score = (0.25 * fact_density
        + 0.25 * metadata_quality
        + 0.15 * f64::from(verified_date)
        + 0.15 * citation_span
        + 0.10 * freshness
        + 0.10 * originality)
        .clamp(0.0, 1.0);
    Evidence {
        schema_version: SCHEMA_VERSION.into(),
        document_id: document.search_result_id.clone(),
        status: if document.status == DocumentStatus::Enriched {
            EvidenceStatus::Complete
        } else {
            EvidenceStatus::Partial
        },
        fact_density: RankingScore::bounded(fact_density),
        verified_date,
        metadata_quality: RankingScore::bounded(metadata_quality),
        named_entities: vec![],
        citation_count,
        citation_span: RankingScore::bounded(citation_span),
        freshness: RankingScore::bounded(freshness),
        originality: RankingScore::bounded(originality),
        evidence_score: RankingScore::bounded(evidence_score),
    }
}

fn count_occurrences(value: &str, pattern: &str) -> usize {
    value.match_indices(pattern).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_query, CanonicalUrl, FetchMethod, OriginalUrl};
    use url::Url;

    fn document(id: &str, content: Option<&str>, hash: &str) -> Document {
        let url = Url::parse(&format!("https://example.com/{id}")).unwrap();
        Document {
            schema_version: SCHEMA_VERSION.into(),
            search_result_id: id.into(),
            original_url: OriginalUrl(url.clone()),
            canonical_url: CanonicalUrl(url.clone()),
            final_url: crate::FinalUrl(url),
            content_hash: hash.into(),
            fetch_method: FetchMethod::Http,
            extractor_used: content.map(|_| "test-v1".into()),
            content_type: Some("text/html".into()),
            size: 10,
            retrieved_at: "2026-08-12T00:00:00Z".into(),
            status: if content.is_some() {
                DocumentStatus::Enriched
            } else {
                DocumentStatus::Superficial
            },
            content: content.map(str::to_owned),
            title: Some(id.into()),
            author: None,
            published_at: Some("2026-08-01T00:00:00Z".into()),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn evidence_is_deterministic_bounded_and_keeps_score_separate() {
        let documents = vec![
            document(
                "a",
                Some("Report 2026 cites https://example.org/source"),
                "same",
            ),
            document("b", None, "same"),
        ];
        let first = analyze_evidence(&documents);
        assert_eq!(first, analyze_evidence(&documents));
        assert_eq!(first[0].citation_count, 1);
        assert_eq!(first[0].originality, RankingScore::new(0.5).unwrap());
        assert!(first
            .iter()
            .all(|value| (0.0..=1.0).contains(&value.evidence_score.get())));
        assert_eq!(first[1].status, EvidenceStatus::Partial);
    }

    #[test]
    fn evidence_v2_fragments_are_exact_deterministic_and_traceable() {
        let content = "General introduction without a claim.\nAMATL security reached 95 percent on 2026-08-01; source https://example.org/report";
        let documents = vec![document("a", Some(content), "raw-response-hash")];
        let query = parse_query("AMATL security".into()).unwrap();
        let baseline = analyze_evidence(&documents);
        let first = analyze_evidence_v2(&query, &documents);
        assert_eq!(first, analyze_evidence_v2(&query, &documents));
        assert_eq!(first[0].schema_version, SCHEMA_VERSION);
        assert_eq!(first[0].evidence_version, EVIDENCE_V2_VERSION);
        assert_eq!(first[0].evidence_score, baseline[0].evidence_score);
        assert_eq!(first[0].score_basis.fact_density, baseline[0].fact_density);
        assert_eq!(first[0].provenance.source_content_hash, "raw-response-hash");
        assert_eq!(
            first[0].provenance.extracted_content_hash,
            Some(hex_digest(content.as_bytes()))
        );
        assert_eq!(first[0].provenance.provenance_id.len(), 64);
        assert!(!first[0].fragments.is_empty());
        for (index, fragment) in first[0].fragments.iter().enumerate() {
            let start = usize::try_from(fragment.start_byte).unwrap();
            let end = usize::try_from(fragment.end_byte).unwrap();
            assert_eq!(fragment.ordinal as usize, index + 1);
            assert_eq!(fragment.text, content[start..end]);
            assert_eq!(fragment.fragment_hash, hex_digest(fragment.text.as_bytes()));
            assert_eq!(fragment.provenance_id, first[0].provenance.provenance_id);
            assert!(fragment.text.len() <= EVIDENCE_V2_FRAGMENT_BYTES);
        }
        let strongest = first[0]
            .fragments
            .iter()
            .find(|fragment| fragment.matched_terms == ["amatl", "security"])
            .unwrap();
        assert!(strongest.signals.contains(&EvidenceSignal::QueryMatch));
        assert!(strongest.signals.contains(&EvidenceSignal::Citation));
        assert!(strongest.signals.contains(&EvidenceSignal::Temporal));
        assert!(strongest.signals.contains(&EvidenceSignal::Numeric));
    }

    #[test]
    fn evidence_v2_is_utf8_safe_bounded_and_links_duplicate_text_to_each_source() {
        let content = format!("common evidence\n{}", "á".repeat(3_000));
        let documents = vec![
            document("a", Some(&content), "raw-a"),
            document("b", Some(&content), "raw-b"),
        ];
        let query = parse_query("common evidence".into()).unwrap();
        let evidence = analyze_evidence_v2(&query, &documents);
        assert_eq!(evidence.len(), 2);
        assert!(evidence
            .iter()
            .all(|value| value.fragments.len() <= EVIDENCE_V2_MAX_FRAGMENTS));
        for value in &evidence {
            for fragment in &value.fragments {
                let start = usize::try_from(fragment.start_byte).unwrap();
                let end = usize::try_from(fragment.end_byte).unwrap();
                assert!(content.is_char_boundary(start));
                assert!(content.is_char_boundary(end));
                assert_eq!(fragment.text, content[start..end]);
                assert!(fragment.text.len() <= EVIDENCE_V2_FRAGMENT_BYTES);
            }
        }
        assert_ne!(
            evidence[0].provenance.provenance_id,
            evidence[1].provenance.provenance_id
        );
        assert_ne!(
            evidence[0].fragments[0].fragment_id,
            evidence[1].fragments[0].fragment_id
        );
        assert_eq!(
            evidence[0].fragments[0].fragment_hash,
            evidence[1].fragments[0].fragment_hash
        );
    }

    #[test]
    fn superficial_document_has_provenance_without_invented_fragments() {
        let documents = vec![document("a", None, "raw")];
        let query = parse_query("anything".into()).unwrap();
        let evidence = analyze_evidence_v2(&query, &documents);
        assert_eq!(evidence[0].status, EvidenceStatus::Partial);
        assert!(evidence[0].fragments.is_empty());
        assert!(evidence[0].provenance.extracted_content_hash.is_none());
        assert_eq!(
            evidence[0].evidence_score,
            analyze_evidence(&documents)[0].evidence_score
        );
    }
}
