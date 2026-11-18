use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogLevel {
    All,
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

impl LogLevel {
    #[allow(dead_code)]
    pub fn all() -> [LogLevel; 9] {
        [
            LogLevel::All,
            LogLevel::Emergency,
            LogLevel::Alert,
            LogLevel::Critical,
            LogLevel::Error,
            LogLevel::Warning,
            LogLevel::Notice,
            LogLevel::Info,
            LogLevel::Debug,
        ]
    }

    pub fn selectable() -> [LogLevel; 8] {
        [
            LogLevel::Emergency,
            LogLevel::Alert,
            LogLevel::Critical,
            LogLevel::Error,
            LogLevel::Warning,
            LogLevel::Notice,
            LogLevel::Info,
            LogLevel::Debug,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            LogLevel::All => "ALL",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Notice => "NOTICE",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRITICAL",
            LogLevel::Alert => "ALERT",
            LogLevel::Emergency => "EMERGENCY",
        }
    }

    pub fn matches(&self, level_str: &str) -> bool {
        if *self == LogLevel::All {
            return true;
        }
        level_str.eq_ignore_ascii_case(self.label())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogPollInterval {
    Off,
    Sec1,
    Sec5,
    Sec10,
    Sec30,
}

impl LogPollInterval {
    pub fn all() -> [LogPollInterval; 5] {
        [
            LogPollInterval::Off,
            LogPollInterval::Sec1,
            LogPollInterval::Sec5,
            LogPollInterval::Sec10,
            LogPollInterval::Sec30,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            LogPollInterval::Off => "Polling Off",
            LogPollInterval::Sec1 => "1 second",
            LogPollInterval::Sec5 => "5 seconds",
            LogPollInterval::Sec10 => "10 seconds",
            LogPollInterval::Sec30 => "30 seconds",
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        match self {
            LogPollInterval::Off => None,
            LogPollInterval::Sec1 => Some(Duration::from_secs(1)),
            LogPollInterval::Sec5 => Some(Duration::from_secs(5)),
            LogPollInterval::Sec10 => Some(Duration::from_secs(10)),
            LogPollInterval::Sec30 => Some(Duration::from_secs(30)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub id: usize,
    pub timestamp: String,
    pub environment: String,
    pub level: String,
    pub message: String,
    pub context: Option<String>,
    pub stack_trace: Vec<String>,
}

pub fn list_log_files(project_path: &str) -> Vec<PathBuf> {
    if project_path.trim().is_empty() {
        return Vec::new();
    }
    let root = Path::new(project_path);
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    let storage_logs = root.join("storage").join("logs");
    if storage_logs.is_dir() {
        collect_logs_recursive(&storage_logs, 0, &mut files, &mut seen);
    }

    let direct_logs = root.join("logs");
    if direct_logs.is_dir() {
        collect_logs_recursive(&direct_logs, 0, &mut files, &mut seen);
    }

    // Only scan root directly if the selected directory itself is a dedicated logs folder
    if files.is_empty()
        && root.is_dir()
        && root
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("logs"))
    {
        collect_logs_recursive(root, 0, &mut files, &mut seen);
    }

    sort_log_paths(&mut files);
    files
}

pub fn list_log_files_in_folder(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    if folder.is_dir() {
        collect_logs_recursive(folder, 0, &mut files, &mut seen);
    }
    sort_log_paths(&mut files);
    files
}

pub fn log_file_display_name(
    path: &Path,
    project_path: &str,
    custom_folder: Option<&Path>,
) -> String {
    if let Some(cf) = custom_folder
        && let Ok(rel) = path.strip_prefix(cf)
    {
        let s = rel.to_string_lossy().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    let root = Path::new(project_path);
    for base in [root.join("storage").join("logs"), root.join("logs"), root.to_path_buf()] {
        if let Ok(rel) = path.strip_prefix(&base) {
            let s = rel.to_string_lossy().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("laravel.log")
        .to_string()
}

fn collect_logs_recursive(
    dir: &Path,
    depth: usize,
    out: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    if depth > 3 || out.len() >= 100 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= 100 {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if dir_name.starts_with('.')
                || matches!(
                    dir_name,
                    "vendor"
                        | "node_modules"
                        | "framework"
                        | "target"
                        | "Library"
                        | "Applications"
                )
            {
                continue;
            }
            collect_logs_recursive(&path, depth + 1, out, seen);
        } else if path.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("log")
            && seen.insert(path.clone())
        {
            out.push(path);
        }
    }
}

fn sort_log_paths(files: &mut [PathBuf]) {
    files.sort_by(|a, b| {
        let a_name = a.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let b_name = b.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let a_is_default = a_name == "laravel.log";
        let b_is_default = b_name == "laravel.log";
        b_is_default.cmp(&a_is_default).then_with(|| a.cmp(b))
    });
}

pub fn parse_log_file(path: &Path) -> Vec<LogEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_log_content(&content)
}

pub fn parse_log_content(content: &str) -> Vec<LogEntry> {
    let header_re = Regex::new(
        r"^\[(\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}[^\]]*)\]\s+([a-zA-Z0-9_.-]+)\.([A-Z]+):\s+(.*)$",
    )
    .expect("valid log header regex");

    let mut entries: Vec<LogEntry> = Vec::new();
    let mut current: Option<LogEntry> = None;

    for raw_line in content.lines() {
        if let Some(caps) = header_re.captures(raw_line) {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            let raw_msg = caps[4].trim();
            let (message, context) = split_trailing_json_context(raw_msg);
            current = Some(LogEntry {
                id: entries.len() + 1,
                timestamp: caps[1].to_string(),
                environment: caps[2].to_string(),
                level: caps[3].to_uppercase(),
                message,
                context,
                stack_trace: Vec::new(),
            });
        } else if let Some(ref mut active) = current {
            let trimmed = raw_line.trim_end();
            if !trimmed.is_empty() {
                active.stack_trace.push(trimmed.to_string());
            }
        }
    }

    if let Some(last) = current {
        entries.push(last);
    }

    // Fallback for non-Monolog plain `.log` files in Logs folder
    if entries.is_empty() {
        for (idx, line) in content.lines().map(str::trim).filter(|l| !l.is_empty()).enumerate() {
            entries.push(LogEntry {
                id: idx + 1,
                timestamp: "—".to_string(),
                environment: "raw".to_string(),
                level: "INFO".to_string(),
                message: line.to_string(),
                context: None,
                stack_trace: Vec::new(),
            });
        }
    }

    // Present newest entries first for log-tailing UX
    entries.reverse();
    entries
}

fn split_trailing_json_context(raw: &str) -> (String, Option<String>) {
    if let Some(idx) = raw.rfind(" {") {
        let candidate = raw[idx + 1..].trim();
        if candidate.ends_with('}') && serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
            return (raw[..idx].trim().to_string(), Some(candidate.to_string()));
        }
    }
    (raw.to_string(), None)
}

#[allow(dead_code)]
pub fn filter_entries<'a>(
    entries: &'a [LogEntry],
    level: LogLevel,
    search: &str,
) -> Vec<&'a LogEntry> {
    let mut set = HashSet::new();
    if level != LogLevel::All {
        set.insert(level);
    }
    filter_entries_multi(entries, &set, search)
}

pub fn filter_entries_multi<'a>(
    entries: &'a [LogEntry],
    selected_levels: &HashSet<LogLevel>,
    search: &str,
) -> Vec<&'a LogEntry> {
    let needle = search.trim().to_lowercase();
    entries
        .iter()
        .filter(|entry| {
            selected_levels.is_empty()
                || selected_levels.iter().any(|lvl| lvl.matches(&entry.level))
        })
        .filter(|entry| {
            if needle.is_empty() {
                return true;
            }
            entry.message.to_lowercase().contains(&needle)
                || entry.level.to_lowercase().contains(&needle)
                || entry.environment.to_lowercase().contains(&needle)
                || entry.timestamp.to_lowercase().contains(&needle)
                || entry
                    .context
                    .as_deref()
                    .map(|c| c.to_lowercase().contains(&needle))
                    .unwrap_or(false)
                || entry
                    .stack_trace
                    .iter()
                    .any(|s| s.to_lowercase().contains(&needle))
        })
        .collect()
}

#[allow(dead_code)]
pub fn count_by_level(entries: &[LogEntry]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for entry in entries {
        *counts.entry(entry.level.clone()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::default_playground_path;

    #[test]
    fn parses_playground_laravel_log_with_stack_traces_and_context() {
        let pg = default_playground_path();
        let files = list_log_files(&pg);
        assert!(files.len() >= 2);
        let entries = parse_log_file(&files[0]);
        assert!(entries.len() >= 5);

        let errors = filter_entries(&entries, LogLevel::Error, "");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Stripe webhook"));
        assert!(errors[0].context.is_some());
        assert!(!errors[0].stack_trace.is_empty());

        let redis_search = filter_entries(&entries, LogLevel::All, "redis");
        assert_eq!(redis_search.len(), 1);
        assert_eq!(redis_search[0].level, "CRITICAL");
    }

    #[test]
    fn discovers_nested_logs_folder_files_and_multi_level_filters() {
        let pg = default_playground_path();
        let files = list_log_files(&pg);
        let names: Vec<String> = files
            .iter()
            .map(|p| log_file_display_name(p, &pg, None))
            .collect();
        assert_eq!(names[0], "laravel.log");
        assert!(names.contains(&"worker.log".to_string()));
        assert!(names.contains(&"daily/laravel-2026-11-17.log".to_string()));

        let daily_idx = names
            .iter()
            .position(|n| n == "daily/laravel-2026-11-17.log")
            .unwrap();
        let daily_entries = parse_log_file(&files[daily_idx]);
        let mut two_levels = HashSet::new();
        two_levels.insert(LogLevel::Error);
        two_levels.insert(LogLevel::Debug);
        let filtered = filter_entries_multi(&daily_entries, &two_levels, "");
        assert!(!filtered.is_empty());
        assert!(filtered
            .iter()
            .all(|e| e.level == "ERROR" || e.level == "DEBUG"));

        let poll_labels: Vec<&str> = LogPollInterval::all().iter().map(|p| p.label()).collect();
        assert_eq!(
            poll_labels,
            vec!["Polling Off", "1 second", "5 seconds", "10 seconds", "30 seconds"]
        );
    }
}
