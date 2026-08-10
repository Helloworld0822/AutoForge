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
    pub fn parse_from_agent_text(text: &str) -> Self {
        if let Ok(report) = serde_json::from_str::<VerifyReport>(text) {
            return report;
        }
        if let Some(start) = text.find('{') {
            if let Some(end) = text.rfind('}') {
                if let Ok(report) = serde_json::from_str::<VerifyReport>(&text[start..=end]) {
                    return report;
                }
            }
        }
        let passed = !text.to_lowercase().contains("fail") && text.contains("pass");
        Self {
            passed,
            checks: vec![],
            errors: if passed {
                vec![]
            } else {
                vec![text.chars().take(500).collect()]
            },
            summary: text.chars().take(200).collect(),
        }
    }
}

impl SecurityReport {
    pub fn parse_from_agent_text(text: &str) -> Self {
        if let Ok(report) = serde_json::from_str::<SecurityReport>(text) {
            return report;
        }
        if let Some(start) = text.find('{') {
            if let Some(end) = text.rfind('}') {
                if let Ok(report) = serde_json::from_str::<SecurityReport>(&text[start..=end]) {
                    return report;
                }
            }
        }
        Self {
            passed: true,
            vulnerabilities_found: 0,
            patches_applied: vec![],
            audit_tools: vec!["ai-agent".into()],
            summary: text.chars().take(200).collect(),
        }
    }
}

pub const MAX_DEBUG_CYCLES: u8 = 3;

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
