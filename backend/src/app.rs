use crate::config::Config;
use crate::domain::{PipelineState, Project, ProjectId, StageId, StageState};
use crate::services::artifacts::{ArtifactStore, S3ArtifactStore};
use crate::services::orchestrator::DagScheduler;
use crate::services::queue::MessageQueue;
use crate::services::store::{MemoryStore, ProjectStore, RedisProjectStore};
use crate::clients::slack::SlackNotifier;
use crate::clients::github::GitHubClient;
use std::collections::HashMap;
use std::sync::Arc;

/// 애플리케이션 전역 상태
pub struct App {
    pub config: Config,
    pub store: Arc<dyn ProjectStore>,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub cursor: Arc<crate::clients::cursor::CursorClient>,
    pub stitch: Arc<crate::clients::stitch::StitchClient>,
    pub queue: Option<Arc<MessageQueue>>,
    pub slack: Option<Arc<SlackNotifier>>,
    pub github: Option<Arc<GitHubClient>>,
}

impl App {
    /// 인메모리 모드 (단일 프로세스, MQ 없음)
    pub fn new(config: Config) -> crate::Result<Self> {
        let store: Arc<dyn ProjectStore> = Arc::new(MemoryStore::new());
        Self::build(config, store, None, None)
    }

    /// Redis MQ 모드 (Podman 멀티 컨테이너)
    pub async fn connect(config: Config) -> crate::Result<Self> {
        let store: Arc<dyn ProjectStore> =
            Arc::new(RedisProjectStore::connect(&config.redis_url).await?);
        let queue = Some(MessageQueue::connect(&config).await?);
        let slack = if config.slack_enabled() {
            Some(Arc::new(SlackNotifier::new(&config)?))
        } else {
            None
        };
        Self::build(config, store, queue, slack)
    }

    fn build(
        config: Config,
        store: Arc<dyn ProjectStore>,
        queue: Option<Arc<MessageQueue>>,
        slack: Option<Arc<SlackNotifier>>,
    ) -> crate::Result<Self> {
        let cursor = Arc::new(crate::clients::cursor::CursorClient::new(
            config.cursor_api_key.clone(),
        )?);
        let stitch = Arc::new(crate::clients::stitch::StitchClient::new(
            config.stitch_api_key.clone(),
        )?);
        let artifacts: Arc<dyn ArtifactStore> = Arc::new(S3ArtifactStore::new(
            &config.artifacts_endpoint,
            &config.artifacts_bucket,
        ));

        let slack = slack.or_else(|| {
            if config.slack_enabled() {
                SlackNotifier::new(&config).ok().map(Arc::new)
            } else {
                None
            }
        });

        let github = if config.github_enabled() {
            GitHubClient::new(
                config.github_token.clone().unwrap_or_default(),
                config.github_org.clone(),
                config.github_auto_merge,
            )
            .ok()
            .map(Arc::new)
        } else {
            None
        };

        Ok(Self {
            config,
            store,
            artifacts,
            cursor,
            stitch,
            queue,
            slack,
            github,
        })
    }

    pub async fn create_project(&self, name: Option<String>, repo_url: Option<String>) -> Project {
        let id = ProjectId::new();
        let project = Project {
            id: id.clone(),
            name,
            repo_url,
            state: PipelineState::Pending,
            stages: StageId::all()
                .iter()
                .map(|&stage| (stage, StageState::Queued))
                .collect(),
            scheduler: DagScheduler::with_quality(id, self.config.max_debug_cycles),
            pdf_bytes: None,
            devops_plan: None,
            stage_outputs: HashMap::new(),
            accumulated_artifacts: Vec::new(),
            slack_message_ts: None,
            created_at: chrono::Utc::now(),
            daily_logs: HashMap::new(),
        };
        let _ = self.store.save(&project).await;
        project
    }

    pub async fn get_project(&self, id: uuid::Uuid) -> Option<Project> {
        self.store.get(id).await.ok().flatten()
    }

    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }
}
