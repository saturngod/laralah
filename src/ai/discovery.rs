use crate::ai::{AiModel, AiModelList, CliProviderId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{LazyLock, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION_TIMEOUT: Duration = Duration::from_secs(8);
const MODELS_TIMEOUT: Duration = Duration::from_secs(10);
const MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_VERSION_CHARS: usize = 120;
const MAX_MODELS: usize = 2000;
const MAX_MODEL_ID_CHARS: usize = 256;
const MAX_CATALOG_BYTES: usize = 4_000_000;

static CACHED_SHELL_DIRS: LazyLock<Mutex<Option<Vec<PathBuf>>>> =
    LazyLock::new(|| Mutex::new(None));
static MODELS_DEV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn candidate_file_names(binary: &str) -> Vec<OsString> {
    let suffixes: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    suffixes
        .iter()
        .map(|suffix| OsString::from(format!("{binary}{suffix}")))
        .collect()
}

fn find_on_path_in(directories: &[PathBuf], binary: &str) -> Option<PathBuf> {
    let candidates = candidate_file_names(binary);
    for directory in directories {
        for candidate in &candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn path_directories() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default()
}

fn home_directory() -> Option<PathBuf> {
    crate::paths::home_dir()
}

fn node_version_bins(home: &Path) -> Vec<PathBuf> {
    let layouts: Vec<(PathBuf, Vec<&str>)> = vec![
        (
            home.join(".local")
                .join("share")
                .join("fnm")
                .join("node-versions"),
            vec!["installation", "bin"],
        ),
        (
            home.join(".fnm").join("node-versions"),
            vec!["installation", "bin"],
        ),
        (
            home.join(".local")
                .join("share")
                .join("mise")
                .join("installs")
                .join("node"),
            vec!["bin"],
        ),
        (
            home.join(".volta").join("tools").join("image").join("node"),
            vec!["bin"],
        ),
        (
            home.join(".asdf").join("installs").join("nodejs"),
            vec!["bin"],
        ),
        (home.join("n").join("versions").join("node"), vec!["bin"]),
    ];
    let mut bins = Vec::new();
    for (base, tail) in layouts {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .map(|p| tail.iter().fold(p, |acc, part| acc.join(part)))
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        bins.extend(versions);
    }
    #[cfg(not(windows))]
    {
        let system_n = PathBuf::from("/usr/local/n/versions/node");
        if let Ok(entries) = std::fs::read_dir(&system_n) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .map(|p| p.join("bin"))
                .filter(|p| p.is_dir())
                .collect();
            versions.sort();
            bins.extend(versions);
        }
    }
    bins
}

fn fallback_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(home) = home_directory() {
        for relative in [
            ".opencode/bin",
            ".local/bin",
            "bin",
            ".cargo/bin",
            ".bun/bin",
            ".deno/bin",
            ".volta/bin",
            ".asdf/shims",
            ".local/share/mise/shims",
            ".config/herd-lite/bin",
            "Library/pnpm/bin",
            "Library/Application Support/Herd/bin",
            ".antigravity/bin",
            ".qoder/bin",
        ] {
            directories.push(home.join(relative));
        }
        #[cfg(windows)]
        {
            if let Some(local) = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
            {
                directories.push(local.join("agy").join("bin"));
            }
            directories.push(home.join("AppData").join("Local").join("agy").join("bin"));
            if let Some(appdata) = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
            {
                directories.push(appdata.join("npm"));
            }
        }
        let nvm_nodes = home.join(".nvm").join("versions").join("node");
        if let Ok(entries) = std::fs::read_dir(&nvm_nodes) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .map(|p| p.join("bin"))
                .collect();
            versions.sort();
            directories.extend(versions);
        }
        directories.extend(node_version_bins(&home));
    }
    #[cfg(not(windows))]
    {
        for sys in [
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/opt/homebrew/opt/node/bin",
            "/usr/local/bin",
            "/usr/local/opt/node/bin",
            "/opt/local/bin",
            "/home/linuxbrew/.linuxbrew/bin",
            "/home/linuxbrew/.linuxbrew/sbin",
        ] {
            directories.push(PathBuf::from(sys));
        }
    }
    directories
}

#[cfg(unix)]
fn shell_path_directories() -> Vec<PathBuf> {
    if let Ok(guard) = CACHED_SHELL_DIRS.lock()
        && let Some(ref cached) = *guard
    {
        return cached.clone();
    }

    const SHELL_TIMEOUT: Duration = Duration::from_secs(2);
    const MAX_PATH_BYTES: usize = 32 * 1024;

    let mut shells: Vec<PathBuf> = Vec::new();
    if let Some(configured) = std::env::var_os("SHELL").map(PathBuf::from)
        && configured.is_absolute()
    {
        shells.push(configured);
    }
    for fallback in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        let path = PathBuf::from(fallback);
        if !shells.contains(&path) {
            shells.push(path);
        }
    }

    let mut resolved = Vec::new();
    for shell in shells {
        let probe = if shell.file_name().is_some_and(|name| name == "fish") {
            "string join : $PATH"
        } else {
            "printf %s \"$PATH\""
        };
        let mut command = Command::new(&shell);
        command
            .args(["-l", "-c", probe])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(child) = command.spawn() else {
            continue;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });
        let Ok(Ok(output)) = rx.recv_timeout(SHELL_TIMEOUT) else {
            continue;
        };
        if !output.status.success() || output.stdout.is_empty() || output.stdout.len() > MAX_PATH_BYTES {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let dirs: Vec<PathBuf> = std::env::split_paths(text.trim()).collect();
        if !dirs.is_empty() {
            resolved = dirs;
            break;
        }
    }

    if let Ok(mut guard) = CACHED_SHELL_DIRS.lock() {
        *guard = Some(resolved.clone());
    }
    resolved
}

#[cfg(windows)]
fn shell_path_directories() -> Vec<PathBuf> {
    Vec::new()
}

/// Fast non-spawning search directories (process PATH + well-known install folders)
/// for instant UI rendering without waiting on a login shell.
pub fn fast_search_directories() -> Vec<PathBuf> {
    let cached_shell = CACHED_SHELL_DIRS
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for dir in path_directories()
        .into_iter()
        .chain(cached_shell)
        .chain(fallback_directories())
    {
        if seen.insert(dir.clone()) {
            ordered.push(dir);
        }
    }
    ordered
}

/// Full search directories including login-shell PATH (run from background threads).
pub fn search_directories() -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for dir in path_directories()
        .into_iter()
        .chain(shell_path_directories())
        .chain(fallback_directories())
    {
        if seen.insert(dir.clone()) {
            ordered.push(dir);
        }
    }
    ordered
}

pub fn detect_providers_fast() -> Vec<(CliProviderId, bool)> {
    let dirs = fast_search_directories();
    CliProviderId::all()
        .into_iter()
        .map(|id| {
            let found = id
                .binaries()
                .iter()
                .any(|bin| find_on_path_in(&dirs, bin).is_some());
            (id, found)
        })
        .collect()
}

pub fn detect_providers_full() -> Vec<(CliProviderId, bool)> {
    let dirs = search_directories();
    CliProviderId::all()
        .into_iter()
        .map(|id| {
            let found = id
                .binaries()
                .iter()
                .any(|bin| find_on_path_in(&dirs, bin).is_some());
            (id, found)
        })
        .collect()
}

pub fn find_cli_binary(id: CliProviderId) -> Option<PathBuf> {
    let dirs = search_directories();
    id.binaries()
        .iter()
        .find_map(|binary| find_on_path_in(&dirs, binary))
}

pub fn cli_child_path_env(binary: &Path) -> Option<OsString> {
    let directories = search_directories();
    let mut combined = Vec::new();
    if let Some(node_dir) =
        find_on_path_in(&directories, "node").and_then(|p| p.parent().map(Path::to_path_buf))
    {
        combined.push(node_dir);
    }
    combined.extend(directories);

    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for dir in binary
        .parent()
        .into_iter()
        .chain(combined.iter().map(PathBuf::as_path))
    {
        if seen.insert(dir.to_path_buf()) {
            ordered.push(dir.to_path_buf());
        }
    }
    if ordered.is_empty() {
        None
    } else {
        std::env::join_paths(ordered).ok()
    }
}

pub fn cli_command(binary: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

pub fn kill_cli(child: &mut Child) {
    #[cfg(windows)]
    if matches!(child.try_wait(), Ok(None))
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        let taskkill = PathBuf::from(root).join("System32").join("taskkill.exe");
        let _ = cli_command(&taskkill)
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

pub fn run_cli_output(binary: &Path, args: &[&str], timeout: Duration) -> Result<Output, String> {
    let mut command = cli_command(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = cli_child_path_env(binary) {
        command.env("PATH", path);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "The CLI tool was found but could not be started.".to_string())?;

    fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut bytes);
            }
            let _ = tx.send(bytes);
        });
        rx
    }

    let stdout_rx = drain(child.stdout.take());
    let stderr_rx = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;

    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                kill_cli(&mut child);
                let _ = child.wait();
                return Err("The CLI tool did not respond in time.".to_string());
            }
            Err(_) => {
                kill_cli(&mut child);
                let _ = child.wait();
                return Err("The CLI tool reported a process error.".to_string());
            }
        }
    };

    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(500));
    let stdout = stdout_rx
        .recv_timeout(remaining)
        .map_err(|_| "Timed out reading CLI output.".to_string())?;
    let stderr = stderr_rx.recv_timeout(remaining).unwrap_or_default();

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub fn check_cli_provider(id: CliProviderId) -> (bool, bool, Option<String>, String) {
    let Some(binary) = find_cli_binary(id) else {
        return (
            false,
            false,
            None,
            format!(
                "Not found on PATH ({}). Install it and click Check.",
                id.binaries().join(" / ")
            ),
        );
    };
    match run_cli_output(&binary, &["--version"], VERSION_TIMEOUT) {
        Ok(output) if output.status.success() => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr)
            } else {
                String::from_utf8_lossy(&output.stdout)
            };
            let version: String = text
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(MAX_VERSION_CHARS)
                .collect();
            if version.is_empty() {
                (true, true, None, "Connected.".to_string())
            } else {
                (
                    true,
                    true,
                    Some(version.clone()),
                    format!("Connected · {version}"),
                )
            }
        }
        Ok(_) => (
            true,
            false,
            None,
            "Binary found, but `--version` returned an error.".to_string(),
        ),
        Err(msg) => (true, false, None, msg),
    }
}

// ==================== MODEL CATALOGS PER PROVIDER ====================

const CODEX_KNOWN_MODELS: &[(&str, &str, &str)] = &[
    (
        "gpt-5.6-sol",
        "GPT-5.6 Sol",
        "Most capable coding, research and computer use",
    ),
    ("gpt-5.6-terra", "GPT-5.6 Terra", "Balanced everyday work"),
    ("gpt-5.6-luna", "GPT-5.6 Luna", "Fast and affordable"),
    (
        "gpt-5.3-codex-spark",
        "GPT-5.3 Codex Spark",
        "Near-instant iteration",
    ),
];

const CLAUDE_KNOWN_MODELS: &[(&str, &str, bool)] = &[
    ("claude-opus-5-5", "Claude Opus 5.5", false),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5", false),
    ("claude-haiku-5-5", "Claude Haiku 5.5", true),
    ("claude-fable-5-2", "Claude Fable 5.2", false),
    ("claude-fable-5-1", "Claude Fable 5.1", false),
    ("claude-sonnet-5", "Claude Sonnet 5", false),
    ("claude-opus-5", "Claude Opus 5", false),
    ("claude-fable-5", "Claude Fable 5", false),
    ("claude-opus-4-8", "Claude Opus 4.8", false),
    ("claude-opus-4-7", "Claude Opus 4.7", false),
    ("claude-opus-4-6", "Claude Opus 4.6", false),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6", false),
    ("claude-haiku-4-5", "Claude Haiku 4.5", false),
    ("sonnet", "Claude Sonnet (latest alias)", false),
    ("opus", "Claude Opus (latest alias)", false),
    ("haiku", "Claude Haiku (latest alias)", false),
];

fn clean_model_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    if id.is_empty() || id.chars().count() > MAX_MODEL_ID_CHARS {
        return None;
    }
    if id.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(id.to_string())
}

fn valid_effort_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub fn claude_efforts(model: &str) -> Vec<String> {
    let name = model.to_ascii_lowercase();
    let levels: &[&str] = if name.contains("4-6") || name.contains("4.6") {
        &["low", "medium", "high", "max"]
    } else if matches!(name.as_str(), "opus" | "sonnet" | "fable")
        || name.contains("fable-5")
        || name.contains("opus-5")
        || name.contains("sonnet-5")
        || name.contains("haiku-5-5")
        || name.contains("haiku-5.5")
        || name.contains("opus-4-7")
        || name.contains("opus-4-8")
    {
        &["low", "medium", "high", "xhigh", "max"]
    } else {
        &[]
    };
    levels.iter().map(|v| v.to_string()).collect()
}

fn extract_json_document(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let end = text.rfind(['}', ']'])?;
    if end < start {
        return None;
    }
    Some(&text[start..=end])
}

fn codex_entry(value: &Value) -> Option<AiModel> {
    match value {
        Value::String(id) => clean_model_id(id).map(|id| AiModel {
            label: id.clone(),
            id,
            detail: None,
            is_default: false,
            reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
        }),
        Value::Object(map) => {
            let id = ["id", "slug", "model", "name"]
                .iter()
                .find_map(|k| map.get(*k)?.as_str())
                .and_then(clean_model_id)?;
            let label = ["display_name", "label"]
                .iter()
                .find_map(|k| map.get(*k)?.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            let detail = map
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(140).collect());
            let reasoning_efforts = map
                .get("supported_reasoning_levels")
                .and_then(|v| v.as_array())
                .map(|levels| {
                    levels
                        .iter()
                        .filter_map(|v| v.as_str().or_else(|| v.get("effort")?.as_str()))
                        .filter(|v| valid_effort_token(v))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec!["low".into(), "medium".into(), "high".into()]);
            Some(AiModel {
                id,
                label,
                detail,
                is_default: false,
                reasoning_efforts,
            })
        }
        _ => None,
    }
}

fn parse_codex_catalog(text: &str) -> Vec<AiModel> {
    let document = serde_json::from_str::<Value>(text)
        .ok()
        .or_else(|| extract_json_document(text).and_then(|doc| serde_json::from_str(doc).ok()));
    let Some(body) = document else {
        return Vec::new();
    };
    let mut entries: Vec<&Value> = Vec::new();
    match &body {
        Value::Array(items) => entries.extend(items.iter()),
        Value::Object(map) => {
            for key in ["models", "data", "catalog"] {
                if let Some(Value::Array(items)) = map.get(key) {
                    entries.extend(items.iter());
                    break;
                }
            }
            if entries.is_empty()
                && let Some(Value::Object(models_map)) = map.get("models")
            {
                let mut seen = HashSet::new();
                let mut out = Vec::new();
                for id in models_map.keys() {
                    if let Some(clean) = clean_model_id(id)
                        && seen.insert(clean.clone())
                    {
                        out.push(AiModel {
                            label: clean.clone(),
                            id: clean,
                            detail: None,
                            is_default: out.is_empty(),
                            reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
                        });
                    }
                    if out.len() >= MAX_MODELS {
                        break;
                    }
                }
                return out;
            }
        }
        _ => return Vec::new(),
    }

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for item in entries {
        if let Some(mut entry) = codex_entry(item)
            && seen.insert(entry.id.clone())
        {
            if models.is_empty() {
                entry.is_default = true;
            }
            models.push(entry);
        }
        if models.len() >= MAX_MODELS {
            break;
        }
    }
    models
}

fn parse_opencode_catalog(text: &str) -> Vec<AiModel> {
    let document = serde_json::from_str::<Value>(text)
        .ok()
        .or_else(|| extract_json_document(text).and_then(|doc| serde_json::from_str(doc).ok()));
    if let Some(body) = document
        && let Some(entries) = body
            .get("data")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
    {
        let mut models = Vec::new();
        let mut seen = HashSet::new();
        for entry in entries {
            let Some(provider) = entry["providerID"].as_str() else {
                continue;
            };
            let Some(model) = entry["modelID"].as_str().filter(|m| !m.contains('/')) else {
                continue;
            };
            let Some(id) = clean_model_id(&format!("{provider}/{model}")) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            let detail = entry["name"]
                .as_str()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(|n| n.chars().take(140).collect::<String>());
            let reasoning_efforts = entry["variants"]
                .as_array()
                .map(|variants| {
                    variants
                        .iter()
                        .filter_map(|v| v["id"].as_str().or_else(|| v.as_str()))
                        .filter(|tok| valid_effort_token(tok))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            models.push(AiModel {
                label: detail.clone().unwrap_or_else(|| id.clone()),
                id,
                detail,
                is_default: models.is_empty(),
                reasoning_efforts,
            });
            if models.len() >= MAX_MODELS {
                break;
            }
        }
        if !models.is_empty() {
            return models;
        }
    }

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        let line = line.trim().trim_matches('"').trim();
        if line.is_empty() || !line.contains('/') {
            continue;
        }
        if let Some(id) = clean_model_id(line)
            && seen.insert(id.clone())
        {
            models.push(AiModel {
                label: id.clone(),
                id,
                detail: None,
                is_default: models.is_empty(),
                reasoning_efforts: Vec::new(),
            });
        }
        if models.len() >= MAX_MODELS {
            break;
        }
    }
    models
}

fn parse_agy_models(text: &str) -> Vec<AiModel> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.trim().splitn(2, char::is_whitespace);
        let Some(first) = parts.next() else {
            continue;
        };
        let Some(id) = clean_model_id(first) else {
            continue;
        };
        if !id.contains('-')
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            || !seen.insert(id.clone())
        {
            continue;
        }
        let label = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string();
        out.push(AiModel {
            id,
            label,
            detail: None,
            is_default: out.is_empty(),
            reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
        });
        if out.len() >= MAX_MODELS {
            break;
        }
    }
    out
}

fn parse_qoder_table(text: &str) -> Vec<AiModel> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(first_tab) = line.trim().split('\t').next() else {
            continue;
        };
        let Some(first_col) = first_tab.split("  ").next() else {
            continue;
        };
        let id = first_col.trim();
        if id.is_empty()
            || id.eq_ignore_ascii_case("model")
            || id.len() > 128
            || id.chars().any(char::is_control)
            || !seen.insert(id.to_owned())
        {
            continue;
        }
        out.push(AiModel {
            id: id.to_owned(),
            label: id.to_owned(),
            detail: None,
            is_default: out.is_empty(),
            reasoning_efforts: Vec::new(),
        });
        if out.len() >= 500 {
            break;
        }
    }
    out
}

pub fn qoder_base_args() -> Vec<String> {
    [
        "--print",
        "--output-format",
        "stream-json",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn list_qoder_models_control(binary: &Path, timeout: Duration) -> Result<Vec<AiModel>, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let work_dir = std::env::temp_dir().join(format!("laralah_qoder_probe_{nanos}"));
    let _ = std::fs::create_dir_all(&work_dir);

    let mut command = cli_command(binary);
    command
        .args(qoder_base_args())
        .args(["--input-format", "stream-json"])
        .current_dir(&work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = cli_child_path_env(binary) {
        command.env("PATH", path);
    }
    let Ok(mut child) = command.spawn() else {
        let _ = std::fs::remove_dir_all(&work_dir);
        return Err("Qoder CLI could not be started.".to_string());
    };
    let Some(mut stdin) = child.stdin.take() else {
        kill_cli(&mut child);
        let _ = std::fs::remove_dir_all(&work_dir);
        return Err("Qoder stdin unavailable.".to_string());
    };
    let Some(stdout) = child.stdout.take() else {
        kill_cli(&mut child);
        let _ = std::fs::remove_dir_all(&work_dir);
        return Err("Qoder stdout unavailable.".to_string());
    };

    let req_id = format!("laralah-models-{nanos}");
    let req_line = json!({
        "type": "control_request",
        "request_id": req_id,
        "request": { "subtype": "get_models" }
    })
    .to_string();
    let _ = writeln!(stdin, "{req_line}");
    let _ = stdin.flush();

    let (tx, rx) = mpsc::sync_channel::<String>(8);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout.take((MAX_CATALOG_BYTES + 1) as u64));
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut found = Vec::new();
    while Instant::now() < deadline {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        let Ok(line) = rx.recv_timeout(wait) else {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v["type"] == "control_response"
            && v["response"]["request_id"].as_str() == Some(req_id.as_str())
        {
            if let Some(arr) = v["response"]["response"]["models"].as_array() {
                let mut seen = HashSet::new();
                for item in arr {
                    let Some(id) = item["id"]
                        .as_str()
                        .or_else(|| item["name"].as_str())
                        .and_then(clean_model_id)
                    else {
                        continue;
                    };
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    let label = item["display_name"]
                        .as_str()
                        .or_else(|| item["label"].as_str())
                        .unwrap_or(&id)
                        .to_string();
                    let efforts = item["reasoning_efforts"]
                        .as_array()
                        .map(|list| {
                            list.iter()
                                .filter_map(|e| e.as_str())
                                .filter(|e| valid_effort_token(e))
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    found.push(AiModel {
                        id,
                        label,
                        detail: None,
                        is_default: found.is_empty(),
                        reasoning_efforts: efforts,
                    });
                }
            }
            break;
        }
    }

    drop(stdin);
    kill_cli(&mut child);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&work_dir);

    if found.is_empty() {
        Err("Qoder models could not be loaded. Run `qodercli login` and retry.".to_string())
    } else {
        Ok(found)
    }
}

// ==================== CLAUDE MODELS.DEV DISK CACHE ====================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AiCatalogDiskCache {
    claude_fetched_at: Option<i64>,
    claude_failed_at: Option<i64>,
    #[serde(default)]
    claude_models: Vec<(String, String)>,
}

fn ai_cache_file_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let local = cwd.join(".laralah_ai_cache.json");
    if local.exists() || cwd.join("Cargo.toml").exists() {
        return local;
    }
    if let Some(home) = home_directory() {
        return home.join(".laralah_ai_cache.json");
    }
    local
}

fn load_ai_disk_cache() -> AiCatalogDiskCache {
    std::fs::read_to_string(ai_cache_file_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_ai_disk_cache(cache: &AiCatalogDiskCache) {
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(ai_cache_file_path(), json);
    }
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn parse_models_dev_anthropic(body: &Value) -> Vec<(String, String)> {
    const MIN_RELEASE: &str = "2025-10-15";
    let mut found: Vec<(String, (String, String))> = body
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            let id = key.strip_prefix("anthropic/")?;
            let released = value["release_date"].as_str().unwrap_or("");
            let dated = id.rsplit_once('-').is_some_and(|(_, tail)| {
                tail.len() == 8 && tail.bytes().all(|c| c.is_ascii_digit())
            });
            if dated
                || released < MIN_RELEASE
                || id.contains("mythos")
                || !id.starts_with("claude-")
                || clean_model_id(id).is_none()
            {
                return None;
            }
            let raw_name = value["name"].as_str().unwrap_or(id);
            let label = if raw_name.starts_with("Claude ") {
                raw_name.to_string()
            } else {
                format!("Claude {raw_name}")
            };
            Some((
                released.to_string(),
                (id.to_string(), label.chars().take(80).collect()),
            ))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.0.cmp(&b.1.0)));
    found.into_iter().map(|(_, entry)| entry).collect()
}

fn fetch_models_dev_via_curl() -> Result<Vec<(String, String)>, String> {
    let curl_bin = find_on_path_in(&fast_search_directories(), "curl")
        .unwrap_or_else(|| PathBuf::from("curl"));
    let out = run_cli_output(
        &curl_bin,
        &[
            "-fsSL",
            "--max-redirs",
            "0",
            "--max-time",
            "10",
            "https://models.dev/models.json",
        ],
        MODELS_TIMEOUT,
    )?;
    if !out.status.success() || out.stdout.len() > 8 * 1024 * 1024 {
        return Err("models.dev could not be reached.".to_string());
    }
    let body: Value =
        serde_json::from_slice(&out.stdout).map_err(|_| "Invalid models.dev JSON.".to_string())?;
    let parsed = parse_models_dev_anthropic(&body);
    if parsed.is_empty() {
        return Err("No Anthropic models found in models.dev.".to_string());
    }
    Ok(parsed)
}

fn build_claude_models(live: &[(String, String)]) -> Vec<AiModel> {
    let mut models: Vec<AiModel> = Vec::new();
    let mut seen = HashSet::new();

    for (id, label) in live {
        if seen.insert(id.clone()) {
            let is_def = id == "claude-haiku-5-5";
            models.push(AiModel {
                id: id.clone(),
                label: label.clone(),
                detail: None,
                is_default: is_def,
                reasoning_efforts: claude_efforts(id),
            });
        }
    }

    for (id, label, is_def) in CLAUDE_KNOWN_MODELS {
        if seen.insert((*id).to_string()) {
            models.push(AiModel {
                id: (*id).to_string(),
                label: (*label).to_string(),
                detail: None,
                is_default: *is_def && !models.iter().any(|m| m.is_default),
                reasoning_efforts: claude_efforts(id),
            });
        }
    }

    if !models.iter().any(|m| m.is_default)
        && let Some(first) = models.first_mut()
    {
        first.is_default = true;
    }
    models
}

fn load_claude_catalog(refresh: bool) -> AiModelList {
    let _guard = MODELS_DEV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut cache = load_ai_disk_cache();
    let now = now_unix_secs();

    if !refresh {
        if let Some(fetched_at) = cache.claude_fetched_at
            && now >= fetched_at
            && now - fetched_at < 24 * 60 * 60
            && !cache.claude_models.is_empty()
        {
            return AiModelList {
                provider: CliProviderId::ClaudeCode,
                source: "cached".to_string(),
                models: build_claude_models(&cache.claude_models),
                message: "Cached from models.dev (refreshed daily).".to_string(),
            };
        }
        if let Some(failed_at) = cache.claude_failed_at
            && now >= failed_at
            && now - failed_at < 60 * 60
        {
            let source = if cache.claude_models.is_empty() {
                "catalog"
            } else {
                "cached"
            };
            return AiModelList {
                provider: CliProviderId::ClaudeCode,
                source: source.to_string(),
                models: build_claude_models(&cache.claude_models),
                message: "Showing saved Claude model catalog.".to_string(),
            };
        }
    }

    match fetch_models_dev_via_curl() {
        Ok(live) => {
            cache.claude_fetched_at = Some(now);
            cache.claude_failed_at = None;
            cache.claude_models = live.clone();
            save_ai_disk_cache(&cache);
            AiModelList {
                provider: CliProviderId::ClaudeCode,
                source: "live".to_string(),
                models: build_claude_models(&live),
                message: "Updated live from models.dev.".to_string(),
            }
        }
        Err(_) => {
            cache.claude_failed_at = Some(now);
            save_ai_disk_cache(&cache);
            let source = if cache.claude_models.is_empty() {
                "catalog"
            } else {
                "cached"
            };
            AiModelList {
                provider: CliProviderId::ClaudeCode,
                source: source.to_string(),
                models: build_claude_models(&cache.claude_models),
                message: "Showing built-in Claude model catalog.".to_string(),
            }
        }
    }
}

pub fn list_models_for_provider(
    provider: CliProviderId,
    refresh: bool,
) -> Result<AiModelList, String> {
    match provider {
        CliProviderId::ClaudeCode => Ok(load_claude_catalog(refresh)),
        CliProviderId::Codex => {
            let binary = find_cli_binary(CliProviderId::Codex).ok_or_else(|| {
                "Codex CLI (`codex`) was not found on PATH.".to_string()
            })?;
            for (args, source, msg) in [
                (&["debug", "models"][..], "live", "Live catalog from Codex CLI."),
                (
                    &["debug", "models", "--bundled"][..],
                    "catalog",
                    "Bundled catalog from Codex CLI.",
                ),
            ] {
                if let Ok(output) = run_cli_output(&binary, args, MODEL_LIST_TIMEOUT)
                    && output.status.success()
                {
                    let models = parse_codex_catalog(&String::from_utf8_lossy(&output.stdout));
                    if !models.is_empty() {
                        return Ok(AiModelList {
                            provider,
                            source: source.to_string(),
                            models,
                            message: msg.to_string(),
                        });
                    }
                }
            }
            let fallback = CODEX_KNOWN_MODELS
                .iter()
                .enumerate()
                .map(|(idx, (id, label, detail))| AiModel {
                    id: (*id).to_string(),
                    label: (*label).to_string(),
                    detail: Some((*detail).to_string()),
                    is_default: idx == 0,
                    reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
                })
                .collect();
            Ok(AiModelList {
                provider,
                source: "catalog".to_string(),
                models: fallback,
                message: "Showing built-in Codex models.".to_string(),
            })
        }
        CliProviderId::Opencode => {
            let binary = find_cli_binary(CliProviderId::Opencode).ok_or_else(|| {
                "OpenCode CLI (`opencode`) was not found on PATH.".to_string()
            })?;
            let catalog = run_cli_output(&binary, &["api", "model.list"], MODEL_LIST_TIMEOUT)
                .ok()
                .filter(|o| o.status.success())
                .map(|o| parse_opencode_catalog(&String::from_utf8_lossy(&o.stdout)))
                .filter(|m| !m.is_empty())
                .or_else(|| {
                    run_cli_output(&binary, &["models"], MODEL_LIST_TIMEOUT)
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| parse_opencode_catalog(&String::from_utf8_lossy(&o.stdout)))
                        .filter(|m| !m.is_empty())
                });
            match catalog {
                Some(models) => Ok(AiModelList {
                    provider,
                    source: "live".to_string(),
                    models,
                    message: "Live catalog from OpenCode.".to_string(),
                }),
                None => Err(
                    "OpenCode models could not be loaded. Check OpenCode provider login and retry."
                        .to_string(),
                ),
            }
        }
        CliProviderId::Qoder => {
            let binary = find_cli_binary(CliProviderId::Qoder).ok_or_else(|| {
                "Qoder CLI (`qodercli`) was not found on PATH.".to_string()
            })?;
            let deadline = Instant::now() + MODEL_LIST_TIMEOUT;
            if let Ok(output) = run_cli_output(&binary, &["--list-models"], MODEL_LIST_TIMEOUT)
                && output.status.success()
            {
                let models = parse_qoder_table(&String::from_utf8_lossy(&output.stdout));
                if !models.is_empty() {
                    return Ok(AiModelList {
                        provider,
                        source: "live".to_string(),
                        models,
                        message: "Live models from Qoder CLI.".to_string(),
                    });
                }
            }
            let rem = deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_secs(3));
            let models = list_qoder_models_control(&binary, rem)?;
            Ok(AiModelList {
                provider,
                source: "live".to_string(),
                models,
                message: "Live models from Qoder CLI.".to_string(),
            })
        }
        CliProviderId::Agy => {
            let binary = find_cli_binary(CliProviderId::Agy).ok_or_else(|| {
                "Antigravity CLI (`agy`) was not found on PATH.".to_string()
            })?;
            let output = run_cli_output(&binary, &["models"], MODEL_LIST_TIMEOUT)?;
            if !output.status.success() {
                return Err(
                    "Antigravity models could not be loaded. Authenticate in `agy` first, then retry."
                        .to_string(),
                );
            }
            let models = parse_agy_models(&String::from_utf8_lossy(&output.stdout));
            if models.is_empty() {
                return Err("Antigravity returned an empty model list.".to_string());
            }
            Ok(AiModelList {
                provider,
                source: "live".to_string(),
                models,
                message: "Live models from Antigravity (`agy`).".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_opencode_agy_and_claude_catalogs() {
        let codex = parse_codex_catalog(
            r#"{"models":[{"id":"gpt-5.6-sol","display_name":"GPT-5.6 Sol","supported_reasoning_levels":["low","high"]}]}"#,
        );
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].id, "gpt-5.6-sol");
        assert_eq!(codex[0].reasoning_efforts, vec!["low", "high"]);

        let oc = parse_opencode_catalog("anthropic/claude-sonnet-5\nopenai/gpt-5\n");
        assert_eq!(oc.len(), 2);
        assert_eq!(oc[0].id, "anthropic/claude-sonnet-5");

        let agy = parse_agy_models("gemini-4-pro  Gemini 4 Pro\ngemini-4-flash Gemini 4 Flash\n");
        assert_eq!(agy.len(), 2);
        assert_eq!(agy[0].label, "Gemini 4 Pro");

        let claude = build_claude_models(&[]);
        assert!(claude.iter().any(|m| m.id == "claude-haiku-5-5" && m.is_default));
    }
}
