use std::env;

fn optional_non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub cursor_api_key: String,
    pub stitch_api_key: String,
    /// Stitch AI 생성(generate_screen 등)용 OAuth Bearer 토큰. API 키만으로는 생성 불가.
    pub stitch_access_token: String,
    /// Bearer 인증 시 GCP 과금 프로젝트 (X-Goog-User-Project)
    pub google_cloud_project: Option<String>,
    /// 파이프라인 산출물 및 이미지 호스팅 파일을 저장할 로컬 디렉터리
    pub artifacts_dir: String,
    /// 업로드 이미지 최대 크기 (bytes, 기본 10MB)
    pub max_image_bytes: usize,
    pub default_repo_url: Option<String>,
    pub max_debug_cycles: u8,
    /// Redis URL — 프로젝트 스토어 및 실시간 알림(pub/sub)
    pub redis_url: String,
    /// RabbitMQ AMQP URL — 파이프라인 커맨드/이벤트 큐
    pub rabbitmq_url: String,
    pub slack_webhook_url: Option<String>,
    pub slack_bot_token: Option<String>,
    pub slack_channel: Option<String>,
    pub queue_commands_stream: String,
    pub queue_events_stream: String,
    pub queue_consumer_group: String,
    pub worker_concurrency: usize,
    pub podman_worker_image: String,
    /// GitHub Personal Access Token (repo 권한)
    pub github_token: Option<String>,
    /// GitHub Organization (없으면 사용자 계정)
    pub github_org: Option<String>,
    /// SecurityPatch 통과 후 PR 자동 머지
    pub github_auto_merge: bool,
    /// 웹 대시보드 공개 URL (Slack 링크용)
    pub public_url: String,
    /// 설정 시 /v1/* API에 `Authorization: Bearer <key>` 인증을 강제한다.
    /// 미설정 시 인증 없이 API가 열려있으므로(개발용) 운영 환경에서는 반드시 설정할 것.
    pub api_key: Option<String>,
    /// CORS 허용 오리진 (콤마 구분). 미설정 시 모든 오리진 허용(개발용).
    pub cors_allowed_origins: Option<Vec<String>>,
    /// 업로드 최대 크기 (bytes, 기본 50MB)
    pub max_upload_bytes: usize,
    /// 파이프라인 산출물 생성 시 프로젝트별 git 자동 커밋
    pub git_auto_commit: bool,
    /// 일일 git push 시각 (UTC, 0–23)
    pub git_daily_push_hour_utc: u8,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            cursor_api_key: env::var("CURSOR_API_KEY").unwrap_or_default(),
            stitch_api_key: env::var("STITCH_API_KEY").unwrap_or_default(),
            stitch_access_token: env::var("STITCH_ACCESS_TOKEN").unwrap_or_default(),
            google_cloud_project: optional_non_empty(
                env::var("GOOGLE_CLOUD_PROJECT")
                    .or_else(|_| env::var("GCLOUD_PROJECT"))
                    .ok(),
            ),
            artifacts_dir: env::var("ARTIFACTS_DIR").unwrap_or_else(|_| "./data/artifacts".into()),
            max_image_bytes: env::var("MAX_IMAGE_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10 * 1024 * 1024),
            default_repo_url: env::var("DEFAULT_REPO_URL").ok(),
            max_debug_cycles: env::var("MAX_DEBUG_CYCLES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            rabbitmq_url: env::var("RABBITMQ_URL")
                .unwrap_or_else(|_| "amqp://guest:guest@127.0.0.1:5672/".into()),
            slack_webhook_url: env::var("SLACK_WEBHOOK_URL").ok(),
            slack_bot_token: env::var("SLACK_BOT_TOKEN").ok(),
            slack_channel: env::var("SLACK_CHANNEL").ok(),
            queue_commands_stream: env::var("QUEUE_COMMANDS_STREAM")
                .unwrap_or_else(|_| "autoforge.commands".into()),
            queue_events_stream: env::var("QUEUE_EVENTS_STREAM")
                .unwrap_or_else(|_| "autoforge.events".into()),
            queue_consumer_group: env::var("QUEUE_CONSUMER_GROUP")
                .unwrap_or_else(|_| "autoforge".into()),
            worker_concurrency: env::var("WORKER_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            podman_worker_image: env::var("PODMAN_WORKER_IMAGE")
                .unwrap_or_else(|_| "localhost/autoforge:latest".into()),
            github_token: optional_non_empty(env::var("GITHUB_TOKEN").ok()),
            github_org: optional_non_empty(env::var("GITHUB_ORG").ok()),
            github_auto_merge: env::var("GITHUB_AUTO_MERGE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(true),
            public_url: env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost".into()),
            api_key: env::var("API_KEY").ok().filter(|v| !v.is_empty()),
            cors_allowed_origins: env::var("CORS_ALLOWED_ORIGINS").ok().map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }),
            max_upload_bytes: env::var("MAX_UPLOAD_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50 * 1024 * 1024),
            git_auto_commit: env::var("GIT_AUTO_COMMIT")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(true),
            git_daily_push_hour_utc: env::var("GIT_DAILY_PUSH_HOUR_UTC")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(23),
        }
    }

    pub fn github_enabled(&self) -> bool {
        self.github_token
            .as_ref()
            .map(|t| !t.is_empty())
            .unwrap_or(false)
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// RabbitMQ 기반 분산 파이프라인 사용 여부
    /// `MESSAGE_QUEUE_ENABLED`이 명시적으로 설정된 경우에만 그 값을 따른다.
    pub fn message_queue_enabled(&self) -> bool {
        env::var("MESSAGE_QUEUE_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    pub fn slack_enabled(&self) -> bool {
        self.slack_webhook_url.is_some()
            || (self.slack_bot_token.is_some() && self.slack_channel.is_some())
    }

    pub fn auth_enabled(&self) -> bool {
        self.api_key.is_some()
    }

    /// 필수/권장 설정 누락을 점검하고 경고를 남긴다. 서버는 계속 기동하되
    /// 운영자가 로그에서 즉시 문제를 인지할 수 있도록 한다.
    pub fn validate_and_warn(&self) {
        if self.cursor_api_key.is_empty() {
            tracing::warn!("CURSOR_API_KEY is not set — Summarize/Architect/Implement/Verify/Debug stages will fail");
        }
        if self.stitch_api_key.is_empty() && self.stitch_access_token.is_empty() {
            tracing::warn!("STITCH_API_KEY / STITCH_ACCESS_TOKEN not set — Design stage will fail");
        } else if !self.stitch_api_key.is_empty() && self.stitch_access_token.is_empty() {
            tracing::warn!(
                "STITCH_ACCESS_TOKEN is not set — Stitch API key alone cannot run generate_screen; \
                 set STITCH_ACCESS_TOKEN from `gcloud auth application-default print-access-token`"
            );
        }
        if !self.auth_enabled() {
            tracing::warn!(
                "API_KEY is not set — the REST API is running WITHOUT authentication. \
                 Set API_KEY before exposing this service publicly."
            );
        }
        if self.github_enabled() && self.github_token.as_deref().unwrap_or_default().len() < 10 {
            tracing::warn!("GITHUB_TOKEN looks malformed (too short) — GitHub automation may fail");
        }
        if self.message_queue_enabled() && self.rabbitmq_url.is_empty() {
            tracing::warn!("MESSAGE_QUEUE_ENABLED is true but RABBITMQ_URL is empty");
        }
        if self.public_url.starts_with("http://localhost") {
            tracing::warn!(
                "PUBLIC_URL is set to localhost — Slack notification links will not work outside this machine"
            );
        }
    }
}
