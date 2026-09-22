use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AiRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiMessage {
    pub role: AiRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRequest {
    pub model: String,
    pub messages: Vec<AiMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormat {
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiResponse {
    pub model: String,
    pub content: String,
    pub usage: TokenUsage,
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn complete(&self, request: AiRequest) -> Result<AiResponse>;
}

#[derive(Clone)]
pub struct OpenRouterClient {
    http: Client,
    api_key: String,
    base_url: String,
    max_retries: u8,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    model: Option<String>,
    choices: Vec<OpenRouterChoice>,
    #[serde(default)]
    usage: Option<OpenRouterUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    message: OpenRouterMessage,
}

#[derive(Debug, Deserialize)]
struct OpenRouterMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    #[serde(default)]
    prompt_tokens_details: Option<TokenDetails>,
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TokenDetails {
    cached_tokens: Option<u64>,
}

impl OpenRouterClient {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(8)
            .build()
            .map_err(|error| AutoForgeError::OpenRouter(error.to_string()))?;
        Ok(Self {
            http,
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            max_retries: 3,
        })
    }

    fn request_body(request: &AiRequest) -> serde_json::Value {
        let messages = request
            .messages
            .iter()
            .map(|message| {
                serde_json::json!({
                    "role": match message.role {
                        AiRole::System => "system",
                        AiRole::User => "user",
                        AiRole::Assistant => "assistant",
                    },
                    "content": message.content,
                })
            })
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages,
        });
        if let Some(value) = request.temperature {
            body["temperature"] = serde_json::json!(value);
        }
        if let Some(value) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(value);
        }
        if let Some(format) = &request.response_format {
            body["response_format"] = serde_json::json!({ "type": format.kind });
        }
        body
    }

    fn parse_response(response: OpenRouterResponse) -> Result<AiResponse> {
        let choice =
            response.choices.into_iter().next().ok_or_else(|| {
                AutoForgeError::OpenRouter("response contained no choices".into())
            })?;
        let content = choice
            .message
            .content
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AutoForgeError::OpenRouter("response contained no content".into()))?;
        let usage = response
            .usage
            .map_or_else(TokenUsage::default, |value| TokenUsage {
                input_tokens: value.prompt_tokens.unwrap_or_default(),
                output_tokens: value.completion_tokens.unwrap_or_default(),
                cached_tokens: value
                    .prompt_tokens_details
                    .and_then(|details| details.cached_tokens)
                    .unwrap_or_default(),
                cost_usd: value.cost,
            });
        Ok(AiResponse {
            model: response.model.unwrap_or_default(),
            content,
            usage,
        })
    }

    async fn complete_once(&self, request: &AiRequest) -> Result<reqwest::Response> {
        self.http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .header(
                "HTTP-Referer",
                "https://github.com/helloworld0822/AutoForge",
            )
            .header("X-Title", "AutoForge")
            .json(&Self::request_body(request))
            .send()
            .await
            .map_err(|error| AutoForgeError::OpenRouter(error.to_string()))
    }
}

#[async_trait]
impl AiProvider for OpenRouterClient {
    async fn complete(&self, request: AiRequest) -> Result<AiResponse> {
        for attempt in 0..=self.max_retries {
            let response = self.complete_once(&request).await?;
            let status = response.status();
            if status.is_success() {
                let payload = response
                    .json::<OpenRouterResponse>()
                    .await
                    .map_err(|error| AutoForgeError::OpenRouter(error.to_string()))?;
                return Self::parse_response(payload);
            }
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            let body = response.text().await.unwrap_or_default();
            if !retryable || attempt == self.max_retries {
                return Err(AutoForgeError::OpenRouter(format!(
                    "chat completion failed ({status}): {body}"
                )));
            }
            tokio::time::sleep(Duration::from_millis(250 * 2_u64.pow(attempt.into()))).await;
        }
        Err(AutoForgeError::OpenRouter("retry loop exhausted".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_and_cached_tokens() {
        let response = serde_json::from_value::<OpenRouterResponse>(serde_json::json!({
            "model": "test/model",
            "choices": [{"message": {"content": "{}"}}],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 20,
                "prompt_tokens_details": {"cached_tokens": 40},
                "cost": 0.12
            }
        }))
        .expect("fixture is valid");
        let parsed = OpenRouterClient::parse_response(response).expect("response parses");
        assert_eq!(parsed.usage.input_tokens, 100);
        assert_eq!(parsed.usage.cached_tokens, 40);
        assert_eq!(parsed.usage.cost_usd, Some(0.12));
    }

    #[test]
    fn builds_structured_json_request() {
        let request = AiRequest {
            model: "test/model".into(),
            messages: vec![AiMessage {
                role: AiRole::User,
                content: "extract".into(),
            }],
            temperature: Some(0.1),
            max_tokens: Some(100),
            response_format: Some(ResponseFormat {
                kind: "json_object".into(),
            }),
        };
        let body = OpenRouterClient::request_body(&request);
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["messages"][0]["role"], "user");
    }
}
