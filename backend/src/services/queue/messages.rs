use crate::domain::{ArtifactRef, StageCommand, StageId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Redis Streams 커맨드 큐 메시지
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueCommand {
    pub project_id: Uuid,
    pub stage: StageId,
    pub attempt: u8,
}

impl From<StageCommand> for QueueCommand {
    fn from(cmd: StageCommand) -> Self {
        Self {
            project_id: cmd.project_id.0,
            stage: cmd.stage,
            attempt: cmd.attempt,
        }
    }
}

impl QueueCommand {
    pub fn to_stage_command(&self) -> StageCommand {
        StageCommand {
            project_id: crate::domain::ProjectId(self.project_id),
            stage: self.stage,
            attempt: self.attempt,
        }
    }
}

/// 파이프라인 이벤트 — 오케스트레이터가 소비
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineEvent {
    ProjectCreated {
        project_id: Uuid,
        name: Option<String>,
    },
    StageStarted {
        project_id: Uuid,
        stage: StageId,
    },
    StageCompleted {
        project_id: Uuid,
        stage: StageId,
        metadata: serde_json::Value,
        #[serde(default)]
        artifacts: Vec<ArtifactRef>,
        passed: Option<bool>,
    },
    StageFailed {
        project_id: Uuid,
        stage: StageId,
        error: String,
    },
    PipelineCompleted {
        project_id: Uuid,
    },
    PipelineFailed {
        project_id: Uuid,
        error: String,
    },
}
