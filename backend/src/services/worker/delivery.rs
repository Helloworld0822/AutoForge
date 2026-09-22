use super::{StageContext, StageExecutor, StageOutput};
use crate::domain::StageId;
use crate::error::Result;
use async_trait::async_trait;
use bytes::Bytes;

pub struct DeliverExecutor;

#[async_trait]
impl StageExecutor for DeliverExecutor {
    fn stage(&self) -> StageId {
        StageId::Deliver
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let manifest = serde_json::json!({
            "project_id": ctx.command.project_id.0,
            "pr_url": ctx.pr_url,
            "artifacts": ctx.input.iter().map(|artifact| &artifact.uri).collect::<Vec<_>>(),
            "stage_outputs": ctx.stage_outputs,
            "delivered_at": chrono::Utc::now().to_rfc3339(),
        });
        let key = format!(
            "projects/{}/deliver/delivery_manifest.json",
            ctx.command.project_id.0
        );
        let artifact = ctx
            .artifacts
            .put(&key, Bytes::from(manifest.to_string()), "application/json")
            .await?;
        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: manifest,
        })
    }
}
