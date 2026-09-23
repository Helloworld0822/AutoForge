use super::*;
use crate::error::AutoForgeError;
use lopdf::{dictionary, Document, Object, Stream};
use pdf_inspector::{MarkdownProfile, PdfType, ScanStrategy};

fn text_pdf() -> Vec<u8> {
    let mut document = Document::with_version("1.4");
    let font = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let content = document.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 24 Tf 72 720 Td (Project Requirements) Tj 0 -36 Td /F1 12 Tf (Scope ................................ 4) Tj 0 -20 Td (2027) Tj 0 -20 Td (Q1: discovery) Tj 0 -20 Td (Use PostgreSQL and Redis.) Tj ET".to_vec(),
    ));
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        "Contents" => content,
    });
    let pages = document.add_object(dictionary! {
        "Type" => "Pages",
        "Count" => 1,
        "Kids" => vec![Object::Reference(page)],
    });
    document
        .get_object_mut(page)
        .expect("page exists")
        .as_dict_mut()
        .expect("page is a dictionary")
        .set("Parent", pages);
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("serialize PDF fixture");
    bytes
}

fn textless_pdf() -> Vec<u8> {
    let mut document = Document::with_version("1.4");
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
    });
    let pages = document.add_object(dictionary! {
        "Type" => "Pages",
        "Count" => 1,
        "Kids" => vec![Object::Reference(page)],
    });
    document
        .get_object_mut(page)
        .expect("page exists")
        .as_dict_mut()
        .expect("page is a dictionary")
        .set("Parent", pages);
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("serialize PDF fixture");
    bytes
}

#[test]
fn rejects_empty_bytes() {
    assert!(ingest_pdf(&[]).is_err());
}

#[test]
fn converts_text_pdf_to_compact_markdown() {
    let result = ingest_pdf(&text_pdf()).expect("text PDF is ingested");

    assert!(result.raw_text.contains("Project Requirements"));
    assert!(!result.raw_text.contains("................"));
    assert!(result.raw_text.contains("2027"));
    assert!(result.raw_text.contains("Q1: discovery"));
    assert_eq!(result.page_count, 1);
    assert_eq!(result.pdf_type, "text_based");
    assert_eq!(result.extraction_method, "pdf_inspector");
    assert_eq!(result.encoding, "native_text_layer");
    assert!(result.estimated_input_tokens > 0);
}

#[test]
fn deduplicates_only_consecutive_identical_prose_blocks() {
    let compact = pdf::compact_markdown("# Title\n\nSame block\n\nSame block\n\nNext\n");

    assert_eq!(compact, "# Title\n\nSame block\n\nNext\n");
}

#[test]
fn preserves_code_tables_lists_and_source_references_when_deduplicating() {
    let markdown = "```sh\necho deploy\n```\n\n```sh\necho deploy\n```\n\n| Name | Value |\n| --- | --- |\n| API | 1 |\n\n| Name | Value |\n| --- | --- |\n| API | 1 |\n\n- require PostgreSQL\n\n- require PostgreSQL\n\nSee [source](https://example.test/spec)\n\nSee [source](https://example.test/spec)\n";

    assert_eq!(pdf::compact_markdown(markdown), markdown);
}

#[test]
fn configures_full_detection_compact_markdown_and_numeric_line_preservation() {
    let options = pdf::pdf_options();

    assert!(matches!(options.detection.strategy, ScanStrategy::Full));
    assert_eq!(options.markdown.profile, MarkdownProfile::Compact);
    assert!(options.markdown.include_page_numbers);
    assert!(!options.markdown.remove_page_numbers);
}

#[test]
fn reports_ocr_requirement_for_unrecovered_mixed_pages() {
    let error = pdf::reject_ocr_required(&PdfType::Mixed, &[2]).unwrap_err();

    assert!(matches!(error, AutoForgeError::Ingest(_)));
    let message = error.to_string();
    assert!(message.contains("requires OCR for page(s) 2"));
    assert!(message.contains("no OCR recovery is configured"));
}

#[test]
fn rejects_textless_pdf_that_requires_ocr() {
    let error = ingest_pdf(&textless_pdf()).expect_err("textless PDF needs OCR");

    assert!(matches!(error, AutoForgeError::Ingest(_)));
    let message = error.to_string();
    assert!(message.contains("requires OCR for page(s) 1"));
}

#[test]
fn pdftotext_fallback_extracts_native_text() {
    let text = pdf::extract_text_pdftotext(&text_pdf()).expect("pdftotext reads native text");

    assert!(text.contains("Project Requirements"));
    assert!(text.contains("2027"));
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
