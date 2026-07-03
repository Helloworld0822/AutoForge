use crate::domain::ArtifactRef;
use crate::error::{AutoForgeError, Result};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct IngestResult {
    pub raw_text: String,
    pub page_count: u32,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct DevopsIngestResult {
    pub raw_text: String,
    pub format: String,
    pub sha256: String,
    pub source: String,
}

pub fn ingest_pdf(bytes: &[u8]) -> Result<IngestResult> {
    let sha256 = hex::encode(Sha256::digest(bytes));

    let doc = lopdf::Document::load_mem(bytes)
        .map_err(|e| AutoForgeError::Ingest(format!("PDF parse failed: {e}")))?;

    let page_count = doc.get_pages().len() as u32;
    let raw_text = doc
        .extract_text(&[])
        .map_err(|e| AutoForgeError::Ingest(format!("text extraction failed: {e}")))?;

    if raw_text.trim().is_empty() {
        return Err(AutoForgeError::Ingest(
            "no extractable text — OCR fallback required".into(),
        ));
    }

    Ok(IngestResult {
        raw_text,
        page_count,
        sha256,
    })
}

/// DevOps 계획서 직접 입력 (Markdown/YAML/텍스트)
pub fn ingest_devops_text(text: &str) -> Result<DevopsIngestResult> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AutoForgeError::Ingest("empty devops plan text".into()));
    }

    let sha256 = hex::encode(Sha256::digest(trimmed.as_bytes()));
    Ok(DevopsIngestResult {
        raw_text: trimmed.to_string(),
        format: "text".into(),
        sha256,
        source: "inline".into(),
    })
}

/// DevOps 계획서 파일 (PDF / Markdown / YAML / TXT)
pub fn ingest_devops_file(bytes: &[u8], filename: Option<&str>) -> Result<DevopsIngestResult> {
    if bytes.is_empty() {
        return Err(AutoForgeError::Ingest("empty devops plan file".into()));
    }

    let ext = filename
        .and_then(|f| f.rsplit('.').next())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if bytes.starts_with(b"%PDF") || ext == "pdf" {
        let pdf = ingest_pdf(bytes)?;
        return Ok(DevopsIngestResult {
            raw_text: pdf.raw_text,
            format: "pdf".into(),
            sha256: pdf.sha256,
            source: filename.unwrap_or("devops_plan.pdf").into(),
        });
    }

    let text = std::str::from_utf8(bytes)
        .map_err(|_| AutoForgeError::Ingest("devops plan must be UTF-8 text or PDF".into()))?;

    if text.trim().is_empty() {
        return Err(AutoForgeError::Ingest("empty devops plan file".into()));
    }

    let format = match ext.as_str() {
        "md" | "markdown" => "markdown",
        "yaml" | "yml" => "yaml",
        "txt" => "text",
        "" => detect_text_format(text),
        _ => {
            return Err(AutoForgeError::Ingest(format!(
                "unsupported devops plan format: .{ext} (use pdf, md, yaml, yml, txt)"
            )));
        }
    };

    Ok(DevopsIngestResult {
        raw_text: text.trim().to_string(),
        format: format.into(),
        sha256: hex::encode(Sha256::digest(bytes)),
        source: filename.unwrap_or("devops_plan").into(),
    })
}

pub fn ingest_devops_plan(input: &crate::domain::DevopsPlanInput) -> Result<DevopsIngestResult> {
    if let Some(text) = &input.text {
        if !text.trim().is_empty() {
            return ingest_devops_text(text);
        }
    }
    if let Some(bytes) = &input.bytes {
        return ingest_devops_file(bytes, input.filename.as_deref());
    }
    Err(AutoForgeError::Ingest("no devops plan content".into()))
}

fn detect_text_format(text: &str) -> &'static str {
    let trimmed = text.trim_start();
    if trimmed.starts_with("---")
        || trimmed.contains("\nkind:")
        || trimmed.contains("\napiVersion:")
    {
        "yaml"
    } else if trimmed.starts_with('#') || trimmed.contains("\n## ") {
        "markdown"
    } else {
        "text"
    }
}

pub fn to_artifacts(result: &IngestResult, base_uri: &str) -> Vec<ArtifactRef> {
    vec![
        ArtifactRef {
            name: "raw_text.md".into(),
            key: format!("{base_uri}/raw_text.md"),
            uri: format!("{base_uri}/raw_text.md"),
            content_type: "text/markdown".into(),
            sha256: Some(result.sha256.clone()),
        },
        ArtifactRef {
            name: "ingest_meta.json".into(),
            key: format!("{base_uri}/ingest_meta.json"),
            uri: format!("{base_uri}/ingest_meta.json"),
            content_type: "application/json".into(),
            sha256: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_bytes() {
        assert!(ingest_pdf(&[]).is_err());
    }

    #[test]
    fn ingest_devops_markdown() {
        let result = ingest_devops_file(b"# CI/CD\n- GitHub Actions", Some("plan.md")).unwrap();
        assert_eq!(result.format, "markdown");
        assert!(result.raw_text.contains("CI/CD"));
    }

    #[test]
    fn ingest_devops_yaml() {
        let yaml = b"apiVersion: v1\nkind: Service\nmetadata:\n  name: api";
        let result = ingest_devops_file(yaml, Some("k8s.yaml")).unwrap();
        assert_eq!(result.format, "yaml");
    }

    #[test]
    fn ingest_devops_inline_text() {
        let result = ingest_devops_text("Docker compose + nginx proxy").unwrap();
        assert_eq!(result.source, "inline");
    }
}
