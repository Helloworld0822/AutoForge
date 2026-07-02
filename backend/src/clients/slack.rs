use crate::config::Config;
use crate::domain::{Project, StageId, StageState};
use crate::error::{AutoForgeError, Result};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tracing::{debug, warn};

pub struct SlackNotifier {
    http: Client,
    webhook_url: Option<String>,
    bot_token: Option<String>,
    channel: Option<String>,
    public_url: String,
}

impl SlackNotifier {
    pub fn new(config: &Config) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AutoForgeError::Slack(e.to_string()))?;
        Ok(Self {
            http,
            webhook_url: config.slack_webhook_url.clone(),
            bot_token: config.slack_bot_token.clone(),
            channel: config.slack_channel.clone(),
            public_url: config.public_url.clone(),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.webhook_url.is_some() || (self.bot_token.is_some() && self.channel.is_some())
    }

    pub async fn notify_daily_digest(
        &self,
        project: &Project,
        log: &crate::domain::DailyLog,
        thread_ts: Option<&str>,
    ) -> Result<()> {
        let web_link = format!(
            "{}/projects/{}#daily-{}",
            self.public_url, project.id.0, log.date
        );
        let preview = if log.markdown.len() > 1200 {
            format!("{}…", &log.markdown[..1200])
        } else {
            log.markdown.clone()
        };
        let text = format!(
            "📅 *Day {} 일일 경과* — `{}` ({})\n\
             진행률: *{}%* | 이벤트: {}건\n\
             <{web_link}|웹에서 전체 보기>\n\n\
             ```markdown\n{preview}\n```",
            log.day_number,
            project.display_name(),
            log.date,
            project.progress_percent(),
            log.entries.len(),
        );
        self.post_message(&text, thread_ts).await?;
        Ok(())
    }

    pub async fn notify_project_created(&self, project: &Project) -> Result<Option<String>> {
        let pct = project.progress_percent();
        let text = format!(
            "🚀 *AutoForge* — 프로젝트 `{}` 시작\n{}\n진행률: {}%",
            project.display_name(),
            stage_bar(project),
            pct
        );
        self.post_message(&text, None).await
    }

    pub async fn notify_stage_update(
        &self,
        project: &Project,
        stage: StageId,
        status: &str,
        thread_ts: Option<&str>,
    ) -> Result<()> {
        let emoji = match status {
            "running" => "🔄",
            "completed" => "✅",
            "failed" => "❌",
            _ => "⏳",
        };
        let pct = project.progress_percent();
        let text = format!(
            "{emoji} *{}* — {status}\n{}\n진행률: *{pct}%* | 상태: `{state}`",
            stage.as_str(),
            stage_bar(project),
            state = format!("{:?}", project.state).to_lowercase()
        );
        self.post_message(&text, thread_ts).await?;
        Ok(())
    }

    pub async fn notify_pipeline_done(&self, project: &Project, thread_ts: Option<&str>) -> Result<()> {
        let text = format!(
            "🎉 *파이프라인 완료* — `{}`\n{}\n진행률: 100%",
            project.display_name(),
            stage_bar(project)
        );
        self.post_message(&text, thread_ts).await?;
        Ok(())
    }

    pub async fn notify_pipeline_failed(
        &self,
        project: &Project,
        error: &str,
        thread_ts: Option<&str>,
    ) -> Result<()> {
        let text = format!(
            "💥 *파이프라인 실패* — `{}`\n{}\n오류: ```{error}```",
            project.display_name(),
            stage_bar(project)
        );
        self.post_message(&text, thread_ts).await?;
        Ok(())
    }

    async fn post_message(&self, text: &str, thread_ts: Option<&str>) -> Result<Option<String>> {
        if !self.is_enabled() {
            debug!("slack disabled, skipping notification");
            return Ok(None);
        }

        if let Some(url) = &self.webhook_url {
            let body = json!({ "text": text });
            let resp = self
                .http
                .post(url)
                .json(&body)
                .send()
                .await
                .map_err(|e| AutoForgeError::Slack(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                warn!(%status, %body, "slack webhook failed");
            }
            return Ok(None);
        }

        if let (Some(token), Some(channel)) = (&self.bot_token, &self.channel) {
            let mut body = json!({
                "channel": channel,
                "text": text,
                "blocks": [
                    {
                        "type": "section",
                        "text": { "type": "mrkdwn", "text": text }
                    }
                ]
            });
            if let Some(ts) = thread_ts {
                body["thread_ts"] = json!(ts);
            }
            let resp = self
                .http
                .post("https://slack.com/api/chat.postMessage")
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .map_err(|e| AutoForgeError::Slack(e.to_string()))?;
            let result: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AutoForgeError::Slack(e.to_string()))?;
            if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                return Err(AutoForgeError::Slack(
                    result
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .into(),
                ));
            }
            return Ok(result
                .get("ts")
                .and_then(|v| v.as_str())
                .map(String::from));
        }

        Ok(None)
    }
}

fn stage_bar(project: &Project) -> String {
    StageId::all()
        .iter()
        .map(|&stage| {
            let st = project.stages.get(&stage).copied().unwrap_or(StageState::Queued);
            let icon = match st {
                StageState::Completed | StageState::Skipped => "✅",
                StageState::Running => "🔄",
                StageState::Failed => "❌",
                StageState::Queued => "⏳",
            };
            format!("{icon} {}", stage.as_str())
        })
        .collect::<Vec<_>>()
        .join("  ")
}
