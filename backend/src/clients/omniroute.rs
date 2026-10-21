use crate::domain::{ArtifactRef, ModelProfile};
use crate::error::{AutoForgeError, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex};
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
        let workspace_root = std::fs::canonicalize(workspace_root).map_err(internal)?;
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
        let context =
            input_context(self.workspace_root.parent().unwrap(), opts.input_artifacts).await?;
        let prompt_text = format!("{prompt_text}\n\n{context}");
        let prompt_text = prompt_text.as_str();
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
        let api_base = std::env::var("OMNIROUTE_CODEX_BASE_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
            .unwrap_or_else(|| self.base_url.clone());
        let config = format!(
            "model = {model:?}\nmodel_provider = \"omniroute\"\nmodel_reasoning_effort = \"high\"\n\n[sandbox_workspace_write]\nnetwork_access = true\n\n[model_providers.omniroute]\nname = \"OmniRoute\"\nbase_url = {api_base:?}\nenv_key = \"OMNIROUTE_API_KEY\"\nrequires_openai_auth = false\nwire_api = \"responses\"\n"
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
            let mut c = git_command(token.as_deref());
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
        let mut command = Command::new("codex");
        command
            .args(["exec", "--full-auto", "--model", model, "--cd"])
            .arg(&repo)
            .arg("-")
            .stdin(Stdio::piped())
            .env("CODEX_HOME", &home)
            .env("OMNIROUTE_API_KEY", &self.api_key)
            .env("OMNIROUTE_BASE_URL", api_base)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| AutoForgeError::OmniRouteApi(format!("Codex CLI unavailable: {e}")))?;
        let output = tokio::time::timeout(Duration::from_secs(900), async {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| internal("Codex stdin unavailable"))?;
            stdin.write_all(prompt.as_bytes()).await.map_err(internal)?;
            drop(stdin);
            child.wait_with_output().await.map_err(internal)
        })
        .await
        .map_err(|_| AutoForgeError::OmniRouteApi("Codex task timed out after 900s".into()))??;
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
            publish_repository(&repo_push, &branch_push, token.as_deref())
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
// Chat completions cannot fetch artifact URLs. Include text content directly;
// repository agents can also read binary inputs at their absolute local paths.
async fn input_context(root: &std::path::Path, artifacts: &[ArtifactRef]) -> Result<String> {
    let root = tokio::fs::canonicalize(root).await.map_err(internal)?;
    let mut context = String::from("Input artifacts (local paths and text contents):\n");
    for artifact in artifacts {
        let key = std::path::Path::new(&artifact.key);
        if key.is_absolute()
            || key.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(AutoForgeError::BadRequest(
                "invalid input artifact key".into(),
            ));
        }
        let path = tokio::fs::canonicalize(root.join(key))
            .await
            .map_err(internal)?;
        if !path.starts_with(&root) {
            return Err(AutoForgeError::BadRequest(
                "input artifact escapes storage root".into(),
            ));
        }
        context.push_str(&format!(
            "\nArtifact: {}\nLocal file: {}\n",
            artifact.name,
            path.display()
        ));
        if artifact.content_type.starts_with("text/") || artifact.content_type == "application/json"
        {
            let content = tokio::fs::read_to_string(&path).await.map_err(internal)?;
            context.push_str(&content);
            context.push('\n');
        }
    }
    Ok(context)
}

// GitHub Git-over-HTTPS uses Basic auth with a token as the password.
// Scope the header to GitHub and keep credentials out of command arguments and
// the persisted repository configuration.
fn git_command(token: Option<&str>) -> std::process::Command {
    let mut command = std::process::Command::new("git");
    command.env("GIT_TERMINAL_PROMPT", "0");
    if let Some(token) = token {
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!(
                    "AUTHORIZATION: Basic {}",
                    STANDARD.encode(format!("x-access-token:{token}"))
                ),
            );
    }
    command
}

fn publish_repository(repo: &std::path::Path, branch: &str, token: Option<&str>) -> Result<()> {
    let run = |args: &[&str]| -> Result<std::process::Output> {
        let output = git_command(token)
            .current_dir(repo)
            .args(args)
            .output()
            .map_err(internal)?;
        if !output.status.success() {
            return Err(AutoForgeError::OmniRouteApi(format!(
                "git {} failed: {}",
                args.first().unwrap_or(&"command"),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(output)
    };
    let changed = run(&["status", "--porcelain"])?;
    if !changed.stdout.is_empty() {
        run(&["add", "-A"])?;
        run(&[
            "-c",
            "user.name=AutoForge",
            "-c",
            "user.email=autoforge@localhost",
            "commit",
            "-m",
            "AutoForge implementation",
        ])?;
    }
    // Codex may have committed everything already. A clean working tree does
    // not mean the remote branch exists or contains the agent's commits.
    run(&["push", "origin", branch])?;
    Ok(())
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
    pub input_artifacts: &'a [ArtifactRef],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    struct Repo {
        root: PathBuf,
        local: PathBuf,
        remote: PathBuf,
    }
    impl Repo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("autoforge-git-{}", uuid::Uuid::new_v4()));
            let local = root.join("local");
            let remote = root.join("remote.git");
            std::fs::create_dir_all(&local).unwrap();
            git(&root, &["init", "--bare", remote.to_str().unwrap()]);
            git(&local, &["init", "-b", "main"]);
            git(&local, &["config", "user.name", "Test"]);
            git(&local, &["config", "user.email", "test@example.invalid"]);
            std::fs::write(local.join("README.md"), "initial").unwrap();
            git(&local, &["add", "README.md"]);
            git(&local, &["commit", "-m", "initial"]);
            git(
                &local,
                &["remote", "add", "origin", remote.to_str().unwrap()],
            );
            git(&local, &["push", "origin", "main"]);
            git(&local, &["checkout", "-b", "autoforge/test"]);
            Self {
                root,
                local,
                remote,
            }
        }
        fn assert_published(&self) {
            assert_eq!(
                git(&self.local, &["rev-parse", "HEAD"]),
                git(&self.remote, &["rev-parse", "refs/heads/autoforge/test"])
            );
            assert!(git(&self.local, &["status", "--porcelain"]).is_empty());
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn publishes_commits_already_created_by_codex() {
        let repo = Repo::new();
        std::fs::write(repo.local.join("implementation.txt"), "done").unwrap();
        git(&repo.local, &["add", "implementation.txt"]);
        git(&repo.local, &["commit", "-m", "agent commit"]);
        assert!(git(&repo.local, &["status", "--porcelain"]).is_empty());
        publish_repository(&repo.local, "autoforge/test", None).unwrap();
        repo.assert_published();
        // Verify/read-only stages can publish an unchanged existing branch.
        publish_repository(&repo.local, "autoforge/test", None).unwrap();
        repo.assert_published();
    }

    #[test]
    fn commits_and_publishes_uncommitted_agent_changes() {
        let repo = Repo::new();
        std::fs::write(repo.local.join("implementation.txt"), "done").unwrap();
        publish_repository(&repo.local, "autoforge/test", None).unwrap();
        repo.assert_published();
        assert_eq!(
            git(&repo.remote, &["show", "autoforge/test:implementation.txt"]),
            "done"
        );
    }

    #[cfg(unix)]
    #[test]
    fn commit_failure_is_reported_before_push() {
        use std::os::unix::fs::PermissionsExt;
        let repo = Repo::new();
        let hook = repo.local.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(repo.local.join("implementation.txt"), "done").unwrap();
        assert!(publish_repository(&repo.local, "autoforge/test", None).is_err());
        assert!(
            !git(&repo.remote, &["for-each-ref", "--format=%(refname)"]).contains("autoforge/test")
        );
    }

    #[test]
    fn git_auth_is_scoped_and_not_persisted() {
        let repo = Repo::new();
        let output = git_command(Some("test-token"))
            .current_dir(&repo.local)
            .args([
                "config",
                "--get-urlmatch",
                "http.extraheader",
                "https://github.com/owner/repo.git",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!(
                "AUTHORIZATION: Basic {}",
                STANDARD.encode("x-access-token:test-token")
            )
        );
        let output = git_command(Some("test-token"))
            .current_dir(&repo.local)
            .args([
                "config",
                "--get-urlmatch",
                "http.extraheader",
                "https://example.invalid/repo.git",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!git(&repo.local, &["config", "--local", "--list"]).contains("test-token"));
    }

    #[test]
    fn stage_defaults_belong_to_omniroute_catalog() {
        use crate::domain::StageId;
        let models = OmniRouteClient::fallback_models();
        for stage in [
            StageId::Summarize,
            StageId::Architect,
            StageId::Implement,
            StageId::Verify,
            StageId::Debug,
            StageId::SecurityPatch,
        ] {
            let profile = ModelProfile::for_stage(stage);
            assert!(models.iter().any(|model| model.id == profile.model_id));
            if matches!(
                stage,
                StageId::Implement | StageId::Verify | StageId::Debug | StageId::SecurityPatch
            ) {
                assert!(profile.model_id.starts_with("cx/"));
            }
        }
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    fn artifact(key: &str, content_type: &str) -> ArtifactRef {
        ArtifactRef {
            name: key.into(),
            key: key.into(),
            uri: format!("http://unreachable/artifacts/{key}"),
            content_type: content_type.into(),
            sha256: None,
        }
    }

    #[tokio::test]
    async fn includes_document_body_and_binary_local_path() {
        let root = std::env::temp_dir().join(format!("autoforge-input-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(
            root.join("raw_text.md"),
            "Unique customer requirement: offline reporting",
        )
        .await
        .unwrap();
        tokio::fs::write(root.join("screen.png"), [137, 80, 78, 71])
            .await
            .unwrap();
        let context = input_context(
            &root,
            &[
                artifact("raw_text.md", "text/markdown"),
                artifact("screen.png", "image/png"),
            ],
        )
        .await
        .unwrap();
        assert!(context.contains("Unique customer requirement: offline reporting"));
        assert!(context.contains(root.join("screen.png").to_str().unwrap()));
        assert!(!context.contains("http://unreachable"));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_input_path_traversal() {
        let root = std::env::temp_dir();
        assert!(
            input_context(&root, &[artifact("../outside.txt", "text/plain")])
                .await
                .is_err()
        );
        assert!(
            input_context(&root, &[artifact("/outside.txt", "text/plain")])
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_outside_storage() {
        let root = std::env::temp_dir().join(format!("autoforge-input-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        std::os::unix::fs::symlink("/etc/hosts", root.join("escape.txt")).unwrap();
        assert!(
            input_context(&root, &[artifact("escape.txt", "text/plain")])
                .await
                .is_err()
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
