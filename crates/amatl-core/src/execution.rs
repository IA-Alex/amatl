use crate::budget::{Budget, BudgetSnapshot};
use crate::canonical::canonicalize;
use crate::classify::classify;
use crate::dedupe::deduplicate;
use crate::diversity::{diversify, DiversityMetrics, DiversityPolicyV1};
use crate::model::{
    Classification, CompositeError, ProviderError, ProviderErrorKind, ProviderExecutionStatus,
    ProviderResult, Query, SearchPlan, SearchResponse, SearchStatus, SCHEMA_VERSION,
};
use crate::normalize::normalize;
use crate::planning::build_search_plan;
use crate::progressive::{
    evaluate_coverage, observed_marginal_gain, CoverageMetrics, ProgressiveRoundTrace,
    SearchPolicyV1, SearchStopReason,
};
use crate::providers::ProviderAvailability;
use crate::providers::{Provider, ProviderContext};
use crate::ranking::{rank, RankingPolicyV1};
use crate::router::{
    AdaptiveRouter, AdaptiveRoutingRecommendation, ProviderDescriptor, RoutingRecommendation,
};
use crate::telemetry::{now_unix, InMemoryTelemetry, ProviderTelemetryInput, TelemetryObservation};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::Instrument;

static JITTER_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct ParallelSearchOutput {
    pub provider_results: Vec<ProviderResult>,
    pub providers_used: Vec<String>,
    pub providers_failed: Vec<String>,
    pub providers_partial: Vec<String>,
    pub errors: Vec<ProviderError>,
    pub sequential_rounds_elapsed_ms: u64,
    pub budget_remaining: BudgetSnapshot,
    pub telemetry_observations: Vec<TelemetryObservation>,
}

pub struct SearchOrchestrator {
    budget: Budget,
    provider_timeout_ms: u64,
    global_concurrency: usize,
    per_provider_concurrency: usize,
    max_retries: u32,
    retry_jitter_ms: u64,
    ranking_policy: RankingPolicyV1,
    diversity_policy: DiversityPolicyV1,
    search_policy: SearchPolicyV1,
    telemetry: InMemoryTelemetry,
    routing_trace: Vec<ProgressiveRoundTrace>,
    last_plan: Option<SearchPlan>,
    request_id: Option<String>,
}

impl SearchOrchestrator {
    pub fn new(budget: Budget, provider_timeout_ms: u64) -> Self {
        Self {
            budget,
            provider_timeout_ms,
            global_concurrency: 4,
            per_provider_concurrency: 1,
            max_retries: 1,
            retry_jitter_ms: 25,
            ranking_policy: RankingPolicyV1::default(),
            diversity_policy: DiversityPolicyV1::default(),
            search_policy: SearchPolicyV1::default(),
            telemetry: InMemoryTelemetry::new(),
            routing_trace: vec![],
            last_plan: None,
            request_id: None,
        }
    }

    pub fn with_execution_limits(
        mut self,
        global_concurrency: usize,
        per_provider_concurrency: usize,
        max_retries: u32,
        retry_jitter_ms: u64,
    ) -> Self {
        self.global_concurrency = global_concurrency.max(1);
        self.per_provider_concurrency = per_provider_concurrency.max(1);
        self.max_retries = max_retries.min(2);
        self.retry_jitter_ms = retry_jitter_ms;
        self
    }

    pub fn with_telemetry(mut self, telemetry: InMemoryTelemetry) -> Self {
        self.telemetry = telemetry;
        self
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    pub fn telemetry(&self) -> InMemoryTelemetry {
        self.telemetry.clone()
    }

    pub fn routing_trace(&self) -> &[ProgressiveRoundTrace] {
        &self.routing_trace
    }

    pub fn last_plan(&self) -> Option<&SearchPlan> {
        self.last_plan.as_ref()
    }

    pub fn with_search_policy(mut self, policy: SearchPolicyV1) -> Self {
        if policy.validate().is_ok() {
            self.search_policy = policy;
        }
        self
    }

    pub fn with_result_policies(
        mut self,
        ranking_policy: RankingPolicyV1,
        diversity_policy: DiversityPolicyV1,
    ) -> Self {
        if ranking_policy.validate().is_ok() && diversity_policy.validate().is_ok() {
            self.ranking_policy = ranking_policy;
            self.diversity_policy = diversity_policy;
        }
        self
    }

    pub fn plan(
        &mut self,
        query: Query,
        classification: Classification,
        providers: &[Arc<dyn Provider>],
    ) -> SearchPlan {
        let adaptive = self.adaptive_recommendation(&query, &classification, providers);
        let recommendation = RoutingRecommendation {
            selected_providers: adaptive.first_round_providers,
            provider_budget_requests: adaptive.provider_budget_requests,
            debug_reasons: adaptive.debug_reasons,
        };
        build_search_plan(query, classification, recommendation, &mut self.budget)
    }

    fn adaptive_recommendation(
        &self,
        query: &Query,
        classification: &Classification,
        providers: &[Arc<dyn Provider>],
    ) -> AdaptiveRoutingRecommendation {
        let descriptors = providers
            .iter()
            .map(|provider| ProviderDescriptor {
                name: provider.name().into(),
                capabilities: provider.capabilities(),
                available: matches!(provider.availability(), ProviderAvailability::Available),
            })
            .collect::<Vec<_>>();
        AdaptiveRouter.recommend(
            query,
            classification,
            &descriptors,
            &self.telemetry,
            &self.search_policy,
            now_unix(),
        )
    }

    pub async fn search(
        &mut self,
        query: Query,
        providers: Vec<Arc<dyn Provider>>,
    ) -> SearchResponse {
        let started = Instant::now();
        self.routing_trace.clear();
        self.last_plan = None;
        let mut availability_degradations = providers
            .iter()
            .filter_map(|provider| match provider.availability() {
                ProviderAvailability::Available => None,
                ProviderAvailability::Unavailable { code, message } => Some(crate::Degradation {
                    code,
                    component: provider.name().into(),
                    message,
                }),
            })
            .collect::<Vec<_>>();
        let classification = classify(&query);
        let adaptive = self.adaptive_recommendation(&query, &classification, &providers);
        let mut next_selected = adaptive.first_round_providers.clone();
        let mut attempted = BTreeSet::new();
        let mut accumulated = empty_parallel_output(self.budget.snapshot());
        let mut current_pipeline = PipelineOutput::default();
        let mut previous_results = Vec::new();
        let mut reference_time: Option<String> = None;
        let mut exception_used = false;
        let mut round = 0_u32;
        let final_stop_reason = loop {
            round += 1;
            let considered = adaptive
                .ordered_providers
                .iter()
                .filter(|provider| !attempted.contains(*provider))
                .cloned()
                .collect::<Vec<_>>();
            let remaining_ms = self
                .budget
                .deadline_ms()
                .saturating_sub(started.elapsed().as_millis() as u64);
            let recommendation = RoutingRecommendation {
                selected_providers: next_selected.clone(),
                provider_budget_requests: next_selected
                    .iter()
                    .map(|provider| (provider.clone(), 1))
                    .collect(),
                debug_reasons: adaptive.debug_reasons.clone(),
            };
            let mut plan = build_search_plan(
                query.clone(),
                classification.clone(),
                recommendation,
                &mut self.budget,
            );
            plan.global_budget.deadline_ms = remaining_ms;
            plan.expansion_policy = "search_policy_v1".into();
            plan.fallback_policy = if adaptive.fallback {
                "bootstrap_static".into()
            } else {
                "provider_value_adaptive".into()
            };
            if let Some(value) = &reference_time {
                plan.ranking_reference_time = value.clone();
            } else {
                reference_time = Some(plan.ranking_reference_time.clone());
            }
            self.last_plan = Some(plan.clone());
            let selected = plan.selected_providers.clone();
            for provider in &selected {
                attempted.insert(provider.clone());
            }
            if !selected.is_empty() {
                let output = self.execute_parallel(&plan, providers.clone()).await;
                merge_parallel_output(&mut accumulated, output);
                current_pipeline = run_pipeline(
                    &query,
                    reference_time.as_deref().unwrap_or(""),
                    &accumulated.provider_results,
                    &self.ranking_policy,
                    &self.diversity_policy,
                );
            }

            let coverage = evaluate_coverage(
                &current_pipeline.results,
                &current_pipeline.diversity,
                &self.search_policy,
            );
            let observed_gain = (round > 1)
                .then(|| observed_marginal_gain(&previous_results, &current_pipeline.results));
            let candidates = adaptive
                .ordered_providers
                .iter()
                .filter(|provider| !attempted.contains(*provider))
                .cloned()
                .collect::<Vec<_>>();
            let pending_filters = pending_explicit_filters(&query, &accumulated.provider_results);
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let remaining_ms = self.budget.deadline_ms().saturating_sub(elapsed_ms);
            let mut debug_reasons = if round == 1 {
                adaptive.debug_reasons.clone()
            } else {
                vec![]
            };
            let decision = decide_progressive_round(RoutingDecisionInput {
                coverage: &coverage,
                candidates: &candidates,
                pending_filters: &pending_filters,
                remaining_ms,
                remaining_provider_calls: self.budget.snapshot().remaining_provider_calls,
                observed_gain,
                expected_gain_by_provider: &adaptive.expected_marginal_gain_by_provider,
                policy: &self.search_policy,
                coverage_exception_used: exception_used,
            });
            let stop_or_next = match decision {
                Ok(next) => {
                    if let Some(reason) = next.debug_reason {
                        debug_reasons.push(reason);
                    }
                    exception_used |= next.uses_coverage_exception;
                    Ok(next.provider)
                }
                Err(reason) => Err(reason),
            };

            let stop_reason = stop_or_next.as_ref().err().cloned();
            let trace = round_trace(
                round,
                considered,
                selected,
                &coverage,
                &adaptive,
                observed_gain,
                stop_reason.clone(),
                debug_reasons,
            );
            tracing::debug!(
                target: "amatl::routing",
                round = trace.round,
                providers_considered = ?trace.providers_considered,
                providers_selected = ?trace.providers_selected,
                providers_skipped = ?trace.providers_skipped,
                useful_results = trace.useful_results,
                unique_domains = trace.unique_domains,
                unique_providers = trace.unique_providers,
                unique_result_types = trace.unique_result_types,
                coverage_minimum = trace.coverage_minimum,
                coverage_target = trace.coverage_target,
                low_diversity = trace.low_diversity,
                expected_marginal_gain_by_provider = ?trace.expected_marginal_gain_by_provider,
                observed_marginal_gain = ?trace.observed_marginal_gain,
                stop_reason = ?trace.stop_reason,
                debug_reasons = ?trace.debug_reasons,
                "progressive routing decision"
            );
            self.routing_trace.push(trace);
            match stop_or_next {
                Ok(candidate) => {
                    previous_results = current_pipeline.results.clone();
                    next_selected = vec![candidate];
                }
                Err(reason) => {
                    break reason;
                }
            }
        };

        let results = current_pipeline.results;
        let mut degradations = current_pipeline.degradations;
        degradations.append(&mut availability_degradations);
        let coverage =
            evaluate_coverage(&results, &current_pipeline.diversity, &self.search_policy);
        if !results.is_empty()
            && !coverage.coverage_target
            && matches!(
                final_stop_reason,
                SearchStopReason::TimeExhausted
                    | SearchStopReason::DeadlineNear
                    | SearchStopReason::ProviderLimit
            )
        {
            degradations.push(crate::Degradation {
                code: final_stop_reason.as_str().into(),
                component: "routing".into(),
                message: "progressive search stopped before target coverage".into(),
            });
        }
        let mut telemetry_observations = accumulated.telemetry_observations.clone();
        enrich_telemetry(&mut telemetry_observations, &results);
        for observation in telemetry_observations {
            self.telemetry.record(observation).await;
        }
        let mut errors = accumulated
            .errors
            .iter()
            .map(composite_error)
            .collect::<Vec<_>>();
        let no_usable_results = results.is_empty()
            && (attempted.is_empty()
                || accumulated.providers_failed.len() == attempted.len()
                || !accumulated.providers_partial.is_empty()
                || !degradations.is_empty());
        if results.is_empty() && attempted.is_empty() {
            errors.push(CompositeError {
                code: "no_available_provider".into(),
                message: "no approved and configured provider is available".into(),
                providers: vec![],
                recoverable: false,
            });
        } else if no_usable_results && errors.is_empty() {
            errors.push(CompositeError {
                code: "no_usable_results".into(),
                message: "providers returned no result that satisfied the Search contract".into(),
                providers: accumulated.providers_used.clone(),
                recoverable: false,
            });
        }
        let status = if no_usable_results {
            SearchStatus::Failure
        } else if !results.is_empty()
            && (!degradations.is_empty()
                || !accumulated.providers_failed.is_empty()
                || !accumulated.providers_partial.is_empty())
        {
            SearchStatus::PartialSuccess
        } else {
            SearchStatus::Success
        };
        SearchResponse {
            schema_version: SCHEMA_VERSION.into(),
            query: query.raw_query,
            status,
            results,
            providers_used: accumulated.providers_used,
            providers_failed: accumulated.providers_failed,
            providers_partial: accumulated.providers_partial,
            errors,
            degradations,
            elapsed_ms: started.elapsed().as_millis() as u64,
            total_results: None,
            page: None,
            page_size: None,
        }
    }

    pub async fn execute_parallel(
        &self,
        plan: &SearchPlan,
        providers: Vec<Arc<dyn Provider>>,
    ) -> ParallelSearchOutput {
        let started = Instant::now();
        let selected: BTreeSet<_> = plan.selected_providers.iter().cloned().collect();
        let by_name: BTreeMap<_, _> = providers
            .into_iter()
            .map(|provider| (provider.name().to_string(), provider))
            .collect();
        let global_semaphore = Arc::new(Semaphore::new(self.global_concurrency));
        let provider_semaphores = plan
            .selected_providers
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    Arc::new(Semaphore::new(self.per_provider_concurrency)),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut tasks = JoinSet::new();
        for provider_name in &plan.selected_providers {
            let Some(provider) = by_name.get(provider_name).cloned() else {
                continue;
            };
            let plan = plan.clone();
            let timeout_ms = self.provider_timeout_ms.min(plan.global_budget.deadline_ms);
            let name = provider_name.clone();
            let category = plan.classification.primary_category.clone();
            let estimated_cost = provider.capabilities().estimated_cost;
            let global_semaphore = global_semaphore.clone();
            let provider_semaphore = provider_semaphores[provider_name].clone();
            let max_retries = self.max_retries;
            let retry_jitter_ms = self.retry_jitter_ms;
            let request_id = self.request_id.clone();
            let provider_span = tracing::info_span!(
                target: "amatl::providers",
                "provider_call",
                request_id = request_id.as_deref().unwrap_or("-"),
                provider = %name,
                timeout_ms
            );
            tasks.spawn(
                async move {
                    let provider_started = Instant::now();
                    let hard_deadline = tokio::time::Instant::now()
                        + Duration::from_millis(plan.global_budget.deadline_ms);
                    let permits = tokio::time::timeout_at(hard_deadline, async {
                        let global = global_semaphore.acquire_owned().await.map_err(|_| ())?;
                        let provider = provider_semaphore.acquire_owned().await.map_err(|_| ())?;
                        Ok::<_, ()>((global, provider))
                    })
                    .await;
                    let _permits = match permits {
                        Ok(Ok(permits)) => permits,
                        _ => {
                            return (
                                name.clone(),
                                Err(ProviderError {
                                    schema_version: SCHEMA_VERSION.into(),
                                    provider: name,
                                    kind: ProviderErrorKind::Timeout,
                                    message: "provider concurrency deadline exceeded".into(),
                                    retry_after_ms: None,
                                }),
                                provider_started.elapsed().as_millis() as u64,
                                category,
                                estimated_cost,
                            )
                        }
                    };
                    let mut attempt = 0_u32;
                    loop {
                        let remaining =
                            hard_deadline.saturating_duration_since(tokio::time::Instant::now());
                        let attempt_timeout = Duration::from_millis(timeout_ms).min(remaining);
                        let result = if attempt_timeout.is_zero() {
                            Err(())
                        } else {
                            tokio::time::timeout(
                                attempt_timeout,
                                provider.search(
                                    &plan,
                                    &ProviderContext {
                                        timeout_ms,
                                        request_id: request_id.clone(),
                                    },
                                ),
                            )
                            .await
                            .map_err(|_| ())
                        };
                        tracing::debug!(
                            target: "amatl::providers",
                            attempt,
                            outcome = match &result {
                                Ok(Ok(_)) => "ok",
                                Ok(Err(_)) => "provider_error",
                                Err(()) => "timeout",
                            },
                            latency_ms = provider_started.elapsed().as_millis() as u64,
                            "provider call finished"
                        );
                        let provider_result = match result {
                            Ok(result) => result,
                            Err(()) => Err(ProviderError {
                                schema_version: SCHEMA_VERSION.into(),
                                provider: name.clone(),
                                kind: ProviderErrorKind::Timeout,
                                message: "provider deadline exceeded".into(),
                                retry_after_ms: None,
                            }),
                        };
                        let retry_delay_ms = provider_result.as_ref().err().and_then(|error| {
                            is_recoverable(&error.kind)
                                .then_some(error.retry_after_ms.unwrap_or(50))
                        });
                        if attempt >= max_retries || retry_delay_ms.is_none() {
                            break (
                                name,
                                provider_result,
                                provider_started.elapsed().as_millis() as u64,
                                category,
                                estimated_cost,
                            );
                        }
                        attempt += 1;
                        let exponential = retry_delay_ms
                            .unwrap_or(50)
                            .saturating_mul(1_u64 << attempt.saturating_sub(1));
                        let jitter_ms = retry_jitter(&name, attempt, retry_jitter_ms);
                        let backoff = Duration::from_millis(exponential.saturating_add(jitter_ms));
                        if tokio::time::Instant::now() + backoff >= hard_deadline {
                            break (
                                name,
                                provider_result,
                                provider_started.elapsed().as_millis() as u64,
                                category,
                                estimated_cost,
                            );
                        }
                        tokio::time::sleep(backoff).await;
                    }
                }
                .instrument(provider_span),
            );
        }
        let mut output = ParallelSearchOutput {
            provider_results: vec![],
            providers_used: vec![],
            providers_failed: vec![],
            providers_partial: vec![],
            errors: vec![],
            sequential_rounds_elapsed_ms: 0,
            budget_remaining: self.budget.snapshot(),
            telemetry_observations: vec![],
        };
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((name, Ok(result), latency_ms, category, estimated_cost)) => {
                    let unique_results = result
                        .results
                        .iter()
                        .map(|item| item.url.as_str())
                        .collect::<BTreeSet<_>>()
                        .len();
                    output
                        .telemetry_observations
                        .push(TelemetryObservation::from_provider_result(
                            ProviderTelemetryInput {
                                provider: name.clone(),
                                category,
                                latency_ms,
                                total_results: result.results.len(),
                                unique_results,
                                error: result.errors.first().map(|error| &error.kind),
                                partial: result.status == ProviderExecutionStatus::Partial,
                                estimated_cost,
                                request_id: self.request_id.clone(),
                            },
                        ));
                    output.providers_used.push(name.clone());
                    if result.status == ProviderExecutionStatus::Partial {
                        output.providers_partial.push(name);
                    }
                    output.errors.extend(result.errors.clone());
                    output.provider_results.push(result);
                }
                Ok((name, Err(error), latency_ms, category, estimated_cost)) => {
                    output
                        .telemetry_observations
                        .push(TelemetryObservation::from_provider_result(
                            ProviderTelemetryInput {
                                provider: name.clone(),
                                category,
                                latency_ms,
                                total_results: 0,
                                unique_results: 0,
                                error: Some(&error.kind),
                                partial: false,
                                estimated_cost,
                                request_id: self.request_id.clone(),
                            },
                        ));
                    output.providers_failed.push(name);
                    output.errors.push(error);
                }
                Err(_) => {}
            }
        }
        for missing in selected.difference(
            &output
                .providers_used
                .iter()
                .chain(output.providers_failed.iter())
                .cloned()
                .collect(),
        ) {
            output.providers_failed.push(missing.clone());
        }
        output.providers_used.sort();
        output.providers_failed.sort();
        output.providers_partial.sort();
        output.sequential_rounds_elapsed_ms = started.elapsed().as_millis() as u64;
        output
    }
}

fn retry_jitter(provider: &str, attempt: u32, maximum_ms: u64) -> u64 {
    if maximum_ms == 0 {
        return 0;
    }
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| u64::from(value.subsec_nanos()));
    let provider_mix = provider
        .bytes()
        .fold(0_u64, |state, byte| state.rotate_left(5) ^ u64::from(byte));
    let counter = JITTER_COUNTER.fetch_add(1, Ordering::Relaxed);
    (clock ^ provider_mix ^ u64::from(attempt) ^ counter) % (maximum_ms + 1)
}

#[derive(Default)]
struct PipelineOutput {
    results: Vec<crate::SearchResult>,
    degradations: Vec<crate::Degradation>,
    diversity: DiversityMetrics,
}

fn run_pipeline(
    query: &Query,
    ranking_reference_time: &str,
    provider_results: &[ProviderResult],
    ranking_policy: &RankingPolicyV1,
    diversity_policy: &DiversityPolicyV1,
) -> PipelineOutput {
    let (normalized, mut degradations) = normalize(provider_results);
    degradations.extend(
        normalized
            .iter()
            .flat_map(|result| result.degradations.iter().cloned()),
    );
    let active_provider_count = normalized
        .iter()
        .map(|result| result.provider.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let canonical = normalized.into_iter().map(canonicalize).collect();
    let deduped = deduplicate(canonical);
    let ranked = rank(
        query,
        ranking_reference_time,
        active_provider_count,
        deduped,
        ranking_policy,
    );
    let diversified = diversify(ranked, diversity_policy);
    PipelineOutput {
        results: diversified.results,
        degradations,
        diversity: diversified.metrics,
    }
}

fn empty_parallel_output(budget_remaining: BudgetSnapshot) -> ParallelSearchOutput {
    ParallelSearchOutput {
        provider_results: vec![],
        providers_used: vec![],
        providers_failed: vec![],
        providers_partial: vec![],
        errors: vec![],
        sequential_rounds_elapsed_ms: 0,
        budget_remaining,
        telemetry_observations: vec![],
    }
}

fn merge_parallel_output(target: &mut ParallelSearchOutput, mut source: ParallelSearchOutput) {
    target.provider_results.append(&mut source.provider_results);
    target.providers_used.append(&mut source.providers_used);
    target.providers_failed.append(&mut source.providers_failed);
    target
        .providers_partial
        .append(&mut source.providers_partial);
    target.errors.append(&mut source.errors);
    target
        .telemetry_observations
        .append(&mut source.telemetry_observations);
    target.sequential_rounds_elapsed_ms = target
        .sequential_rounds_elapsed_ms
        .saturating_add(source.sequential_rounds_elapsed_ms);
    target.budget_remaining = source.budget_remaining;
    target.providers_used.sort();
    target.providers_used.dedup();
    target.providers_failed.sort();
    target.providers_failed.dedup();
    target.providers_partial.sort();
    target.providers_partial.dedup();
}

fn pending_explicit_filters(query: &Query, results: &[ProviderResult]) -> BTreeSet<String> {
    let mut required = BTreeSet::new();
    if !query.domains.is_empty() {
        required.insert("site".into());
    }
    if !query.excluded_domains.is_empty() {
        required.insert("excluded_site".into());
    }
    if !query.file_types.is_empty() {
        required.insert("filetype".into());
    }
    if query.language.is_some() {
        required.insert("language".into());
    }
    if query.region.is_some() {
        required.insert("region".into());
    }
    if query.date_from.is_some() {
        required.insert("date_from".into());
    }
    if query.date_to.is_some() {
        required.insert("date_to".into());
    }
    let mut covered = results
        .iter()
        .flat_map(|result| result.accepted_filters.iter().cloned())
        .collect::<BTreeSet<_>>();
    if covered.contains("time_range") {
        covered.insert("date_from".into());
        covered.insert("date_to".into());
    }
    required.retain(|filter| !covered.contains(filter));
    required
}

struct RoutingDecisionInput<'a> {
    coverage: &'a CoverageMetrics,
    candidates: &'a [String],
    pending_filters: &'a BTreeSet<String>,
    remaining_ms: u64,
    remaining_provider_calls: u32,
    observed_gain: Option<f64>,
    expected_gain_by_provider: &'a BTreeMap<String, f64>,
    policy: &'a SearchPolicyV1,
    coverage_exception_used: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct NextProviderDecision {
    provider: String,
    uses_coverage_exception: bool,
    debug_reason: Option<String>,
}

fn decide_progressive_round(
    input: RoutingDecisionInput<'_>,
) -> Result<NextProviderDecision, SearchStopReason> {
    let expected_gain_low = input.coverage.coverage_minimum
        && !input.candidates.is_empty()
        && input.pending_filters.is_empty()
        && input.candidates.iter().all(|provider| {
            input
                .expected_gain_by_provider
                .get(provider)
                .copied()
                .unwrap_or(0.0)
                < input.policy.minimum_expected_marginal_gain
        });
    let observed_gain_low = input.observed_gain.is_some_and(|gain| {
        gain < input.policy.minimum_marginal_gain && input.pending_filters.is_empty()
    });

    if input.remaining_ms == 0 {
        return Err(SearchStopReason::TimeExhausted);
    }
    if input.remaining_ms < input.policy.minimum_remaining_deadline_ms {
        return Err(SearchStopReason::DeadlineNear);
    }
    if !input.candidates.is_empty() && input.remaining_provider_calls == 0 {
        return Err(SearchStopReason::ProviderLimit);
    }
    if input.coverage.coverage_target && input.pending_filters.is_empty() {
        return Err(SearchStopReason::CoverageTargetReached);
    }
    if expected_gain_low || observed_gain_low {
        return Err(SearchStopReason::MarginalGainLow);
    }
    if input.candidates.is_empty() {
        return Err(SearchStopReason::ProvidersExhausted);
    }
    if input.coverage.coverage_minimum
        && !input.coverage.low_diversity
        && input.pending_filters.is_empty()
    {
        return Err(SearchStopReason::ExplicitFilterSatisfied);
    }
    if !input.pending_filters.is_empty() {
        return Ok(NextProviderDecision {
            provider: input.candidates[0].clone(),
            uses_coverage_exception: false,
            debug_reason: Some(format!(
                "explicit_filter_pending:{}",
                input
                    .pending_filters
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(",")
            )),
        });
    }
    if let Some(provider) = input.candidates.iter().find(|provider| {
        input
            .expected_gain_by_provider
            .get(*provider)
            .copied()
            .unwrap_or(0.0)
            >= input.policy.minimum_expected_marginal_gain
    }) {
        return Ok(NextProviderDecision {
            provider: provider.clone(),
            uses_coverage_exception: false,
            debug_reason: None,
        });
    }
    if !input.coverage.coverage_minimum && !input.coverage_exception_used {
        return Ok(NextProviderDecision {
            provider: input.candidates[0].clone(),
            uses_coverage_exception: true,
            debug_reason: Some("coverage_exception_once".into()),
        });
    }
    Err(SearchStopReason::MarginalGainLow)
}

#[allow(clippy::too_many_arguments)]
fn round_trace(
    round: u32,
    providers_considered: Vec<String>,
    providers_selected: Vec<String>,
    coverage: &CoverageMetrics,
    adaptive: &AdaptiveRoutingRecommendation,
    observed_marginal_gain: Option<f64>,
    stop_reason: Option<SearchStopReason>,
    debug_reasons: Vec<String>,
) -> ProgressiveRoundTrace {
    let selected = providers_selected.iter().collect::<BTreeSet<_>>();
    let providers_skipped = providers_considered
        .iter()
        .filter(|provider| !selected.contains(provider))
        .cloned()
        .collect();
    let expected_marginal_gain_by_provider = providers_considered
        .iter()
        .filter_map(|provider| {
            adaptive
                .expected_marginal_gain_by_provider
                .get(provider)
                .map(|gain| (provider.clone(), *gain))
        })
        .collect();
    ProgressiveRoundTrace {
        round,
        providers_considered,
        providers_selected,
        providers_skipped,
        useful_results: coverage.useful_results,
        unique_domains: coverage.unique_domains,
        unique_providers: coverage.unique_providers,
        unique_result_types: coverage.unique_result_types,
        coverage_minimum: coverage.coverage_minimum,
        coverage_target: coverage.coverage_target,
        low_diversity: coverage.low_diversity,
        expected_marginal_gain_by_provider,
        observed_marginal_gain,
        stop_reason,
        debug_reasons,
    }
}

fn is_recoverable(kind: &ProviderErrorKind) -> bool {
    matches!(
        kind,
        ProviderErrorKind::Timeout
            | ProviderErrorKind::RateLimit
            | ProviderErrorKind::Network
            | ProviderErrorKind::Unavailable
    )
}

fn composite_error(error: &ProviderError) -> CompositeError {
    CompositeError {
        code: match error.kind {
            ProviderErrorKind::Timeout => "provider_timeout",
            ProviderErrorKind::RateLimit => "provider_rate_limit",
            ProviderErrorKind::Auth => "provider_auth",
            ProviderErrorKind::Network => "provider_network",
            ProviderErrorKind::InvalidResponse => "provider_invalid_response",
            ProviderErrorKind::ParserError => "provider_parser_error",
            ProviderErrorKind::Quota => "provider_quota",
            ProviderErrorKind::Unavailable => "provider_unavailable",
        }
        .into(),
        message: error.message.clone(),
        providers: vec![error.provider.clone()],
        recoverable: is_recoverable(&error.kind),
    }
}

fn enrich_telemetry(observations: &mut [TelemetryObservation], results: &[crate::SearchResult]) {
    let top_k = results.iter().take(10).collect::<Vec<_>>();
    for observation in observations {
        let contributed = results
            .iter()
            .filter(|result| result.providers.contains(&observation.provider))
            .collect::<Vec<_>>();
        observation.top_k_contribution = if top_k.is_empty() {
            0.0
        } else {
            top_k
                .iter()
                .filter(|result| result.providers.contains(&observation.provider))
                .count() as f64
                / top_k.len() as f64
        };
        observation.diversity = if contributed.is_empty() {
            0.0
        } else {
            contributed
                .iter()
                .map(|result| result.domain.as_str())
                .collect::<BTreeSet<_>>()
                .len() as f64
                / contributed.len() as f64
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_query, MockBehavior, MockProvider, ProviderItem, Rank};

    fn item(url: &str) -> ProviderItem {
        ProviderItem {
            title: Some("Rust async guide".into()),
            url: url.into(),
            provider_rank: Some(Rank::FIRST),
            snippet: Some("Guide".into()),
            result_type: None,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: Default::default(),
        }
    }

    /// Provider that records the request id it was called with.
    struct RecordingProvider {
        seen: Arc<std::sync::Mutex<Vec<Option<String>>>>,
    }

    #[async_trait::async_trait]
    impl Provider for RecordingProvider {
        fn name(&self) -> &str {
            "recorder"
        }

        fn capabilities(&self) -> crate::ProviderCapabilities {
            MockProvider::success("recorder", vec![]).capabilities()
        }

        async fn search(
            &self,
            plan: &SearchPlan,
            context: &ProviderContext,
        ) -> Result<ProviderResult, ProviderError> {
            self.seen.lock().unwrap().push(context.request_id.clone());
            MockProvider::success("recorder", vec![item("https://example.com/recorded")])
                .search(plan, context)
                .await
        }
    }

    #[tokio::test]
    async fn request_id_reaches_every_outbound_provider_call() {
        let seen = Arc::new(std::sync::Mutex::new(vec![]));
        let providers: Vec<Arc<dyn Provider>> =
            vec![Arc::new(RecordingProvider { seen: seen.clone() })];
        SearchOrchestrator::new(Budget::new(1, 8_000), 100)
            .with_request_id(Some("req-42".into()))
            .search(parse_query("rust async".into()).unwrap(), providers)
            .await;
        assert_eq!(seen.lock().unwrap().as_slice(), [Some("req-42".to_owned())]);
    }

    #[tokio::test]
    async fn partial_failure_keeps_useful_results() {
        let providers: Vec<Arc<dyn Provider>> = vec![
            Arc::new(MockProvider::success(
                "ok",
                vec![item("https://example.com/?utm_source=x")],
            )),
            Arc::new(MockProvider::new(
                "fail",
                MockBehavior::Failure(ProviderErrorKind::Unavailable),
            )),
        ];
        let response = SearchOrchestrator::new(Budget::new(2, 8_000), 100)
            .search(parse_query("rust async".into()).unwrap(), providers)
            .await;
        assert_eq!(response.status, SearchStatus::PartialSuccess);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.providers_failed, ["fail"]);
        assert_eq!(response.errors.len(), 1);
    }

    #[tokio::test]
    async fn slow_provider_does_not_block_successful_provider() {
        let providers: Vec<Arc<dyn Provider>> = vec![
            Arc::new(MockProvider::success(
                "ok",
                vec![item("https://example.com/")],
            )),
            Arc::new(MockProvider::new(
                "slow",
                MockBehavior::Delayed(vec![], 100),
            )),
        ];
        let response = SearchOrchestrator::new(Budget::new(2, 8_000), 10)
            .search(parse_query("rust".into()).unwrap(), providers)
            .await;
        assert_eq!(response.status, SearchStatus::PartialSuccess);
        assert_eq!(response.results.len(), 1);
    }

    #[tokio::test]
    async fn retries_recoverable_error_once_but_never_auth() {
        let retry = Arc::new(MockProvider::new(
            "retry",
            MockBehavior::Failure(ProviderErrorKind::Network),
        ));
        let auth = Arc::new(MockProvider::new(
            "auth",
            MockBehavior::Failure(ProviderErrorKind::Auth),
        ));
        let providers: Vec<Arc<dyn Provider>> = vec![retry.clone(), auth.clone()];
        let query = parse_query("rust".into()).unwrap();
        SearchOrchestrator::new(Budget::new(2, 1_000), 100)
            .search(query, providers)
            .await;
        assert_eq!(retry.attempts(), 2);
        assert_eq!(auth.attempts(), 1);
    }

    #[tokio::test]
    async fn success_serializes_an_empty_error_list() {
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::success(
            "ok",
            vec![item("https://example.com/")],
        ))];
        let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
            .search(parse_query("rust".into()).unwrap(), providers)
            .await;
        assert_eq!(response.status, SearchStatus::Success);
        assert!(response.errors.is_empty());
    }

    #[tokio::test]
    async fn global_concurrency_limit_serializes_provider_calls() {
        let providers: Vec<Arc<dyn Provider>> = vec![
            Arc::new(MockProvider::new(
                "slow-a",
                MockBehavior::Delayed(vec![item("https://a.example/")], 40),
            )),
            Arc::new(MockProvider::new(
                "slow-b",
                MockBehavior::Delayed(vec![item("https://b.example/")], 40),
            )),
        ];
        let started = Instant::now();
        let response = SearchOrchestrator::new(Budget::new(2, 1_000), 200)
            .with_execution_limits(1, 1, 0, 0)
            .search(parse_query("rust".into()).unwrap(), providers)
            .await;
        assert_eq!(response.providers_used.len(), 2);
        assert!(started.elapsed() >= Duration::from_millis(70));
    }

    #[test]
    fn progressive_decision_table_covers_priority_and_expansion_paths() {
        let policy = SearchPolicyV1::default();
        let candidates = vec!["a".into(), "b".into()];
        let gains = BTreeMap::from([("a".into(), 0.20), ("b".into(), 0.10)]);
        let empty_filters = BTreeSet::new();
        let minimum = CoverageMetrics {
            coverage_minimum: true,
            ..Default::default()
        };
        let decide = |coverage: &CoverageMetrics,
                      candidates: &[String],
                      filters: &BTreeSet<String>,
                      remaining_ms,
                      remaining_calls,
                      observed_gain,
                      exception_used| {
            decide_progressive_round(RoutingDecisionInput {
                coverage,
                candidates,
                pending_filters: filters,
                remaining_ms,
                remaining_provider_calls: remaining_calls,
                observed_gain,
                expected_gain_by_provider: &gains,
                policy: &policy,
                coverage_exception_used: exception_used,
            })
        };

        assert_eq!(
            decide(&minimum, &candidates, &empty_filters, 0, 1, None, false),
            Err(SearchStopReason::TimeExhausted)
        );
        assert_eq!(
            decide(
                &minimum,
                &candidates,
                &empty_filters,
                10_000,
                0,
                None,
                false
            ),
            Err(SearchStopReason::ProviderLimit)
        );
        let target = CoverageMetrics {
            coverage_minimum: true,
            coverage_target: true,
            ..Default::default()
        };
        assert_eq!(
            decide(&target, &candidates, &empty_filters, 10_000, 1, None, false),
            Err(SearchStopReason::CoverageTargetReached)
        );
        let low_diversity = CoverageMetrics {
            low_diversity: true,
            ..minimum.clone()
        };
        assert_eq!(
            decide(
                &low_diversity,
                &candidates,
                &empty_filters,
                10_000,
                1,
                None,
                false
            )
            .unwrap()
            .provider,
            "a"
        );
        let filters = BTreeSet::from(["site".into()]);
        let next = decide(&minimum, &candidates, &filters, 10_000, 1, None, false).unwrap();
        assert_eq!(next.provider, "a");
        assert_eq!(
            next.debug_reason.as_deref(),
            Some("explicit_filter_pending:site")
        );
    }
}
