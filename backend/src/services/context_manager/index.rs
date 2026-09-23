use super::CodeIndexEntry;
use std::collections::HashMap;
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

const MAX_REPORTED_OMISSIONS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeIndexLimits {
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
}

impl Default for CodeIndexLimits {
    fn default() -> Self {
        Self {
            max_files: 512,
            max_total_bytes: 8 * 1024 * 1024,
            max_file_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodeIndexReport {
    pub index: HashMap<String, CodeIndexEntry>,
    pub omitted: Vec<IndexOmission>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOmission {
    pub path: PathBuf,
    pub reason: IndexOmissionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexOmissionReason {
    IgnoredDirectory,
    Symlink,
    BinaryFile,
    FileByteLimit,
    TotalByteLimit,
    FileCountLimit,
}

pub(super) fn build(root: &Path, limits: CodeIndexLimits) -> std::io::Result<CodeIndexReport> {
    let canonical_root = root.canonicalize()?;
    if !fs::metadata(&canonical_root)?.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "code index root is not a directory",
        ));
    }
    let mut state = BuildState::default();
    walk(&canonical_root, &canonical_root, limits, &mut state)?;
    Ok(state.into_report())
}

#[derive(Default)]
struct BuildState {
    index: HashMap<String, CodeIndexEntry>,
    omitted: Vec<IndexOmission>,
    indexed_bytes: u64,
}

impl BuildState {
    fn omit(&mut self, root: &Path, path: &Path, reason: IndexOmissionReason) {
        if self.omitted.len() < MAX_REPORTED_OMISSIONS {
            self.omitted.push(IndexOmission {
                path: relative_path(root, path),
                reason,
            });
        }
    }

    fn into_report(self) -> CodeIndexReport {
        CodeIndexReport {
            index: self.index,
            omitted: self.omitted,
        }
    }
}

fn walk(
    root: &Path,
    directory: &Path,
    limits: CodeIndexLimits,
    state: &mut BuildState,
) -> std::io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            state.omit(root, &path, IndexOmissionReason::Symlink);
            continue;
        }
        if file_type.is_dir() {
            if ignored_directory(&path) {
                state.omit(root, &path, IndexOmissionReason::IgnoredDirectory);
            } else {
                walk(root, &path, limits, state)?;
            }
            continue;
        }
        if !file_type.is_file() || !supported_source(&path) {
            continue;
        }

        let bytes = entry.metadata()?.len();
        if bytes > limits.max_file_bytes {
            state.omit(root, &path, IndexOmissionReason::FileByteLimit);
            continue;
        }
        if state.index.len() == limits.max_files {
            state.omit(root, &path, IndexOmissionReason::FileCountLimit);
            continue;
        }
        if state.indexed_bytes.saturating_add(bytes) > limits.max_total_bytes {
            state.omit(root, &path, IndexOmissionReason::TotalByteLimit);
            continue;
        }

        let content = fs::read(&path)?;
        if content.contains(&0) {
            state.omit(root, &path, IndexOmissionReason::BinaryFile);
            continue;
        }
        let content = match String::from_utf8(content) {
            Ok(content) => content,
            Err(_) => {
                state.omit(root, &path, IndexOmissionReason::BinaryFile);
                continue;
            }
        };
        let extension = source_extension(&path).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                "supported source file has no extension",
            )
        })?;
        state.index.insert(
            relative_path(root, &path).to_string_lossy().into_owned(),
            CodeIndexEntry {
                purpose: format!("{extension} source file"),
                symbols: symbols(&content),
            },
        );
        state.indexed_bytes = state.indexed_bytes.saturating_add(bytes);
    }
    Ok(())
}

fn ignored_directory(path: &Path) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(".git" | "node_modules" | "target" | "dist" | "build" | "vendor" | "generated") => {
            true
        }
        Some(_) | None => false,
    }
}

fn supported_source(path: &Path) -> bool {
    source_extension(path).is_some()
}

fn source_extension(path: &Path) -> Option<&str> {
    let extension = path.extension().and_then(|extension| extension.to_str())?;
    match extension {
        "rs" | "ts" | "tsx" | "js" | "py" | "go" => Some(extension),
        _ => None,
    }
}

fn symbols(content: &str) -> Vec<String> {
    const PREFIXES: [&str; 12] = [
        "pub async fn ",
        "pub fn ",
        "async fn ",
        "fn ",
        "pub struct ",
        "struct ",
        "class ",
        "async def ",
        "def ",
        "func ",
        "interface ",
        "type ",
    ];
    content
        .lines()
        .filter_map(|line| {
            PREFIXES
                .iter()
                .find_map(|prefix| line.trim().strip_prefix(prefix))
                .and_then(|name| {
                    name.split(|character: char| !character.is_alphanumeric() && character != '_')
                        .next()
                })
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        })
        .collect()
}

fn relative_path(root: &Path, path: &Path) -> PathBuf {
    match path.strip_prefix(root) {
        Ok(path) => path.to_path_buf(),
        Err(_) => path.to_path_buf(),
    }
}
