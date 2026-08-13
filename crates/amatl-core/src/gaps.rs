use crate::diversity::DiversityPolicyV1;
use crate::model::{
    Category, Evidence, Gap, GapSeverity, GapStatus, GapType, Query, SearchPlan, SearchResponse,
    SCHEMA_VERSION,
};
use crate::progressive::SearchPolicyV1;
use crate::providers::Provider;
use crate::ranking::RankingPolicyV1;
use crate::text::tokens;
use crate::{Budget, Document, SearchOrchestrator};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GapPolicyV1 {
    pub version: String,
    pub minimum_documents: u32,
    pub minimum_unique_domains: u32,
    pub minimum_enriched_ratio: f64,
    pub minimum_average_evidence: f64,
    pub difficult_confidence_max: f64,
    pub difficult_minimum_terms: u32,
    pub max_subqueries: u32,
}

impl Default for GapPolicyV1 {
    fn default() -> Self {
        Self {
            version: "v1".into(),
            minimum_documents: 3,
            minimum_unique_domains: 3,
            minimum_enriched_ratio: 0.60,
            minimum_average_evidence: 0.45,
            difficult_confidence_max: 0.75,
            difficult_minimum_terms: 4,
            max_subqueries: 2,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum GapPolicyError {
    #[error("gap policy violates version, range or limit invariants")]
    InvalidPolicy,
}

impl GapPolicyV1 {
    pub fn validate(&self) -> Result<(), GapPolicyError> {
        let ratios = [
            self.minimum_enriched_ratio,
            self.minimum_average_evidence,
            self.difficult_confidence_max,
        ];
        if self.version != "v1"
            || self.minimum_documents == 0
            || self.minimum_unique_domains == 0
            || self.difficult_minimum_terms == 0
            || self.max_subqueries == 0
            || ratios
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(GapPolicyError::InvalidPolicy);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GapAnalysis {
    pub gaps: Vec<Gap>,
    pub coverage_low: bool,
    pub difficult_query: bool,
}

#[derive(Clone, Debug)]
pub struct GapAnalyzer {
    policy: GapPolicyV1,
}

impl GapAnalyzer {
    pub fn new(policy: GapPolicyV1) -> Result<Self, GapPolicyError> {
        policy.validate()?;
        Ok(Self { policy })
    }

    pub fn analyze(
        &self,
        query: &Query,
        plan: &SearchPlan,
        documents: &[Document],
        evidence: &[Evidence],
    ) -> GapAnalysis {
        let unique_domains = documents
            .iter()
            .filter_map(|document| document.canonical_url.0.host_str())
            .collect::<BTreeSet<_>>()
            .len() as u32;
        let enriched = documents
            .iter()
            .filter(|document| document.content.is_some())
            .count() as f64;
        let enriched_ratio = if documents.is_empty() {
            0.0
        } else {
            enriched / documents.len() as f64
        };
        let average_evidence = if evidence.is_empty() {
            0.0
        } else {
            evidence
                .iter()
                .map(|value| value.evidence_score.get())
                .sum::<f64>()
                / evidence.len() as f64
        };
        let coverage_low = documents.len() < self.policy.minimum_documents as usize
            || unique_domains < self.policy.minimum_unique_domains
            || enriched_ratio < self.policy.minimum_enriched_ratio
            || average_evidence < self.policy.minimum_average_evidence;
        let query_terms = tokens(&query.normalized_query);
        let explicit_intent = !query.file_types.is_empty()
            || query.date_from.is_some()
            || query.date_to.is_some()
            || query.region.is_some()
            || query_terms.iter().any(|term| {
                matches!(
                    term.as_str(),
                    "official"
                        | "primary"
                        | "original"
                        | "documentation"
                        | "docs"
                        | "rfc"
                        | "spec"
                        | "specification"
                        | "standard"
                        | "code"
                        | "repository"
                        | "github"
                        | "gitlab"
                )
            });
        let difficult_query = plan.classification.confidence
            <= self.policy.difficult_confidence_max
            || query_terms.len() >= self.policy.difficult_minimum_terms as usize
            || explicit_intent;
        let mut gaps = Vec::new();

        if explicit_primary_source(&query_terms) && !has_primary_source(documents) {
            gaps.push(gap(
                GapType::PrimarySource,
                GapSeverity::High,
                "The query requests a primary or official source, but none is observable",
                query,
                "official primary source",
                2,
            ));
        }
        if requires_recency(query, plan) && !evidence.iter().any(|value| value.verified_date) {
            gaps.push(gap(
                GapType::Recency,
                GapSeverity::High,
                "No document has a verified publication date for a recency-sensitive query",
                query,
                "recent verified update",
                2,
            ));
        }
        if let Some(region) = &query.region {
            let covered = documents.iter().any(|document| {
                document
                    .metadata
                    .get("region")
                    .is_some_and(|value| value.eq_ignore_ascii_case(region))
            });
            if !covered {
                gaps.push(gap(
                    GapType::GeographicDiversity,
                    GapSeverity::Medium,
                    "The requested region is not represented in document metadata",
                    query,
                    &format!("regional source {region}"),
                    1,
                ));
            }
        }
        if (plan.classification.primary_category == Category::Documentation
            || requires_documentation(&query_terms))
            && !has_documentation(documents)
        {
            gaps.push(gap(
                GapType::Documentation,
                GapSeverity::High,
                "A documentation query has no observable documentation source",
                query,
                "official documentation",
                2,
            ));
        }
        if query
            .file_types
            .iter()
            .any(|value| value.eq_ignore_ascii_case("pdf"))
            && !has_pdf(documents)
        {
            gaps.push(gap(
                GapType::Pdf,
                GapSeverity::High,
                "The query requests PDF content, but no PDF document was retrieved",
                query,
                "filetype:pdf",
                2,
            ));
        }
        if (plan.classification.primary_category == Category::Code || requires_code(&query_terms))
            && !has_code(documents)
        {
            gaps.push(gap(
                GapType::Code,
                GapSeverity::High,
                "A code query has no observable source-code document",
                query,
                "source code repository",
                2,
            ));
        }
        if requires_specification(&query_terms) && !has_specification(documents) {
            gaps.push(gap(
                GapType::Specification,
                GapSeverity::High,
                "The query requests a specification, but no specification is observable",
                query,
                "official specification standard",
                2,
            ));
        }
        if unique_domains < self.policy.minimum_unique_domains {
            let deficit = self
                .policy
                .minimum_unique_domains
                .saturating_sub(unique_domains)
                .max(1);
            gaps.push(gap(
                GapType::SourceDiversity,
                GapSeverity::Medium,
                "The Deep result set has insufficient unique source domains",
                query,
                "independent source",
                deficit.min(2),
            ));
        }
        GapAnalysis {
            gaps,
            coverage_low,
            difficult_query,
        }
    }

    pub fn proposals(&self, analysis: &GapAnalysis) -> Vec<Gap> {
        if !analysis.coverage_low || !analysis.difficult_query {
            return vec![];
        }
        let mut proposals = analysis
            .gaps
            .iter()
            .filter(|gap| gap.recommended_query.is_some())
            .cloned()
            .collect::<Vec<_>>();
        proposals.sort_by_key(|gap| severity_order(&gap.severity));
        proposals.truncate(self.policy.max_subqueries as usize);
        proposals
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SubQueryExecutionError {
    #[error("subquery execution failed")]
    Failed,
}

#[async_trait]
pub trait SubQueryExecutor: Send + Sync {
    async fn execute(&self, query: Query) -> Result<SearchResponse, SubQueryExecutionError>;
}

pub struct SearchSubQueryExecutor {
    providers: Vec<Arc<dyn Provider>>,
    max_provider_calls: u32,
    provider_timeout_ms: u64,
    global_timeout_ms: u64,
    ranking: RankingPolicyV1,
    diversity: DiversityPolicyV1,
    search: SearchPolicyV1,
}

impl SearchSubQueryExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        providers: Vec<Arc<dyn Provider>>,
        max_provider_calls: u32,
        provider_timeout_ms: u64,
        global_timeout_ms: u64,
        ranking: RankingPolicyV1,
        diversity: DiversityPolicyV1,
        search: SearchPolicyV1,
    ) -> Self {
        Self {
            providers,
            max_provider_calls,
            provider_timeout_ms,
            global_timeout_ms,
            ranking,
            diversity,
            search,
        }
    }
}

#[async_trait]
impl SubQueryExecutor for SearchSubQueryExecutor {
    async fn execute(&self, query: Query) -> Result<SearchResponse, SubQueryExecutionError> {
        let mut orchestrator = SearchOrchestrator::new(
            Budget::new(self.max_provider_calls, self.global_timeout_ms),
            self.provider_timeout_ms,
        )
        .with_result_policies(self.ranking.clone(), self.diversity.clone())
        .with_search_policy(self.search.clone());
        Ok(orchestrator.search(query, self.providers.clone()).await)
    }
}

fn gap(
    kind: GapType,
    severity: GapSeverity,
    reason: &str,
    query: &Query,
    suffix: &str,
    expected_gain: u32,
) -> Gap {
    let suffix = suffix.to_lowercase();
    let recommended = if query.normalized_query.contains(&suffix)
        || query.raw_query.to_lowercase().contains(&suffix)
    {
        query.raw_query.clone()
    } else {
        format!("{} {}", query.raw_query.trim(), suffix)
    };
    Gap {
        schema_version: SCHEMA_VERSION.into(),
        gap_type: kind,
        severity,
        reason: reason.into(),
        recommended_query: Some(recommended),
        estimated_cost: Some(1),
        expected_gain: Some(expected_gain),
        status: GapStatus::Detected,
    }
}

fn explicit_primary_source(terms: &BTreeSet<String>) -> bool {
    terms
        .iter()
        .any(|term| matches!(term.as_str(), "official" | "primary" | "original"))
}
fn requires_recency(query: &Query, plan: &SearchPlan) -> bool {
    query.date_from.is_some()
        || query.date_to.is_some()
        || plan.classification.primary_category == Category::News
}
fn requires_specification(terms: &BTreeSet<String>) -> bool {
    terms
        .iter()
        .any(|term| matches!(term.as_str(), "rfc" | "spec" | "specification" | "standard"))
}
fn requires_documentation(terms: &BTreeSet<String>) -> bool {
    terms
        .iter()
        .any(|term| matches!(term.as_str(), "documentation" | "docs"))
}
fn requires_code(terms: &BTreeSet<String>) -> bool {
    terms
        .iter()
        .any(|term| matches!(term.as_str(), "code" | "repository" | "github" | "gitlab"))
}
fn severity_order(severity: &GapSeverity) -> u8 {
    match severity {
        GapSeverity::High => 0,
        GapSeverity::Medium => 1,
        GapSeverity::Low => 2,
    }
}
fn document_text(document: &Document) -> String {
    format!(
        "{} {}",
        document.title.as_deref().unwrap_or_default(),
        document.content.as_deref().unwrap_or_default()
    )
    .to_lowercase()
}
fn has_primary_source(documents: &[Document]) -> bool {
    documents.iter().any(|document| {
        document_text(document).contains("official")
            || document
                .final_url
                .host_str()
                .is_some_and(|host| host.ends_with(".gov") || host.ends_with(".edu"))
    })
}
fn has_documentation(documents: &[Document]) -> bool {
    documents.iter().any(|document| {
        document.final_url.path().contains("docs")
            || document.final_url.path().contains("documentation")
            || document_text(document).contains("documentation")
    })
}
fn has_pdf(documents: &[Document]) -> bool {
    documents.iter().any(|document| {
        document.final_url.path().to_lowercase().ends_with(".pdf")
            || document
                .content_type
                .as_deref()
                .is_some_and(|value| value.to_lowercase().contains("application/pdf"))
    })
}
fn has_code(documents: &[Document]) -> bool {
    documents.iter().any(|document| {
        document
            .final_url
            .host_str()
            .is_some_and(|host| matches!(host, "github.com" | "gitlab.com" | "codeberg.org"))
            || ["```", "fn ", "def ", "class "].iter().any(|marker| {
                document
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains(marker))
            })
    })
}
fn has_specification(documents: &[Document]) -> bool {
    documents.iter().any(|document| {
        let text = document_text(document);
        document.final_url.path().to_lowercase().contains("rfc")
            || ["specification", "standard", " rfc "]
                .iter()
                .any(|marker| text.contains(marker))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::build_search_plan;
    use crate::router::RoutingRecommendation;
    use crate::{classify, parse_query};
    use std::collections::BTreeMap;

    fn plan(query: &Query) -> SearchPlan {
        build_search_plan(
            query.clone(),
            classify(query),
            RoutingRecommendation {
                selected_providers: vec![],
                provider_budget_requests: BTreeMap::new(),
                debug_reasons: vec![],
            },
            &mut Budget::new(1, 1000),
        )
    }

    #[test]
    fn detects_only_observable_requested_gaps() {
        let query = parse_query("official RFC filetype:pdf".into()).unwrap();
        let analysis = GapAnalyzer::new(GapPolicyV1::default()).unwrap().analyze(
            &query,
            &plan(&query),
            &[],
            &[],
        );
        let kinds = analysis
            .gaps
            .iter()
            .map(|gap| &gap.gap_type)
            .collect::<BTreeSet<_>>();
        assert!(kinds.contains(&GapType::PrimarySource));
        assert!(kinds.contains(&GapType::Pdf));
        assert!(kinds.contains(&GapType::Specification));
        assert!(kinds.contains(&GapType::SourceDiversity));
        assert!(!kinds.contains(&GapType::Recency));
    }

    #[test]
    fn third_proposal_is_rejected_by_hard_limit() {
        let query = parse_query("official RFC filetype:pdf".into()).unwrap();
        let analyzer = GapAnalyzer::new(GapPolicyV1::default()).unwrap();
        let analysis = analyzer.analyze(&query, &plan(&query), &[], &[]);
        assert!(analysis.gaps.len() >= 3);
        assert_eq!(analyzer.proposals(&analysis).len(), 2);
    }

    #[test]
    fn adequate_easy_coverage_does_not_expand() {
        let query = parse_query("rust".into()).unwrap();
        let analyzer = GapAnalyzer::new(GapPolicyV1::default()).unwrap();
        let mut analysis = analyzer.analyze(&query, &plan(&query), &[], &[]);
        analysis.coverage_low = false;
        assert!(analyzer.proposals(&analysis).is_empty());
    }
}
