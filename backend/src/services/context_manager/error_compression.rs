use super::{CompressedError, ErrorEntry};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorCompressionLimits {
    pub max_output_bytes: usize,
    pub max_errors: usize,
    pub max_message_bytes: usize,
}

impl Default for ErrorCompressionLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 64 * 1024,
            max_errors: 64,
            max_message_bytes: 2 * 1024,
        }
    }
}

pub(super) fn compress(
    command: &str,
    exit_code: i32,
    output: &str,
    limits: ErrorCompressionLimits,
) -> CompressedError {
    let bounded_output = truncate_utf8(output, limits.max_output_bytes);
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    let mut location = None;
    let mut latest_error = None;

    for line in bounded_output.lines() {
        if let Some(found) = parse_location(line) {
            location = Some(found);
            if let Some(index) = latest_error {
                attach_location(&mut errors[index], location.as_ref());
            }
        }
        if !is_error_context(line) || errors.len() == limits.max_errors {
            continue;
        }
        let entry = ErrorEntry {
            file: location.as_ref().map(|location| location.file.clone()),
            line: location.as_ref().map(|location| location.line),
            code: error_code(line),
            message: truncate_utf8(line.trim(), limits.max_message_bytes).to_owned(),
        };
        let key = format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            option_str(entry.file.as_deref()),
            entry.line.map_or(0, u32::from),
            option_str(entry.code.as_deref()),
            entry.message
        );
        if seen.insert(key) {
            errors.push(entry);
            latest_error = errors.len().checked_sub(1);
        }
    }
    CompressedError {
        command: command.to_owned(),
        exit_code,
        errors,
    }
}

#[derive(Clone)]
struct Location {
    file: String,
    line: u32,
}

fn is_error_context(line: &str) -> bool {
    let line = line.to_lowercase();
    [
        "error",
        "traceback",
        "failed",
        "failure",
        "panic",
        "exception",
    ]
    .iter()
    .any(|marker| line.contains(marker))
}

fn error_code(line: &str) -> Option<String> {
    let offset = find_ascii_case_insensitive(line.as_bytes(), b"error[")? + "error[".len();
    let suffix = &line[offset..];
    suffix
        .split(']')
        .next()
        .filter(|code| !code.is_empty())
        .map(str::to_owned)
}

fn parse_location(line: &str) -> Option<Location> {
    let trimmed = line.trim();
    if let Some(file) = trimmed.strip_prefix("File \"") {
        let (file, rest) = file.split_once('"')?;
        let line = rest.trim_start().strip_prefix(", line ")?;
        let line = line
            .split(|character: char| !character.is_ascii_digit())
            .next()?;
        return line.parse().ok().map(|line| Location {
            file: file.to_owned(),
            line,
        });
    }
    let location = match trimmed.strip_prefix("-->") {
        Some(location) => location,
        None => trimmed,
    };
    parse_colon_location(location.trim())
}

fn parse_colon_location(value: &str) -> Option<Location> {
    let mut segments = value.rsplitn(3, ':');
    let last = segments.next()?.trim();
    let middle = segments.next()?.trim();
    let before = match segments.next() {
        Some(before) => before.trim(),
        None => "",
    };
    if let Ok(line) = middle.parse() {
        if !before.is_empty() && last.parse::<u32>().is_ok() {
            return Some(Location {
                file: before.to_owned(),
                line,
            });
        }
    }
    last.parse()
        .ok()
        .filter(|_| !middle.is_empty())
        .map(|line| Location {
            file: middle.to_owned(),
            line,
        })
}

fn attach_location(entry: &mut ErrorEntry, location: Option<&Location>) {
    if let Some(location) = location {
        entry.file = Some(location.file.clone());
        entry.line = Some(location.line);
    }
}

fn option_str(value: Option<&str>) -> &str {
    value.unwrap_or_default()
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(value, expected)| value.eq_ignore_ascii_case(expected))
    })
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    let mut end = value.len().min(max_bytes);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}
