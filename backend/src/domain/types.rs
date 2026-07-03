use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::services::orchestrator::DagScheduler;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageId {
    Ingest,
    Summarize,
    Architect,
    Design,
    Implement,
    Verify,
    Debug,
    SecurityPatch,
    Deliver,
}

impl StageId {
    pub fn all() -> &'static [StageId] {
        &[
            StageId::Ingest,
            StageId::Summarize,
            StageId::Architect,
            StageId::Design,
            StageId::Implement,
            StageId::Verify,
            StageId::Debug,
            StageId::SecurityPatch,
            StageId::Deliver,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            StageId::Ingest => "ingest",
            StageId::Summarize => "summarize",
            StageId::Architect => "architect",
            StageId::Design => "design",
            StageId::Implement => "implement",
            StageId::Verify => "verify",
            StageId::Debug => "debug",
            StageId::SecurityPatch => "security_patch",
            StageId::Deliver => "deliver",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    Agent,
    Plan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    pub model_id: String,
    pub mode: AgentMode,
    #[serde(default)]
    pub params: Vec<ModelParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelParam {
    pub id: String,
    pub value: String,
}

impl ModelProfile {
    pub fn summarize() -> Self {
        Self {
            model_id: "claude-haiku-4-5".into(),
            mode: AgentMode::Agent,
            params: vec![],
        }
    }

    pub fn architect() -> Self {
        Self {
            model_id: "claude-4.6-sonnet-high-thinking".into(),
            mode: AgentMode::Plan,
            params: vec![],
        }
    }

    pub fn implement() -> Self {
        Self {
            model_id: "gpt-5.3-codex-high".into(),
            mode: AgentMode::Agent,
            params: vec![],
        }
    }

    pub fn verify() -> Self {
        Self {
            model_id: "gpt-5.3-codex-high".into(),
            mode: AgentMode::Agent,
            params: vec![],
        }
    }

    pub fn debug() -> Self {
        Self {
            model_id: "gpt-5.3-codex-high".into(),
            mode: AgentMode::Agent,
            params: vec![],
        }
    }

    pub fn security_patch() -> Self {
        Self {
            model_id: "claude-fable-5-thinking-high".into(),
            mode: AgentMode::Agent,
            params: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectId(pub Uuid);

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunId(pub Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    Queued,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// 짧은 표시 이름 (매칭/식별용, 예: "plan.pdf", "devops_plan.md")
    pub name: String,
    /// 아티팩트 스토어 내 실제 조회 키 (예: "projects/{id}/plan.pdf")
    #[serde(default)]
    pub key: String,
    pub uri: String,
    pub content_type: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageCommand {
    pub project_id: ProjectId,
    pub stage: StageId,
    pub attempt: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageCompleted {
    pub project_id: ProjectId,
    pub stage: StageId,
    pub output_artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyLogEntry {
    pub at: DateTime<Utc>,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<StageId>,
    pub message: String,
    pub progress_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyLog {
    pub date: String,
    pub day_number: u32,
    pub entries: Vec<DailyLogEntry>,
    pub markdown: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyLogSummary {
    pub date: String,
    pub day_number: u32,
    pub entry_count: usize,
    pub progress_percent: u8,
    pub updated_at: DateTime<Utc>,
}

impl From<&DailyLog> for DailyLogSummary {
    fn from(log: &DailyLog) -> Self {
        let progress = log.entries.last().map(|e| e.progress_percent).unwrap_or(0);
        Self {
            date: log.date.clone(),
            day_number: log.day_number,
            entry_count: log.entries.len(),
            progress_percent: progress,
            updated_at: log.updated_at,
        }
    }
}

fn default_created_at() -> DateTime<Utc> {
    Utc::now()
}

/// DevOps 계획서 입력 (파일 또는 직접 작성)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DevopsPlanInput {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl DevopsPlanInput {
    pub fn has_content(&self) -> bool {
        self.bytes.as_ref().is_some_and(|b| !b.is_empty())
            || self.text.as_ref().is_some_and(|t| !t.trim().is_empty())
    }
}

/// 런타임 프로젝트 엔티티
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub state: PipelineState,
    pub stages: HashMap<StageId, StageState>,
    pub scheduler: DagScheduler,
    pub pdf_bytes: Option<Vec<u8>>,
    /// DevOps 계획서 (CI/CD, 인프라, 배포 파이프라인 등)
    #[serde(default)]
    pub devops_plan: Option<DevopsPlanInput>,
    /// 스테이지별 메타데이터 (pr_url, verify_report 등)
    pub stage_outputs: HashMap<StageId, serde_json::Value>,
    /// 누적 산출물 참조
    pub accumulated_artifacts: Vec<ArtifactRef>,
    /// Slack 진행 메시지 ts (업데이트용)
    #[serde(default)]
    pub slack_message_ts: Option<String>,
    #[serde(default = "default_created_at")]
    pub created_at: DateTime<Utc>,
    /// 일별 경과 로그 (YYYY-MM-DD → DailyLog)
    #[serde(default)]
    pub daily_logs: HashMap<String, DailyLog>,
}

impl Project {
    /// 전체 파이프라인 진행률 (0–100)
    pub fn progress_percent(&self) -> u8 {
        let total = StageId::all().len() as u32;
        let done = self
            .stages
            .values()
            .filter(|s| matches!(s, StageState::Completed | StageState::Skipped))
            .count() as u32;
        ((done * 100) / total.max(1)) as u8
    }

    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("Project {}", &self.id.0.to_string()[..8]))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    pub id: Uuid,
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub state: PipelineState,
    pub stages: Vec<StageStatusView>,
    pub progress_percent: u8,
    pub pr_url: Option<String>,
    pub merge_status: Option<String>,
    pub github_repo: Option<String>,
    pub has_devops_plan: bool,
    pub daily_log_count: usize,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StageStatusView {
    pub stage: StageId,
    pub status: StageState,
}

impl From<&Project> for ProjectView {
    fn from(p: &Project) -> Self {
        let pr_url = p
            .stage_outputs
            .get(&StageId::Implement)
            .and_then(|m| m.get("pr_url"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let merge_status = p
            .stage_outputs
            .get(&StageId::Deliver)
            .and_then(|m| m.get("merge_status"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let github_repo = p
            .stage_outputs
            .get(&StageId::Ingest)
            .and_then(|m| m.get("github_repo"))
            .and_then(|v| v.as_str())
            .map(String::from);

        Self {
            id: p.id.0,
            name: p.name.clone(),
            repo_url: p.repo_url.clone(),
            state: p.state,
            stages: StageId::all()
                .iter()
                .map(|&stage| StageStatusView {
                    stage,
                    status: p.stages.get(&stage).copied().unwrap_or(StageState::Queued),
                })
                .collect(),
            progress_percent: p.progress_percent(),
            pr_url,
            merge_status,
            github_repo,
            has_devops_plan: p.devops_plan.as_ref().is_some_and(|d| d.has_content()),
            daily_log_count: p.daily_logs.len(),
            created_at: p.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectDetailView {
    #[serde(flatten)]
    pub view: ProjectView,
    pub stage_outputs: HashMap<String, serde_json::Value>,
}

impl From<&Project> for ProjectDetailView {
    fn from(p: &Project) -> Self {
        let stage_outputs = p
            .stage_outputs
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.clone()))
            .collect();
        Self {
            view: ProjectView::from(p),
            stage_outputs,
        }
    }
}
