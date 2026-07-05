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
            model_id: "claude-4.6-sonnet-high-thinking".into(),
            mode: AgentMode::Agent,
            params: vec![],
        }
    }

    pub fn architect() -> Self {
        Self {
            model_id: "claude-fable-5-thinking-high".into(),
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
    /// 아키텍처 설계 질문에 대한 사용자 입력 대기
    AwaitingInput,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgrammingLanguage {
    Rust,
    TypeScript,
    Python,
    Go,
    Java,
    Kotlin,
    Swift,
    CSharp,
    Ruby,
    Php,
}

impl ProgrammingLanguage {
    pub fn all() -> &'static [ProgrammingLanguage] {
        &[
            ProgrammingLanguage::Rust,
            ProgrammingLanguage::TypeScript,
            ProgrammingLanguage::Python,
            ProgrammingLanguage::Go,
            ProgrammingLanguage::Java,
            ProgrammingLanguage::Kotlin,
            ProgrammingLanguage::Swift,
            ProgrammingLanguage::CSharp,
            ProgrammingLanguage::Ruby,
            ProgrammingLanguage::Php,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ProgrammingLanguage::Rust => "rust",
            ProgrammingLanguage::TypeScript => "typescript",
            ProgrammingLanguage::Python => "python",
            ProgrammingLanguage::Go => "go",
            ProgrammingLanguage::Java => "java",
            ProgrammingLanguage::Kotlin => "kotlin",
            ProgrammingLanguage::Swift => "swift",
            ProgrammingLanguage::CSharp => "csharp",
            ProgrammingLanguage::Ruby => "ruby",
            ProgrammingLanguage::Php => "php",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProgrammingLanguage::Rust => "Rust",
            ProgrammingLanguage::TypeScript => "TypeScript",
            ProgrammingLanguage::Python => "Python",
            ProgrammingLanguage::Go => "Go",
            ProgrammingLanguage::Java => "Java",
            ProgrammingLanguage::Kotlin => "Kotlin",
            ProgrammingLanguage::Swift => "Swift",
            ProgrammingLanguage::CSharp => "C#",
            ProgrammingLanguage::Ruby => "Ruby",
            ProgrammingLanguage::Php => "PHP",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "")
            .as_str()
        {
            "rust" => Some(ProgrammingLanguage::Rust),
            "typescript" | "ts" | "javascript" | "js" | "nodejs" | "node" => {
                Some(ProgrammingLanguage::TypeScript)
            }
            "python" | "py" => Some(ProgrammingLanguage::Python),
            "go" | "golang" => Some(ProgrammingLanguage::Go),
            "java" => Some(ProgrammingLanguage::Java),
            "kotlin" | "kt" => Some(ProgrammingLanguage::Kotlin),
            "swift" => Some(ProgrammingLanguage::Swift),
            "csharp" | "c#" | "dotnet" | "cs" => Some(ProgrammingLanguage::CSharp),
            "ruby" | "rb" => Some(ProgrammingLanguage::Ruby),
            "php" => Some(ProgrammingLanguage::Php),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    #[default]
    Auto,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureClarification {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<DateTime<Utc>>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureAnswerInput {
    pub id: String,
    pub answer: String,
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
    /// 사용자가 지정한 구현 언어 (language_mode=manual일 때)
    #[serde(default)]
    pub programming_language: Option<ProgrammingLanguage>,
    /// 자동 선택 vs 사용자 지정
    #[serde(default)]
    pub language_mode: LanguageMode,
    /// summarize 이후 확정된 구현 언어
    #[serde(default)]
    pub resolved_language: Option<ProgrammingLanguage>,
    /// 아키텍처 설계 단계 질의응답
    #[serde(default)]
    pub architecture_clarifications: Vec<ArchitectureClarification>,
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
    pub programming_language: Option<String>,
    pub resolved_language: Option<String>,
    pub language_mode: LanguageMode,
    pub awaiting_architecture_input: bool,
    pub architecture_clarifications: Vec<ArchitectureClarification>,
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
            programming_language: p.programming_language.map(|l| l.as_str().to_string()),
            resolved_language: p.resolved_language.map(|l| l.as_str().to_string()),
            language_mode: p.language_mode,
            awaiting_architecture_input: p.state == PipelineState::AwaitingInput,
            architecture_clarifications: p.architecture_clarifications.clone(),
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
