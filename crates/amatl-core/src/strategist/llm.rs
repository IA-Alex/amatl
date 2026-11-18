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

/// Hard cap on the strategist response body, mirroring the shared transport's
/// limit, so a misbehaving endpoint cannot buffer unbounded memory.
const MAX_STRATEGIST_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

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
        current_results: &SearchResponse,
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

        let current_summary = current_results
            .results
            .iter()
            .take(5)
            .map(|result| {
                let title = result
                    .title
                    .as_deref()
                    .unwrap_or_else(|| result.canonical_url.0.as_str());
                format!("- {title}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "Analiza la consulta original: '{}' y estos resultados actuales:\n{}\n\
             Sugiere una query mejorada (usando operadores avanzados como site:, filetype:, \
             inurl:) para encontrar información técnica específica. Devuelve SOLO la query \
             mejorada sin explicaciones.",
            original_query, current_summary
        );

        let request = CompletionRequest {
            model,
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
        };

        let mut response = self
            .client
            .post(endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&request)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "strategist endpoint returned status {}",
                response.status()
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_STRATEGIST_RESPONSE_BYTES {
                return Err(anyhow!(
                    "strategist response exceeded the {MAX_STRATEGIST_RESPONSE_BYTES}-byte limit"
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let parsed: CompletionResponse = serde_json::from_slice(&body)?;

        Ok(parsed
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string()))
    }
}
