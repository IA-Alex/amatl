use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TimeoutConfig {
    pub provider_ms: u64,
    pub global_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BudgetConfig {
    pub max_provider_calls: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionConfig {
    pub global_concurrency: usize,
    pub per_provider_concurrency: usize,
    pub max_retries: u32,
    pub retry_jitter_ms: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            provider_ms: 3_000,
            global_ms: 8_000,
        }
    }
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_provider_calls: 3,
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            global_concurrency: 4,
            per_provider_concurrency: 1,
            max_retries: 1,
            retry_jitter_ms: 25,
        }
    }
}

impl Config {
    /// Parallel-execution limits and the circuit-breaker sub-policy.
    pub(super) fn validate_execution(&self) -> Result<(), ConfigError> {
        if self.execution.global_concurrency == 0
            || self.execution.per_provider_concurrency == 0
            || self.execution.per_provider_concurrency > self.execution.global_concurrency
            || self.execution.max_retries > 2
            || self.execution.retry_jitter_ms > 1_000
        {
            return Err(ConfigError::Policy(
                "invalid parallel search execution limit".into(),
            ));
        }
        self.circuit_breaker
            .validate()
            .map_err(|error| ConfigError::Policy(error.into()))?;
        Ok(())
    }
}
