use crate::model::GlobalBudgetSnapshot;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum BudgetExhaustionCause {
    #[error("provider_limit")]
    ProviderLimit,
    #[error("time_exhausted")]
    TimeExhausted,
    #[error("deadline_near")]
    DeadlineNear,
    #[error("fetch_limit")]
    FetchLimit,
    #[error("byte_limit")]
    ByteLimit,
    #[error("redirect_limit")]
    RedirectLimit,
    #[error("browser_limit")]
    BrowserLimit,
    #[error("crawl_limit")]
    CrawlLimit,
    #[error("cost_limit")]
    CostLimit,
    #[error("subquery_limit")]
    SubqueryLimit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeepBudgetSnapshot {
    pub remaining_fetches: u32,
    pub remaining_bytes: u64,
    pub remaining_redirects: u32,
    pub remaining_browser_calls: u32,
    pub remaining_crawl_urls: u32,
    pub remaining_subqueries: u32,
    pub remaining_cost: u64,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug)]
pub struct DeepBudget {
    snapshot: DeepBudgetSnapshot,
    max_fetches: u32,
    max_browser_calls: u32,
    max_crawl_urls: u32,
}

impl DeepBudget {
    pub fn new(
        fetches: u32,
        bytes: u64,
        redirects: u32,
        browser_calls: u32,
        crawl_urls: u32,
        deadline_ms: u64,
    ) -> Self {
        Self {
            snapshot: DeepBudgetSnapshot {
                remaining_fetches: fetches,
                remaining_bytes: bytes,
                remaining_redirects: redirects,
                remaining_browser_calls: browser_calls,
                remaining_crawl_urls: crawl_urls,
                remaining_subqueries: 0,
                remaining_cost: 0,
                deadline_ms,
            },
            max_fetches: fetches,
            max_browser_calls: browser_calls,
            max_crawl_urls: crawl_urls,
        }
    }

    pub fn with_gap_limits(mut self, max_subqueries: u32, max_cost: u64) -> Self {
        self.snapshot.remaining_subqueries = max_subqueries.min(2);
        self.snapshot.remaining_cost = max_cost;
        self
    }

    pub fn reserve_fetch(&mut self) -> Result<u64, BudgetExhaustionCause> {
        if self.snapshot.remaining_fetches == 0 {
            return Err(BudgetExhaustionCause::FetchLimit);
        }
        if self.snapshot.remaining_crawl_urls == 0 {
            return Err(BudgetExhaustionCause::CrawlLimit);
        }
        if self.snapshot.remaining_bytes == 0 {
            return Err(BudgetExhaustionCause::ByteLimit);
        }
        self.snapshot.remaining_fetches -= 1;
        self.snapshot.remaining_crawl_urls -= 1;
        Ok(self.snapshot.remaining_bytes)
    }

    pub fn consume_fetch(
        &mut self,
        bytes: u64,
        redirects: u32,
    ) -> Result<(), BudgetExhaustionCause> {
        if bytes > self.snapshot.remaining_bytes {
            return Err(BudgetExhaustionCause::ByteLimit);
        }
        if redirects > self.snapshot.remaining_redirects {
            return Err(BudgetExhaustionCause::RedirectLimit);
        }
        self.snapshot.remaining_bytes -= bytes;
        self.snapshot.remaining_redirects -= redirects;
        Ok(())
    }

    pub fn reserve_browser(&mut self) -> Result<(), BudgetExhaustionCause> {
        if self.snapshot.remaining_browser_calls == 0 {
            return Err(BudgetExhaustionCause::BrowserLimit);
        }
        self.snapshot.remaining_browser_calls -= 1;
        Ok(())
    }

    pub fn release_fetch(&mut self) {
        self.snapshot.remaining_fetches = self
            .snapshot
            .remaining_fetches
            .saturating_add(1)
            .min(self.max_fetches);
        self.snapshot.remaining_crawl_urls = self
            .snapshot
            .remaining_crawl_urls
            .saturating_add(1)
            .min(self.max_crawl_urls);
    }

    pub fn release_browser(&mut self) {
        self.snapshot.remaining_browser_calls = self
            .snapshot
            .remaining_browser_calls
            .saturating_add(1)
            .min(self.max_browser_calls);
    }

    pub fn reserve_subquery(&mut self, estimated_cost: u64) -> Result<(), BudgetExhaustionCause> {
        if self.snapshot.remaining_subqueries == 0 {
            return Err(BudgetExhaustionCause::SubqueryLimit);
        }
        if estimated_cost > self.snapshot.remaining_cost {
            return Err(BudgetExhaustionCause::CostLimit);
        }
        self.snapshot.remaining_subqueries -= 1;
        self.snapshot.remaining_cost -= estimated_cost;
        Ok(())
    }

    pub fn snapshot(&self) -> DeepBudgetSnapshot {
        self.snapshot.clone()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub max_provider_calls: u32,
    pub remaining_provider_calls: u32,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug)]
pub struct Budget {
    max_provider_calls: u32,
    remaining_provider_calls: u32,
    deadline_ms: u64,
}

impl Budget {
    pub fn new(max_provider_calls: u32, deadline_ms: u64) -> Self {
        Self {
            max_provider_calls,
            remaining_provider_calls: max_provider_calls,
            deadline_ms,
        }
    }
    pub fn reserve_provider(&mut self) -> Result<(), BudgetExhaustionCause> {
        if self.remaining_provider_calls == 0 {
            return Err(BudgetExhaustionCause::ProviderLimit);
        }
        self.remaining_provider_calls -= 1;
        Ok(())
    }
    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            max_provider_calls: self.max_provider_calls,
            remaining_provider_calls: self.remaining_provider_calls,
            deadline_ms: self.deadline_ms,
        }
    }
    pub fn global_snapshot(&self) -> GlobalBudgetSnapshot {
        GlobalBudgetSnapshot {
            max_provider_calls: self.max_provider_calls,
            remaining_provider_calls: self.remaining_provider_calls,
            deadline_ms: self.deadline_ms,
        }
    }
    pub fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_never_expands() {
        let mut budget = Budget::new(1, 3_000);
        assert!(budget.reserve_provider().is_ok());
        assert_eq!(
            budget.reserve_provider(),
            Err(BudgetExhaustionCause::ProviderLimit)
        );
    }

    #[test]
    fn deep_budget_accounts_each_resource_without_expansion() {
        let mut budget = DeepBudget::new(1, 10, 2, 1, 1, 100);
        assert_eq!(budget.reserve_fetch(), Ok(10));
        assert!(budget.reserve_fetch().is_err());
        assert!(budget.consume_fetch(7, 1).is_ok());
        assert_eq!(budget.snapshot().remaining_bytes, 3);
        assert!(budget.consume_fetch(4, 0).is_err());
    }

    #[test]
    fn gap_budget_enforces_cost_and_two_subquery_hard_limit() {
        let mut budget = DeepBudget::new(1, 10, 1, 1, 1, 100).with_gap_limits(99, 2);
        assert!(budget.reserve_subquery(1).is_ok());
        assert!(budget.reserve_subquery(1).is_ok());
        assert_eq!(
            budget.reserve_subquery(0),
            Err(BudgetExhaustionCause::SubqueryLimit)
        );
        assert_eq!(budget.snapshot().remaining_subqueries, 0);

        let mut cost_limited = DeepBudget::new(1, 10, 1, 1, 1, 100).with_gap_limits(2, 1);
        assert_eq!(
            cost_limited.reserve_subquery(2),
            Err(BudgetExhaustionCause::CostLimit)
        );
    }
}
