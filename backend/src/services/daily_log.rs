use chrono::Utc;

use crate::domain::{DailyLog, DailyLogEntry, PipelineState, Project, StageId, StageState};

#[derive(Debug, Clone)]
pub struct DailyEvent {
    pub event: &'static str,
    pub stage: Option<StageId>,
    pub message: String,
}

pub fn day_number(project: &Project, date: &str) -> u32 {
    let created = project.created_at.date_naive();
    let current = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .unwrap_or_else(|_| Utc::now().date_naive());
    (current - created).num_days().max(0) as u32 + 1
}

pub fn today_key() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

pub fn append_entry(project: &mut Project, event: DailyEvent) -> DailyLog {
    let date = today_key();
    let entry = DailyLogEntry {
        at: Utc::now(),
        event: event.event.into(),
        stage: event.stage,
        message: event.message,
        progress_percent: project.progress_percent(),
    };

    let day_num = day_number(project, &date);
    {
        let log = project
            .daily_logs
            .entry(date.clone())
            .or_insert_with(|| DailyLog {
                date: date.clone(),
                day_number: day_num,
                entries: Vec::new(),
                markdown: String::new(),
                updated_at: Utc::now(),
            });
        log.day_number = day_num;
        log.entries.push(entry);
        log.updated_at = Utc::now();
    }

    let snapshot = project.daily_logs.get(&date).cloned().unwrap();
    let markdown = render_markdown(project, &snapshot);
    project.daily_logs.get_mut(&date).unwrap().markdown = markdown;

    project.daily_logs.get(&date).cloned().unwrap()
}

pub fn render_markdown(project: &Project, log: &DailyLog) -> String {
    let state = format!("{:?}", project.state).to_lowercase();
    let completed: Vec<_> = StageId::all()
        .iter()
        .filter(|s| {
            matches!(
                project.stages.get(s),
                Some(StageState::Completed | StageState::Skipped)
            )
        })
        .map(|s| s.as_str())
        .collect();
    let running: Vec<_> = StageId::all()
        .iter()
        .filter(|s| project.stages.get(s) == Some(&StageState::Running))
        .map(|s| s.as_str())
        .collect();
    let failed: Vec<_> = StageId::all()
        .iter()
        .filter(|s| project.stages.get(s) == Some(&StageState::Failed))
        .map(|s| s.as_str())
        .collect();

    let mut md = format!(
        "# Day {} — {}\n\n\
         **프로젝트:** {}  \n\
         **진행률:** {}%  \n\
         **상태:** {}  \n",
        log.day_number,
        log.date,
        project.display_name(),
        project.progress_percent(),
        state,
    );

    if project.repo_url.is_some() {
        md.push_str(&format!(
            "**Repository:** {}  \n",
            project.repo_url.as_deref().unwrap_or("-")
        ));
    }
    if project
        .devops_plan
        .as_ref()
        .is_some_and(|d| d.has_content())
    {
        md.push_str("**DevOps 계획서:** 포함  \n");
    }
    md.push('\n');

    md.push_str("## 오늘 타임라인\n\n");
    md.push_str("| 시각 (UTC) | 이벤트 | 스테이지 | 진행률 | 메모 |\n");
    md.push_str("|------------|--------|----------|--------|------|\n");

    for e in &log.entries {
        let time = e.at.format("%H:%M").to_string();
        let stage = e
            .stage
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "-".into());
        let msg = e.message.replace('|', "\\|");
        md.push_str(&format!(
            "| {time} | {} | {stage} | {}% | {msg} |\n",
            e.event, e.progress_percent
        ));
    }

    md.push_str("\n## 스테이지 스냅샷\n\n");
    if !completed.is_empty() {
        md.push_str(&format!("- ✅ 완료: {}\n", completed.join(", ")));
    }
    if !running.is_empty() {
        md.push_str(&format!("- 🔄 실행 중: {}\n", running.join(", ")));
    }
    if !failed.is_empty() {
        md.push_str(&format!("- ❌ 실패: {}\n", failed.join(", ")));
    }
    if completed.is_empty() && running.is_empty() && failed.is_empty() {
        md.push_str("- ⏳ 아직 시작 전\n");
    }

    md.push_str("\n## 요약\n\n");
    match project.state {
        PipelineState::Completed => md.push_str("파이프라인이 성공적으로 완료되었습니다.\n"),
        PipelineState::Failed => {
            md.push_str("파이프라인이 실패했습니다. Debug/Verify 로그를 확인하세요.\n")
        }
        PipelineState::Running => md.push_str("파이프라인이 실행 중입니다.\n"),
        PipelineState::Cancelled => md.push_str("프로젝트가 취소되었습니다.\n"),
        PipelineState::Pending => md.push_str("프로젝트가 대기 중입니다.\n"),
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PipelineState, ProjectId};
    use crate::services::orchestrator::DagScheduler;
    use std::collections::HashMap;

    fn sample_project() -> Project {
        Project {
            id: ProjectId::new(),
            name: Some("Test".into()),
            repo_url: None,
            state: PipelineState::Running,
            stages: HashMap::new(),
            scheduler: DagScheduler::with_quality(ProjectId::new(), 3),
            pdf_bytes: None,
            devops_plan: None,
            stage_outputs: HashMap::new(),
            accumulated_artifacts: Vec::new(),
            slack_message_ts: None,
            created_at: Utc::now(),
            daily_logs: HashMap::new(),
        }
    }

    #[test]
    fn append_entry_builds_markdown() {
        let mut p = sample_project();
        let log = append_entry(
            &mut p,
            DailyEvent {
                event: "project_created",
                stage: None,
                message: "프로젝트 생성".into(),
            },
        );
        assert!(!log.markdown.is_empty());
        assert!(log.markdown.contains("Day 1"));
        assert_eq!(log.entries.len(), 1);
    }
}
