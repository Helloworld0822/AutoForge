#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Diagnosis {
    pub(super) root_cause: String,
    pub(super) affected_files: Vec<String>,
    pub(super) recommended_fix: String,
    pub(super) risk: String,
    pub(super) additional_tests: Vec<String>,
}

impl Diagnosis {
    pub(super) fn bounded(mut self) -> Self {
        self.root_cause = self.root_cause.chars().take(512).collect();
        self.affected_files.truncate(8);
        self.affected_files = self
            .affected_files
            .into_iter()
            .map(|file| file.chars().take(160).collect())
            .collect();
        self.recommended_fix = self.recommended_fix.chars().take(512).collect();
        self.risk = self.risk.chars().take(256).collect();
        self.additional_tests.truncate(8);
        self.additional_tests = self
            .additional_tests
            .into_iter()
            .map(|test| test.chars().take(160).collect())
            .collect();
        self
    }
}
