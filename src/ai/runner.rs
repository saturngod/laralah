use crate::ai::CliProviderId;
use crate::ai::discovery::{cli_child_path_env, cli_command, find_cli_binary, kill_cli, qoder_base_args};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const POLL_INTERVAL: Duration = Duration::from_millis(120);
const MAX_OUTPUT_BYTES: usize = 1_000_000;
const MAX_LINE_BYTES: usize = 4_000_000;
const SYSTEM_PROMPT_FILE: &str = "laralah-system-prompt.md";

struct TempWorkDir {
    path: PathBuf,
}

impl TempWorkDir {
    fn create() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("laralah-ai-{pid}-{nanos}"));
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("Could not create temporary AI workspace: {e}"))?;
        Ok(Self { path })
    }
}

impl Drop for TempWorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct ManagedProcess(Child);

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        kill_cli(&mut self.0);
        let _ = self.0.wait();
    }
}

pub fn run_cli_turn(
    provider: CliProviderId,
    model: &str,
    effort: Option<&str>,
    instructions: &str,
    prompt: &str,
    cancel: &AtomicBool,
    timeout: Duration,
    mut on_delta: impl FnMut(&str),
) -> Result<String, String> {
    let binary = find_cli_binary(provider).ok_or_else(|| {
        format!(
            "{} CLI (`{}`) was not found. Install it or check Settings → AI Assistant.",
            provider.label(),
            provider.binaries().join(" / ")
        )
    })?;

    let work_dir = TempWorkDir::create()?;
    let effort_clean = effort
        .map(str::trim)
        .filter(|e| !e.is_empty() && !e.eq_ignore_ascii_case("default"));

    match provider {
        CliProviderId::Codex => run_codex_turn(
            &binary,
            model,
            effort_clean,
            instructions,
            prompt,
            &work_dir.path,
            cancel,
            timeout,
            &mut on_delta,
        ),
        CliProviderId::Opencode => run_opencode_turn(
            &binary,
            model,
            effort_clean,
            instructions,
            prompt,
            &work_dir.path,
            cancel,
            timeout,
            &mut on_delta,
        ),
        CliProviderId::ClaudeCode | CliProviderId::Qoder | CliProviderId::Agy => {
            let mut args = build_stdin_cli_args(provider, model, effort_clean);
            write_cli_instructions(provider, &mut args, instructions, &work_dir.path)?;
            let stdin_bytes = build_stdin_payload(provider, prompt)?;
            run_streaming_stdin_provider(
                provider,
                &binary,
                &args,
                Some(&stdin_bytes),
                &work_dir.path,
                cancel,
                timeout,
                &mut on_delta,
            )
        }
    }
}

fn build_stdin_cli_args(
    provider: CliProviderId,
    model: &str,
    effort: Option<&str>,
) -> Vec<String> {
    let mut args = match provider {
        CliProviderId::ClaudeCode => vec![
            "-p".to_string(),
            "--model".to_string(),
            model.to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--include-partial-messages".to_string(),
            "--max-turns".to_string(),
            "4".to_string(),
        ],
        CliProviderId::Qoder => {
            let mut a = qoder_base_args();
            a.extend([
                "--model".to_string(),
                model.to_string(),
                "--include-partial-messages".to_string(),
                "--permission-mode".to_string(),
                "dont_ask".to_string(),
            ]);
            a
        }
        CliProviderId::Agy => vec![
            "--print=".to_string(),
            "--input-format".to_string(),
            "stream-json".to_string(),
            "--model".to_string(),
            model.to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--print-timeout".to_string(),
            "15m".to_string(),
        ],
        _ => Vec::new(),
    };

    if let Some(eff) = effort {
        let eff_lower = eff.to_ascii_lowercase();
        let flags = match provider {
            CliProviderId::Qoder => vec!["--reasoning-effort".to_string(), eff_lower],
            CliProviderId::ClaudeCode | CliProviderId::Agy => {
                vec!["--effort".to_string(), eff_lower]
            }
            _ => Vec::new(),
        };
        if !flags.is_empty() {
            args.splice(0..0, flags);
        }
    }
    args
}

fn write_cli_instructions(
    provider: CliProviderId,
    args: &mut Vec<String>,
    instructions: &str,
    work_dir: &Path,
) -> Result<(), String> {
    match provider {
        CliProviderId::ClaudeCode | CliProviderId::Qoder => {
            std::fs::write(work_dir.join(SYSTEM_PROMPT_FILE), instructions)
                .map_err(|e| format!("Failed writing system prompt file: {e}"))?;
            args.extend([
                "--system-prompt-file".to_string(),
                SYSTEM_PROMPT_FILE.to_string(),
            ]);
        }
        CliProviderId::Agy => {
            let agents_dir = work_dir.join(".agents").join("agents");
            std::fs::create_dir_all(&agents_dir)
                .map_err(|e| format!("Failed creating agy agent folder: {e}"))?;
            let content = format!(
                "---\nname: laralah-assistant\ndescription: Generate Laravel Eloquent & PHP code for Laralah.\nmainAgent: true\nsubagent: false\ntools: []\nskills: []\nplugins: []\nmcpServers: []\n---\n\n# System Prompt\n{instructions}\n"
            );
            std::fs::write(agents_dir.join("laralah-assistant.md"), content)
                .map_err(|e| format!("Failed writing agy agent definition: {e}"))?;
            args.extend(["--agent".to_string(), "laralah-assistant".to_string()]);
        }
        _ => {}
    }
    Ok(())
}

fn build_stdin_payload(provider: CliProviderId, prompt: &str) -> Result<Vec<u8>, String> {
    match provider {
        CliProviderId::ClaudeCode | CliProviderId::Qoder => Ok(prompt.as_bytes().to_vec()),
        CliProviderId::Agy => {
            let msg = json!({
                "event": "user",
                "message": { "role": "user", "content": prompt }
            });
            let mut bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
            bytes.push(b'\n');
            Ok(bytes)
        }
        _ => Ok(Vec::new()),
    }
}

struct RawStreamResult {
    status: ExitStatus,
    stderr: Vec<u8>,
}

fn execute_process_streaming(
    binary: &Path,
    args: &[String],
    stdin: Option<&[u8]>,
    work_dir: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    mut on_chunk: impl FnMut(&[u8]),
) -> Result<RawStreamResult, String> {
    let mut command = cli_command(binary);
    command
        .args(args)
        .current_dir(work_dir)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = cli_child_path_env(binary) {
        command.env("PATH", path);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to start CLI tool: {e}"))?;

    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let owned = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&owned);
        });
    }

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        if let Some(mut pipe) = stdout_pipe {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if out_tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    let (err_tx, err_rx) = mpsc::sync_channel::<Vec<u8>>(16);
    std::thread::spawn(move || {
        if let Some(mut pipe) = stderr_pipe {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if err_tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut total_bytes = 0usize;
    let mut stderr = Vec::new();
    let mut stdout_closed = false;

    loop {
        while let Ok(chunk) = err_rx.try_recv() {
            stderr.extend_from_slice(&chunk);
            if stderr.len() > 64 * 1024 {
                stderr.drain(..stderr.len() - 64 * 1024);
            }
        }

        if !stdout_closed {
            match out_rx.recv_timeout(POLL_INTERVAL) {
                Ok(first) => {
                    for chunk in std::iter::once(first).chain(out_rx.try_iter()) {
                        total_bytes += chunk.len();
                        if total_bytes > MAX_OUTPUT_BYTES {
                            kill_cli(&mut child);
                            let _ = child.wait();
                            return Err("AI response exceeded maximum output limit.".to_string());
                        }
                        on_chunk(&chunk);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    stdout_closed = true;
                }
            }
        }

        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => {
                kill_cli(&mut child);
                let _ = child.wait();
                return Err("CLI process encountered an execution error.".to_string());
            }
        }

        if cancel.load(Ordering::SeqCst) {
            kill_cli(&mut child);
            let _ = child.wait();
            return Err("__CANCELLED__".to_string());
        }

        if Instant::now() >= deadline {
            kill_cli(&mut child);
            let _ = child.wait();
            return Err("AI agent timed out waiting for response.".to_string());
        }

        if stdout_closed {
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    while let Ok(chunk) = out_rx.recv_timeout(Duration::from_secs(1)) {
        total_bytes += chunk.len();
        if total_bytes <= MAX_OUTPUT_BYTES {
            on_chunk(&chunk);
        }
    }
    while let Ok(chunk) = err_rx.try_recv() {
        stderr.extend_from_slice(&chunk);
    }

    let status = child
        .wait()
        .map_err(|_| "Failed waiting for CLI exit status.".to_string())?;

    Ok(RawStreamResult { status, stderr })
}

fn run_streaming_stdin_provider(
    provider: CliProviderId,
    binary: &Path,
    args: &[String],
    stdin: Option<&[u8]>,
    work_dir: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    on_delta: &mut impl FnMut(&str),
) -> Result<String, String> {
    if provider == CliProviderId::Agy {
        let mut decoder = AgyStreamDecoder::default();
        let res = execute_process_streaming(binary, args, stdin, work_dir, cancel, timeout, |chunk| {
            decoder.push(chunk, on_delta);
        })?;
        if !res.status.success() {
            let tail = String::from_utf8_lossy(&res.stderr).trim().to_string();
            return Err(if tail.is_empty() {
                "Antigravity exited with an error. Check `agy` authentication.".to_string()
            } else {
                format!("Antigravity error: {tail}")
            });
        }
        return decoder.finish();
    }

    let mut decoder = JsonLineStreamDecoder::new(provider);
    let res = execute_process_streaming(binary, args, stdin, work_dir, cancel, timeout, |chunk| {
        decoder.push(chunk, on_delta);
    })?;
    if !res.status.success() && decoder.final_response.is_none() && decoder.text.trim().is_empty() {
        let tail = String::from_utf8_lossy(&res.stderr).trim().to_string();
        if tail.contains("node: No such file or directory") {
            return Err("Node.js runtime not found on PATH for CLI shim.".to_string());
        }
        return Err(if tail.is_empty() {
            format!("{} exited with an error.", provider.label())
        } else {
            format!("{} error: {}", provider.label(), tail)
        });
    }
    decoder.finish(on_delta)
}

#[derive(Default)]
struct AgyStreamDecoder {
    pending: Vec<u8>,
    streamed: String,
    response: Option<String>,
    failed: bool,
}

impl AgyStreamDecoder {
    fn push(&mut self, chunk: &[u8], on_delta: &mut impl FnMut(&str)) {
        self.pending.extend_from_slice(chunk);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            self.handle_line(&line, on_delta);
        }
    }

    fn handle_line(&mut self, line: &[u8], on_delta: &mut impl FnMut(&str)) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        match v["event"].as_str() {
            Some("step_update") if v["step_update"]["step_type"] == "agent_response" => {
                if let Some(delta) = v["step_update"]["text_delta"].as_str() {
                    self.streamed.push_str(delta);
                    on_delta(delta);
                }
            }
            Some("result") => {
                if v["result"]["status"] != "SUCCESS" {
                    self.failed = true;
                }
                self.response = v["result"]["response"].as_str().map(str::to_owned);
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Result<String, String> {
        let rem = std::mem::take(&mut self.pending);
        self.handle_line(&rem, &mut |_| {});
        if self.failed {
            return Err("Antigravity reported an unsuccessful run.".to_string());
        }
        let out = self.response.unwrap_or(self.streamed);
        if out.trim().is_empty() {
            return Err("Antigravity returned an empty response.".to_string());
        }
        Ok(out)
    }
}

struct JsonLineStreamDecoder {
    provider: CliProviderId,
    pending: Vec<u8>,
    text: String,
    final_response: Option<String>,
    failed: bool,
    failure_msg: Option<String>,
    snapshots: HashMap<String, String>,
}

impl JsonLineStreamDecoder {
    fn new(provider: CliProviderId) -> Self {
        Self {
            provider,
            pending: Vec::new(),
            text: String::new(),
            final_response: None,
            failed: false,
            failure_msg: None,
            snapshots: HashMap::new(),
        }
    }

    fn push(&mut self, chunk: &[u8], on_delta: &mut impl FnMut(&str)) {
        self.pending.extend_from_slice(chunk);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            self.handle_line(&line, on_delta);
        }
    }

    fn handle_line(&mut self, line: &[u8], on_delta: &mut impl FnMut(&str)) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let kind = v["type"].as_str().unwrap_or("");
        match self.provider {
            CliProviderId::ClaudeCode | CliProviderId::Qoder => match kind {
                "stream_event" if v["parent_tool_use_id"].is_null() => {
                    let ev = &v["event"];
                    if ev["type"] == "content_block_delta"
                        && ev["delta"]["type"] == "text_delta"
                        && let Some(delta) = ev["delta"]["text"].as_str()
                    {
                        self.text.push_str(delta);
                        on_delta(delta);
                    }
                }
                "assistant" if v["parent_tool_use_id"].is_null() && self.text.is_empty() => {
                    if let Some(blocks) = v["message"]["content"].as_array() {
                        let combined: String = blocks
                            .iter()
                            .filter(|b| b["type"] == "text")
                            .filter_map(|b| b["text"].as_str())
                            .collect();
                        if !combined.is_empty() {
                            self.text.push_str(&combined);
                            on_delta(&combined);
                        }
                    }
                }
                "result" => {
                    if self.provider == CliProviderId::Qoder && v["error_code"] == 110 {
                        self.failure_msg = Some("Qoder daily usage limit reached.".to_string());
                    }
                    if v["is_error"].as_bool() == Some(true) {
                        self.failed = true;
                        if let Some(err_txt) = v["result"].as_str() {
                            self.failure_msg = Some(err_txt.to_string());
                        }
                    } else if let Some(res_txt) = v["result"].as_str() {
                        self.final_response = Some(res_txt.to_string());
                    }
                }
                "error" => {
                    self.failed = true;
                }
                _ => {}
            },
            CliProviderId::Codex => match kind {
                "item.started" | "item.updated" | "item.completed"
                    if v["item"]["type"] == "agent_message" =>
                {
                    if let Some(full) = v["item"]["text"].as_str() {
                        let key = v["item"]["id"].as_str().unwrap_or("msg").to_string();
                        let prev = self.snapshots.get(&key).map(String::as_str).unwrap_or("");
                        if let Some(delta) = full.strip_prefix(prev)
                            && !delta.is_empty()
                        {
                            self.text.push_str(delta);
                            on_delta(delta);
                        }
                        self.snapshots.insert(key, full.to_string());
                        if kind == "item.completed" {
                            self.final_response = Some(full.to_string());
                        }
                    }
                }
                "turn.failed" | "error" => self.failed = true,
                _ => {}
            },
            CliProviderId::Opencode => match kind {
                "text" => {
                    if let Some(full) = v["part"]["text"].as_str() {
                        let key = v["part"]["id"].as_str().unwrap_or("text").to_string();
                        let prev = self.snapshots.get(&key).map(String::as_str).unwrap_or("");
                        if let Some(delta) = full.strip_prefix(prev)
                            && !delta.is_empty()
                        {
                            self.text.push_str(delta);
                            on_delta(delta);
                        }
                        self.snapshots.insert(key, full.to_string());
                    }
                }
                "error" => self.failed = true,
                _ => {}
            },
            CliProviderId::Agy => {}
        }
    }

    fn finish(mut self, on_delta: &mut impl FnMut(&str)) -> Result<String, String> {
        let rem = std::mem::take(&mut self.pending);
        self.handle_line(&rem, on_delta);
        if self.failed {
            return Err(self
                .failure_msg
                .unwrap_or_else(|| format!("{} reported an error.", self.provider.label())));
        }
        let out = self.final_response.unwrap_or(self.text);
        if out.trim().is_empty() {
            return Err(format!(
                "{} returned an empty response. Check login & model selection.",
                self.provider.label()
            ));
        }
        Ok(out)
    }
}

// ==================== CODEX APP SERVER ADAPTER ====================

#[allow(clippy::too_many_arguments)]
fn run_codex_turn(
    binary: &Path,
    model: &str,
    effort: Option<&str>,
    instructions: &str,
    prompt: &str,
    work_dir: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    on_delta: &mut impl FnMut(&str),
) -> Result<String, String> {
    match run_codex_app_server(
        binary,
        model,
        effort,
        instructions,
        prompt,
        work_dir,
        cancel,
        timeout,
        on_delta,
    ) {
        Ok(out) => Ok(out),
        Err(err) if err == "__CANCELLED__" => Err(err),
        Err(_) => {
            // Fallback to `codex exec --json` for older Codex CLI builds
            let combined_prompt = format!(
                "<system_instructions>\n{instructions}\n</system_instructions>\n\n{prompt}"
            );
            let mut args = vec![
                "exec".to_string(),
                "--skip-git-repo-check".to_string(),
                "--json".to_string(),
                "--model".to_string(),
                model.to_string(),
            ];
            if let Some(eff) = effort {
                args.push("--config".to_string());
                args.push(format!("model_reasoning_effort=\"{eff}\""));
            }
            args.push(combined_prompt);
            let mut dec = JsonLineStreamDecoder::new(CliProviderId::Codex);
            let res =
                execute_process_streaming(binary, &args, None, work_dir, cancel, timeout, |c| {
                    dec.push(c, on_delta);
                })?;
            if !res.status.success() && dec.final_response.is_none() && dec.text.trim().is_empty() {
                let tail = String::from_utf8_lossy(&res.stderr).trim().to_string();
                return Err(format!("Codex CLI failed: {tail}"));
            }
            dec.finish(on_delta)
        }
    }
}

fn send_jsonrpc(pipe: &mut ChildStdin, msg: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *pipe, msg).map_err(|e| e.to_string())?;
    pipe.write_all(b"\n")
        .and_then(|_| pipe.flush())
        .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn run_codex_app_server(
    binary: &Path,
    model: &str,
    effort: Option<&str>,
    instructions: &str,
    prompt: &str,
    cwd: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    on_delta: &mut impl FnMut(&str),
) -> Result<String, String> {
    let mut command = cli_command(binary);
    command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = cli_child_path_env(binary) {
        command.env("PATH", path);
    }

    let mut proc = ManagedProcess(
        command
            .spawn()
            .map_err(|e| format!("Could not start Codex App Server: {e}"))?,
    );
    let mut stdin = proc.0.stdin.take().ok_or("Codex stdin unavailable")?;
    let mut stdout = proc.0.stdout.take().ok_or("Codex stdout unavailable")?;
    let stderr = proc.0.stderr.take();

    std::thread::spawn(move || {
        if let Some(mut pipe) = stderr {
            let mut b = [0u8; 4096];
            while matches!(pipe.read(&mut b), Ok(n) if n > 0) {}
        }
    });

    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(32);
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    send_jsonrpc(
        &mut stdin,
        &json!({
            "id": 0,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "laralah",
                    "title": "Laralah",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
    )?;

    let started = Instant::now();
    let mut pending = Vec::new();
    let mut stage: u8 = 0;
    let mut thread_id: Option<String> = None;
    let mut turn_id: Option<String> = None;
    let mut text_by_item: HashMap<String, String> = HashMap::new();
    let mut phases: HashMap<String, String> = HashMap::new();
    let mut final_text: Option<String> = None;
    let mut done = false;

    while !done {
        if cancel.load(Ordering::SeqCst) {
            return Err("__CANCELLED__".to_string());
        }
        if started.elapsed() >= timeout {
            return Err("Codex timed out waiting for response.".to_string());
        }

        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                pending.extend_from_slice(&chunk);
                while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                    if end > MAX_LINE_BYTES {
                        return Err("Codex output line exceeded size limit.".to_string());
                    }
                    let line: Vec<u8> = pending.drain(..=end).collect();
                    if line.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    let v: Value =
                        serde_json::from_slice(&line).map_err(|_| "Invalid Codex JSON-RPC".to_string())?;

                    if let Some(method) = v["method"].as_str() {
                        if let Some(req_id) = v.get("id") {
                            let decision = match method {
                                "item/commandExecution/requestApproval"
                                | "item/fileChange/requestApproval" => {
                                    json!({ "decision": "decline" })
                                }
                                "item/permissions/requestApproval" => {
                                    json!({ "permissions": {}, "scope": "turn" })
                                }
                                "mcpServer/elicitation/request" => {
                                    json!({ "action": "decline", "content": null })
                                }
                                _ => json!({ "decision": "decline" }),
                            };
                            send_jsonrpc(&mut stdin, &json!({ "id": req_id, "result": decision }))?;
                            continue;
                        }

                        let p = &v["params"];
                        let belongs = p["threadId"].as_str() == thread_id.as_deref()
                            && p["turnId"].as_str() == turn_id.as_deref()
                            && turn_id.is_some();

                        match method {
                            "turn/started" if p["threadId"].as_str() == thread_id.as_deref() => {
                                turn_id = p["turn"]["id"].as_str().map(str::to_owned);
                            }
                            "item/started" if belongs && p["item"]["type"] == "agentMessage" => {
                                if let (Some(id), Some(ph)) =
                                    (p["item"]["id"].as_str(), p["item"]["phase"].as_str())
                                {
                                    phases.insert(id.to_string(), ph.to_string());
                                }
                            }
                            "item/agentMessage/delta" if belongs => {
                                if let (Some(id), Some(delta)) =
                                    (p["itemId"].as_str(), p["delta"].as_str())
                                {
                                    text_by_item
                                        .entry(id.to_string())
                                        .or_default()
                                        .push_str(delta);
                                    if phases.get(id).is_none_or(|ph| ph == "final_answer") {
                                        on_delta(delta);
                                    }
                                }
                            }
                            "item/completed" if belongs && p["item"]["type"] == "agentMessage" => {
                                let item = &p["item"];
                                if let (Some(id), Some(txt)) =
                                    (item["id"].as_str(), item["text"].as_str())
                                    && item["phase"]
                                        .as_str()
                                        .or_else(|| phases.get(id).map(String::as_str))
                                        .is_none_or(|ph| ph == "final_answer")
                                {
                                    let prev =
                                        text_by_item.get(id).map(String::as_str).unwrap_or("");
                                    if let Some(delta) = txt.strip_prefix(prev)
                                        && !delta.is_empty()
                                    {
                                        on_delta(delta);
                                    }
                                    text_by_item.insert(id.to_string(), txt.to_string());
                                    final_text = Some(txt.to_string());
                                }
                            }
                            "turn/completed"
                                if p["threadId"].as_str() == thread_id.as_deref()
                                    && p["turn"]["id"].as_str() == turn_id.as_deref() =>
                            {
                                if p["turn"]["status"] != "completed" {
                                    return Err("Codex turn did not complete.".to_string());
                                }
                                done = true;
                            }
                            "error" if belongs && p["willRetry"] != true => {
                                return Err("Codex reported a turn error.".to_string());
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if v.get("error").is_some() {
                        return Err("Codex App Server returned an RPC error.".to_string());
                    }

                    match (stage, v["id"].as_u64()) {
                        (0, Some(0)) => {
                            stage = 1;
                            send_jsonrpc(&mut stdin, &json!({ "method": "initialized", "params": {} }))?;
                            send_jsonrpc(
                                &mut stdin,
                                &json!({
                                    "id": 1,
                                    "method": "thread/start",
                                    "params": {
                                        "model": model,
                                        "cwd": cwd,
                                        "ephemeral": true,
                                        "approvalPolicy": "never",
                                        "sandbox": "read-only",
                                        "developerInstructions": instructions
                                    }
                                }),
                            )?;
                        }
                        (1, Some(1)) => {
                            let tid = v["result"]["thread"]["id"]
                                .as_str()
                                .ok_or("Missing Codex thread id")?
                                .to_string();
                            thread_id = Some(tid.clone());
                            stage = 2;
                            let mut params = json!({
                                "threadId": tid,
                                "model": model,
                                "cwd": cwd,
                                "input": [{ "type": "text", "text": prompt }]
                            });
                            if let Some(eff) = effort {
                                params["effort"] = json!(eff.to_ascii_lowercase());
                            }
                            send_jsonrpc(
                                &mut stdin,
                                &json!({
                                    "id": 2,
                                    "method": "turn/start",
                                    "params": params
                                }),
                            )?;
                        }
                        (2, Some(2)) => {
                            if let Some(t) = v["result"]["turn"]["id"].as_str() {
                                turn_id = Some(t.to_string());
                            }
                            stage = 3;
                        }
                        _ => {}
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Codex App Server disconnected unexpectedly.".to_string());
            }
        }
    }

    drop(stdin);
    let out = final_text
        .or_else(|| text_by_item.into_values().next())
        .unwrap_or_default();
    if out.trim().is_empty() {
        Err("Codex returned an empty answer.".to_string())
    } else {
        Ok(out)
    }
}

// ==================== OPENCODE ADAPTER (LOOPBACK HTTP/SSE + CLI FALLBACK) ====================

#[allow(clippy::too_many_arguments)]
fn run_opencode_turn(
    binary: &Path,
    model: &str,
    effort: Option<&str>,
    instructions: &str,
    prompt: &str,
    work_dir: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    on_delta: &mut impl FnMut(&str),
) -> Result<String, String> {
    match run_opencode_server_turn(
        binary,
        model,
        effort,
        instructions,
        prompt,
        work_dir,
        cancel,
        timeout,
        on_delta,
    ) {
        Ok(out) => Ok(out),
        Err(err) if err == "__CANCELLED__" => Err(err),
        Err(_) => {
            let combined_prompt = format!(
                "<system_instructions>\n{instructions}\n</system_instructions>\n\n{prompt}"
            );
            let mut args = vec![
                "run".to_string(),
                "--model".to_string(),
                model.to_string(),
                "--format".to_string(),
                "json".to_string(),
            ];
            if let Some(eff) = effort {
                args.push("--variant".to_string());
                args.push(eff.to_string());
            }
            args.push(combined_prompt);

            let mut dec = JsonLineStreamDecoder::new(CliProviderId::Opencode);
            let res =
                execute_process_streaming(binary, &args, None, work_dir, cancel, timeout, |c| {
                    dec.push(c, on_delta);
                })?;
            if !res.status.success() && dec.text.trim().is_empty() {
                let tail = String::from_utf8_lossy(&res.stderr).trim().to_string();
                return Err(format!("OpenCode failed: {tail}"));
            }
            dec.finish(on_delta)
        }
    }
}

fn encode_basic_auth(user: &str, pass: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let raw = format!("{user}:{pass}");
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    format!("Basic {out}")
}

fn local_http_json(
    port: u16,
    auth_header: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Value), String> {
    let mut stream = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(5),
    )
    .map_err(|e| format!("OpenCode connect error: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let body_bytes = body
        .map(|b| serde_json::to_vec(b).unwrap_or_default())
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth_header}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    );
    stream
        .write_all(req.as_bytes())
        .and_then(|_| stream.write_all(&body_bytes))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("OpenCode write error: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|e| e.to_string())?;
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    loop {
        let mut hdr = String::new();
        if reader.read_line(&mut hdr).unwrap_or(0) == 0 || hdr == "\r\n" || hdr == "\n" {
            break;
        }
    }

    let mut resp_bytes = Vec::new();
    let _ = reader.take(2 * 1024 * 1024).read_to_end(&mut resp_bytes);
    let val = serde_json::from_slice::<Value>(&resp_bytes).unwrap_or(Value::Null);
    Ok((status_code, val))
}

fn parse_listening_port(output: &str) -> Option<u16> {
    let suffix = output.split("http://127.0.0.1:").nth(1)?;
    let digits: String = suffix.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || suffix.len() == digits.len() {
        return None;
    }
    digits.parse::<u16>().ok().filter(|p| *p > 0)
}

#[allow(clippy::too_many_arguments)]
fn run_opencode_server_turn(
    binary: &Path,
    selected_model: &str,
    effort: Option<&str>,
    instructions: &str,
    prompt: &str,
    cwd: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
    on_delta: &mut impl FnMut(&str),
) -> Result<String, String> {
    let (provider_id, model_id) = selected_model
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
        .ok_or_else(|| "OpenCode model must be in provider/model format.".to_string())?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    let password = format!("laralah-{nanos}");
    let auth_header = encode_basic_auth("opencode", &password);

    let mut command = cli_command(binary);
    command
        .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
        .current_dir(cwd)
        .env("OPENCODE_CONFIG_CONTENT", r#"{"share":"disabled"}"#)
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = cli_child_path_env(binary) {
        command.env("PATH", path);
    }

    let mut proc = ManagedProcess(
        command
            .spawn()
            .map_err(|e| format!("Failed starting OpenCode server: {e}"))?,
    );
    let mut stdout = proc.0.stdout.take().ok_or("OpenCode stdout unavailable")?;
    let stderr = proc.0.stderr.take();
    std::thread::spawn(move || {
        if let Some(mut p) = stderr {
            let mut b = [0u8; 4096];
            while matches!(p.read(&mut b), Ok(n) if n > 0) {}
        }
    });

    let (port_tx, port_rx) = mpsc::sync_channel::<Vec<u8>>(16);
    std::thread::spawn(move || {
        let mut b = [0u8; 4096];
        loop {
            match stdout.read(&mut b) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if port_tx.send(b[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let started = Instant::now();
    let mut boot_out = Vec::new();
    let port = loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("__CANCELLED__".to_string());
        }
        if started.elapsed() > Duration::from_secs(20) {
            return Err("OpenCode server startup timed out.".to_string());
        }
        match port_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                boot_out.extend_from_slice(&chunk);
                if let Some(p) = parse_listening_port(&String::from_utf8_lossy(&boot_out)) {
                    break p;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err("OpenCode server exited before listening.".to_string()),
        }
    };
    std::thread::spawn(move || while port_rx.recv().is_ok() {});

    let mut model_obj = json!({ "id": model_id, "providerID": provider_id });
    if let Some(eff) = effort {
        model_obj["variant"] = json!(eff);
    }

    let (st_sess, sess_body) = local_http_json(
        port,
        &auth_header,
        "POST",
        "/api/session",
        Some(&json!({
            "title": "Laralah temporary session",
            "model": model_obj,
            "permissions": [{ "action": "*", "resource": "*", "effect": "ask" }]
        })),
    )?;
    if !(200..300).contains(&st_sess) {
        return Err("OpenCode rejected session creation.".to_string());
    }
    let session_id = sess_body["data"]["id"]
        .as_str()
        .ok_or("Missing OpenCode session ID")?
        .to_string();

    let mut chunk_buf = String::new();
    let mut chunks = Vec::new();
    for ch in instructions.chars() {
        chunk_buf.push(ch);
        if chunk_buf.chars().count() == 1000 {
            chunks.push(std::mem::take(&mut chunk_buf));
        }
    }
    if !chunk_buf.is_empty() {
        chunks.push(chunk_buf);
    }
    for (idx, part) in chunks.iter().enumerate() {
        let (st_i, _) = local_http_json(
            port,
            &auth_header,
            "PUT",
            &format!("/api/experimental/session/{session_id}/instructions/entries/laralah-{idx:06}"),
            Some(&json!({ "value": { "instructions": part } })),
        )?;
        if !(200..300).contains(&st_i) {
            return Err("OpenCode instructions endpoint unavailable.".to_string());
        }
    }

    let sse_auth = auth_header.clone();
    let (sse_tx, sse_rx) = mpsc::sync_channel::<Result<Vec<u8>, ()>>(32);
    std::thread::spawn(move || {
        let Ok(mut s) = TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_secs(5),
        ) else {
            let _ = sse_tx.send(Err(()));
            return;
        };
        let req = format!(
            "GET /api/event HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {sse_auth}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
        );
        if s.write_all(req.as_bytes()).is_err() {
            let _ = sse_tx.send(Err(()));
            return;
        }
        let mut reader = BufReader::new(s);
        loop {
            let mut hdr = String::new();
            if reader.read_line(&mut hdr).unwrap_or(0) == 0 || hdr == "\r\n" || hdr == "\n" {
                break;
            }
        }
        let _ = sse_tx.send(Ok(Vec::new()));
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if sse_tx.send(Ok(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sse_tx.send(Err(()));
    });

    // Wait for SSE stream ready
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("__CANCELLED__".to_string());
        }
        match sse_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(_)) => break,
            _ => return Err("Could not open OpenCode SSE stream.".to_string()),
        }
    }

    let (st_p, _) = local_http_json(
        port,
        &auth_header,
        "POST",
        &format!("/api/session/{session_id}/prompt"),
        Some(&json!({ "text": prompt })),
    )?;
    if !(200..300).contains(&st_p) {
        return Err("OpenCode rejected prompt.".to_string());
    }

    let mut sse_pending = Vec::new();
    let mut sse_data = Vec::new();
    let mut assistant_msg_id: Option<String> = None;
    let mut blocks: HashMap<u64, (String, usize)> = HashMap::new();
    let mut order: Vec<u64> = Vec::new();
    let mut done = false;

    while !done {
        if cancel.load(Ordering::SeqCst) {
            let _ = local_http_json(
                port,
                &auth_header,
                "DELETE",
                &format!("/api/session/{session_id}"),
                None,
            );
            return Err("__CANCELLED__".to_string());
        }
        if started.elapsed() >= timeout {
            return Err("OpenCode timed out.".to_string());
        }
        match sse_rx.recv_timeout(Duration::from_millis(60)) {
            Ok(Ok(chunk)) => {
                sse_pending.extend_from_slice(&chunk);
                while let Some(end) = sse_pending.iter().position(|b| *b == b'\n') {
                    let raw_line: Vec<u8> = sse_pending.drain(..=end).collect();
                    let line = raw_line
                        .strip_suffix(b"\n")
                        .unwrap_or(&raw_line)
                        .strip_suffix(b"\r")
                        .unwrap_or(&raw_line);
                    if line.is_empty() {
                        if !sse_data.is_empty() {
                            if let Ok(v) = serde_json::from_slice::<Value>(&sse_data) {
                                let kind = v["type"].as_str().unwrap_or("");
                                let data = &v["data"];
                                if data["sessionID"].as_str() == Some(session_id.as_str()) {
                                    match kind {
                                        "session.step.started" => {
                                            assistant_msg_id = data["assistantMessageID"]
                                                .as_str()
                                                .map(str::to_string);
                                        }
                                        "session.text.delta" => {
                                            if data["assistantMessageID"].as_str()
                                                == assistant_msg_id.as_deref()
                                                && let (Some(ord), Some(d)) = (
                                                    data["ordinal"].as_u64(),
                                                    data["delta"].as_str(),
                                                )
                                            {
                                                if !blocks.contains_key(&ord) {
                                                    order.push(ord);
                                                    blocks.insert(ord, (String::new(), 0));
                                                }
                                                if let Some((txt, sent)) = blocks.get_mut(&ord) {
                                                    txt.push_str(d);
                                                    on_delta(&txt[*sent..]);
                                                    *sent = txt.len();
                                                }
                                            }
                                        }
                                        "session.execution.succeeded"
                                        | "session.execution.failed" => {
                                            done = true;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            sse_data.clear();
                        }
                    } else if let Some(rest) = line.strip_prefix(b"data:") {
                        if !sse_data.is_empty() {
                            sse_data.push(b'\n');
                        }
                        sse_data.extend_from_slice(rest.strip_prefix(b" ").unwrap_or(rest));
                    }
                }
            }
            Ok(Err(())) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    let _ = local_http_json(
        port,
        &auth_header,
        "DELETE",
        &format!("/api/session/{session_id}"),
        None,
    );

    let full: String = order
        .into_iter()
        .filter_map(|ord| blocks.remove(&ord).map(|(t, _)| t))
        .collect();
    if full.trim().is_empty() {
        Err("OpenCode returned an empty response.".to_string())
    } else {
        Ok(full)
    }
}
