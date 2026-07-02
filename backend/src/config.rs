use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub cursor_api_key: String,
    pub stitch_api_key: String,
    pub artifacts_endpoint: String,
    pub artifacts_bucket: String,
    pub default_repo_url: Option<String>,
    pub max_debug_cycles: u8,
    /// Redis URL — 설정 시 MQ 모드 활성화
    pub redis_url: String,
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
            artifacts_endpoint: env::var("ARTIFACTS_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:9000".into()),
            artifacts_bucket: env::var("ARTIFACTS_BUCKET")
                .unwrap_or_else(|_| "autoforge".into()),
            default_repo_url: env::var("DEFAULT_REPO_URL").ok(),
            max_debug_cycles: env::var("MAX_DEBUG_CYCLES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            slack_webhook_url: env::var("SLACK_WEBHOOK_URL").ok(),
            slack_bot_token: env::var("SLACK_BOT_TOKEN").ok(),
            slack_channel: env::var("SLACK_CHANNEL").ok(),
            queue_commands_stream: env::var("QUEUE_COMMANDS_STREAM")
                .unwrap_or_else(|_| "autoforge:commands".into()),
            queue_events_stream: env::var("QUEUE_EVENTS_STREAM")
                .unwrap_or_else(|_| "autoforge:events".into()),
            queue_consumer_group: env::var("QUEUE_CONSUMER_GROUP")
                .unwrap_or_else(|_| "autoforge".into()),
            worker_concurrency: env::var("WORKER_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            podman_worker_image: env::var("PODMAN_WORKER_IMAGE")
                .unwrap_or_else(|_| "localhost/autoforge:latest".into()),
            github_token: env::var("GITHUB_TOKEN").ok(),
            github_org: env::var("GITHUB_ORG").ok(),
            github_auto_merge: env::var("GITHUB_AUTO_MERGE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(true),
            public_url: env::var("PUBLIC_URL")
                .unwrap_or_else(|_| "http://localhost".into()),
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

    /// Redis + MQ 스트림 사용 여부
    pub fn message_queue_enabled(&self) -> bool {
        env::var("MESSAGE_QUEUE_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or_else(|_| !self.redis_url.is_empty())
    }

    pub fn slack_enabled(&self) -> bool {
        self.slack_webhook_url.is_some()
            || (self.slack_bot_token.is_some() && self.slack_channel.is_some())
    }
}
