use crate::domain::ModelProfile;
use crate::error::{AutoForgeError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{process::Command, sync::Mutex};
use tracing::instrument;

#[derive(Clone)]
pub struct OmniRouteClient {
    http: Client,
    base_url: String,
    api_key: String,
    github_token: Option<String>,
    workspace_root: PathBuf,
    completed: std::sync::Arc<Mutex<HashMap<String, GetRunResponse>>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAgentResponse {
    pub agent: AgentInfo,
    pub run: RunInfo,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub status: String,
    pub url: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct RunInfo {
    pub id: String,
    pub status: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GetRunResponse {
    pub id: String,
    pub status: String,
    pub result: Option<RunResult>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunResult {
    pub text: Option<String>,
    pub git: Option<GitResult>,
}
impl GetRunResponse {
    pub fn result_text(&self) -> Option<String> {
        self.result.as_ref().and_then(|r| r.text.clone())
    }
    pub fn is_terminal(&self) -> bool {
        true
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitResult {
    pub branches: Option<Vec<GitBranch>>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitBranch {
    pub name: Option<String>,
    pub pr_url: Option<String>,
}

impl OmniRouteClient {
    pub fn new(
        base_url: String,
        api_key: String,
        github_token: Option<String>,
        artifacts_dir: &str,
    ) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(900))
            .build()
            .map_err(|e| AutoForgeError::OmniRouteApi(e.to_string()))?;
        let workspace_root = PathBuf::from(artifacts_dir).join("omniroute-workspaces");
        std::fs::create_dir_all(&workspace_root)
            .map_err(|e| AutoForgeError::Internal(format!("workspace directory: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            github_token,
            workspace_root,
            completed: Default::default(),
        })
    }

    #[instrument(skip(self, prompt_text, opts), fields(model = %profile.model_id))]
    pub async fn create_agent(
        &self,
        prompt_text: &str,
        profile: &ModelProfile,
        opts: CreateAgentOpts<'_>,
    ) -> Result<CreateAgentResponse> {
        let agent_id = uuid::Uuid::new_v4().to_string();
        let run_id = uuid::Uuid::new_v4().to_string();
        let (text, branch, pr_url) = if let Some(repo_url) = opts.repo_url {
            let target_branch = if let Some(pr_url) = opts.pr_url {
                Some(self.pull_request_branch(pr_url).await?)
            } else {
                None
            };
            let start_ref = target_branch
                .as_deref()
                .or(opts.starting_ref)
                .unwrap_or("main");
            let (text, branch) = self
                .run_codex(
                    repo_url,
                    start_ref,
                    target_branch.as_deref(),
                    prompt_text,
                    &profile.model_id,
                    &agent_id,
                )
                .await?;
            let pr = if opts.auto_create_pr == Some(true) {
                Some(
                    self.create_pull_request(repo_url, &branch, prompt_text)
                        .await?,
                )
            } else {
                None
            };
            (text, Some(branch), pr)
        } else {
            (self.chat(prompt_text, profile).await?, None, None)
        };
        let result = GetRunResponse {
            id: run_id.clone(),
            status: "COMPLETED".into(),
            result: Some(RunResult {
                text: Some(text),
                git: branch.map(|name| GitResult {
                    branches: Some(vec![GitBranch {
                        name: Some(name),
                        pr_url,
                    }]),
                }),
            }),
        };
        self.completed.lock().await.insert(run_id.clone(), result);
        Ok(CreateAgentResponse {
            agent: AgentInfo {
                id: agent_id,
                status: "COMPLETED".into(),
                url: None,
            },
            run: RunInfo {
                id: run_id,
                status: "COMPLETED".into(),
            },
        })
    }

    async fn chat(&self, prompt: &str, profile: &ModelProfile) -> Result<String> {
        let response = self.http.post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key).json(&serde_json::json!({"model":profile.model_id,"messages":[{"role":"user","content":prompt}],"stream":false}))
            .send().await.map_err(|e| AutoForgeError::OmniRouteApi(e.to_string()))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AutoForgeError::OmniRouteApi(format!(
                "chat completion failed ({status}): {body}"
            )));
        }
        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| AutoForgeError::OmniRouteApi(e.to_string()))?;
        value
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| {
                AutoForgeError::OmniRouteApi("OmniRoute returned an empty completion".into())
            })
    }

    async fn run_codex(
        &self,
        repo_url: &str,
        starting_ref: &str,
        target_branch: Option<&str>,
        prompt: &str,
        model: &str,
        id: &str,
    ) -> Result<(String, String)> {
        let root = self.workspace_root.join(id);
        let home = root.join(".codex-home");
        let repo = root.join("repo");
        tokio::fs::create_dir_all(&home).await.map_err(internal)?;
        let api_base =
            std::env::var("OMNIROUTE_CODEX_BASE_URL").unwrap_or_else(|_| self.base_url.clone());
        let config = format!(
            "model = {model:?}\nmodel_provider = \"omniroute\"\nmodel_reasoning_effort = \"high\"\n\n[model_providers.omniroute]\nname = \"OmniRoute\"\nbase_url = {api_base:?}\nenv_key = \"OMNIROUTE_API_KEY\"\nrequires_openai_auth = false\nwire_api = \"responses\"\n"
        );
        tokio::fs::write(home.join("config.toml"), config)
            .await
            .map_err(internal)?;
        let branch = target_branch
            .map(str::to_owned)
            .unwrap_or_else(|| format!("autoforge/{}", &id[..8]));
        let create_branch = target_branch.is_none();
        let repo_url = repo_url.to_owned();
        let start_ref = starting_ref.to_owned();
        let token = self.github_token.clone();
        let repo_clone = repo.clone();
        let checkout_branch = branch.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = std::process::Command::new("git");
            if let Some(t) = token.as_deref() {
                c.args(["-c", &format!("http.extraheader=AUTHORIZATION: Bearer {t}")]);
            }
            c.args([
                "clone",
                "--branch",
                &start_ref,
                "--single-branch",
                &repo_url,
                repo_clone.to_str().unwrap_or_default(),
            ]);
            let out = c.output().map_err(internal)?;
            if !out.status.success() {
                return Err(AutoForgeError::OmniRouteApi(format!(
                    "git clone failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                )));
            }
            if create_branch {
                let mut c = std::process::Command::new("git");
                c.current_dir(&repo_clone)
                    .args(["checkout", "-b", &checkout_branch]);
                let out = c.output().map_err(internal)?;
                if !out.status.success() {
                    return Err(AutoForgeError::OmniRouteApi(
                        String::from_utf8_lossy(&out.stderr).into_owned(),
                    ));
                }
            }
            Ok::<_, AutoForgeError>(())
        })
        .await
        .map_err(internal)??;
        let output = Command::new("codex")
            .args(["exec", "--full-auto", "--model", model, "--cd"])
            .arg(&repo)
            .arg(prompt)
            .env("CODEX_HOME", &home)
            .env("OMNIROUTE_API_KEY", &self.api_key)
            .env("OMNIROUTE_BASE_URL", api_base)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AutoForgeError::OmniRouteApi(format!("Codex CLI unavailable: {e}")))?;
        if !output.status.success() {
            return Err(AutoForgeError::OmniRouteApi(format!(
                "Codex task failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let message = String::from_utf8_lossy(&output.stdout).into_owned();
        let repo_push = repo.clone();
        let branch_push = branch.clone();
        let token = self.github_token.clone();
        tokio::task::spawn_blocking(move || {
            let run = |args: &[&str]| -> Result<std::process::Output> {
                let mut c = std::process::Command::new("git");
                c.current_dir(&repo_push);
                if let Some(t) = token.as_deref() {
                    c.args(["-c", &format!("http.extraheader=AUTHORIZATION: Bearer {t}")]);
                }
                c.args(args);
                c.output().map_err(internal)
            };
            let changed = run(&["status", "--porcelain"])?;
            if !String::from_utf8_lossy(&changed.stdout).trim().is_empty() {
                let _ = run(&["add", "-A"])?;
                let _ = run(&[
                    "-c",
                    "user.name=AutoForge",
                    "-c",
                    "user.email=autoforge@localhost",
                    "commit",
                    "-m",
                    "AutoForge implementation",
                ])?;
                let pushed = run(&["push", "origin", &branch_push])?;
                if !pushed.status.success() {
                    return Err(AutoForgeError::OmniRouteApi(format!(
                        "git push failed: {}",
                        String::from_utf8_lossy(&pushed.stderr)
                    )));
                }
            }
            Ok::<_, AutoForgeError>(())
        })
        .await
        .map_err(internal)??;
        Ok((message, branch))
    }

    async fn create_pull_request(
        &self,
        repo_url: &str,
        branch: &str,
        prompt: &str,
    ) -> Result<String> {
        let token = self.github_token.as_ref().ok_or_else(|| {
            AutoForgeError::BadRequest("GITHUB_TOKEN required to create PR".into())
        })?;
        let (owner, repo) = crate::clients::github::GitHubClient::parse_repo_url(repo_url)
            .ok_or_else(|| AutoForgeError::BadRequest("invalid GitHub repository URL".into()))?;
        let response=self.http.post(format!("https://api.github.com/repos/{owner}/{repo}/pulls")).bearer_auth(token)
            .header("Accept","application/vnd.github+json").header("User-Agent","AutoForge")
            .json(&serde_json::json!({"title":"AutoForge implementation","head":branch,"base":"main","body":prompt.chars().take(4000).collect::<String>()})).send().await.map_err(internal)?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.map_err(internal)?;
        if !status.is_success() {
            return Err(AutoForgeError::OmniRouteApi(format!(
                "GitHub PR creation failed ({status}): {value}"
            )));
        }
        value
            .get("html_url")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| AutoForgeError::OmniRouteApi("GitHub omitted PR URL".into()))
    }

    async fn pull_request_branch(&self, pr_url: &str) -> Result<String> {
        let token = self.github_token.as_ref().ok_or_else(|| {
            AutoForgeError::BadRequest("GITHUB_TOKEN required to update PR branch".into())
        })?;
        let (owner, repo, number) = crate::clients::github::GitHubClient::parse_pr_url(pr_url)
            .ok_or_else(|| AutoForgeError::BadRequest("invalid pull request URL".into()))?;
        let response = self
            .http
            .get(format!(
                "https://api.github.com/repos/{owner}/{repo}/pulls/{number}"
            ))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "AutoForge")
            .send()
            .await
            .map_err(internal)?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.map_err(internal)?;
        if !status.is_success() {
            return Err(AutoForgeError::OmniRouteApi(format!(
                "GitHub PR lookup failed ({status}): {value}"
            )));
        }
        value
            .pointer("/head/ref")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| AutoForgeError::OmniRouteApi("GitHub PR omitted head branch".into()))
    }

    pub async fn wait_for_run(
        &self,
        _agent: &str,
        run: &str,
        _poll: Duration,
    ) -> Result<GetRunResponse> {
        self.completed.lock().await.remove(run).ok_or_else(|| {
            AutoForgeError::OmniRouteApi("completed OmniRoute result expired".into())
        })
    }
    pub async fn health_check(&self) -> std::result::Result<(), String> {
        let r = self
            .http
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "HTTP {}: {}",
                r.status(),
                r.text().await.unwrap_or_default()
            ))
        }
    }
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let r = self
            .http
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(internal)?;
        let status = r.status();
        let body = r.text().await.map_err(internal)?;
        if !status.is_success() {
            return Err(AutoForgeError::OmniRouteApi(format!(
                "models failed ({status}): {body}"
            )));
        }
        let value: serde_json::Value = serde_json::from_str(&body).map_err(internal)?;
        let rows = value
            .get("data")
            .or_else(|| value.get("models"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut models: Vec<ModelInfo> = rows
            .into_iter()
            .filter_map(|v| {
                Some(ModelInfo {
                    id: v.get("id")?.as_str()?.to_owned(),
                    name: v.get("name").and_then(|n| n.as_str()).map(str::to_owned),
                })
            })
            .filter(|m| !m.id.starts_with("cursor/") && !m.id.starts_with("cu/"))
            .collect();
        if models.is_empty() {
            models = Self::fallback_models();
        }
        Ok(models)
    }
    pub fn fallback_models() -> Vec<ModelInfo> {
        [
            ("cx/gpt-5.5", "GPT-5.5 Codex"),
            ("cx/gpt-5.4", "GPT-5.4 Codex"),
            ("claude-sonnet-5", "Claude Sonnet 5"),
            ("claude-opus-5-5", "Claude Opus 5.5"),
            ("deepseek-v4-pro", "DeepSeek V4 Pro"),
            ("kimi-k3", "Kimi K3"),
            ("qwen3.8-max", "Qwen 3.8 Max"),
            ("glm-5.2", "GLM 5.2"),
        ]
        .into_iter()
        .map(|(id, name)| ModelInfo {
            id: id.into(),
            name: Some(name.into()),
        })
        .collect()
    }
}
fn internal(e: impl std::fmt::Display) -> AutoForgeError {
    AutoForgeError::Internal(e.to_string())
}
#[derive(Debug, Default)]
pub struct CreateAgentOpts<'a> {
    pub repo_url: Option<&'a str>,
    pub starting_ref: Option<&'a str>,
    pub auto_create_pr: Option<bool>,
    pub agent_id: Option<&'a str>,
    pub pr_url: Option<&'a str>,
}
