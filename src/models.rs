use crate::ssh::{SshAuthMethod, default_private_key_path, default_ssh_color};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionTarget {
    Local {
        project_path: String,
    },
    Docker {
        container_id: String,
        container_name: String,
        image: String,
        workdir: String,
        local_path: String,
    },
    Ssh {
        #[serde(default)]
        id: String,
        name: String,
        host: String,
        port: u16,
        user: String,
        workdir: String,
        #[serde(default)]
        docker_container: Option<String>,
        is_production: bool,
        #[serde(default)]
        local_path: Option<String>,
        #[serde(default)]
        auth_method: SshAuthMethod,
        #[serde(default = "default_private_key_path")]
        private_key: String,
        #[serde(default)]
        passphrase: String,
        #[serde(default = "default_ssh_color")]
        color_hex: u32,
    },
}

impl ExecutionTarget {
    pub fn label(&self) -> String {
        match self {
            ExecutionTarget::Local { project_path } => {
                let name = std::path::Path::new(project_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(project_path);
                format!("Local ({name})")
            }
            ExecutionTarget::Docker { container_name, .. } => {
                format!("Docker ({container_name})")
            }
            ExecutionTarget::Ssh {
                name,
                docker_container,
                is_production,
                ..
            } => {
                let docker_suffix = match docker_container.as_deref().filter(|s| !s.is_empty()) {
                    Some(c) => format!(" ({c})"),
                    None => String::new(),
                };
                if *is_production {
                    format!("SSH: {name}{docker_suffix} [PROD]")
                } else {
                    format!("SSH: {name}{docker_suffix}")
                }
            }
        }
    }

    pub fn kind_badge(&self) -> &'static str {
        match self {
            ExecutionTarget::Local { .. } => "LOCAL",
            ExecutionTarget::Docker { .. } => "DOCKER",
            ExecutionTarget::Ssh {
                is_production: true,
                docker_container: Some(_),
                ..
            } => "SSH DOCKER PROD",
            ExecutionTarget::Ssh {
                is_production: false,
                docker_container: Some(_),
                ..
            } => "SSH DOCKER",
            ExecutionTarget::Ssh {
                is_production: true,
                docker_container: None,
                ..
            } => "SSH PROD",
            ExecutionTarget::Ssh {
                is_production: false,
                docker_container: None,
                ..
            } => "SSH",
        }
    }

    pub fn is_ssh(&self) -> bool {
        matches!(self, ExecutionTarget::Ssh { .. })
    }

    pub fn ssh_info(&self) -> Option<(&str, u32)> {
        match self {
            ExecutionTarget::Ssh {
                name, color_hex, ..
            } => Some((name.as_str(), *color_hex)),
            _ => None,
        }
    }

    pub fn local_or_workdir(&self) -> &str {
        match self {
            ExecutionTarget::Local { project_path } => project_path,
            ExecutionTarget::Docker { workdir, .. } => workdir,
            ExecutionTarget::Ssh { workdir, .. } => workdir,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    DetailDive,
    TableMode,
    SqlQueries,
    MailablePreview,
    RawCli,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectNode {
    pub id: String,
    pub label: String,
    pub type_name: String,
    pub class_name: Option<String>,
    #[serde(default)]
    pub object_id: Option<usize>,
    pub visibility: Option<String>,
    pub summary: String,
    pub scalar_value: Option<String>,
    #[serde(default)]
    pub children: Vec<ObjectNode>,
    #[serde(default)]
    pub expanded_by_default: bool,
    #[serde(default)]
    pub html_preview: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DumpCard {
    pub id: String,
    pub line: usize,
    pub file: String,
    pub kind: String,
    pub node: ObjectNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SqlQueryEntry {
    pub sql: String,
    #[serde(default)]
    pub bindings: Vec<String>,
    pub interpolated_sql: String,
    pub time_ms: f64,
    pub connection: String,
    #[serde(default)]
    pub is_duplicate: bool,
    #[serde(default)]
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DetailStreamItem {
    Sql(SqlQueryEntry),
    Value {
        card: DumpCard,
        attach_mailable_eye: bool,
    },
}

pub fn ordered_detail_items(res: &EvaluationResult, include_sql: bool) -> Vec<DetailStreamItem> {
    let mut staged: Vec<(usize, u8, usize, DetailStreamItem)> = Vec::new();

    if include_sql {
        for (idx, q) in res.sql_queries.iter().enumerate() {
            staged.push((q.line, 0, idx, DetailStreamItem::Sql(q.clone())));
        }
    }

    for (idx, d) in res.dumps.iter().enumerate() {
        staged.push((
            d.line,
            1,
            idx,
            DetailStreamItem::Value {
                card: d.clone(),
                attach_mailable_eye: false,
            },
        ));
    }

    if let Some(ref ret) = res.return_value {
        let null_after_dump_on_same_line = ret.node.type_name == "null"
            && res.dumps.iter().any(|d| d.line == ret.line);
        if !null_after_dump_on_same_line {
            staged.push((
                ret.line,
                3,
                0,
                DetailStreamItem::Value {
                    card: ret.clone(),
                    attach_mailable_eye: res.mailable.is_some(),
                },
            ));
        }
    }

    staged.sort_by_key(|(line, rank, seq, _)| (*line, *rank, *seq));
    staged.into_iter().map(|(_, _, _, item)| item).collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MailablePreviewData {
    pub class_name: String,
    pub subject: String,
    pub from: String,
    #[serde(default)]
    pub to: Vec<String>,
    pub html: String,
    pub text_summary: String,
    #[serde(default)]
    pub markdown_sections: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableDataset {
    pub source_label: String,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
    #[serde(default)]
    pub raw_rows_json: Vec<String>,
    #[serde(default)]
    pub raw_rows_php: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExceptionData {
    pub class: String,
    pub message: String,
    pub line: usize,
    pub file: String,
    #[serde(default)]
    pub trace: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvaluationResult {
    pub status: String,
    pub duration_ms: f64,
    pub memory_mb: f64,
    pub php_version: String,
    pub laravel_version: String,
    pub driver_name: String,
    #[serde(default)]
    pub driver_vars: Vec<(String, String)>,
    #[serde(default)]
    pub raw_output: String,
    #[serde(default)]
    pub dumps: Vec<DumpCard>,
    pub return_value: Option<DumpCard>,
    #[serde(default)]
    pub sql_queries: Vec<SqlQueryEntry>,
    pub mailable: Option<MailablePreviewData>,
    pub table: Option<TableDataset>,
    pub exception: Option<ExceptionData>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct LaravelTemplate {
    pub id: &'static str,
    pub code: &'static str,
}

#[cfg(test)]
pub fn builtin_laravel_templates() -> Vec<LaravelTemplate> {
    vec![
        LaravelTemplate {
            id: "eloquent-overview",
            code: r#"use App\Mail\InvoiceShipped;
use App\Models\Order;
use App\Models\User;

// Get a user from your database
$user = User::find(1);
dump($user);

// Inspect active Pro & Enterprise accounts
$activeUsers = User::where('active', true)
    ->where('plan', '!=', 'starter')
    ->get();
dump($activeUsers->count());

$totalMrr = $activeUsers->sum('mrr');
dump($totalMrr);

// Preview shipped invoice email
$order = Order::find(103);
new InvoiceShipped($order->user(), $order);"#,
        },
        LaravelTemplate {
            id: "detail-dive",
            code: r#"use App\Models\User;
use App\Models\Order;

$taylor = User::with(['orders', 'posts'])->find(1);
dump($taylor);

$summary = (object) [
    'driver' => $appName ?? 'Laravel',
    'version' => $appVersion ?? app()->version(),
    'top_order' => Order::orderByDesc('total')->first(),
    'metrics' => [
        'total_users' => User::count(),
        'total_orders' => Order::count(),
        'paid_revenue' => Order::where('status', 'paid')->get()->sum('total'),
    ],
];

$summary;"#,
        },
        LaravelTemplate {
            id: "table-report",
            code: r#"use App\Models\User;

User::query()
    ->orderByDesc('mrr')
    ->get();"#,
        },
        LaravelTemplate {
            id: "mailable-preview",
            code: r#"use App\Mail\InvoiceShipped;
use App\Models\Order;

$order = Order::find(103);
$user = $order->user();

new InvoiceShipped($user, $order);"#,
        },
        LaravelTemplate {
            id: "sql-n1-check",
            code: r#"use App\Models\User;

// Intentionally trigger lazy-loading to demonstrate duplicate / N+1 detection
$users = User::where('plan', 'enterprise')->get();

$report = $users->map(function ($user) {
    $orders = \App\Models\Order::where('user_id', $user->id)->get();
    $dupCheck = \App\Models\Order::where('user_id', 1)->get();
    return [
        'name' => $user->name,
        'orders_count' => $orders->count(),
        'sample' => $dupCheck->count(),
    ];
});

$report;"#,
        },
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliTokenKind {
    ClassName,
    ObjectHandle,
    Key,
    String,
    Number,
    BoolOrNull,
    Stdout,
    Error,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliLine {
    pub indent_depth: usize,
    pub spans: Vec<(CliTokenKind, String)>,
}

pub fn format_cli_text(res: &EvaluationResult) -> String {
    format_cli_lines(res)
        .into_iter()
        .map(|line| {
            if line.spans.is_empty() {
                String::new()
            } else {
                let indent = "  ".repeat(line.indent_depth);
                let body: String = line.spans.into_iter().map(|(_, s)| s).collect();
                format!("{indent}{body}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_cli_lines(res: &EvaluationResult) -> Vec<CliLine> {
    let mut lines: Vec<CliLine> = Vec::new();

    let trimmed_raw = res.raw_output.trim_end_matches(['\r', '\n']);
    if !trimmed_raw.is_empty() {
        for raw_line in trimmed_raw.split('\n') {
            lines.push(CliLine {
                indent_depth: 0,
                spans: vec![(
                    CliTokenKind::Stdout,
                    raw_line.trim_end_matches('\r').to_string(),
                )],
            });
        }
    }

    for item in ordered_detail_items(res, false) {
        if let DetailStreamItem::Value { card, .. } = item {
            if !lines.is_empty() {
                lines.push(CliLine {
                    indent_depth: 0,
                    spans: Vec::new(),
                });
            }
            format_node_cli(&card.node, 0, None, false, &mut lines);
        }
    }

    if lines.is_empty() && res.exception.is_none() {
        lines.push(CliLine {
            indent_depth: 0,
            spans: vec![(CliTokenKind::BoolOrNull, "null".to_string())],
        });
    }

    if let Some(ref exc) = res.exception {
        if !lines.is_empty() {
            lines.push(CliLine {
                indent_depth: 0,
                spans: Vec::new(),
            });
        }
        let mut msg_lines = exc.message.split('\n');
        let first_msg = msg_lines.next().unwrap_or("").trim_end_matches('\r');
        lines.push(CliLine {
            indent_depth: 0,
            spans: vec![
                (CliTokenKind::Error, format!("{}  ", exc.class)),
                (CliTokenKind::Plain, first_msg.to_string()),
            ],
        });
        for extra_msg in msg_lines {
            lines.push(CliLine {
                indent_depth: 1,
                spans: vec![(
                    CliTokenKind::Plain,
                    extra_msg.trim_end_matches('\r').to_string(),
                )],
            });
        }
        if exc.line > 0 {
            lines.push(CliLine {
                indent_depth: 1,
                spans: vec![(
                    CliTokenKind::ObjectHandle,
                    format!("at {}:{}", exc.file, exc.line),
                )],
            });
        }
        for tr in &exc.trace {
            for tr_line in tr.split('\n') {
                lines.push(CliLine {
                    indent_depth: 1,
                    spans: vec![(
                        CliTokenKind::ObjectHandle,
                        tr_line.trim_end_matches('\r').to_string(),
                    )],
                });
            }
        }
        return lines;
    }

    lines
}

fn format_node_cli(
    node: &ObjectNode,
    depth: usize,
    prefix: Option<String>,
    trailing_comma: bool,
    out: &mut Vec<CliLine>,
) {
    let comma = if trailing_comma { "," } else { "" };

    let is_collection = node.type_name.starts_with("Collection:")
        || node
            .class_name
            .as_deref()
            .is_some_and(|c| c.ends_with("\\Collection") || c == "Collection");

    if is_collection {
        let fqcn = node
            .class_name
            .as_deref()
            .unwrap_or("Illuminate\\Support\\Collection");
        let oid_open = match node.object_id {
            Some(oid) => format!(" {{#{oid}"),
            None => " {".to_string(),
        };
        let mut open_spans = Vec::new();
        if let Some(p) = prefix {
            open_spans.push((CliTokenKind::Key, p));
        }
        open_spans.push((CliTokenKind::ClassName, fqcn.to_string()));
        open_spans.push((CliTokenKind::ObjectHandle, oid_open));
        out.push(CliLine {
            indent_depth: depth,
            spans: open_spans,
        });

        if node.children.is_empty() {
            out.push(CliLine {
                indent_depth: depth + 1,
                spans: vec![
                    (CliTokenKind::Key, "all: ".to_string()),
                    (CliTokenKind::ObjectHandle, "[],".to_string()),
                ],
            });
        } else {
            out.push(CliLine {
                indent_depth: depth + 1,
                spans: vec![
                    (CliTokenKind::Key, "all: ".to_string()),
                    (CliTokenKind::ObjectHandle, "[".to_string()),
                ],
            });
            let is_seq = node
                .children
                .iter()
                .enumerate()
                .all(|(i, c)| c.label == i.to_string());
            for ch in &node.children {
                let ch_prefix = if is_seq {
                    None
                } else {
                    Some(format!("{} => ", format_php_array_key(&ch.label)))
                };
                format_node_cli(ch, depth + 2, ch_prefix, true, out);
            }
            out.push(CliLine {
                indent_depth: depth + 1,
                spans: vec![(CliTokenKind::ObjectHandle, "],".to_string())],
            });
        }

        out.push(CliLine {
            indent_depth: depth,
            spans: vec![(CliTokenKind::ObjectHandle, format!("}}{comma}"))],
        });
        return;
    }

    if let Some(ref fqcn) = node.class_name {
        if node.type_name == "DateTime" {
            let oid_open = match node.object_id {
                Some(oid) => format!(" {{#{oid}"),
                None => " {".to_string(),
            };
            let mut open_spans = Vec::new();
            if let Some(p) = prefix {
                open_spans.push((CliTokenKind::Key, p));
            }
            open_spans.push((CliTokenKind::ClassName, fqcn.clone()));
            open_spans.push((CliTokenKind::ObjectHandle, oid_open));
            out.push(CliLine {
                indent_depth: depth,
                spans: open_spans,
            });
            out.push(CliLine {
                indent_depth: depth + 1,
                spans: vec![
                    (CliTokenKind::Key, "date: ".to_string()),
                    (
                        CliTokenKind::String,
                        format!("\"{}\"", escape_cli_str(&node.summary)),
                    ),
                    (CliTokenKind::ObjectHandle, ",".to_string()),
                ],
            });
            out.push(CliLine {
                indent_depth: depth,
                spans: vec![(CliTokenKind::ObjectHandle, format!("}}{comma}"))],
            });
            return;
        }

        let visible_children: Vec<&ObjectNode> = node
            .children
            .iter()
            .filter(|c| c.visibility.as_deref() != Some("graph_rel"))
            .collect();

        if visible_children.is_empty() {
            let handle = match node.object_id {
                Some(oid) => format!(" {{#{oid}}}{comma}"),
                None => format!(" {{}}{comma}"),
            };
            let mut spans = Vec::new();
            if let Some(p) = prefix {
                spans.push((CliTokenKind::Key, p));
            }
            spans.push((CliTokenKind::ClassName, fqcn.clone()));
            spans.push((CliTokenKind::ObjectHandle, handle));
            out.push(CliLine {
                indent_depth: depth,
                spans,
            });
            return;
        }

        let oid_open = match node.object_id {
            Some(oid) => format!(" {{#{oid}"),
            None => " {".to_string(),
        };
        let mut open_spans = Vec::new();
        if let Some(p) = prefix {
            open_spans.push((CliTokenKind::Key, p));
        }
        open_spans.push((CliTokenKind::ClassName, fqcn.clone()));
        open_spans.push((CliTokenKind::ObjectHandle, oid_open));
        out.push(CliLine {
            indent_depth: depth,
            spans: open_spans,
        });

        let is_model = node.type_name == "Model";
        for ch in visible_children {
            let vis_prefix = if is_model {
                ""
            } else {
                match ch.visibility.as_deref() {
                    Some("protected") => "#",
                    Some("private") => "-",
                    Some("public") => "+",
                    _ => "",
                }
            };
            let prop_prefix = format!("{vis_prefix}{}: ", ch.label);
            format_node_cli(ch, depth + 1, Some(prop_prefix), true, out);
        }

        out.push(CliLine {
            indent_depth: depth,
            spans: vec![(CliTokenKind::ObjectHandle, format!("}}{comma}"))],
        });
        return;
    }

    if node.type_name.starts_with("array:") {
        if node.children.is_empty() {
            let mut spans = Vec::new();
            if let Some(p) = prefix {
                spans.push((CliTokenKind::Key, p));
            }
            spans.push((CliTokenKind::ObjectHandle, format!("[]{comma}")));
            out.push(CliLine {
                indent_depth: depth,
                spans,
            });
            return;
        }

        let mut open_spans = Vec::new();
        if let Some(p) = prefix {
            open_spans.push((CliTokenKind::Key, p));
        }
        open_spans.push((CliTokenKind::ObjectHandle, "[".to_string()));
        out.push(CliLine {
            indent_depth: depth,
            spans: open_spans,
        });

        let is_seq = node
            .children
            .iter()
            .enumerate()
            .all(|(i, c)| c.label == i.to_string());
        for ch in &node.children {
            if ch.type_name == "truncated" {
                out.push(CliLine {
                    indent_depth: depth + 1,
                    spans: vec![(
                        CliTokenKind::ObjectHandle,
                        format!("// ... {}", ch.summary),
                    )],
                });
                continue;
            }
            let ch_prefix = if is_seq {
                None
            } else {
                Some(format!("{} => ", format_php_array_key(&ch.label)))
            };
            format_node_cli(ch, depth + 1, ch_prefix, true, out);
        }

        out.push(CliLine {
            indent_depth: depth,
            spans: vec![(CliTokenKind::ObjectHandle, format!("]{comma}"))],
        });
        return;
    }

    let raw_val = node.scalar_value.as_deref().unwrap_or(&node.summary);
    if node.type_name.starts_with("string") {
        let unquoted = raw_val
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(raw_val);
        if unquoted.contains('\n') {
            let mut open_spans = Vec::new();
            if let Some(p) = prefix {
                open_spans.push((CliTokenKind::Key, p));
            }
            open_spans.push((CliTokenKind::String, "\"\"\"".to_string()));
            out.push(CliLine {
                indent_depth: depth,
                spans: open_spans,
            });

            for seg in unquoted.split('\n') {
                let clean_seg = escape_cli_multiline_line(seg.trim_end_matches('\r'));
                let spans = if clean_seg.is_empty() {
                    Vec::new()
                } else {
                    vec![(CliTokenKind::String, clean_seg)]
                };
                out.push(CliLine {
                    indent_depth: depth + 1,
                    spans,
                });
            }

            let mut close_spans = vec![(CliTokenKind::String, "\"\"\"".to_string())];
            if trailing_comma {
                close_spans.push((CliTokenKind::ObjectHandle, ",".to_string()));
            }
            out.push(CliLine {
                indent_depth: depth,
                spans: close_spans,
            });
            return;
        }
    } else if !matches!(node.type_name.as_str(), "int" | "float" | "bool" | "null")
        && raw_val.contains('\n')
    {
        let segs: Vec<&str> = raw_val.split('\n').collect();
        for (idx, seg) in segs.iter().enumerate() {
            let clean_seg = escape_cli_multiline_line(seg.trim_end_matches('\r'));
            let is_first = idx == 0;
            let is_last = idx + 1 == segs.len();
            let mut spans = Vec::new();
            if is_first && let Some(ref p) = prefix {
                spans.push((CliTokenKind::Key, p.clone()));
            }
            if !clean_seg.is_empty() {
                spans.push((CliTokenKind::Plain, clean_seg));
            }
            if is_last && trailing_comma {
                spans.push((CliTokenKind::ObjectHandle, ",".to_string()));
            }
            out.push(CliLine {
                indent_depth: if is_first { depth } else { depth + 1 },
                spans,
            });
        }
        return;
    }

    let (tok_kind, formatted_val) = match node.type_name.as_str() {
        "int" | "float" => (CliTokenKind::Number, raw_val.to_string()),
        "bool" | "null" => (CliTokenKind::BoolOrNull, raw_val.to_string()),
        t if t.starts_with("string") => {
            let unquoted = raw_val
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(raw_val);
            (
                CliTokenKind::String,
                format!("\"{}\"", escape_cli_str(unquoted)),
            )
        }
        _ => (CliTokenKind::Plain, raw_val.to_string()),
    };

    let mut spans = Vec::new();
    if let Some(p) = prefix {
        spans.push((CliTokenKind::Key, p));
    }
    spans.push((tok_kind, formatted_val));
    if trailing_comma {
        spans.push((CliTokenKind::ObjectHandle, ",".to_string()));
    }
    out.push(CliLine {
        indent_depth: depth,
        spans,
    });
}

fn format_php_array_key(key: &str) -> String {
    if key.parse::<i64>().is_ok() {
        key.to_string()
    } else {
        format!("\"{}\"", escape_cli_str(key))
    }
}

fn escape_cli_multiline_line(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("    "),
            c if c.is_control() => {
                for esc in c.escape_default() {
                    out.push(esc);
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn escape_cli_str(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Extracts a `TableDataset` from any `ObjectNode` representing a non-empty Collection or array
/// of Eloquent Models, objects, or associative rows (so every Detail Dive card can open Table Preview).
pub fn extract_table_from_node(node: &ObjectNode, source_label: &str) -> Option<TableDataset> {
    let is_collection = node.type_name.starts_with("Collection:")
        || node
            .class_name
            .as_deref()
            .is_some_and(|c| c.ends_with("\\Collection") || c == "Collection");
    let is_array = node.type_name.starts_with("array:");
    if !is_collection && !is_array {
        return None;
    }

    let items: Vec<&ObjectNode> = node
        .children
        .iter()
        .filter(|c| c.type_name != "truncated")
        .collect();
    if items.is_empty() {
        return None;
    }

    // Check if items are structured rows (Models, objects, or associative arrays)
    let all_structured = items.iter().all(|it| !it.children.is_empty());
    if all_structured {
        let mut columns: Vec<String> = Vec::new();
        let mut has_named_column = false;
        for item in &items {
            if item.class_name.is_some() {
                has_named_column = true;
            }
            for ch in &item.children {
                if ch.type_name == "truncated" || ch.visibility.as_deref() == Some("graph_rel") {
                    continue;
                }
                if ch.label.parse::<usize>().is_err() {
                    has_named_column = true;
                }
                if !columns.contains(&ch.label) {
                    columns.push(ch.label.clone());
                }
            }
        }
        if !has_named_column || columns.is_empty() {
            return None;
        }

        let mut rows = Vec::with_capacity(items.len());
        let mut raw_rows_json = Vec::with_capacity(items.len());
        let mut raw_rows_php = Vec::with_capacity(items.len());

        for item in &items {
            let mut row_cells = Vec::with_capacity(columns.len());
            let mut json_map = serde_json::Map::new();
            let mut php_pairs = Vec::with_capacity(columns.len());

            for col in &columns {
                if let Some(ch) = item.children.iter().find(|c| &c.label == col) {
                    let cell = node_cell_display(ch);
                    row_cells.push(cell);
                    json_map.insert(col.clone(), node_to_json_val(ch));
                    php_pairs.push(format!(
                        "'{}' => {}",
                        col.replace('\'', "\\'"),
                        node_to_php_literal(ch)
                    ));
                } else {
                    row_cells.push(String::new());
                }
            }

            rows.push(row_cells);
            raw_rows_json.push(
                serde_json::to_string_pretty(&serde_json::Value::Object(json_map))
                    .unwrap_or_else(|_| "{}".to_string()),
            );
            raw_rows_php.push(format!("[{}]", php_pairs.join(", ")));
        }

        return Some(TableDataset {
            source_label: source_label.to_string(),
            columns,
            rows,
            raw_rows_json,
            raw_rows_php,
        });
    }

    None
}

pub fn extract_first_table_from_result(res: &EvaluationResult) -> Option<TableDataset> {
    if let Some(ref tbl) = res.table {
        return Some(tbl.clone());
    }
    if let Some(ref ret) = res.return_value
        && let Some(tbl) = extract_table_from_node(&ret.node, &format!("Line {}", ret.line))
    {
        return Some(tbl);
    }
    for dump in &res.dumps {
        if let Some(tbl) = extract_table_from_node(&dump.node, &format!("Line {}", dump.line)) {
            return Some(tbl);
        }
    }
    None
}

fn node_cell_display(node: &ObjectNode) -> String {
    if node.type_name == "null" {
        return String::new();
    }
    if node.type_name.starts_with("string") {
        let raw = node.scalar_value.as_deref().unwrap_or(&node.summary);
        return raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(raw)
            .to_string();
    }
    if matches!(node.type_name.as_str(), "int" | "float" | "bool") {
        return node
            .scalar_value
            .as_deref()
            .unwrap_or(&node.summary)
            .to_string();
    }
    node.summary.clone()
}

fn node_to_json_val(node: &ObjectNode) -> serde_json::Value {
    match node.type_name.as_str() {
        "null" => serde_json::Value::Null,
        "bool" => serde_json::Value::Bool(
            node.scalar_value.as_deref().unwrap_or(&node.summary) == "true",
        ),
        "int" => node
            .scalar_value
            .as_deref()
            .unwrap_or(&node.summary)
            .parse::<i64>()
            .map(|n| serde_json::Value::Number(n.into()))
            .unwrap_or_else(|_| serde_json::Value::String(node.summary.clone())),
        "float" => node
            .scalar_value
            .as_deref()
            .unwrap_or(&node.summary)
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| serde_json::Value::String(node.summary.clone())),
        t if t.starts_with("string") => serde_json::Value::String(node_cell_display(node)),
        _ => serde_json::Value::String(node.summary.clone()),
    }
}

fn node_to_php_literal(node: &ObjectNode) -> String {
    match node.type_name.as_str() {
        "null" => "null".to_string(),
        "bool" | "int" | "float" => node
            .scalar_value
            .as_deref()
            .unwrap_or(&node.summary)
            .to_string(),
        t if t.starts_with("string") => {
            format!("'{}'", node_cell_display(node).replace('\'', "\\'"))
        }
        _ => format!("'{}'", node.summary.replace('\'', "\\'")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorDiagnostic {
    pub line: usize,
    pub message: String,
    pub token_hint: Option<String>,
}

pub fn diagnostic_from_exception(exc: &ExceptionData, total_lines: usize) -> EditorDiagnostic {
    let max_line = total_lines.max(1);
    let line = if exc.line == 0 {
        max_line
    } else {
        exc.line.min(max_line)
    };

    let short_cls = exc.class.rsplit('\\').next().unwrap_or(&exc.class);
    let message = if exc.message.starts_with("PHP Parse error:") {
        exc.message.clone()
    } else if short_cls == "ParseError" {
        let mut msg = exc.message.trim().to_string();
        if let Some(rest) = msg.strip_prefix("syntax error") {
            msg = format!("Syntax error{rest}");
        }
        if msg.contains(" on line ") {
            format!("PHP Parse error: {msg}")
        } else {
            format!("PHP Parse error: {msg} on line {line}")
        }
    } else {
        format!("{short_cls}: {}", exc.message.trim())
    };

    let token_hint = extract_error_token_hint(&message);
    EditorDiagnostic {
        line,
        message,
        token_hint,
    }
}

pub fn extract_error_token_hint(msg: &str) -> Option<String> {
    for marker in ["unexpected token \"", "unexpected \""] {
        if let Some(idx) = msg.find(marker) {
            let rest = &msg[idx + marker.len()..];
            if let Some(end) = rest.find('"') {
                let tok = &rest[..end];
                if !tok.is_empty() {
                    return Some(tok.to_string());
                }
            }
        }
    }
    if let Some(idx) = msg.find("unexpected '") {
        let rest = &msg[idx + "unexpected '".len()..];
        if let Some(end) = rest.find('\'') {
            let tok = &rest[..end];
            if !tok.is_empty() {
                return Some(tok.to_string());
            }
        }
    }
    if let Some(idx) = msg.find("Undefined variable $") {
        let rest = &msg[idx + "Undefined variable ".len()..];
        let tok: String = rest
            .chars()
            .take_while(|c| *c == '$' || c.is_alphanumeric() || *c == '_')
            .collect();
        if tok.len() > 1 {
            return Some(tok);
        }
    }
    if let Some(idx) = msg.find("undefined method ") {
        let rest = &msg[idx + "undefined method ".len()..];
        if let Some(cc) = rest.find("::") {
            let after = &rest[cc + 2..];
            let m: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !m.is_empty() {
                return Some(m);
            }
        }
    }
    None
}

pub fn extract_first_html_from_result(res: &EvaluationResult) -> Option<String> {
    if let Some(ref ret) = res.return_value
        && let Some(ref html) = ret.node.html_preview
    {
        return Some(html.clone());
    }
    if let Some(ref mail) = res.mailable {
        return Some(mail.html.clone());
    }
    for dump in &res.dumps {
        if let Some(ref html) = dump.node.html_preview {
            return Some(html.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_eval_result() -> EvaluationResult {
        EvaluationResult {
            status: "ok".to_string(),
            duration_ms: 1.0,
            memory_mb: 1.0,
            php_version: "8.3.0".to_string(),
            laravel_version: "11.0.0".to_string(),
            driver_name: "Laravel".to_string(),
            driver_vars: Vec::new(),
            raw_output: String::new(),
            dumps: Vec::new(),
            return_value: None,
            sql_queries: Vec::new(),
            mailable: None,
            table: None,
            exception: None,
        }
    }

    #[test]
    fn formats_multiline_model_string_into_distinct_cli_lines() {
        let multiline = "KBZ\nName: Htain Lin Shwe\nAccount No: 07830107800882101\n\nKBZ Pay\nName: Htain Lin Shwe\nPhone: 09974274616";
        let model = ObjectNode {
            id: "n1".to_string(),
            label: "return".to_string(),
            type_name: "Model".to_string(),
            class_name: Some("App\\Models\\Invoice".to_string()),
            object_id: Some(42),
            visibility: None,
            summary: "App\\Models\\Invoice {#42}".to_string(),
            scalar_value: None,
            children: vec![
                ObjectNode {
                    id: "n2".to_string(),
                    label: "bank_account_info".to_string(),
                    type_name: format!("string({})", multiline.len()),
                    class_name: None,
                    object_id: None,
                    visibility: Some("public".to_string()),
                    summary: multiline.to_string(),
                    scalar_value: Some(format!("\"{multiline}\"")),
                    children: Vec::new(),
                    expanded_by_default: false,
                    html_preview: None,
                },
                ObjectNode {
                    id: "n3".to_string(),
                    label: "created_at".to_string(),
                    type_name: "string(19)".to_string(),
                    class_name: None,
                    object_id: None,
                    visibility: Some("public".to_string()),
                    summary: "2026-11-11 23:32:04".to_string(),
                    scalar_value: Some("\"2026-11-11 23:32:04\"".to_string()),
                    children: Vec::new(),
                    expanded_by_default: false,
                    html_preview: None,
                },
            ],
            expanded_by_default: true,
            html_preview: None,
        };

        let mut res = empty_eval_result();
        res.return_value = Some(DumpCard {
            id: "ret".to_string(),
            line: 1,
            file: String::new(),
            kind: "return".to_string(),
            node: model,
        });

        let lines = format_cli_lines(&res);
        for line in &lines {
            for (_, seg) in &line.spans {
                assert!(
                    !seg.contains('\n') && !seg.contains('\r'),
                    "CLI span should not contain raw line breaks: {seg:?}"
                );
            }
        }

        let text = format_cli_text(&res);
        let expected = "\
App\\Models\\Invoice {#42
  bank_account_info: \"\"\"
    KBZ
    Name: Htain Lin Shwe
    Account No: 07830107800882101

    KBZ Pay
    Name: Htain Lin Shwe
    Phone: 09974274616
  \"\"\",
  created_at: \"2026-11-11 23:32:04\",
}";
        assert_eq!(text, expected);
    }

    #[test]
    fn splits_crlf_and_multiline_exceptions_in_cli_lines() {
        let mut res = empty_eval_result();
        res.raw_output = "first\r\nsecond\r\n".to_string();
        res.exception = Some(ExceptionData {
            class: "RuntimeException".to_string(),
            message: "Something failed\r\nDetails on line 2".to_string(),
            line: 10,
            file: "app/Services/Pay.php".to_string(),
            trace: vec!["#0 foo()\n#1 bar()".to_string()],
        });

        let lines = format_cli_lines(&res);
        for line in &lines {
            for (_, seg) in &line.spans {
                assert!(!seg.contains('\n') && !seg.contains('\r'));
            }
        }

        let text = format_cli_text(&res);
        assert!(text.contains("first\nsecond\n\nRuntimeException  Something failed\n  Details on line 2\n  at app/Services/Pay.php:10\n  #0 foo()\n  #1 bar()"));
    }
}
