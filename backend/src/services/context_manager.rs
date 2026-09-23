use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod error_compression;
mod index;
mod selection;

pub use error_compression::ErrorCompressionLimits;
pub use index::{CodeIndexLimits, CodeIndexReport, IndexOmission, IndexOmissionReason};
pub use selection::{FileSelectionReport, SelectionOmission, SelectionOmissionReason};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeIndexEntry {
    pub purpose: String,
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub completed_tasks: Vec<String>,
    pub current_task: Option<String>,
    pub decisions: Vec<String>,
    pub known_issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressedError {
    pub command: String,
    pub exit_code: i32,
    pub errors: Vec<ErrorEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorEntry {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct ContextManager;

impl ContextManager {
    pub fn build_code_index(root: &Path) -> std::io::Result<HashMap<String, CodeIndexEntry>> {
        Self::build_code_index_with_limits(root, CodeIndexLimits::default())
            .map(|report| report.index)
    }

    pub fn build_code_index_with_limits(
        root: &Path,
        limits: CodeIndexLimits,
    ) -> std::io::Result<CodeIndexReport> {
        index::build(root, limits)
    }

    pub fn select_files(
        root: &Path,
        index: &HashMap<String, CodeIndexEntry>,
        hints: &[String],
        max_files: usize,
        max_bytes: usize,
    ) -> std::io::Result<Vec<PathBuf>> {
        Self::select_files_with_report(root, index, hints, max_files, max_bytes)
            .map(|report| report.selected)
    }

    pub fn select_files_with_report(
        root: &Path,
        index: &HashMap<String, CodeIndexEntry>,
        hints: &[String],
        max_files: usize,
        max_bytes: usize,
    ) -> std::io::Result<FileSelectionReport> {
        selection::select(root, index, hints, max_files, max_bytes)
    }

    pub fn compress_errors(command: &str, exit_code: i32, output: &str) -> CompressedError {
        Self::compress_errors_with_limits(
            command,
            exit_code,
            output,
            ErrorCompressionLimits::default(),
        )
    }

    pub fn compress_errors_with_limits(
        command: &str,
        exit_code: i32,
        output: &str,
        limits: ErrorCompressionLimits,
    ) -> CompressedError {
        error_compression::compress(command, exit_code, output, limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn selects_related_files_with_limits() {
        let root = std::env::temp_dir().join(format!("autoforge-context-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("tempdir");
        fs::write(root.join("auth.rs"), "fn login() {}\n").expect("write");
        fs::write(root.join("cart.rs"), "struct Cart;\n").expect("write");
        let index = ContextManager::build_code_index(&root).expect("index");
        let selected = ContextManager::select_files(&root, &index, &["auth".into()], 8, 100)
            .expect("selection");
        assert_eq!(selected.len(), 1);
        assert_eq!(index["auth.rs"].symbols, vec!["login"]);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn compresses_duplicate_errors() {
        let result =
            ContextManager::compress_errors("cargo test", 101, "error[E1]: bad\nerror[E1]: bad\n");
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn indexes_source_with_directory_file_and_byte_budgets() {
        // Given: source files alongside generated dependencies, a binary, and an oversized file.
        let root = test_root("index-budgets");
        fs::create_dir_all(root.join("src")).expect("source directory");
        fs::write(root.join("src/login.rs"), "fn login() {}\n").expect("source");
        fs::write(root.join("src/z-extra.rs"), "fn extra() {}\n").expect("extra source");
        fs::write(root.join("src/binary.rs"), [0_u8, 159, 146, 150]).expect("binary");
        fs::write(root.join("src/large.rs"), "fn oversized() {}\n".repeat(20)).expect("large");
        for directory in [
            ".git",
            "node_modules",
            "target",
            "dist",
            "build",
            "vendor",
            "generated",
        ] {
            fs::create_dir_all(root.join(directory)).expect("ignored directory");
            fs::write(root.join(directory).join("ignored.rs"), "fn ignored() {}\n")
                .expect("ignored source");
        }

        // When: the index is built with strict input limits.
        let report = ContextManager::build_code_index_with_limits(
            &root,
            CodeIndexLimits {
                max_files: 8,
                max_total_bytes: 20,
                max_file_bytes: 32,
            },
        )
        .expect("index");

        // Then: only the eligible source is indexed and exclusions have typed reasons.
        assert_eq!(report.index.len(), 1);
        assert!(report.index.contains_key("src/login.rs"));
        assert!(report
            .omitted
            .iter()
            .any(|entry| entry.reason == IndexOmissionReason::IgnoredDirectory));
        assert!(report
            .omitted
            .iter()
            .any(|entry| entry.reason == IndexOmissionReason::BinaryFile));
        assert!(report
            .omitted
            .iter()
            .any(|entry| entry.reason == IndexOmissionReason::FileByteLimit));
        assert!(report
            .omitted
            .iter()
            .any(|entry| entry.reason == IndexOmissionReason::TotalByteLimit));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_traversal_paths_when_indexing_and_selecting() {
        use std::os::unix::fs::symlink;

        // Given: symlinks inside the root and untrusted indexed paths pointing outside it.
        let root = test_root("symlinks");
        let outside = test_root("outside");
        fs::create_dir_all(&root).expect("root directory");
        fs::create_dir_all(&outside).expect("outside directory");
        fs::write(root.join("local.rs"), "fn local() {}\n").expect("local source");
        fs::write(outside.join("secret.rs"), "fn secret() {}\n").expect("outside source");
        symlink(&outside, root.join("linked-directory")).expect("directory link");
        symlink(outside.join("secret.rs"), root.join("linked.rs")).expect("file link");
        let mut index = ContextManager::build_code_index(&root).expect("index");
        index.insert(
            "../outside/secret.rs".into(),
            CodeIndexEntry {
                purpose: "rust source file".into(),
                symbols: vec!["secret".into()],
            },
        );
        index.insert(
            "linked.rs".into(),
            CodeIndexEntry {
                purpose: "rust source file".into(),
                symbols: vec!["secret".into()],
            },
        );

        // When: related files are selected from the untrusted index.
        let report =
            ContextManager::select_files_with_report(&root, &index, &["secret".into()], 8, 1024)
                .expect("selection");

        // Then: no path outside the canonical root or symlink target is returned.
        assert!(report.selected.is_empty());
        assert!(report
            .omitted
            .iter()
            .any(|entry| entry.reason == SelectionOmissionReason::OutsideRoot));
        assert!(report
            .omitted
            .iter()
            .any(|entry| entry.reason == SelectionOmissionReason::Symlink));
        fs::remove_dir_all(root).expect("cleanup root");
        fs::remove_dir_all(outside).expect("cleanup outside");
    }

    #[test]
    fn ranks_symbol_matches_deterministically_and_reports_byte_omissions() {
        // Given: a path-only match and a stronger symbol match that fit different byte budgets.
        let root = test_root("selection-ranking");
        fs::create_dir_all(&root).expect("root directory");
        fs::write(root.join("payment_notes.rs"), "fn notes() {}\n").expect("path match");
        fs::write(root.join("billing.rs"), "fn payment_handler() {}\n").expect("symbol match");
        fs::write(
            root.join("payment_overflow.rs"),
            "fn payment_overflow() {}\n".repeat(8),
        )
        .expect("overflow match");
        let index = ContextManager::build_code_index(&root).expect("index");
        let billing_bytes = fs::metadata(root.join("billing.rs"))
            .expect("billing metadata")
            .len();

        // When: selection is limited to the symbol-matched file's byte size.
        let report = ContextManager::select_files_with_report(
            &root,
            &index,
            &["payment".into()],
            8,
            billing_bytes as usize,
        )
        .expect("selection");

        // Then: symbol relevance wins deterministically and oversized matches are explained.
        assert_eq!(
            report.selected,
            vec![root
                .canonicalize()
                .expect("canonical root")
                .join("billing.rs")]
        );
        assert!(report
            .omitted
            .iter()
            .any(|entry| entry.reason == SelectionOmissionReason::ByteBudget));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn compresses_unicode_errors_with_context_and_bounded_output() {
        // Given: duplicate Unicode output containing a traceback, uppercase error, location, and failure.
        let output = concat!(
            "Traceback (most recent call last):\n",
            "  File \"src/한글.py\", line 7, in run\n",
            "ValueError: 실패 😱\n",
            "ERROR[E900]: broken\n",
            " --> src/lib.rs:12:4\n",
            "test parser::unicode_case ... FAILED\n",
            "ERROR[E900]: broken\n"
        );

        // When: compression uses explicit record and byte limits.
        let result = ContextManager::compress_errors_with_limits(
            "cargo test",
            101,
            output,
            ErrorCompressionLimits {
                max_output_bytes: 512,
                max_errors: 4,
                max_message_bytes: 48,
            },
        );

        // Then: relevant context survives, duplicates are removed, and messages remain bounded UTF-8.
        assert_eq!(result.exit_code, 101);
        assert!(result.errors.len() <= 4);
        assert!(result.errors.iter().any(|entry| {
            entry.file.as_deref() == Some("src/한글.py")
                && entry.line == Some(7)
                && entry.message.contains("ValueError")
        }));
        assert!(result.errors.iter().any(|entry| {
            entry.file.as_deref() == Some("src/lib.rs")
                && entry.line == Some(12)
                && entry.code.as_deref() == Some("E900")
        }));
        assert!(result
            .errors
            .iter()
            .any(|entry| entry.message.contains("FAILED")));
        assert!(result.errors.iter().all(|entry| entry.message.len() <= 48));
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("autoforge-context-{name}-{}", uuid::Uuid::new_v4()))
    }
}
