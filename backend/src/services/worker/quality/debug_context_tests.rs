use super::*;
use bytes::Bytes;

#[test]
fn includes_actual_verification_failures_instead_of_only_counts() {
    // Given: failed checks and report errors whose details are absent from stage metadata.
    let report = VerifyReport {
        passed: false,
        checks: vec![crate::services::quality::report::CheckResult {
            name: "cargo test".into(),
            passed: false,
            output: Some("error[E0425]: cannot find value `total`".into()),
        }],
        errors: vec!["error[E0425]: cannot find value `total`".into()],
        summary: "one failure".into(),
    };

    // When: debug evidence is derived from the report artifact.
    let evidence = build(report, None);

    // Then: the actual distinct failure and its failed check are retained.
    assert_eq!(evidence.failed_checks, vec!["cargo test"]);
    assert_eq!(evidence.errors.len(), 1);
    assert!(evidence
        .errors
        .iter()
        .any(|issue| issue.message.contains("E0425")));
}

#[test]
fn bounds_and_deduplicates_debug_evidence() {
    // Given: repeated large verification output and a verbose previous debug report.
    let report = VerifyReport {
        passed: false,
        checks: vec![crate::services::quality::report::CheckResult {
            name: "cargo test".into(),
            passed: false,
            output: Some("error: repeated failure\n".repeat(1_000)),
        }],
        errors: vec!["error: repeated failure\n".repeat(1_000)],
        summary: String::new(),
    };
    let previous = bound_previous_debug(DebugReport {
        fixes_applied: vec!["fix".repeat(100); 10],
        files_changed: vec!["src/lib.rs".repeat(100); 10],
        summary: "summary".repeat(100),
        resolved_errors: 1,
    });

    // When: the evidence is compressed for both diagnosis and Cursor.
    let evidence = build(report, Some(previous));
    let json = evidence.json().expect("bounded JSON");

    // Then: duplicate errors collapse and the serialized context stays within budget.
    assert_eq!(evidence.errors.len(), 1);
    assert!(json.len() <= MAX_EVIDENCE_BYTES);
}

#[test]
fn bounds_error_compression_before_prompt_serialization() {
    // Given: a failed command with more repeated error output than a debug prompt may carry.
    let output = "error[E1000]: oversized diagnostic payload\n".repeat(100);

    // When: ContextManager compresses it with the debug evidence limits.
    let compressed = ContextManager::compress_errors_with_limits(
        "cargo test",
        1,
        &output,
        ErrorCompressionLimits {
            max_output_bytes: 128,
            max_errors: 2,
            max_message_bytes: 32,
        },
    );

    // Then: the compressor itself enforces both record and UTF-8 byte budgets.
    assert!(compressed.errors.len() <= 2);
    assert!(compressed
        .errors
        .iter()
        .all(|error| error.message.len() <= 32));
}

#[tokio::test]
async fn fails_when_verify_artifact_is_missing_or_malformed() {
    // Given: a debug execution without a verify artifact, then a malformed one.
    let project_id = ProjectId::new();
    let missing = TestArtifacts::default();
    let malformed = TestArtifacts::with_verify(&project_id, br#"{"passed":false,"checks":"bad"}"#);

    // When: each artifact boundary is loaded.
    let missing_result = load(&missing, &project_id, &[]).await;
    let malformed_result = load(&malformed, &project_id, &[]).await;

    // Then: neither condition can silently fall back to metadata counts.
    assert!(missing_result.is_err());
    assert!(malformed_result.is_err());
}

#[derive(Default)]
struct TestArtifacts {
    objects: std::collections::HashMap<String, Bytes>,
}

impl TestArtifacts {
    fn with_verify(project_id: &ProjectId, report: &[u8]) -> Self {
        let mut objects = std::collections::HashMap::new();
        objects.insert(
            format!("projects/{}/verify/verify_report.json", project_id.0),
            Bytes::copy_from_slice(report),
        );
        Self { objects }
    }
}

#[async_trait::async_trait]
impl ArtifactStore for TestArtifacts {
    async fn put(&self, _key: &str, _data: Bytes, _content_type: &str) -> Result<ArtifactRef> {
        Err(AutoForgeError::Artifacts("test store is read-only".into()))
    }

    async fn get(&self, key: &str) -> Result<Bytes> {
        self.objects
            .get(key)
            .cloned()
            .ok_or_else(|| AutoForgeError::Artifacts(format!("not found: {key}")))
    }

    fn uri_for(&self, key: &str) -> String {
        format!("test://{key}")
    }
}
