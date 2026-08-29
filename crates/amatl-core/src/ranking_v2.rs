use crate::model::{
    DeepRankedDocument, DeepRankingExplanation, Document, Evidence, Query, Rank, RankingScore,
    RankingV2Output, RankingV2Status, SCHEMA_VERSION,
};
use crate::text::normalized_text;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use thiserror::Error;

pub const BENCHMARK_ID: &str = "ranking-v2-human-labeled-v2";
const BENCHMARK_CORPUS: &str = include_str!("../benchmarks/ranking_v2_corpus.json");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RankingV2Policy {
    pub version: String,
    pub bm25_k1: f64,
    pub bm25_b: f64,
    pub weight_bm25: f64,
    pub weight_semantic: f64,
    pub weight_reranker: f64,
    pub weight_relevance: f64,
    pub weight_evidence: f64,
    pub benchmark_minimum_ndcg_delta: f64,
    pub benchmark_minimum_ndcg: f64,
}

impl Default for RankingV2Policy {
    fn default() -> Self {
        Self {
            version: "v2".into(),
            bm25_k1: 1.2,
            bm25_b: 0.75,
            weight_bm25: 1.0,
            weight_semantic: 0.0,
            weight_reranker: 0.0,
            weight_relevance: 0.85,
            weight_evidence: 0.15,
            benchmark_minimum_ndcg_delta: 0.05,
            benchmark_minimum_ndcg: 0.90,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum RankingV2Error {
    #[error("ranking v2 policy violates version, range or weight invariants")]
    InvalidPolicy,
    #[error("optional ranking backend failed")]
    Backend,
}

impl RankingV2Policy {
    pub fn validate(&self) -> Result<(), RankingV2Error> {
        let component_weights = [self.weight_bm25, self.weight_semantic, self.weight_reranker];
        let final_weights = [self.weight_relevance, self.weight_evidence];
        let values = [
            self.bm25_k1,
            self.bm25_b,
            self.weight_bm25,
            self.weight_semantic,
            self.weight_reranker,
            self.weight_relevance,
            self.weight_evidence,
            self.benchmark_minimum_ndcg_delta,
            self.benchmark_minimum_ndcg,
        ];
        if self.version != "v2"
            || values.iter().any(|value| !value.is_finite())
            || self.bm25_k1 <= 0.0
            || !(0.0..=1.0).contains(&self.bm25_b)
            || component_weights
                .iter()
                .chain(final_weights.iter())
                .any(|value| !(0.0..=1.0).contains(value))
            || (component_weights.iter().sum::<f64>() - 1.0).abs() > 1e-12
            || (final_weights.iter().sum::<f64>() - 1.0).abs() > 1e-12
            || !(0.0..=1.0).contains(&self.benchmark_minimum_ndcg_delta)
            || !(0.0..=1.0).contains(&self.benchmark_minimum_ndcg)
        {
            return Err(RankingV2Error::InvalidPolicy);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RankingBenchmarkReport {
    pub schema_version: String,
    pub benchmark_id: String,
    pub policy_version: String,
    pub query_count: u32,
    pub baseline_ndcg_at_3: f64,
    pub candidate_ndcg_at_3: f64,
    pub ndcg_delta: f64,
    pub baseline_mrr: f64,
    pub candidate_mrr: f64,
    pub passed: bool,
}

/// Optional semantic relevance backend.
///
/// The contract is async because a backend may be remote under
/// `data_policy.inference = "remote_explicit"`; the shipped local backend
/// resolves immediately without touching the network.
#[async_trait::async_trait]
pub trait SemanticScorer: Send + Sync {
    fn name(&self) -> &str;
    async fn score(
        &self,
        query: &Query,
        documents: &[Document],
    ) -> Result<Vec<f64>, RankingV2Error>;
}

#[async_trait::async_trait]
pub trait DeepReranker: Send + Sync {
    fn name(&self) -> &str;
    async fn score(
        &self,
        query: &Query,
        documents: &[Document],
        relevance: &[f64],
    ) -> Result<Vec<f64>, RankingV2Error>;
}

pub struct RankingV2Engine {
    policy: RankingV2Policy,
    benchmark: RankingBenchmarkReport,
    semantic: Option<Arc<dyn SemanticScorer>>,
    reranker: Option<Arc<dyn DeepReranker>>,
}

impl RankingV2Engine {
    pub fn new(policy: RankingV2Policy) -> Result<Self, RankingV2Error> {
        policy.validate()?;
        let benchmark = run_builtin_benchmark(&policy);
        Ok(Self {
            policy,
            benchmark,
            semantic: None,
            reranker: None,
        })
    }

    pub fn with_semantic_scorer(mut self, scorer: Arc<dyn SemanticScorer>) -> Self {
        self.semantic = Some(scorer);
        self
    }

    pub fn with_reranker(mut self, reranker: Arc<dyn DeepReranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    pub fn benchmark(&self) -> &RankingBenchmarkReport {
        &self.benchmark
    }

    pub async fn rank(
        &self,
        query: &Query,
        documents: &[Document],
        evidence: &[Evidence],
        original_ranks: &BTreeMap<String, Rank>,
    ) -> Result<RankingV2Output, RankingV2Error> {
        if !self.benchmark.passed {
            return Ok(output(
                &self.policy,
                RankingV2Status::BenchmarkRejected,
                vec![],
            ));
        }
        if documents.is_empty() {
            return Ok(output(
                &self.policy,
                RankingV2Status::InsufficientDocuments,
                vec![],
            ));
        }
        let bm25 = bm25_scores(query, documents, &self.policy);
        let semantic = if self.policy.weight_semantic > 0.0 {
            optional_scores(self.semantic.as_deref(), query, documents).await?
        } else {
            None
        };
        let mut relevance = combine_relevance(&bm25, semantic.as_deref(), None, &self.policy);
        let reranker = match (self.policy.weight_reranker > 0.0, self.reranker.as_ref()) {
            (true, Some(backend)) => Some(validate_backend_scores(
                backend.score(query, documents, &relevance).await?,
                documents.len(),
            )?),
            _ => None,
        };
        relevance = combine_relevance(
            &bm25,
            semantic.as_deref(),
            reranker.as_deref(),
            &self.policy,
        );
        let evidence_by_id = evidence
            .iter()
            .map(|value| (value.document_id.as_str(), value.evidence_score.get()))
            .collect::<BTreeMap<_, _>>();
        let mut ranked = documents
            .iter()
            .enumerate()
            .map(|(index, document)| {
                let evidence_score = evidence_by_id
                    .get(document.search_result_id.as_str())
                    .copied()
                    .unwrap_or(0.0);
                let final_score = (self.policy.weight_relevance * relevance[index]
                    + self.policy.weight_evidence * evidence_score)
                    .clamp(0.0, 1.0);
                let original_rank = original_ranks
                    .get(&document.search_result_id)
                    .copied()
                    .unwrap_or(Rank::MAX);
                DeepRankedDocument {
                    document_id: document.search_result_id.clone(),
                    rank: original_rank,
                    original_rank,
                    relevance_score: RankingScore::bounded(relevance[index]),
                    evidence_score: RankingScore::bounded(evidence_score),
                    final_score: RankingScore::bounded(final_score),
                    explanation: DeepRankingExplanation {
                        ranking_policy: self.policy.version.clone(),
                        bm25: RankingScore::bounded(bm25[index]),
                        semantic: semantic
                            .as_ref()
                            .map(|scores| RankingScore::bounded(scores[index])),
                        reranker: reranker
                            .as_ref()
                            .map(|scores| RankingScore::bounded(scores[index])),
                        relevance_score: RankingScore::bounded(relevance[index]),
                        evidence_score: RankingScore::bounded(evidence_score),
                        final_score: RankingScore::bounded(final_score),
                    },
                }
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .final_score
                .get()
                .total_cmp(&left.final_score.get())
                .then_with(|| {
                    right
                        .relevance_score
                        .get()
                        .total_cmp(&left.relevance_score.get())
                })
                .then_with(|| left.original_rank.cmp(&right.original_rank))
                .then_with(|| left.document_id.cmp(&right.document_id))
        });
        for (index, value) in ranked.iter_mut().enumerate() {
            value.rank = Rank::new(index as u32 + 1).unwrap_or(Rank::MAX);
        }
        Ok(output(&self.policy, RankingV2Status::Applied, ranked))
    }
}

async fn optional_scores(
    backend: Option<&dyn SemanticScorer>,
    query: &Query,
    documents: &[Document],
) -> Result<Option<Vec<f64>>, RankingV2Error> {
    let Some(backend) = backend else {
        return Ok(None);
    };
    let scores = backend.score(query, documents).await?;
    validate_backend_scores(scores, documents.len()).map(Some)
}

fn validate_backend_scores(scores: Vec<f64>, expected: usize) -> Result<Vec<f64>, RankingV2Error> {
    if scores.len() != expected || scores.iter().any(|value| !value.is_finite()) {
        return Err(RankingV2Error::Backend);
    }
    Ok(scores
        .into_iter()
        .map(|value| value.clamp(0.0, 1.0))
        .collect())
}

fn combine_relevance(
    bm25: &[f64],
    semantic: Option<&[f64]>,
    reranker: Option<&[f64]>,
    policy: &RankingV2Policy,
) -> Vec<f64> {
    let semantic_weight = if semantic.is_some() {
        policy.weight_semantic
    } else {
        0.0
    };
    let reranker_weight = if reranker.is_some() {
        policy.weight_reranker
    } else {
        0.0
    };
    let total = policy.weight_bm25 + semantic_weight + reranker_weight;
    bm25.iter()
        .enumerate()
        .map(|(index, bm25)| {
            ((policy.weight_bm25 * bm25
                + semantic_weight * semantic.map_or(0.0, |values| values[index])
                + reranker_weight * reranker.map_or(0.0, |values| values[index]))
                / total)
                .clamp(0.0, 1.0)
        })
        .collect()
}

fn output(
    policy: &RankingV2Policy,
    status: RankingV2Status,
    results: Vec<DeepRankedDocument>,
) -> RankingV2Output {
    RankingV2Output {
        schema_version: SCHEMA_VERSION.into(),
        policy_version: policy.version.clone(),
        benchmark_id: BENCHMARK_ID.into(),
        status,
        results,
    }
}

pub fn disabled_output() -> RankingV2Output {
    output(
        &RankingV2Policy::default(),
        RankingV2Status::Disabled,
        vec![],
    )
}

pub fn rejected_output() -> RankingV2Output {
    output(
        &RankingV2Policy::default(),
        RankingV2Status::BenchmarkRejected,
        vec![],
    )
}

fn bm25_scores(query: &Query, documents: &[Document], policy: &RankingV2Policy) -> Vec<f64> {
    let query_terms = terms(&query.normalized_query)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if query_terms.is_empty() || documents.is_empty() {
        return vec![0.0; documents.len()];
    }
    let fields = documents.iter().map(document_terms).collect::<Vec<_>>();
    let average_length =
        fields.iter().map(Vec::len).sum::<usize>().max(1) as f64 / fields.len() as f64;
    let mut raw = vec![0.0; documents.len()];
    for term in query_terms {
        let document_frequency = fields.iter().filter(|field| field.contains(&term)).count() as f64;
        let idf = (1.0
            + (documents.len() as f64 - document_frequency + 0.5) / (document_frequency + 0.5))
            .ln();
        for (index, field) in fields.iter().enumerate() {
            let frequency = field.iter().filter(|value| **value == term).count() as f64;
            let length_ratio = field.len() as f64 / average_length;
            let denominator =
                frequency + policy.bm25_k1 * (1.0 - policy.bm25_b + policy.bm25_b * length_ratio);
            if denominator > 0.0 {
                raw[index] += idf * frequency * (policy.bm25_k1 + 1.0) / denominator;
            }
        }
    }
    let maximum = raw.iter().copied().fold(0.0_f64, f64::max);
    if maximum == 0.0 {
        raw
    } else {
        raw.into_iter().map(|value| value / maximum).collect()
    }
}

fn document_terms(document: &Document) -> Vec<String> {
    let mut output = terms(document.title.as_deref().unwrap_or_default());
    output.extend(terms(document.content.as_deref().unwrap_or_default()));
    output
}

fn terms(value: &str) -> Vec<String> {
    normalized_text(value)
        .split_whitespace()
        .map(|token| token.trim_matches(|character: char| !character.is_alphanumeric()))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Clone, Deserialize)]
struct BenchmarkCase {
    query: String,
    documents: Vec<BenchmarkDocument>,
}

#[derive(Clone, Deserialize)]
struct BenchmarkDocument {
    url: String,
    title: String,
    snippet: String,
    content: String,
    relevance: u32,
    provider_rank: u32,
}

pub fn run_builtin_benchmark(policy: &RankingV2Policy) -> RankingBenchmarkReport {
    let Ok(cases) = serde_json::from_str::<Vec<BenchmarkCase>>(BENCHMARK_CORPUS) else {
        return rejected_benchmark_report(policy);
    };
    let mut baseline_ndcg = 0.0;
    let mut candidate_ndcg = 0.0;
    let mut baseline_mrr = 0.0;
    let mut candidate_mrr = 0.0;
    for case in &cases {
        let Ok(query) = crate::query::parse_query(case.query.clone()) else {
            return rejected_benchmark_report(policy);
        };
        let Some(documents) = corpus_documents(&case.documents) else {
            return rejected_benchmark_report(policy);
        };
        let scores = bm25_scores(&query, &documents, policy);
        let evidence = crate::evidence::analyze_evidence(&documents);
        let final_scores = scores
            .iter()
            .zip(evidence.iter())
            .map(|(relevance, evidence)| {
                policy.weight_relevance * relevance
                    + policy.weight_evidence * evidence.evidence_score.get()
            })
            .collect::<Vec<_>>();
        let mut order = (0..documents.len()).collect::<Vec<_>>();
        order.sort_by(|left, right| {
            final_scores[*right]
                .total_cmp(&final_scores[*left])
                .then_with(|| left.cmp(right))
        });
        let baseline_order = baseline_order(&query, &case.documents);
        let relevance = case
            .documents
            .iter()
            .map(|document| document.relevance)
            .collect::<Vec<_>>();
        let before = ndcg(&baseline_order, &relevance, 3);
        let after = ndcg(&order, &relevance, 3);
        baseline_ndcg += before;
        candidate_ndcg += after;
        baseline_mrr += reciprocal_rank(&baseline_order, &relevance);
        candidate_mrr += reciprocal_rank(&order, &relevance);
    }
    let count = cases.len() as f64;
    baseline_ndcg /= count;
    candidate_ndcg /= count;
    baseline_mrr /= count;
    candidate_mrr /= count;
    let delta = candidate_ndcg - baseline_ndcg;
    RankingBenchmarkReport {
        schema_version: SCHEMA_VERSION.into(),
        benchmark_id: BENCHMARK_ID.into(),
        policy_version: policy.version.clone(),
        query_count: cases.len() as u32,
        baseline_ndcg_at_3: baseline_ndcg,
        candidate_ndcg_at_3: candidate_ndcg,
        ndcg_delta: delta,
        baseline_mrr,
        candidate_mrr,
        passed: candidate_ndcg >= policy.benchmark_minimum_ndcg
            && delta >= policy.benchmark_minimum_ndcg_delta
            && candidate_mrr >= baseline_mrr,
    }
}

fn rejected_benchmark_report(policy: &RankingV2Policy) -> RankingBenchmarkReport {
    RankingBenchmarkReport {
        schema_version: SCHEMA_VERSION.into(),
        benchmark_id: BENCHMARK_ID.into(),
        policy_version: policy.version.clone(),
        query_count: 0,
        baseline_ndcg_at_3: 0.0,
        candidate_ndcg_at_3: 0.0,
        ndcg_delta: 0.0,
        baseline_mrr: 0.0,
        candidate_mrr: 0.0,
        passed: false,
    }
}

fn corpus_documents(values: &[BenchmarkDocument]) -> Option<Vec<Document>> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let url = url::Url::parse(&value.url).ok()?;
            Some(Document {
                schema_version: SCHEMA_VERSION.into(),
                search_result_id: index.to_string(),
                original_url: crate::OriginalUrl(url.clone()),
                canonical_url: crate::CanonicalUrl(url.clone()),
                final_url: crate::FinalUrl(url),
                content_hash: index.to_string(),
                fetch_method: crate::FetchMethod::Http,
                extractor_used: Some("recorded-human-labeled-corpus".into()),
                content_type: Some("text/plain".into()),
                size: value.content.len() as u64,
                retrieved_at: "2026-08-12T00:00:00Z".into(),
                status: crate::DocumentStatus::Enriched,
                content: Some(value.content.clone()),
                title: Some(value.title.clone()),
                author: None,
                published_at: None,
                metadata: BTreeMap::new(),
            })
        })
        .collect()
}

fn baseline_order(query: &Query, values: &[BenchmarkDocument]) -> Vec<usize> {
    let results = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let url = url::Url::parse(&value.url).ok()?;
            Some(crate::DeduplicatedResult {
                schema_version: SCHEMA_VERSION.into(),
                title: Some(value.title.clone()),
                original_url: crate::OriginalUrl(url.clone()),
                canonical_url: crate::CanonicalUrl(url),
                original_urls: vec![],
                providers: vec!["recorded".into()],
                representative_provider: "recorded".into(),
                provider_ranks: BTreeMap::from([(
                    "recorded".into(),
                    crate::Rank::new(value.provider_rank).ok(),
                )]),
                snippet: Some(value.snippet.clone()),
                alternate_snippets: vec![],
                result_type: crate::ResultType::Document,
                published_at: None,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: BTreeMap::from([("benchmark_index".into(), index.to_string())]),
                observed_dates: vec![],
                duplicate_status: crate::DuplicateStatus::Distinct,
                merge_reason: None,
                possible_duplicate_with: vec![],
            })
        })
        .collect();
    crate::ranking::rank(
        query,
        "2026-08-12T00:00:00Z",
        1,
        results,
        &crate::RankingPolicyV1::default(),
    )
    .iter()
    .filter_map(|ranked| {
        ranked
            .result
            .metadata
            .get("benchmark_index")
            .and_then(|value| value.parse().ok())
    })
    .collect()
}

#[cfg(test)]
fn benchmark_documents(contents: [&str; 3]) -> Vec<Document> {
    contents
        .into_iter()
        .enumerate()
        .map(|(index, content)| {
            let url = url::Url::parse(&format!("https://benchmark.invalid/{index}")).unwrap();
            Document {
                schema_version: SCHEMA_VERSION.into(),
                search_result_id: index.to_string(),
                original_url: crate::OriginalUrl(url.clone()),
                canonical_url: crate::CanonicalUrl(url.clone()),
                final_url: crate::FinalUrl(url),
                content_hash: index.to_string(),
                fetch_method: crate::FetchMethod::Http,
                extractor_used: Some("benchmark".into()),
                content_type: Some("text/plain".into()),
                size: content.len() as u64,
                retrieved_at: "2026-08-12T00:00:00Z".into(),
                status: crate::DocumentStatus::Enriched,
                content: Some(content.into()),
                title: None,
                author: None,
                published_at: None,
                metadata: BTreeMap::new(),
            }
        })
        .collect()
}

fn ndcg(order: &[usize], relevance: &[u32], k: usize) -> f64 {
    let dcg = order
        .iter()
        .take(k)
        .enumerate()
        .map(|(index, item)| {
            (2_f64.powi(relevance[*item] as i32) - 1.0) / (index as f64 + 2.0).log2()
        })
        .sum::<f64>();
    let mut ideal = relevance.to_vec();
    ideal.sort_by(|left, right| right.cmp(left));
    let idcg = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(index, value)| (2_f64.powi(*value as i32) - 1.0) / (index as f64 + 2.0).log2())
        .sum::<f64>();
    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

fn reciprocal_rank(order: &[usize], relevance: &[u32]) -> f64 {
    order
        .iter()
        .position(|item| relevance[*item] > 0)
        .map_or(0.0, |index| 1.0 / (index as f64 + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_policy_passes_reproducible_quality_gate() {
        let report = run_builtin_benchmark(&RankingV2Policy::default());
        assert!(report.passed, "{report:?}");
        assert!(report.candidate_ndcg_at_3 > report.baseline_ndcg_at_3);
        assert_eq!(report, run_builtin_benchmark(&RankingV2Policy::default()));
    }

    #[test]
    fn policy_values_are_calibrable_inside_version_v2() {
        let changed = RankingV2Policy {
            bm25_k1: 2.0,
            ..RankingV2Policy::default()
        };
        assert_eq!(changed.validate(), Ok(()));
    }

    #[test]
    fn bm25_prefers_document_matching_all_query_terms() {
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = benchmark_documents(["rust", "rust async runtime", "cooking"]);
        let scores = bm25_scores(&query, &documents, &RankingV2Policy::default());
        assert!(scores[1] > scores[0] && scores[0] > scores[2]);
    }

    #[test]
    fn ndcg_is_one_for_ideal_order() {
        assert!((ndcg(&[1, 2, 0], &[0, 3, 1], 3) - 1.0).abs() < 1e-12);
    }

    /// Recorded calibration of the shipped policy against the shipped corpus.
    ///
    /// These are not thresholds to pass; they are the values this policy
    /// currently produces. Drifting away from them means the calibration
    /// changed, which is a review decision, not an accident: update these
    /// constants in the same change that moves the ranking.
    const RECORDED_CANDIDATE_NDCG_AT_3: f64 = 0.9193779960897104;
    const RECORDED_BASELINE_NDCG_AT_3: f64 = 0.6557679882437799;
    const RECORDED_CANDIDATE_MRR: f64 = 0.9;
    const CALIBRATION_TOLERANCE: f64 = 1e-6;

    #[test]
    fn builtin_benchmark_holds_its_recorded_calibration() {
        let report = run_builtin_benchmark(&RankingV2Policy::default());
        assert!(report.passed, "{report:?}");
        assert_eq!(report.benchmark_id, BENCHMARK_ID);
        assert!(report.query_count >= 5, "{report:?}");
        for (name, actual, recorded) in [
            (
                "candidate_ndcg_at_3",
                report.candidate_ndcg_at_3,
                RECORDED_CANDIDATE_NDCG_AT_3,
            ),
            (
                "baseline_ndcg_at_3",
                report.baseline_ndcg_at_3,
                RECORDED_BASELINE_NDCG_AT_3,
            ),
            (
                "candidate_mrr",
                report.candidate_mrr,
                RECORDED_CANDIDATE_MRR,
            ),
        ] {
            assert!(
                (actual - recorded).abs() <= CALIBRATION_TOLERANCE,
                "{name} drifted: {actual} vs recorded {recorded}. If the change is \
                 intended, update the recorded calibration in the same commit."
            );
        }
    }

    #[tokio::test]
    async fn ranking_keeps_relevance_evidence_and_final_scores_separate() {
        let policy = RankingV2Policy::default();
        let engine = RankingV2Engine::new(policy.clone()).unwrap();
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = benchmark_documents(["rust", "rust async runtime", "cooking"]);
        let evidence = crate::evidence::analyze_evidence(&documents);
        let original = BTreeMap::from([
            ("0".into(), Rank::new(1).unwrap()),
            ("1".into(), Rank::new(2).unwrap()),
            ("2".into(), Rank::new(3).unwrap()),
        ]);
        let output = engine
            .rank(&query, &documents, &evidence, &original)
            .await
            .unwrap();
        assert_eq!(output.status, RankingV2Status::Applied);
        assert_eq!(output.results[0].document_id, "1");
        for result in output.results {
            let expected = policy.weight_relevance * result.relevance_score.get()
                + policy.weight_evidence * result.evidence_score.get();
            assert!((result.final_score.get() - expected).abs() < 1e-12);
            assert_eq!(result.explanation.relevance_score, result.relevance_score);
            assert_eq!(result.explanation.evidence_score, result.evidence_score);
        }
    }

    #[tokio::test]
    async fn failed_benchmark_gate_returns_no_ranking() {
        let mut engine = RankingV2Engine::new(RankingV2Policy::default()).unwrap();
        engine.benchmark.passed = false;
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = benchmark_documents(["rust", "rust async runtime", "cooking"]);
        let output = engine
            .rank(
                &query,
                &documents,
                &crate::evidence::analyze_evidence(&documents),
                &BTreeMap::new(),
            )
            .await
            .unwrap();
        assert_eq!(output.status, RankingV2Status::BenchmarkRejected);
        assert!(output.results.is_empty());
    }
}

/// Measures the Deep reranker against the labeled corpus.
///
/// The Ranking v2 gate gives the *search* pipeline; it never exercises
/// [`crate::DeepReranker`], which is why a reranker regression could land
/// without turning CI red. This module closes that hole.
#[cfg(test)]
mod reranker_measurement {
    use super::*;

    /// Rank the labeled corpus with a reranker and report mean nDCG@3.
    async fn measure(reranker: &dyn DeepReranker) -> f64 {
        let cases: Vec<BenchmarkCase> = serde_json::from_str(BENCHMARK_CORPUS).unwrap();
        let mut total = 0.0;
        for case in &cases {
            let query = crate::query::parse_query(case.query.clone()).unwrap();
            let documents = corpus_documents(&case.documents).unwrap();
            let relevance: Vec<u32> = case.documents.iter().map(|d| d.relevance).collect();
            // Uniform prior so the measurement isolates the reranker signal.
            let prior = vec![0.5_f64; documents.len()];
            let scores = reranker.score(&query, &documents, &prior).await.unwrap();
            let mut order: Vec<usize> = (0..documents.len()).collect();
            order.sort_by(|a, b| {
                scores[*b]
                    .partial_cmp(&scores[*a])
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(b))
            });
            total += ndcg(&order, &relevance, 3);
        }
        total / cases.len() as f64
    }

    /// Records the evidence behind `InferenceRuntime::reranker` defaulting to
    /// lexical coverage: with the default feature-hashing backend, embedding
    /// similarity ranks the labeled corpus strictly worse.
    #[tokio::test]
    async fn hashing_embeddings_rerank_worse_than_lexical_coverage() {
        use crate::inference::{EmbeddingReranker, LexicalCoverageReranker, LocalHashingEmbedder};
        use std::sync::Arc;

        let lexical = LexicalCoverageReranker::new(64, 0.5).unwrap();
        let lexical_score = measure(&lexical).await;

        let backend = Arc::new(LocalHashingEmbedder::new(256, 4096).unwrap());
        let embedding = EmbeddingReranker::new(backend, 64, 0.5).unwrap();
        let embedding_score = measure(&embedding).await;

        assert!(
            lexical_score > embedding_score,
            "expected lexical coverage to beat hashed embeddings on the labeled corpus \
             (lexical={lexical_score:.6}, embedding={embedding_score:.6}); if this flips, \
             revisit the default chosen in InferenceRuntime::reranker"
        );
    }

    /// The default runtime must therefore report the lexical reranker.
    #[tokio::test]
    async fn default_runtime_reranker_is_lexical() {
        use crate::config::{DataPolicyConfig, InferenceConfig, InferenceMode};

        let policy = DataPolicyConfig {
            inference: InferenceMode::LocalOnly,
            ..DataPolicyConfig::default()
        };
        let runtime = crate::inference::InferenceRuntime::from_policy(
            &policy,
            &InferenceConfig::default(),
            None,
        )
        .unwrap()
        .expect("local_only yields a runtime");
        assert_eq!(
            runtime.reranker().unwrap().name(),
            crate::inference::LOCAL_RERANKER_ID
        );
    }
}
