use super::{StageContext, StageExecutor, StageOutput};
use crate::clients::cursor::CreateAgentOpts;
use crate::domain::{ArtifactRef, StageId};
use crate::error::{AutoForgeError, Result};
use crate::services::artifact_access::{
    is_coder_artifact_allowed, issue_coder_url, looks_like_secret,
};
use crate::services::language::language_prompt_note;
use async_trait::async_trait;
use chrono::Utc;

pub struct ImplementExecutor;

#[async_trait]
impl StageExecutor for ImplementExecutor {
    fn stage(&self) -> StageId {
        StageId::Implement
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let repo_url = ctx
            .repo_url
            .as_deref()
            .ok_or_else(|| AutoForgeError::BadRequest("repo_url required".into()))?;

        let profile = ctx.model_config.profile_for(StageId::Implement);
        let opts = CreateAgentOpts {
            repo_url: Some(repo_url),
            starting_ref: Some("main"),
            auto_create_pr: Some(true),
            agent_id: None,
        };

        let response = ctx
            .cursor
            .create_agent(&build_prompt(ctx).await?, &profile, opts)
            .await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &response.agent.id,
                &response.run.id,
                std::time::Duration::from_secs(15),
            )
            .await?;

        let branch = run
            .result
            .and_then(|result| result.git)
            .and_then(|git| git.branches)
            .and_then(|branches| branches.into_iter().find(|branch| branch.pr_url.is_some()));

        let pr_url = branch.as_ref().and_then(|branch| branch.pr_url.clone());
        let pr_branch = branch.as_ref().and_then(|branch| branch.name.clone());
        let head_sha = branch.as_ref().and_then(|branch| branch.sha.clone());

        if pr_url.is_none() || pr_branch.is_none() {
            return Err(AutoForgeError::StageFailed {
                stage: StageId::Implement,
                message: "cursor run completed without a pull request branch; \
                          verify/debug/security require a PR head to check"
                    .into(),
            });
        }

        Ok(StageOutput {
            artifacts: vec![],
            metadata: serde_json::json!({
                "cursor_agent_id": response.agent.id,
                "pr_url": pr_url,
                "pr_branch": pr_branch,
                "head_sha": head_sha,
            }),
        })
    }
}

async fn build_prompt(ctx: &StageContext) -> Result<String> {
    let has_devops = ctx
        .input
        .iter()
        .any(|artifact| artifact.name.starts_with("devops_plan"));
    let devops_note = if has_devops {
        "DevOps 계획서에 따라 Containerfile, compose.yml, CI/CD 워크플로우(.github/workflows), \
         nginx/인프라 설정을 구현하세요. 배포 자동화를 포함하세요.\n"
    } else {
        ""
    };
    let language_note = language_prompt_note(
        ctx.language_mode,
        ctx.programming_language,
        ctx.resolved_language,
    );

    let mut delivered: Vec<String> = Vec::new();
    for artifact in ctx
        .input
        .iter()
        .filter(|artifact| implementation_artifact(&artifact.name))
    {
        if let Some(line) = deliver_artifact(ctx, artifact).await? {
            delivered.push(line);
        }
    }

    Ok(format!(
        "tasks.json 순서대로 구현하세요. design/screens/ 의 UI 참고 자료를 사용하세요. \
         Stitch HTML 또는 Figma PNG/export JSON이 포함될 수 있습니다.\n\
         {language_note}{devops_note}입력:\n{}",
        delivered.join("\n")
    ))
}

async fn deliver_artifact(ctx: &StageContext, artifact: &ArtifactRef) -> Result<Option<String>> {
    if !is_coder_artifact_allowed(&artifact.name) {
        return Ok(None);
    }
    if let Ok(url) = issue_coder_url(
        &ctx.artifact_signing_secret,
        &ctx.public_url,
        ctx.command.project_id.0,
        artifact,
        Utc::now(),
    ) {
        return Ok(Some(format!("- {}: {url}", artifact.name)));
    }
    inline_artifact(ctx, artifact).await
}

async fn inline_artifact(ctx: &StageContext, artifact: &ArtifactRef) -> Result<Option<String>> {
    if artifact.content_type.starts_with("image/") || artifact.name.ends_with(".png") {
        return Ok(None);
    }
    let bytes = match ctx.artifacts.get(&artifact.key).await {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    if bytes.len() > ctx.coder_artifact_max_bytes {
        return Ok(None);
    }
    let Ok(text) = String::from_utf8(bytes.to_vec()) else {
        return Ok(None);
    };
    if looks_like_secret(&text) {
        tracing::warn!(
            artifact = %artifact.name,
            "artifact appears to contain credentials; omitted from the coder prompt"
        );
        return Ok(Some(format!(
            "- {}: omitted (potential secret material)",
            artifact.name
        )));
    }
    Ok(Some(format!("- {} (inline):\n{text}", artifact.name)))
}

fn implementation_artifact(name: &str) -> bool {
    matches!(
        name,
        "architecture.md" | "spec.md" | "tasks.json" | "project_spec.json" | "figma-design.json"
    ) || name.starts_with("screens/")
        || name.ends_with(".png")
        || name.ends_with(".html")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_raw_inputs_and_internal_state_from_coder_prompt() {
        for name in [
            "plan.pdf",
            "raw_text.md",
            "devops_raw_text.md",
            "extract_cache.json",
            "usage.json",
        ] {
            assert!(!implementation_artifact(name));
        }
        for name in [
            "architecture.md",
            "spec.md",
            "tasks.json",
            "screens/login.html",
        ] {
            assert!(implementation_artifact(name));
        }
    }
}
