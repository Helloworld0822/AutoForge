use crate::clients::cursor::{CreateAgentOpts, CursorClient};
use crate::clients::figma::FigmaClient;
use crate::clients::model_router::{ModelRole, ModelRouter};
use crate::clients::openrouter::{AiProvider, OpenRouterClient};
use crate::clients::stitch::StitchClient;
use crate::domain::{
    ArtifactRef, LanguageMode, PipelineModelConfig, ProgrammingLanguage, StageCommand, StageId,
};
use crate::error::{AutoForgeError, Result};
use crate::services::ai::{complete_json, parse_json, QuestionList};
use crate::services::artifacts::ArtifactStore;
use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;

mod delivery;
mod design;
mod implementation;
mod input;
mod quality;

pub use delivery::DeliverExecutor;
pub use design::DesignExecutor;
pub use implementation::ImplementExecutor;
pub use input::{IngestExecutor, SummarizeExecutor};
pub use quality::{DebugExecutor, SecurityPatchExecutor, VerifyExecutor};

pub struct StageContext {
    pub command: StageCommand,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub cursor: Arc<CursorClient>,
    pub openrouter: Arc<OpenRouterClient>,
    pub model_router: ModelRouter,
    pub deepseek_debug_retries: u8,
    pub mid_debug_retries: u8,
    pub opus_max_calls: u8,
    pub project_budget_usd: f64,
    pub task_budget_usd: f64,
    pub stitch: Arc<StitchClient>,
    pub figma: Arc<FigmaClient>,
    pub input: Vec<ArtifactRef>,
    pub repo_url: Option<String>,
    pub stage_outputs: HashMap<StageId, serde_json::Value>,
    pub pr_url: Option<String>,
    pub language_mode: LanguageMode,
    pub programming_language: Option<ProgrammingLanguage>,
    pub resolved_language: Option<ProgrammingLanguage>,
    pub architecture_finalize: bool,
    pub architecture_answers: Vec<(String, String)>,
    pub model_config: PipelineModelConfig,
}

#[derive(Debug)]
pub struct StageOutput {
    pub artifacts: Vec<ArtifactRef>,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait StageExecutor: Send + Sync {
    fn stage(&self) -> StageId;
    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput>;
}

async fn read_named_text(ctx: &StageContext, name: &str) -> Result<String> {
    let artifact = ctx
        .input
        .iter()
        .find(|artifact| artifact.name == name)
        .ok_or_else(|| AutoForgeError::Ingest(format!("missing {name} artifact")))?;
    let bytes = ctx.artifacts.get(&artifact.key).await?;
    String::from_utf8(bytes.to_vec())
        .map_err(|error| AutoForgeError::Ingest(format!("{name} is not UTF-8: {error}")))
}

pub struct ArchitectExecutor;

#[async_trait]
impl StageExecutor for ArchitectExecutor {
    fn stage(&self) -> StageId {
        StageId::Architect
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        if ctx.architecture_finalize {
            return run_architect_finalize(ctx).await;
        }

        let spec = read_named_text(ctx, "project_spec.json").await?;
        let response = complete_json(
            &ctx.openrouter,
            ctx.model_router.model(ModelRole::Plan),
            "Create clarification questions from the structured project spec. Do not invent requirements.",
            format!("project_spec.json:\n{spec}"),
        )
        .await?;
        let question_list: QuestionList = parse_json(&response.content)?;
        let questions = question_list.questions;

        if questions.is_empty() {
            return run_architect_finalize_with_answers(ctx, &[]).await;
        }

        let base = format!("projects/{}/architect", ctx.command.project_id.0);
        let questions_json = serde_json::to_string(&serde_json::json!({ "questions": questions }))
            .unwrap_or_default();
        let draft = ctx
            .artifacts
            .put(
                &format!("{base}/clarifications.json"),
                Bytes::from(questions_json),
                "application/json",
            )
            .await?;

        let question_views: Vec<_> = questions
            .iter()
            .map(|q| {
                serde_json::json!({
                    "id": q.id,
                    "question": q.question,
                    "options": q.options,
                    "required": q.required,
                    "category": q.category,
                })
            })
            .collect();

        Ok(StageOutput {
            artifacts: vec![draft],
            metadata: serde_json::json!({
                "phase": "draft",
                "model": response.model,
                "questions": question_views,
                "question_count": questions.len(),
            }),
        })
    }
}

fn agent_opts<'a>(repo_url: &'a str, _pr_url: Option<&'a str>) -> CreateAgentOpts<'a> {
    CreateAgentOpts {
        repo_url: Some(repo_url),
        starting_ref: Some("main"),
        auto_create_pr: Some(false),
        agent_id: None,
    }
}

pub fn executors() -> Vec<Arc<dyn StageExecutor>> {
    vec![
        Arc::new(IngestExecutor),
        Arc::new(SummarizeExecutor),
        Arc::new(ArchitectExecutor),
        Arc::new(DesignExecutor),
        Arc::new(ImplementExecutor),
        Arc::new(VerifyExecutor),
        Arc::new(DebugExecutor),
        Arc::new(SecurityPatchExecutor),
        Arc::new(DeliverExecutor),
    ]
}

async fn run_architect_finalize(ctx: &StageContext) -> Result<StageOutput> {
    run_architect_finalize_with_answers(ctx, &ctx.architecture_answers).await
}

async fn run_architect_finalize_with_answers(
    ctx: &StageContext,
    answers: &[(String, String)],
) -> Result<StageOutput> {
    let spec = read_named_text(ctx, "project_spec.json").await?;
    let response = ctx
        .openrouter
        .complete(crate::clients::openrouter::AiRequest {
            model: ctx.model_router.model(ModelRole::Plan).to_string(),
            messages: vec![
                crate::clients::openrouter::AiMessage { role: crate::clients::openrouter::AiRole::System, content: "Plan from project_spec.json. Return JSON with architecture, spec, tasks, and planning_meta. Keep tasks small and independent.".into() },
                crate::clients::openrouter::AiMessage { role: crate::clients::openrouter::AiRole::User, content: format!("spec:\n{spec}\nanswers:\n{answers:?}"), },
            ],
            temperature: Some(0.1),
            max_tokens: Some(16_000),
            response_format: Some(crate::clients::openrouter::ResponseFormat { kind: "json_object".into() }),
        })
        .await?;
    let text = response.content.clone();
    let base = format!("projects/{}/architect", ctx.command.project_id.0);
    let spec = ctx
        .artifacts
        .put(
            &format!("{base}/spec.md"),
            Bytes::from(text),
            "text/markdown",
        )
        .await?;

    Ok(StageOutput {
        artifacts: vec![spec],
        metadata: serde_json::json!({
            "phase": "finalize",
            "model": response.model,
            "input_tokens": response.usage.input_tokens,
            "output_tokens": response.usage.output_tokens,
            "cost_usd": response.usage.cost_usd,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn executor_registry_contains_each_stage_once() {
        let registered: Vec<_> = executors()
            .into_iter()
            .map(|executor| executor.stage())
            .collect();
        let unique: HashSet<_> = registered.iter().copied().collect();

        assert_eq!(registered.len(), StageId::all().len());
        assert_eq!(unique.len(), StageId::all().len());
        assert!(StageId::all().iter().all(|stage| unique.contains(stage)));
    }
}
