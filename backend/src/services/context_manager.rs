use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
        let mut index = HashMap::new();
        Self::walk(root, root, &mut index)?;
        Ok(index)
    }

    pub fn select_files(
        root: &Path,
        index: &HashMap<String, CodeIndexEntry>,
        hints: &[String],
        max_files: usize,
        max_bytes: usize,
    ) -> std::io::Result<Vec<PathBuf>> {
        let mut candidates: Vec<_> = index
            .keys()
            .filter(|path| hints.iter().any(|hint| path.contains(hint)))
            .cloned()
            .collect();
        candidates.sort();
        let mut selected = Vec::new();
        let mut bytes: usize = 0;
        for relative in candidates {
            if selected.len() == max_files {
                break;
            }
            let path = root.join(&relative);
            let size = fs::metadata(&path)?.len() as usize;
            if bytes.saturating_add(size) > max_bytes {
                continue;
            }
            bytes += size;
            selected.push(path);
        }
        Ok(selected)
    }

    pub fn compress_errors(command: &str, exit_code: i32, output: &str) -> CompressedError {
        let mut errors = Vec::new();
        for line in output.lines().filter(|line| line.contains("error")) {
            if errors
                .iter()
                .any(|error: &ErrorEntry| error.message == line)
            {
                continue;
            }
            errors.push(ErrorEntry {
                file: None,
                line: None,
                code: None,
                message: line.trim().to_string(),
            });
        }
        CompressedError {
            command: command.to_string(),
            exit_code,
            errors,
        }
    }

    fn walk(
        root: &Path,
        directory: &Path,
        index: &mut HashMap<String, CodeIndexEntry>,
    ) -> std::io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                Self::walk(root, &path, index)?;
                continue;
            }
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            if !matches!(extension, "rs" | "ts" | "tsx" | "js" | "py" | "go") {
                continue;
            }
            let content = fs::read_to_string(&path)?;
            let symbols = content
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    ["fn ", "struct ", "class ", "def ", "func "]
                        .iter()
                        .find_map(|prefix| trimmed.strip_prefix(prefix))
                        .map(|name| {
                            name.split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                                .next()
                                .unwrap_or(name)
                                .to_string()
                        })
                })
                .collect();
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            index.insert(
                relative,
                CodeIndexEntry {
                    purpose: format!("{} source file", extension),
                    symbols,
                },
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
