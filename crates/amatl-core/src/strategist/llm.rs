use crate::config::StrategistConfig;
use crate::model::SearchResponse;
use crate::strategist::SearchStrategist;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    Client,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub struct LlmStrategist {
    client: Client,
    config: StrategistConfig,
}

impl LlmStrategist {
    pub fn new(config: StrategistConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()?;
        Ok(Self { client, config })
    }
}

#[derive(Serialize)]
struct CompletionRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: String,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Deserialize)]
struct MessageContent {
    content: String,
}

#[async_trait]
impl SearchStrategist for LlmStrategist {
    async fn generate_strategy(
        &self,
        original_query: &str,
        _current_results: &SearchResponse,
    ) -> Result<Option<String>> {
        let endpoint = self
            .config
            .endpoint
            .as_ref()
            .ok_or_else(|| anyhow!("No endpoint configured"))?;
        let model = self
            .config
            .model
            .as_ref()
            .ok_or_else(|| anyhow!("No model configured"))?;

        let api_key_env = self
            .config
            .credential_env
            .as_ref()
            .ok_or_else(|| anyhow!("no credential_env configured for the strategist"))?;
        let api_key = std::env::var(api_key_env)
            .map_err(|_| anyhow!("API key not found in environment variable {api_key_env}"))?;

        let prompt = format!(
            "Analiza la consulta original: '{}' y sugiere una query mejorada (usando operadores avanzados como site:, filetype:, inurl:) para encontrar información técnica específica. Devuelve SOLO la query mejorada sin explicaciones.", 
            original_query
        );

        let request = CompletionRequest {
            model,
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
        };

        let response: CompletionResponse = self
            .client
            .post(endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        Ok(response
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string()))
    }
}
