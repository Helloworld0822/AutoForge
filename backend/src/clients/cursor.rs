use crate::domain::{AgentMode, ModelProfile};
use crate::error::{AutoForgeError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::instrument;

const CURSOR_API_BASE: &str = "https://api.cursor.com";

#[derive(Clone)]
pub struct CursorClient {
    http: Client,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct CreateAgentRequest<'a> {
    prompt: Prompt<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<ModelSelection<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repos: Option<Vec<RepoConfig<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_create_pr: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct Prompt<'a> {
    text: &'a str,
}

#[derive(Debug, Serialize)]
struct ModelSelection<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Vec<ModelParam<'a>>>,
}

#[derive(Debug, Serialize)]
struct ModelParam<'a> {
    id: &'a str,
    value: &'a str,
}

#[derive(Debug, Serialize)]
struct RepoConfig<'a> {
    url: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    starting_ref: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentResponse {
    pub agent: AgentInfo,
    pub run: RunInfo,
}

#[derive(Debug, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub status: String,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RunInfo {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct GetRunResponse {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub result: Option<RunResult>,
}

#[derive(Debug, Deserialize)]
pub struct RunResult {
    pub text: Option<String>,
    pub git: Option<GitResult>,
}

#[derive(Debug, Deserialize)]
pub struct GitResult {
    pub branches: Option<Vec<GitBranch>>,
}

#[derive(Debug, Deserialize)]
pub struct GitBranch {
    pub name: Option<String>,
    pub pr_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CursorModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

impl CursorClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(8)
            .build()
            .map_err(|e| AutoForgeError::CursorApi(e.to_string()))?;

        Ok(Self {
            http,
            api_key: api_key.into(),
        })
    }

    #[instrument(skip(self, prompt_text, opts), fields(model = %profile.model_id))]
    pub async fn create_agent(
        &self,
        prompt_text: &str,
        profile: &ModelProfile,
        opts: CreateAgentOpts<'_>,
    ) -> Result<CreateAgentResponse> {
        let mode = match profile.mode {
            AgentMode::Agent => "agent",
            AgentMode::Plan => "plan",
        };

        let model = ModelSelection {
            id: &profile.model_id,
            params: None,
        };

        let repos = opts.repo_url.map(|url| {
            vec![RepoConfig {
                url,
                starting_ref: opts.starting_ref,
            }]
        });

        let body = CreateAgentRequest {
            prompt: Prompt { text: prompt_text },
            model: Some(model),
            mode: Some(mode),
            repos,
            auto_create_pr: opts.auto_create_pr,
            agent_id: opts.agent_id.map(String::from),
        };

        let resp = self
            .http
            .post(format!("{CURSOR_API_BASE}/v1/agents"))
            .basic_auth(&self.api_key, Some(""))
            .json(&body)
            .send()
            .await
            .map_err(|e| AutoForgeError::CursorApi(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AutoForgeError::CursorApi(format!(
                "create agent failed ({status}): {body}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| AutoForgeError::CursorApi(e.to_string()))
    }

    pub async fn get_run(&self, agent_id: &str, run_id: &str) -> Result<GetRunResponse> {
        let resp = self
            .http
            .get(format!(
                "{CURSOR_API_BASE}/v1/agents/{agent_id}/runs/{run_id}"
            ))
            .basic_auth(&self.api_key, Some(""))
            .send()
            .await
            .map_err(|e| AutoForgeError::CursorApi(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AutoForgeError::CursorApi(format!(
                "get run failed ({status}): {body}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| AutoForgeError::CursorApi(e.to_string()))
    }

    pub async fn wait_for_run(
        &self,
        agent_id: &str,
        run_id: &str,
        poll_interval: Duration,
    ) -> Result<GetRunResponse> {
        loop {
            let run = self.get_run(agent_id, run_id).await?;
            match run.status.as_str() {
                "COMPLETED" | "FAILED" | "CANCELLED" => return Ok(run),
                _ => tokio::time::sleep(poll_interval).await,
            }
        }
    }

    /// Cursor Cloud Agents API 연결 확인 (GET /v1/models)
    pub async fn health_check(&self) -> std::result::Result<(), String> {
        let resp = self
            .http
            .get(format!("{CURSOR_API_BASE}/v1/models"))
            .basic_auth(&self.api_key, Some(""))
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(format!("HTTP {status}: {body}"))
        }
    }

    /// 사용 가능한 Cursor 모델 목록
    pub async fn list_models(&self) -> Result<Vec<CursorModelInfo>> {
        let resp = self
            .http
            .get(format!("{CURSOR_API_BASE}/v1/models"))
            .basic_auth(&self.api_key, Some(""))
            .send()
            .await
            .map_err(|e| AutoForgeError::CursorApi(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AutoForgeError::CursorApi(format!(
                "list models failed ({status}): {body}"
            )));
        }

        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AutoForgeError::CursorApi(e.to_string()))?;

        let models = if let Some(arr) = value.get("models").and_then(|v| v.as_array()) {
            parse_model_array(arr)
        } else if let Some(arr) = value.as_array() {
            parse_model_array(arr)
        } else {
            vec![]
        };

        if models.is_empty() {
            Ok(Self::fallback_models())
        } else {
            Ok(models)
        }
    }

    pub fn fallback_models() -> Vec<CursorModelInfo> {
        [
            ("claude-haiku-4-5", "Claude Haiku 4.5"),
            ("claude-4.6-sonnet-high-thinking", "Claude Sonnet 4.6"),
            ("claude-sonnet-5-thinking-high", "Claude Sonnet 5"),
            ("claude-fable-5-thinking-high", "Claude Fable 5"),
            ("gpt-5.3-codex-high", "GPT-5.3 Codex"),
            ("gpt-5.5-medium", "GPT-5.5"),
            ("composer-2.5", "Composer 2.5"),
        ]
        .into_iter()
        .map(|(id, name)| CursorModelInfo {
            id: id.into(),
            name: Some(name.into()),
        })
        .collect()
    }
}

fn parse_model_array(arr: &[serde_json::Value]) -> Vec<CursorModelInfo> {
    arr.iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("modelId"))
                .and_then(|v| v.as_str())?;
            let name = item
                .get("name")
                .or_else(|| item.get("displayName"))
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(CursorModelInfo {
                id: id.to_string(),
                name,
            })
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct CreateAgentOpts<'a> {
    pub repo_url: Option<&'a str>,
    pub starting_ref: Option<&'a str>,
    pub auto_create_pr: Option<bool>,
    pub agent_id: Option<&'a str>,
}
