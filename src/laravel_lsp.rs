use regex::Regex;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};

static NS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*namespace\s+([A-Za-z0-9_\\]+)\s*[;{]").unwrap());
static CLS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(?:final\s+|readonly\s+)?(?:class|interface|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s+extends\s+([A-Za-z0-9_\\]+))?",
    )
    .unwrap()
});
static FN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"public\s+(?:static\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
});
static CLASSMAP_ENTRY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"'([A-Za-z0-9_\\\\]+)'\s*=>").unwrap());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Model,
    Mailable,
    Facade,
    Support,
    Class,
    Method,
    Helper,
    Variable,
}

impl SymbolKind {
    pub fn icon_path(&self) -> &'static str {
        match self {
            SymbolKind::Model
            | SymbolKind::Mailable
            | SymbolKind::Facade
            | SymbolKind::Support
            | SymbolKind::Class => "icons/symbol-class.svg",
            SymbolKind::Method | SymbolKind::Helper => "icons/symbol-method.svg",
            SymbolKind::Variable => "icons/symbol-var.svg",
        }
    }

    pub fn color_hex(&self) -> u32 {
        match self {
            SymbolKind::Model => 0xf59e0b,
            SymbolKind::Mailable => 0xf59e0b,
            SymbolKind::Facade => 0xa78bfa,
            SymbolKind::Support => 0x38bdf8,
            SymbolKind::Class => 0xf59e0b,
            SymbolKind::Method => 0x34d399,
            SymbolKind::Helper => 0x38bdf8,
            SymbolKind::Variable => 0xfb7185,
        }
    }

    fn priority_boost(&self) -> i32 {
        match self {
            SymbolKind::Model => 95,
            SymbolKind::Mailable => 80,
            SymbolKind::Class => 72,
            SymbolKind::Facade => 68,
            SymbolKind::Support => 62,
            SymbolKind::Variable => 60,
            SymbolKind::Method => 55,
            SymbolKind::Helper => 40,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassSymbol {
    pub short_name: String,
    pub fqcn: String,
    pub kind: SymbolKind,
    pub methods: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub namespace_context: Option<String>,
    pub insert_text: String,
    pub detail: String,
    pub kind: SymbolKind,
    pub auto_import_fqcn: Option<String>,
    pub matched_prefix_len: usize,
}

#[derive(Clone, Debug)]
pub struct CompletionPopupState {
    pub replace_range: Range<usize>,
    pub items: Vec<CompletionItem>,
    pub selected_idx: usize,
}

#[derive(Clone, Debug, Default)]
pub struct LaravelSymbolIndex {
    #[allow(dead_code)]
    pub project_path: String,
    pub classes: Vec<ClassSymbol>,
    pub by_short_name: HashMap<String, Vec<ClassSymbol>>,
    pub external_lsp_name: Option<String>,
}

pub struct LaravelLanguageService {
    pub index: Arc<LaravelSymbolIndex>,
    external_lsp: Option<Arc<Mutex<ExternalLspProcess>>>,
}

impl LaravelLanguageService {
    pub fn for_project(project_path: &str) -> Self {
        let external_lsp = ExternalLspProcess::try_spawn(project_path);
        let ext_name = external_lsp
            .as_ref()
            .and_then(|m| m.lock().ok().map(|p| p.server_name.clone()));

        let mut index = index_laravel_project(project_path);
        index.external_lsp_name = ext_name;

        Self {
            index: Arc::new(index),
            external_lsp,
        }
    }

    pub fn complete(&self, code: &str, cursor_offset: usize) -> Option<CompletionPopupState> {
        let mut popup = complete_from_index(&self.index, code, cursor_offset)?;

        // Augment with external JSON-RPC LSP completions if an LSP binary is active
        if let Some(ref lsp_arc) = self.external_lsp
            && let Ok(mut lsp) = lsp_arc.try_lock()
        {
            let ext_items = lsp.request_completions(code, cursor_offset);
            for ext in ext_items {
                if !popup.items.iter().any(|existing| {
                    existing.label == ext.label && existing.auto_import_fqcn == ext.auto_import_fqcn
                }) {
                    popup.items.push(ext);
                }
            }
            popup.items.truncate(12);
        }

        if popup.items.is_empty() {
            None
        } else {
            Some(popup)
        }
    }

    pub fn auto_import_missing(&self, code: &str, cursor_offset: usize) -> (String, usize, Vec<String>) {
        auto_import_missing_classes(code, cursor_offset, &self.index)
    }
}

// ==================== OPTIONAL EXTERNAL STDIO JSON-RPC LSP CLIENT ====================

struct ExternalLspProcess {
    server_name: String,
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    doc_version: i32,
    doc_uri: String,
}

impl ExternalLspProcess {
    fn try_spawn(project_path: &str) -> Option<Arc<Mutex<Self>>> {
        let candidates: [(&str, &[&str]); 3] = [
            ("laravel-ls", &[]),
            ("phpactor", &["language-server"]),
            ("intelephense", &["--stdio"]),
        ];

        for (bin, args) in candidates {
            let Some(resolved) = find_executable(bin) else {
                continue;
            };
            let mut cmd = Command::new(&resolved);
            cmd.args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            if Path::new(project_path).is_dir() {
                cmd.current_dir(project_path);
            }
            let Ok(mut child) = cmd.spawn() else {
                continue;
            };
            let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
                let _ = child.kill();
                continue;
            };

            let root_uri = format!("file://{project_path}");
            let doc_uri = format!("file://{project_path}/scratchpad.php");
            let mut proc = Self {
                server_name: bin.to_string(),
                _child: child,
                stdin,
                stdout: BufReader::new(stdout),
                next_id: 1,
                doc_version: 1,
                doc_uri: doc_uri.clone(),
            };

            let init_req = json!({
                "jsonrpc": "2.0",
                "id": proc.alloc_id(),
                "method": "initialize",
                "params": {
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "capabilities": {
                        "textDocument": {
                            "completion": {
                                "completionItem": {
                                    "additionalTextEditsSupport": true
                                }
                            }
                        }
                    }
                }
            });
            if proc.send_message(&init_req).is_err() || proc.read_response().is_none() {
                return None;
            }
            let _ = proc.send_message(&json!({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {}
            }));
            let _ = proc.send_message(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": doc_uri,
                        "languageId": "php",
                        "version": 1,
                        "text": "<?php\n"
                    }
                }
            }));

            return Some(Arc::new(Mutex::new(proc)));
        }

        None
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send_message(&mut self, payload: &Value) -> std::io::Result<()> {
        let body = serde_json::to_string(payload)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes())?;
        self.stdin.write_all(body.as_bytes())?;
        self.stdin.flush()
    }

    fn read_response(&mut self) -> Option<Value> {
        let mut content_len: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).ok()?;
            if n == 0 {
                return None;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                content_len = rest.trim().parse::<usize>().ok();
            }
        }
        let len = content_len?;
        let mut buf = vec![0u8; len];
        self.stdout.read_exact(&mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }

    fn request_completions(&mut self, code: &str, cursor_offset: usize) -> Vec<CompletionItem> {
        self.doc_version += 1;
        let full_php = format!("<?php\n{code}");
        let (line, character) = offset_to_lsp_pos(code, cursor_offset);

        let _ = self.send_message(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": self.doc_uri, "version": self.doc_version },
                "contentChanges": [{ "text": full_php }]
            }
        }));

        let req_id = self.alloc_id();
        if self
            .send_message(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": { "uri": self.doc_uri },
                    "position": { "line": line + 1, "character": character }
                }
            }))
            .is_err()
        {
            return Vec::new();
        }

        for _ in 0..4 {
            let Some(msg) = self.read_response() else {
                break;
            };
            if msg.get("id").and_then(|v| v.as_u64()) == Some(req_id) {
                return parse_lsp_completion_items(&msg);
            }
        }
        Vec::new()
    }
}

fn parse_lsp_completion_items(msg: &Value) -> Vec<CompletionItem> {
    let result = &msg["result"];
    let arr = if result.is_array() {
        result.as_array()
    } else {
        result.get("items").and_then(|i| i.as_array())
    };
    let Some(items) = arr else {
        return Vec::new();
    };

    let use_re = Regex::new(r"use\s+([A-Za-z0-9_\\]+)\s*;").unwrap();
    let mut out = Vec::new();

    for raw in items.iter().take(15) {
        let Some(label) = raw.get("label").and_then(|v| v.as_str()) else {
            continue;
        };
        let insert_text = raw
            .get("insertText")
            .and_then(|v| v.as_str())
            .unwrap_or(label)
            .to_string();
        let detail = raw
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut auto_fqcn = None;
        if let Some(edits) = raw.get("additionalTextEdits").and_then(|v| v.as_array()) {
            for ed in edits {
                if let Some(nt) = ed.get("newText").and_then(|v| v.as_str())
                    && let Some(caps) = use_re.captures(nt)
                {
                    auto_fqcn = Some(caps[1].trim_start_matches('\\').to_string());
                }
            }
        }
        if auto_fqcn.is_none() && detail.contains('\\') && !detail.contains(' ') {
            auto_fqcn = Some(detail.trim_start_matches('\\').to_string());
        }

        let ns_ctx = auto_fqcn
            .as_deref()
            .and_then(|fq| fq.rsplit_once('\\').map(|(ns, _)| ns.to_string()));
        let formatted_detail = if let Some(ref fq) = auto_fqcn {
            format!("↓ {fq}")
        } else {
            detail
        };
        out.push(CompletionItem {
            label: label.to_string(),
            namespace_context: ns_ctx,
            insert_text,
            detail: formatted_detail,
            kind: SymbolKind::Class,
            auto_import_fqcn: auto_fqcn,
            matched_prefix_len: 0,
        });
    }
    out
}

fn find_executable(name: &str) -> Option<PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let home = crate::paths::home_dir_string().unwrap_or_default();
    let extra = [
        format!("{home}/.composer/vendor/bin/{name}"),
        format!("{home}/.local/bin/{name}"),
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
    ];
    for c in extra {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn offset_to_lsp_pos(code: &str, cursor_offset: usize) -> (u32, u32) {
    let clamped = cursor_offset.min(code.len());
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (idx, b) in code[..clamped].bytes().enumerate() {
        if b == b'\n' {
            line += 1;
            line_start = idx + 1;
        }
    }
    let utf16_col: usize = code[line_start..clamped]
        .chars()
        .map(|c| c.len_utf16())
        .sum();
    (line, utf16_col as u32)
}

// ==================== BUILT-IN LARAVEL WORKSPACE INDEXER ====================

pub fn index_laravel_project(project_path: &str) -> LaravelSymbolIndex {
    let mut seen_fqcn: HashSet<String> = HashSet::new();
    let mut classes: Vec<ClassSymbol> = Vec::new();

    let root = Path::new(project_path);

    // 1. Scan project directories (`app/`, `database/`, `domain/`, `src/`, `modules/`)
    for dir_name in ["app", "database", "domain", "src", "modules", "Modules"] {
        let sub = root.join(dir_name);
        if sub.is_dir() {
            scan_php_directory(&sub, &mut classes, &mut seen_fqcn, 0);
        }
    }

    // 2. Scan `vendor/autoload.php` if it defines Laravel/App/Database classes (e.g. bundled playground)
    let vendor_autoload = root.join("vendor").join("autoload.php");
    if vendor_autoload.is_file() {
        scan_php_file(&vendor_autoload, &mut classes, &mut seen_fqcn, true);
    }

    // 3. Fast PSR-4 filename walk over `vendor/laravel/framework/src/Illuminate` if present
    let illuminate_src = root
        .join("vendor")
        .join("laravel")
        .join("framework")
        .join("src")
        .join("Illuminate");
    if illuminate_src.is_dir() {
        scan_psr4_framework_dir(&illuminate_src, "Illuminate", &mut classes, &mut seen_fqcn, 0);
    }

    // 4. Scan Composer's `vendor/composer/autoload_classmap.php` if available in a full Laravel project
    let composer_classmap = root
        .join("vendor")
        .join("composer")
        .join("autoload_classmap.php");
    if composer_classmap.is_file() {
        scan_composer_classmap(&composer_classmap, &mut classes, &mut seen_fqcn);
    }

    // 4. Register standard Laravel Framework Facades & Support classes
    for (short, fqcn, kind, methods) in builtin_laravel_framework_classes() {
        if seen_fqcn.insert(fqcn.to_string()) {
            classes.push(ClassSymbol {
                short_name: short.to_string(),
                fqcn: fqcn.to_string(),
                kind,
                methods: methods.iter().map(|s| (*s).to_string()).collect(),
            });
        }
    }

    classes.sort_by(|a, b| {
        b.kind
            .priority_boost()
            .cmp(&a.kind.priority_boost())
            .then_with(|| a.short_name.cmp(&b.short_name))
    });

    let mut by_short_name: HashMap<String, Vec<ClassSymbol>> = HashMap::new();
    for cls in &classes {
        by_short_name
            .entry(cls.short_name.clone())
            .or_default()
            .push(cls.clone());
    }

    LaravelSymbolIndex {
        project_path: project_path.to_string(),
        classes,
        by_short_name,
        external_lsp_name: None,
    }
}

fn scan_php_directory(
    dir: &Path,
    out: &mut Vec<ClassSymbol>,
    seen: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_php_directory(&path, out, seen, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("php") {
            scan_php_file(&path, out, seen, false);
        }
    }
}

fn scan_php_file(
    path: &Path,
    out: &mut Vec<ClassSymbol>,
    seen: &mut HashSet<String>,
    app_namespaces_only: bool,
) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };

    // Collect namespace spans in file (supports both single PSR-4 namespace and multi-namespace files)
    let ns_matches: Vec<(usize, String)> = NS_RE
        .captures_iter(&content)
        .filter_map(|c| {
            let m = c.get(0)?;
            Some((m.start(), c[1].trim().to_string()))
        })
        .collect();

    let class_matches: Vec<(usize, String, Option<String>)> = CLS_RE
        .captures_iter(&content)
        .filter_map(|c| {
            let m = c.get(0)?;
            let name = c[1].to_string();
            let ext = c.get(2).map(|e| e.as_str().to_string());
            Some((m.start(), name, ext))
        })
        .collect();

    for (idx, (pos, short_name, extends_opt)) in class_matches.iter().enumerate() {
        let ns = ns_matches
            .iter()
            .rev()
            .find(|(ns_pos, _)| ns_pos <= pos)
            .map(|(_, n)| n.as_str())
            .unwrap_or("");

        if app_namespaces_only
            && !ns.starts_with("App\\")
            && ns != "App"
            && !ns.starts_with("Database\\")
            && !ns.starts_with("Illuminate\\")
        {
            continue;
        }

        let fqcn = if ns.is_empty() {
            short_name.clone()
        } else {
            format!("{ns}\\{short_name}")
        };

        if !seen.insert(fqcn.clone()) {
            continue;
        }

        let next_class_pos = class_matches
            .get(idx + 1)
            .map(|(p, _, _)| *p)
            .unwrap_or(content.len());
        let slice = &content[*pos..next_class_pos];

        let methods: Vec<String> = FN_RE
            .captures_iter(slice)
            .map(|c| c[1].to_string())
            .filter(|m| !m.starts_with("__"))
            .collect();

        let kind = classify_fqcn(&fqcn, extends_opt.as_deref());
        out.push(ClassSymbol {
            short_name: short_name.clone(),
            fqcn,
            kind,
            methods,
        });
    }
}

fn scan_psr4_framework_dir(
    dir: &Path,
    ns_prefix: &str,
    out: &mut Vec<ClassSymbol>,
    seen: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                let child_ns = format!("{ns_prefix}\\{name}");
                scan_psr4_framework_dir(&path, &child_ns, out, seen, depth + 1);
            }
        } else if let Some(stem) = name.strip_suffix(".php")
            && stem.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
        {
            let fqcn = format!("{ns_prefix}\\{stem}");
            if seen.insert(fqcn.clone()) {
                let kind = classify_fqcn(&fqcn, None);
                out.push(ClassSymbol {
                    short_name: stem.to_string(),
                    fqcn,
                    kind,
                    methods: Vec::new(),
                });
            }
        }
    }
}

fn scan_composer_classmap(
    path: &Path,
    out: &mut Vec<ClassSymbol>,
    seen: &mut HashSet<String>,
) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for caps in CLASSMAP_ENTRY_RE.captures_iter(&content) {
        let fqcn = caps[1].replace("\\\\", "\\");
        let keep = fqcn.starts_with("App\\")
            || fqcn.starts_with("Database\\")
            || fqcn.starts_with("Domain\\")
            || fqcn.starts_with("Modules\\")
            || fqcn.starts_with("Illuminate\\");
        if !keep || !seen.insert(fqcn.clone()) {
            continue;
        }
        let Some(short) = fqcn.rsplit('\\').next() else {
            continue;
        };
        let kind = classify_fqcn(&fqcn, None);
        out.push(ClassSymbol {
            short_name: short.to_string(),
            fqcn,
            kind,
            methods: Vec::new(),
        });
    }
}

fn classify_fqcn(fqcn: &str, extends_clause: Option<&str>) -> SymbolKind {
    if fqcn.starts_with("Illuminate\\Support\\Facades\\") {
        SymbolKind::Facade
    } else if fqcn.starts_with("Illuminate\\Support\\") {
        SymbolKind::Support
    } else if fqcn.starts_with("Illuminate\\")
        || fqcn.starts_with("Symfony\\")
        || fqcn.starts_with("Psr\\")
    {
        SymbolKind::Class
    } else if fqcn.contains("\\Models\\")
        || matches!(extends_clause, Some(e) if e.ends_with("Model") || e.ends_with("Authenticatable"))
    {
        SymbolKind::Model
    } else if fqcn.contains("\\Mail\\")
        || matches!(extends_clause, Some(e) if e.ends_with("Mailable"))
    {
        SymbolKind::Mailable
    } else {
        SymbolKind::Class
    }
}

fn builtin_laravel_framework_classes() -> Vec<(&'static str, &'static str, SymbolKind, &'static [&'static str])> {
    vec![
        (
            "DB",
            "Illuminate\\Support\\Facades\\DB",
            SymbolKind::Facade,
            &["table", "select", "statement", "transaction", "raw", "listen", "getQueryLog"],
        ),
        (
            "Mail",
            "Illuminate\\Support\\Facades\\Mail",
            SymbolKind::Facade,
            &["to", "raw", "send", "queue"],
        ),
        (
            "Log",
            "Illuminate\\Support\\Facades\\Log",
            SymbolKind::Facade,
            &["info", "error", "warning", "debug", "critical", "channel"],
        ),
        (
            "Http",
            "Illuminate\\Support\\Facades\\Http",
            SymbolKind::Facade,
            &["get", "post", "put", "delete", "withToken", "withHeaders", "timeout"],
        ),
        (
            "Cache",
            "Illuminate\\Support\\Facades\\Cache",
            SymbolKind::Facade,
            &["get", "put", "remember", "rememberForever", "forget", "flush"],
        ),
        (
            "Storage",
            "Illuminate\\Support\\Facades\\Storage",
            SymbolKind::Facade,
            &["disk", "get", "put", "exists", "files", "allFiles", "delete"],
        ),
        (
            "Queue",
            "Illuminate\\Support\\Facades\\Queue",
            SymbolKind::Facade,
            &["push", "later", "size"],
        ),
        (
            "Event",
            "Illuminate\\Support\\Facades\\Event",
            SymbolKind::Facade,
            &["dispatch", "listen", "fake"],
        ),
        (
            "Route",
            "Illuminate\\Support\\Facades\\Route",
            SymbolKind::Facade,
            &["get", "post", "getRoutes", "currentRouteName"],
        ),
        (
            "Auth",
            "Illuminate\\Support\\Facades\\Auth",
            SymbolKind::Facade,
            &["user", "id", "check", "loginUsingId"],
        ),
        (
            "Hash",
            "Illuminate\\Support\\Facades\\Hash",
            SymbolKind::Facade,
            &["make", "check", "needsRehash"],
        ),
        (
            "Artisan",
            "Illuminate\\Support\\Facades\\Artisan",
            SymbolKind::Facade,
            &["call", "output", "queue"],
        ),
        (
            "Schema",
            "Illuminate\\Support\\Facades\\Schema",
            SymbolKind::Facade,
            &["hasTable", "hasColumn", "getColumnListing"],
        ),
        (
            "Config",
            "Illuminate\\Support\\Facades\\Config",
            SymbolKind::Facade,
            &["get", "set", "all"],
        ),
        (
            "Validator",
            "Illuminate\\Support\\Facades\\Validator",
            SymbolKind::Facade,
            &["make"],
        ),
        (
            "Notification",
            "Illuminate\\Support\\Facades\\Notification",
            SymbolKind::Facade,
            &["send", "route", "fake"],
        ),
        (
            "Str",
            "Illuminate\\Support\\Str",
            SymbolKind::Support,
            &["slug", "upper", "lower", "limit", "contains", "uuid", "ulid", "camel", "snake"],
        ),
        (
            "Arr",
            "Illuminate\\Support\\Arr",
            SymbolKind::Support,
            &["get", "pluck", "only", "except", "flatten", "wrap"],
        ),
        (
            "Carbon",
            "Illuminate\\Support\\Carbon",
            SymbolKind::Support,
            &["now", "parse", "today", "yesterday", "tomorrow"],
        ),
        (
            "Collection",
            "Illuminate\\Support\\Collection",
            SymbolKind::Support,
            &["make", "times", "wrap"],
        ),
    ]
}

// ==================== COMPLETION ENGINE ====================

pub fn complete_from_index(
    index: &LaravelSymbolIndex,
    code: &str,
    cursor_offset: usize,
) -> Option<CompletionPopupState> {
    let cur = clamp_boundary(code, cursor_offset);
    if is_inside_comment_or_string(code, cur) {
        return None;
    }

    let line_start = code[..cur].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let line_prefix = &code[line_start..cur];
    if line_prefix.trim_start().starts_with("use ")
        || line_prefix.trim_start().starts_with("namespace ")
    {
        return None;
    }

    // 1. Static member completion: `ClassName::prefix`
    let static_re = Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)::([A-Za-z0-9_]*)$").unwrap();
    if let Some(caps) = static_re.captures(line_prefix) {
        let class_name = &caps[1];
        let method_prefix = &caps[2];
        let replace_start = cur - method_prefix.len();
        let items = complete_static_members(index, class_name, method_prefix);
        if !items.is_empty() {
            return Some(CompletionPopupState {
                replace_range: replace_start..cur,
                items,
                selected_idx: 0,
            });
        }
        return None;
    }

    // 2. Instance / fluent chain completion: `->prefix`
    let arrow_re = Regex::new(r"->([A-Za-z0-9_]*)$").unwrap();
    if let Some(caps) = arrow_re.captures(line_prefix) {
        let method_prefix = &caps[1];
        let replace_start = cur - method_prefix.len();
        let items = complete_fluent_methods(index, method_prefix);
        if !items.is_empty() {
            return Some(CompletionPopupState {
                replace_range: replace_start..cur,
                items,
                selected_idx: 0,
            });
        }
        return None;
    }

    // 2.5 PHP `$variable` completion from current buffer + custom driver variables
    let var_re = Regex::new(r"(\$[A-Za-z_][A-Za-z0-9_]*)$").unwrap();
    if let Some(caps) = var_re.captures(line_prefix) {
        let var_prefix = &caps[1];
        if var_prefix.len() >= 2 {
            let replace_start = cur - var_prefix.len();
            let items = complete_buffer_variables(code, replace_start, var_prefix);
            if !items.is_empty() {
                return Some(CompletionPopupState {
                    replace_range: replace_start..cur,
                    items,
                    selected_idx: 0,
                });
            }
        }
        return None;
    }

    // 3. Class / Model / Facade / Helper identifier completion
    let ident_re = Regex::new(r"(^|[^$A-Za-z0-9_\\])([A-Za-z_][A-Za-z0-9_]*)$").unwrap();
    let caps = ident_re.captures(line_prefix)?;
    let prefix = &caps[2];
    if prefix.is_empty() {
        return None;
    }

    // Ignore PHP language keywords while typing them directly
    if matches!(
        prefix,
        "use" | "new" | "fn" | "if" | "as" | "for" | "try" | "echo" | "true" | "false" | "null"
    ) {
        return None;
    }

    let replace_start = cur - prefix.len();
    let existing_imports = parse_existing_imports(code);
    let prefix_lower = prefix.to_lowercase();

    let mut scored: Vec<(i32, CompletionItem)> = Vec::new();

    for cls in &index.classes {
        if let Some(match_score) = symbol_match_score(&prefix_lower, prefix, &cls.short_name) {
            let already_imported = existing_imports
                .get(&cls.short_name)
                .map(|fq| fq == &cls.fqcn)
                .unwrap_or(false);

            let ns_ctx = cls
                .fqcn
                .rsplit_once('\\')
                .map(|(ns, _)| ns.to_string());
            let detail = if already_imported {
                cls.fqcn.clone()
            } else {
                format!("↓ {}", cls.fqcn)
            };
            let matched_len = if cls.short_name.to_lowercase().starts_with(&prefix_lower) {
                prefix.len().min(cls.short_name.len())
            } else {
                0
            };

            scored.push((
                match_score + cls.kind.priority_boost(),
                CompletionItem {
                    label: cls.short_name.clone(),
                    namespace_context: ns_ctx,
                    insert_text: cls.short_name.clone(),
                    detail,
                    kind: cls.kind,
                    auto_import_fqcn: if already_imported {
                        None
                    } else {
                        Some(cls.fqcn.clone())
                    },
                    matched_prefix_len: matched_len,
                },
            ));
        }
    }

    // Global Laravel helpers if prefix starts lowercase
    if prefix
        .chars()
        .next()
        .map(|c| c.is_lowercase())
        .unwrap_or(false)
    {
        for (helper, sig) in laravel_global_helpers() {
            if let Some(ms) = symbol_match_score(&prefix_lower, prefix, helper) {
                let matched_len = if helper.starts_with(&prefix_lower) {
                    prefix.len().min(helper.len())
                } else {
                    0
                };
                scored.push((
                    ms + SymbolKind::Helper.priority_boost(),
                    CompletionItem {
                        label: format!("{helper}()"),
                        namespace_context: None,
                        insert_text: format!("{helper}("),
                        detail: sig.to_string(),
                        kind: SymbolKind::Helper,
                        auto_import_fqcn: None,
                        matched_prefix_len: matched_len,
                    },
                ));
            }
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    let items: Vec<CompletionItem> = scored.into_iter().take(14).map(|(_, it)| it).collect();
    if items.is_empty() {
        return None;
    }

    // Avoid popping up when the user has already typed the exact completed identifier with no import needed
    if items.len() == 1 && items[0].insert_text == prefix && items[0].auto_import_fqcn.is_none() {
        return None;
    }

    Some(CompletionPopupState {
        replace_range: replace_start..cur,
        items,
        selected_idx: 0,
    })
}

fn complete_buffer_variables(code: &str, exclude_at: usize, var_prefix: &str) -> Vec<CompletionItem> {
    let re = Regex::new(r"\$[A-Za-z_][A-Za-z0-9_]*").unwrap();
    let p_low = var_prefix.to_lowercase();
    let mut vars: BTreeSet<String> = BTreeSet::new();
    vars.insert("$appName".to_string());
    vars.insert("$appVersion".to_string());

    for m in re.find_iter(code) {
        if m.start() != exclude_at {
            vars.insert(m.as_str().to_string());
        }
    }

    vars.into_iter()
        .filter(|v| v != var_prefix && v.to_lowercase().starts_with(&p_low))
        .take(10)
        .map(|v| CompletionItem {
            matched_prefix_len: var_prefix.len().min(v.len()),
            label: v.clone(),
            namespace_context: None,
            insert_text: v,
            detail: "variable".to_string(),
            kind: SymbolKind::Variable,
            auto_import_fqcn: None,
        })
        .collect()
}

fn complete_static_members(
    index: &LaravelSymbolIndex,
    class_name: &str,
    prefix: &str,
) -> Vec<CompletionItem> {
    let p_low = prefix.to_lowercase();
    let mut candidates: Vec<(String, &'static str)> = Vec::new();

    if let Some(symbols) = index.by_short_name.get(class_name)
        && let Some(sym) = symbols.first()
    {
        if sym.kind == SymbolKind::Model {
            for (m, det) in eloquent_static_methods() {
                candidates.push((m.to_string(), det));
            }
        }
        for m in &sym.methods {
            if !candidates.iter().any(|(existing, _)| existing == m) {
                candidates.push((format!("{m}("), "method"));
            }
        }
    } else {
        for (m, det) in eloquent_static_methods() {
            candidates.push((m.to_string(), det));
        }
    }
    candidates.push(("class".to_string(), "class-string"));

    candidates
        .into_iter()
        .filter(|(m, _)| p_low.is_empty() || m.to_lowercase().starts_with(&p_low))
        .take(12)
        .map(|(m, det)| {
            let label = if m.ends_with('(') {
                format!("{m}...)")
            } else {
                m.clone()
            };
            let matched = prefix.len().min(label.len());
            CompletionItem {
                label,
                namespace_context: None,
                insert_text: m,
                detail: det.to_string(),
                kind: SymbolKind::Method,
                auto_import_fqcn: None,
                matched_prefix_len: matched,
            }
        })
        .collect()
}

fn complete_fluent_methods(index: &LaravelSymbolIndex, prefix: &str) -> Vec<CompletionItem> {
    let p_low = prefix.to_lowercase();
    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for (m, det) in eloquent_and_collection_chain_methods() {
        if (p_low.is_empty() || m.to_lowercase().starts_with(&p_low)) && seen.insert(m.to_string()) {
            let label = if m.ends_with('(') {
                format!("{m}...)")
            } else {
                m.to_string()
            };
            items.push(CompletionItem {
                matched_prefix_len: prefix.len().min(label.len()),
                label,
                namespace_context: None,
                insert_text: m.to_string(),
                detail: det.to_string(),
                kind: SymbolKind::Method,
                auto_import_fqcn: None,
            });
        }
    }

    for cls in &index.classes {
        if matches!(cls.kind, SymbolKind::Model | SymbolKind::Mailable) {
            for m in &cls.methods {
                let ins = format!("{m}(");
                if (p_low.is_empty() || m.to_lowercase().starts_with(&p_low))
                    && seen.insert(ins.clone())
                {
                    let label = format!("{m}()");
                    items.push(CompletionItem {
                        matched_prefix_len: prefix.len().min(label.len()),
                        label,
                        namespace_context: Some(cls.short_name.clone()),
                        insert_text: ins,
                        detail: "relation/method".to_string(),
                        kind: SymbolKind::Method,
                        auto_import_fqcn: None,
                    });
                }
            }
        }
    }

    items.truncate(12);
    items
}

fn eloquent_static_methods() -> &'static [(&'static str, &'static str)] {
    &[
        ("where(", "Eloquent\\Builder"),
        ("find(", "Model|null"),
        ("all()", "Collection<Model>"),
        ("query()", "Eloquent\\Builder"),
        ("with(", "Eloquent\\Builder"),
        ("orderBy(", "Eloquent\\Builder"),
        ("orderByDesc(", "Eloquent\\Builder"),
        ("count()", "int"),
        ("first()", "Model|null"),
    ]
}

fn eloquent_and_collection_chain_methods() -> &'static [(&'static str, &'static str)] {
    &[
        ("get()", "Collection"),
        ("first()", "mixed"),
        ("where(", "Builder|Collection"),
        ("with(", "Eloquent\\Builder"),
        ("count()", "int"),
        ("orderBy(", "Eloquent\\Builder"),
        ("orderByDesc(", "Eloquent\\Builder"),
        ("limit(", "Eloquent\\Builder"),
        ("take(", "Builder|Collection"),
        ("toSql()", "string"),
        ("map(", "Collection"),
        ("filter(", "Collection"),
        ("pluck(", "Collection"),
        ("sum(", "int|float"),
        ("avg(", "float"),
        ("groupBy(", "Collection"),
        ("sortBy(", "Collection"),
        ("sortByDesc(", "Collection"),
        ("values()", "Collection"),
        ("toArray()", "array"),
        ("toJson()", "string"),
    ]
}

fn laravel_global_helpers() -> &'static [(&'static str, &'static str)] {
    &[
        ("collect", "collect(array $items = []): Collection"),
        ("now", "now(): Carbon"),
        ("config", "config(string $key, mixed $default = null)"),
        ("env", "env(string $key, mixed $default = null)"),
        ("app", "app(?string $abstract = null)"),
        ("base_path", "base_path(string $path = ''): string"),
        ("storage_path", "storage_path(string $path = ''): string"),
        ("dump", "dump(mixed ...$vars): mixed"),
        ("dd", "dd(mixed ...$vars): never"),
    ]
}

fn symbol_match_score(prefix_lower: &str, raw_prefix: &str, candidate: &str) -> Option<i32> {
    let cand_lower = candidate.to_lowercase();
    if cand_lower == prefix_lower {
        return Some(500 + if candidate == raw_prefix { 50 } else { 0 });
    }
    if cand_lower.starts_with(prefix_lower) {
        let case_bonus = if candidate.starts_with(raw_prefix) {
            40
        } else {
            0
        };
        return Some(400 + case_bonus - (candidate.len() as i32));
    }
    if prefix_lower.len() >= 2 && cand_lower.contains(prefix_lower) {
        return Some(200 - (candidate.len() as i32));
    }
    None
}

// ==================== AUTO-IMPORT ENGINE ====================

/// Inserts `use <fqcn>;` cleanly into the top-level `use` block of `code` (if not already present)
/// and returns `(updated_code, updated_cursor_offset)`.
pub fn insert_use_import(code: &str, cursor_offset: usize, fqcn: &str) -> (String, usize) {
    let clean_fqcn = fqcn.trim().trim_start_matches('\\').trim_end_matches(';');
    if clean_fqcn.is_empty() || !clean_fqcn.contains('\\') {
        return (code.to_string(), cursor_offset);
    }

    let short_name = clean_fqcn.rsplit('\\').next().unwrap_or(clean_fqcn);
    let existing = parse_existing_imports(code);
    if existing.contains_key(short_name) {
        return (code.to_string(), cursor_offset);
    }

    let use_stmt = format!("use {clean_fqcn};");
    let lines: Vec<&str> = code.split('\n').collect();

    // Locate header (`<?php`, `declare(...)`) and any existing top-level `use ...;` lines
    let mut header_end_line = 0usize;
    let mut use_line_range: Option<(usize, usize)> = None;

    for (idx, raw) in lines.iter().enumerate() {
        let t = raw.trim();
        if idx == 0 && t.starts_with("<?") {
            header_end_line = idx + 1;
            continue;
        }
        if t.starts_with("declare(") {
            header_end_line = idx + 1;
            continue;
        }
        if t.is_empty() && use_line_range.is_none() && idx == header_end_line {
            header_end_line = idx + 1;
            continue;
        }
        if t.starts_with("use ") && t.ends_with(';') && !t.contains('(') {
            if let Some((s, _)) = use_line_range {
                use_line_range = Some((s, idx));
            } else {
                use_line_range = Some((idx, idx));
            }
            continue;
        }
        if use_line_range.is_some() {
            break;
        }
        if !t.is_empty() {
            break;
        }
    }

    if let Some((first_use, last_use)) = use_line_range {
        let block_start_byte = line_index_to_byte(code, first_use);
        let block_end_byte = line_end_index_to_byte(code, last_use);

        let mut use_set: BTreeSet<String> = lines[first_use..=last_use]
            .iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        use_set.insert(use_stmt);

        let new_block = use_set.into_iter().collect::<Vec<_>>().join("\n");
        let old_len = block_end_byte - block_start_byte;
        let new_len = new_block.len();

        let mut updated = String::with_capacity(code.len() + 64);
        updated.push_str(&code[..block_start_byte]);
        updated.push_str(&new_block);
        updated.push_str(&code[block_end_byte..]);

        let new_cursor = if cursor_offset >= block_end_byte {
            (cursor_offset + new_len).saturating_sub(old_len)
        } else {
            cursor_offset
        };
        return (updated, new_cursor);
    }

    // No existing `use` block yet: insert right at `header_end_line` with a blank line separator
    let insert_byte = line_index_to_byte(code, header_end_line);
    let rest = &code[insert_byte..];
    let separator = if rest.starts_with('\n') || rest.is_empty() {
        "\n"
    } else {
        "\n\n"
    };
    let insertion = format!("{use_stmt}{separator}");

    let mut updated = String::with_capacity(code.len() + insertion.len());
    updated.push_str(&code[..insert_byte]);
    updated.push_str(&insertion);
    updated.push_str(rest);

    let new_cursor = if cursor_offset >= insert_byte {
        cursor_offset + insertion.len()
    } else {
        cursor_offset
    };

    (updated, new_cursor)
}

/// Scans `code` for unimported class references (`ClassName::`, `new ClassName`, `instanceof ClassName`)
/// outside strings/comments and automatically inserts matching `use ...;` statements at the top.
pub fn auto_import_missing_classes(
    code: &str,
    cursor_offset: usize,
    index: &LaravelSymbolIndex,
) -> (String, usize, Vec<String>) {
    let sanitized = mask_strings_and_comments(code);
    let existing = parse_existing_imports(&sanitized);
    let local_defs = parse_locally_defined_classes(&sanitized);

    let ref_re = Regex::new(
        r"(?:(?:\bnew|\binstanceof)\s+([A-Z][A-Za-z0-9_]*)\b|(?:^|[^\\A-Za-z0-9_])([A-Z][A-Za-z0-9_]*)::)",
    )
    .unwrap();

    let mut needed_fqcns: Vec<String> = Vec::new();
    let mut seen_short: HashSet<String> = HashSet::new();

    for caps in ref_re.captures_iter(&sanitized) {
        let Some(m) = caps.get(1).or_else(|| caps.get(2)) else {
            continue;
        };
        let short = m.as_str();
        if matches!(short, "Self" | "Static" | "Parent")
            || existing.contains_key(short)
            || local_defs.contains(short)
            || !seen_short.insert(short.to_string())
        {
            continue;
        }
        if let Some(candidates) = index.by_short_name.get(short)
            && let Some(best) = candidates.first()
        {
            needed_fqcns.push(best.fqcn.clone());
        }
    }

    let mut current_code = code.to_string();
    let mut current_cursor = cursor_offset;
    let mut added = Vec::new();

    for fqcn in needed_fqcns {
        let (next_code, next_cursor) = insert_use_import(&current_code, current_cursor, &fqcn);
        if next_code != current_code {
            current_code = next_code;
            current_cursor = next_cursor;
            added.push(fqcn);
        }
    }

    (current_code, current_cursor, added)
}

pub fn parse_existing_imports(code: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let use_re =
        Regex::new(r"(?m)^\s*use\s+([A-Za-z0-9_\\]+)(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;")
            .unwrap();
    for caps in use_re.captures_iter(code) {
        let fqcn = caps[1].trim_start_matches('\\').to_string();
        let alias = caps
            .get(2)
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| fqcn.rsplit('\\').next().unwrap_or(&fqcn).to_string());
        map.insert(alias, fqcn);
    }
    map
}

fn parse_locally_defined_classes(code: &str) -> HashSet<String> {
    let re = Regex::new(r"\b(?:class|interface|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    re.captures_iter(code)
        .map(|c| c[1].to_string())
        .collect()
}

fn line_index_to_byte(code: &str, target_line: usize) -> usize {
    if target_line == 0 {
        return 0;
    }
    let mut line = 0usize;
    for (idx, b) in code.bytes().enumerate() {
        if b == b'\n' {
            line += 1;
            if line == target_line {
                return idx + 1;
            }
        }
    }
    code.len()
}

fn line_end_index_to_byte(code: &str, target_line: usize) -> usize {
    let start = line_index_to_byte(code, target_line);
    code[start..]
        .find('\n')
        .map(|rel| start + rel)
        .unwrap_or(code.len())
}

fn clamp_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn is_inside_comment_or_string(code: &str, target: usize) -> bool {
    let bytes = code.as_bytes();
    let end = target.min(bytes.len());
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < end {
        let b = bytes[i];
        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if i + 1 < end && b == b'*' && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if in_single {
            if b == b'\\' && i + 1 < end {
                i += 2;
                continue;
            }
            if b == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if b == b'\\' && i + 1 < end {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }

        if i + 1 < end && b == b'/' && bytes[i + 1] == b'/' {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if i + 1 < end && b == b'/' && bytes[i + 1] == b'*' {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if b == b'\'' {
            in_single = true;
        } else if b == b'"' {
            in_double = true;
        }
        i += 1;
    }

    in_single || in_double || in_line_comment || in_block_comment
}

/// Replaces string literal and comment contents with spaces of identical byte length so regexes
/// never match class names inside strings or comments while preserving byte indices.
fn mask_strings_and_comments(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = bytes.to_vec();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let q = b;
            out[i] = b' ';
            i += 1;
            while i < len {
                let c = bytes[i];
                if c != b'\n' {
                    out[i] = b' ';
                }
                if c == b'\\' && i + 1 < len {
                    if bytes[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
                    i += 2;
                    continue;
                }
                i += 1;
                if c == q {
                    break;
                }
            }
            continue;
        }
        if i + 1 < len && b == b'/' && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
            continue;
        }
        if i + 1 < len && b == b'/' && bytes[i + 1] == b'*' {
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i < len {
                if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    break;
                }
                if bytes[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }

    String::from_utf8(out).unwrap_or_else(|_| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::default_playground_path;

    #[test]
    fn indexes_playground_models_mailables_and_facades() {
        let pg = default_playground_path();
        let index = index_laravel_project(&pg);
        assert!(index.by_short_name.contains_key("Article"));
        assert!(index.by_short_name.contains_key("User"));
        assert!(index.by_short_name.contains_key("InvoiceShipped"));
        assert!(index.by_short_name.contains_key("DB"));
        assert!(index.by_short_name.contains_key("Str"));
    }

    #[test]
    fn completes_article_model_and_inserts_use_import_matching_screenshot() {
        let pg = default_playground_path();
        let index = index_laravel_project(&pg);

        let code = "$articles = Art";
        let popup = complete_from_index(&index, code, code.len()).expect("popup should appear");
        assert_eq!(popup.items[0].label, "Article");
        assert_eq!(
            popup.items[0].auto_import_fqcn.as_deref(),
            Some("App\\Models\\Article")
        );

        // Simulate accepting `Article` completion + auto-importing `App\Models\Article`
        let mut updated = code.to_string();
        updated.replace_range(popup.replace_range.clone(), &popup.items[0].insert_text);
        let cursor_after_word = popup.replace_range.start + popup.items[0].insert_text.len();
        let (final_code, final_cursor) = insert_use_import(
            &updated,
            cursor_after_word,
            popup.items[0].auto_import_fqcn.as_deref().unwrap(),
        );

        assert_eq!(
            final_code,
            "use App\\Models\\Article;\n\n$articles = Article"
        );
        assert_eq!(final_cursor, final_code.len());
    }

    #[test]
    fn auto_imports_missing_models_and_facades_without_duplicating() {
        let pg = default_playground_path();
        let index = index_laravel_project(&pg);

        let code = "$articles = Article::all();\n$slug = Str::slug('Hello World');";
        let (updated, _, added) = auto_import_missing_classes(code, code.len(), &index);
        assert_eq!(added.len(), 2);
        assert!(updated.contains("use App\\Models\\Article;"));
        assert!(updated.contains("use Illuminate\\Support\\Str;"));

        // Second pass adds nothing
        let (again, _, added2) = auto_import_missing_classes(&updated, updated.len(), &index);
        assert!(added2.is_empty());
        assert_eq!(again, updated);
    }

    #[test]
    fn populates_rich_dropdown_items_with_namespace_context_and_variables() {
        let pg = default_playground_path();
        let index = index_laravel_project(&pg);

        let code = "$articles = Arti";
        let popup = complete_from_index(&index, code, code.len()).expect("popup should appear");
        assert!(popup.items.len() >= 6);
        assert_eq!(popup.items[0].label, "Article");
        assert_eq!(
            popup.items[0].namespace_context.as_deref(),
            Some("App\\Models")
        );
        assert_eq!(popup.items[0].detail, "↓ App\\Models\\Article");
        assert_eq!(popup.items[0].matched_prefix_len, 4);

        let labels: Vec<&str> = popup.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"ArticleFactory"));
        assert!(labels.contains(&"ArticlePolicy"));
        assert!(labels.contains(&"ArticleResource"));
        assert!(labels.contains(&"Artisan"));
        assert!(labels.contains(&"ArtisanServiceProvider"));
        assert!(labels.contains(&"ArtisanStarting"));

        let var_code = "$articles = Article::all();\n$art";
        let var_popup =
            complete_from_index(&index, var_code, var_code.len()).expect("var popup should appear");
        assert_eq!(var_popup.items[0].label, "$articles");
        assert_eq!(var_popup.items[0].kind, SymbolKind::Variable);
        assert_eq!(var_popup.items[0].matched_prefix_len, 4);
    }

    #[test]
    fn excludes_framework_illuminate_mail_classes_from_user_mailables() {
        assert_eq!(
            classify_fqcn("Illuminate\\Mail\\Transport\\CloudflareTransport", None),
            SymbolKind::Class
        );
        assert_eq!(
            classify_fqcn("Illuminate\\Mail\\Mailables\\Envelope", None),
            SymbolKind::Class
        );
        assert_eq!(
            classify_fqcn("App\\Mail\\InvoiceShipped", None),
            SymbolKind::Mailable
        );
        assert_eq!(
            classify_fqcn("App\\Models\\EnglishWord", None),
            SymbolKind::Model
        );
    }
}
