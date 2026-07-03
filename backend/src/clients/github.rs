use crate::error::{AutoForgeError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const GITHUB_API: &str = "https://api.github.com";

#[derive(Clone)]
pub struct GitHubClient {
    http: Client,
    token: String,
    org: Option<String>,
    auto_merge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedRepo {
    pub name: String,
    pub full_name: String,
    pub html_url: String,
    pub clone_url: String,
    pub private: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestRef {
    pub number: u64,
    pub html_url: String,
    pub state: String,
    pub merged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    pub merged: bool,
    pub sha: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct CreateRepoResponse {
    name: String,
    full_name: String,
    html_url: String,
    clone_url: String,
    private: bool,
}

#[derive(Debug, Deserialize)]
struct ListPrResponse {
    number: u64,
    html_url: String,
    state: String,
    merged_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergePrResponse {
    merged: Option<bool>,
    sha: Option<String>,
    message: Option<String>,
}

impl GitHubClient {
    pub fn new(token: String, org: Option<String>, auto_merge: bool) -> Result<Self> {
        if token.is_empty() {
            return Err(AutoForgeError::BadRequest(
                "GITHUB_TOKEN is required for GitHub automation".into(),
            ));
        }
        Ok(Self {
            http: Client::new(),
            token,
            org,
            auto_merge,
        })
    }

    pub fn is_configured(&self) -> bool {
        !self.token.is_empty()
    }

    pub fn auto_merge_enabled(&self) -> bool {
        self.auto_merge
    }

    /// GitHub 프라이빗 레포 자동 생성 (auto_init README 포함)
    pub async fn create_private_repo(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<CreatedRepo> {
        let body = serde_json::json!({
            "name": name,
            "description": description.unwrap_or("AutoForge AI-generated project"),
            "private": true,
            "auto_init": true,
            "has_issues": true,
            "has_projects": false,
        });

        let url = if let Some(org) = &self.org {
            format!("{GITHUB_API}/orgs/{org}/repos")
        } else {
            format!("{GITHUB_API}/user/repos")
        };

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&body)
            .send()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("github request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AutoForgeError::Internal(format!(
                "github create repo failed ({status}): {text}"
            )));
        }

        let repo: CreateRepoResponse = resp
            .json()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("github parse error: {e}")))?;

        info!(repo = %repo.full_name, "created private github repository");

        Ok(CreatedRepo {
            name: repo.name,
            full_name: repo.full_name,
            html_url: repo.html_url,
            clone_url: repo.clone_url,
            private: repo.private,
        })
    }

    /// PR URL에서 owner/repo/number 파싱
    pub fn parse_pr_url(pr_url: &str) -> Option<(String, String, u64)> {
        let url = pr_url.trim_end_matches('/');
        let pull_idx = url.rfind("/pull/")?;
        let base = &url[..pull_idx];
        let number = url[pull_idx + 6..].parse().ok()?;

        let segments: Vec<&str> = base.rsplit('/').take(2).collect();
        if segments.len() < 2 {
            return None;
        }
        let repo = segments[0].to_string();
        let owner = segments[1].to_string();
        Some((owner, repo, number))
    }

    /// repo URL에서 owner/repo 파싱
    pub fn parse_repo_url(repo_url: &str) -> Option<(String, String)> {
        let url = repo_url.trim_end_matches('/').trim_end_matches(".git");
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() < 2 {
            return None;
        }
        let repo = parts.last()?.to_string();
        let owner = parts.get(parts.len() - 2)?.to_string();
        Some((owner, repo))
    }

    /// 열린 PR 목록 조회 (최신 1건)
    pub async fn find_open_pr(&self, owner: &str, repo: &str) -> Result<Option<PullRequestRef>> {
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/pulls?state=open&sort=created&direction=desc&per_page=1");

        let resp = self
            .api_get(&url)
            .await
            .map_err(|e| AutoForgeError::Internal(e.to_string()))?;

        let prs: Vec<ListPrResponse> = resp
            .json()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("github parse pr list: {e}")))?;

        Ok(prs.into_iter().next().map(|pr| PullRequestRef {
            number: pr.number,
            html_url: pr.html_url,
            state: pr.state,
            merged: pr.merged_at.is_some(),
        }))
    }

    /// PR 자동 머지 (squash)
    pub async fn merge_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pull_number: u64,
    ) -> Result<MergeResult> {
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/pulls/{pull_number}/merge");
        let body = serde_json::json!({
            "merge_method": "squash",
            "commit_title": format!("AutoForge: merge PR #{pull_number}"),
        });

        let resp = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&body)
            .send()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("github merge request failed: {e}")))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if status.is_success() {
            let parsed: MergePrResponse = serde_json::from_str(&text).unwrap_or(MergePrResponse {
                merged: Some(true),
                sha: None,
                message: None,
            });
            info!(owner, repo, pull_number, "merged pull request");
            return Ok(MergeResult {
                merged: parsed.merged.unwrap_or(true),
                sha: parsed.sha,
                message: parsed.message.unwrap_or_else(|| "merged".into()),
            });
        }

        // 이미 머지된 경우
        if text.contains("Pull Request is not mergeable")
            || text.contains("was already merged")
            || text.contains("No commits between")
        {
            warn!(owner, repo, pull_number, body = %text, "merge skipped or already done");
            return Ok(MergeResult {
                merged: false,
                sha: None,
                message: text,
            });
        }

        Err(AutoForgeError::Internal(format!(
            "github merge failed ({status}): {text}"
        )))
    }

    /// PR URL 또는 repo에서 PR을 찾아 자동 머지
    pub async fn auto_merge_pr(&self, repo_url: &str, pr_url: Option<&str>) -> Result<MergeResult> {
        if !self.auto_merge {
            return Ok(MergeResult {
                merged: false,
                sha: None,
                message: "auto_merge disabled".into(),
            });
        }

        if let Some(url) = pr_url {
            if let Some((owner, repo, number)) = Self::parse_pr_url(url) {
                return self.merge_pull_request(&owner, &repo, number).await;
            }
        }

        let (owner, repo) = Self::parse_repo_url(repo_url)
            .ok_or_else(|| AutoForgeError::BadRequest("invalid repo_url".into()))?;

        if let Some(pr) = self.find_open_pr(&owner, &repo).await? {
            return self.merge_pull_request(&owner, &repo, pr.number).await;
        }

        Ok(MergeResult {
            merged: false,
            sha: None,
            message: "no open pull request found".into(),
        })
    }

    async fn api_get(&self, url: &str) -> Result<reqwest::Response> {
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| AutoForgeError::Internal(format!("github GET failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AutoForgeError::Internal(format!(
                "github GET {status}: {text}"
            )));
        }
        Ok(resp)
    }
}

/// 프로젝트 이름을 GitHub 레포 이름으로 정규화
pub fn slugify_repo_name(name: &str, suffix: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else if c == ' ' || c == '-' || c == '_' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|c| *c != '\0')
        .collect();

    slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        slug = "autoforge-project".into();
    }
    if slug.len() > 40 {
        slug.truncate(40);
        slug = slug.trim_end_matches('-').to_string();
    }
    format!("{slug}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_url_works() {
        let (owner, repo, num) =
            GitHubClient::parse_pr_url("https://github.com/acme/my-app/pull/42").unwrap();
        assert_eq!(owner, "acme");
        assert_eq!(repo, "my-app");
        assert_eq!(num, 42);
    }

    #[test]
    fn parse_repo_url_works() {
        let (owner, repo) =
            GitHubClient::parse_repo_url("https://github.com/acme/my-app.git").unwrap();
        assert_eq!(owner, "acme");
        assert_eq!(repo, "my-app");
    }

    #[test]
    fn slugify_repo_name_works() {
        let slug = super::slugify_repo_name("테스트 프로젝트!", "a1b2c3d4");
        assert!(slug.ends_with("-a1b2c3d4"));
        assert!(slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
