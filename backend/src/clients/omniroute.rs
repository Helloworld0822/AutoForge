use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::{header::RETRY_AFTER, Client, Response, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;

const MAX_RESPONSE_BODY_BYTES: usize = 1024 * 1024;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
enum RetryDelay {
    Wait(Duration),
    ExceedsMaximum,
}

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
    pub temperature: Option<f64>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OmniRouteModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn complete(&self, request: AiRequest) -> Result<AiResponse>;
}

#[derive(Clone)]
pub struct OmniRouteClient {
    http: Client,
    api_key: String,
    base_url: String,
    max_retries: u8,
}

#[derive(Debug, Deserialize)]
struct OmniRouteResponse {
    model: Option<String>,
    choices: Vec<OmniRouteChoice>,
    #[serde(default)]
    usage: Option<OmniRouteUsage>,
}

#[derive(Debug, Deserialize)]
struct OmniRouteChoice {
    message: OmniRouteMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OmniRouteMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OmniRouteUsage {
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

#[derive(Debug, Deserialize)]
struct ModelListResponse {
    data: Vec<OmniRouteModelInfo>,
}

impl OmniRouteClient {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(8)
            .build()
            .map_err(Self::request_error)?;
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

    pub async fn list_models(&self) -> Result<Vec<OmniRouteModelInfo>> {
        let response = self
            .http
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(Self::request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Self::status_error("model listing", status));
        }
        let payload: ModelListResponse = Self::read_json(response, "model listing").await?;
        Ok(payload.data)
    }

    fn parse_response(response: OmniRouteResponse) -> Result<AiResponse> {
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AutoForgeError::OmniRoute("response contained no choices".into()))?;
        if choice.finish_reason.as_deref() == Some("length") {
            return Err(AutoForgeError::OmniRoute(
                "response was truncated (finish_reason: length)".into(),
            ));
        }
        let content = choice
            .message
            .content
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AutoForgeError::OmniRoute("response contained no content".into()))?;
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

    async fn complete_once(
        &self,
        request: &AiRequest,
    ) -> std::result::Result<reqwest::Response, reqwest::Error> {
        self.http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&Self::request_body(request))
            .send()
            .await
    }

    async fn read_json<T: DeserializeOwned>(response: Response, operation: &str) -> Result<T> {
        let body = Self::read_response_body(response).await?;
        serde_json::from_slice(&body).map_err(|_| {
            AutoForgeError::OmniRoute(format!("{operation} response was not valid JSON"))
        })
    }

    async fn read_response_body(response: Response) -> Result<Vec<u8>> {
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES as u64)
        {
            return Err(AutoForgeError::OmniRoute(
                "response body exceeded the maximum size".into(),
            ));
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|_| AutoForgeError::OmniRoute("failed reading response body".into()))?;
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
                return Err(AutoForgeError::OmniRoute(
                    "response body exceeded the maximum size".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn request_error(error: reqwest::Error) -> AutoForgeError {
        let summary = if error.is_timeout() {
            "request timed out"
        } else if error.is_connect() {
            "connection failed"
        } else if error.is_body() {
            "failed reading response body"
        } else if error.is_decode() {
            "failed decoding response"
        } else {
            "HTTP request failed"
        };
        AutoForgeError::OmniRoute(summary.into())
    }

    fn status_error(operation: &str, status: StatusCode) -> AutoForgeError {
        AutoForgeError::OmniRoute(format!(
            "{operation} failed ({status}): upstream returned an error status"
        ))
    }

    fn retry_delay(
        attempt: u8,
        retry_after: Option<&reqwest::header::HeaderValue>,
        now: DateTime<Utc>,
    ) -> RetryDelay {
        if let Some(delay) = retry_after
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Self::parse_retry_after(value, now))
        {
            return if delay <= MAX_RETRY_AFTER {
                RetryDelay::Wait(delay)
            } else {
                RetryDelay::ExceedsMaximum
            };
        }

        let exponent = u32::from(attempt.min(6));
        RetryDelay::Wait(Duration::from_millis(250 * 2_u64.pow(exponent)))
    }

    fn parse_retry_after(value: &str, now: DateTime<Utc>) -> Option<Duration> {
        if let Ok(seconds) = value.parse::<u64>() {
            return Some(Duration::from_secs(seconds));
        }

        let retry_at = DateTime::parse_from_rfc2822(value)
            .ok()?
            .with_timezone(&Utc);
        Some(
            retry_at
                .signed_duration_since(now)
                .to_std()
                .unwrap_or(Duration::ZERO),
        )
    }
}

#[async_trait]
impl AiProvider for OmniRouteClient {
    async fn complete(&self, request: AiRequest) -> Result<AiResponse> {
        for attempt in 0..=self.max_retries {
            let response = match self.complete_once(&request).await {
                Ok(response) => response,
                Err(error) => return Err(Self::request_error(error)),
            };
            let status = response.status();
            if status.is_success() {
                let payload = Self::read_json(response, "chat completion").await?;
                return Self::parse_response(payload);
            }
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            let retry_after = response.headers().get(RETRY_AFTER).cloned();
            if !retryable || attempt == self.max_retries {
                return Err(Self::status_error("chat completion", status));
            }
            match Self::retry_delay(attempt, retry_after.as_ref(), Utc::now()) {
                RetryDelay::Wait(delay) => tokio::time::sleep(delay).await,
                RetryDelay::ExceedsMaximum => {
                    return Err(AutoForgeError::OmniRoute(format!(
                        "chat completion failed ({status}): retry-after exceeds the maximum wait"
                    )));
                }
            };
        }
        Err(AutoForgeError::OmniRoute("retry loop exhausted".into()))
    }
}

#[cfg(test)]
#[path = "omniroute/tests.rs"]
mod tests;
