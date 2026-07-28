use crate::domain::{ArtifactRef, StageId};
use crate::error::{AutoForgeError, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

/// 파이프라인 산출물을 프로젝트별 로컬 git 저장소에 커밋하고, 하루 종료 시 원격에 push한다.
#[derive(Clone)]
pub struct ProjectGitSync {
    artifacts_dir: PathBuf,
    github_token: Option<String>,
    daily_push_hour_utc: u8,
}

impl ProjectGitSync {
    pub fn new(
        artifacts_dir: impl Into<PathBuf>,
        github_token: Option<String>,
        daily_push_hour_utc: u8,
    ) -> Self {
        Self {
            artifacts_dir: artifacts_dir.into(),
            github_token,
            daily_push_hour_utc: daily_push_hour_utc.min(23),
        }
    }

    pub fn project_dir(&self, project_id: Uuid) -> PathBuf {
        self.artifacts_dir
            .join("projects")
            .join(project_id.to_string())
    }

    /// 스테이지 산출물마다 개별 커밋을 생성한다.
    pub async fn commit_stage_artifacts(
        &self,
        project_id: Uuid,
        stage: StageId,
        artifacts: &[ArtifactRef],
    ) -> Result<usize> {
        if artifacts.is_empty() {
            return Ok(0);
        }

        let project_dir = self.project_dir(project_id);
        if !project_dir.exists() {
            return Ok(0);
        }

        self.ensure_repo(&project_dir).await?;
        let prefix = format!("projects/{project_id}/");
        let mut committed = 0usize;

        for artifact in artifacts {
            let rel = artifact
                .key
                .strip_prefix(&prefix)
                .unwrap_or(artifact.key.as_str());
            let file_path = project_dir.join(rel);
            if !file_path.exists() {
                continue;
            }
            let message = format!("{}: add {}", stage.as_str(), rel);
            if self.commit_file(&project_dir, rel, &message).await? {
                committed += 1;
                info!(%project_id, ?stage, file = %rel, "git commit created for artifact");
            }
        }

        Ok(committed)
    }

    /// 원격 GitHub 레포로 push (GITHUB_TOKEN 필요).
    pub async fn push_project(&self, project_id: Uuid, remote_url: &str) -> Result<bool> {
        let token = match self.github_token.as_deref() {
            Some(t) if !t.is_empty() => t,
            _ => {
                warn!(%project_id, "skip git push — GITHUB_TOKEN not set");
                return Ok(false);
            }
        };

        let project_dir = self.project_dir(project_id);
        if !project_dir.join(".git").exists() {
            return Ok(false);
        }

        let push_url = authenticated_remote_url(remote_url, token)?;
        self.ensure_remote(&project_dir, &push_url).await?;

        if !self.has_commits(&project_dir).await? {
            return Ok(false);
        }

        self.run_git(&project_dir, &["branch", "-M", "main"])
            .await?;
        match self
            .run_git(&project_dir, &["push", "-u", "origin", "main"])
            .await
        {
            Ok(()) => {
                info!(%project_id, remote = %mask_url(remote_url), "git push completed");
                Ok(true)
            }
            Err(e) => {
                // 이미 최신이면 성공으로 간주
                let msg = e.to_string();
                if msg.contains("Everything up-to-date") {
                    info!(%project_id, "git push skipped — already up to date");
                    return Ok(false);
                }
                Err(e)
            }
        }
    }

    pub fn daily_push_hour_utc(&self) -> u8 {
        self.daily_push_hour_utc
    }

    /// 단일 상대 경로 파일 커밋 (일일 경과 등).
    pub async fn commit_path(&self, project_id: Uuid, rel: &str, message: &str) -> Result<bool> {
        let project_dir = self.project_dir(project_id);
        if !project_dir.join(rel).exists() {
            return Ok(false);
        }
        self.ensure_repo(&project_dir).await?;
        self.commit_file(&project_dir, rel, message).await
    }

    async fn ensure_repo(&self, dir: &Path) -> Result<()> {
        if dir.join(".git").exists() {
            return Ok(());
        }
        self.run_git(dir, &["init"]).await?;
        self.run_git(dir, &["config", "user.email", "autoforge@local"])
            .await?;
        self.run_git(dir, &["config", "user.name", "AutoForge"])
            .await?;
        info!(dir = %dir.display(), "initialized project git repository");
        Ok(())
    }

    async fn ensure_remote(&self, dir: &Path, push_url: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(dir)
            .output()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("git remote: {e}")))?;

        if output.status.success() {
            self.run_git(dir, &["remote", "set-url", "origin", push_url])
                .await?;
        } else {
            self.run_git(dir, &["remote", "add", "origin", push_url])
                .await?;
        }
        Ok(())
    }

    async fn commit_file(&self, dir: &Path, rel: &str, message: &str) -> Result<bool> {
        self.run_git(dir, &["add", "--", rel]).await?;

        let status = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(dir)
            .status()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("git diff: {e}")))?;

        if status.success() {
            return Ok(false);
        }

        self.run_git(dir, &["commit", "-m", message]).await?;
        Ok(true)
    }

    async fn has_commits(&self, dir: &Path) -> Result<bool> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("git rev-parse: {e}")))?;
        Ok(output.status.success())
    }

    async fn run_git(&self, dir: &Path, args: &[&str]) -> Result<()> {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("git spawn: {e}")))?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(AutoForgeError::Internal(format!(
            "git {} failed: {}{}",
            args.join(" "),
            stderr,
            stdout
        )))
    }
}

fn authenticated_remote_url(remote_url: &str, token: &str) -> Result<String> {
    let url = remote_url.trim_end_matches('/').trim_end_matches(".git");
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        return Ok(format!(
            "https://x-access-token:{token}@github.com/{rest}.git"
        ));
    }
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        return Ok(format!(
            "https://x-access-token:{token}@github.com/{rest}.git"
        ));
    }
    Err(AutoForgeError::BadRequest(format!(
        "unsupported remote for git push: {remote_url}"
    )))
}

fn mask_url(url: &str) -> String {
    url.replace("x-access-token:", "x-access-token:***@")
        .chars()
        .take(60)
        .collect()
}

/// 매일 지정 시각(UTC)에 커밋된 프로젝트를 일괄 push한다.
pub async fn run_daily_push_loop(app: std::sync::Arc<crate::app::App>) {
    let git = match &app.project_git {
        Some(g) => g.clone(),
        None => return,
    };

    loop {
        let sleep_secs = seconds_until_hour(git.daily_push_hour_utc());
        tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
        if let Err(e) = push_all_projects(&app, &git).await {
            warn!(error = %e, "daily git push failed");
        }
    }
}

pub async fn push_all_projects(app: &crate::app::App, git: &ProjectGitSync) -> Result<usize> {
    let projects = app.store.list().await?;
    let mut pushed = 0usize;

    for project in projects {
        let Some(repo_url) = project.repo_url.as_deref() else {
            continue;
        };
        let project_id = project.id.0;
        match git.push_project(project_id, repo_url).await {
            Ok(true) => {
                pushed += 1;
                if let Ok(Some(mut p)) = app.store.get(project_id).await {
                    let _ = crate::services::daily_log_notify::record_daily_event(
                        app,
                        &mut p,
                        crate::services::daily_log::DailyEvent {
                            event: "git_pushed",
                            stage: None,
                            message: format!("일일 git push 완료 — {repo_url}"),
                        },
                    )
                    .await;
                }
            }
            Ok(false) => {}
            Err(e) => {
                warn!(%project_id, error = %e, "project git push failed");
            }
        }
    }

    if pushed > 0 {
        info!(count = pushed, "daily git push finished");
    }
    Ok(pushed)
}

fn seconds_until_hour(target_hour: u8) -> u64 {
    use chrono::Utc;
    let now = Utc::now();
    let target = now
        .date_naive()
        .and_hms_opt(target_hour as u32, 0, 0)
        .unwrap()
        .and_utc();
    let target = if target <= now {
        target + chrono::Duration::days(1)
    } else {
        target
    };
    (target - now).num_seconds().max(1) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_remote_url_https() {
        let url = authenticated_remote_url("https://github.com/acme/my-app", "ghp_test").unwrap();
        assert!(url.contains("x-access-token:ghp_test@github.com/acme/my-app.git"));
    }

    #[test]
    fn authenticated_remote_url_ssh() {
        let url = authenticated_remote_url("git@github.com:acme/my-app.git", "ghp_test").unwrap();
        assert!(url.contains("acme/my-app.git"));
    }
}
