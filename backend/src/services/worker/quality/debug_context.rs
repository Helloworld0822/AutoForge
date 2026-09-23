use crate::domain::{ArtifactRef, ProjectId, StageId};
use crate::error::{AutoForgeError, Result};
use crate::services::artifacts::ArtifactStore;
use crate::services::context_manager::{ContextManager, ErrorCompressionLimits, ErrorEntry};
use crate::services::quality::{DebugReport, VerifyReport};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashSet;

const MAX_EVIDENCE_BYTES: usize = 8 * 1024;
const MAX_SOURCE_BYTES: usize = 4 * 1024;
const MAX_ISSUES: usize = 8;
const MAX_FAILED_CHECKS: usize = 6;
const MAX_SOURCE_NAME_BYTES: usize = 64;
const MAX_FILE_BYTES: usize = 160;
const MAX_CODE_BYTES: usize = 48;
const MAX_MESSAGE_BYTES: usize = 256;
const MAX_PREVIOUS_ITEMS: usize = 4;
const MAX_PREVIOUS_ITEM_BYTES: usize = 160;
const MAX_PREVIOUS_SUMMARY_BYTES: usize = 256;

#[derive(Debug, Serialize)]
pub(super) struct DebugEvidence {
    passed: bool,
    failed_checks: Vec<String>,
    errors: Vec<DebugIssue>,
    previous_debug: Option<PreviousDebugEvidence>,
}

#[derive(Debug, Serialize)]
struct DebugIssue {
    source: String,
    file: Option<String>,
    line: Option<u32>,
    code: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
struct PreviousDebugEvidence {
    fixes_applied: Vec<String>,
    files_changed: Vec<String>,
    summary: String,
    resolved_errors: usize,
}

impl DebugEvidence {
    pub(super) fn json(&self) -> Result<String> {
        let json = serde_json::to_string(self).map_err(|error| {
            AutoForgeError::Internal(format!("serialize debug evidence: {error}"))
        })?;
        if json.len() > MAX_EVIDENCE_BYTES {
            return Err(AutoForgeError::Internal(format!(
                "debug evidence exceeded {MAX_EVIDENCE_BYTES} byte budget"
            )));
        }
        Ok(json)
    }
}

pub(super) async fn load(
    store: &dyn ArtifactStore,
    project_id: &ProjectId,
    inputs: &[ArtifactRef],
) -> Result<DebugEvidence> {
    let verify_key = format!("projects/{}/verify/verify_report.json", project_id.0);
    let verify_report: VerifyReport = parse_artifact(
        &store.get(&verify_key).await.map_err(|error| {
            stage_failure(format!(
                "required verify_report.json artifact is unavailable: {error}"
            ))
        })?,
        "verify_report.json",
    )?;
    let debug_key = format!("projects/{}/debug/debug_report.json", project_id.0);
    let previous_debug = inputs
        .iter()
        .rev()
        .find(|artifact| artifact.key == debug_key)
        .map(|artifact| artifact.key.as_str());
    let previous_debug = match previous_debug {
        Some(key) => Some(bound_previous_debug(parse_artifact(
            &store.get(key).await.map_err(|error| {
                stage_failure(format!(
                    "previous debug_report.json artifact is unavailable: {error}"
                ))
            })?,
            "debug_report.json",
        )?)),
        None => None,
    };

    Ok(build(verify_report, previous_debug))
}

fn parse_artifact<T: DeserializeOwned>(bytes: &[u8], name: &str) -> Result<T> {
    let content = std::str::from_utf8(bytes)
        .map_err(|error| stage_failure(format!("{name} is not valid UTF-8: {error}")))?;
    serde_json::from_str(content)
        .map_err(|error| stage_failure(format!("{name} is malformed JSON: {error}")))
}

fn stage_failure(message: String) -> AutoForgeError {
    AutoForgeError::StageFailed {
        stage: StageId::Debug,
        message,
    }
}

fn build(report: VerifyReport, previous_debug: Option<PreviousDebugEvidence>) -> DebugEvidence {
    let mut failed_checks = Vec::new();
    let mut issues = Vec::new();
    let mut seen = HashSet::new();

    for error in report.errors {
        add_issues(&mut issues, &mut seen, "verify_report.errors", &error);
    }
    for check in report.checks.into_iter().filter(|check| !check.passed) {
        if failed_checks.len() < MAX_FAILED_CHECKS {
            failed_checks.push(bound(&check.name, MAX_SOURCE_NAME_BYTES));
        }
        if let Some(output) = check.output {
            add_issues(&mut issues, &mut seen, &check.name, &output);
        }
    }

    DebugEvidence {
        passed: report.passed,
        failed_checks,
        errors: issues,
        previous_debug,
    }
}

fn add_issues(
    issues: &mut Vec<DebugIssue>,
    seen: &mut HashSet<String>,
    source: &str,
    output: &str,
) {
    if issues.len() == MAX_ISSUES || output.trim().is_empty() {
        return;
    }
    let source = bound(source, MAX_SOURCE_NAME_BYTES);
    let compressed = ContextManager::compress_errors_with_limits(
        &source,
        1,
        &bounded_output(output),
        ErrorCompressionLimits {
            max_output_bytes: MAX_SOURCE_BYTES,
            max_errors: MAX_ISSUES,
            max_message_bytes: MAX_MESSAGE_BYTES,
        },
    );
    let entries = if compressed.errors.is_empty() {
        vec![ErrorEntry {
            file: None,
            line: None,
            code: None,
            message: bound(output.trim(), MAX_MESSAGE_BYTES),
        }]
    } else {
        compressed.errors
    };

    for entry in entries {
        if issues.len() == MAX_ISSUES {
            return;
        }
        let issue = bounded_issue(&source, entry);
        let fingerprint = format!(
            "{:?}|{:?}|{:?}|{}",
            issue.file, issue.line, issue.code, issue.message
        );
        if seen.insert(fingerprint) {
            issues.push(issue);
        }
    }
}

fn bounded_issue(source: &str, entry: ErrorEntry) -> DebugIssue {
    DebugIssue {
        source: bound(source, MAX_SOURCE_NAME_BYTES),
        file: entry.file.map(|file| bound(&file, MAX_FILE_BYTES)),
        line: entry.line,
        code: entry.code.map(|code| bound(&code, MAX_CODE_BYTES)),
        message: bound(&entry.message, MAX_MESSAGE_BYTES),
    }
}

fn bound_previous_debug(report: DebugReport) -> PreviousDebugEvidence {
    PreviousDebugEvidence {
        fixes_applied: report
            .fixes_applied
            .iter()
            .take(MAX_PREVIOUS_ITEMS)
            .map(|item| bound(item, MAX_PREVIOUS_ITEM_BYTES))
            .collect(),
        files_changed: report
            .files_changed
            .iter()
            .take(MAX_PREVIOUS_ITEMS)
            .map(|item| bound(item, MAX_PREVIOUS_ITEM_BYTES))
            .collect(),
        summary: bound(&report.summary, MAX_PREVIOUS_SUMMARY_BYTES),
        resolved_errors: report.resolved_errors,
    }
}

fn bounded_output(output: &str) -> String {
    let mut result = String::new();
    let mut seen = HashSet::new();
    for line in output.lines() {
        if !seen.insert(line) {
            continue;
        }
        let remaining = MAX_SOURCE_BYTES.saturating_sub(result.len());
        if remaining <= 1 {
            break;
        }
        result.push_str(&bound(line, remaining - 1));
        result.push('\n');
    }
    result
}

fn bound(value: &str, max_bytes: usize) -> String {
    value
        .chars()
        .scan(0usize, |bytes, character| {
            let width = character.len_utf8();
            if (*bytes).saturating_add(width) > max_bytes {
                None
            } else {
                *bytes += width;
                Some(character)
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "debug_context_tests.rs"]
mod tests;
