use crate::domain::models::{AIError, ChatRequest, ChatResponse};
use crate::domain::traits::LlmGateway;
use async_trait::async_trait;
use reqwest::Client;

pub struct DeepSeekClient {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    client: Client,
}

impl DeepSeekClient {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        Self {
            api_key,
            base_url,
            model,
            client: Client::new(),
        }
    }

    fn normalize_url(&self) -> String {
        let mut trimmed = self.base_url.trim().to_string();

        if !trimmed.to_lowercase().starts_with("http://")
            && !trimmed.to_lowercase().starts_with("https://")
        {
            trimmed = format!("https://{}", trimmed);
        }

        let no_tail_slash = if trimmed.ends_with('/') {
            trimmed[..trimmed.len() - 1].to_string()
        } else {
            trimmed
        };

        if no_tail_slash.to_lowercase().ends_with("/chat/completions") {
            no_tail_slash
        } else {
            format!("{}/chat/completions", no_tail_slash)
        }
    }
}

#[async_trait]
impl LlmGateway for DeepSeekClient {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, AIError> {
        let url = self.normalize_url();
        let mut req_builder = self.client.post(&url);

        req_builder = req_builder.header("Authorization", format!("Bearer {}", self.api_key));

        let response = req_builder
            .json(&request)
            .send()
            .await
            .map_err(|e| AIError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AIError::NetworkError(format!("HTTP {}: {}", status, body)));
        }

        let chat_response = response.json::<ChatResponse>().await.map_err(|e| {
            eprintln!("Failed to parse LLM response: {}", e);
            AIError::ParsingFailed
        })?;

        Ok(chat_response)
    }

    fn get_model_name(&self) -> String {
        self.model.clone()
    }

    fn get_temperature(&self) -> f64 {
        let is_moonshot = self.base_url.to_lowercase().contains("api.moonshot.cn");

        if is_moonshot {
            1.0
        } else {
            0.1
        }
    }
}
