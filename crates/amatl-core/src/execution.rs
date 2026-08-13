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

static JITTER_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct ParallelSearchOutput {
    pub provider_results: Vec<ProviderResult>,
    pub providers_used: Vec<String>,
    pub providers_failed: Vec<String>,
    pub providers_partial: Vec<String>,
    pub errors: Vec<ProviderError>,
    pub elapsed_ms: u64,
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
        let descriptors = providers
            .iter()
            .map(|provider| ProviderDescriptor {
                name: provider.name().into(),
                capabilities: provider.capabilities(),
                available: matches!(provider.availability(), ProviderAvailability::Available),
            })
            .collect::<Vec<_>>();
        let adaptive = AdaptiveRouter.recommend(
            &query,
            &classification,
            &descriptors,
            &self.telemetry,
            &self.search_policy,
            now_unix(),
        );
        let recommendation = RoutingRecommendation {
            selected_providers: adaptive.first_round_providers,
            provider_budget_requests: adaptive.provider_budget_requests,
            debug_reasons: adaptive.debug_reasons,
        };
        build_search_plan(query, classification, recommendation, &mut self.budget)
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
        let descriptors = providers
            .iter()
            .map(|provider| ProviderDescriptor {
                name: provider.name().into(),
                capabilities: provider.capabilities(),
                available: matches!(provider.availability(), ProviderAvailability::Available),
            })
            .collect::<Vec<_>>();
        let adaptive = AdaptiveRouter.recommend(
            &query,
            &classification,
            &descriptors,
            &self.telemetry,
            &self.search_policy,
            now_unix(),
        );
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
            let expected_gain_low = coverage.coverage_minimum
                && !candidates.is_empty()
                && pending_filters.is_empty()
                && candidates.iter().all(|provider| {
                    adaptive
                        .expected_marginal_gain_by_provider
                        .get(provider)
                        .copied()
                        .unwrap_or(0.0)
                        < self.search_policy.minimum_expected_marginal_gain
                });
            let observed_gain_low = observed_gain.is_some_and(|gain| {
                gain < self.search_policy.minimum_marginal_gain && pending_filters.is_empty()
            });

            let stop_or_next = if remaining_ms == 0 {
                Err(SearchStopReason::TimeExhausted)
            } else if remaining_ms < self.search_policy.minimum_remaining_deadline_ms {
                Err(SearchStopReason::DeadlineNear)
            } else if !candidates.is_empty() && self.budget.snapshot().remaining_provider_calls == 0
            {
                Err(SearchStopReason::ProviderLimit)
            } else if coverage.coverage_target && pending_filters.is_empty() {
                Err(SearchStopReason::CoverageTargetReached)
            } else if expected_gain_low || observed_gain_low {
                Err(SearchStopReason::MarginalGainLow)
            } else if candidates.is_empty() {
                Err(SearchStopReason::ProvidersExhausted)
            } else if coverage.coverage_minimum
                && !coverage.low_diversity
                && pending_filters.is_empty()
            {
                Err(SearchStopReason::ExplicitFilterSatisfied)
            } else if !pending_filters.is_empty() {
                debug_reasons.push(format!(
                    "explicit_filter_pending:{}",
                    pending_filters
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
                Ok(candidates[0].clone())
            } else if let Some(candidate) = candidates.iter().find(|provider| {
                adaptive
                    .expected_marginal_gain_by_provider
                    .get(*provider)
                    .copied()
                    .unwrap_or(0.0)
                    >= self.search_policy.minimum_expected_marginal_gain
            }) {
                Ok(candidate.clone())
            } else if !coverage.coverage_minimum && !exception_used {
                exception_used = true;
                debug_reasons.push("coverage_exception_once".into());
                Ok(candidates[0].clone())
            } else {
                Err(SearchStopReason::MarginalGainLow)
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
            tasks.spawn(async move {
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
                            provider.search(&plan, &ProviderContext { timeout_ms }),
                        )
                        .await
                        .map_err(|_| ())
                    };
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
                        is_recoverable(&error.kind).then_some(error.retry_after_ms.unwrap_or(50))
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
            });
        }
        let mut output = ParallelSearchOutput {
            provider_results: vec![],
            providers_used: vec![],
            providers_failed: vec![],
            providers_partial: vec![],
            errors: vec![],
            elapsed_ms: 0,
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
        output.elapsed_ms = started.elapsed().as_millis() as u64;
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
        elapsed_ms: 0,
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
    target.elapsed_ms = target.elapsed_ms.saturating_add(source.elapsed_ms);
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
}
