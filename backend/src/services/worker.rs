use crate::clients::cursor::{CreateAgentOpts, CursorClient};
use crate::clients::figma::FigmaClient;
use crate::clients::model_router::ModelRouter;
use crate::clients::omniroute::OmniRouteClient;
use crate::clients::stitch::StitchClient;
use crate::domain::{
    ArtifactRef, LanguageMode, PipelineModelConfig, ProgrammingLanguage, StageCommand, StageId,
};
use crate::error::{AutoForgeError, Result};
use crate::services::ai::TokenPolicy;
use crate::services::artifacts::ArtifactStore;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

mod delivery;
mod design;
mod extraction_cache;
mod implementation;
mod input;
mod planning;
mod quality;

pub use delivery::DeliverExecutor;
pub use design::DesignExecutor;
pub use implementation::ImplementExecutor;
pub use input::{IngestExecutor, SummarizeExecutor};
pub use planning::ArchitectExecutor;
pub use quality::{DebugExecutor, SecurityPatchExecutor, VerifyExecutor};

pub struct StageContext {
    pub command: StageCommand,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub cursor: Arc<CursorClient>,
    pub omniroute: Arc<OmniRouteClient>,
    pub model_router: ModelRouter,
    pub token_policy: TokenPolicy,
    pub astra_max_calls: u8,
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

fn gateway_model<'a>(configured: Option<&'a str>, default: &'a str) -> &'a str {
    configured
        .filter(|model| !model.trim().is_empty())
        .unwrap_or(default)
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

    #[test]
    fn gateway_model_prefers_non_empty_project_override() {
        assert_eq!(
            gateway_model(Some("project/model"), "default/model"),
            "project/model"
        );
        assert_eq!(gateway_model(Some("  "), "default/model"), "default/model");
        assert_eq!(gateway_model(None, "default/model"), "default/model");
    }
}
