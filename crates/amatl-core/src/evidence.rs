use crate::model::{
    Document, DocumentStatus, Evidence, EvidenceStatus, RankingScore, SCHEMA_VERSION,
};
use crate::text::tokens;
use std::collections::BTreeMap;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

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
    use crate::{CanonicalUrl, FetchMethod, OriginalUrl};
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
}
