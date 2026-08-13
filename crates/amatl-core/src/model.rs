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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
