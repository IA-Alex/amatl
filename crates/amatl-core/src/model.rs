use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;
use url::Url;

pub const SCHEMA_VERSION: &str = "1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct OriginalUrl(pub Url);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct CanonicalUrl(pub Url);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct FinalUrl(pub Url);

impl Deref for FinalUrl {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for FinalUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct Rank(u32);

impl Rank {
    pub const FIRST: Self = Self(1);
    pub const MAX: Self = Self(u32::MAX);

    pub fn new(value: u32) -> Result<Self, ValueInvariantError> {
        if value == 0 {
            Err(ValueInvariantError::Rank)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Rank {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u32::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, PartialOrd)]
#[serde(transparent)]
pub struct RankingScore(f64);

impl RankingScore {
    pub fn new(value: f64) -> Result<Self, ValueInvariantError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ValueInvariantError::RankingScore)
        }
    }

    pub const fn get(self) -> f64 {
        self.0
    }

    pub(crate) fn bounded(value: f64) -> Self {
        Self(if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        })
    }
}

impl<'de> Deserialize<'de> for RankingScore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(f64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueInvariantError {
    Rank,
    RankingScore,
}

impl fmt::Display for ValueInvariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rank => formatter.write_str("rank must be greater than or equal to one"),
            Self::RankingScore => {
                formatter.write_str("ranking score must be finite and between zero and one")
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchStatus {
    Success,
    PartialSuccess,
    Failure,
}

impl SearchStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Visible,
    RelegatedByDiversity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FetchMethod {
    Http,
    Rendered,
    Local,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    Enriched,
    Superficial,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    General,
    Technical,
    Code,
    Documentation,
    News,
    Academic,
    Commercial,
    Forum,
    Social,
    Media,
    Navigation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ResultType {
    Organic,
    News,
    Media,
    Document,
    Code,
    Forum,
    Social,
    Commercial,
    Navigation,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderExecutionStatus {
    Success,
    Partial,
    Failure,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    Timeout,
    RateLimit,
    Auth,
    Network,
    InvalidResponse,
    ParserError,
    Quota,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldProvenance {
    Reported,
    Derived,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalizationStatus {
    Complete,
    Degraded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "field", rename_all = "snake_case")]
pub enum CanonicalTransformation {
    LowercaseScheme,
    LowercaseHost,
    IdnToPunycode,
    RemoveDefaultPort,
    AddRootPath,
    NormalizePercentHex,
    RemoveTrackingParameter(String),
    RemoveEmptyFragment,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateStatus {
    ConfirmedDuplicate,
    PossibleDuplicate,
    Distinct,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergeReason {
    OriginalUrlExact,
    CanonicalUrlExact,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TieBreakReason {
    CombinedScore,
    TitleMatch,
    StableOrder,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryWarning {
    pub code: String,
    pub operator: Option<String>,
    pub value: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Degradation {
    pub code: String,
    pub component: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Query {
    pub schema_version: String,
    pub raw_query: String,
    pub normalized_query: String,
    pub quoted_terms: Vec<String>,
    pub excluded_terms: Vec<String>,
    pub domains: Vec<String>,
    pub excluded_domains: Vec<String>,
    pub file_types: Vec<String>,
    pub language: Option<String>,
    pub region: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub warnings: Vec<QueryWarning>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Classification {
    pub schema_version: String,
    pub primary_category: Category,
    pub secondary_categories: Vec<Category>,
    pub confidence: f64,
    pub confidence_by_category: BTreeMap<Category, f64>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub schema_version: String,
    pub pagination: bool,
    pub language: bool,
    pub region: bool,
    pub time_range: bool,
    pub site_filter: bool,
    pub file_filter: bool,
    pub news: bool,
    pub code: bool,
    pub docs: bool,
    pub academic: bool,
    pub authentication: bool,
    pub estimated_cost: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderError {
    pub schema_version: String,
    pub provider: String,
    pub kind: ProviderErrorKind,
    pub message: String,
    pub retry_after_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositeError {
    pub code: String,
    pub message: String,
    pub providers: Vec<String>,
    pub recoverable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchPlan {
    pub schema_version: String,
    pub query: Query,
    pub classification: Classification,
    pub selected_providers: Vec<String>,
    pub provider_priority: Vec<String>,
    pub provider_budget_requests: BTreeMap<String, u32>,
    pub provider_budgets: BTreeMap<String, u32>,
    pub global_budget: GlobalBudgetSnapshot,
    pub ranking_reference_time: String,
    pub fallback_policy: String,
    pub expansion_policy: String,
    pub stop_conditions: Vec<String>,
    pub debug_reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalBudgetSnapshot {
    pub max_provider_calls: u32,
    pub remaining_provider_calls: u32,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderItem {
    pub title: Option<String>,
    pub url: String,
    pub provider_rank: Option<Rank>,
    pub snippet: Option<String>,
    pub result_type: Option<ResultType>,
    pub published_at: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub file_type: Option<String>,
    pub thumbnail: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderResult {
    pub schema_version: String,
    pub provider: String,
    pub status: ProviderExecutionStatus,
    pub results: Vec<ProviderItem>,
    pub accepted_filters: Vec<String>,
    pub ignored_filters: Vec<String>,
    pub approximated_filters: Vec<String>,
    pub errors: Vec<ProviderError>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedResult {
    pub schema_version: String,
    pub title: Option<String>,
    pub raw_url: String,
    pub url: OriginalUrl,
    pub provider: String,
    pub provider_rank: Option<Rank>,
    pub snippet: Option<String>,
    pub result_type: ResultType,
    pub published_at: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub file_type: Option<String>,
    pub thumbnail: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub provenance: BTreeMap<String, FieldProvenance>,
    pub degradations: Vec<Degradation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalResult {
    pub schema_version: String,
    pub title: Option<String>,
    pub original_url: OriginalUrl,
    pub canonical_url: CanonicalUrl,
    pub provider: String,
    pub provider_rank: Option<Rank>,
    pub snippet: Option<String>,
    pub result_type: ResultType,
    pub published_at: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub file_type: Option<String>,
    pub thumbnail: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub provenance: BTreeMap<String, FieldProvenance>,
    pub transformations: Vec<CanonicalTransformation>,
    pub canonicalization_status: CanonicalizationStatus,
    pub degradations: Vec<Degradation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeduplicatedResult {
    pub schema_version: String,
    pub title: Option<String>,
    pub original_url: OriginalUrl,
    pub canonical_url: CanonicalUrl,
    pub original_urls: Vec<OriginalUrl>,
    pub providers: Vec<String>,
    pub representative_provider: String,
    pub provider_ranks: BTreeMap<String, Option<Rank>>,
    pub snippet: Option<String>,
    pub alternate_snippets: Vec<String>,
    pub result_type: ResultType,
    pub published_at: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub file_type: Option<String>,
    pub thumbnail: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub observed_dates: Vec<String>,
    pub duplicate_status: DuplicateStatus,
    pub merge_reason: Option<MergeReason>,
    pub possible_duplicate_with: Vec<CanonicalUrl>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RankingExplanation {
    pub ranking_policy: String,
    pub rrf: RankingScore,
    pub title_match: RankingScore,
    pub snippet_match: RankingScore,
    pub freshness: RankingScore,
    pub provider_agreement: RankingScore,
    pub combined_score: RankingScore,
    pub tie_break: TieBreakReason,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RankedResult {
    pub result: DeduplicatedResult,
    pub score: RankingScore,
    pub title_match: RankingScore,
    pub stable_order: usize,
    pub explanation: RankingExplanation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchResult {
    pub schema_version: String,
    pub rank: Rank,
    pub title: Option<String>,
    pub original_url: OriginalUrl,
    pub canonical_url: CanonicalUrl,
    pub domain: String,
    pub snippet: Option<String>,
    pub providers: Vec<String>,
    pub published_at: Option<String>,
    pub status: ResultStatus,
}

/// STEP 2 — COMPLEMENTARITY CONTRACT.
///
/// Objective, structural measurement of what the PRIMARY role found, what the
/// EXPANSION role found, what overlapped, and what was exclusive to each. It is
/// derived entirely from post-dedupe provenance (`DeduplicatedResult.providers`
/// / `duplicate_status`) plus the router's [`crate::RoleAssignment`]; provider
/// names are never hardcoded here.
///
/// # These numbers are NOT a relevance signal
///
/// A result counted in `unique_expansion` is only VALID + CANONICALIZABLE +
/// structurally UNIQUE. The Wiby evidence showed that a valid canonical URL on a
/// unique domain that is exclusive to one provider still need not be useful.
/// `UNIQUE_EXPANSION != UNIQUE_RELEVANT_EXPANSION`. Relevance lives in the
/// separate, deliberately unpopulated [`RelevanceMetrics`] layer. Nothing in
/// this struct may feed adaptive routing in STEP 2.
///
/// # PROVENANCE vs COMPLEMENTARITY vs RELEVANCE
///
/// * PROVENANCE — which role produced a result (STEP 1, already in the trace).
/// * COMPLEMENTARITY — this struct: overlap / exclusivity arithmetic.
/// * RELEVANCE — [`RelevanceMetrics`], not implemented.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ComplementarityMetrics {
    pub schema_version: String,

    /// `|PRIMARY_RESULT_SET|` — post-dedupe results where a PRIMARY-role
    /// provider appears in `providers`.
    pub found_primary: u32,
    /// `|EXPANSION_RESULT_SET|` — post-dedupe results where an EXPANSION-role
    /// provider appears in `providers`.
    pub found_expansion: u32,

    /// Results where a PRIMARY-role provider AND an EXPANSION-role provider both
    /// appear in `providers` — i.e. the shared identity was established by the
    /// confirmed dedupe / canonicalization mechanism (exact original or
    /// canonical URL match). This is the only overlap the contract treats as
    /// certain.
    pub overlap_confirmed: u32,
    /// Distinct results (one PRIMARY-only, one EXPANSION-only) linked by
    /// `possible_duplicate_with` under the current `DuplicateStatus` semantics
    /// (title-similarity across hosts). NOT added to `overlap_confirmed`: a
    /// possible duplicate is explicitly not a confirmed one. Kept separate so a
    /// consumer never conflates the two.
    pub overlap_possible: u32,

    /// PRIMARY results not in the confirmed overlap with EXPANSION.
    /// `found_primary == overlap_confirmed + unique_primary`.
    pub unique_primary: u32,
    /// EXPANSION results not in the confirmed overlap with PRIMARY.
    /// `found_expansion == overlap_confirmed + unique_expansion`.
    pub unique_expansion: u32,

    /// Count of distinct hosts that appear only on PRIMARY-exclusive results.
    pub primary_unique_domains: u32,
    /// Count of distinct hosts that appear only on EXPANSION-exclusive results.
    pub expansion_unique_domains: u32,
    /// Count of distinct hosts across the whole post-dedupe result set.
    pub final_unique_domains: u32,
    /// `hosts(EXPANSION) \ hosts(PRIMARY)` — hosts EXPANSION contributed that no
    /// PRIMARY result carried. Raw coverage gain, NOT "useful domain gain".
    pub expansion_new_domains: u32,

    /// `OVERLAP_CONFIRMED / max(1, FOUND_EXPANSION)`. `None` when no EXPANSION
    /// provider ran this search (no denominator to speak of); `0.0` when
    /// EXPANSION ran but returned nothing. Never NaN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion_overlap_ratio: Option<f64>,
    /// `UNIQUE_EXPANSION / max(1, FOUND_EXPANSION)`. `None` / `0.0` semantics as
    /// above. This is a structural exclusivity ratio, not a noise or utility
    /// rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion_unique_ratio: Option<f64>,

    /// The relevance layer. Always `not_implemented` in STEP 2; present so the
    /// shape is stable and consumers can see, in the payload itself, that no
    /// relevance judgement has been made.
    pub relevance: RelevanceMetrics,
}

/// STEP 2E — RELEVANCE FRONTIER (first deterministic signal).
///
/// This layer answers a question the complementarity contract deliberately
/// does not: is a structurally valid / unique result *probably relevant to the
/// query*? The first implementation is intentionally small — a deterministic,
/// local, explainable lexical-overlap heuristic (see [`crate::relevance`]). It
/// never calls an LLM, an embedding service, or any remote API, and it is
/// computed *after* dedupe + roles but *independently of the final ranking*, so
/// there is no RELEVANCE → RANKING → RELEVANCE circuit.
///
/// `status` is `NotImplemented` until [`crate::relevance::assess_relevance`]
/// populates this struct, then `Assessed`.
///
/// It must remain impossible to reach `unique_relevant_expansion` by reading
/// `ComplementarityMetrics::unique_expansion`: they are different fields, in
/// different structs. `unique_expansion` (the raw structural count) is
/// preserved untouched; `unique_relevant_expansion` is a strictly smaller-or-
/// equal subset.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RelevanceMetrics {
    pub status: RelevanceAssessmentStatus,

    /// PRIMARY-role results with assessment `Relevant`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant_primary: Option<u32>,
    /// EXPANSION-role results with assessment `Relevant`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant_expansion: Option<u32>,

    /// PRIMARY-exclusive (no confirmed overlap) results with assessment
    /// `Relevant`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_relevant_primary: Option<u32>,
    /// EXPANSION-exclusive (no confirmed overlap) results with assessment
    /// `Relevant`. Strictly a subset of `ComplementarityMetrics::unique_expansion`;
    /// never substitute one for the other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_relevant_expansion: Option<u32>,

    /// PRIMARY / EXPANSION results assessed `PossiblyRelevant`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possibly_relevant_primary: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possibly_relevant_expansion: Option<u32>,

    /// PRIMARY / EXPANSION results assessed `NotRelevant`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_relevant_primary: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_relevant_expansion: Option<u32>,

    /// PRIMARY / EXPANSION results assessed `Unknown` or `InsufficientEvidence`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_primary: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_expansion: Option<u32>,

    /// `relevant_expansion / max(1, FOUND_EXPANSION)`. Structural denominator,
    /// reported for observation only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion_relevant_ratio: Option<f64>,

    /// `unique_relevant_expansion / max(1, UNIQUE_EXPANSION_RAW)` — how much of
    /// the raw unique-expansion gain survives the relevance filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_to_relevant_expansion_ratio: Option<f64>,

    /// EXPANSION-exclusive confirmed noise rate:
    /// `not_relevant / max(1, relevant + not_relevant)` over EXPANSION-exclusive
    /// results. `PossiblyRelevant` / `Unknown` / `InsufficientEvidence` are
    /// deliberately outside the denominator — an inconclusive result is not
    /// noise. Never `1 - relevant`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion_confirmed_noise_rate: Option<f64>,

    /// Legacy field name kept for payload stability. Mirrors
    /// `expansion_confirmed_noise_rate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion_noise_rate: Option<f64>,
}

impl Default for RelevanceMetrics {
    fn default() -> Self {
        Self {
            status: RelevanceAssessmentStatus::NotImplemented,
            relevant_primary: None,
            relevant_expansion: None,
            unique_relevant_primary: None,
            unique_relevant_expansion: None,
            possibly_relevant_primary: None,
            possibly_relevant_expansion: None,
            not_relevant_primary: None,
            not_relevant_expansion: None,
            unknown_primary: None,
            unknown_expansion: None,
            expansion_relevant_ratio: None,
            raw_to_relevant_expansion_ratio: None,
            expansion_confirmed_noise_rate: None,
            expansion_noise_rate: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RelevanceAssessmentStatus {
    /// No relevance signal was computed for this payload.
    #[default]
    NotImplemented,
    /// The deterministic relevance heuristic ran and populated the metrics.
    Assessed,
    /// The heuristic ran but too few results carried enough text to judge for
    /// the aggregate to be meaningful. Individual counts are still present.
    InsufficientEvidence,
}

/// STEP 2E — per-result deterministic relevance assessment.
///
/// Every component that fed the decision is kept explicit and auditable; the
/// `classification` is a pure function of the other fields plus the
/// [`crate::relevance::RelevanceThresholds`] in force. No single opaque float.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResultRelevanceAssessment {
    /// Fraction of significant query terms present in the title (`0.0` when no
    /// title text is available).
    pub title_term_coverage: f64,
    /// Fraction of significant query terms present in the snippet (`0.0` when no
    /// snippet text is available).
    pub snippet_term_coverage: f64,
    /// Fraction of significant query terms present in host + path tokens.
    pub url_term_coverage: f64,
    /// A quoted phrase from the query occurred literally in title or snippet.
    pub exact_phrase_match: bool,
    /// Distinct significant query terms matched anywhere in title/snippet/url.
    pub matched_query_terms: u32,
    /// Total significant query terms considered.
    pub query_terms_total: u32,
    /// `true` when the result carried enough text (title or snippet) to judge.
    pub had_sufficient_text: bool,
    /// Secondary signal only: `Some(true)` when the provider's own rank for this
    /// result was 1. Never on its own promotes a result to `Relevant`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_rank_is_top: Option<bool>,
    pub classification: RelevanceClassification,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RelevanceClassification {
    /// Strong deterministic textual evidence the result matches the query.
    Relevant,
    /// Partial, non-trivial textual overlap.
    PossiblyRelevant,
    /// Enough text to judge, and essentially no relation to the query.
    NotRelevant,
    /// Not enough text (empty/again generic title, no snippet) to judge either
    /// way. Never treated as noise.
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchResponse {
    pub schema_version: String,
    pub query: String,
    pub status: SearchStatus,
    pub results: Vec<SearchResult>,
    pub providers_used: Vec<String>,
    pub providers_failed: Vec<String>,
    pub providers_partial: Vec<String>,
    pub errors: Vec<CompositeError>,
    pub degradations: Vec<Degradation>,
    pub elapsed_ms: u64,
    /// STEP 2 — objective provider complementarity measurement. `None` in
    /// legacy routing mode (no PRIMARY/EXPANSION roles to compare). Additive:
    /// omitted from JSON when absent, existing consumers are unaffected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complementarity: Option<ComplementarityMetrics>,
    /// Total number of results before pagination (server-side count).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_results: Option<u64>,
    /// Current page number (0-based) when pagination is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Number of results per page when pagination is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Document {
    pub schema_version: String,
    pub search_result_id: String,
    pub original_url: OriginalUrl,
    pub canonical_url: CanonicalUrl,
    pub final_url: FinalUrl,
    pub content_hash: String,
    pub fetch_method: FetchMethod,
    pub extractor_used: Option<String>,
    pub content_type: Option<String>,
    pub size: u64,
    pub retrieved_at: String,
    pub status: DocumentStatus,
    pub content: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Complete,
    Partial,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub schema_version: String,
    pub document_id: String,
    pub status: EvidenceStatus,
    pub fact_density: RankingScore,
    pub verified_date: bool,
    pub metadata_quality: RankingScore,
    pub named_entities: Vec<String>,
    pub citation_count: u32,
    pub citation_span: RankingScore,
    pub freshness: RankingScore,
    pub originality: RankingScore,
    pub evidence_score: RankingScore,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSignal {
    QueryMatch,
    Citation,
    Temporal,
    Numeric,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceProvenance {
    pub schema_version: String,
    pub provenance_id: String,
    pub document_id: String,
    pub original_url: OriginalUrl,
    pub canonical_url: CanonicalUrl,
    pub final_url: FinalUrl,
    pub source_content_hash: String,
    pub extracted_content_hash: Option<String>,
    pub fetch_method: FetchMethod,
    pub extractor_used: Option<String>,
    pub retrieved_at: String,
    pub published_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceFragment {
    pub schema_version: String,
    pub fragment_id: String,
    pub provenance_id: String,
    pub ordinal: u32,
    pub text: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub fragment_hash: String,
    pub matched_terms: Vec<String>,
    pub signals: Vec<EvidenceSignal>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EvidenceScoreBasis {
    pub schema_version: String,
    pub fact_density: RankingScore,
    pub verified_date: bool,
    pub metadata_quality: RankingScore,
    pub citation_count: u32,
    pub citation_span: RankingScore,
    pub freshness: RankingScore,
    pub originality: RankingScore,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EvidenceV2 {
    pub schema_version: String,
    pub evidence_version: String,
    pub document_id: String,
    pub status: EvidenceStatus,
    pub provenance: EvidenceProvenance,
    pub fragments: Vec<EvidenceFragment>,
    pub score_basis: EvidenceScoreBasis,
    pub evidence_score: RankingScore,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeepRankingExplanation {
    pub ranking_policy: String,
    pub bm25: RankingScore,
    pub semantic: Option<RankingScore>,
    pub reranker: Option<RankingScore>,
    pub relevance_score: RankingScore,
    pub evidence_score: RankingScore,
    pub final_score: RankingScore,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeepRankedDocument {
    pub document_id: String,
    pub rank: Rank,
    pub original_rank: Rank,
    pub relevance_score: RankingScore,
    pub evidence_score: RankingScore,
    pub final_score: RankingScore,
    pub explanation: DeepRankingExplanation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RankingV2Status {
    Applied,
    Disabled,
    BenchmarkRejected,
    InsufficientDocuments,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RankingV2Output {
    pub schema_version: String,
    pub policy_version: String,
    pub benchmark_id: String,
    pub status: RankingV2Status,
    pub results: Vec<DeepRankedDocument>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GapType {
    PrimarySource,
    Recency,
    GeographicDiversity,
    Documentation,
    Pdf,
    Code,
    Specification,
    SourceDiversity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GapSeverity {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GapStatus {
    Detected,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gap {
    pub schema_version: String,
    pub gap_type: GapType,
    pub severity: GapSeverity,
    pub reason: String,
    pub recommended_query: Option<String>,
    pub estimated_cost: Option<u64>,
    pub expected_gain: Option<u32>,
    pub status: GapStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubQueryStatus {
    Proposed,
    Executed,
    Failed,
    RejectedBudget,
    Invalid,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubQuery {
    pub schema_version: String,
    pub raw_query: String,
    pub reason: String,
    pub gap_type: GapType,
    pub estimated_cost: u64,
    pub expected_gain: u32,
    pub actual_gain: u32,
    pub status: SubQueryStatus,
    pub results: Vec<SearchResult>,
    pub errors: Vec<CompositeError>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeepResponse {
    pub schema_version: String,
    pub query: String,
    pub documents: Vec<Document>,
    pub errors: Vec<CompositeError>,
    pub degradations: Vec<Degradation>,
    pub evidence: Vec<Evidence>,
    pub evidence_v2: Vec<EvidenceV2>,
    pub ranking_v2: RankingV2Output,
    pub gaps: Vec<Gap>,
    pub subqueries: Vec<SubQuery>,
    pub elapsed_ms: u64,
}
