use super::extraction_cache::{fingerprint, parse_spec, usage_metadata, ExtractionCache};
use super::{
    complete_json_budgeted, gateway_model, read_named_text, StageContext, StageExecutor,
    StageOutput,
};
use crate::clients::model_router::ModelRole;
use crate::domain::{DevopsPlanInput, StageId};
use crate::error::{AutoForgeError, Result};
use crate::services::ai::{project_spec_system, AiPurpose};
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
        let result = tokio::task::spawn_blocking(move || ingest_pdf(&bytes))
            .await
            .map_err(|error| {
                AutoForgeError::Ingest(format!("PDF parser task failed: {error}"))
            })??;
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
            "pdf_type": result.pdf_type,
            "confidence": result.confidence,
            "pages_needing_ocr": result.pages_needing_ocr,
            "encoding": result.encoding,
            "extraction_method": result.extraction_method,
            "estimated_input_tokens": result.estimated_input_tokens,
            "token_estimate_only": true,
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
            {
                let devops = tokio::task::spawn_blocking(move || ingest_devops_plan(&devops_input))
                    .await
                    .map_err(|error| {
                        AutoForgeError::Ingest(format!("DevOps parser task failed: {error}"))
                    })??;
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
        let mut raw_text = read_named_text(ctx, "raw_text.md").await?;
        if ctx
            .input
            .iter()
            .any(|artifact| artifact.name == "devops_raw_text.md")
        {
            raw_text.push_str("\n\n# DevOps source\n");
            raw_text.push_str(&read_named_text(ctx, "devops_raw_text.md").await?);
        }
        let model = gateway_model(
            ctx.model_config.summarize.as_deref(),
            ctx.model_router.model(ModelRole::Extract),
        );
        let fingerprint = fingerprint(&raw_text, model, &ctx.token_policy);
        let cached = if ctx
            .input
            .iter()
            .any(|artifact| artifact.name == "extract_cache.json")
        {
            let cached: ExtractionCache =
                serde_json::from_str(&read_named_text(ctx, "extract_cache.json").await?).map_err(
                    |error| AutoForgeError::Artifacts(format!("invalid extraction cache: {error}")),
                )?;
            (cached.fingerprint == fingerprint).then_some(cached)
        } else {
            None
        };
        let cache_hit = cached.is_some();
        let mut responses = Vec::new();
        let cached = if let Some(cached) = cached {
            cached
        } else {
            let mut last_error = None;
            let mut result = None;
            for attempt in 0..2 {
                let correction = last_error.as_ref().map(|error| format!("\nPrevious schema error: {error}. Return all fields with correct types.")).unwrap_or_default();
                let response = complete_json_budgeted(ctx, model, project_spec_system(),
                    format!("Extract this source document into project_spec.json:\n\n{raw_text}{correction}"),
                    AiPurpose::Extract, &format!("extract:{attempt}")).await?;
                let parsed = parse_spec(&response.content);
                let response_model = response.model.clone();
                responses.push(response);
                match parsed {
                    Ok(spec) => {
                        result = Some(ExtractionCache {
                            fingerprint: fingerprint.clone(),
                            spec,
                            model: response_model,
                        });
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            result.ok_or_else(|| {
                last_error.unwrap_or_else(|| AutoForgeError::OmniRoute("extraction failed".into()))
            })?
        };
        let text = serde_json::to_string(&cached.spec)
            .map_err(|error| AutoForgeError::OmniRoute(error.to_string()))?;
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
        let cache = ctx
            .artifacts
            .put(
                &format!("{base}/extract_cache.json"),
                Bytes::from(
                    serde_json::to_vec(&cached)
                        .map_err(|error| AutoForgeError::Internal(error.to_string()))?,
                ),
                "application/json",
            )
            .await?;
        let mut metadata = usage_metadata(&responses);
        metadata["model"] = serde_json::json!(cached.model);
        metadata["cache_hit"] = serde_json::json!(cache_hit);
        metadata["source_fingerprint"] = serde_json::json!(fingerprint);
        metadata["programming_language"] = serde_json::json!(resolved.as_str());
        metadata["ui_required"] = serde_json::json!(!cached.spec.ui_requirements.is_empty());

        Ok(StageOutput {
            artifacts: vec![artifact, cache],
            metadata,
        })
    }
}
