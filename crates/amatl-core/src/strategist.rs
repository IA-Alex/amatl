pub mod llm;

use crate::model::SearchResponse;
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait SearchStrategist: Send + Sync {
    /// Analiza los resultados actuales y decide si se requiere una nueva consulta más profunda.
    /// Retorna un `Option<String>` con la nueva query optimizada si es necesario.
    async fn generate_strategy(
        &self,
        original_query: &str,
        current_results: &SearchResponse,
    ) -> Result<Option<String>>;
}

pub struct NoOpStrategist;

#[async_trait]
impl SearchStrategist for NoOpStrategist {
    async fn generate_strategy(
        &self,
        _original_query: &str,
        _current_results: &SearchResponse,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}
