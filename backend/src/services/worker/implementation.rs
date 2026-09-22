use super::{StageContext, StageExecutor, StageOutput};
use crate::clients::cursor::CreateAgentOpts;
use crate::domain::StageId;
use crate::error::{AutoForgeError, Result};
use crate::services::language::language_prompt_note;
use async_trait::async_trait;

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
            .create_agent(&build_prompt(ctx), &profile, opts)
            .await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &response.agent.id,
                &response.run.id,
                std::time::Duration::from_secs(15),
            )
            .await?;

        let pr_url = run
            .result
            .and_then(|result| result.git)
            .and_then(|git| git.branches)
            .and_then(|branches| branches.into_iter().next())
            .and_then(|branch| branch.pr_url);

        Ok(StageOutput {
            artifacts: vec![],
            metadata: serde_json::json!({
                "cursor_agent_id": response.agent.id,
                "pr_url": pr_url,
            }),
        })
    }
}

fn build_prompt(ctx: &StageContext) -> String {
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

    format!(
        "tasks.json 순서대로 구현하세요. design/screens/ 의 UI 참고 자료를 사용하세요. \
         Stitch HTML 또는 Figma PNG/export JSON이 포함될 수 있습니다.\n\
         {language_note}{devops_note}입력: {:?}",
        ctx.input
            .iter()
            .map(|artifact| &artifact.uri)
            .collect::<Vec<_>>()
    )
}
