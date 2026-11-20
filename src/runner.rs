use crate::models::{
    extract_error_token_hint, EditorDiagnostic, EvaluationResult, ExceptionData, ExecutionTarget,
};
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::process::{Command, Stdio};

const JSON_START_MARKER: &str = "__TW_JSON_START__";
const JSON_END_MARKER: &str = "__TW_JSON_END__";

pub fn default_workspace_fallback_path() -> String {
    if let Ok(cwd) = std::env::current_dir() {
        let s = cwd.to_string_lossy().to_string();
        if is_laravel_project(&s) {
            return s;
        }
    }
    std::env::temp_dir().to_string_lossy().to_string()
}

#[cfg(test)]
pub fn default_playground_path() -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_app")
        .to_string_lossy()
        .to_string()
}

pub fn discover_laravel_projects() -> Vec<String> {
    let mut projects = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_str = cwd.to_string_lossy().to_string();
        if is_laravel_project(&cwd_str) {
            projects.push(cwd_str);
        }
    }
    projects
}

pub fn is_laravel_project(path: &str) -> bool {
    let p = std::path::Path::new(path);
    p.join("artisan").exists() && p.join("bootstrap").join("app.php").exists()
}

/// Finds the `docker` CLI. Apps started from the macOS Dock or Finder get a minimal PATH,
/// so the usual Docker Desktop / OrbStack / Homebrew install locations are checked too.
/// Not cached: Docker Desktop updates can move the CLI (e.g. drop the `/usr/local/bin`
/// symlink in favour of `~/.docker/bin`) while Laralah is running.
pub fn docker_binary() -> String {
    let exe = if cfg!(windows) { "docker.exe" } else { "docker" };
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(home) = crate::paths::home_dir() {
        candidates.push(home.join(".docker/bin/docker"));
        candidates.push(home.join(".orbstack/bin/docker"));
        candidates.push(home.join(".rd/bin/docker"));
    }
    candidates.extend([
        "/Applications/Docker.app/Contents/Resources/bin/docker".into(),
        "/usr/local/bin/docker".into(),
        "/opt/homebrew/bin/docker".into(),
        "/usr/bin/docker".into(),
    ]);
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| exe.to_string())
}

/// Running containers on this machine, with each one's configured working directory,
/// for the "Local Docker" connection form.
pub fn detect_local_docker_containers() -> Result<Vec<RemoteDockerContainer>> {
    let docker = docker_binary();
    let ps = Command::new(&docker)
        .args(["ps", "-q"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("Could not run '{docker}'. Is Docker installed?"))?;
    if !ps.status.success() {
        let stderr = String::from_utf8_lossy(&ps.stderr).trim().to_string();
        anyhow::bail!(
            "{}",
            if stderr.is_empty() {
                format!("docker ps exited with status {}", ps.status)
            } else {
                stderr
            }
        );
    }
    let ids: Vec<String> = String::from_utf8_lossy(&ps.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let out = Command::new(&docker)
        .arg("inspect")
        .arg("--format")
        .arg("{{.Name}}\t{{.Id}}\t{{.Config.Image}}\t{{.Config.WorkingDir}}")
        .args(&ids)
        .stdin(Stdio::null())
        .output()
        .context("docker inspect failed")?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(parse_remote_docker_inspect_output(&String::from_utf8_lossy(
        &out.stdout,
    )))
}

pub fn detect_docker_containers(fallback_local_path: &str) -> Vec<ExecutionTarget> {
    let output = Command::new(docker_binary())
        .args(["ps", "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}"])
        .output();

    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                Some(ExecutionTarget::Docker {
                    container_id: parts[0].trim().to_string(),
                    container_name: parts[1].trim().to_string(),
                    image: parts[2].trim().to_string(),
                    workdir: "/var/www/html".to_string(),
                    local_path: fallback_local_path.to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDockerContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    pub workdir: String,
}

pub fn parse_remote_docker_inspect_output(raw: &str) -> Vec<RemoteDockerContainer> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let parts: Vec<&str> = trimmed.split('\t').collect();
            if parts.len() < 3 {
                return None;
            }
            let name = parts[0].trim().trim_start_matches('/').to_string();
            let id: String = parts[1].trim().chars().take(12).collect();
            if name.is_empty() && id.is_empty() {
                return None;
            }
            let image = parts[2].trim().to_string();
            let raw_workdir = parts.get(3).map(|s| s.trim()).unwrap_or("");
            let workdir = if raw_workdir.starts_with('/') {
                raw_workdir.to_string()
            } else {
                crate::ssh::default_docker_application_folder().to_string()
            };
            Some(RemoteDockerContainer {
                id,
                name,
                image,
                workdir,
            })
        })
        .collect()
}

pub fn detect_remote_ssh_docker_containers(
    host: &str,
    port: u16,
    user: &str,
    auth_method: crate::ssh::SshAuthMethod,
    private_key: &str,
    passphrase: &str,
) -> Result<Vec<RemoteDockerContainer>> {
    let remote_cmd = "PATH=\"$PATH:/usr/local/bin:/snap/bin\"; ids=$(docker ps -q); if [ -n \"$ids\" ]; then docker inspect --format '{{.Name}}\t{{.Id}}\t{{.Config.Image}}\t{{.Config.WorkingDir}}' $ids; fi";
    let mut cmd = build_ssh_command_with_askpass(
        host,
        port,
        user,
        remote_cmd,
        auth_method,
        private_key,
        passphrase,
    )?;

    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let out = output.with_context(|| format!("Failed to spawn ssh to {host}:{port}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("SSH / Docker exited with status {}", out.status)
        } else {
            stderr
        };
        anyhow::bail!("{detail}");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(parse_remote_docker_inspect_output(&stdout))
}

pub fn default_ssh_targets() -> Vec<ExecutionTarget> {
    crate::ssh::load_ssh_store()
        .connections
        .into_iter()
        .map(|c| c.to_execution_target())
        .collect()
}

pub fn detect_local_laravel_version(project_path: &str) -> String {
    let root = std::path::Path::new(project_path);
    // Check composer.lock for laravel/framework version
    if let Ok(lock_raw) = std::fs::read_to_string(root.join("composer.lock"))
        && let Ok(re) = Regex::new(r#""name"\s*:\s*"laravel/framework"[\s\S]{0,280}?"version"\s*:\s*"v?([^"]+)""#)
        && let Some(caps) = re.captures(&lock_raw)
        && let Some(ver) = caps.get(1)
    {
        return ver.as_str().to_string();
    }
    // Check bootstrap/app.php or Application::VERSION constant
    if let Ok(boot_raw) = std::fs::read_to_string(root.join("bootstrap").join("app.php"))
        && let Ok(re) = Regex::new(r#"VERSION\s*=\s*'([^']+)'"#)
        && let Some(caps) = re.captures(&boot_raw)
        && let Some(ver) = caps.get(1)
    {
        return ver.as_str().to_string();
    }
    "11.x".to_string()
}

pub fn resolve_php_binary() -> String {
    if let Ok(explicit) = std::env::var("PHP_BINARY") {
        if !explicit.is_empty() && std::path::Path::new(&explicit).exists() {
            return explicit;
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("php");
            if candidate.is_file() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    for fallback in [
        "/opt/homebrew/opt/php@8.4/bin/php",
        "/opt/homebrew/opt/php@8.3/bin/php",
        "/opt/homebrew/opt/php@8.2/bin/php",
        "/opt/homebrew/bin/php",
        "/usr/local/bin/php",
        "/usr/bin/php",
    ] {
        if std::path::Path::new(fallback).is_file() {
            return fallback.to_string();
        }
    }
    "php".to_string()
}

#[cfg(test)]
pub fn evaluate_code(code: &str, target: &ExecutionTarget) -> EvaluationResult {
    evaluate_code_with_options(code, target, None)
}

pub fn evaluate_code_with_options(
    code: &str,
    target: &ExecutionTarget,
    php_bin_override: Option<&str>,
) -> EvaluationResult {
    let project_path = target.local_or_workdir();
    let script = build_piped_runner_script(project_path, code);

    match execute_php_script(&script, target, php_bin_override) {
        Ok(res) => res,
        Err(err) => error_result(
            "ExecutionError",
            format!("{err:#}"),
            0,
            project_path,
            Vec::new(),
        ),
    }
}

fn build_ssh_command_with_askpass(
    host: &str,
    port: u16,
    user: &str,
    remote_cmd: &str,
    auth_method: crate::ssh::SshAuthMethod,
    private_key: &str,
    passphrase: &str,
) -> Result<Command> {
    crate::ssh::validate_ssh_destination(host, user).map_err(|e| anyhow::anyhow!(e))?;
    let mut c = Command::new("ssh");
    let has_secret = !passphrase.is_empty();
    let ssh_args = crate::ssh::build_ssh_cli_args(
        host,
        port,
        user,
        remote_cmd,
        auth_method,
        private_key,
        has_secret,
    );
    c.args(&ssh_args);

    if has_secret {
        // ssh runs Laralah's own binary as its askpass helper (see `run_askpass_if_requested`).
        // The secret travels only through the ssh child's environment and is never written
        // to disk, and this works the same on macOS, Linux and Windows.
        let exe = std::env::current_exe().context("Failed to locate the Laralah executable")?;
        c.env(ASKPASS_MODE_ENV, "1");
        c.env(ASKPASS_SECRET_ENV, passphrase);
        c.env("SSH_ASKPASS", exe);
        c.env("SSH_ASKPASS_REQUIRE", "force");
        if std::env::var_os("DISPLAY").is_none() {
            c.env("DISPLAY", ":0");
        }
    }
    Ok(c)
}

fn build_target_php_command(
    target: &ExecutionTarget,
    php_bin: &str,
) -> Result<Command> {
    match target {
        ExecutionTarget::Local { project_path } => {
            let mut c = Command::new(php_bin);
            if std::path::Path::new(project_path).exists() {
                c.current_dir(project_path);
            }
            Ok(c)
        }
        ExecutionTarget::Docker {
            container_id,
            workdir,
            ..
        } => {
            let mut c = Command::new(docker_binary());
            c.args(["exec", "-i", "-w", workdir, container_id, "php"]);
            Ok(c)
        }
        ExecutionTarget::Ssh {
            local_docker: true,
            workdir,
            docker_container,
            ..
        } => {
            let container = docker_container
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .context("This Docker connection has no container name or ID")?;
            let workdir = match workdir.trim() {
                "" => crate::ssh::default_docker_application_folder(),
                w => w,
            };
            let mut c = Command::new(docker_binary());
            c.args(["exec", "-i", "-w", workdir, container, "php"]);
            Ok(c)
        }
        ExecutionTarget::Ssh {
            id,
            host,
            port,
            user,
            workdir,
            docker_container,
            auth_method,
            private_key,
            passphrase,
            ..
        } => {
            let remote_cmd =
                crate::ssh::build_ssh_remote_php_command(workdir, docker_container.as_deref());
            // Saved secrets are read from the Keychain only now, when the connection is used.
            let passphrase = if passphrase.is_empty()
                && *auth_method != crate::ssh::SshAuthMethod::SshAgent
            {
                crate::ssh::ssh_secret(id)
            } else {
                passphrase.clone()
            };
            build_ssh_command_with_askpass(
                host,
                *port,
                user,
                &remote_cmd,
                *auth_method,
                private_key,
                &passphrase,
            )
        }
    }
}

fn execute_php_script(
    script: &str,
    target: &ExecutionTarget,
    php_bin_override: Option<&str>,
) -> Result<EvaluationResult> {
    let php_bin = php_bin_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(resolve_php_binary);
    let mut cmd = build_target_php_command(target, &php_bin)?;

    let mut child = match cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(ch) => ch,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound
                && matches!(target, ExecutionTarget::Local { .. })
            {
                anyhow::bail!(
                    "PHP not found: couldn't run '{php_bin}'. Install PHP or choose a PHP binary in Settings → PHP Binary (or use \"Select PHP…\" on the home screen)."
                );
            }
            return Err(e).with_context(|| {
                format!("Failed to spawn PHP runner for target {}", target.label())
            });
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }

    let output = child.wait_with_output();
    let output = output?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if let (Some(start_idx), Some(end_idx)) =
        (stdout.find(JSON_START_MARKER), stdout.rfind(JSON_END_MARKER))
    {
        let json_slice = &stdout[start_idx + JSON_START_MARKER.len()..end_idx];
        let mut parsed: EvaluationResult = serde_json::from_str(json_slice.trim())
            .with_context(|| format!("Failed to parse runner JSON output: {json_slice}"))?;
        if !stderr.trim().is_empty() && parsed.raw_output.is_empty() {
            parsed.raw_output = stderr;
        }
        return Ok(parsed);
    }

    let combined = if stderr.trim().is_empty() {
        stdout
    } else if stdout.trim().is_empty() {
        stderr
    } else {
        format!("{stdout}\n{stderr}")
    };

    Ok(error_result(
        "PhpProcessError",
        if combined.trim().is_empty() {
            format!("PHP exited with status {}", output.status)
        } else {
            combined.trim().to_string()
        },
        0,
        target.local_or_workdir(),
        Vec::new(),
    ))
}

/// Runs fast `php -l` syntax validation against the editor buffer on stdin so syntax errors
/// appear live in the editor as the user types (even when Auto-Run is off).
#[cfg(test)]
pub fn lint_php_syntax(code: &str) -> Option<EditorDiagnostic> {
    lint_php_syntax_with_bin(code, None)
}

pub fn lint_php_syntax_with_bin(
    code: &str,
    php_bin_override: Option<&str>,
) -> Option<EditorDiagnostic> {
    if code.trim().is_empty() {
        return None;
    }
    let total_lines = code.split('\n').count().max(1);
    let trimmed_start = code.trim_start();
    let (payload, line_offset) = if trimmed_start.starts_with("<?php") || trimmed_start.starts_with("<?") {
        (code.to_string(), 0usize)
    } else {
        (format!("<?php\n{code}"), 1usize)
    };

    let php_bin = php_bin_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(resolve_php_binary);
    let mut child = Command::new(&php_bin)
        .args(["-d", "display_errors=1", "-l"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }

    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        return None;
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let err_line_raw = combined
        .lines()
        .map(str::trim)
        .find(|l| {
            !l.is_empty()
                && (l.starts_with("PHP Parse error:")
                    || l.starts_with("Parse error:")
                    || l.starts_with("PHP Fatal error:")
                    || l.starts_with("Fatal error:"))
        })?;

    let line_re = Regex::new(r"on line (\d+)\s*$").ok()?;
    let raw_line_num = line_re
        .captures(err_line_raw)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<usize>().ok())
        .unwrap_or(total_lines + line_offset);
    let editor_line = raw_line_num.saturating_sub(line_offset).clamp(1, total_lines);

    // Normalize message into clean `PHP Parse error: Syntax error, ... on line N`
    let mut clean = err_line_raw
        .replace("PHP Parse error:  ", "PHP Parse error: ")
        .replace(" in Standard input code ", " ")
        .replace(" in - ", " ");
    if let Some(rest) = clean.strip_prefix("Parse error:") {
        clean = format!("PHP Parse error: {}", rest.trim_start());
    } else if let Some(rest) = clean.strip_prefix("Fatal error:") {
        clean = format!("PHP Fatal error: {}", rest.trim_start());
    }
    clean = clean.replace(
        "PHP Parse error: syntax error",
        "PHP Parse error: Syntax error",
    );
    if let Ok(tok_re) = Regex::new(r#"unexpected token "([^"]+)""#) {
        clean = tok_re.replace_all(&clean, "unexpected '$1'").to_string();
    }
    clean = line_re
        .replace(&clean, format!("on line {editor_line}"))
        .to_string();

    let token_hint = extract_error_token_hint(&clean);
    Some(EditorDiagnostic {
        line: editor_line,
        message: clean,
        token_hint,
    })
}

fn error_result(
    class: &str,
    message: String,
    line: usize,
    file: &str,
    trace: Vec<String>,
) -> EvaluationResult {
    EvaluationResult {
        status: "error".to_string(),
        duration_ms: 0.0,
        memory_mb: 0.0,
        php_version: String::new(),
        laravel_version: String::new(),
        driver_name: "None".to_string(),
        driver_vars: Vec::new(),
        raw_output: message.clone(),
        dumps: Vec::new(),
        return_value: None,
        sql_queries: Vec::new(),
        mailable: None,
        table: None,
        exception: Some(ExceptionData {
            class: class.to_string(),
            message,
            line,
            file: file.to_string(),
            trace,
        }),
    }
}

const ASKPASS_MODE_ENV: &str = "LARALAH_SSH_ASKPASS";
const ASKPASS_SECRET_ENV: &str = "LARALAH_SSH_ASKPASS_SECRET";

/// Call first thing in `main`. When ssh launches this executable as its `SSH_ASKPASS`
/// helper, print the secret from the environment and return `true` so `main` exits
/// before any UI starts.
pub fn run_askpass_if_requested() -> bool {
    if std::env::var_os(ASKPASS_MODE_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return false;
    }
    let secret = std::env::var(ASKPASS_SECRET_ENV).unwrap_or_default();
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{secret}");
    let _ = out.flush();
    true
}

fn build_piped_runner_script(project_path: &str, code: &str) -> String {
    let b64_path = base64_encode(project_path.as_bytes());
    let b64_code = base64_encode(code.as_bytes());
    LARAVEL_RUNNER_PHP
        .replace("__PROJECT_PATH_B64__", &b64_path)
        .replace("__USER_CODE_B64__", &b64_code)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

const LARAVEL_RUNNER_PHP: &str = r##"<?php
declare(strict_types=1);

// mbstring is optional for the runner itself (e.g. Debian's php-cli ships without it).
function __tw_strlen(string $s): int
{
    return function_exists('mb_strlen') ? mb_strlen($s) : strlen($s);
}

function __tw_substr(string $s, int $start, int $length): string
{
    return function_exists('mb_substr') ? mb_substr($s, $start, $length) : substr($s, $start, $length);
}

$__tw_project_path = base64_decode('__PROJECT_PATH_B64__');
$__tw_user_code = base64_decode('__USER_CODE_B64__');

$__tw_dumps = [];
$__tw_sql_queries = [];
$__tw_sql_counts = [];
$__tw_node_counter = 0;
$__tw_dd_triggered = false;
$__tw_mute_sql = false;
$__tw_graph_rel_budget = 28;

class __TwDdAbort extends \RuntimeException {}

function __tw_next_id(string $prefix = 'node'): string {
    global $__tw_node_counter;
    $__tw_node_counter++;
    return $prefix . '-' . $__tw_node_counter;
}

function __tw_short_summary(mixed $val): string {
    if ($val === null) return 'null';
    if (is_bool($val)) return $val ? 'true' : 'false';
    if (is_int($val) || is_float($val)) return (string) $val;
    if (is_string($val)) {
        $clean = str_replace(["\r", "\n"], ['\r', '\n'], $val);
        if (__tw_strlen($clean) > 64) {
            return '"' . __tw_substr($clean, 0, 61) . '..."';
        }
        return '"' . $clean . '"';
    }
    if (is_array($val)) {
        return 'array(' . count($val) . ')';
    }
    if ($val instanceof \Illuminate\Support\Collection) {
        return 'Collection(' . $val->count() . ')';
    }
    if ($val instanceof \Illuminate\Database\Eloquent\Model) {
        $parts = explode('\\', get_class($val));
        $short = end($parts);
        $attrs = method_exists($val, 'getAttributes') ? $val->getAttributes() : [];
        $id = $attrs['id'] ?? null;
        return $id !== null ? "{$short}#{$id}" : $short;
    }
    if (is_object($val)) {
        $parts = explode('\\', get_class($val));
        return end($parts);
    }
    return gettype($val);
}

function __tw_serialize_node(
    mixed $val,
    string $label,
    int $depth = 0,
    ?string $visibility = null,
    array $seen = []
): array {
    $id = __tw_next_id('n');

    if ($val === null) {
        return [
            'id' => $id,
            'label' => $label,
            'type_name' => 'null',
            'class_name' => null,
            'visibility' => $visibility,
            'summary' => 'null',
            'scalar_value' => 'null',
            'children' => [],
            'expanded_by_default' => false,
        ];
    }

    if (is_bool($val)) {
        $s = $val ? 'true' : 'false';
        return [
            'id' => $id,
            'label' => $label,
            'type_name' => 'bool',
            'class_name' => null,
            'visibility' => $visibility,
            'summary' => $s,
            'scalar_value' => $s,
            'children' => [],
            'expanded_by_default' => false,
        ];
    }

    if (is_int($val)) {
        return [
            'id' => $id,
            'label' => $label,
            'type_name' => 'int',
            'class_name' => null,
            'visibility' => $visibility,
            'summary' => (string) $val,
            'scalar_value' => (string) $val,
            'children' => [],
            'expanded_by_default' => false,
        ];
    }

    if (is_float($val)) {
        return [
            'id' => $id,
            'label' => $label,
            'type_name' => 'float',
            'class_name' => null,
            'visibility' => $visibility,
            'summary' => (string) $val,
            'scalar_value' => (string) $val,
            'children' => [],
            'expanded_by_default' => false,
        ];
    }

    if (is_string($val)) {
        return [
            'id' => $id,
            'label' => $label,
            'type_name' => 'string(' . __tw_strlen($val) . ')',
            'class_name' => null,
            'visibility' => $visibility,
            'summary' => __tw_short_summary($val),
            'scalar_value' => '"' . $val . '"',
            'children' => [],
            'expanded_by_default' => false,
        ];
    }

    if (is_array($val)) {
        $children = [];
        if ($depth < 5) {
            $count = 0;
            foreach ($val as $k => $v) {
                if ($count >= 100) {
                    $children[] = [
                        'id' => __tw_next_id('n'),
                        'label' => '...',
                        'type_name' => 'truncated',
                        'class_name' => null,
                        'visibility' => null,
                        'summary' => (count($val) - 100) . ' more items',
                        'scalar_value' => null,
                        'children' => [],
                        'expanded_by_default' => false,
                    ];
                    break;
                }
                $children[] = __tw_serialize_node($v, (string) $k, $depth + 1, null, $seen);
                $count++;
            }
        }
        return [
            'id' => $id,
            'label' => $label,
            'type_name' => 'array:' . count($val),
            'class_name' => null,
            'visibility' => $visibility,
            'summary' => 'array(' . count($val) . ')',
            'scalar_value' => null,
            'children' => $children,
            'expanded_by_default' => $depth < 2,
        ];
    }

    if (is_object($val)) {
        $oid = spl_object_id($val);
        $fqcn = get_class($val);
        if (isset($seen[$oid]) || $depth >= 6) {
            return [
                'id' => $id,
                'label' => $label,
                'type_name' => 'object',
                'class_name' => $fqcn,
                'object_id' => $oid,
                'visibility' => $visibility,
                'summary' => __tw_short_summary($val) . (isset($seen[$oid]) ? ' (circular ref)' : ' (max depth)'),
                'scalar_value' => null,
                'children' => [],
                'expanded_by_default' => false,
            ];
        }
        $seen[$oid] = true;
        $children = [];

        if ($val instanceof \Illuminate\Support\Collection) {
            $items = $val->all();
            $idx = 0;
            foreach ($items as $k => $item) {
                if ($idx >= 100) break;
                $children[] = __tw_serialize_node($item, (string) $k, $depth + 1, null, $seen);
                $idx++;
            }
            return [
                'id' => $id,
                'label' => $label,
                'type_name' => 'Collection:' . count($items),
                'class_name' => $fqcn,
                'object_id' => $oid,
                'visibility' => $visibility,
                'summary' => $fqcn . ' (' . count($items) . ')',
                'scalar_value' => null,
                'children' => $children,
                'expanded_by_default' => $depth < 1,
            ];
        }

        if ($val instanceof \Illuminate\Database\Eloquent\Model) {
            $attrs = method_exists($val, 'getAttributes') ? $val->getAttributes() : [];
            $rels = method_exists($val, 'getRelations') ? $val->getRelations() : [];

            foreach ($attrs as $k => $v) {
                $children[] = __tw_serialize_node($v, (string) $k, $depth + 1, null, $seen);
            }
            foreach ($rels as $k => $v) {
                $children[] = __tw_serialize_node($v, (string) $k, $depth + 1, 'rel', $seen);
            }

            if ($depth <= 1) {
                global $__tw_mute_sql, $__tw_graph_rel_budget;
                if ($__tw_graph_rel_budget > 0) {
                    $mRef = new \ReflectionObject($val);
                    foreach ($mRef->getMethods(\ReflectionMethod::IS_PUBLIC) as $rm) {
                        if ($__tw_graph_rel_budget <= 0) break;
                        if ($rm->isStatic() || $rm->getNumberOfRequiredParameters() > 0) continue;
                        if ($rm->getDeclaringClass()->getName() !== $fqcn) continue;
                        $mName = $rm->getName();
                        if (array_key_exists($mName, $rels) || array_key_exists($mName, $attrs)) continue;
                        if (preg_match('/^(__|get|set|scope|boot|initialize|to|is|has|can|casts|new|resolve|prunable|broadcast|routeNotification)/i', $mName)) continue;

                        $looksLikeRel = false;
                        $retType = $rm->getReturnType();
                        if ($retType instanceof \ReflectionNamedType) {
                            $rtName = ltrim($retType->getName(), '?\\');
                            if (str_contains($rtName, 'Relation')
                                || str_contains($rtName, 'Collection')
                                || str_contains($rtName, '\\Models\\')
                                || is_subclass_of($rtName, \Illuminate\Database\Eloquent\Model::class)) {
                                $looksLikeRel = true;
                            }
                        } elseif (array_key_exists($mName . '_id', $attrs)) {
                            $looksLikeRel = true;
                        } else {
                            $mFile = $rm->getFileName();
                            $sLine = $rm->getStartLine();
                            $eLine = $rm->getEndLine();
                            if ($mFile && $sLine && $eLine && ($eLine - $sLine) <= 12 && is_file($mFile)) {
                                $mLines = @file($mFile);
                                if (is_array($mLines)) {
                                    $mBody = implode('', array_slice($mLines, $sLine - 1, $eLine - $sLine + 1));
                                    if (preg_match('/\$this\s*->\s*(hasOne|hasMany|belongsTo|belongsToMany|morphTo|morphOne|morphMany|morphToMany|morphedByMany|hasOneThrough|hasManyThrough)\s*\(/', $mBody)) {
                                        $looksLikeRel = true;
                                    }
                                }
                            }
                        }

                        if (!$looksLikeRel) continue;
                        $__tw_graph_rel_budget--;
                        $prevMute = $__tw_mute_sql;
                        $__tw_mute_sql = true;
                        try {
                            $relOut = $rm->invoke($val);
                            if (is_object($relOut) && method_exists($relOut, 'getResults')) {
                                $relOut = $relOut->getResults();
                            }
                            if ($relOut instanceof \Illuminate\Database\Eloquent\Model
                                || $relOut instanceof \Illuminate\Support\Collection) {
                                $children[] = __tw_serialize_node($relOut, $mName, $depth + 1, 'graph_rel', $seen);
                            }
                        } catch (\Throwable $ignored) {
                        } finally {
                            $__tw_mute_sql = $prevMute;
                        }
                    }
                }
            }

            return [
                'id' => $id,
                'label' => $label,
                'type_name' => 'Model',
                'class_name' => $fqcn,
                'object_id' => $oid,
                'visibility' => $visibility,
                'summary' => __tw_short_summary($val),
                'scalar_value' => null,
                'children' => $children,
                'expanded_by_default' => $depth < 1,
            ];
        }

        if ($val instanceof \DateTimeInterface) {
            $formatted = $val->format(\DateTimeInterface::ATOM);
            return [
                'id' => $id,
                'label' => $label,
                'type_name' => 'DateTime',
                'class_name' => $fqcn,
                'object_id' => $oid,
                'visibility' => $visibility,
                'summary' => $formatted,
                'scalar_value' => $formatted,
                'children' => [],
                'expanded_by_default' => false,
            ];
        }

        $ref = new \ReflectionObject($val);
        foreach ($ref->getProperties() as $prop) {
            $prop->setAccessible(true);
            $vis = $prop->isPublic() ? 'public' : ($prop->isProtected() ? 'protected' : 'private');
            $propVal = $prop->isInitialized($val) ? $prop->getValue($val) : null;
            $children[] = __tw_serialize_node($propVal, $prop->getName(), $depth + 1, $vis, $seen);
        }

        // Dynamic properties (e.g. stdClass)
        foreach (get_object_vars($val) as $k => $v) {
            $already = false;
            foreach ($children as $ch) {
                if ($ch['label'] === (string) $k) {
                    $already = true;
                    break;
                }
            }
            if (!$already) {
                $children[] = __tw_serialize_node($v, (string) $k, $depth + 1, 'public', $seen);
            }
        }

        return [
            'id' => $id,
            'label' => $label,
            'type_name' => 'object',
            'class_name' => $fqcn,
            'object_id' => $oid,
            'visibility' => $visibility,
            'summary' => __tw_short_summary($val) . ' {' . count($children) . '}',
            'scalar_value' => null,
            'children' => $children,
            'expanded_by_default' => $depth < 1,
            'html_preview' => ($depth === 0 ? __tw_extract_html_preview($val) : null),
        ];
    }

    return [
        'id' => $id,
        'label' => $label,
        'type_name' => gettype($val),
        'class_name' => null,
        'visibility' => $visibility,
        'summary' => (string) gettype($val),
        'scalar_value' => null,
        'children' => [],
        'expanded_by_default' => false,
    ];
}

function __tw_caller_line(): int {
    $bt = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 4);
    foreach ($bt as $frame) {
        if (isset($frame['file']) && str_contains((string) $frame['file'], "eval()'d code")) {
            return (int) ($frame['line'] ?? 0);
        }
    }
    return (int) ($bt[1]['line'] ?? 0);
}

function __tw_dump(mixed ...$vars): mixed {
    global $__tw_dumps, $__tw_project_path;
    $line = __tw_caller_line();
    foreach ($vars as $var) {
        $__tw_dumps[] = [
            'id' => __tw_next_id('dump'),
            'line' => $line,
            'file' => $__tw_project_path . '/scratchpad.php',
            'kind' => 'dump',
            'node' => __tw_serialize_node($var, 'dump()', 0),
        ];
    }
    return count($vars) === 1 ? $vars[0] : $vars;
}

function __tw_dd(mixed ...$vars): never {
    global $__tw_dumps, $__tw_project_path, $__tw_dd_triggered;
    $line = __tw_caller_line();
    foreach ($vars as $var) {
        $__tw_dumps[] = [
            'id' => __tw_next_id('dd'),
            'line' => $line,
            'file' => $__tw_project_path . '/scratchpad.php',
            'kind' => 'dd',
            'node' => __tw_serialize_node($var, 'dd()', 0),
        ];
    }
    $__tw_dd_triggered = true;
    throw new __TwDdAbort('dd() halted execution');
}

if (!function_exists('dump')) {
    function dump(mixed ...$vars): mixed {
        return __tw_dump(...$vars);
    }
}

if (!function_exists('dd')) {
    function dd(mixed ...$vars): never {
        __tw_dd(...$vars);
    }
}

function __tw_expr(int $line, mixed $val): mixed {
    global $__tw_dumps, $__tw_project_path;
    if ($val !== null) {
        $__tw_dumps[] = [
            'id' => __tw_next_id('expr'),
            'line' => $line,
            'file' => $__tw_project_path . '/scratchpad.php',
            'kind' => 'expr',
            'node' => __tw_serialize_node($val, 'expr', 0),
        ];
    }
    return $val;
}

function __tw_php_export(mixed $val): string {
    if ($val === null) return 'null';
    if (is_bool($val)) return $val ? 'true' : 'false';
    if (is_int($val) || is_float($val)) return (string) $val;
    if (is_string($val)) return "'" . str_replace(["\\", "'"], ["\\\\", "\\'"], $val) . "'";
    if ($val instanceof \Illuminate\Support\Collection || $val instanceof \Illuminate\Database\Eloquent\Model) {
        $val = $val->toArray();
    }
    if (is_array($val)) {
        $isList = array_is_list($val);
        $parts = [];
        foreach ($val as $k => $v) {
            if ($isList) {
                $parts[] = __tw_php_export($v);
            } else {
                $parts[] = __tw_php_export((string) $k) . ' => ' . __tw_php_export($v);
            }
        }
        return '[' . implode(', ', $parts) . ']';
    }
    if (is_object($val)) {
        return __tw_php_export(get_object_vars($val));
    }
    return 'null';
}

function __tw_cell_string(mixed $val): string {
    if ($val === null) return 'null';
    if (is_bool($val)) return $val ? 'true' : 'false';
    if (is_int($val) || is_float($val)) return (string) $val;
    if (is_string($val)) return $val;
    if ($val instanceof \DateTimeInterface) return $val->format('Y-m-d H:i:s');
    if ($val instanceof \Illuminate\Support\Collection) return '[' . $val->count() . ' items]';
    if (is_array($val)) return json_encode($val, JSON_UNESCAPED_SLASHES) ?: '[]';
    if (is_object($val)) {
        if (method_exists($val, '__toString')) return (string) $val;
        if (method_exists($val, 'toArray')) return json_encode($val->toArray(), JSON_UNESCAPED_SLASHES) ?: '{}';
        return __tw_short_summary($val);
    }
    return (string) $val;
}

function __tw_extract_table(mixed $val, string $sourceLabel): ?array {
    $items = null;
    if ($val instanceof \Illuminate\Support\Collection) {
        $items = $val->all();
    } elseif (is_array($val) && !empty($val)) {
        $items = array_values($val);
    }
    if ($items === null || count($items) === 0) {
        return null;
    }

    $normalizedRows = [];
    $columns = [];
    foreach ($items as $item) {
        $assoc = null;
        if ($item instanceof \Illuminate\Database\Eloquent\Model) {
            $assoc = $item->toArray();
        } elseif (is_array($item)) {
            $assoc = $item;
        } elseif (is_object($item)) {
            $assoc = method_exists($item, 'toArray') ? $item->toArray() : get_object_vars($item);
        }
        if (!is_array($assoc)) {
            return null;
        }
        // Must have string keys (associative row)
        $hasStringKey = false;
        foreach (array_keys($assoc) as $k) {
            if (is_string($k)) {
                $hasStringKey = true;
                break;
            }
        }
        if (!$hasStringKey && empty($assoc)) {
            continue;
        }
        if (!$hasStringKey) {
            return null;
        }
        foreach (array_keys($assoc) as $col) {
            $colStr = (string) $col;
            if (!in_array($colStr, $columns, true)) {
                $columns[] = $colStr;
            }
        }
        $normalizedRows[] = $assoc;
    }

    if (empty($columns) || empty($normalizedRows)) {
        return null;
    }

    $rows = [];
    $rawJson = [];
    $rawPhp = [];
    foreach ($normalizedRows as $assoc) {
        $rowCells = [];
        foreach ($columns as $col) {
            $rowCells[] = array_key_exists($col, $assoc) ? __tw_cell_string($assoc[$col]) : '';
        }
        $rows[] = $rowCells;
        $rawJson[] = json_encode($assoc, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) ?: '{}';
        $rawPhp[] = __tw_php_export($assoc);
    }

    return [
        'source_label' => $sourceLabel,
        'columns' => $columns,
        'rows' => $rows,
        'raw_rows_json' => $rawJson,
        'raw_rows_php' => $rawPhp,
    ];
}

function __tw_extract_html_preview(mixed $val): ?string {
    if (!is_object($val)) {
        return null;
    }
    global $__tw_mute_sql;
    $prevMute = $__tw_mute_sql;
    $__tw_mute_sql = true;
    try {
        if ($val instanceof \Illuminate\Contracts\Mail\Mailable) {
            return (string) $val->render();
        }
        if ($val instanceof \Illuminate\Mail\SentMessage) {
            if (method_exists($val, 'getHtmlBody')) {
                return (string) $val->getHtmlBody();
            }
            if (method_exists($val, 'getOriginalMessage')) {
                $orig = $val->getOriginalMessage();
                if (is_object($orig) && method_exists($orig, 'getHtmlBody')) {
                    return (string) $orig->getHtmlBody();
                }
            }
        }
        if (interface_exists('Illuminate\\Contracts\\Support\\Renderable')
            && $val instanceof \Illuminate\Contracts\Support\Renderable) {
            $out = $val->render();
            return is_object($out) && method_exists($out, 'toHtml') ? (string) $out->toHtml() : (string) $out;
        }
        if (interface_exists('Illuminate\\Contracts\\Support\\Htmlable')
            && $val instanceof \Illuminate\Contracts\Support\Htmlable) {
            return (string) $val->toHtml();
        }
        $cls = get_class($val);
        if ((str_contains($cls, '\\Mail\\') || str_ends_with($cls, 'MailMessage'))
            && method_exists($val, 'render')) {
            $rm = new \ReflectionMethod($val, 'render');
            if ($rm->isPublic() && $rm->getNumberOfRequiredParameters() === 0) {
                $out = $val->render();
                return is_object($out) && method_exists($out, 'toHtml') ? (string) $out->toHtml() : (string) $out;
            }
        }
    } catch (\Throwable $ignored) {
    } finally {
        $__tw_mute_sql = $prevMute;
    }
    return null;
}

function __tw_extract_mailable(mixed $val): ?array {
    if (!($val instanceof \Illuminate\Contracts\Mail\Mailable)) {
        return null;
    }
    $html = '';
    try {
        $html = $val->render();
    } catch (\Throwable $e) {
        $html = '<p>Render error: ' . htmlspecialchars($e->getMessage()) . '</p>';
    }
    $subject = method_exists($val, 'getSubject') ? (string) $val->getSubject() : get_class($val);
    $from = method_exists($val, 'getFrom') ? (string) $val->getFrom() : 'noreply@laravel.test';
    $to = method_exists($val, 'getTo') ? array_values((array) $val->getTo()) : [];
    $sections = [];
    if (method_exists($val, 'getSections')) {
        foreach ((array) $val->getSections() as $k => $v) {
            $sections[] = [(string) $k, (string) $v];
        }
    }
    $textSummary = trim(preg_replace('/\s+/', ' ', strip_tags($html)) ?? '');
    return [
        'class_name' => get_class($val),
        'subject' => $subject,
        'from' => $from,
        'to' => array_map('strval', $to),
        'html' => $html,
        'text_summary' => $textSummary,
        'markdown_sections' => $sections,
    ];
}

function __tw_prepare_code(string $code): array {
    $work = $code;
    if (preg_match('/^(\s*)(<\?php\b|<\?)/i', $work, $m)) {
        $tagLen = strlen($m[0]);
        // Preserve exact newline positions while blanking out the opening PHP tag
        $prefixKeepNewlines = preg_replace('/[^\r\n]/', ' ', substr($work, 0, $tagLen)) ?? str_repeat(' ', $tagLen);
        $work = $prefixKeepNewlines . substr($work, $tagLen);
    }
    if (trim($work) === '') {
        return ['', 1, false];
    }

    $tokens = @token_get_all("<?php\n" . $work);
    if (!is_array($tokens) || count($tokens) <= 1) {
        // Syntax error: return $work unmodified so eval() reports the exact ParseError line
        return [$work, substr_count(rtrim($work), "\n") + 1, false];
    }

    // Split into top-level statements while tracking exact 1-based editor line numbers
    $statements = [];
    $stmtEndLines = [];
    $current = '';
    $curLine = 1;
    $lastTokenEndLine = 1;
    $parenDepth = 0;
    $braceDepth = 0;
    $bracketDepth = 0;

    $count = count($tokens);
    for ($i = 1; $i < $count; $i++) {
        $tok = $tokens[$i];
        $text = is_array($tok) ? $tok[1] : $tok;
        $tokStartLine = $curLine;
        $curLine += substr_count($text, "\n");
        $tokEndLine = $curLine;

        $isTrivia = is_array($tok) && in_array($tok[0], [T_WHITESPACE, T_COMMENT, T_DOC_COMMENT], true);
        if (!$isTrivia && $tok !== ';') {
            $lastTokenEndLine = $tokEndLine;
        }

        $current .= $text;

        if (!is_array($tok)) {
            if ($tok === '(') $parenDepth++;
            elseif ($tok === ')') $parenDepth = max(0, $parenDepth - 1);
            elseif ($tok === '[') $bracketDepth++;
            elseif ($tok === ']') $bracketDepth = max(0, $bracketDepth - 1);
            elseif ($tok === '{') $braceDepth++;
            elseif ($tok === '}') {
                $braceDepth = max(0, $braceDepth - 1);
                if ($parenDepth === 0 && $bracketDepth === 0 && $braceDepth === 0) {
                    $check = __tw_strip_leading_comments($current);
                    if (preg_match('/^(if|elseif|else|for|foreach|while|switch|try|catch|finally|class|abstract|final|readonly|interface|trait|enum|function|namespace)\b/i', $check)) {
                        $statements[] = $current;
                        $stmtEndLines[] = $lastTokenEndLine;
                        $current = '';
                    }
                }
            } elseif ($tok === ';' && $parenDepth === 0 && $bracketDepth === 0 && $braceDepth === 0) {
                $statements[] = $current;
                $stmtEndLines[] = $lastTokenEndLine > 0 ? $lastTokenEndLine : $tokStartLine;
                $current = '';
            }
        } else {
            if ($tok[0] === T_CURLY_OPEN || $tok[0] === T_DOLLAR_OPEN_CURLY_BRACES) {
                $braceDepth++;
            }
        }
    }
    if ($current !== '') {
        $statements[] = $current;
        $stmtEndLines[] = $lastTokenEndLine;
    }

    // Find last non-empty, non-comment statement
    $lastIdx = null;
    for ($i = count($statements) - 1; $i >= 0; $i--) {
        if (__tw_strip_leading_comments($statements[$i]) !== '') {
            $lastIdx = $i;
            break;
        }
    }

    $nonReturnRe = '/^(return|echo|print|throw|exit|die|yield|goto|use|namespace|class|abstract|final|readonly|interface|trait|enum|function|if|for|foreach|while|do|switch|try|declare|unset|global|const|include|include_once|require|require_once|dump|dd|var_dump|print_r|__tw_dump|__tw_dd)\b/i';
    $nonIntermediateRe = '/^(return|echo|print|throw|exit|die|yield|goto|use|namespace|class|abstract|final|readonly|interface|trait|enum|function|if|for|foreach|while|do|switch|try|declare|unset|global|const|include|include_once|require|require_once|dump|dd|var_dump|print_r|__tw_dump|__tw_dd|__tw_expr)\b/i';

    $finalReturnLine = substr_count(rtrim($work), "\n") + 1;
    // True when the final statement only prints (echo/print/...): its "return value" is a
    // meaningless null, so the caller should show the captured output instead of a null card.
    $finalIsOutputOnly = false;

    if ($lastIdx !== null) {
        // 1. Blank-line block rule: wrap the final expression of each blank-line-separated block
        //    (before $lastIdx) in __tw_expr(line, expr) so multi-line blocks run together while
        //    blank-line breaks emit distinct Detail Dive cards in a single shared-scope run.
        for ($idx = 0; $idx < $lastIdx; $idx++) {
            [$prefix, $core, $suffix] = __tw_split_stmt_trivia($statements[$idx]);
            if ($core === '' || !str_ends_with($core, ';')) {
                continue;
            }

            // Collect trivia (whitespace/comments) between this ';' and the next real statement
            $between = $suffix;
            for ($scan = $idx + 1; $scan <= $lastIdx; $scan++) {
                [$nextPrefix, $nextCore, $nextSuffix] = __tw_split_stmt_trivia($statements[$scan]);
                $between .= $nextPrefix;
                if ($nextCore !== '') {
                    break;
                }
                $between .= $nextSuffix;
            }
            if (!preg_match("/\r?\n[ \t]*\r?\n/", $between)) {
                continue;
            }

            $exprBody = rtrim(substr($core, 0, -1));
            $trailingWsBeforeSemi = substr($core, strlen($exprBody), strlen($core) - 1 - strlen($exprBody));
            if ($exprBody === '' || preg_match($nonIntermediateRe, $exprBody)) {
                continue;
            }
            if (__tw_has_top_level_assignment($exprBody)) {
                continue;
            }
            $lineNo = (int) ($stmtEndLines[$idx] ?? 1);
            $statements[$idx] = $prefix . '__tw_expr(' . $lineNo . ', ' . $exprBody . $trailingWsBeforeSemi . ');' . $suffix;
        }

        // 2. Wrap final statement in return ...;
        [$prefixLast, $coreLast, $suffixLast] = __tw_split_stmt_trivia($statements[$lastIdx]);
        $finalReturnLine = (int) ($stmtEndLines[$lastIdx] ?? $finalReturnLine);
        $finalIsOutputOnly = (bool) preg_match('/^(echo|print|printf|var_dump|print_r)\b/i', $coreLast);
        if ($coreLast !== '' && !preg_match($nonReturnRe, $coreLast)) {
            $hasSemi = str_ends_with($coreLast, ';');
            $statements[$lastIdx] = $prefixLast . 'return ' . $coreLast . ($hasSemi ? '' : ';') . $suffixLast;
        }
    }

    return [implode('', $statements), $finalReturnLine, $finalIsOutputOnly];
}

function __tw_split_stmt_trivia(string $stmt): array {
    $tokens = @token_get_all("<?php\n" . $stmt);
    if (!is_array($tokens)) {
        return ['', trim($stmt), ''];
    }
    $n = count($tokens);
    $firstReal = null;
    $lastReal = null;
    for ($i = 1; $i < $n; $i++) {
        $tok = $tokens[$i];
        $isTrivia = is_array($tok) && in_array($tok[0], [T_WHITESPACE, T_COMMENT, T_DOC_COMMENT], true);
        if (!$isTrivia) {
            if ($firstReal === null) {
                $firstReal = $i;
            }
            $lastReal = $i;
        }
    }
    if ($firstReal === null || $lastReal === null) {
        return [$stmt, '', ''];
    }
    $prefix = '';
    for ($i = 1; $i < $firstReal; $i++) {
        $tok = $tokens[$i];
        $prefix .= is_array($tok) ? $tok[1] : $tok;
    }
    $core = '';
    for ($i = $firstReal; $i <= $lastReal; $i++) {
        $tok = $tokens[$i];
        $core .= is_array($tok) ? $tok[1] : $tok;
    }
    $suffix = '';
    for ($i = $lastReal + 1; $i < $n; $i++) {
        $tok = $tokens[$i];
        $suffix .= is_array($tok) ? $tok[1] : $tok;
    }
    return [$prefix, $core, $suffix];
}

function __tw_has_top_level_assignment(string $expr): bool {
    $tokens = @token_get_all("<?php\n" . $expr);
    if (!is_array($tokens)) {
        return false;
    }
    $p = 0;
    $b = 0;
    $c = 0;
    $assignToks = [
        T_PLUS_EQUAL,
        T_MINUS_EQUAL,
        T_MUL_EQUAL,
        T_DIV_EQUAL,
        T_CONCAT_EQUAL,
        T_MOD_EQUAL,
        T_AND_EQUAL,
        T_OR_EQUAL,
        T_XOR_EQUAL,
        T_SL_EQUAL,
        T_SR_EQUAL,
        T_POW_EQUAL,
        T_COALESCE_EQUAL,
        T_INC,
        T_DEC,
    ];
    $n = count($tokens);
    for ($i = 1; $i < $n; $i++) {
        $tok = $tokens[$i];
        if (!is_array($tok)) {
            if ($tok === '(') $p++;
            elseif ($tok === ')') $p = max(0, $p - 1);
            elseif ($tok === '[') $b++;
            elseif ($tok === ']') $b = max(0, $b - 1);
            elseif ($tok === '{') $c++;
            elseif ($tok === '}') $c = max(0, $c - 1);
            elseif ($tok === '=' && $p === 0 && $b === 0 && $c === 0) {
                return true;
            }
        } else {
            if ($tok[0] === T_CURLY_OPEN || $tok[0] === T_DOLLAR_OPEN_CURLY_BRACES) {
                $c++;
            } elseif ($p === 0 && $b === 0 && $c === 0 && in_array($tok[0], $assignToks, true)) {
                return true;
            }
        }
    }
    return false;
}

function __tw_strip_leading_comments(string $stmt): string {
    $tokens = @token_get_all("<?php\n" . $stmt);
    $out = '';
    $started = false;
    for ($i = 1; $i < count($tokens); $i++) {
        $tok = $tokens[$i];
        if (!$started && is_array($tok) && in_array($tok[0], [T_WHITESPACE, T_COMMENT, T_DOC_COMMENT], true)) {
            continue;
        }
        $started = true;
        $out .= is_array($tok) ? $tok[1] : $tok;
    }
    return trim($out);
}

// Bootstrap Laravel project
$__tw_laravel_version = 'PHP Only';
$__tw_driver_name = 'Standard Composer';
$__tw_driver_vars = [];
$__tw_app = null;

$autoloadPath = rtrim($__tw_project_path, '/') . '/vendor/autoload.php';
if (file_exists($autoloadPath)) {
    require_once $autoloadPath;
}

$bootstrapPath = rtrim($__tw_project_path, '/') . '/bootstrap/app.php';
if (file_exists($bootstrapPath)) {
    $__tw_app = require $bootstrapPath;
    if (is_object($__tw_app)) {
        if (method_exists($__tw_app, 'version')) {
            $__tw_laravel_version = 'Laravel ' . $__tw_app->version();
        } else {
            $__tw_laravel_version = 'Laravel';
        }
        $__tw_driver_name = 'LaravelDriver';

        // Boot Console Kernel if running on a full Laravel installation
        $kernelInterface = 'Illuminate\\Contracts\\Console\\Kernel';
        if (interface_exists($kernelInterface) && method_exists($__tw_app, 'make')) {
            try {
                $kernel = $__tw_app->make($kernelInterface);
                if (is_object($kernel) && method_exists($kernel, 'bootstrap')) {
                    $kernel->bootstrap();
                }
            } catch (\Throwable $ignored) {
            }
        }
    }
}

// Detect & apply custom .laralah/*Driver.php
$driverFiles = glob(rtrim($__tw_project_path, '/') . '/.laralah/*Driver.php') ?: [];
foreach ($driverFiles as $driverFile) {
    require_once $driverFile;
    $className = basename($driverFile, '.php');
    if (class_exists($className)) {
        $driver = new $className();
        if (method_exists($driver, 'canBootstrap') && $driver->canBootstrap($__tw_project_path)) {
            if (method_exists($driver, 'bootstrap')) {
                $driver->bootstrap($__tw_app);
            }
            $__tw_driver_name = $className;
            if (method_exists($driver, 'getAvailableVariables')) {
                $__tw_driver_vars = (array) $driver->getAvailableVariables();
            }
            break;
        }
    }
}

// Register Tinker-style short class-alias autoloader for Models, Mailables, Facades & Support
spl_autoload_register(static function (string $shortClass): void {
    if (str_contains($shortClass, '\\')) {
        return;
    }
    $prefixes = [
        'App\\Models\\',
        'App\\Mail\\',
        'App\\Jobs\\',
        'App\\Events\\',
        'App\\Services\\',
        'App\\',
        'Illuminate\\Support\\Facades\\',
        'Illuminate\\Support\\',
    ];
    foreach ($prefixes as $prefix) {
        $candidate = $prefix . $shortClass;
        if (class_exists($candidate) || interface_exists($candidate) || enum_exists($candidate)) {
            @class_alias($candidate, $shortClass);
            return;
        }
    }
});

// Register DB::listen() for SQL query inspection
if (class_exists('Illuminate\\Support\\Facades\\DB')) {
    if (method_exists('Illuminate\\Support\\Facades\\DB', 'flushQueryLog')) {
        \Illuminate\Support\Facades\DB::flushQueryLog();
    }
    \Illuminate\Support\Facades\DB::listen(function ($query) use (&$__tw_sql_queries, &$__tw_sql_counts) {
        global $__tw_mute_sql;
        if (!empty($__tw_mute_sql)) {
            return;
        }
        $sql = (string) ($query->sql ?? '');
        $bindings = (array) ($query->bindings ?? []);
        $time = (float) ($query->time ?? 0.0);
        $conn = (string) ($query->connectionName ?? 'default');

        $bindingStrings = [];
        $interpolated = $sql;
        foreach ($bindings as $b) {
            if ($b === null) {
                $rep = 'NULL';
            } elseif (is_bool($b)) {
                $rep = $b ? '1' : '0';
            } elseif (is_int($b) || is_float($b)) {
                $rep = (string) $b;
            } else {
                $rep = "'" . str_replace("'", "''", (string) $b) . "'";
            }
            $bindingStrings[] = $rep;
            $pos = strpos($interpolated, '?');
            if ($pos !== false) {
                $interpolated = substr_replace($interpolated, $rep, $pos, 1);
            }
        }

        $sqlLine = 0;
        foreach (debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 24) as $btFrame) {
            if (isset($btFrame['file']) && str_contains((string) $btFrame['file'], "eval()'d code")) {
                $sqlLine = (int) ($btFrame['line'] ?? 0);
                break;
            }
        }

        $__tw_sql_counts[$sql] = ($__tw_sql_counts[$sql] ?? 0) + 1;
        $__tw_sql_queries[] = [
            'sql' => $sql,
            'bindings' => $bindingStrings,
            'interpolated_sql' => $interpolated,
            'time_ms' => round($time, 2),
            'connection' => $conn,
            'is_duplicate' => false,
            'line' => $sqlLine,
        ];
    });
}

[$__tw_prepared, $__tw_final_return_line, $__tw_final_is_output_only] = __tw_prepare_code($__tw_user_code);
$__tw_status = 'ok';
$__tw_return_val = null;
$__tw_has_return = false;
$__tw_exception = null;

$__tw_t0 = microtime(true);
ob_start();
try {
    $__tw_exec = static function (string $__code, array $__vars) {
        extract($__vars, EXTR_SKIP);
        return eval($__code);
    };
    $__tw_return_val = $__tw_exec($__tw_prepared, $__tw_driver_vars);
    $__tw_has_return = true;
} catch (__TwDdAbort $e) {
    $__tw_status = 'ok';
} catch (\Throwable $e) {
    $__tw_status = 'error';
    $traceLines = [];
    $userLine = str_contains($e->getFile(), "eval()'d code") ? (int) $e->getLine() : 0;
    foreach (array_slice($e->getTrace(), 0, 12) as $idx => $frame) {
        $f = (string) ($frame['file'] ?? '[internal]');
        $l = (int) ($frame['line'] ?? 0);
        if ($userLine === 0 && str_contains($f, "eval()'d code") && $l > 0) {
            $userLine = $l;
        }
        $fn = ($frame['class'] ?? '') . ($frame['type'] ?? '') . ($frame['function'] ?? '');
        $traceLines[] = "#{$idx} {$f}({$l}): {$fn}()";
    }
    if ($userLine === 0) {
        $userLine = (int) $e->getLine();
    }
    $errMsg = $e->getMessage();
    if ($e instanceof \ParseError) {
        $norm = preg_replace('/^syntax error/i', 'Syntax error', $errMsg) ?? $errMsg;
        $norm = preg_replace('/unexpected token "([^"]+)"/', "unexpected '$1'", $norm) ?? $norm;
        $errMsg = "PHP Parse error: {$norm} on line {$userLine}";
    }
    $__tw_exception = [
        'class' => get_class($e),
        'message' => $errMsg,
        'line' => $userLine,
        'file' => $e->getFile(),
        'trace' => $traceLines,
    ];
}
$__tw_raw_output = (string) ob_get_clean();
$__tw_duration_ms = round((microtime(true) - $__tw_t0) * 1000.0, 2);
$__tw_memory_mb = round(memory_get_peak_usage(true) / (1024 * 1024), 2);

// Mark duplicate SQL queries (N+1 indicator)
foreach ($__tw_sql_queries as &$qEntry) {
    if (($__tw_sql_counts[$qEntry['sql']] ?? 0) > 1) {
        $qEntry['is_duplicate'] = true;
    }
}
unset($qEntry);

$__tw_return_card = null;
if ($__tw_has_return && $__tw_final_is_output_only && $__tw_return_val === null) {
    // e.g. `echo "..."` as the last line: nothing was returned, the text is in raw_output.
    $__tw_has_return = false;
}
if ($__tw_has_return && !$__tw_dd_triggered && $__tw_exception === null) {
    $__tw_return_card = [
        'id' => __tw_next_id('ret'),
        'line' => $__tw_final_return_line,
        'file' => $__tw_project_path . '/scratchpad.php',
        'kind' => 'return',
        'node' => __tw_serialize_node($__tw_return_val, 'return', 0),
    ];
}

// Detect Mailable from return value or dumps
$__tw_mailable = null;
if ($__tw_has_return) {
    $__tw_mailable = __tw_extract_mailable($__tw_return_val);
}

// Extract Table Mode dataset from return value (if collection/array of rows)
$__tw_table = null;
if ($__tw_has_return && $__tw_mailable === null) {
    $__tw_table = __tw_extract_table($__tw_return_val, 'Return Value');
}

$__tw_driver_var_pairs = [];
foreach ($__tw_driver_vars as $k => $v) {
    $__tw_driver_var_pairs[] = ['$' . $k, __tw_short_summary($v)];
}

$__tw_payload = [
    'status' => $__tw_status,
    'duration_ms' => $__tw_duration_ms,
    'memory_mb' => $__tw_memory_mb,
    'php_version' => PHP_VERSION,
    'laravel_version' => $__tw_laravel_version,
    'driver_name' => $__tw_driver_name,
    'driver_vars' => $__tw_driver_var_pairs,
    'raw_output' => $__tw_raw_output,
    'dumps' => $__tw_dumps,
    'return_value' => $__tw_return_card,
    'sql_queries' => $__tw_sql_queries,
    'mailable' => $__tw_mailable,
    'table' => $__tw_table,
    'exception' => $__tw_exception,
];

echo "\n__TW_JSON_START__\n";
echo json_encode($__tw_payload, JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE);
echo "\n__TW_JSON_END__\n";
"##;

const SCHEMA_START_MARKER: &str = "__TW_SCHEMA_START__";
const SCHEMA_END_MARKER: &str = "__TW_SCHEMA_END__";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaIntrospectRequest {
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub list_tables: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaColumnInfo {
    pub name: String,
    pub col_type: String,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaRelationInfo {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub related: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelSchemaInfo {
    pub fqcn: String,
    pub table: String,
    pub primary_key: String,
    #[serde(default)]
    pub timestamps: bool,
    #[serde(default)]
    pub soft_deletes: bool,
    #[serde(default)]
    pub columns: Vec<SchemaColumnInfo>,
    #[serde(default)]
    pub fillable: Vec<String>,
    #[serde(default)]
    pub casts: Vec<(String, String)>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub relations: Vec<SchemaRelationInfo>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TableSchemaInfo {
    pub table: String,
    #[serde(default)]
    pub columns: Vec<SchemaColumnInfo>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaIntrospectResponse {
    pub status: String,
    #[serde(default)]
    pub laravel_version: String,
    #[serde(default)]
    pub php_version: String,
    #[serde(default)]
    pub db_driver: String,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub all_tables: Vec<String>,
    #[serde(default)]
    pub models: Vec<ModelSchemaInfo>,
    #[serde(default)]
    pub tables: Vec<TableSchemaInfo>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn introspect_laravel_schema(
    request: &SchemaIntrospectRequest,
    target: &ExecutionTarget,
    php_bin_override: Option<&str>,
) -> Result<SchemaIntrospectResponse> {
    let project_path = target.local_or_workdir();
    let req_json = serde_json::to_string(request).unwrap_or_else(|_| "{}".to_string());
    let script = LARAVEL_SCHEMA_PHP
        .replace("__PROJECT_PATH_B64__", &base64_encode(project_path.as_bytes()))
        .replace("__SCHEMA_REQ_B64__", &base64_encode(req_json.as_bytes()));

    let php_bin = php_bin_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(resolve_php_binary);

    let mut cmd = build_target_php_command(target, &php_bin)?;

    let mut child = match cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(ch) => ch,
        Err(e) => {
            return Err(e).with_context(|| {
                format!("Failed to spawn schema inspector for {}", target.label())
            });
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }

    let output = child.wait_with_output();
    let output = output?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    if let (Some(start_idx), Some(end_idx)) =
        (stdout.find(SCHEMA_START_MARKER), stdout.rfind(SCHEMA_END_MARKER))
    {
        let json_slice = &stdout[start_idx + SCHEMA_START_MARKER.len()..end_idx];
        let parsed: SchemaIntrospectResponse = serde_json::from_str(json_slice.trim())
            .with_context(|| format!("Failed to parse schema JSON: {json_slice}"))?;
        return Ok(parsed);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "Schema inspection did not return structured metadata: {}",
        stderr.trim()
    )
}

const LARAVEL_SCHEMA_PHP: &str = r##"<?php
declare(strict_types=1);

$__tw_project_path = base64_decode('__PROJECT_PATH_B64__');
$__tw_req = json_decode(base64_decode('__SCHEMA_REQ_B64__'), true) ?: [];

$__tw_laravel_version = 'PHP Only';
$__tw_app = null;

$autoloadPath = rtrim($__tw_project_path, '/') . '/vendor/autoload.php';
if (file_exists($autoloadPath)) {
    require_once $autoloadPath;
}

$bootstrapPath = rtrim($__tw_project_path, '/') . '/bootstrap/app.php';
if (file_exists($bootstrapPath)) {
    $__tw_app = require $bootstrapPath;
    if (is_object($__tw_app)) {
        $__tw_laravel_version = method_exists($__tw_app, 'version')
            ? 'Laravel ' . $__tw_app->version()
            : 'Laravel';
        $kernelInterface = 'Illuminate\\Contracts\\Console\\Kernel';
        if (interface_exists($kernelInterface) && method_exists($__tw_app, 'make')) {
            try {
                $kernel = $__tw_app->make($kernelInterface);
                if (is_object($kernel) && method_exists($kernel, 'bootstrap')) {
                    $kernel->bootstrap();
                }
            } catch (\Throwable $ignored) {
            }
        }
    }
}

$driverFiles = glob(rtrim($__tw_project_path, '/') . '/.laralah/*Driver.php') ?: [];
foreach ($driverFiles as $driverFile) {
    require_once $driverFile;
    $className = basename($driverFile, '.php');
    if (class_exists($className)) {
        $driver = new $className();
        if (method_exists($driver, 'canBootstrap') && $driver->canBootstrap($__tw_project_path)) {
            if (method_exists($driver, 'bootstrap')) {
                $driver->bootstrap($__tw_app);
            }
            break;
        }
    }
}

function __tw_resolve_model_class(string $raw): ?string {
    $clean = ltrim(trim($raw), '\\');
    if ($clean === '') {
        return null;
    }
    if (class_exists($clean)) {
        return $clean;
    }
    foreach (['App\\Models\\', 'App\\', 'Domain\\'] as $prefix) {
        $candidate = $prefix . $clean;
        if (class_exists($candidate)) {
            return $candidate;
        }
    }
    return null;
}

function __tw_table_columns(string $table): array {
    $cols = [];
    if (class_exists('Illuminate\\Support\\Facades\\Schema')) {
        try {
            if (method_exists('Illuminate\\Support\\Facades\\Schema', 'getColumns')) {
                $rawCols = \Illuminate\Support\Facades\Schema::getColumns($table);
                if (is_array($rawCols) && !empty($rawCols)) {
                    foreach ($rawCols as $col) {
                        $cols[] = [
                            'name' => (string) ($col['name'] ?? ''),
                            'col_type' => (string) ($col['type_name'] ?? $col['type'] ?? 'mixed'),
                            'nullable' => (bool) ($col['nullable'] ?? false),
                            'default' => isset($col['default']) ? (string) $col['default'] : null,
                        ];
                    }
                    return $cols;
                }
            }
            if (method_exists('Illuminate\\Support\\Facades\\Schema', 'getColumnListing')) {
                $names = \Illuminate\Support\Facades\Schema::getColumnListing($table);
                if (is_array($names) && !empty($names)) {
                    foreach ($names as $name) {
                        $type = 'mixed';
                        try {
                            $type = (string) \Illuminate\Support\Facades\Schema::getColumnType($table, (string) $name);
                        } catch (\Throwable $ignored) {
                        }
                        $cols[] = [
                            'name' => (string) $name,
                            'col_type' => $type,
                            'nullable' => false,
                            'default' => null,
                        ];
                    }
                    return $cols;
                }
            }
        } catch (\Throwable $ignored) {
        }
    }
    return $cols;
}

function __tw_inspect_model(string $rawName): array {
    $fqcn = __tw_resolve_model_class($rawName);
    if ($fqcn === null) {
        return [
            'fqcn' => $rawName,
            'table' => '',
            'primary_key' => 'id',
            'error' => "Model class '{$rawName}' not found.",
        ];
    }

    try {
        $ref = new \ReflectionClass($fqcn);
        $instance = $ref->isInstantiable() ? $ref->newInstanceWithoutConstructor() : null;
        try {
            if ($ref->isInstantiable()) {
                $instance = new $fqcn();
            }
        } catch (\Throwable $ignored) {
        }

        $table = '';
        if ($instance !== null && method_exists($instance, 'getTable')) {
            $table = (string) $instance->getTable();
        } elseif (method_exists($fqcn, 'tableName')) {
            $table = (string) $fqcn::tableName();
        }

        $primaryKey = ($instance !== null && method_exists($instance, 'getKeyName'))
            ? (string) $instance->getKeyName()
            : 'id';

        $timestamps = ($instance !== null && method_exists($instance, 'usesTimestamps'))
            ? (bool) $instance->usesTimestamps()
            : true;

        $traits = [];
        $cur = $ref;
        while ($cur) {
            foreach ($cur->getTraitNames() as $t) {
                $traits[] = $t;
            }
            $cur = $cur->getParentClass();
        }
        $softDeletes = in_array('Illuminate\\Database\\Eloquent\\SoftDeletes', $traits, true);

        $columns = $table !== '' ? __tw_table_columns($table) : [];

        // Fallback for mock/custom driver models when DB Schema facade has no live columns
        if (empty($columns)) {
            $sampleRow = null;
            if (method_exists($fqcn, 'seedRows')) {
                $rows = $fqcn::seedRows();
                if (is_array($rows) && !empty($rows)) {
                    $sampleRow = reset($rows);
                }
            }
            if (is_array($sampleRow)) {
                foreach ($sampleRow as $k => $v) {
                    $t = is_int($v) ? 'integer' : (is_float($v) ? 'float' : (is_bool($v) ? 'boolean' : 'string'));
                    $columns[] = [
                        'name' => (string) $k,
                        'col_type' => $t,
                        'nullable' => $v === null,
                        'default' => null,
                    ];
                }
            }
        }

        $fillable = ($instance !== null && method_exists($instance, 'getFillable'))
            ? array_values(array_map('strval', (array) $instance->getFillable()))
            : [];

        $castsOut = [];
        if ($instance !== null && method_exists($instance, 'getCasts')) {
            foreach ((array) $instance->getCasts() as $col => $castType) {
                $castStr = (string) $castType;
                if (function_exists('enum_exists') && enum_exists($castStr)) {
                    $cases = [];
                    foreach ($castStr::cases() as $case) {
                        $cases[] = property_exists($case, 'value') ? (string) $case->value : $case->name;
                    }
                    if (!empty($cases)) {
                        $castStr .= ' [' . implode('|', array_slice($cases, 0, 20)) . ']';
                    }
                }
                $castsOut[] = [(string) $col, $castStr];
            }
        }

        $scopes = [];
        $relations = [];
        $fileLines = ($ref->getFileName() && is_file($ref->getFileName()))
            ? @file($ref->getFileName())
            : false;

        $relShortKinds = [
            'HasOne', 'HasMany', 'BelongsTo', 'BelongsToMany',
            'MorphTo', 'MorphOne', 'MorphMany', 'MorphToMany',
            'HasOneThrough', 'HasManyThrough',
        ];

        foreach ($ref->getMethods(\ReflectionMethod::IS_PUBLIC) as $method) {
            if ($method->getDeclaringClass()->getName() !== $fqcn) {
                continue;
            }
            $mName = $method->getName();
            if (str_starts_with($mName, 'scope') && strlen($mName) > 5) {
                $scopes[] = lcfirst(substr($mName, 5));
                continue;
            }
            if ($method->isStatic() || str_starts_with($mName, '__')) {
                continue;
            }

            $relKind = null;
            $relatedModel = null;
            $retType = $method->getReturnType();
            if ($retType instanceof \ReflectionNamedType) {
                $rtName = $retType->getName();
                foreach ($relShortKinds as $rk) {
                    if ($rtName === $rk || str_ends_with($rtName, '\\' . $rk)) {
                        $relKind = $rk;
                        break;
                    }
                }
            }

            if (is_array($fileLines) && $method->getStartLine() && $method->getEndLine()) {
                $start = max(0, $method->getStartLine() - 1);
                $len = max(1, $method->getEndLine() - $method->getStartLine() + 1);
                $body = implode('', array_slice($fileLines, $start, min($len, 30)));
                if (preg_match('/\$this->(hasOne|hasMany|belongsTo|belongsToMany|morphTo|morphOne|morphMany|morphToMany|hasOneThrough|hasManyThrough)\s*\(\s*([A-Za-z0-9_\\\\]+)::class/i', $body, $m)) {
                    $relKind = $relKind ?? ucfirst($m[1]);
                    $relatedModel = $m[2];
                } elseif ($relKind === null && preg_match('/\$this->(hasOne|hasMany|belongsTo|belongsToMany|morphTo|morphOne|morphMany)\s*\(/i', $body, $m)) {
                    $relKind = ucfirst($m[1]);
                }
            }

            if ($relKind !== null) {
                $relations[] = [
                    'name' => $mName,
                    'kind' => $relKind,
                    'related' => $relatedModel,
                ];
            }
        }

        return [
            'fqcn' => $fqcn,
            'table' => $table,
            'primary_key' => $primaryKey,
            'timestamps' => $timestamps,
            'soft_deletes' => $softDeletes,
            'columns' => $columns,
            'fillable' => $fillable,
            'casts' => $castsOut,
            'scopes' => $scopes,
            'relations' => $relations,
            'error' => null,
        ];
    } catch (\Throwable $e) {
        return [
            'fqcn' => $fqcn,
            'table' => '',
            'primary_key' => 'id',
            'error' => $e->getMessage(),
        ];
    }
}

$__tw_db_driver = 'unknown';
$__tw_timezone = date_default_timezone_get() ?: 'UTC';
if (function_exists('config')) {
    try {
        $defConn = (string) config('database.default', '');
        if ($defConn !== '') {
            $__tw_db_driver = (string) config("database.connections.{$defConn}.driver", $defConn);
        }
        $tz = (string) config('app.timezone', '');
        if ($tz !== '') {
            $__tw_timezone = $tz;
        }
    } catch (\Throwable $ignored) {
    }
}

$__tw_all_tables = [];
if (!empty($__tw_req['list_tables']) && class_exists('Illuminate\\Support\\Facades\\Schema')) {
    try {
        if (method_exists('Illuminate\\Support\\Facades\\Schema', 'getTableListing')) {
            $rawTables = \Illuminate\Support\Facades\Schema::getTableListing();
            if (is_array($rawTables)) {
                $__tw_all_tables = array_values(array_map('strval', $rawTables));
            }
        }
    } catch (\Throwable $ignored) {
    }
}

$__tw_models_out = [];
foreach ((array) ($__tw_req['models'] ?? []) as $mName) {
    if (is_string($mName) && trim($mName) !== '') {
        $__tw_models_out[] = __tw_inspect_model($mName);
    }
}

$__tw_tables_out = [];
foreach ((array) ($__tw_req['tables'] ?? []) as $tName) {
    if (is_string($tName) && trim($tName) !== '') {
        $cols = __tw_table_columns(trim($tName));
        $__tw_tables_out[] = [
            'table' => trim($tName),
            'columns' => $cols,
            'error' => empty($cols) ? "No columns found for table '{$tName}'." : null,
        ];
    }
}

$__tw_schema_payload = [
    'status' => 'ok',
    'laravel_version' => $__tw_laravel_version,
    'php_version' => PHP_VERSION,
    'db_driver' => $__tw_db_driver,
    'timezone' => $__tw_timezone,
    'all_tables' => $__tw_all_tables,
    'models' => $__tw_models_out,
    'tables' => $__tw_tables_out,
    'error' => null,
];

echo "\n__TW_SCHEMA_START__\n";
echo json_encode($__tw_schema_payload, JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE);
echo "\n__TW_SCHEMA_END__\n";
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::builtin_laravel_templates;

    #[test]
    fn executes_eloquent_overview_template() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let tpl = builtin_laravel_templates()
            .into_iter()
            .find(|t| t.id == "eloquent-overview")
            .unwrap();
        let res = evaluate_code(tpl.code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);
        assert_eq!(res.driver_name, "CustomLaravelDriver");
        assert!(res.laravel_version.contains("11.34.0"));
        assert!(res.dumps.len() >= 3);
        assert!(res.sql_queries.len() >= 2);
        assert!(res.mailable.is_some());
    }

    #[test]
    fn executes_detail_dive_object_graph_and_custom_driver_vars() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let tpl = builtin_laravel_templates()
            .into_iter()
            .find(|t| t.id == "detail-dive")
            .unwrap();
        let res = evaluate_code(tpl.code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);
        assert_eq!(res.dumps.len(), 1);
        assert!(res.return_value.is_some());
        assert!(res
            .driver_vars
            .iter()
            .any(|(k, _)| k == "$appName"));
    }

    #[test]
    fn executes_mailable_preview_template() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let tpl = builtin_laravel_templates()
            .into_iter()
            .find(|t| t.id == "mailable-preview")
            .unwrap();
        let res = evaluate_code(tpl.code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);
        let mail = res.mailable.expect("mailable preview should be detected");
        assert!(mail.subject.contains("INV-2026-0103"));
        assert!(mail.html.contains("Jess Archer"));
        assert_eq!(mail.to, vec!["jess@laravel.com"]);
    }

    #[test]
    fn executes_sql_n1_detection_template() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let tpl = builtin_laravel_templates()
            .into_iter()
            .find(|t| t.id == "sql-n1-check")
            .unwrap();
        let res = evaluate_code(tpl.code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);
        assert!(res.sql_queries.iter().any(|q| q.is_duplicate));
    }

    #[test]
    fn executes_table_report_template_with_php_and_json_export() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let tpl = builtin_laravel_templates()
            .into_iter()
            .find(|t| t.id == "table-report")
            .unwrap();
        let res = evaluate_code(tpl.code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);
        let table = res.table.expect("table dataset should be extracted");
        assert_eq!(table.rows.len(), 8);
        assert!(table.columns.contains(&"email".to_string()));
        assert!(!table.raw_rows_json[0].is_empty());
        assert!(table.raw_rows_php[0].contains("=>"));
    }

    #[test]
    fn resolves_unimported_article_model_and_facades_automatically() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let res = evaluate_code("$articles = Article::all();", &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);
        let table = res.table.expect("Article::all() should produce table rows");
        assert_eq!(table.rows.len(), 3);
    }

    #[test]
    fn formats_eloquent_collection_as_psysh_cli_output_matching_screenshot() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let res = evaluate_code("$articles = Article::all();", &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);

        let cli = crate::models::format_cli_text(&res);
        assert!(
            cli.starts_with("Illuminate\\Database\\Eloquent\\Collection {#"),
            "unexpected cli header: {cli}"
        );
        assert!(cli.contains("\n  all: [\n    App\\Models\\Article {#"));
        assert!(cli.contains("\n      id: 1,\n"));
        assert!(cli.contains("title: \"Introducing Laravel Cloud\","));
        assert!(cli.ends_with("\n  ],\n}"));
    }

    #[test]
    fn extracts_table_preview_from_any_collection_card() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let res = evaluate_code("dump(User::all());\n$articles = Article::all();", &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);

        let dump_node = &res.dumps[0].node;
        let users_tbl = crate::models::extract_table_from_node(dump_node, "Line 1")
            .expect("dump Collection card should produce TableDataset");
        assert_eq!(users_tbl.rows.len(), 8);
        assert!(users_tbl.columns.contains(&"email".to_string()));
        assert_eq!(users_tbl.rows[0][1], "Taylor Otwell");

        let ret_node = &res.return_value.as_ref().unwrap().node;
        let articles_tbl = crate::models::extract_table_from_node(ret_node, "Line 2")
            .expect("return Collection card should produce TableDataset");
        assert_eq!(articles_tbl.rows.len(), 3);
        assert!(articles_tbl.columns.contains(&"slug".to_string()));
    }

    #[test]
    fn lints_and_evaluates_php_syntax_errors_with_exact_editor_line() {
        let bad_code = "use App\\Models\\User;\n\n$link = User:first();";
        let diag = lint_php_syntax(bad_code).expect("syntax error should be caught by php -l");
        assert_eq!(diag.line, 3);
        assert!(
            diag.message
                .starts_with("PHP Parse error: Syntax error, unexpected ':'"),
            "unexpected lint message: {}",
            diag.message
        );
        assert!(diag.message.ends_with("on line 3"));
        assert_eq!(diag.token_hint.as_deref(), Some(":"));

        // Valid code produces None
        assert!(lint_php_syntax("use App\\Models\\User;\n\n$link = User::first();").is_none());

        // Full runner evaluation also maps ParseError to exact editor line 3
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let res = evaluate_code(bad_code, &target);
        assert_eq!(res.status, "error");
        let exc = res.exception.expect("exception should be populated");
        assert_eq!(exc.line, 3);
        assert!(
            exc.message
                .starts_with("PHP Parse error: Syntax error, unexpected ':'"),
            "unexpected eval parse error: {}",
            exc.message
        );
    }

    #[test]
    fn extracts_html_webview_preview_for_link_approved_and_sent_message() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };

        let code = "use App\\Models\\Link;\nuse App\\Mail\\LinkApproved;\n\n$link = Link::first();\nnew LinkApproved($link);";
        let res = evaluate_code(code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);
        let ret = res.return_value.as_ref().expect("return card should exist");
        assert_eq!(ret.node.class_name.as_deref(), Some("App\\Mail\\LinkApproved"));
        let html = ret
            .node
            .html_preview
            .as_deref()
            .expect("html_preview should be populated on Mailable card");
        assert!(html.contains("Your Link is approved"));
        assert!(html.contains("Eloquent touch()"));

        let sent_code = "use App\\Models\\Link;\nuse App\\Mail\\LinkApproved;\n\n$link = Link::first();\nMail::to('user@example.com')->send(new LinkApproved($link));";
        let sent_res = evaluate_code(sent_code, &target);
        assert_eq!(sent_res.status, "ok", "exception: {:?}", sent_res.exception);
        let sent_ret = sent_res.return_value.as_ref().unwrap();
        assert_eq!(
            sent_ret.node.class_name.as_deref(),
            Some("Illuminate\\Mail\\SentMessage")
        );
        assert!(sent_ret
            .node
            .html_preview
            .as_deref()
            .unwrap()
            .contains("Your Link is approved"));
    }

    #[test]
    fn orders_detail_dive_cards_top_to_bottom_matching_editor_lines() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let tpl = builtin_laravel_templates()
            .into_iter()
            .find(|t| t.id == "eloquent-overview")
            .unwrap();
        let res = evaluate_code(tpl.code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);

        let value_cards: Vec<(usize, Option<String>, String)> =
            crate::models::ordered_detail_items(&res, false)
                .into_iter()
                .filter_map(|item| match item {
                    crate::models::DetailStreamItem::Value { card, .. } => Some((
                        card.line,
                        card.node.class_name.clone(),
                        card.node.summary.clone(),
                    )),
                    _ => None,
                })
                .collect();

        assert_eq!(value_cards.len(), 4, "cards: {value_cards:?}");
        // Strict ascending top-to-bottom editor line order: Line 7 -> Line 13 -> Line 16 -> Line 20
        let lines: Vec<usize> = value_cards.iter().map(|(l, _, _)| *l).collect();
        assert_eq!(lines, vec![7, 13, 16, 20]);
        assert_eq!(value_cards[0].1.as_deref(), Some("App\\Models\\User"));
        assert_eq!(value_cards[1].2, "7");
        assert_eq!(value_cards[2].2, "2093");
        assert_eq!(
            value_cards[3].1.as_deref(),
            Some("App\\Mail\\InvoiceShipped")
        );
    }

    #[test]
    fn captures_multiple_standalone_model_queries_as_separate_cards() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let code = "use App\\Models\\Article;\nuse App\\Models\\User;\n\nUser::query()->limit(5)->get();\n\nArticle::query()->limit(10)->get();";
        let res = evaluate_code(code, &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);

        let cards: Vec<crate::models::DumpCard> =
            crate::models::ordered_detail_items(&res, false)
                .into_iter()
                .filter_map(|item| match item {
                    crate::models::DetailStreamItem::Value { card, .. } => Some(card),
                    _ => None,
                })
                .collect();

        assert_eq!(cards.len(), 2, "expected both User and Article cards: {cards:?}");
        assert_eq!(cards[0].line, 4);
        assert_eq!(
            cards[0].node.children[0].class_name.as_deref(),
            Some("App\\Models\\User")
        );
        assert_eq!(cards[0].node.children.len(), 5);

        assert_eq!(cards[1].line, 6);
        assert_eq!(
            cards[1].node.children[0].class_name.as_deref(),
            Some("App\\Models\\Article")
        );
        assert_eq!(cards[1].node.children.len(), 3);

        // Contiguous lines without a blank line combine into one block (only block's final expr outputs a card)
        let combined_code = "use App\\Models\\Article;\nuse App\\Models\\User;\n\n$user = User::where('email', 'taylor@laravel.com')->first();\ntrue;\n$user->email;\n\nArticle::first();";
        let combined_res = evaluate_code(combined_code, &target);
        assert_eq!(combined_res.status, "ok", "exception: {:?}", combined_res.exception);
        let combined_cards: Vec<crate::models::DumpCard> =
            crate::models::ordered_detail_items(&combined_res, false)
                .into_iter()
                .filter_map(|item| match item {
                    crate::models::DetailStreamItem::Value { card, .. } => Some(card),
                    _ => None,
                })
                .collect();
        assert_eq!(combined_cards.len(), 2, "cards: {combined_cards:?}");
        assert_eq!(combined_cards[0].line, 6);
        assert_eq!(
            combined_cards[0].node.scalar_value.as_deref(),
            Some("\"taylor@laravel.com\"")
        );
        assert_eq!(combined_cards[1].line, 8);
        assert_eq!(
            combined_cards[1].node.class_name.as_deref(),
            Some("App\\Models\\Article")
        );
    }

    #[test]
    fn introspects_eloquent_model_columns_and_relations_safely() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let req = SchemaIntrospectRequest {
            models: vec!["User".to_string(), "App\\Models\\Article".to_string()],
            tables: Vec::new(),
            list_tables: false,
        };
        let res = introspect_laravel_schema(&req, &target, None)
            .expect("schema introspection should succeed");
        assert_eq!(res.status, "ok");
        assert_eq!(res.models.len(), 2);

        let user = &res.models[0];
        assert_eq!(user.fqcn, "App\\Models\\User");
        assert_eq!(user.table, "users");
        assert!(user.columns.iter().any(|c| c.name == "email"));
        assert!(user.columns.iter().any(|c| c.name == "id"));

        let article = &res.models[1];
        assert_eq!(article.fqcn, "App\\Models\\Article");
        assert_eq!(article.table, "articles");
        assert!(article.columns.iter().any(|c| c.name == "title"));
    }

    #[test]
    fn parses_remote_docker_inspect_lines_and_normalizes_workdirs() {
        let sample = "\
/laralah-app-1\t9f8e7d6c5b4a3210fedcba9876543210\tserversideup/php:8.3-fpm-nginx\t/var/www/html\n\
/worker_queue\tabcdef12345678900000\tphp:8.4-cli-alpine\t\n";
        let parsed = parse_remote_docker_inspect_output(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "laralah-app-1");
        assert_eq!(parsed[0].id, "9f8e7d6c5b4a");
        assert_eq!(parsed[0].image, "serversideup/php:8.3-fpm-nginx");
        assert_eq!(parsed[0].workdir, "/var/www/html");

        assert_eq!(parsed[1].name, "worker_queue");
        assert_eq!(parsed[1].id, "abcdef123456");
        assert_eq!(parsed[1].image, "php:8.4-cli-alpine");
        assert_eq!(parsed[1].workdir, "/var/www/html");
    }
}

#[cfg(test)]
mod askpass_tests {
    use super::*;

    #[test]
    fn ssh_command_uses_own_binary_as_askpass_without_temp_files() {
        let cmd = build_ssh_command_with_askpass(
            "example.com",
            22,
            "forge",
            "php",
            crate::ssh::SshAuthMethod::Password,
            "",
            "s3cr'et $x",
        )
        .unwrap();
        let envs: std::collections::HashMap<_, _> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_string_lossy().to_string(), v?.to_owned())))
            .collect();
        assert_eq!(envs["SSH_ASKPASS"], std::env::current_exe().unwrap().into_os_string());
        assert_eq!(envs[ASKPASS_MODE_ENV], "1");
        assert_eq!(envs[ASKPASS_SECRET_ENV], "s3cr'et $x");
        assert!(!cmd.get_args().any(|a| a.to_string_lossy().contains("s3cr")));
    }

    #[test]
    fn ssh_command_rejects_injected_host() {
        let err = build_ssh_command_with_askpass(
            "-oProxyCommand=touch /tmp/pwned",
            22,
            "forge",
            "php",
            crate::ssh::SshAuthMethod::SshAgent,
            "",
            "",
        );
        assert!(err.is_err());
    }

    #[test]
    fn local_docker_connection_runs_docker_exec_without_ssh() {
        let mut p = crate::ssh::SshConnectionProfile::new_empty();
        p.kind = crate::ssh::ConnectionKind::LocalDocker;
        p.docker_container = "my-app".to_string();
        p.application_folder = "/app".to_string();
        let cmd = build_target_php_command(&p.to_execution_target(), "php").unwrap();
        let program = std::path::Path::new(cmd.get_program());
        let stem = program
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        assert_eq!(stem, "docker", "unexpected program: {}", program.display());
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, ["exec", "-i", "-w", "/app", "my-app", "php"]);
    }

    #[test]
    #[ignore = "needs a running local Docker container in LARALAH_E2E_CONTAINER"]
    fn local_docker_e2e() {
        let container = std::env::var("LARALAH_E2E_CONTAINER").unwrap();
        let found = detect_local_docker_containers().expect("fetch containers");
        eprintln!("docker={} found={found:?}", docker_binary());
        assert!(found.iter().any(|c| container.starts_with(&c.id) || c.name == container));
        let mut p = crate::ssh::SshConnectionProfile::new_empty();
        p.kind = crate::ssh::ConnectionKind::LocalDocker;
        p.docker_container = container;
        p.application_folder = "/var/www/html".to_string();
        let res = evaluate_code("return 1 + 2;", &p.to_execution_target());
        eprintln!("result={res:?}");
        assert!(format!("{res:?}").contains('3'));
    }
}
