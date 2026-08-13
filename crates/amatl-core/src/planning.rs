use crate::budget::Budget;
use crate::model::{Classification, Query, SearchPlan, SCHEMA_VERSION};
use crate::router::RoutingRecommendation;
use std::collections::BTreeMap;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub fn build_search_plan(
    query: Query,
    classification: Classification,
    recommendation: RoutingRecommendation,
    budget: &mut Budget,
) -> SearchPlan {
    let mut provider_budgets = BTreeMap::new();
    for provider in &recommendation.selected_providers {
        if budget.reserve_provider().is_ok() {
            provider_budgets.insert(provider.clone(), 1);
        }
    }
    let selected_providers = recommendation
        .selected_providers
        .into_iter()
        .filter(|provider| provider_budgets.contains_key(provider))
        .collect::<Vec<_>>();
    SearchPlan {
        schema_version: SCHEMA_VERSION.into(),
        query,
        classification,
        provider_priority: selected_providers.clone(),
        selected_providers,
        provider_budget_requests: recommendation.provider_budget_requests,
        provider_budgets,
        global_budget: budget.global_snapshot(),
        ranking_reference_time: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into()),
        fallback_policy: "bootstrap_static".into(),
        expansion_policy: "search_policy_v1".into(),
        stop_conditions: vec![
            "time_exhausted".into(),
            "deadline_near".into(),
            "provider_limit".into(),
            "coverage_target_reached".into(),
            "marginal_gain_low".into(),
            "providers_exhausted".into(),
        ],
        debug_reasons: recommendation.debug_reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{classify, parse_query};
    #[test]
    fn orchestrator_budget_limits_final_plan() {
        let query = parse_query("rust".into()).unwrap();
        let classification = classify(&query);
        let recommendation = RoutingRecommendation {
            selected_providers: vec!["a".into(), "b".into()],
            provider_budget_requests: [("a".into(), 1), ("b".into(), 1)].into(),
            debug_reasons: vec![],
        };
        let plan = build_search_plan(
            query,
            classification,
            recommendation,
            &mut Budget::new(1, 3_000),
        );
        assert_eq!(plan.selected_providers, ["a"]);
        assert_eq!(plan.provider_budgets.len(), 1);
    }
}
