use serde::{Deserialize, Serialize};

/// 검증 리포트 — Verify 스테이지 산출물
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    pub passed: bool,
    pub checks: Vec<CheckResult>,
    pub errors: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub output: Option<String>,
}

/// 디버깅 리포트 — Debug 스테이지 산출물
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugReport {
    pub fixes_applied: Vec<String>,
    pub files_changed: Vec<String>,
    pub summary: String,
    pub resolved_errors: usize,
}

/// 보안 패치 리포트 — SecurityPatch 스테이지 산출물
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityReport {
    pub passed: bool,
    pub vulnerabilities_found: usize,
    pub patches_applied: Vec<SecurityPatch>,
    pub audit_tools: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPatch {
    pub id: String,
    pub severity: String,
    pub package: String,
    pub action: String,
}

impl VerifyReport {
    /// fail-closed: 빈/파싱 불가 텍스트는 거부하고, checks 근거 없는 `passed: true`는 거부한다.
    pub fn parse_from_agent_text(text: &str) -> std::result::Result<Self, String> {
        let mut report = parse_strict::<VerifyReport>(text, "verify_report")?;
        if report.passed {
            let evidence_ok = !report.checks.is_empty()
                && report.checks.iter().all(|check| check.passed)
                && report.errors.is_empty();
            if !evidence_ok {
                report.passed = false;
                report
                    .errors
                    .push("verify claimed passed without supporting checks".into());
            }
        }
        Ok(report)
    }
}

impl SecurityReport {
    /// fail-closed: 빈/파싱 불가 텍스트와 빈 요약의 `passed: true`를 거부한다.
    pub fn parse_from_agent_text(text: &str) -> std::result::Result<Self, String> {
        let mut report = parse_strict::<SecurityReport>(text, "security_report")?;
        if report.passed && report.summary.trim().is_empty() {
            report.passed = false;
        }
        Ok(report)
    }
}

fn parse_strict<T: for<'de> Deserialize<'de>>(
    text: &str,
    label: &str,
) -> std::result::Result<T, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{label}: empty agent output"));
    }
    if let Ok(parsed) = serde_json::from_str::<T>(trimmed) {
        return Ok(parsed);
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            if let Ok(parsed) = serde_json::from_str::<T>(&trimmed[start..=end]) {
                return Ok(parsed);
            }
        }
    }
    Err(format!(
        "{label}: agent output is not a valid JSON report: {}",
        trimmed.chars().take(200).collect::<String>()
    ))
}

pub const MAX_DEBUG_CYCLES: u8 = 4;

pub const VERIFY_CHECKS: &[&str] = &[
    "cargo check",
    "cargo test",
    "cargo clippy -- -D warnings",
    "cargo fmt --check",
];

pub const SECURITY_CHECKS: &[&str] = &[
    "cargo audit",
    "dependency vulnerability scan",
    "OWASP Top 10 static analysis",
    "secrets/credential leak scan",
    "insecure crypto / TLS config review",
];

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_VERIFY: &str = r#"{"passed":true,"checks":[{"name":"cargo test","passed":true,"output":"ok"}],"errors":[],"summary":"all checks passed"}"#;

    #[test]
    fn verify_rejects_empty_and_unparseable_output() {
        assert!(VerifyReport::parse_from_agent_text("").is_err());
        assert!(VerifyReport::parse_from_agent_text("all checks pass, no fail detected").is_err());
        assert!(VerifyReport::parse_from_agent_text("{\"passed\":").is_err());
    }

    #[test]
    fn verify_accepts_valid_json_with_supporting_checks() {
        let report = VerifyReport::parse_from_agent_text(VALID_VERIFY).expect("valid report");
        assert!(report.passed);
        assert_eq!(report.checks.len(), 1);
    }

    #[test]
    fn verify_rejects_passed_without_checks() {
        let report = VerifyReport::parse_from_agent_text(
            r#"{"passed":true,"checks":[],"errors":[],"summary":"looks fine"}"#,
        )
        .expect("valid json");
        assert!(!report.passed);
        assert!(!report.errors.is_empty());
    }

    #[test]
    fn verify_rejects_passed_with_failing_check_or_errors() {
        let failing_check = VerifyReport::parse_from_agent_text(
            r#"{"passed":true,"checks":[{"name":"cargo test","passed":false,"output":"x"}],"errors":[],"summary":"s"}"#,
        )
        .expect("valid json");
        assert!(!failing_check.passed);

        let with_errors = VerifyReport::parse_from_agent_text(
            r#"{"passed":true,"checks":[{"name":"cargo test","passed":true,"output":"x"}],"errors":["boom"],"summary":"s"}"#,
        )
        .expect("valid json");
        assert!(!with_errors.passed);
    }

    #[test]
    fn verify_preserves_valid_failure_report() {
        let report = VerifyReport::parse_from_agent_text(
            r#"{"passed":false,"checks":[{"name":"cargo test","passed":false,"output":"1 failed"}],"errors":["assert"],"summary":"failed"}"#,
        )
        .expect("valid json");
        assert!(!report.passed);
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn security_rejects_empty_prose_and_defaults_fail_closed() {
        assert!(SecurityReport::parse_from_agent_text("").is_err());
        assert!(SecurityReport::parse_from_agent_text("no vulnerabilities found, pass").is_err());
    }

    #[test]
    fn security_requires_summary_for_passed() {
        let report = SecurityReport::parse_from_agent_text(
            r#"{"passed":true,"vulnerabilities_found":0,"patches_applied":[],"audit_tools":[],"summary":""}"#,
        )
        .expect("valid json");
        assert!(!report.passed);
    }

    #[test]
    fn security_accepts_valid_report() {
        let report = SecurityReport::parse_from_agent_text(
            r#"{"passed":true,"vulnerabilities_found":1,"patches_applied":[{"id":"CVE-1","severity":"high","package":"x","action":"upgrade"}],"audit_tools":["cargo audit"],"summary":"patched"}"#,
        )
        .expect("valid json");
        assert!(report.passed);
        assert_eq!(report.patches_applied.len(), 1);
    }
}
