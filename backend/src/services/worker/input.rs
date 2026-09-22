use super::{read_named_text, StageContext, StageExecutor, StageOutput};
use crate::clients::model_router::ModelRole;
use crate::domain::{DevopsPlanInput, StageId};
use crate::error::{AutoForgeError, Result};
use crate::services::ai::{complete_json, parse_json, project_spec_system, ProjectSpec};
use crate::services::ingest::{ingest_devops_plan, ingest_pdf};
use crate::services::language::resolve_effective_language;
use async_trait::async_trait;
use bytes::Bytes;

pub struct IngestExecutor;

#[async_trait]
impl StageExecutor for IngestExecutor {
    fn stage(&self) -> StageId {
        StageId::Ingest
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let pdf_ref = ctx
            .input
            .iter()
            .find(|artifact| artifact.name.ends_with(".pdf") && !artifact.name.contains("devops"))
            .or_else(|| {
                ctx.input
                    .iter()
                    .find(|artifact| artifact.name == "plan.pdf")
            })
            .ok_or_else(|| AutoForgeError::Ingest("missing PDF input".into()))?;
        let bytes = ctx.artifacts.get(&pdf_ref.key).await?;
        let result = ingest_pdf(&bytes)?;
        let base = format!("projects/{}/ingest", ctx.command.project_id.0);
        let text_uri = ctx
            .artifacts
            .put(
                &format!("{base}/raw_text.md"),
                Bytes::from(result.raw_text),
                "text/markdown",
            )
            .await?;
        let mut metadata = serde_json::json!({
            "page_count": result.page_count,
            "sha256": result.sha256,
            "has_devops_plan": false,
        });
        let mut artifacts = vec![text_uri];

        if let Some(devops_ref) = ctx
            .input
            .iter()
            .find(|artifact| artifact.name.starts_with("devops_plan"))
        {
            let devops_bytes = ctx.artifacts.get(&devops_ref.key).await?;
            let devops_input = DevopsPlanInput {
                filename: Some(devops_ref.name.clone()),
                content_type: Some(devops_ref.content_type.clone()),
                bytes: Some(devops_bytes.to_vec()),
                text: None,
            };
            if let Ok(devops) = ingest_devops_plan(&devops_input) {
                let artifact = ctx
                    .artifacts
                    .put(
                        &format!("{base}/devops_raw_text.md"),
                        Bytes::from(devops.raw_text),
                        "text/markdown",
                    )
                    .await?;
                artifacts.push(artifact);
                metadata["has_devops_plan"] = serde_json::json!(true);
                metadata["devops_format"] = serde_json::json!(devops.format);
                metadata["devops_source"] = serde_json::json!(devops.source);
                metadata["devops_sha256"] = serde_json::json!(devops.sha256);
            }
        }

        let metadata_artifact = ctx
            .artifacts
            .put(
                &format!("{base}/ingest_meta.json"),
                Bytes::from(metadata.to_string()),
                "application/json",
            )
            .await?;
        artifacts.push(metadata_artifact);

        Ok(StageOutput {
            artifacts,
            metadata,
        })
    }
}

pub struct SummarizeExecutor;

#[async_trait]
impl StageExecutor for SummarizeExecutor {
    fn stage(&self) -> StageId {
        StageId::Summarize
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let raw_text = read_named_text(ctx, "raw_text.md").await?;
        let response = complete_json(
            &ctx.openrouter,
            ctx.model_router.model(ModelRole::Extract),
            project_spec_system(),
            format!("Extract this source document into project_spec.json:\n\n{raw_text}"),
        )
        .await?;
        let spec: ProjectSpec = parse_json(&response.content)?;
        let text = serde_json::to_string_pretty(&spec)
            .map_err(|error| AutoForgeError::OpenRouter(error.to_string()))?;
        let resolved =
            resolve_effective_language(ctx.language_mode, ctx.programming_language, &text);
        let base = format!("projects/{}/extract", ctx.command.project_id.0);
        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/project_spec.json"),
                Bytes::from(text),
                "application/json",
            )
            .await?;

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({
                "model": response.model,
                "input_tokens": response.usage.input_tokens,
                "output_tokens": response.usage.output_tokens,
                "cost_usd": response.usage.cost_usd,
                "programming_language": resolved.as_str(),
                "ui_required": !spec.ui_requirements.is_empty(),
            }),
        })
    }
}
