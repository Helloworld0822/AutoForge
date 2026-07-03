use crate::app::App;
use crate::domain::{DailyLog, Project};
use crate::error::Result;
use crate::services::daily_log::{append_entry, today_key, DailyEvent};
use bytes::Bytes;

/// 일별 경과 기록 + MD 아티팩트 저장 + Slack 알림
pub async fn record_daily_event(
    app: &App,
    project: &mut Project,
    event: DailyEvent,
) -> Result<DailyLog> {
    let log = append_entry(project, event);
    let project_id = project.id.0;
    let date = today_key();
    let key = format!("projects/{project_id}/daily/{date}.md");

    app.artifacts
        .put(
            key.as_str(),
            Bytes::from(log.markdown.clone()),
            "text/markdown",
        )
        .await?;

    app.store.save(project).await?;

    if let Some(slack) = &app.slack {
        let _ = slack
            .notify_daily_digest(project, &log, project.slack_message_ts.as_deref())
            .await;
    }

    Ok(log)
}
