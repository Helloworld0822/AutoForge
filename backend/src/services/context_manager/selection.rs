use super::CodeIndexEntry;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileSelectionReport {
    pub selected: Vec<PathBuf>,
    pub omitted: Vec<SelectionOmission>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionOmission {
    pub path: PathBuf,
    pub reason: SelectionOmissionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOmissionReason {
    OutsideRoot,
    Symlink,
    Missing,
    NotAFile,
    ByteBudget,
    FileBudget,
}

pub(super) fn select(
    root: &Path,
    index: &HashMap<String, CodeIndexEntry>,
    hints: &[String],
    max_files: usize,
    max_bytes: usize,
) -> std::io::Result<FileSelectionReport> {
    let canonical_root = root.canonicalize()?;
    let normalized_hints = hints
        .iter()
        .map(|hint| hint.trim().to_lowercase())
        .filter(|hint| !hint.is_empty())
        .collect::<Vec<_>>();
    let mut candidates = index
        .iter()
        .filter_map(|(path, entry)| Candidate::new(path, entry, &normalized_hints))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| (Reverse(candidate.score), candidate.relative.clone()));

    let mut report = FileSelectionReport {
        selected: Vec::new(),
        omitted: Vec::new(),
    };
    let mut selected_bytes = 0_u64;
    let max_bytes = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    for candidate in candidates {
        let path = PathBuf::from(&candidate.relative);
        if !safe_relative_path(&path) {
            omit(&mut report, path, SelectionOmissionReason::OutsideRoot);
            continue;
        }
        let joined = canonical_root.join(&path);
        let metadata = match fs::symlink_metadata(&joined) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                omit(&mut report, path, SelectionOmissionReason::Missing);
                continue;
            }
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink() {
            omit(&mut report, path, SelectionOmissionReason::Symlink);
            continue;
        }
        if !metadata.is_file() {
            omit(&mut report, path, SelectionOmissionReason::NotAFile);
            continue;
        }
        let canonical_path = joined.canonicalize()?;
        if !canonical_path.starts_with(&canonical_root) {
            omit(&mut report, path, SelectionOmissionReason::OutsideRoot);
            continue;
        }
        if report.selected.len() == max_files {
            omit(&mut report, path, SelectionOmissionReason::FileBudget);
            continue;
        }
        let bytes = metadata.len();
        if selected_bytes.saturating_add(bytes) > max_bytes {
            omit(&mut report, path, SelectionOmissionReason::ByteBudget);
            continue;
        }
        selected_bytes = selected_bytes.saturating_add(bytes);
        report.selected.push(canonical_path);
    }
    Ok(report)
}

struct Candidate {
    relative: String,
    score: u16,
}

impl Candidate {
    fn new(path: &str, entry: &CodeIndexEntry, hints: &[String]) -> Option<Self> {
        let path_lower = path.to_lowercase();
        let purpose_lower = entry.purpose.to_lowercase();
        let score = hints.iter().fold(0_u16, |score, hint| {
            score.saturating_add(relevance(&path_lower, &purpose_lower, &entry.symbols, hint))
        });
        (score > 0).then(|| Self {
            relative: path.to_owned(),
            score,
        })
    }
}

fn relevance(path: &str, purpose: &str, symbols: &[String], hint: &str) -> u16 {
    let path_score = u16::from(path.contains(hint)) * 8;
    let purpose_score = u16::from(purpose.contains(hint)) * 4;
    let matching_symbols = symbols
        .iter()
        .filter(|symbol| symbol.to_lowercase().contains(hint))
        .count()
        .min(4);
    let matching_symbols = u16::try_from(matching_symbols).unwrap_or(4);
    let symbol_score = matching_symbols * 12;
    path_score + purpose_score + symbol_score
}

fn safe_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn omit(report: &mut FileSelectionReport, path: PathBuf, reason: SelectionOmissionReason) {
    report.omitted.push(SelectionOmission { path, reason });
}
