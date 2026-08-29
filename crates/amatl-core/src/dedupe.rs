use crate::model::{
    CanonicalResult, CanonicalUrl, DeduplicatedResult, DuplicateStatus, MergeReason, SCHEMA_VERSION,
};
use crate::text::normalized_confusable_text;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

pub fn deduplicate(mut results: Vec<CanonicalResult>) -> Vec<DeduplicatedResult> {
    results.sort_by(|left, right| {
        left.canonical_url
            .cmp(&right.canonical_url)
            .then_with(|| left.original_url.cmp(&right.original_url))
            .then_with(|| left.provider.cmp(&right.provider))
    });
    let mut groups = BTreeMap::<CanonicalUrl, Vec<CanonicalResult>>::new();
    for result in results {
        groups
            .entry(result.canonical_url.clone())
            .or_default()
            .push(result);
    }
    let mut output = groups
        .into_values()
        .map(merge_confirmed_group)
        .collect::<Vec<_>>();
    mark_possible_duplicates(&mut output);
    output
}

fn merge_confirmed_group(mut group: Vec<CanonicalResult>) -> DeduplicatedResult {
    group.sort_by(|left, right| {
        Reverse(left.title.is_some())
            .cmp(&Reverse(right.title.is_some()))
            .then_with(|| Reverse(left.snippet.is_some()).cmp(&Reverse(right.snippet.is_some())))
            .then_with(|| Reverse(&left.published_at).cmp(&Reverse(&right.published_at)))
            .then_with(|| left.canonical_url.cmp(&right.canonical_url))
            .then_with(|| left.original_url.cmp(&right.original_url))
            .then_with(|| left.provider.cmp(&right.provider))
    });
    let representative = &group[0];
    let providers = group
        .iter()
        .map(|result| result.provider.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut provider_ranks = BTreeMap::new();
    for result in &group {
        provider_ranks
            .entry(result.provider.clone())
            .and_modify(|rank: &mut Option<crate::Rank>| {
                *rank = match (*rank, result.provider_rank) {
                    (Some(left), Some(right)) => Some(left.min(right)),
                    (None, right) => right,
                    (left, None) => left,
                }
            })
            .or_insert(result.provider_rank);
    }
    let original_urls = group
        .iter()
        .map(|result| result.original_url.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let snippets = group
        .iter()
        .filter_map(|result| result.snippet.clone())
        .collect::<BTreeSet<_>>();
    let alternate_snippets = snippets
        .into_iter()
        .filter(|snippet| Some(snippet) != representative.snippet.as_ref())
        .collect();
    let observed_dates = group
        .iter()
        .filter_map(|result| result.published_at.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let confirmed = group.len() > 1;
    let original_exact = confirmed
        && group
            .iter()
            .all(|result| result.original_url == representative.original_url);
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: representative.title.clone(),
        original_url: representative.original_url.clone(),
        canonical_url: representative.canonical_url.clone(),
        original_urls,
        providers,
        representative_provider: representative.provider.clone(),
        provider_ranks,
        snippet: representative.snippet.clone(),
        alternate_snippets,
        result_type: representative.result_type.clone(),
        published_at: representative.published_at.clone(),
        author: representative.author.clone(),
        language: representative.language.clone(),
        file_type: representative.file_type.clone(),
        thumbnail: representative.thumbnail.clone(),
        metadata: representative.metadata.clone(),
        observed_dates,
        duplicate_status: if confirmed {
            DuplicateStatus::ConfirmedDuplicate
        } else {
            DuplicateStatus::Distinct
        },
        merge_reason: confirmed.then_some(if original_exact {
            MergeReason::OriginalUrlExact
        } else {
            MergeReason::CanonicalUrlExact
        }),
        possible_duplicate_with: vec![],
    }
}

fn mark_possible_duplicates(results: &mut [DeduplicatedResult]) {
    for left_index in 0..results.len() {
        for right_index in left_index + 1..results.len() {
            if title_similarity(&results[left_index], &results[right_index])
                .is_some_and(|v| v >= 0.9)
            {
                let left_url = results[left_index].canonical_url.clone();
                let right_url = results[right_index].canonical_url.clone();
                results[left_index].possible_duplicate_with.push(right_url);
                results[right_index].possible_duplicate_with.push(left_url);
                if results[left_index].duplicate_status == DuplicateStatus::Distinct {
                    results[left_index].duplicate_status = DuplicateStatus::PossibleDuplicate;
                }
                if results[right_index].duplicate_status == DuplicateStatus::Distinct {
                    results[right_index].duplicate_status = DuplicateStatus::PossibleDuplicate;
                }
            }
        }
    }
}

fn title_similarity(left: &DeduplicatedResult, right: &DeduplicatedResult) -> Option<f64> {
    if left.canonical_url.0.host_str() == right.canonical_url.0.host_str() {
        return None;
    }
    let left_title = left.title.as_deref()?;
    let right_title = right.title.as_deref()?;
    if left_title.chars().count() < 20 || right_title.chars().count() < 20 {
        return None;
    }
    let left_tokens = title_tokens(left_title);
    let right_tokens = title_tokens(right_title);
    if left_tokens.len() < 4 || right_tokens.len() < 4 {
        return None;
    }
    let intersection = left_tokens.intersection(&right_tokens).count();
    let union = left_tokens.union(&right_tokens).count();
    Some(intersection as f64 / union as f64)
}

/// Tokenize a title after NFKC + case folding + confusable folding, so that
/// multilingual titles differing only in script compare equal.
fn title_tokens(title: &str) -> BTreeSet<String> {
    normalized_confusable_text(title)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CanonicalTransformation, CanonicalizationStatus, FieldProvenance, OriginalUrl, ResultType,
    };

    fn item(provider: &str, url: &str, title: &str) -> CanonicalResult {
        let url = url::Url::parse(url).unwrap();
        CanonicalResult {
            schema_version: SCHEMA_VERSION.into(),
            title: Some(title.into()),
            original_url: OriginalUrl(url.clone()),
            canonical_url: CanonicalUrl(url),
            provider: provider.into(),
            provider_rank: None,
            snippet: Some(format!("snippet-{provider}")),
            result_type: ResultType::Organic,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
            provenance: BTreeMap::from([("url".into(), FieldProvenance::Reported)]),
            transformations: Vec::<CanonicalTransformation>::new(),
            canonicalization_status: CanonicalizationStatus::Complete,
            degradations: vec![],
        }
    }

    #[test]
    fn confirmed_merge_preserves_provenance_and_alternates() {
        let results = deduplicate(vec![
            item("b", "https://example.com/", "A useful title"),
            item("a", "https://example.com/", "A useful title"),
        ]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].providers, ["a", "b"]);
        assert_eq!(results[0].original_urls.len(), 1);
        assert_eq!(results[0].alternate_snippets.len(), 1);
        assert_eq!(
            results[0].duplicate_status,
            DuplicateStatus::ConfirmedDuplicate
        );
        assert_eq!(results[0].merge_reason, Some(MergeReason::OriginalUrlExact));
    }

    #[test]
    fn long_similar_titles_only_mark_possible_duplicate() {
        let title = "Complete deterministic guide to asynchronous Rust programming";
        let results = deduplicate(vec![
            item("a", "https://one.example/a", title),
            item("b", "https://two.example/b", title),
        ]);
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|result| result.duplicate_status == DuplicateStatus::PossibleDuplicate));
        assert!(results.iter().all(|result| result.merge_reason.is_none()));
    }

    #[test]
    fn short_titles_are_not_compared() {
        let results = deduplicate(vec![
            item("a", "https://one.example/a", "one two three"),
            item("b", "https://two.example/b", "one two three"),
        ]);
        assert!(results
            .iter()
            .all(|result| result.duplicate_status == DuplicateStatus::Distinct));
    }

    #[test]
    fn confusable_titles_are_marked_possible_duplicates() {
        // The two titles differ only in script: Cyrillic `а`/`е`/`о`/`р`/`с`
        // vs Latin `a`/`e`/`o`/`p`/`c`. After confusable folding they compare
        // equal, so they are flagged as possible duplicates across hosts.
        let cyrillic = "Полное руководство по асинхронному программированию на Rust";
        let latin = "Полное руководство по асинхронному программированию на Rust"
            .replace('о', "o")
            .replace('а', "a")
            .replace('е', "e")
            .replace('р', "p")
            .replace('с', "c");
        assert_ne!(cyrillic, latin);
        let results = deduplicate(vec![
            item("a", "https://one.example/a", cyrillic),
            item("b", "https://two.example/b", &latin),
        ]);
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|result| result.duplicate_status == DuplicateStatus::PossibleDuplicate));
        assert_eq!(results[0].possible_duplicate_with.len(), 1);
        assert_eq!(results[1].possible_duplicate_with.len(), 1);
    }

    #[test]
    fn greek_and_latin_confusable_titles_are_marked_possible_duplicates() {
        // Greek omicron `ο` vs Latin `o`, and Greek `ρ` vs Latin `p`.
        let greek = "Οδηγός για ασύγχρονο προγραμματισμό σε Rust";
        let latin = "Οδηγός για ασύγχρονο προγραμματισμό σε Rust"
            .replace('ο', "o")
            .replace('ρ', "p");
        assert_ne!(greek, latin, "the two titles must differ byte-wise");
        let results = deduplicate(vec![
            item("a", "https://one.example/a", greek),
            item("b", "https://two.example/b", &latin),
        ]);
        assert!(results
            .iter()
            .all(|result| result.duplicate_status == DuplicateStatus::PossibleDuplicate));
    }

    #[test]
    fn distinct_greek_titles_are_not_confused_with_each_other() {
        // Guards against folding Greek phonetically instead of visually: θ, φ,
        // γ, ψ, π and λ were all collapsed onto Latin letters they do not
        // resemble, which made unrelated Greek titles compare equal.
        let first = "Θεωρία γραφημάτων και πολυπλοκότητα αλγορίθμων";
        let second = "Φεωρία δραφημάτων και ξολυξλοκότητα αλψορίθμων";
        assert_ne!(first, second);
        let results = deduplicate(vec![
            item("a", "https://one.example/a", first),
            item("b", "https://two.example/b", second),
        ]);
        assert!(
            results
                .iter()
                .all(|result| result.duplicate_status == DuplicateStatus::Distinct),
            "distinct Greek titles must not be flagged: {:?}",
            results
                .iter()
                .map(|result| &result.duplicate_status)
                .collect::<Vec<_>>()
        );
    }
}
