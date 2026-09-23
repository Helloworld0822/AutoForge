use super::IngestResult;
use crate::error::{AutoForgeError, Result};
use pdf_inspector::{
    process_pdf_mem_with_options, DetectionConfig, MarkdownOptions, MarkdownProfile, PdfOptions,
    PdfType, ScanStrategy,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PDFTOTEXT_TIMEOUT: Duration = Duration::from_secs(20);
const PDFTOTEXT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CHARS_PER_INPUT_TOKEN: usize = 4;

pub(super) fn ingest_pdf(bytes: &[u8]) -> Result<IngestResult> {
    if bytes.is_empty() {
        return Err(AutoForgeError::Ingest("empty PDF input".into()));
    }

    let result = process_pdf_mem_with_options(bytes, pdf_options())
        .map_err(|error| AutoForgeError::Ingest(format!("PDF inspection failed: {error}")))?;
    reject_ocr_required(&result.pdf_type, &result.pages_needing_ocr)?;

    let inspector_markdown = result.markdown.unwrap_or_default();
    let uses_fallback = inspector_markdown.trim().is_empty() || result.has_encoding_issues;
    let raw_text = if uses_fallback {
        compact_markdown(&extract_text_pdftotext(bytes)?)
    } else {
        compact_markdown(&inspector_markdown)
    };

    if raw_text.trim().is_empty() {
        return Err(AutoForgeError::Ingest(
            "no extractable native text; OCR is required".into(),
        ));
    }

    Ok(IngestResult {
        estimated_input_tokens: estimate_input_tokens(&raw_text),
        raw_text,
        page_count: result.page_count,
        sha256: hex::encode(Sha256::digest(bytes)),
        pdf_type: pdf_type_name(&result.pdf_type).into(),
        confidence: result.confidence,
        pages_needing_ocr: result.pages_needing_ocr,
        encoding: if uses_fallback {
            "pdftotext_native_fallback".into()
        } else {
            "native_text_layer".into()
        },
        extraction_method: if uses_fallback {
            "pdftotext_fallback".into()
        } else {
            "pdf_inspector".into()
        },
    })
}

pub(super) fn pdf_options() -> PdfOptions {
    let markdown = MarkdownOptions {
        profile: MarkdownProfile::Compact,
        include_page_numbers: true,
        remove_page_numbers: false,
        ..MarkdownOptions::default()
    };
    let detection = DetectionConfig {
        strategy: ScanStrategy::Full,
        ..DetectionConfig::default()
    };
    PdfOptions::new().detection(detection).markdown(markdown)
}

pub(super) fn reject_ocr_required(pdf_type: &PdfType, pages_needing_ocr: &[u32]) -> Result<()> {
    if matches!(pdf_type, PdfType::TextBased) && pages_needing_ocr.is_empty() {
        return Ok(());
    }

    let pages = pages_needing_ocr
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let location = if pages.is_empty() {
        "one or more pages".into()
    } else {
        format!("page(s) {pages}")
    };
    Err(AutoForgeError::Ingest(format!(
        "PDF requires OCR for {location}; no OCR recovery is configured"
    )))
}

fn pdf_type_name(pdf_type: &PdfType) -> &'static str {
    match pdf_type {
        PdfType::TextBased => "text_based",
        PdfType::Scanned => "scanned",
        PdfType::ImageBased => "image_based",
        PdfType::Mixed => "mixed",
    }
}

fn estimate_input_tokens(markdown: &str) -> usize {
    markdown.chars().count().div_ceil(CHARS_PER_INPUT_TOKEN)
}

pub(super) fn compact_markdown(markdown: &str) -> String {
    if markdown.contains("```") || markdown.contains("~~~") {
        return markdown.to_owned();
    }
    let mut blocks = Vec::new();
    for block in markdown.split("\n\n") {
        let is_duplicate = is_deduplicable_prose(block)
            && blocks.last().is_some_and(|previous| *previous == block);
        if !is_duplicate {
            blocks.push(block);
        }
    }
    blocks.join("\n\n")
}

fn is_deduplicable_prose(block: &str) -> bool {
    let trimmed = block.trim();
    !trimmed.is_empty()
        && !trimmed.contains("```")
        && !trimmed.contains('[')
        && !trimmed.starts_with("<!--")
        && !trimmed.lines().any(is_structured_markdown_line)
}

fn is_structured_markdown_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('|')
        || trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || numbered_list_item(trimmed)
}

fn numbered_list_item(line: &str) -> bool {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    matches!(line.as_bytes().get(digits..), Some([b'.' | b')', b' ', ..])) && digits > 0
}

pub(super) fn extract_text_pdftotext(bytes: &[u8]) -> Result<String> {
    let directory = TemporaryPdfDirectory::new()?;
    let input = directory.path().join("source.pdf");
    let output = directory.path().join("source.txt");
    fs::write(&input, bytes).map_err(|error| {
        AutoForgeError::Ingest(format!("pdftotext input write failed: {error}"))
    })?;

    let mut child = Command::new("pdftotext")
        .arg("-layout")
        .arg(&input)
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(pdftotext_spawn_error)?;

    wait_for_pdftotext(&mut child)?;
    let text = fs::read_to_string(&output).map_err(|error| {
        AutoForgeError::Ingest(format!("pdftotext output read failed: {error}"))
    })?;
    if text.trim().is_empty() {
        return Err(AutoForgeError::Ingest(
            "no extractable native text; OCR is required".into(),
        ));
    }
    Ok(text)
}

fn pdftotext_spawn_error(error: std::io::Error) -> AutoForgeError {
    if error.kind() == std::io::ErrorKind::NotFound {
        AutoForgeError::Ingest(
            "no extractable native text; install poppler-utils (pdftotext) or provide OCR".into(),
        )
    } else {
        AutoForgeError::Ingest(format!("pdftotext spawn failed: {error}"))
    }
}

fn wait_for_pdftotext(child: &mut std::process::Child) -> Result<()> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| AutoForgeError::Ingest(format!("pdftotext wait failed: {error}")))?
        {
            return if status.success() {
                Ok(())
            } else {
                Err(AutoForgeError::Ingest("pdftotext failed".into()))
            };
        }
        if started.elapsed() >= PDFTOTEXT_TIMEOUT {
            child.kill().map_err(|error| {
                AutoForgeError::Ingest(format!("pdftotext kill failed: {error}"))
            })?;
            child.wait().map_err(|error| {
                AutoForgeError::Ingest(format!("pdftotext cleanup failed: {error}"))
            })?;
            return Err(AutoForgeError::Ingest(
                "pdftotext timed out after 20 seconds".into(),
            ));
        }
        thread::sleep(PDFTOTEXT_POLL_INTERVAL);
    }
}

struct TemporaryPdfDirectory {
    path: PathBuf,
}

impl TemporaryPdfDirectory {
    fn new() -> Result<Self> {
        let entropy = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| AutoForgeError::Ingest(format!("system clock error: {error}")))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "autoforge-pdftotext-{}-{entropy}",
            std::process::id()
        ));
        fs::create_dir(&path).map_err(|error| {
            AutoForgeError::Ingest(format!("pdftotext temp directory failed: {error}"))
        })?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryPdfDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
