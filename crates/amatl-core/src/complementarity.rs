//! STEP 2 — COMPLEMENTARITY CONTRACT.
//!
//! One canonical function, [`compute_complementarity_metrics`], turns a
//! post-dedupe result set plus the router's [`RoleAssignment`] into
//! [`ComplementarityMetrics`]. It answers, objectively and structurally:
//!
//! * What did PRIMARY find? What did EXPANSION find?
//! * Which results appeared in both (confirmed by dedupe)?
//! * Which were exclusive to PRIMARY? To EXPANSION?
//! * Which hosts did EXPANSION add that PRIMARY never carried?
//! * How much structural overlap was there?
//!
//! It does NOT — and must not — say whether any of those results is *relevant*.
//! `unique_expansion` is a count of structurally exclusive results, nothing
//! more. See [`ComplementarityMetrics`] and [`crate::RelevanceMetrics`].
//!
//! ## Where identity comes from
//!
//! The function never compares URLs itself and never re-canonicalizes. Dedupe
//! already resolved identity: a [`DeduplicatedResult`] whose `providers` list
//! contains both a PRIMARY-role and an EXPANSION-role name *is* a confirmed
//! overlap, because dedupe only merges providers onto one result on an exact
//! original- or canonical-URL match. `possible_duplicate_with` /
//! `DuplicateStatus::PossibleDuplicate` is a weaker, title-similarity link and
//! is counted separately as `overlap_possible`.
//!
//! ## Role source
//!
//! Roles come from [`RoleAssignment::role_of`]. Provider names
//! ("marginalia" / "searxng" / …) are configuration, never constants in this
//! module. Multiple EXPANSION providers are supported: any EXPANSION-role name
//! in `providers` puts the result in the EXPANSION set.

use crate::model::{ComplementarityMetrics, DeduplicatedResult, RelevanceMetrics, SCHEMA_VERSION};
use crate::router::{ProviderRole, RoleAssignment};
use std::collections::BTreeSet;

/// Compute the STEP 2 complementarity metrics over a post-dedupe result set.
///
/// Returns `None` in legacy mode (`!roles.active`): with no PRIMARY/EXPANSION
/// split there is nothing to compare, and the contract does not fabricate one.
///
/// `deduped` is the deduplicated result set for the whole search so far
/// (accumulated across rounds when used for a round trace, or the final set for
/// the response). All arithmetic is exact and total; ratios never produce NaN.
pub fn compute_complementarity_metrics(
    deduped: &[DeduplicatedResult],
    roles: &RoleAssignment,
) -> Option<ComplementarityMetrics> {
    if !roles.active {
        return None;
    }

    let has_role = |result: &DeduplicatedResult, want: ProviderRole| {
        result
            .providers
            .iter()
            .any(|name| roles.role_of(name) == Some(want))
    };

    let host_of = |result: &DeduplicatedResult| {
        result
            .canonical_url
            .0
            .host_str()
            .map(str::to_ascii_lowercase)
    };

    let mut found_primary = 0_u32;
    let mut found_expansion = 0_u32;
    let mut overlap_confirmed = 0_u32;
    let mut unique_primary = 0_u32;
    let mut unique_expansion = 0_u32;

    // Host partitions. A host is PRIMARY-exclusive only if it appears on
    // PRIMARY-exclusive results and never on any EXPANSION result, and
    // symmetrically for EXPANSION.
    let mut primary_hosts = BTreeSet::<String>::new();
    let mut expansion_hosts = BTreeSet::<String>::new();
    let mut all_hosts = BTreeSet::<String>::new();

    for result in deduped {
        let in_primary = has_role(result, ProviderRole::Primary);
        let in_expansion = has_role(result, ProviderRole::Expansion);
        let host = host_of(result);
        if let Some(host) = &host {
            all_hosts.insert(host.clone());
        }

        if in_primary {
            found_primary += 1;
            if let Some(host) = &host {
                primary_hosts.insert(host.clone());
            }
        }
        if in_expansion {
            found_expansion += 1;
            if let Some(host) = &host {
                expansion_hosts.insert(host.clone());
            }
        }
        match (in_primary, in_expansion) {
            (true, true) => overlap_confirmed += 1,
            (true, false) => unique_primary += 1,
            (false, true) => unique_expansion += 1,
            (false, false) => {}
        }
    }

    // OVERLAP_POSSIBLE: pairs of *distinct* results, one PRIMARY-exclusive and
    // one EXPANSION-exclusive, linked by the possible-duplicate relation. Count
    // each PRIMARY-exclusive result that has at least one EXPANSION-exclusive
    // possible-duplicate partner. Deliberately not folded into
    // `overlap_confirmed`.
    let expansion_only_urls = deduped
        .iter()
        .filter(|r| !has_role(r, ProviderRole::Primary) && has_role(r, ProviderRole::Expansion))
        .map(|r| r.canonical_url.clone())
        .collect::<BTreeSet<_>>();
    let overlap_possible = deduped
        .iter()
        .filter(|r| has_role(r, ProviderRole::Primary) && !has_role(r, ProviderRole::Expansion))
        .filter(|r| {
            r.possible_duplicate_with
                .iter()
                .any(|url| expansion_only_urls.contains(url))
        })
        .count() as u32;

    let primary_unique_domains = primary_hosts.difference(&expansion_hosts).count() as u32;
    let expansion_unique_domains = expansion_hosts.difference(&primary_hosts).count() as u32;
    let expansion_new_domains = expansion_hosts.difference(&primary_hosts).count() as u32;
    let final_unique_domains = all_hosts.len() as u32;

    // Ratios: `None` when EXPANSION never ran (no denominator), `0.0` when it
    // ran but found nothing. `max(1, FOUND_EXPANSION)` guards the division; the
    // `None` case is decided by role membership, not by the count.
    let expansion_ran = roles
        .expansion_providers
        .iter()
        .any(|name| deduped.iter().any(|r| r.providers.contains(name)))
        || found_expansion > 0;
    let (expansion_overlap_ratio, expansion_unique_ratio) = if expansion_ran {
        let denominator = found_expansion.max(1) as f64;
        (
            Some(overlap_confirmed as f64 / denominator),
            Some(unique_expansion as f64 / denominator),
        )
    } else {
        (None, None)
    };

    Some(ComplementarityMetrics {
        schema_version: SCHEMA_VERSION.into(),
        found_primary,
        found_expansion,
        overlap_confirmed,
        overlap_possible,
        unique_primary,
        unique_expansion,
        primary_unique_domains,
        expansion_unique_domains,
        final_unique_domains,
        expansion_new_domains,
        expansion_overlap_ratio,
        expansion_unique_ratio,
        relevance: RelevanceMetrics::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CanonicalUrl, DuplicateStatus, OriginalUrl, ResultType, SCHEMA_VERSION};
    use std::collections::BTreeMap;

    fn deduped(url: &str, providers: &[&str]) -> DeduplicatedResult {
        let parsed = url::Url::parse(url).unwrap();
        DeduplicatedResult {
            schema_version: SCHEMA_VERSION.into(),
            title: Some("a result title long enough".into()),
            original_url: OriginalUrl(parsed.clone()),
            canonical_url: CanonicalUrl(parsed.clone()),
            original_urls: vec![OriginalUrl(parsed)],
            providers: providers.iter().map(|p| p.to_string()).collect(),
            representative_provider: providers.first().copied().unwrap_or("").into(),
            provider_ranks: BTreeMap::new(),
            snippet: None,
            alternate_snippets: vec![],
            result_type: ResultType::Organic,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
            observed_dates: vec![],
            duplicate_status: if providers.len() > 1 {
                DuplicateStatus::ConfirmedDuplicate
            } else {
                DuplicateStatus::Distinct
            },
            merge_reason: None,
            possible_duplicate_with: vec![],
        }
    }

    fn roles() -> RoleAssignment {
        RoleAssignment::primary_expansion("marginalia", ["searxng".to_string()])
    }

    #[test]
    fn legacy_mode_returns_none() {
        let set = [deduped("https://a.example/1", &["marginalia"])];
        assert!(compute_complementarity_metrics(&set, &RoleAssignment::legacy()).is_none());
    }

    #[test]
    fn primary_only() {
        let set = [
            deduped("https://a.example/1", &["marginalia"]),
            deduped("https://b.example/2", &["marginalia"]),
        ];
        let m = compute_complementarity_metrics(&set, &roles()).unwrap();
        assert_eq!(m.found_primary, 2);
        assert_eq!(m.found_expansion, 0);
        assert_eq!(m.overlap_confirmed, 0);
        assert_eq!(m.unique_primary, 2);
        assert_eq!(m.unique_expansion, 0);
        assert_eq!(m.expansion_overlap_ratio, None);
        assert_eq!(m.expansion_unique_ratio, None);
    }

    #[test]
    fn full_overlap() {
        let set = [
            deduped("https://a.example/1", &["marginalia", "searxng"]),
            deduped("https://b.example/2", &["marginalia", "searxng"]),
        ];
        let m = compute_complementarity_metrics(&set, &roles()).unwrap();
        assert_eq!(m.found_primary, 2);
        assert_eq!(m.found_expansion, 2);
        assert_eq!(m.overlap_confirmed, 2);
        assert_eq!(m.unique_primary, 0);
        assert_eq!(m.unique_expansion, 0);
        assert_eq!(m.expansion_overlap_ratio, Some(1.0));
        assert_eq!(m.expansion_unique_ratio, Some(0.0));
    }

    #[test]
    fn zero_expansion_results_do_not_produce_nan() {
        let set = [deduped("https://a.example/1", &["marginalia"])];
        let m = compute_complementarity_metrics(&set, &roles()).unwrap();
        // EXPANSION configured but never contributed a result: ratios are None,
        // never NaN.
        assert_eq!(m.expansion_overlap_ratio, None);
        assert!(m
            .expansion_overlap_ratio
            .map(f64::is_nan)
            .unwrap_or(false)
            .eq(&false));
    }
}
