//! Core shared by every AMATL surface. Keep product logic out of CLI/UI/API/MCP.

pub mod audit;
pub mod budget;
pub mod cache;
pub mod canonical;
pub mod circuit;
pub mod classify;
pub mod config;
pub mod dedupe;
pub mod deep;
pub mod diversity;
pub mod document_cache;
pub mod errors;
pub mod evidence;
pub mod execution;
pub mod extract;
pub mod fetch;
pub mod gaps;
pub mod inference;
pub mod ingest;
pub mod model;
pub mod normalize;
pub mod operational;
pub mod planning;
pub mod progressive;
pub mod providers;
pub mod query;
pub mod ranking;
pub mod ranking_v2;
pub mod render;
pub mod robots;
pub mod router;
pub mod security;
pub mod service;
pub mod storage;
pub mod telemetry;
mod text;

pub use audit::{
    SecurityAudit, SecurityEventInput, AUDIT_DEFAULT_RETENTION_DAYS, AUDIT_MAX_RETENTION_DAYS,
};
pub use budget::{Budget, BudgetExhaustionCause, BudgetSnapshot, DeepBudget, DeepBudgetSnapshot};
pub use cache::{
    CacheCounters, CacheEffectiveness, CachedProvider, ProviderSearchCache,
    ProviderSearchCachePolicy,
};
pub use circuit::{CircuitPolicy, CircuitSnapshot, CircuitState, ProviderCircuit};
pub use classify::classify;
pub use config::{
    ApprovalStatus, Config, ConfigError, DataPolicyConfig, EgressPolicy, ExecutionConfig,
    InferenceConfig, InferenceMode, ProviderConfig, ProviderRuntimeConfig, Scope, SecurityProfile,
    ServerClient, ServerConfig, TlsConfig, MCP_TOOLS,
};
pub use deep::{DeepCandidate, DeepOrchestrator, DeepRequest};
pub use diversity::{DiversityDecision, DiversityMetrics, DiversityOutput, DiversityPolicyV1};
pub use document_cache::{DocumentCache, DocumentCachePolicy};
pub use errors::{ErrorCode, ERROR_CATALOG};
pub use evidence::{
    analyze_evidence, analyze_evidence_bundle, analyze_evidence_v2, EVIDENCE_V2_FRAGMENT_BYTES,
    EVIDENCE_V2_MAX_FRAGMENTS, EVIDENCE_V2_VERSION,
};
pub use execution::{ParallelSearchOutput, SearchOrchestrator};
pub use extract::{
    ExtractError, ExtractionResult, Extractor, TrafilaturaExtractor, UnavailableExtractor,
};
pub use fetch::{
    DnsResolver, FetchError, FetchRequest, FetchResult, Fetcher, SafeFetcher, SystemDnsResolver,
};
pub use gaps::{
    GapAnalysis, GapAnalyzer, GapPolicyError, GapPolicyV1, SearchSubQueryExecutor,
    SubQueryExecutionError, SubQueryExecutor,
};
pub use inference::{
    validate_remote_endpoint, EmbeddingBackend, EmbeddingSemanticScorer, InferenceError,
    InferenceRuntime, LexicalCoverageReranker, LocalHashingEmbedder, RemoteEmbeddingBackend,
    LOCAL_EMBEDDING_BACKEND_ID, LOCAL_RERANKER_ID, REMOTE_EMBEDDING_BACKEND_ID,
};
pub use ingest::{
    LocalDocumentType, LocalIngestError, LocalIngestResponse, LocalIngestor,
    LOCAL_INGEST_MAX_INPUT_BYTES, LOCAL_INGEST_MAX_OUTPUT_BYTES, LOCAL_INGEST_PDF_TIMEOUT_MS,
};
pub use model::{
    CanonicalResult, CanonicalTransformation, CanonicalUrl, CanonicalizationStatus, Category,
    Classification, CompositeError, DeduplicatedResult, DeepRankedDocument, DeepRankingExplanation,
    DeepResponse, Degradation, Document, DocumentStatus, DuplicateStatus, Evidence,
    EvidenceFragment, EvidenceProvenance, EvidenceScoreBasis, EvidenceSignal, EvidenceStatus,
    EvidenceV2, FetchMethod, FieldProvenance, FinalUrl, Gap, GapSeverity, GapStatus, GapType,
    GlobalBudgetSnapshot, MergeReason, NormalizedResult, OriginalUrl, ProviderCapabilities,
    ProviderError, ProviderErrorKind, ProviderExecutionStatus, ProviderItem, ProviderResult, Query,
    QueryWarning, Rank, RankedResult, RankingExplanation, RankingScore, RankingV2Output,
    RankingV2Status, ResultStatus, ResultType, SearchPlan, SearchResponse, SearchResult,
    SearchStatus, SubQuery, SubQueryStatus, TieBreakReason, ValueInvariantError, SCHEMA_VERSION,
};
pub use operational::{
    run_operational_benchmark, LatencyPercentiles, OperationalBenchmarkError,
    OperationalBenchmarkReport, SearchOperationalReport, SqliteOperationalReport,
};
pub use progressive::{
    CoverageMetrics, ProgressiveRoundTrace, SearchPolicyError, SearchPolicyV1, SearchStopReason,
};
pub use providers::{
    BraveProvider, DuckDuckGoHtmlProvider, HttpRequest, HttpResponse, HttpTransport, MockBehavior,
    MockProvider, MojeekProvider, Provider, ProviderAvailability, ProviderBuildContext,
    ProviderContext, ProviderFactory, ProviderRegistry, ReqwestTransport,
};
pub use query::{parse_query, QueryParseError};
pub use ranking::{RankingPolicyError, RankingPolicyV1};
pub use ranking_v2::{
    run_builtin_benchmark, DeepReranker, RankingBenchmarkReport, RankingV2Engine, RankingV2Error,
    RankingV2Policy, SemanticScorer, BENCHMARK_ID,
};
pub use render::{ChromiumRenderer, RenderError, RenderResult, Renderer, RendererPool};
pub use robots::{
    RobotsCache, RobotsDecision, RobotsRules, MAXIMUM_CRAWL_DELAY_MS, ROBOTS_USER_AGENT,
};
pub use router::{AdaptiveRouter, AdaptiveRoutingRecommendation, ProviderDescriptor, StaticRouter};
pub use service::{
    validate_provider_canary, validate_provider_canary_with, AmatlService, CacheStatus,
    ExecutionLimits, ProviderCanaryError, ProviderSummary, ProviderSurfaceStatus,
    SaveDocumentInput, SearchExecution, ServiceError, ServiceStatus, ServiceSurface,
    ServiceSurfaceKind, SourceStatus, StorageStatus,
};
pub use storage::{
    CacheStats, CachedDocument, SavedDocument, SearchHistoryEntry, SecurityEvent, SqliteStorage,
    StorageError, StorageHealth, StoredCircuitRecord, MIGRATION_VERSION,
};
pub use telemetry::{
    InMemoryTelemetry, ProviderHealth, ProviderValueSnapshot, ProviderValueState,
    TelemetryObservation, TelemetryOutcome, TelemetryStatus, TELEMETRY_DEFAULT_RETENTION_DAYS,
    TELEMETRY_MAX_RETENTION_DAYS, TELEMETRY_MIN_RETENTION_DAYS,
};
