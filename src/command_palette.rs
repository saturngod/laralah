use crate::models::{ExecutionTarget, OutputMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteItemKind {
    Project,
    Target,
    Command,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    OpenDirectoryDialog,
    ShowLandingPage,
    OpenSettings,
    OpenAiSettings,
    ToggleAiSidebar,
    OpenSshManager,
    SwitchProject(String),
    SwitchTarget(ExecutionTarget),
    RunCode,
    ToggleAutoRun,
    SwitchOutputMode(OutputMode),
    OpenLogViewer,
    NewTab,
    CloseActiveTab,
    ClearOutput,
    ExportTableCsv,
    ExportTableJson,
}

#[derive(Clone, Debug)]
pub struct PaletteEntry {
    pub kind: PaletteItemKind,
    pub prefix_tag: &'static str,
    pub title: String,
    pub subtitle: String,
    pub shortcut: &'static str,
    pub action: PaletteAction,
}

pub fn filter_palette_entries(
    query: &str,
    projects: &[String],
    targets: &[ExecutionTarget],
) -> Vec<PaletteEntry> {
    let trimmed = query.trim();

    if let Some(project_q) = trimmed.strip_prefix('/') {
        let mut items: Vec<(i32, PaletteEntry)> = projects
            .iter()
            .filter_map(|path| {
                let name = std::path::Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path);
                let score = score_item(project_q, name, path)?;
                Some((
                    score,
                    PaletteEntry {
                        kind: PaletteItemKind::Project,
                        prefix_tag: "/",
                        title: name.to_string(),
                        subtitle: path.clone(),
                        shortcut: "Open Project",
                        action: PaletteAction::SwitchProject(path.clone()),
                    },
                ))
            })
            .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0));
        let mut out: Vec<PaletteEntry> = items.into_iter().map(|(_, e)| e).collect();
        if project_q.trim().is_empty() {
            out.insert(
                0,
                PaletteEntry {
                    kind: PaletteItemKind::Project,
                    prefix_tag: "/",
                    title: "Open Local Directory...".to_string(),
                    subtitle: "Browse for a Laravel folder in macOS Finder".to_string(),
                    shortcut: "Cmd+O",
                    action: PaletteAction::OpenDirectoryDialog,
                },
            );
        }
        return out;
    }

    if let Some(target_q) = trimmed.strip_prefix('@') {
        let mut items: Vec<(i32, PaletteEntry)> = targets
            .iter()
            .filter_map(|t| {
                let sub = match t {
                    ExecutionTarget::Local { project_path } => {
                        format!("Local PHP CLI · {project_path}")
                    }
                    ExecutionTarget::Docker {
                        image, workdir, ..
                    } => format!("Docker container · {image} · {workdir}"),
                    ExecutionTarget::Ssh {
                        local_docker: true,
                        workdir,
                        docker_container,
                        ..
                    } => format!(
                        "Local Docker · {} · {workdir}",
                        docker_container.as_deref().unwrap_or("")
                    ),
                    ExecutionTarget::Ssh {
                        user,
                        host,
                        port,
                        workdir,
                        docker_container,
                        is_production,
                        ..
                    } => {
                        let safety = if *is_production {
                            "Auto-Run Disabled"
                        } else {
                            "Staging"
                        };
                        match docker_container.as_deref().filter(|s| !s.is_empty()) {
                            Some(container) => {
                                format!(
                                    "{user}@{host}:{port} · docker: {container} · {workdir} · {safety}"
                                )
                            }
                            None => format!("{user}@{host}:{port} · {workdir} · {safety}"),
                        }
                    }
                };
                let title = t.label();
                let score = score_item(target_q, &title, &sub)?;
                Some((
                    score,
                    PaletteEntry {
                        kind: PaletteItemKind::Target,
                        prefix_tag: "@",
                        title: t.label(),
                        subtitle: sub,
                        shortcut: t.kind_badge(),
                        action: PaletteAction::SwitchTarget(t.clone()),
                    },
                ))
            })
            .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0));
        return items.into_iter().map(|(_, e)| e).collect();
    }

    // Unprefixed: Commands + optional matching projects/targets when user types a query
    let entries = vec![
        PaletteEntry {
            kind: PaletteItemKind::Project,
            prefix_tag: "/",
            title: "Open Local Directory...".to_string(),
            subtitle: "Choose a Laravel folder from macOS Finder".to_string(),
            shortcut: "Cmd+O",
            action: PaletteAction::OpenDirectoryDialog,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: "@",
            title: "Manage SSH Connections...".to_string(),
            subtitle: "Create, edit, group, and connect to remote Laravel servers via SSH".to_string(),
            shortcut: "Cmd+K",
            action: PaletteAction::OpenSshManager,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Toggle AI Assistant Sidebar".to_string(),
            subtitle: "Expand or collapse the right-side AI coding assistant".to_string(),
            shortcut: "Cmd+I",
            action: PaletteAction::ToggleAiSidebar,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "AI Assistant Settings...".to_string(),
            subtitle: "Enable or disable AI CLI agents (Codex, Claude, Agy, Qoder, OpenCode)".to_string(),
            shortcut: "AI",
            action: PaletteAction::OpenAiSettings,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Open Settings".to_string(),
            subtitle: "Configure PHP binary, layout, editor font, and advanced options".to_string(),
            shortcut: "Cmd+,",
            action: PaletteAction::OpenSettings,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Welcome / Get Started".to_string(),
            subtitle: "Return to the Get Started landing screen".to_string(),
            shortcut: "Home",
            action: PaletteAction::ShowLandingPage,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Run Laravel Code".to_string(),
            subtitle: "Execute active scratchpad tab in current Laravel environment".to_string(),
            shortcut: "Cmd+R",
            action: PaletteAction::RunCode,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Toggle Auto-Run Evaluation".to_string(),
            subtitle: "Automatically re-evaluate code as you type (disabled on SSH)".to_string(),
            shortcut: "Auto",
            action: PaletteAction::ToggleAutoRun,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Open Laravel Log Viewer".to_string(),
            subtitle: "Inspect, filter, and live-tail storage/logs/*.log".to_string(),
            shortcut: "Cmd+L",
            action: PaletteAction::OpenLogViewer,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "View: Detail Dive (Object Graph)".to_string(),
            subtitle: "Switch output panel to interactive Detail Dive cards".to_string(),
            shortcut: "Cmd+1",
            action: PaletteAction::SwitchOutputMode(OutputMode::DetailDive),
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "View: Table Mode".to_string(),
            subtitle: "Switch output panel to searchable, sortable Table Mode".to_string(),
            shortcut: "Cmd+2",
            action: PaletteAction::SwitchOutputMode(OutputMode::TableMode),
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Toggle CLI Output".to_string(),
            subtitle: "Toggle between Detail Dive / Table and PsySH-style CLI result".to_string(),
            shortcut: "Cmd+3",
            action: PaletteAction::SwitchOutputMode(OutputMode::RawCli),
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "View: SQL Queries".to_string(),
            subtitle: "Inspect DB::listen() SQL queries, bindings, and N+1 duplicates".to_string(),
            shortcut: "Cmd+S",
            action: PaletteAction::SwitchOutputMode(OutputMode::SqlQueries),
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "View: Mailable Preview".to_string(),
            subtitle: "Preview rendered Laravel Mailable HTML and envelope headers".to_string(),
            shortcut: "Mail",
            action: PaletteAction::SwitchOutputMode(OutputMode::MailablePreview),
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Export Table as CSV".to_string(),
            subtitle: "Save active Table dataset as a CSV file and copy to clipboard".to_string(),
            shortcut: "CSV",
            action: PaletteAction::ExportTableCsv,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Export Table as JSON".to_string(),
            subtitle: "Save active Table dataset as a JSON file and copy to clipboard".to_string(),
            shortcut: "JSON",
            action: PaletteAction::ExportTableJson,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "New Scratchpad Tab".to_string(),
            subtitle: "Open a new Laravel REPL tab".to_string(),
            shortcut: "Cmd+T",
            action: PaletteAction::NewTab,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Close Active Tab".to_string(),
            subtitle: "Close the currently active scratchpad tab".to_string(),
            shortcut: "Tab",
            action: PaletteAction::CloseActiveTab,
        },
        PaletteEntry {
            kind: PaletteItemKind::Command,
            prefix_tag: ">",
            title: "Clear Evaluation Output".to_string(),
            subtitle: "Reset dumps, SQL log, and output cards for active tab".to_string(),
            shortcut: "Clear",
            action: PaletteAction::ClearOutput,
        },
    ];

    if trimmed.is_empty() {
        return entries;
    }

    let mut scored: Vec<(i32, PaletteEntry)> = entries
        .into_iter()
        .filter_map(|e| {
            let score = score_item(trimmed, &e.title, &e.subtitle)?;
            Some((score, e))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, e)| e).collect()
}

pub fn score_item(query: &str, title: &str, subtitle: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(1);
    }
    let t = title.to_lowercase();
    let s = subtitle.to_lowercase();

    if let Some(pos) = t.find(&q) {
        return Some(2000 - pos as i32);
    }
    if let Some(pos) = s.find(&q) {
        return Some(1200 - pos as i32);
    }
    // Only use subsequence matching on the short title, never noisy path subtitles
    fuzzy_score(&q, &t)
}

pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(1);
    }
    let c = candidate.to_lowercase();

    if let Some(pos) = c.find(&q) {
        return Some(1000 - pos as i32);
    }

    // Word-boundary-aware subsequence match: consecutive matches or matches at token boundaries
    let chars: Vec<char> = c.chars().collect();
    let mut score = 0i32;
    let mut prev_idx: Option<usize> = None;
    let mut search_from = 0usize;

    for qch in q.chars() {
        let mut found = None;
        for (idx, &ch) in chars.iter().enumerate().skip(search_from) {
            if ch != qch {
                continue;
            }
            let is_consecutive = prev_idx == Some(idx.saturating_sub(1)) && prev_idx.is_some();
            let is_word_start = idx == 0
                || matches!(
                    chars[idx - 1],
                    ' ' | '-' | '_' | '/' | '.' | ':' | '(' | '[' | '@'
                );
            if is_consecutive || is_word_start {
                found = Some((idx, is_consecutive));
                break;
            }
        }
        let (idx, is_consecutive) = found?;
        score += if is_consecutive { 15 } else { 8 };
        prev_idx = Some(idx);
        search_from = idx + 1;
    }

    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_slash_prefix_to_projects_and_at_prefix_to_targets() {
        let projects = vec![
            "/Users/dev/laravel-playground".to_string(),
            "/Users/dev/billing-laravel".to_string(),
        ];
        let targets = vec![
            ExecutionTarget::Local {
                project_path: projects[0].clone(),
            },
            ExecutionTarget::Ssh {
                id: "t1".to_string(),
                local_docker: false,
                name: "prod".to_string(),
                host: "prod.example.com".to_string(),
                port: 22,
                user: "forge".to_string(),
                workdir: "/home/forge/app".to_string(),
                docker_container: None,
                is_production: true,
                local_path: None,
                auth_method: crate::ssh::SshAuthMethod::PrivateKey,
                private_key: "~/.ssh/id_rsa".to_string(),
                passphrase: String::new(),
                color_hex: 0xef4444,
            },
        ];

        let proj_hits = filter_palette_entries("/billing", &projects, &targets);
        assert_eq!(proj_hits.len(), 1);
        assert_eq!(proj_hits[0].kind, PaletteItemKind::Project);

        let target_hits = filter_palette_entries("@prod", &projects, &targets);
        assert_eq!(target_hits.len(), 1);
        assert_eq!(target_hits[0].kind, PaletteItemKind::Target);

        let cmd_hits = filter_palette_entries("mailable", &projects, &targets);
        assert!(!cmd_hits.is_empty());
        assert!(cmd_hits
            .iter()
            .any(|e| e.title.contains("Mailable")));
    }
}
