use crate::keymap::{ShortcutSpec, shortcut_specs};
use crate::runner::resolve_php_binary;
pub use crate::theme::ThemeMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingsTab {
    General,
    Editor,
    Ai,
    Advanced,
    Shortcuts,
}

impl SettingsTab {
    pub fn all() -> [SettingsTab; 5] {
        [
            SettingsTab::General,
            SettingsTab::Editor,
            SettingsTab::Ai,
            SettingsTab::Advanced,
            SettingsTab::Shortcuts,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Editor => "Editor",
            SettingsTab::Ai => "AI Assistant",
            SettingsTab::Advanced => "Advanced",
            SettingsTab::Shortcuts => "Shortcuts",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaneLayout {
    Vertical,
    Horizontal,
}

impl PaneLayout {
    pub fn label(&self) -> &'static str {
        match self {
            PaneLayout::Vertical => "Vertical",
            PaneLayout::Horizontal => "Horizontal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvaluateScope {
    SelectedIfAvailable,
    FullBuffer,
}

impl EvaluateScope {
    pub fn label(&self) -> &'static str {
        match self {
            EvaluateScope::SelectedIfAvailable => "Evaluate selected code, if available",
            EvaluateScope::FullBuffer => "Always evaluate full buffer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefaultOutputSetting {
    DetailDive,
    CliOutput,
}

impl DefaultOutputSetting {
    pub fn label(&self) -> &'static str {
        match self {
            DefaultOutputSetting::DetailDive => "Detail Dive",
            DefaultOutputSetting::CliOutput => "CLI Output",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetailDiveExpansion {
    FirstLevel,
    CollapseAll,
    ExpandAll,
}

impl DetailDiveExpansion {
    pub fn label(&self) -> &'static str {
        match self {
            DetailDiveExpansion::FirstLevel => "Expand the first level",
            DetailDiveExpansion::CollapseAll => "Collapse all",
            DetailDiveExpansion::ExpandAll => "Expand all",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    // General -> General
    pub php_binary: String,
    pub ask_before_closing_tab: bool,
    pub show_welcome_tab: bool,

    // General -> Appearance
    pub theme: ThemeMode,
    pub layout: PaneLayout,
    pub split_ratio: f32,
    pub stack_ratio: f32,
    pub sidebar_width: f32,
    pub show_toolbar: bool,
    pub show_explorer: bool,
    pub always_on_top: bool,
    pub auto_hide_output: bool,

    // General -> Behaviour
    pub auto_evaluate_code: bool,
    pub evaluate_scope: EvaluateScope,
    pub auto_log: bool,
    pub buffered_output: bool,
    pub default_output: DefaultOutputSetting,
    pub detail_dive_expansion: DetailDiveExpansion,
    pub show_execution_metrics: bool,
    pub show_sql_inline: bool,

    // Editor -> Editor
    pub show_line_numbers: bool,
    pub line_wrap: bool,
    pub indentation_size: usize,
    pub show_inline_errors: bool,
    pub use_vim_keymap: bool,
    pub history_entries: usize,

    // Editor -> Font
    pub font_family: String,
    pub font_size: f32,
    pub line_height_ratio: f32,
    pub use_font_ligatures: bool,

    // AI Assistant (Right sidebar & CLI agents)
    pub ai_enabled: bool,
    pub show_ai_sidebar: bool,
    pub ai_sidebar_width: f32,
    pub ai_auto_write_editor: bool,
    pub ai_include_editor_context: bool,
    pub ai_allow_ssh_schema: bool,
    pub ai_selected_agent: String,
    pub ai_selected_models: HashMap<String, String>,
    pub ai_selected_efforts: HashMap<String, String>,
    pub ai_enabled_agents: HashMap<String, bool>,

    // Advanced
    pub default_working_directory: Option<String>,
    pub collision_enabled: bool,
    pub autocompletion_service_port: u16,

    // Shortcuts: shortcut id -> keystrokes (gpui syntax). Missing ids use the
    // defaults; an empty list means the shortcut is unbound.
    pub custom_shortcuts: HashMap<String, Vec<String>>,
}

pub fn default_ai_enabled_agents() -> HashMap<String, bool> {
    let mut map = HashMap::new();
    for id in ["codex", "claude_code", "agy", "qoder", "opencode"] {
        map.insert(id.to_string(), true);
    }
    map
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            php_binary: resolve_php_binary(),
            ask_before_closing_tab: false,
            show_welcome_tab: true,

            theme: ThemeMode::System,
            layout: PaneLayout::Vertical,
            split_ratio: 0.5,
            stack_ratio: 0.5,
            sidebar_width: 216.0,
            show_toolbar: true,
            show_explorer: true,
            always_on_top: false,
            auto_hide_output: false,

            auto_evaluate_code: false,
            evaluate_scope: EvaluateScope::SelectedIfAvailable,
            auto_log: false,
            buffered_output: true,
            default_output: DefaultOutputSetting::DetailDive,
            detail_dive_expansion: DetailDiveExpansion::FirstLevel,
            show_execution_metrics: true,
            show_sql_inline: true,

            show_line_numbers: true,
            line_wrap: true,
            indentation_size: 4,
            show_inline_errors: true,
            use_vim_keymap: false,
            history_entries: 150,

            font_family: "Menlo".to_string(),
            font_size: 13.5,
            line_height_ratio: 1.65,
            use_font_ligatures: true,

            ai_enabled: true,
            show_ai_sidebar: true,
            ai_sidebar_width: 340.0,
            ai_auto_write_editor: true,
            ai_include_editor_context: false,
            ai_allow_ssh_schema: false,
            ai_selected_agent: "claude_code".to_string(),
            ai_selected_models: HashMap::new(),
            ai_selected_efforts: HashMap::new(),
            ai_enabled_agents: default_ai_enabled_agents(),

            default_working_directory: None,
            collision_enabled: true,
            autocompletion_service_port: 3050,

            custom_shortcuts: HashMap::new(),
        }
    }
}

pub fn settings_file_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let local = cwd.join(".laralah_settings.json");
    if local.exists() || cwd.join("Cargo.toml").exists() {
        return local;
    }
    if let Some(home) = crate::paths::home_dir()
        && home.is_dir()
    {
        return home.join(".laralah_settings.json");
    }
    local
}

pub fn load_settings() -> AppSettings {
    let path = settings_file_path();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return AppSettings::default();
    };
    let mut s = serde_json::from_str::<AppSettings>(&raw).unwrap_or_default();
    if !(0.15..=0.85).contains(&s.split_ratio) {
        s.split_ratio = 0.5;
    }
    if !(0.15..=0.85).contains(&s.stack_ratio) {
        s.stack_ratio = 0.5;
    }
    if !(150.0..=480.0).contains(&s.sidebar_width) {
        s.sidebar_width = 216.0;
    }
    if !(280.0..=580.0).contains(&s.ai_sidebar_width) {
        s.ai_sidebar_width = 340.0;
    }
    for (id, def_val) in default_ai_enabled_agents() {
        s.ai_enabled_agents.entry(id).or_insert(def_val);
    }
    // A saved PHP path can go stale (PHP uninstalled, Homebrew upgrade moved it).
    // Fall back to auto-detection instead of keeping a path that no longer exists.
    let saved_bin = s.php_binary.trim();
    if saved_bin.is_empty()
        || (std::path::Path::new(saved_bin).is_absolute()
            && !std::path::Path::new(saved_bin).is_file())
    {
        s.php_binary = resolve_php_binary();
    }
    s
}

/// True when `detect_php_version_line` successfully ran the PHP binary.
pub fn is_php_detected(version_line: &str) -> bool {
    version_line.starts_with("Detected version: ")
}

pub fn save_settings(settings: &AppSettings) {
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(settings_file_path(), json);
    }
}

pub fn detect_php_version_line(php_bin: &str) -> String {
    let bin = if php_bin.trim().is_empty() {
        resolve_php_binary()
    } else {
        php_bin.trim().to_string()
    };
    match Command::new(&bin).arg("-v").output() {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| format!("Detected version: {s}"))
                .unwrap_or_else(|| "Detected version: PHP CLI".to_string())
        }
        _ => format!("Unable to execute PHP binary at '{bin}'"),
    }
}

pub fn available_editor_fonts() -> &'static [&'static str] {
    &[
        "Menlo",
        "SF Mono",
        "JetBrains Mono",
        "Fira Code",
        "Monaco",
        "Courier New",
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKey {
    // General -> General
    PhpBinary,
    AskBeforeClosingTab,
    WelcomeTab,
    // General -> Appearance
    Theme,
    Layout,
    Toolbar,
    ProjectExplorer,
    AlwaysOnTop,
    AutoHideOutput,
    // General -> Behaviour
    AutoEvaluateCode,
    EvaluatingCode,
    AutoLog,
    OutputType,
    DefaultOutputType,
    AutoExpandDetailDive,
    ExecutionTimeMemory,
    // Editor -> Editor
    LineNumbers,
    LineWrap,
    IndentationSize,
    InlineErrorMessages,
    UseVimKeymap,
    HistoryEntries,
    // Editor -> Font
    FontFamily,
    FontSize,
    LineHeight,
    FontLigatures,
    // AI Assistant
    AiEnabled,
    AiAutoWrite,
    AiAllowSshSchema,
    AiAgentsList,
    // Advanced -> Advanced
    DefaultWorkingDirectory,
    CollisionEnabled,
    AutocompletionServicePort,
    RecentFolders,
}

#[derive(Clone, Copy, Debug)]
pub struct SettingMeta {
    pub key: SettingKey,
    pub tab: SettingsTab,
    pub section: &'static str,
    pub label: &'static str,
    pub keywords: &'static str,
}

pub fn all_setting_specs() -> &'static [SettingMeta] {
    &[
        // General -> General
        SettingMeta {
            key: SettingKey::PhpBinary,
            tab: SettingsTab::General,
            section: "General",
            label: "PHP binary",
            keywords: "php cli binary path executable interpreter version homebrew",
        },
        SettingMeta {
            key: SettingKey::AskBeforeClosingTab,
            tab: SettingsTab::General,
            section: "General",
            label: "Ask before closing tab",
            keywords: "confirm close scratchpad tab prompt warning",
        },
        SettingMeta {
            key: SettingKey::WelcomeTab,
            tab: SettingsTab::General,
            section: "General",
            label: "Welcome tab",
            keywords: "get started home landing screen startup welcome",
        },
        // General -> Appearance
        SettingMeta {
            key: SettingKey::Theme,
            tab: SettingsTab::General,
            section: "Appearance",
            label: "Theme",
            keywords: "theme appearance dark light system mode color scheme night",
        },
        SettingMeta {
            key: SettingKey::Layout,
            tab: SettingsTab::General,
            section: "Appearance",
            label: "Layout",
            keywords: "split pane vertical horizontal orientation side by side",
        },
        SettingMeta {
            key: SettingKey::Toolbar,
            tab: SettingsTab::General,
            section: "Appearance",
            label: "Navigation rail",
            keywords: "left rail icon bar sidebar navigation buttons show hide toolbar",
        },
        SettingMeta {
            key: SettingKey::ProjectExplorer,
            tab: SettingsTab::General,
            section: "Appearance",
            label: "Project explorer",
            keywords: "explorer sidebar models mailables workspaces panel cmd b show hide on off",
        },
        SettingMeta {
            key: SettingKey::AlwaysOnTop,
            tab: SettingsTab::General,
            section: "Appearance",
            label: "Always on top",
            keywords: "float pin window above top",
        },
        SettingMeta {
            key: SettingKey::AutoHideOutput,
            tab: SettingsTab::General,
            section: "Appearance",
            label: "Automatically hide output",
            keywords: "collapse output result panel empty hide",
        },
        // General -> Behaviour
        SettingMeta {
            key: SettingKey::AutoEvaluateCode,
            tab: SettingsTab::General,
            section: "Behaviour",
            label: "Auto evaluate code",
            keywords: "live run automatic execute typing autorun",
        },
        SettingMeta {
            key: SettingKey::EvaluatingCode,
            tab: SettingsTab::General,
            section: "Behaviour",
            label: "Evaluating code",
            keywords: "selection selected code full buffer evaluate scope",
        },
        SettingMeta {
            key: SettingKey::AutoLog,
            tab: SettingsTab::General,
            section: "Behaviour",
            label: "Auto log",
            keywords: "logging automatic log",
        },
        SettingMeta {
            key: SettingKey::OutputType,
            tab: SettingsTab::General,
            section: "Behaviour",
            label: "Output type",
            keywords: "buffered stream line by line result",
        },
        SettingMeta {
            key: SettingKey::DefaultOutputType,
            tab: SettingsTab::General,
            section: "Behaviour",
            label: "Default output type",
            keywords: "detail dive cli raw output default mode",
        },
        SettingMeta {
            key: SettingKey::AutoExpandDetailDive,
            tab: SettingsTab::General,
            section: "Behaviour",
            label: "Automatically expand detail dive",
            keywords: "tree object node collapse expand first level",
        },
        SettingMeta {
            key: SettingKey::ExecutionTimeMemory,
            tab: SettingsTab::General,
            section: "Behaviour",
            label: "Execution time & memory",
            keywords: "footer metrics duration ms mb ram performance",
        },
        // Editor -> Editor
        SettingMeta {
            key: SettingKey::LineNumbers,
            tab: SettingsTab::Editor,
            section: "Editor",
            label: "Line numbers",
            keywords: "gutter numbers editor",
        },
        SettingMeta {
            key: SettingKey::LineWrap,
            tab: SettingsTab::Editor,
            section: "Editor",
            label: "Line wrap",
            keywords: "soft wrap word wrap horizontal scroll",
        },
        SettingMeta {
            key: SettingKey::IndentationSize,
            tab: SettingsTab::Editor,
            section: "Editor",
            label: "Indentation size",
            keywords: "tab width spaces indent 2 4",
        },
        SettingMeta {
            key: SettingKey::InlineErrorMessages,
            tab: SettingsTab::Editor,
            section: "Editor",
            label: "Inline error messages",
            keywords: "syntax parse error diagnostic red underline squiggle",
        },
        SettingMeta {
            key: SettingKey::UseVimKeymap,
            tab: SettingsTab::Editor,
            section: "Editor",
            label: "Use Vim keymap",
            keywords: "vim modal keybindings normal insert",
        },
        SettingMeta {
            key: SettingKey::HistoryEntries,
            tab: SettingsTab::Editor,
            section: "Editor",
            label: "History entries",
            keywords: "max history buffer limit",
        },
        // Editor -> Font
        SettingMeta {
            key: SettingKey::FontFamily,
            tab: SettingsTab::Editor,
            section: "Font",
            label: "Font family",
            keywords: "typeface monospace menlo jetbrains fira sf mono monaco",
        },
        SettingMeta {
            key: SettingKey::FontSize,
            tab: SettingsTab::Editor,
            section: "Font",
            label: "Font size",
            keywords: "text size zoom pt px",
        },
        SettingMeta {
            key: SettingKey::LineHeight,
            tab: SettingsTab::Editor,
            section: "Font",
            label: "Line height",
            keywords: "spacing leading line height ratio",
        },
        SettingMeta {
            key: SettingKey::FontLigatures,
            tab: SettingsTab::Editor,
            section: "Font",
            label: "Use font ligatures",
            keywords: "ligatures arrows operators => ->",
        },
        // AI Assistant -> Preferences & CLI Agents
        SettingMeta {
            key: SettingKey::AiEnabled,
            tab: SettingsTab::Ai,
            section: "AI assistant",
            label: "Enable AI assistant",
            keywords: "ai assistant chat right sidebar toggle cmd i llm eloquent",
        },
        SettingMeta {
            key: SettingKey::AiAutoWrite,
            tab: SettingsTab::Ai,
            section: "AI assistant",
            label: "Auto-write code to editor",
            keywords: "ai auto write apply insert scratchpad buffer code",
        },
        SettingMeta {
            key: SettingKey::AiAllowSshSchema,
            tab: SettingsTab::Ai,
            section: "AI assistant",
            label: "Allow schema inspection on SSH",
            keywords: "ai ssh remote schema columns database introspection safety",
        },
        SettingMeta {
            key: SettingKey::AiAgentsList,
            tab: SettingsTab::Ai,
            section: "CLI agents",
            label: "Enabled AI agents",
            keywords: "ai cli agents codex claude agy antigravity qoder opencode enable disable switch on off",
        },
        // Advanced -> Advanced
        SettingMeta {
            key: SettingKey::DefaultWorkingDirectory,
            tab: SettingsTab::Advanced,
            section: "Advanced",
            label: "Default working directory",
            keywords: "default folder project path cwd directory new tab",
        },
        SettingMeta {
            key: SettingKey::CollisionEnabled,
            tab: SettingsTab::Advanced,
            section: "Advanced",
            label: "Collision enabled",
            keywords: "nunomaduro collision exception formatter trace driver",
        },
        SettingMeta {
            key: SettingKey::AutocompletionServicePort,
            tab: SettingsTab::Advanced,
            section: "Advanced",
            label: "Autocompletion service port",
            keywords: "lsp port autocomplete intellisense service",
        },
        SettingMeta {
            key: SettingKey::RecentFolders,
            tab: SettingsTab::Advanced,
            section: "Advanced",
            label: "Recent folders",
            keywords: "clear history workspaces recent projects",
        },
    ]
}

#[derive(Clone, Debug)]
pub struct SettingsSearchResults {
    pub matching_specs: Vec<SettingMeta>,
    pub matching_shortcuts: Vec<&'static ShortcutSpec>,
}

impl SettingsSearchResults {
    pub fn count_for_tab(&self, tab: SettingsTab) -> usize {
        if tab == SettingsTab::Shortcuts {
            self.matching_shortcuts.len()
        } else {
            self.matching_specs.iter().filter(|m| m.tab == tab).count()
        }
    }

    pub fn total_count(&self) -> usize {
        self.matching_specs.len() + self.matching_shortcuts.len()
    }
}

fn dynamic_value_haystack(
    key: SettingKey,
    s: &AppSettings,
    detected_php_version: &str,
) -> String {
    match key {
        SettingKey::PhpBinary => format!("{} {}", s.php_binary, detected_php_version),
        SettingKey::AskBeforeClosingTab => {
            if s.ask_before_closing_tab { "yes" } else { "no" }.to_string()
        }
        SettingKey::WelcomeTab => if s.show_welcome_tab { "show" } else { "hide" }.to_string(),
        SettingKey::Theme => s.theme.label().to_string(),
        SettingKey::Layout => s.layout.label().to_string(),
        SettingKey::Toolbar => if s.show_toolbar { "show" } else { "hide" }.to_string(),
        SettingKey::ProjectExplorer => if s.show_explorer { "on enabled show" } else { "off disabled hide" }.to_string(),
        SettingKey::AlwaysOnTop => if s.always_on_top { "yes" } else { "no" }.to_string(),
        SettingKey::AutoHideOutput => if s.auto_hide_output { "yes" } else { "no" }.to_string(),
        SettingKey::AutoEvaluateCode => if s.auto_evaluate_code {
            "yes automatically evaluate code"
        } else {
            "no manually run the code"
        }
        .to_string(),
        SettingKey::EvaluatingCode => s.evaluate_scope.label().to_string(),
        SettingKey::AutoLog => if s.auto_log { "yes" } else { "no" }.to_string(),
        SettingKey::OutputType => if s.buffered_output {
            "buffered output at once"
        } else {
            "stream line by line"
        }
        .to_string(),
        SettingKey::DefaultOutputType => s.default_output.label().to_string(),
        SettingKey::AutoExpandDetailDive => s.detail_dive_expansion.label().to_string(),
        SettingKey::ExecutionTimeMemory => {
            if s.show_execution_metrics { "show" } else { "hide" }.to_string()
        }
        SettingKey::LineNumbers => if s.show_line_numbers { "show" } else { "hide" }.to_string(),
        SettingKey::LineWrap => if s.line_wrap { "yes" } else { "no" }.to_string(),
        SettingKey::IndentationSize => s.indentation_size.to_string(),
        SettingKey::InlineErrorMessages => {
            if s.show_inline_errors { "show" } else { "hide" }.to_string()
        }
        SettingKey::UseVimKeymap => if s.use_vim_keymap { "yes" } else { "no" }.to_string(),
        SettingKey::HistoryEntries => s.history_entries.to_string(),
        SettingKey::FontFamily => s.font_family.clone(),
        SettingKey::FontSize => s.font_size.to_string(),
        SettingKey::LineHeight => s.line_height_ratio.to_string(),
        SettingKey::FontLigatures => if s.use_font_ligatures { "yes" } else { "no" }.to_string(),
        SettingKey::AiEnabled => if s.ai_enabled { "on enabled" } else { "off disabled" }.to_string(),
        SettingKey::AiAutoWrite => if s.ai_auto_write_editor { "on" } else { "off" }.to_string(),
        SettingKey::AiAllowSshSchema => if s.ai_allow_ssh_schema { "on" } else { "off" }.to_string(),
        SettingKey::AiAgentsList => "codex claude_code agy qoder opencode".to_string(),
        SettingKey::DefaultWorkingDirectory => {
            s.default_working_directory.clone().unwrap_or_default()
        }
        SettingKey::CollisionEnabled => if s.collision_enabled { "yes" } else { "no" }.to_string(),
        SettingKey::AutocompletionServicePort => s.autocompletion_service_port.to_string(),
        SettingKey::RecentFolders => "clear stored".to_string(),
    }
}

pub fn search_settings(
    query: &str,
    settings: &AppSettings,
    detected_php_version: &str,
) -> SettingsSearchResults {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.is_empty() {
        return SettingsSearchResults {
            matching_specs: all_setting_specs().to_vec(),
            matching_shortcuts: shortcut_specs().iter().collect(),
        };
    }

    let matching_specs = all_setting_specs()
        .iter()
        .copied()
        .filter(|meta| {
            let haystack = format!(
                "{} {} {} {} {}",
                meta.tab.label(),
                meta.section,
                meta.label,
                meta.keywords,
                dynamic_value_haystack(meta.key, settings, detected_php_version)
            )
            .to_lowercase();
            tokens.iter().all(|tok| haystack.contains(tok))
        })
        .collect();

    let matching_shortcuts = shortcut_specs()
        .iter()
        .filter(|spec| {
            let keys = crate::keymap::effective_keys(spec, &settings.custom_shortcuts);
            let raw = keys
                .join(" ")
                .replace('-', " ")
                .replace("secondary", "secondary cmd command ctrl control");
            let shown = keys
                .iter()
                .map(|k| crate::keymap::display(k))
                .collect::<Vec<_>>()
                .join(" ");
            let normalized_keys = shown
                .replace('⌘', "cmd command ")
                .replace('⇧', "shift ")
                .replace('⌃', "ctrl control ")
                .replace('⌥', "alt option ")
                .replace('+', " ");
            let haystack = format!(
                "shortcuts keyboard hotkey keybinding customize rebind {} {} {} {}",
                spec.label, raw, shown, normalized_keys
            )
            .to_lowercase();
            tokens.iter().all(|tok| haystack.contains(tok))
        })
        .collect();

    SettingsSearchResults {
        matching_specs,
        matching_shortcuts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_local_php_cli_version_and_roundtrips_settings_json() {
        let mut s = AppSettings::default();
        let version_line = detect_php_version_line(&s.php_binary);
        assert!(
            version_line.starts_with("Detected version: PHP "),
            "unexpected version line: {version_line}"
        );

        s.indentation_size = 2;
        s.show_line_numbers = false;
        s.layout = PaneLayout::Horizontal;
        s.autocompletion_service_port = 4010;

        let json = serde_json::to_string(&s).unwrap();
        let parsed: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn persists_show_sql_inline_setting() {
        let mut s = AppSettings::default();
        assert!(s.show_sql_inline);

        s.show_sql_inline = false;
        let json = serde_json::to_string(&s).unwrap();
        let parsed: AppSettings = serde_json::from_str(&json).unwrap();
        assert!(!parsed.show_sql_inline);

        let from_empty: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(from_empty.show_sql_inline);
    }

    #[test]
    fn searches_settings_across_tabs_and_synonyms_including_toolbar() {
        let s = AppSettings::default();
        let php_ver = "Detected version: PHP 8.4.27 (cli)";

        // Direct label search: "Toolbar"
        let res = search_settings("Toolbar", &s, php_ver);
        assert!(
            res.matching_specs
                .iter()
                .any(|m| m.key == SettingKey::Toolbar && m.tab == SettingsTab::General),
            "expected Toolbar setting in General > Appearance"
        );
        assert!(res.count_for_tab(SettingsTab::General) >= 1);

        // Synonym search: "left rail" also finds Toolbar
        let rail_res = search_settings("left rail", &s, php_ver);
        assert!(
            rail_res
                .matching_specs
                .iter()
                .any(|m| m.key == SettingKey::Toolbar)
        );

        // Cross-tab search: "font size" finds Editor > Font > Font Size
        let font_res = search_settings("font size", &s, php_ver);
        assert_eq!(font_res.matching_specs.len(), 1);
        assert_eq!(font_res.matching_specs[0].key, SettingKey::FontSize);
        assert_eq!(font_res.count_for_tab(SettingsTab::Editor), 1);

        // Shortcut search with "cmd l"
        let sc_res = search_settings("cmd l", &s, php_ver);
        assert!(
            sc_res
                .matching_shortcuts
                .iter()
                .any(|spec| spec.label.to_lowercase().contains("logs"))
        );

        // Explorer search finds Project Explorer in General > Appearance
        let exp_res = search_settings("explorer", &s, php_ver);
        assert!(
            exp_res
                .matching_specs
                .iter()
                .any(|m| m.key == SettingKey::ProjectExplorer && m.tab == SettingsTab::General)
        );

        // AI Assistant & Agent On/Off search
        let ai_res = search_settings("qoder", &s, php_ver);
        assert!(
            ai_res
                .matching_specs
                .iter()
                .any(|m| m.key == SettingKey::AiAgentsList && m.tab == SettingsTab::Ai)
        );
    }

    #[test]
    fn persists_ai_agent_on_off_switches_and_sidebar_width() {
        let mut s = AppSettings::default();
        assert!(s.ai_enabled);
        assert!(s.show_ai_sidebar);
        assert!(!s.ai_include_editor_context);
        assert_eq!(s.ai_enabled_agents.get("claude_code"), Some(&true));
        assert_eq!(s.ai_enabled_agents.get("codex"), Some(&true));

        s.ai_enabled_agents.insert("qoder".to_string(), false);
        s.ai_sidebar_width = 410.0;
        s.show_ai_sidebar = false;
        let json = serde_json::to_string(&s).unwrap();
        let parsed_off: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed_off.ai_enabled_agents.get("qoder"), Some(&false));
        assert_eq!(parsed_off.ai_sidebar_width, 410.0);
        assert!(!parsed_off.show_ai_sidebar);
        assert!(parsed_off.ai_enabled);

        s.show_ai_sidebar = true;
        let json_on = serde_json::to_string(&s).unwrap();
        let parsed_on: AppSettings = serde_json::from_str(&json_on).unwrap();
        assert!(parsed_on.show_ai_sidebar);
    }
}
