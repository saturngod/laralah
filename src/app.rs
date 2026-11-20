#[path = "ai_ui.rs"]
mod ai_ui;

use crate::ai::discovery::detect_providers_fast;
use crate::ai::{AiChatSession, AiUiEvent, CliAgentRuntimeState, CliProviderId};
use crate::command_palette::{filter_palette_entries, PaletteAction, PaletteEntry, PaletteItemKind};
use crate::editor::{CodeEditor, TextField};
use crate::laravel_lsp::{
    ClassSymbol, LaravelLanguageService, SymbolKind, insert_use_import,
};
use crate::log_viewer::{
    count_by_level, filter_entries_multi, list_log_files, list_log_files_in_folder,
    log_file_display_name, parse_log_file, LogEntry, LogLevel, LogPollInterval,
};
use crate::mail_webview::MailWebView;
use crate::models::{
    DetailStreamItem, diagnostic_from_exception, extract_first_html_from_result,
    extract_first_table_from_result, extract_table_from_node, format_cli_lines, format_cli_text,
    ordered_detail_items, CliTokenKind, DumpCard, EvaluationResult, ExecutionTarget, ObjectNode,
    OutputMode, TableDataset,
};
use crate::object_graph::{has_object_graph, ObjectGraphView};
use crate::runner::{
    ModelSchemaInfo, RemoteDockerContainer, default_ssh_targets, default_workspace_fallback_path,
    detect_docker_containers, detect_local_docker_containers, detect_local_laravel_version, detect_remote_ssh_docker_containers,
    discover_laravel_projects, evaluate_code_with_options, is_laravel_project,
    lint_php_syntax_with_bin, resolve_php_binary,
};
use crate::settings::{
    AppSettings, DefaultOutputSetting, DetailDiveExpansion, EvaluateScope, PaneLayout, SettingKey,
    ThemeMode,
    SettingsTab, all_setting_specs, available_editor_fonts, detect_php_version_line, is_php_detected,
    load_settings,
    save_settings, search_settings, shortcut_reference_rows,
};
use crate::ssh::{
    ConnectionKind, SSH_COLOR_SWATCHES, SshAuthMethod, SshConfigHost, SshConnectionProfile, SshStore,
    default_docker_application_folder, default_private_key_path, default_ssh_application_folder,
    default_ssh_color, generate_ssh_id, load_ssh_store, load_system_ssh_config, save_ssh_store,
};
use gpui::{
    actions, div, img, point, prelude::*, px, relative, svg, App, ClipboardItem,
    Context, Entity, FocusHandle, Focusable, ImageSource, PathPromptOptions, Render, Resource,
    ScrollDelta, ScrollHandle, SharedString, Window,
};
use crate::theme::{self, fixed_rgb, rgb, rgba};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

actions!(
    laralah,
    [
        QuitApp,
        RunCode,
        OpenDirectory,
        ToggleSidebar,
        ToggleAiSidebar,
        SubmitAiPrompt,
        NewScratchTab,
        CloseScratchTab,
        PrevScratchTab,
        NextScratchTab,
        ToggleSettings,
        ToggleSshModal,
        TogglePalette,
        ToggleLogs,
        ToggleSql,
        ModeDetailDive,
        ModeTable,
        ModeRawCli,
        CloseOverlay,
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
        FocusLogSearch,
        FocusSettingsSearch,
        FocusSshSearch,
        GraphZoomIn,
        GraphZoomOut,
        GraphFitView,
    ]
);

// Clean Light Studio Palette (matching reference screenshot)
const BG_WINDOW: u32 = 0xffffff;
const BG_RAIL: u32 = 0xfbfbfc;
const BG_CANVAS: u32 = 0xf6f8fa;
const BG_CARD: u32 = 0xffffff;
const BG_SUBTLE: u32 = 0xf1f5f9;
const BG_HOVER: u32 = 0xeef2f6;
const BORDER_LIGHT: u32 = 0xe5e7eb;
const BORDER_CARD: u32 = 0xe2e8f0;

const TEXT_MAIN: u32 = 0x1e293b;
const TEXT_SECONDARY: u32 = 0x64748b;
const TEXT_MUTED: u32 = 0x94a3b8;

const COLOR_PROP_KEY: u32 = 0xe11d48;
const COLOR_STRING: u32 = 0xd97706;
const COLOR_NUMBER: u32 = 0x2563eb;
const COLOR_BOOL: u32 = 0x7c3aed;
const COLOR_SQL_ACCENT: u32 = 0x9333ea;
const COLOR_SUCCESS: u32 = 0x10b981;
const COLOR_DANGER: u32 = 0xef4444;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogMenuPopup {
    None,
    Files,
    Levels,
    Polling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SshModalMode {
    List,
    Form,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    None,
    CommandPalette,
    LogViewer,
    TablePreview,
    ObjectGraph,
    HtmlPreview,
    Settings,
    SshConnections,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SashDragState {
    None,
    Sidebar { start_x: f32, start_width: f32 },
    AiSidebar { start_x: f32, start_width: f32 },
    SplitVertical { start_x: f32, start_ratio: f32 },
    StackHorizontal { start_y: f32, start_ratio: f32 },
}

#[derive(Clone, Debug)]
struct ScratchTab {
    title: String,
    project_path: String,
    target: ExecutionTarget,
    code: String,
    last_result: Option<EvaluationResult>,
    expanded_nodes: HashSet<String>,
    output_mode: OutputMode,
}

pub struct LaralahApp {
    focus_handle: FocusHandle,
    editor: Entity<CodeEditor>,
    object_graph_view: Entity<ObjectGraphView>,
    mail_webview: Entity<MailWebView>,
    palette_input: Entity<TextField>,
    table_search_input: Entity<TextField>,
    log_search_input: Entity<TextField>,
    settings_search_input: Entity<TextField>,
    indent_size_input: Entity<TextField>,
    history_entries_input: Entity<TextField>,
    font_size_input: Entity<TextField>,
    line_height_input: Entity<TextField>,
    autocomplete_port_input: Entity<TextField>,

    last_editor_version: u64,
    last_palette_version: u64,
    last_table_search_version: u64,
    last_settings_search_version: u64,
    last_settings_input_versions: [u64; 5],

    // Persisted application settings
    settings: AppSettings,
    active_settings_tab: SettingsTab,
    detected_php_version: String,
    detected_laravel_version: String,

    // SSH setup & connections manager state
    ssh_store: SshStore,
    ssh_modal_mode: SshModalMode,
    ssh_editing_id: Option<String>,
    ssh_creating_group: bool,
    ssh_config_hosts: Vec<SshConfigHost>,
    active_dropdown_id: Option<SharedString>,
    ssh_selected_config_label: String,
    ssh_form_kind: ConnectionKind,
    ssh_form_docker_enabled: bool,
    ssh_docker_fetching: bool,
    ssh_docker_picker_open: bool,
    ssh_detected_containers: Vec<RemoteDockerContainer>,
    ssh_form_local_path: Option<String>,
    ssh_form_auth_method: SshAuthMethod,
    ssh_form_color_hex: u32,
    ssh_form_group: Option<String>,
    ssh_form_error: Option<String>,
    ssh_search_input: Entity<TextField>,
    ssh_group_input: Entity<TextField>,
    ssh_group_combo_input: Entity<TextField>,
    ssh_label_input: Entity<TextField>,
    ssh_host_input: Entity<TextField>,
    ssh_port_input: Entity<TextField>,
    ssh_user_input: Entity<TextField>,
    ssh_folder_input: Entity<TextField>,
    ssh_docker_container_input: Entity<TextField>,
    ssh_key_input: Entity<TextField>,
    ssh_passphrase_input: Entity<TextField>,
    last_ssh_search_version: u64,

    // Landing screen, collapsible explorer sidebar & scratchpad tabs
    show_landing: bool,
    show_sidebar: bool,
    explorer_search_input: Entity<TextField>,
    last_explorer_search_version: u64,
    explorer_models: Vec<ClassSymbol>,
    explorer_mailables: Vec<ClassSymbol>,
    tab_scroll_handle: ScrollHandle,
    tab_context_menu: Option<(usize, gpui::Point<gpui::Pixels>)>,
    tabs: Vec<ScratchTab>,
    active_tab_idx: usize,
    next_tab_num: usize,
    focus_editor_next_frame: bool,

    // Project & execution state
    projects: Vec<String>,
    active_project: String,
    targets: Vec<ExecutionTarget>,
    active_target: ExecutionTarget,
    auto_run: bool,
    is_running: bool,
    eval_seq: u64,
    auto_run_seq: u64,
    last_result: Option<EvaluationResult>,
    expanded_nodes: HashSet<String>,

    // Right pane display state
    output_mode: OutputMode,
    prev_non_cli_mode: OutputMode,
    open_html_on_next_result: bool,

    // Table Preview state
    active_table_preview: Option<TableDataset>,
    open_table_on_next_result: bool,
    table_sort_col: Option<usize>,
    table_sort_asc: bool,
    table_page: usize,
    table_page_size: usize,
    selected_table_row: Option<usize>,
    table_export_status: Option<(String, bool)>,

    // Overlays, split sash drag & rail tooltip
    overlay: OverlayKind,
    sash_drag: SashDragState,
    palette_selected_idx: usize,
    hovered_rail_tooltip: Option<(usize, &'static str)>,

    // Log viewer state
    custom_logs_folder: Option<PathBuf>,
    log_files: Vec<PathBuf>,
    active_log_file_idx: usize,
    log_entries: Vec<LogEntry>,
    selected_log_levels: HashSet<LogLevel>,
    log_poll_interval: LogPollInterval,
    log_menu_popup: LogMenuPopup,
    log_page: usize,
    log_page_size: usize,
    last_log_search_version: u64,
    expanded_log_ids: HashSet<usize>,
    live_tail_epoch: u64,

    // AI Assistant right sidebar & CLI agent state
    show_ai_sidebar: bool,
    ai_picker_open: bool,
    ai_effort_menu_open: bool,
    ai_picker_show_older: bool,
    ai_sessions_menu_open: bool,
    ai_prompt_input: Entity<TextField>,
    ai_model_search_input: Entity<TextField>,
    last_ai_model_search_version: u64,
    ai_chat_scroll: ScrollHandle,
    ai_agents: HashMap<CliProviderId, CliAgentRuntimeState>,
    ai_sessions: Vec<AiChatSession>,
    ai_active_session_idx: usize,
    ai_next_session_id: u64,
    ai_next_msg_id: u64,
    ai_active_cancel: Option<Arc<AtomicBool>>,
    ai_ui_tx: std::sync::mpsc::Sender<AiUiEvent>,
    ai_schema_cache: HashMap<String, HashMap<String, ModelSchemaInfo>>,
    ai_tables_cache: HashMap<String, Vec<String>>,
    ai_editor_undo_banner: bool,
    ai_copied_msg_id: Option<u64>,
}

impl Focusable for LaralahApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl LaralahApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let is_clean_start = std::env::args()
            .skip(1)
            .any(|a| a == "--clean" || a == "--fresh");

        let settings = if is_clean_start {
            AppSettings::default()
        } else {
            load_settings()
        };
        let detected_php_version = detect_php_version_line(&settings.php_binary);

        let mut projects = if is_clean_start {
            Vec::new()
        } else {
            load_recent_folders()
        };
        if !is_clean_start {
            if let Some(ref def_dir) = settings.default_working_directory
                && std::path::Path::new(def_dir).is_dir()
            {
                projects.retain(|p| p != def_dir);
                projects.insert(0, def_dir.clone());
            }
            for discovered in discover_laravel_projects() {
                if !projects.contains(&discovered) {
                    projects.push(discovered);
                }
            }
        }
        let resolved_initial_project = settings
            .default_working_directory
            .clone()
            .filter(|d| std::path::Path::new(d).is_dir())
            .or_else(|| projects.first().cloned());
        let has_initial_project = resolved_initial_project.is_some();
        let active_project =
            resolved_initial_project.unwrap_or_else(default_workspace_fallback_path);

        let active_target = ExecutionTarget::Local {
            project_path: active_project.clone(),
        };
        let mut targets = vec![active_target.clone()];
        if has_initial_project {
            targets.extend(detect_docker_containers(&active_project));
        }
        if !is_clean_start {
            targets.extend(default_ssh_targets());
        }

        let initial_code = starter_code_for_project(&active_project);
        let editor = cx.new(|cx| {
            let mut ed = CodeEditor::new(&initial_code, cx);
            ed.apply_settings(&settings, cx);
            ed
        });
        let palette_input = cx.new(|cx| {
            TextField::new("Type a command, / for projects, @ for connections...", cx)
        });
        let table_search_input = cx.new(|cx| TextField::new("Filter rows...", cx));
        let log_search_input = cx.new(|cx| {
            TextField::new("Press CMD + F to focus search", cx).with_text_color(0xe2e8f0)
        });
        let settings_search_input = cx.new(|cx| {
            TextField::new(
                "Search settings",
                cx,
            )
            .with_text_color(0xf3f4f6)
        });

        let indent_init = settings.indentation_size.to_string();
        let history_init = settings.history_entries.to_string();
        let font_size_init = format_setting_float(settings.font_size);
        let line_height_init = format_setting_float(settings.line_height_ratio);
        let port_init = settings.autocompletion_service_port.to_string();

        let indent_size_input = cx.new(|cx| {
            let mut tf = TextField::new("4", cx).with_text_color(0xf3f4f6);
            tf.set_text(&indent_init, cx);
            tf
        });
        let history_entries_input = cx.new(|cx| {
            let mut tf = TextField::new("150", cx).with_text_color(0xf3f4f6);
            tf.set_text(&history_init, cx);
            tf
        });
        let font_size_input = cx.new(|cx| {
            let mut tf = TextField::new("14", cx).with_text_color(0xf3f4f6);
            tf.set_text(&font_size_init, cx);
            tf
        });
        let line_height_input = cx.new(|cx| {
            let mut tf = TextField::new("1.8", cx).with_text_color(0xf3f4f6);
            tf.set_text(&line_height_init, cx);
            tf
        });
        let autocomplete_port_input = cx.new(|cx| {
            let mut tf = TextField::new("3050", cx).with_text_color(0xf3f4f6);
            tf.set_text(&port_init, cx);
            tf
        });

        let init_versions = [
            indent_size_input.read(cx).edit_version,
            history_entries_input.read(cx).edit_version,
            font_size_input.read(cx).edit_version,
            line_height_input.read(cx).edit_version,
            autocomplete_port_input.read(cx).edit_version,
        ];

        cx.observe(&editor, |this: &mut Self, ed, cx| {
            let ver = ed.read(cx).edit_version;
            if ver != this.last_editor_version {
                this.last_editor_version = ver;
                this.ai_editor_undo_banner = false;
                this.schedule_auto_run(cx);
            }
        })
        .detach();

        cx.observe(&palette_input, |this: &mut Self, input, cx| {
            let ver = input.read(cx).edit_version;
            if ver != this.last_palette_version {
                this.last_palette_version = ver;
                this.palette_selected_idx = 0;
                cx.notify();
            }
        })
        .detach();

        cx.observe(&table_search_input, |this: &mut Self, input, cx| {
            let ver = input.read(cx).edit_version;
            if ver != this.last_table_search_version {
                this.last_table_search_version = ver;
                this.table_page = 0;
                cx.notify();
            }
        })
        .detach();

        cx.observe(&log_search_input, |this: &mut Self, input, cx| {
            let ver = input.read(cx).edit_version;
            if ver != this.last_log_search_version {
                this.last_log_search_version = ver;
                this.log_page = 0;
                cx.notify();
            }
        })
        .detach();

        cx.observe(&settings_search_input, |this: &mut Self, input, cx| {
            let ver = input.read(cx).edit_version;
            if ver != this.last_settings_search_version {
                this.last_settings_search_version = ver;
                cx.notify();
            }
        })
        .detach();

        let explorer_search_input =
            cx.new(|cx| TextField::new("Filter models...", cx).with_text_color(TEXT_MAIN));
        cx.observe(&explorer_search_input, |this: &mut Self, input, cx| {
            let ver = input.read(cx).edit_version;
            if ver != this.last_explorer_search_version {
                this.last_explorer_search_version = ver;
                cx.notify();
            }
        })
        .detach();

        for (slot, entity) in [
            (0usize, indent_size_input.clone()),
            (1, history_entries_input.clone()),
            (2, font_size_input.clone()),
            (3, line_height_input.clone()),
            (4, autocomplete_port_input.clone()),
        ] {
            cx.observe(&entity, move |this: &mut Self, input, cx| {
                let ver = input.read(cx).edit_version;
                if ver != this.last_settings_input_versions[slot] {
                    this.last_settings_input_versions[slot] = ver;
                    this.sync_settings_from_inputs(cx);
                }
            })
            .detach();
        }

        let log_files = if has_initial_project {
            list_log_files(&active_project)
        } else {
            Vec::new()
        };
        let log_entries = log_files
            .first()
            .map(|p| parse_log_file(p))
            .unwrap_or_default();

        let object_graph_view = cx.new(ObjectGraphView::new);
        cx.observe(&object_graph_view, |_, _, cx| {
            cx.notify();
        })
        .detach();
        let mail_webview = cx.new(MailWebView::new);

        let initial_mode = match settings.default_output {
            DefaultOutputSetting::DetailDive => OutputMode::DetailDive,
            DefaultOutputSetting::CliOutput => OutputMode::RawCli,
        };
        let show_landing = if is_clean_start || !has_initial_project {
            true
        } else {
            settings.show_welcome_tab
        };
        let auto_run = settings.auto_evaluate_code;
        let detected_laravel_version = if has_initial_project {
            detect_local_laravel_version(&active_project)
        } else {
            "11.x".to_string()
        };

        let ssh_store = if is_clean_start {
            SshStore::default()
        } else {
            load_ssh_store()
        };
        let ssh_config_hosts = load_system_ssh_config();

        let ssh_search_input = cx.new(|cx| TextField::new("", cx).with_text_color(0xf3f4f6));
        let ssh_group_input =
            cx.new(|cx| TextField::new("Group name (e.g. Production)", cx).with_text_color(0xf3f4f6));
        let ssh_group_combo_input =
            cx.new(|cx| TextField::new("Filter or create group...", cx).with_text_color(0xf3f4f6));
        cx.observe(&ssh_group_combo_input, |_, _, cx| {
            cx.notify();
        })
        .detach();
        let ssh_label_input =
            cx.new(|cx| TextField::new("Laralah Production", cx).with_text_color(0xf3f4f6));
        let ssh_host_input =
            cx.new(|cx| TextField::new("Host (IP Address)", cx).with_text_color(0xf3f4f6));
        let ssh_port_input = cx.new(|cx| {
            let mut tf = TextField::new("22", cx).with_text_color(0xf3f4f6);
            tf.set_text("22", cx);
            tf
        });
        let ssh_user_input = cx.new(|cx| {
            let mut tf = TextField::new("forge", cx).with_text_color(0xf3f4f6);
            tf.set_text("forge", cx);
            tf
        });
        let ssh_folder_input = cx.new(|cx| {
            let mut tf =
                TextField::new(default_ssh_application_folder(), cx).with_text_color(0xf3f4f6);
            tf.set_text(default_ssh_application_folder(), cx);
            tf
        });
        let ssh_docker_container_input = cx.new(|cx| {
            TextField::new("Container name or ID (e.g. laravel.test, a1b2c3d4e5f6)", cx)
                .with_text_color(0xf3f4f6)
        });
        let ssh_key_input = cx.new(|cx| {
            let mut tf = TextField::new("~/.ssh/id_rsa", cx).with_text_color(0xf3f4f6);
            tf.set_text(default_private_key_path(), cx);
            tf
        });
        let ssh_passphrase_input = cx.new(|cx| {
            TextField::new("Passphrase", cx)
                .with_text_color(0xf3f4f6)
                .with_masked(true)
        });

        cx.observe(&ssh_search_input, |this: &mut Self, input, cx| {
            let ver = input.read(cx).edit_version;
            if ver != this.last_ssh_search_version {
                this.last_ssh_search_version = ver;
                cx.notify();
            }
        })
        .detach();

        let initial_title = if has_initial_project {
            std::path::Path::new(&active_project)
                .file_name()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Scratch 1")
                .to_string()
        } else {
            "Scratch 1".to_string()
        };
        let initial_tab = ScratchTab {
            title: initial_title,
            project_path: active_project.clone(),
            target: active_target.clone(),
            code: initial_code.clone(),
            last_result: None,
            expanded_nodes: HashSet::new(),
            output_mode: initial_mode,
        };

        let initial_show_sidebar = settings.show_explorer;
        let initial_show_ai_sidebar = settings.ai_enabled && settings.show_ai_sidebar;

        let ai_prompt_input = cx.new(|cx| {
            TextField::new("Ask AI to write Eloquent code…", cx).with_text_color(TEXT_MAIN)
        });
        let ai_model_search_input =
            cx.new(|cx| TextField::new("Search models", cx).with_text_color(TEXT_MAIN));
        cx.observe(&ai_model_search_input, |this: &mut Self, input, cx| {
            let ver = input.read(cx).edit_version;
            if ver != this.last_ai_model_search_version {
                this.last_ai_model_search_version = ver;
                cx.notify();
            }
        })
        .detach();

        let (ai_ui_tx, ai_ui_rx) = std::sync::mpsc::channel::<AiUiEvent>();
        let ai_agents: HashMap<CliProviderId, CliAgentRuntimeState> = detect_providers_fast()
            .into_iter()
            .map(|(id, avail)| (id, CliAgentRuntimeState::new(avail)))
            .collect();

        let mut app = Self {
            focus_handle: cx.focus_handle(),
            editor,
            object_graph_view,
            mail_webview,
            palette_input,
            table_search_input,
            log_search_input,
            settings_search_input,
            indent_size_input,
            history_entries_input,
            font_size_input,
            line_height_input,
            autocomplete_port_input,
            last_editor_version: 0,
            last_palette_version: 0,
            last_table_search_version: 0,
            last_settings_search_version: 0,
            last_settings_input_versions: init_versions,
            settings,
            active_settings_tab: SettingsTab::General,
            detected_php_version,
            detected_laravel_version,
            ssh_store,
            ssh_modal_mode: SshModalMode::List,
            ssh_editing_id: None,
            ssh_creating_group: false,
            ssh_config_hosts,
            active_dropdown_id: None,
            ssh_selected_config_label: "Preselect SSH Config".to_string(),
            ssh_form_kind: ConnectionKind::Ssh,
            ssh_form_docker_enabled: false,
            ssh_docker_fetching: false,
            ssh_docker_picker_open: false,
            ssh_detected_containers: Vec::new(),
            ssh_form_local_path: None,
            ssh_form_auth_method: SshAuthMethod::PrivateKey,
            ssh_form_color_hex: default_ssh_color(),
            ssh_form_group: None,
            ssh_form_error: None,
            ssh_search_input,
            ssh_group_input,
            ssh_group_combo_input,
            ssh_label_input,
            ssh_host_input,
            ssh_port_input,
            ssh_user_input,
            ssh_folder_input,
            ssh_docker_container_input,
            ssh_key_input,
            ssh_passphrase_input,
            last_ssh_search_version: 0,
            show_landing,
            show_sidebar: initial_show_sidebar,
            explorer_search_input,
            last_explorer_search_version: 0,
            explorer_models: Vec::new(),
            explorer_mailables: Vec::new(),
            tab_scroll_handle: ScrollHandle::new(),
            tab_context_menu: None,
            tabs: vec![initial_tab],
            active_tab_idx: 0,
            next_tab_num: 2,
            focus_editor_next_frame: !show_landing,
            projects,
            active_project: active_project.clone(),
            targets,
            active_target,
            auto_run,
            is_running: false,
            eval_seq: 0,
            auto_run_seq: 0,
            last_result: None,
            expanded_nodes: HashSet::new(),
            output_mode: initial_mode,
            prev_non_cli_mode: OutputMode::DetailDive,
            open_html_on_next_result: false,
            active_table_preview: None,
            open_table_on_next_result: false,
            table_sort_col: None,
            table_sort_asc: true,
            table_page: 0,
            table_page_size: 25,
            selected_table_row: None,
            table_export_status: None,
            overlay: OverlayKind::None,
            sash_drag: SashDragState::None,
            palette_selected_idx: 0,
            hovered_rail_tooltip: None,
            custom_logs_folder: None,
            log_files,
            active_log_file_idx: 0,
            log_entries,
            selected_log_levels: HashSet::new(),
            log_poll_interval: LogPollInterval::Off,
            log_menu_popup: LogMenuPopup::None,
            log_page: 0,
            log_page_size: 20,
            last_log_search_version: 0,
            expanded_log_ids: HashSet::new(),
            live_tail_epoch: 0,
            show_ai_sidebar: initial_show_ai_sidebar,
            ai_picker_open: false,
            ai_effort_menu_open: false,
            ai_picker_show_older: false,
            ai_sessions_menu_open: false,
            ai_prompt_input,
            ai_model_search_input,
            last_ai_model_search_version: 0,
            ai_chat_scroll: ScrollHandle::new(),
            ai_agents,
            ai_sessions: vec![AiChatSession::new(1)],
            ai_active_session_idx: 0,
            ai_next_session_id: 2,
            ai_next_msg_id: 1,
            ai_active_cancel: None,
            ai_ui_tx,
            ai_schema_cache: HashMap::new(),
            ai_tables_cache: HashMap::new(),
            ai_editor_undo_banner: false,
            ai_copied_msg_id: None,
        };
        app.spawn_ai_event_listener(ai_ui_rx, cx);
        app.bootstrap_ai_discovery(cx);
        if has_initial_project {
            app.attach_project_lsp(&active_project, cx);
        }
        app
    }

    fn attach_project_lsp(&mut self, project_path: &str, cx: &mut Context<Self>) {
        let path_owned = project_path.to_string();
        cx.spawn(async move |this, cx| {
            let service = cx
                .background_executor()
                .spawn(async move { Arc::new(LaravelLanguageService::for_project(&path_owned)) })
                .await;
            this.update(cx, |app, cx| {
                app.explorer_models = service
                    .index
                    .classes
                    .iter()
                    .filter(|s| s.kind == SymbolKind::Model)
                    .cloned()
                    .collect();
                app.explorer_mailables = service
                    .index
                    .classes
                    .iter()
                    .filter(|s| s.kind == SymbolKind::Mailable)
                    .cloned()
                    .collect();
                app.editor.update(cx, |ed, cx| {
                    ed.set_language_service(service, cx);
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn sync_active_tab_snapshot(&mut self, cx: &Context<Self>) {
        let cur_code = self.editor.read(cx).text().to_string();
        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
            tab.project_path = self.active_project.clone();
            tab.target = self.active_target.clone();
            tab.code = cur_code;
            tab.last_result = self.last_result.clone();
            tab.expanded_nodes = self.expanded_nodes.clone();
            tab.output_mode = self.output_mode;
        }
    }

    fn apply_tab_project_context(&mut self, tab: &ScratchTab, cx: &mut Context<Self>) {
        let project_changed = self.active_project != tab.project_path;
        self.active_project = tab.project_path.clone();
        self.active_target = tab.target.clone();
        self.auto_run = self.settings.auto_evaluate_code && !self.active_target.is_ssh();

        if project_changed && !tab.project_path.is_empty() && std::path::Path::new(&tab.project_path).is_dir() {
            self.detected_laravel_version = detect_local_laravel_version(&tab.project_path);
            let local = ExecutionTarget::Local {
                project_path: tab.project_path.clone(),
            };
            let mut next_targets = vec![local];
            next_targets.extend(detect_docker_containers(&tab.project_path));
            next_targets.extend(default_ssh_targets());
            self.targets = next_targets;
            self.explorer_search_input.update(cx, |tf, cx| {
                tf.set_text("", cx);
            });
            self.custom_logs_folder = None;
            self.active_log_file_idx = 0;
            self.log_page = 0;
            self.attach_project_lsp(&tab.project_path, cx);
            self.refresh_logs(cx);
        }
    }

    fn switch_to_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() || idx == self.active_tab_idx {
            self.show_landing = false;
            self.focus_editor_next_frame = true;
            cx.notify();
            return;
        }
        self.sync_active_tab_snapshot(cx);
        self.active_tab_idx = idx;
        let tab = self.tabs[idx].clone();
        self.apply_tab_project_context(&tab, cx);
        self.editor.update(cx, |ed, cx| {
            ed.set_text(tab.code, cx);
            ed.set_diagnostic(None, cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        self.last_result = tab.last_result;
        self.expanded_nodes = tab.expanded_nodes;
        self.output_mode = tab.output_mode;
        self.show_landing = false;
        self.focus_editor_next_frame = true;
        self.tab_scroll_handle.scroll_to_item(self.active_tab_idx);
        cx.notify();
    }

    fn create_new_tab(&mut self, cx: &mut Context<Self>) {
        self.sync_active_tab_snapshot(cx);
        let num = self.next_tab_num;
        self.next_tab_num += 1;
        let starter = starter_code_for_project(&self.active_project);
        let default_mode = match self.settings.default_output {
            DefaultOutputSetting::DetailDive => OutputMode::DetailDive,
            DefaultOutputSetting::CliOutput => OutputMode::RawCli,
        };
        let new_tab = ScratchTab {
            title: format!("Scratch {num}"),
            project_path: self.active_project.clone(),
            target: self.active_target.clone(),
            code: starter.clone(),
            last_result: None,
            expanded_nodes: HashSet::new(),
            output_mode: default_mode,
        };
        self.tabs.push(new_tab);
        self.active_tab_idx = self.tabs.len() - 1;
        self.editor.update(cx, |ed, cx| {
            ed.set_text(starter, cx);
            ed.set_diagnostic(None, cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        self.last_result = None;
        self.expanded_nodes.clear();
        self.output_mode = default_mode;
        self.show_landing = false;
        self.focus_editor_next_frame = true;
        self.tab_scroll_handle.scroll_to_item(self.active_tab_idx);
        cx.notify();
    }

    fn close_tab_at(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 {
            // Reset single remaining scratchpad cleanly
            let starter = starter_code_for_project(&self.active_project);
            if let Some(tab) = self.tabs.get_mut(0) {
                tab.code = starter.clone();
                tab.last_result = None;
                tab.expanded_nodes.clear();
            }
            self.editor.update(cx, |ed, cx| {
                ed.set_text(starter, cx);
                ed.set_diagnostic(None, cx);
            });
            self.last_editor_version = self.editor.read(cx).edit_version;
            self.last_result = None;
            self.expanded_nodes.clear();
            cx.notify();
            return;
        }
        self.sync_active_tab_snapshot(cx);
        self.tabs.remove(idx);
        if self.active_tab_idx >= self.tabs.len() {
            self.active_tab_idx = self.tabs.len() - 1;
        } else if idx < self.active_tab_idx {
            self.active_tab_idx -= 1;
        }
        let tab = self.tabs[self.active_tab_idx].clone();
        self.apply_tab_project_context(&tab, cx);
        self.editor.update(cx, |ed, cx| {
            ed.set_text(tab.code, cx);
            ed.set_diagnostic(None, cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        self.last_result = tab.last_result;
        self.expanded_nodes = tab.expanded_nodes;
        self.output_mode = tab.output_mode;
        self.focus_editor_next_frame = true;
        self.tab_scroll_handle.scroll_to_item(self.active_tab_idx);
        cx.notify();
    }

    fn close_other_tabs(&mut self, keep_idx: usize, cx: &mut Context<Self>) {
        if keep_idx >= self.tabs.len() || self.tabs.len() <= 1 {
            return;
        }
        self.sync_active_tab_snapshot(cx);
        let kept = self.tabs[keep_idx].clone();
        self.tabs = vec![kept.clone()];
        self.active_tab_idx = 0;
        self.apply_tab_project_context(&kept, cx);
        self.editor.update(cx, |ed, cx| {
            ed.set_text(kept.code, cx);
            ed.set_diagnostic(None, cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        self.last_result = kept.last_result;
        self.expanded_nodes = kept.expanded_nodes;
        self.output_mode = kept.output_mode;
        self.focus_editor_next_frame = true;
        self.tab_scroll_handle.set_offset(point(px(0.0), px(0.0)));
        cx.notify();
    }

    fn close_tabs_to_right(&mut self, from_idx: usize, cx: &mut Context<Self>) {
        if from_idx + 1 >= self.tabs.len() {
            return;
        }
        self.sync_active_tab_snapshot(cx);
        self.tabs.truncate(from_idx + 1);
        if self.active_tab_idx > from_idx {
            self.active_tab_idx = from_idx;
            let tab = self.tabs[from_idx].clone();
            self.apply_tab_project_context(&tab, cx);
            self.editor.update(cx, |ed, cx| {
                ed.set_text(tab.code, cx);
                ed.set_diagnostic(None, cx);
            });
            self.last_editor_version = self.editor.read(cx).edit_version;
            self.last_result = tab.last_result;
            self.expanded_nodes = tab.expanded_nodes;
            self.output_mode = tab.output_mode;
        }
        self.focus_editor_next_frame = true;
        self.tab_scroll_handle.scroll_to_item(self.active_tab_idx);
        cx.notify();
    }

    fn close_tabs_to_left(&mut self, from_idx: usize, cx: &mut Context<Self>) {
        if from_idx == 0 || from_idx >= self.tabs.len() {
            return;
        }
        self.sync_active_tab_snapshot(cx);
        let was_left = self.active_tab_idx < from_idx;
        self.tabs.drain(0..from_idx);
        if was_left {
            self.active_tab_idx = 0;
            let tab = self.tabs[0].clone();
            self.apply_tab_project_context(&tab, cx);
            self.editor.update(cx, |ed, cx| {
                ed.set_text(tab.code, cx);
                ed.set_diagnostic(None, cx);
            });
            self.last_editor_version = self.editor.read(cx).edit_version;
            self.last_result = tab.last_result;
            self.expanded_nodes = tab.expanded_nodes;
            self.output_mode = tab.output_mode;
        } else {
            self.active_tab_idx -= from_idx;
        }
        self.focus_editor_next_frame = true;
        self.tab_scroll_handle.scroll_to_item(self.active_tab_idx);
        cx.notify();
    }

    fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        let title = std::path::Path::new(&self.active_project)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty() && is_laravel_project(&self.active_project))
            .unwrap_or("Scratch 1")
            .to_string();
        let starter = starter_code_for_project(&self.active_project);
        let default_mode = match self.settings.default_output {
            DefaultOutputSetting::DetailDive => OutputMode::DetailDive,
            DefaultOutputSetting::CliOutput => OutputMode::RawCli,
        };
        self.tabs = vec![ScratchTab {
            title,
            project_path: self.active_project.clone(),
            target: self.active_target.clone(),
            code: starter.clone(),
            last_result: None,
            expanded_nodes: HashSet::new(),
            output_mode: default_mode,
        }];
        self.active_tab_idx = 0;
        self.next_tab_num = 2;
        self.editor.update(cx, |ed, cx| {
            ed.set_text(starter, cx);
            ed.set_diagnostic(None, cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        self.last_result = None;
        self.expanded_nodes.clear();
        self.output_mode = default_mode;
        self.focus_editor_next_frame = true;
        self.tab_scroll_handle.set_offset(point(px(0.0), px(0.0)));
        cx.notify();
    }

    fn insert_symbol_snippet(&mut self, sym: &ClassSymbol, cx: &mut Context<Self>) {
        let cur = self.editor.read(cx).text().to_string();
        let (with_import, _) = insert_use_import(&cur, cur.len(), &sym.fqcn);
        let snippet = match sym.kind {
            SymbolKind::Model => format!(
                "{}\n\n{}::query()->limit(10)->get();",
                with_import.trim_end(),
                sym.short_name
            ),
            SymbolKind::Mailable => {
                format!("{}\n\n// Inspect {}\n", with_import.trim_end(), sym.short_name)
            }
            _ => with_import,
        };
        self.editor.update(cx, |ed, cx| {
            ed.set_text(snippet, cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        self.show_landing = false;
        self.focus_editor_next_frame = true;
        if sym.kind == SymbolKind::Model && !self.active_target.is_ssh() {
            self.run_evaluation(cx);
        } else {
            cx.notify();
        }
    }

    pub fn open_directory_dialog(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Laravel Project".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(chosen) = paths.into_iter().next()
            {
                let path_str = chosen.to_string_lossy().to_string();
                this.update(cx, |app, cx| {
                    app.open_project_path(path_str, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn open_project_path(&mut self, path: String, cx: &mut Context<Self>) {
        self.projects.retain(|p| p != &path);
        self.projects.insert(0, path.clone());
        save_recent_folders(&self.projects);

        // If already viewing this exact local workspace, preserve everything as-is
        if !self.show_landing && self.active_project == path && !self.active_target.is_ssh() {
            self.focus_editor_next_frame = true;
            cx.notify();
            return;
        }

        // Snapshot current active tab before switching or creating a workspace tab
        self.sync_active_tab_snapshot(cx);

        // If an open tab already belongs to this local workspace, switch back to it with all state intact
        if let Some(existing_idx) = self.tabs.iter().position(|t| {
            t.project_path == path && matches!(t.target, ExecutionTarget::Local { .. })
        }) {
            if existing_idx == self.active_tab_idx {
                let tab = self.tabs[existing_idx].clone();
                self.apply_tab_project_context(&tab, cx);
                self.show_landing = false;
                self.focus_editor_next_frame = true;
                cx.notify();
            } else {
                self.switch_to_tab(existing_idx, cx);
            }
            return;
        }

        let folder_title = std::path::Path::new(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Workspace")
            .to_string();

        let local = ExecutionTarget::Local {
            project_path: path.clone(),
        };
        let mut next_targets = vec![local.clone()];
        next_targets.extend(detect_docker_containers(&path));
        next_targets.extend(default_ssh_targets());

        let cur_code = self.editor.read(cx).text().to_string();
        let prev_starter = starter_code_for_project(&self.active_project);
        let can_reuse_initial_tab = self.show_landing
            && self.tabs.len() == 1
            && self.last_result.is_none()
            && (cur_code.trim().is_empty() || cur_code == prev_starter);

        let starter_code = starter_code_for_project(&path);
        let default_mode = match self.settings.default_output {
            DefaultOutputSetting::DetailDive => OutputMode::DetailDive,
            DefaultOutputSetting::CliOutput => OutputMode::RawCli,
        };

        self.active_project = path.clone();
        self.detected_laravel_version = detect_local_laravel_version(&path);
        self.targets = next_targets;
        self.active_target = local.clone();
        self.auto_run = self.settings.auto_evaluate_code;

        if can_reuse_initial_tab {
            if let Some(tab) = self.tabs.get_mut(0) {
                tab.title = folder_title;
                tab.project_path = path.clone();
                tab.target = local;
                tab.code = starter_code.clone();
                tab.last_result = None;
                tab.expanded_nodes.clear();
                tab.output_mode = default_mode;
            }
            self.active_tab_idx = 0;
        } else {
            // Open the new workspace in its own tab so the previous workspace's code & output are never lost
            let new_tab = ScratchTab {
                title: folder_title,
                project_path: path.clone(),
                target: local,
                code: starter_code.clone(),
                last_result: None,
                expanded_nodes: HashSet::new(),
                output_mode: default_mode,
            };
            self.tabs.push(new_tab);
            self.active_tab_idx = self.tabs.len() - 1;
        }
        self.tab_scroll_handle.scroll_to_item(self.active_tab_idx);

        self.editor.update(cx, |ed, cx| {
            ed.set_text(starter_code, cx);
            ed.set_diagnostic(None, cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        self.last_result = None;
        self.expanded_nodes.clear();
        self.output_mode = default_mode;

        self.explorer_search_input.update(cx, |tf, cx| {
            tf.set_text("", cx);
        });
        self.show_landing = false;
        self.focus_editor_next_frame = true;
        self.open_html_on_next_result = false;
        self.custom_logs_folder = None;
        self.active_log_file_idx = 0;
        self.log_page = 0;
        self.attach_project_lsp(&path, cx);
        self.refresh_logs(cx);
        if self.auto_run {
            self.run_evaluation(cx);
        } else {
            cx.notify();
        }
    }

    fn run_evaluation(&mut self, cx: &mut Context<Self>) {
        // Automatically inject missing `use App\Models\...;` / Facade imports before running
        self.editor.update(cx, |ed, cx| {
            ed.apply_auto_imports(cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;

        let full_text = self.editor.read(cx).text().to_string();
        let code = if self.settings.evaluate_scope == EvaluateScope::SelectedIfAvailable
            && let Some(selected) = self.editor.read(cx).selected_text()
            && !selected.trim().is_empty()
        {
            prepend_buffer_use_statements(&full_text, selected)
        } else {
            full_text
        };

        let target = self.active_target.clone();
        let php_bin = self.settings.php_binary.clone();
        self.eval_seq = self.eval_seq.wrapping_add(1);
        let seq = self.eval_seq;
        self.is_running = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let (result, fallback_syntax_diag) = cx
                .background_executor()
                .spawn(async move {
                    let res = evaluate_code_with_options(&code, &target, Some(&php_bin));
                    let fb = if res.status == "error"
                        && res.exception.as_ref().is_none_or(|e| e.line == 0)
                    {
                        lint_php_syntax_with_bin(&code, Some(&php_bin))
                    } else {
                        None
                    };
                    (res, fb)
                })
                .await;

            this.update(cx, |app, cx| {
                if app.eval_seq != seq {
                    return;
                }
                app.is_running = false;

                // Update run-only diagnostic in editor
                app.editor.update(cx, |ed, cx| {
                    let line_count = ed.text().split('\n').count().max(1);
                    let diag = fallback_syntax_diag.or_else(|| {
                        result
                            .exception
                            .as_ref()
                            .map(|exc| diagnostic_from_exception(exc, line_count))
                    });
                    ed.set_diagnostic(diag, cx);
                });

                // Seed expansions according to user setting
                let mut expanded = HashSet::new();
                match app.settings.detail_dive_expansion {
                    DetailDiveExpansion::FirstLevel => {
                        if let Some(ref ret) = result.return_value {
                            collect_default_expanded(&ret.node, &mut expanded);
                        }
                        for d in &result.dumps {
                            collect_default_expanded(&d.node, &mut expanded);
                        }
                    }
                    DetailDiveExpansion::CollapseAll => {}
                    DetailDiveExpansion::ExpandAll => {
                        if let Some(ref ret) = result.return_value {
                            collect_all_expanded(&ret.node, &mut expanded);
                        }
                        for d in &result.dumps {
                            collect_all_expanded(&d.node, &mut expanded);
                        }
                    }
                }
                app.expanded_nodes = expanded;
                app.selected_table_row = None;
                app.table_export_status = None;
                app.table_page = 0;
                if app.open_table_on_next_result {
                    app.open_table_on_next_result = false;
                    if let Some(tbl) = extract_first_table_from_result(&result) {
                        app.active_table_preview = Some(tbl);
                        app.overlay = OverlayKind::TablePreview;
                    }
                } else if app.overlay == OverlayKind::TablePreview
                    && let Some(tbl) = extract_first_table_from_result(&result)
                {
                    app.active_table_preview = Some(tbl);
                }

                if app.open_html_on_next_result {
                    app.open_html_on_next_result = false;
                    if let Some(html) = extract_first_html_from_result(&result) {
                        app.mail_webview.update(cx, |wv, cx| wv.set_html(html, cx));
                        app.overlay = OverlayKind::HtmlPreview;
                    }
                } else if app.overlay == OverlayKind::HtmlPreview
                    && let Some(html) = extract_first_html_from_result(&result)
                {
                    app.mail_webview.update(cx, |wv, cx| wv.set_html(html, cx));
                }

                app.last_result = Some(result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn schedule_auto_run(&mut self, cx: &mut Context<Self>) {
        if !self.auto_run || self.active_target.is_ssh() {
            return;
        }
        self.auto_run_seq = self.auto_run_seq.wrapping_add(1);
        let seq = self.auto_run_seq;

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(450))
                .await;
            this.update(cx, |app, cx| {
                if app.auto_run_seq == seq && app.auto_run && !app.active_target.is_ssh() {
                    app.run_evaluation(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn toggle_auto_run(&mut self, cx: &mut Context<Self>) {
        if self.active_target.is_ssh() {
            self.auto_run = false;
            cx.notify();
            return;
        }
        self.auto_run = !self.auto_run;
        self.settings.auto_evaluate_code = self.auto_run;
        save_settings(&self.settings);
        if self.auto_run {
            self.run_evaluation(cx);
        } else {
            cx.notify();
        }
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mail_webview.update(cx, |wv, _| wv.hide());
        self.log_menu_popup = LogMenuPopup::None;
        self.settings_search_input.update(cx, |tf, cx| {
            tf.set_text("", cx);
        });
        self.overlay = OverlayKind::Settings;
        let fh = self.settings_search_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    fn sync_settings_from_inputs(&mut self, cx: &mut Context<Self>) {
        if let Ok(indent) = self.indent_size_input.read(cx).text().trim().parse::<usize>()
            && (1..=16).contains(&indent)
        {
            self.settings.indentation_size = indent;
        }
        if let Ok(hist) = self
            .history_entries_input
            .read(cx)
            .text()
            .trim()
            .parse::<usize>()
            && (10..=2000).contains(&hist)
        {
            self.settings.history_entries = hist;
        }
        if let Ok(fs) = self.font_size_input.read(cx).text().trim().parse::<f32>()
            && (9.0..=32.0).contains(&fs)
        {
            self.settings.font_size = fs;
        }
        if let Ok(lh) = self.line_height_input.read(cx).text().trim().parse::<f32>()
            && (1.1..=3.0).contains(&lh)
        {
            self.settings.line_height_ratio = lh;
        }
        if let Ok(port) = self
            .autocomplete_port_input
            .read(cx)
            .text()
            .trim()
            .parse::<u16>()
            && port >= 1024
        {
            self.settings.autocompletion_service_port = port;
        }
        let snapshot = self.settings.clone();
        self.editor.update(cx, |ed, cx| {
            ed.apply_settings(&snapshot, cx);
        });
        save_settings(&self.settings);
        cx.notify();
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.settings.theme
    }

    fn persist_and_apply_settings(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.settings.clone();
        self.editor.update(cx, |ed, cx| {
            ed.apply_settings(&snapshot, cx);
        });
        save_settings(&self.settings);
        cx.notify();
    }

    fn select_php_binary_dialog(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select PHP Binary".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(chosen) = paths.into_iter().next()
            {
                let bin_str = chosen.to_string_lossy().to_string();
                let bin_for_check = bin_str.clone();
                let ver_line = cx
                    .background_executor()
                    .spawn(async move { detect_php_version_line(&bin_for_check) })
                    .await;
                this.update(cx, |app, cx| {
                    app.settings.php_binary = bin_str;
                    app.detected_php_version = ver_line;
                    save_settings(&app.settings);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Re-run PHP auto-detection (e.g. after the user installs PHP) and save the result.
    fn redetect_php_binary(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let (bin, ver_line) = cx
                .background_executor()
                .spawn(async move {
                    let bin = resolve_php_binary();
                    let ver = detect_php_version_line(&bin);
                    (bin, ver)
                })
                .await;
            this.update(cx, |app, cx| {
                if is_php_detected(&ver_line) {
                    app.settings.php_binary = bin;
                    save_settings(&app.settings);
                }
                app.detected_php_version = ver_line;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_default_workdir_dialog(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select Default Working Directory".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(chosen) = paths.into_iter().next()
            {
                let dir_str = chosen.to_string_lossy().to_string();
                this.update(cx, |app, cx| {
                    app.settings.default_working_directory = Some(dir_str);
                    save_settings(&app.settings);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    // ---- SSH Setup & Connection Manager Methods ----

    fn rebuild_execution_targets(&mut self) {
        let mut next = vec![ExecutionTarget::Local {
            project_path: self.active_project.clone(),
        }];
        next.extend(detect_docker_containers(&self.active_project));
        next.extend(
            self.ssh_store
                .connections
                .iter()
                .map(|c| c.to_execution_target()),
        );
        self.targets = next;
    }

    fn open_ssh_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mail_webview.update(cx, |wv, _| wv.hide());
        self.log_menu_popup = LogMenuPopup::None;
        self.ssh_config_hosts = load_system_ssh_config();
        self.active_dropdown_id = None;
        self.ssh_creating_group = false;
        self.ssh_form_error = None;
        self.ssh_modal_mode = SshModalMode::List;
        self.overlay = OverlayKind::SshConnections;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn start_new_ssh_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blank = SshConnectionProfile::new_empty();
        self.populate_ssh_form(&blank, cx);
        self.ssh_editing_id = None;
        self.ssh_modal_mode = SshModalMode::Form;
        let fh = self.ssh_label_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    fn start_edit_ssh_connection(
        &mut self,
        profile_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(existing) = self
            .ssh_store
            .connections
            .iter()
            .find(|c| c.id == profile_id)
            .cloned()
        else {
            return;
        };
        self.populate_ssh_form(&existing, cx);
        self.ssh_editing_id = Some(existing.id);
        self.ssh_modal_mode = SshModalMode::Form;
        let fh = self.ssh_label_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    fn populate_ssh_form(&mut self, p: &SshConnectionProfile, cx: &mut Context<Self>) {
        self.ssh_label_input.update(cx, |tf, cx| tf.set_text(&p.label, cx));
        self.ssh_host_input.update(cx, |tf, cx| tf.set_text(&p.host, cx));
        self.ssh_port_input
            .update(cx, |tf, cx| tf.set_text(p.port.to_string(), cx));
        self.ssh_user_input
            .update(cx, |tf, cx| tf.set_text(&p.username, cx));
        let folder_placeholder = if p.docker_enabled || p.is_local_docker() {
            default_docker_application_folder()
        } else {
            default_ssh_application_folder()
        };
        self.ssh_folder_input.update(cx, |tf, cx| {
            tf.set_placeholder(folder_placeholder, cx);
            tf.set_text(&p.application_folder, cx);
        });
        self.ssh_docker_container_input
            .update(cx, |tf, cx| tf.set_text(&p.docker_container, cx));
        self.ssh_key_input
            .update(cx, |tf, cx| tf.set_text(&p.private_key, cx));
        // Editing a saved connection is a use of it: read its secret now (cached afterwards).
        let passphrase = if p.passphrase.is_empty() && p.auth_method != SshAuthMethod::SshAgent {
            crate::ssh::ssh_secret(&p.id)
        } else {
            p.passphrase.clone()
        };
        self.ssh_passphrase_input.update(cx, |tf, cx| {
            tf.set_text(&passphrase, cx);
            tf.set_masked(true, cx);
        });
        self.ssh_form_kind = p.kind;
        self.ssh_form_docker_enabled = p.docker_enabled;
        self.ssh_docker_fetching = false;
        self.ssh_docker_picker_open = false;
        self.ssh_detected_containers.clear();
        self.ssh_form_local_path = p.local_path.clone();
        self.ssh_form_auth_method = p.auth_method;
        self.ssh_form_color_hex = p.color_hex;
        self.ssh_form_group = p.group.clone();
        self.ssh_selected_config_label = "Preselect SSH Config".to_string();
        self.active_dropdown_id = None;
        self.ssh_form_error = None;
    }

    fn set_ssh_form_kind(&mut self, kind: ConnectionKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.ssh_form_kind == kind {
            return;
        }
        self.ssh_form_kind = kind;
        self.ssh_docker_picker_open = false;
        self.ssh_detected_containers.clear();
        self.active_dropdown_id = None;
        self.ssh_form_error = None;
        let docker_folder = kind == ConnectionKind::LocalDocker || self.ssh_form_docker_enabled;
        let current_folder = self.ssh_folder_input.read(cx).text().trim().to_string();
        let (placeholder, swap_from) = if docker_folder {
            (default_docker_application_folder(), default_ssh_application_folder())
        } else {
            (default_ssh_application_folder(), default_docker_application_folder())
        };
        self.ssh_folder_input.update(cx, |tf, cx| {
            tf.set_placeholder(placeholder, cx);
            if current_folder.is_empty() || current_folder == swap_from {
                tf.set_text(placeholder, cx);
            }
        });
        // A new connection defaults to red ("production"), which turns Auto-Run off. A local
        // dev container is not production, so start it on a calmer color.
        if self.ssh_editing_id.is_none() {
            if kind == ConnectionKind::LocalDocker && self.ssh_form_color_hex == default_ssh_color() {
                self.ssh_form_color_hex = 0x38bdf8;
            } else if kind == ConnectionKind::Ssh && self.ssh_form_color_hex == 0x38bdf8 {
                self.ssh_form_color_hex = default_ssh_color();
            }
        }
        let fh = if kind == ConnectionKind::LocalDocker {
            self.ssh_docker_container_input.focus_handle(cx)
        } else {
            self.ssh_host_input.focus_handle(cx)
        };
        window.focus(&fh);
        cx.notify();
    }

    fn fetch_local_docker_containers(&mut self, cx: &mut Context<Self>) {
        if self.ssh_docker_fetching {
            return;
        }
        self.ssh_docker_fetching = true;
        self.ssh_docker_picker_open = false;
        self.ssh_form_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { detect_local_docker_containers() })
                .await;
            this.update(cx, |app, cx| {
                app.ssh_docker_fetching = false;
                match outcome {
                    Ok(list) => {
                        app.ssh_detected_containers = list;
                        app.ssh_docker_picker_open = true;
                    }
                    Err(err) => {
                        app.ssh_detected_containers.clear();
                        app.ssh_form_error =
                            Some(format!("Could not fetch Docker containers: {err}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_ssh_form_docker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = !self.ssh_form_docker_enabled;
        self.ssh_form_docker_enabled = next;
        self.ssh_docker_picker_open = false;
        self.ssh_form_error = None;
        let current_folder = self.ssh_folder_input.read(cx).text().trim().to_string();
        if next {
            self.ssh_folder_input.update(cx, |tf, cx| {
                tf.set_placeholder(default_docker_application_folder(), cx);
                if current_folder.is_empty() || current_folder == default_ssh_application_folder() {
                    tf.set_text(default_docker_application_folder(), cx);
                }
            });
            let fh = self.ssh_docker_container_input.focus_handle(cx);
            window.focus(&fh);
        } else {
            self.ssh_folder_input.update(cx, |tf, cx| {
                tf.set_placeholder(default_ssh_application_folder(), cx);
                if current_folder == default_docker_application_folder() {
                    tf.set_text(default_ssh_application_folder(), cx);
                }
            });
        }
        cx.notify();
    }

    fn fetch_ssh_docker_containers(&mut self, cx: &mut Context<Self>) {
        if self.ssh_form_kind == ConnectionKind::LocalDocker {
            self.fetch_local_docker_containers(cx);
            return;
        }
        if self.ssh_docker_fetching {
            return;
        }
        let host = self.ssh_host_input.read(cx).text().trim().to_string();
        if host.is_empty() {
            self.ssh_form_error = Some(
                "Enter Host (and SSH credentials) first to fetch running Docker containers, or type a container name/ID directly."
                    .to_string(),
            );
            cx.notify();
            return;
        }
        let port = self
            .ssh_port_input
            .read(cx)
            .text()
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .unwrap_or(22);
        let user_raw = self.ssh_user_input.read(cx).text().trim().to_string();
        let user = if user_raw.is_empty() {
            "forge".to_string()
        } else {
            user_raw
        };
        let auth_method = self.ssh_form_auth_method;
        let private_key_raw = self.ssh_key_input.read(cx).text().trim().to_string();
        let private_key = if private_key_raw.is_empty() {
            default_private_key_path()
        } else {
            private_key_raw
        };
        let passphrase = self.ssh_passphrase_input.read(cx).text().to_string();

        self.ssh_docker_fetching = true;
        self.ssh_docker_picker_open = false;
        self.ssh_form_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    detect_remote_ssh_docker_containers(
                        &host,
                        port,
                        &user,
                        auth_method,
                        &private_key,
                        &passphrase,
                    )
                })
                .await;

            this.update(cx, |app, cx| {
                app.ssh_docker_fetching = false;
                match outcome {
                    Ok(list) => {
                        app.ssh_detected_containers = list;
                        app.ssh_docker_picker_open = true;
                        app.ssh_form_error = None;
                    }
                    Err(err) => {
                        app.ssh_detected_containers.clear();
                        app.ssh_docker_picker_open = false;
                        app.ssh_form_error = Some(format!(
                            "Could not fetch Docker containers: {err}"
                        ));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_ssh_config_host(&mut self, cfg: &SshConfigHost, cx: &mut Context<Self>) {
        self.ssh_selected_config_label = format!("{} ({})", cfg.alias, cfg.hostname);
        if self.ssh_label_input.read(cx).text().trim().is_empty() {
            self.ssh_label_input
                .update(cx, |tf, cx| tf.set_text(&cfg.alias, cx));
        }
        self.ssh_host_input
            .update(cx, |tf, cx| tf.set_text(&cfg.hostname, cx));
        self.ssh_port_input.update(cx, |tf, cx| {
            tf.set_text(cfg.port.unwrap_or(22).to_string(), cx);
        });
        if let Some(ref u) = cfg.user {
            self.ssh_user_input.update(cx, |tf, cx| tf.set_text(u, cx));
        }
        if let Some(ref key_file) = cfg.identity_file {
            self.ssh_key_input
                .update(cx, |tf, cx| tf.set_text(key_file, cx));
            self.ssh_form_auth_method = SshAuthMethod::PrivateKey;
        }
        self.active_dropdown_id = None;
        cx.notify();
    }

    fn select_ssh_local_path_dialog(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select Local Mirror Folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(chosen) = paths.into_iter().next()
            {
                let dir_str = chosen.to_string_lossy().to_string();
                this.update(cx, |app, cx| {
                    app.ssh_form_local_path = Some(dir_str);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn select_ssh_private_key_dialog(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select SSH Private Key".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(chosen) = paths.into_iter().next()
            {
                let file_str = chosen.to_string_lossy().to_string();
                this.update(cx, |app, cx| {
                    app.ssh_key_input
                        .update(cx, |tf, cx| tf.set_text(file_str, cx));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn save_ssh_form(&mut self, and_connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.ssh_form_kind == ConnectionKind::LocalDocker {
            self.save_local_docker_form(and_connect, window, cx);
            return;
        }
        let host = self.ssh_host_input.read(cx).text().trim().to_string();
        if host.is_empty() {
            self.ssh_form_error = Some("Please enter a Host (IP address or domain).".to_string());
            cx.notify();
            return;
        }

        let username_raw = self.ssh_user_input.read(cx).text().trim().to_string();
        let username = if username_raw.is_empty() {
            "forge".to_string()
        } else {
            username_raw
        };
        let label_raw = self.ssh_label_input.read(cx).text().trim().to_string();
        let label = if label_raw.is_empty() {
            format!("{username}@{host}")
        } else {
            label_raw
        };
        let port = self
            .ssh_port_input
            .read(cx)
            .text()
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .unwrap_or(22);
        if let Err(err) = crate::ssh::validate_ssh_destination(&host, &username) {
            self.ssh_form_error = Some(err);
            cx.notify();
            return;
        }
        let docker_enabled = self.ssh_form_docker_enabled;
        let docker_container = self
            .ssh_docker_container_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        if docker_enabled && docker_container.is_empty() {
            self.ssh_form_error = Some(
                "Please enter a Docker container name or ID (or switch off Docker).".to_string(),
            );
            cx.notify();
            return;
        }

        let folder_raw = self.ssh_folder_input.read(cx).text().trim().to_string();
        let application_folder = if folder_raw.is_empty() {
            default_docker_application_folder().to_string()
        } else {
            folder_raw
        };
        if docker_enabled && !application_folder.starts_with('/') {
            self.ssh_form_error = Some(
                "Docker Application Folder must be an absolute container path (e.g. /var/www/html)."
                    .to_string(),
            );
            cx.notify();
            return;
        }
        let private_key = self.ssh_key_input.read(cx).text().trim().to_string();
        let passphrase = self.ssh_passphrase_input.read(cx).text().to_string();

        let id = self
            .ssh_editing_id
            .clone()
            .unwrap_or_else(generate_ssh_id);

        let profile = SshConnectionProfile {
            id: id.clone(),
            kind: ConnectionKind::Ssh,
            label,
            host,
            port,
            username,
            application_folder,
            docker_enabled,
            docker_container,
            local_path: self.ssh_form_local_path.clone(),
            auth_method: self.ssh_form_auth_method,
            private_key: if private_key.is_empty() {
                default_private_key_path()
            } else {
                private_key
            },
            passphrase,
            color_hex: self.ssh_form_color_hex,
            group: self.ssh_form_group.clone(),
        };

        self.commit_connection_profile(profile, and_connect, window, cx);
    }

    fn save_local_docker_form(
        &mut self,
        and_connect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let container = self
            .ssh_docker_container_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        if container.is_empty() {
            self.ssh_form_error =
                Some("Please enter a Docker container name or ID (or use Fetch).".to_string());
            cx.notify();
            return;
        }
        if container.starts_with('-') || container.chars().any(char::is_whitespace) {
            self.ssh_form_error =
                Some("Container name or ID cannot start with '-' or contain spaces.".to_string());
            cx.notify();
            return;
        }
        let folder_raw = self.ssh_folder_input.read(cx).text().trim().to_string();
        let application_folder = if folder_raw.is_empty() {
            default_docker_application_folder().to_string()
        } else {
            folder_raw
        };
        if !application_folder.starts_with('/') {
            self.ssh_form_error = Some(
                "Application Folder must be an absolute container path (e.g. /var/www/html)."
                    .to_string(),
            );
            cx.notify();
            return;
        }
        let label_raw = self.ssh_label_input.read(cx).text().trim().to_string();
        let id = self.ssh_editing_id.clone().unwrap_or_else(generate_ssh_id);
        let mut profile = SshConnectionProfile::new_empty();
        profile.id = id;
        profile.kind = ConnectionKind::LocalDocker;
        profile.label = if label_raw.is_empty() {
            container.clone()
        } else {
            label_raw
        };
        profile.host = String::new();
        profile.application_folder = application_folder;
        profile.docker_enabled = true;
        profile.docker_container = container;
        profile.local_path = self.ssh_form_local_path.clone();
        profile.auth_method = SshAuthMethod::SshAgent;
        profile.color_hex = self.ssh_form_color_hex;
        profile.group = self.ssh_form_group.clone();
        self.commit_connection_profile(profile, and_connect, window, cx);
    }

    fn commit_connection_profile(
        &mut self,
        profile: SshConnectionProfile,
        and_connect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = profile.id.clone();
        if let Some(pos) = self
            .ssh_store
            .connections
            .iter()
            .position(|c| c.id == id)
        {
            self.ssh_store.connections[pos] = profile.clone();
        } else {
            self.ssh_store.connections.insert(0, profile.clone());
        }
        if profile.passphrase.is_empty() {
            // The form held the saved secret (or none), so an empty field means "remove it".
            if let Err(err) = crate::ssh::store_ssh_secret(&profile.id, "") {
                eprintln!("Laralah: failed to clear SSH secret: {err}");
            }
        }
        save_ssh_store(&self.ssh_store);
        self.rebuild_execution_targets();
        self.ssh_form_error = None;

        // If we edited the currently active SSH connection, refresh active_target metadata immediately
        if let ExecutionTarget::Ssh { id: active_id, .. } = &self.active_target
            && active_id == &profile.id
        {
            self.active_target = profile.to_execution_target();
        }

        if and_connect {
            self.connect_ssh_profile(&profile, window, cx);
        } else {
            self.ssh_modal_mode = SshModalMode::List;
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    fn connect_ssh_profile(
        &mut self,
        profile: &SshConnectionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_target = profile.to_execution_target();
        self.auto_run = false;
        self.show_landing = false;
        if let Some(ref lp) = profile.local_path
            && std::path::Path::new(lp).is_dir()
        {
            self.detected_laravel_version = detect_local_laravel_version(lp);
            self.attach_project_lsp(lp, cx);
        }
        self.close_overlay(window, cx);
        cx.notify();
    }

    fn disconnect_ssh(&mut self, cx: &mut Context<Self>) {
        self.active_target = ExecutionTarget::Local {
            project_path: self.active_project.clone(),
        };
        self.detected_laravel_version = detect_local_laravel_version(&self.active_project);
        self.attach_project_lsp(&self.active_project.clone(), cx);
        cx.notify();
    }

    fn delete_ssh_profile(&mut self, id: &str, cx: &mut Context<Self>) {
        let was_active = matches!(&self.active_target, ExecutionTarget::Ssh { id: aid, .. } if aid == id);
        self.ssh_store.connections.retain(|c| c.id != id);
        crate::ssh::delete_ssh_secret(id);
        save_ssh_store(&self.ssh_store);
        self.rebuild_execution_targets();
        if was_active {
            self.disconnect_ssh(cx);
        } else {
            cx.notify();
        }
    }

    fn create_ssh_group(&mut self, cx: &mut Context<Self>) {
        let name = self.ssh_group_input.read(cx).text().trim().to_string();
        if !name.is_empty() && !self.ssh_store.groups.iter().any(|g| g.eq_ignore_ascii_case(&name)) {
            self.ssh_store.groups.push(name);
            save_ssh_store(&self.ssh_store);
        }
        self.ssh_group_input.update(cx, |tf, cx| tf.set_text("", cx));
        self.ssh_creating_group = false;
        cx.notify();
    }

    fn open_palette(&mut self, initial: &str, window: &mut Window, cx: &mut Context<Self>) {
        if initial.starts_with('@') {
            let mut next = vec![ExecutionTarget::Local {
                project_path: self.active_project.clone(),
            }];
            next.extend(detect_docker_containers(&self.active_project));
            next.extend(default_ssh_targets());
            self.targets = next;
        }
        self.palette_input.update(cx, |f, cx| {
            f.set_text(initial, cx);
        });
        self.palette_selected_idx = 0;
        self.overlay = OverlayKind::CommandPalette;
        let fh = self.palette_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    fn open_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mail_webview.update(cx, |wv, _| wv.hide());
        self.log_menu_popup = LogMenuPopup::None;
        self.refresh_logs(cx);
        self.overlay = OverlayKind::LogViewer;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn choose_logs_folder_dialog(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select Logs Folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(chosen) = paths.into_iter().next()
            {
                this.update(cx, |app, cx| {
                    app.custom_logs_folder = Some(chosen);
                    app.active_log_file_idx = 0;
                    app.log_page = 0;
                    app.log_menu_popup = LogMenuPopup::None;
                    app.refresh_logs(cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn open_table_preview(
        &mut self,
        table: TableDataset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.table_search_input.update(cx, |f, cx| {
            f.set_text("", cx);
        });
        self.table_page = 0;
        self.table_sort_col = None;
        self.table_sort_asc = true;
        self.selected_table_row = None;
        self.table_export_status = None;
        self.active_dropdown_id = None;
        self.active_table_preview = Some(table);
        self.overlay = OverlayKind::TablePreview;
        let fh = self.table_search_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    fn open_object_graph(
        &mut self,
        node: ObjectNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mail_webview.update(cx, |wv, _| wv.hide());
        self.object_graph_view.update(cx, |g, cx| {
            g.set_root(node, cx);
        });
        self.overlay = OverlayKind::ObjectGraph;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn open_html_preview(
        &mut self,
        html: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mail_webview.update(cx, |wv, cx| {
            wv.set_html(html, cx);
        });
        self.overlay = OverlayKind::HtmlPreview;
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mail_webview.update(cx, |wv, _| wv.hide());
        self.overlay = OverlayKind::None;
        self.active_dropdown_id = None;
        self.table_export_status = None;
        self.log_menu_popup = LogMenuPopup::None;
        self.log_poll_interval = LogPollInterval::Off;
        self.live_tail_epoch = self.live_tail_epoch.wrapping_add(1);
        let fh = if self.show_landing {
            self.focus_handle.clone()
        } else {
            self.editor.focus_handle(cx)
        };
        window.focus(&fh);
        cx.notify();
    }

    fn refresh_logs(&mut self, cx: &mut Context<Self>) {
        self.log_files = if let Some(ref folder) = self.custom_logs_folder {
            list_log_files_in_folder(folder)
        } else {
            list_log_files(&self.active_project)
        };
        if self.active_log_file_idx >= self.log_files.len() {
            self.active_log_file_idx = 0;
        }
        self.log_entries = self
            .log_files
            .get(self.active_log_file_idx)
            .map(|p| parse_log_file(p))
            .unwrap_or_default();
        cx.notify();
    }

    fn set_log_poll_interval(&mut self, interval: LogPollInterval, cx: &mut Context<Self>) {
        self.log_poll_interval = interval;
        self.log_menu_popup = LogMenuPopup::None;
        self.live_tail_epoch = self.live_tail_epoch.wrapping_add(1);
        let epoch = self.live_tail_epoch;
        if let Some(dur) = interval.duration() {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(dur).await;
                    let keep = this
                        .update(cx, |app, cx| {
                            if app.log_poll_interval == interval
                                && app.live_tail_epoch == epoch
                                && app.overlay == OverlayKind::LogViewer
                            {
                                app.refresh_logs(cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    fn current_palette_entries(&self, cx: &App) -> Vec<PaletteEntry> {
        let query = self.palette_input.read(cx).text();
        let mut entries = filter_palette_entries(query, &self.projects, &self.targets);
        if let Some(raw_path) = query.trim().strip_prefix('/') {
            let candidate = raw_path.trim();
            if candidate.starts_with('/') && is_laravel_project(candidate) {
                entries.insert(
                    0,
                    PaletteEntry {
                        kind: PaletteItemKind::Project,
                        prefix_tag: "/",
                        title: format!("Open Laravel Project: {candidate}"),
                        subtitle: "Verified artisan + bootstrap/app.php".to_string(),
                        shortcut: "Open",
                        action: PaletteAction::SwitchProject(candidate.to_string()),
                    },
                );
            }
        }
        entries
    }

    fn execute_palette_action(
        &mut self,
        action: PaletteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlay(window, cx);
        match action {
            PaletteAction::OpenDirectoryDialog => {
                self.open_directory_dialog(cx);
            }
            PaletteAction::ShowLandingPage => {
                self.show_landing = true;
                window.focus(&self.focus_handle);
                cx.notify();
            }
            PaletteAction::OpenSettings => {
                self.open_settings(window, cx);
            }
            PaletteAction::OpenAiSettings => {
                self.active_settings_tab = SettingsTab::Ai;
                self.open_settings(window, cx);
            }
            PaletteAction::ToggleAiSidebar => {
                self.on_toggle_ai_sidebar(&ToggleAiSidebar, window, cx);
            }
            PaletteAction::OpenSshManager => {
                self.open_ssh_modal(window, cx);
            }
            PaletteAction::SwitchProject(path) => {
                self.open_project_path(path, cx);
            }
            PaletteAction::SwitchTarget(target) => {
                if target.is_ssh() {
                    self.auto_run = false;
                }
                self.active_target = target;
                self.show_landing = false;
                self.focus_editor_next_frame = true;
                cx.notify();
            }
            PaletteAction::RunCode => {
                self.show_landing = false;
                self.focus_editor_next_frame = true;
                self.run_evaluation(cx);
            }
            PaletteAction::ToggleAutoRun => self.toggle_auto_run(cx),
            PaletteAction::SwitchOutputMode(mode) => {
                self.show_landing = false;
                match mode {
                    OutputMode::SqlQueries => {
                        self.output_mode = OutputMode::DetailDive;
                        self.prev_non_cli_mode = OutputMode::DetailDive;
                        self.settings.show_sql_inline = true;
                        save_settings(&self.settings);
                    }
                    OutputMode::MailablePreview => {
                        if let Some(html) = self
                            .last_result
                            .as_ref()
                            .and_then(extract_first_html_from_result)
                        {
                            self.open_html_preview(html, window, cx);
                            return;
                        }
                        self.output_mode = OutputMode::DetailDive;
                        self.prev_non_cli_mode = OutputMode::DetailDive;
                    }
                    OutputMode::RawCli => {
                        self.toggle_cli_mode(cx);
                        return;
                    }
                    OutputMode::TableMode => {
                        if let Some(tbl) = self
                            .last_result
                            .as_ref()
                            .and_then(extract_first_table_from_result)
                        {
                            self.open_table_preview(tbl, window, cx);
                            return;
                        }
                        self.output_mode = OutputMode::DetailDive;
                    }
                    other => {
                        self.output_mode = other;
                        self.prev_non_cli_mode = other;
                    }
                }
                cx.notify();
            }
            PaletteAction::OpenLogViewer => self.open_logs(window, cx),
            PaletteAction::NewTab => {
                self.create_new_tab(cx);
            }
            PaletteAction::ClearOutput => {
                self.show_landing = false;
                self.focus_editor_next_frame = true;
                self.last_result = None;
                self.expanded_nodes.clear();
                cx.notify();
            }
            PaletteAction::CloseActiveTab => {
                let idx = self.active_tab_idx;
                self.close_tab_at(idx, cx);
            }
            PaletteAction::ExportTableCsv => self.export_table(true, cx),
            PaletteAction::ExportTableJson => self.export_table(false, cx),
        }
    }

    fn active_table_with_indices(&self, cx: &App) -> Option<(TableDataset, Vec<usize>)> {
        let table = self
            .active_table_preview
            .clone()
            .filter(|_| self.overlay == OverlayKind::TablePreview)
            .or_else(|| {
                self.last_result
                    .as_ref()
                    .and_then(extract_first_table_from_result)
            })?;
        let apply_view_state =
            self.overlay == OverlayKind::TablePreview || self.output_mode == OutputMode::TableMode;
        let indices = if apply_view_state {
            let needle = self.table_search_input.read(cx).text().trim().to_lowercase();
            let mut idxs: Vec<usize> = (0..table.rows.len())
                .filter(|&i| {
                    needle.is_empty()
                        || table.rows[i]
                            .iter()
                            .any(|c| c.to_lowercase().contains(&needle))
                })
                .collect();
            if let Some(col_idx) = self.table_sort_col {
                let asc = self.table_sort_asc;
                idxs.sort_by(|&a, &b| {
                    let va = table.rows[a].get(col_idx).map(|s| s.as_str()).unwrap_or("");
                    let vb = table.rows[b].get(col_idx).map(|s| s.as_str()).unwrap_or("");
                    let cmp = match (va.parse::<f64>(), vb.parse::<f64>()) {
                        (Ok(na), Ok(nb)) => {
                            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        _ => va.to_lowercase().cmp(&vb.to_lowercase()),
                    };
                    if asc { cmp } else { cmp.reverse() }
                });
            }
            idxs
        } else {
            (0..table.rows.len()).collect()
        };
        Some((table, indices))
    }

    fn default_export_directory(&self) -> PathBuf {
        if let Some(home_path) = crate::paths::home_dir() {
            let downloads = home_path.join("Downloads");
            if downloads.is_dir() {
                return downloads;
            }
            let desktop = home_path.join("Desktop");
            if desktop.is_dir() {
                return desktop;
            }
            if home_path.is_dir() {
                return home_path;
            }
        }
        if !self.active_target.is_ssh() && !self.active_project.is_empty() {
            let proj = PathBuf::from(&self.active_project);
            if proj.is_dir() {
                return proj;
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp"))
    }

    fn copy_table_data(&mut self, as_csv: bool, cx: &mut Context<Self>) {
        let Some((table, indices)) = self.active_table_with_indices(cx) else {
            return;
        };
        let content = if as_csv {
            table_indices_to_csv(&table, &indices)
        } else {
            table_indices_to_json(&table, &indices)
        };
        let count = indices.len();
        let fmt_label = if as_csv { "CSV" } else { "JSON" };
        cx.write_to_clipboard(ClipboardItem::new_string(content));
        self.table_export_status = Some((
            format!(
                "Copied {count} {} as {fmt_label}",
                if count == 1 { "row" } else { "rows" }
            ),
            false,
        ));
        cx.notify();
    }

    fn export_table(&mut self, as_csv: bool, cx: &mut Context<Self>) {
        let Some((table, indices)) = self.active_table_with_indices(cx) else {
            return;
        };
        let csv_content = table_indices_to_csv(&table, &indices);
        let json_content = table_indices_to_json(&table, &indices);
        let default_copy = if as_csv {
            csv_content.clone()
        } else {
            json_content.clone()
        };
        cx.write_to_clipboard(ClipboardItem::new_string(default_copy));

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ext = if as_csv { "csv" } else { "json" };
        let suggested_name = format!("laralah_export_{ts}.{ext}");
        let export_dir = self.default_export_directory();
        let receiver = cx.prompt_for_new_path(&export_dir, Some(&suggested_name));

        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(chosen_path))) = receiver.await {
                let final_path = if chosen_path.extension().is_none() {
                    chosen_path.with_extension(ext)
                } else {
                    chosen_path
                };
                let use_json = final_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"));
                let payload = if use_json { json_content } else { csv_content };

                let write_res = if let Some(parent) = final_path.parent() {
                    std::fs::create_dir_all(parent).and_then(|_| std::fs::write(&final_path, payload))
                } else {
                    std::fs::write(&final_path, payload)
                };

                this.update(cx, |app, cx| {
                    match write_res {
                        Ok(()) => {
                            let fname = final_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| final_path.to_string_lossy().to_string());
                            app.table_export_status = Some((format!("Saved {fname}"), false));
                        }
                        Err(err) => {
                            app.table_export_status = Some((format!("Export failed: {err}"), true));
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    // ---- Keyboard Action Handlers ----

    fn on_quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }
    fn on_run(&mut self, _: &RunCode, _: &mut Window, cx: &mut Context<Self>) {
        if self.show_landing {
            if let Some(first_proj) = self.projects.first().cloned() {
                self.open_project_path(first_proj, cx);
            } else {
                self.open_directory_dialog(cx);
            }
        } else {
            self.run_evaluation(cx);
        }
    }
    fn on_open_directory(&mut self, _: &OpenDirectory, _: &mut Window, cx: &mut Context<Self>) {
        self.open_directory_dialog(cx);
    }
    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.show_explorer {
            return;
        }
        self.show_landing = false;
        self.show_sidebar = !self.show_sidebar;
        cx.notify();
    }
    fn on_new_tab(&mut self, _: &NewScratchTab, _: &mut Window, cx: &mut Context<Self>) {
        self.create_new_tab(cx);
    }
    fn on_close_tab(&mut self, _: &CloseScratchTab, _: &mut Window, cx: &mut Context<Self>) {
        let idx = self.active_tab_idx;
        self.close_tab_at(idx, cx);
    }
    fn on_prev_tab(&mut self, _: &PrevScratchTab, _: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let prev = if self.active_tab_idx == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab_idx - 1
            };
            self.switch_to_tab(prev, cx);
        }
    }
    fn on_next_tab(&mut self, _: &NextScratchTab, _: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() > 1 {
            let next = (self.active_tab_idx + 1) % self.tabs.len();
            self.switch_to_tab(next, cx);
        }
    }
    fn on_toggle_settings(&mut self, _: &ToggleSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == OverlayKind::Settings {
            self.close_overlay(window, cx);
        } else {
            self.open_settings(window, cx);
        }
    }
    fn on_toggle_ssh_modal(
        &mut self,
        _: &ToggleSshModal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay == OverlayKind::SshConnections {
            self.close_overlay(window, cx);
        } else {
            self.open_ssh_modal(window, cx);
        }
    }
    fn on_focus_settings_search(
        &mut self,
        _: &FocusSettingsSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay == OverlayKind::Settings {
            let fh = self.settings_search_input.focus_handle(cx);
            window.focus(&fh);
            cx.notify();
        }
    }
    fn on_focus_ssh_search(
        &mut self,
        _: &FocusSshSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay == OverlayKind::SshConnections && self.ssh_modal_mode == SshModalMode::List {
            let fh = self.ssh_search_input.focus_handle(cx);
            window.focus(&fh);
            cx.notify();
        }
    }
    fn on_toggle_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == OverlayKind::CommandPalette {
            self.close_overlay(window, cx);
        } else {
            self.open_palette("", window, cx);
        }
    }
    fn on_toggle_logs(&mut self, _: &ToggleLogs, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == OverlayKind::LogViewer {
            self.close_overlay(window, cx);
        } else {
            self.open_logs(window, cx);
        }
    }
    fn on_focus_log_search(
        &mut self,
        _: &FocusLogSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay == OverlayKind::LogViewer {
            self.log_menu_popup = LogMenuPopup::None;
            let fh = self.log_search_input.focus_handle(cx);
            window.focus(&fh);
            cx.notify();
        }
    }
    fn on_toggle_sql(&mut self, _: &ToggleSql, _: &mut Window, cx: &mut Context<Self>) {
        self.settings.show_sql_inline = !self.settings.show_sql_inline;
        save_settings(&self.settings);
        cx.notify();
    }
    fn toggle_cli_mode(&mut self, cx: &mut Context<Self>) {
        if self.output_mode == OutputMode::RawCli {
            self.output_mode = self.prev_non_cli_mode;
        } else {
            self.prev_non_cli_mode = self.output_mode;
            self.output_mode = OutputMode::RawCli;
        }
        cx.notify();
    }

    fn on_mode_detail(&mut self, _: &ModeDetailDive, _: &mut Window, cx: &mut Context<Self>) {
        self.output_mode = OutputMode::DetailDive;
        self.prev_non_cli_mode = OutputMode::DetailDive;
        cx.notify();
    }
    fn on_mode_table(&mut self, _: &ModeTable, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == OverlayKind::TablePreview {
            self.close_overlay(window, cx);
            return;
        }
        if let Some(tbl) = self
            .last_result
            .as_ref()
            .and_then(extract_first_table_from_result)
        {
            self.open_table_preview(tbl, window, cx);
        }
    }
    fn on_mode_raw(&mut self, _: &ModeRawCli, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_cli_mode(cx);
    }
    fn on_close_overlay(&mut self, _: &CloseOverlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_effort_menu_open {
            self.ai_effort_menu_open = false;
            cx.notify();
            return;
        }
        if self.ai_sessions_menu_open {
            self.ai_sessions_menu_open = false;
            cx.notify();
            return;
        }
        if self.ai_picker_open {
            self.ai_picker_open = false;
            cx.notify();
            return;
        }
        if self.tab_context_menu.is_some() {
            self.tab_context_menu = None;
            cx.notify();
            return;
        }
        if self.active_dropdown_id.is_some() {
            self.active_dropdown_id = None;
            cx.notify();
            return;
        }
        if self.overlay == OverlayKind::Settings
            && !self.settings_search_input.read(cx).text().trim().is_empty()
        {
            self.settings_search_input.update(cx, |tf, cx| {
                tf.set_text("", cx);
            });
            let fh = self.settings_search_input.focus_handle(cx);
            window.focus(&fh);
            cx.notify();
            return;
        }
        if self.overlay != OverlayKind::None {
            self.close_overlay(window, cx);
        }
    }
    fn on_graph_zoom_in(&mut self, _: &GraphZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == OverlayKind::ObjectGraph {
            self.object_graph_view.update(cx, |g, cx| g.zoom_in(cx));
        }
    }
    fn on_graph_zoom_out(&mut self, _: &GraphZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == OverlayKind::ObjectGraph {
            self.object_graph_view.update(cx, |g, cx| g.zoom_out(cx));
        }
    }
    fn on_graph_fit(&mut self, _: &GraphFitView, _: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == OverlayKind::ObjectGraph {
            self.object_graph_view.update(cx, |g, cx| g.fit_view(cx));
        }
    }
    fn on_palette_up(&mut self, _: &PaletteUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_selected_idx > 0 {
            self.palette_selected_idx -= 1;
            cx.notify();
        }
    }
    fn on_palette_down(&mut self, _: &PaletteDown, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.current_palette_entries(cx).len();
        if count > 0 && self.palette_selected_idx + 1 < count {
            self.palette_selected_idx += 1;
            cx.notify();
        }
    }
    fn on_palette_confirm(
        &mut self,
        _: &PaletteConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = self.current_palette_entries(cx);
        if let Some(entry) = entries.get(self.palette_selected_idx).cloned() {
            self.execute_palette_action(entry.action, window, cx);
        }
    }

    // ==================== CLEAN UI RENDERING ====================

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.show_landing {
            return div()
                .h(px(40.0))
                .w_full()
                .bg(rgb(BG_WINDOW))
                .border_b_1()
                .border_color(rgb(BORDER_LIGHT))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .window_control_area(gpui::WindowControlArea::Drag)
                .on_mouse_down(gpui::MouseButton::Left, titlebar_double_click)
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(TEXT_SECONDARY))
                        .child("Laralah – Get Started"),
                )
                .into_any_element();
        }

        let project_name = std::path::Path::new(&self.active_project)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("laravel")
            .to_string();

        let (conn_label, conn_dot) = match &self.active_target {
            ExecutionTarget::Local { .. } => ("Local".to_string(), COLOR_SUCCESS),
            ExecutionTarget::Docker { container_name, .. } => {
                (format!("Docker · {container_name}"), 0x0284c7)
            }
            ExecutionTarget::Ssh {
                name,
                local_docker: true,
                color_hex,
                ..
            } => (format!("Docker · {name}"), *color_hex),
            ExecutionTarget::Ssh {
                name,
                docker_container,
                color_hex,
                ..
            } => {
                let lbl = match docker_container.as_deref().filter(|s| !s.is_empty()) {
                    Some(c) => format!("SSH · {name} ({c})"),
                    None => format!("SSH · {name}"),
                };
                (lbl, *color_hex)
            }
        };

        let auto_active = self.auto_run && !self.active_target.is_ssh();
        let sql_count = self
            .last_result
            .as_ref()
            .map(|r| r.sql_queries.len())
            .unwrap_or(0);
        let has_n1 = self
            .last_result
            .as_ref()
            .is_some_and(|r| r.sql_queries.iter().any(|q| q.is_duplicate));
        div()
            .h(px(42.0))
            .w_full()
            .bg(rgb(BG_WINDOW))
            .border_b_1()
            .border_color(rgb(BORDER_LIGHT))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_3()
            .pl(px(80.0))
            .pr_3()
            // Left cluster: Sidebar toggle + Project selector pill + Target badge
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .when(self.settings.show_explorer, |row| {
                        row.child(
                            div()
                                .id("top-toggle-sidebar")
                                .size(px(28.0))
                                .rounded_md()
                                .bg(if self.show_sidebar {
                                    rgb(0xe2e8f0)
                                } else {
                                    rgba(0x00000000)
                                })
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_sidebar = !this.show_sidebar;
                                    cx.notify();
                                }))
                                .child(
                                    svg()
                                        .path("icons/sidebar.svg")
                                        .size(px(15.5))
                                        .text_color(if self.show_sidebar {
                                            rgb(0x0f172a)
                                        } else {
                                            rgb(TEXT_SECONDARY)
                                        }),
                                ),
                        )
                    })
                    .child(
                        div()
                            .id("titlebar-project-btn")
                            .px_2p5()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0xf8fafc))
                            .border_1()
                            .border_color(rgb(0xe2e8f0))
                            .hover(|s| s.bg(rgb(BG_HOVER)).border_color(rgb(0xcbd5e1)))
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_palette("/", window, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/folder.svg")
                                    .size(px(13.5))
                                    .text_color(rgb(0x475569)),
                            )
                            .child(
                                div()
                                    .max_w(px(190.0))
                                    .truncate()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT_MAIN))
                                    .child(project_name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(TEXT_MUTED))
                                    .child("▾"),
                            ),
                    )
                    .child(
                        div()
                            .id("titlebar-target-btn")
                            .px_2p5()
                            .py_1()
                            .rounded_md()
                            .bg(if self.active_target.is_ssh() {
                                rgb(0xfef2f2)
                            } else {
                                rgb(0xf8fafc)
                            })
                            .border_1()
                            .border_color(if self.active_target.is_ssh() {
                                rgb(0xfecaca)
                            } else {
                                rgb(0xe2e8f0)
                            })
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_ssh_modal(window, cx);
                            }))
                            .child(div().size(px(7.5)).rounded_full().bg(rgb(conn_dot)))
                            .child(
                                div()
                                    .max_w(px(150.0))
                                    .truncate()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(if self.active_target.is_ssh() {
                                        rgb(0x991b1b)
                                    } else {
                                        rgb(TEXT_SECONDARY)
                                    })
                                    .child(conn_label),
                            ),
                    ),
            )
            // Center cluster: Horizontally scrollable Multi-tab Scratchpad strip + pinned `+` & All-Tabs menu
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .window_control_area(gpui::WindowControlArea::Drag)
                    .on_mouse_down(gpui::MouseButton::Left, titlebar_double_click),
            )
            // Right cluster: `[ Dive | CLI ]` · SQL · Auto · ▶ Run · AI (single blue accent; amber only for N+1)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    // Segmented View Switcher: `[ Dive | CLI ]`
                    .child(
                        div()
                            .p_0p5()
                            .rounded_md()
                            .bg(rgb(0xf1f5f9))
                            .border_1()
                            .border_color(rgb(0xe2e8f0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_0p5()
                            .child(
                                div()
                                    .id("top-mode-detail")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(if self.output_mode != OutputMode::RawCli {
                                        rgb(0xffffff)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.output_mode = OutputMode::DetailDive;
                                        this.prev_non_cli_mode = OutputMode::DetailDive;
                                        cx.notify();
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/layers.svg")
                                            .size(px(13.0))
                                            .text_color(if self.output_mode != OutputMode::RawCli {
                                                rgb(0x0f172a)
                                            } else {
                                                rgb(TEXT_SECONDARY)
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(if self.output_mode != OutputMode::RawCli {
                                                gpui::FontWeight::SEMIBOLD
                                            } else {
                                                gpui::FontWeight::NORMAL
                                            })
                                            .text_color(if self.output_mode != OutputMode::RawCli {
                                                rgb(0x0f172a)
                                            } else {
                                                rgb(TEXT_SECONDARY)
                                            })
                                            .child("Dive"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("top-mode-raw")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(if self.output_mode == OutputMode::RawCli {
                                        rgb(0xffffff)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_cli_mode(cx);
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/code.svg")
                                            .size(px(13.0))
                                            .text_color(if self.output_mode == OutputMode::RawCli {
                                                rgb(0x0f172a)
                                            } else {
                                                rgb(TEXT_SECONDARY)
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(if self.output_mode == OutputMode::RawCli {
                                                gpui::FontWeight::SEMIBOLD
                                            } else {
                                                gpui::FontWeight::NORMAL
                                            })
                                            .text_color(if self.output_mode == OutputMode::RawCli {
                                                rgb(0x0f172a)
                                            } else {
                                                rgb(TEXT_SECONDARY)
                                            })
                                            .child("CLI"),
                                    ),
                            ),
                    )
                    // SQL Inline Toggle Badge
                    .child(
                        div()
                            .id("top-toggle-sql")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(if self.settings.show_sql_inline {
                                if has_n1 {
                                    theme::pick(0xfef3c7, 0x3a2a0f)
                                } else {
                                    theme::pick(0xeff6ff, 0x1c2b4a)
                                }
                            } else {
                                rgb(0xf8fafc)
                            })
                            .border_1()
                            .border_color(if self.settings.show_sql_inline {
                                if has_n1 {
                                    theme::pick(0xfcd34d, 0x8a6a1f)
                                } else {
                                    theme::pick(0x93c5fd, 0x3d63b8)
                                }
                            } else {
                                rgb(0xe2e8f0)
                            })
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.show_sql_inline = !this.settings.show_sql_inline;
                                this.output_mode = OutputMode::DetailDive;
                                save_settings(&this.settings);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if self.settings.show_sql_inline {
                                        if has_n1 {
                                            theme::pick(0xb45309, 0xfcd34d)
                                        } else {
                                            theme::pick(0x1d4ed8, 0xbfdbfe)
                                        }
                                    } else {
                                        rgb(TEXT_MUTED)
                                    })
                                    .child(if sql_count > 0 {
                                        format!("SQL {sql_count}")
                                    } else {
                                        "SQL".to_string()
                                    }),
                            ),
                    )
                    // Auto-Run toggle pill
                    .child(
                        div()
                            .id("top-autorun-pill")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(if auto_active {
                                theme::pick(0xeff6ff, 0x1c2b4a)
                            } else {
                                rgb(0xf8fafc)
                            })
                            .border_1()
                            .border_color(if auto_active {
                                theme::pick(0x93c5fd, 0x3d63b8)
                            } else {
                                rgb(0xe2e8f0)
                            })
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_auto_run(cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/bolt.svg")
                                    .size(px(12.5))
                                    .text_color(if auto_active {
                                        theme::pick(0x1d4ed8, 0xbfdbfe)
                                    } else {
                                        rgb(TEXT_MUTED)
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(if auto_active {
                                        theme::pick(0x1d4ed8, 0xbfdbfe)
                                    } else {
                                        rgb(TEXT_SECONDARY)
                                    })
                                    .child("Auto"),
                            ),
                    )
                    // Primary `▶ Run` CTA Button
                    .child(
                        div()
                            .id("top-primary-run")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(if self.is_running {
                                rgb(0x059669)
                            } else {
                                theme::pick(0x0f172a, 0x2563eb)
                            })
                            .hover(|s| {
                                if self.is_running {
                                    s.bg(rgb(0x047857))
                                } else {
                                    s.bg(theme::pick(0x1e293b, 0x3b82f6))
                                }
                            })
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_evaluation(cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/play.svg")
                                    .size(px(12.0))
                                    .text_color(fixed_rgb(0xffffff)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(fixed_rgb(0xffffff))
                                    .child(if self.is_running { "Running" } else { "Run" }),
                            )
                            .child(
                                div()
                                    .font_family("Menlo")
                                    .text_xs()
                                    .text_color(theme::pick(0x94a3b8, 0xbfdbfe))
                                    .child("⌘R"),
                            ),
                    )
                    // Right AI Assistant Sidebar Toggle (`⌘ + I`)
                    .when(self.settings.ai_enabled, |row| {
                        row.child(
                            div()
                                .id("top-toggle-ai-sidebar")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(if self.show_ai_sidebar {
                                    theme::pick(0xeff6ff, 0x1c2b4a)
                                } else {
                                    rgb(0xf8fafc)
                                })
                                .border_1()
                                .border_color(if self.show_ai_sidebar {
                                    theme::pick(0x93c5fd, 0x3d63b8)
                                } else {
                                    rgb(0xe2e8f0)
                                })
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.on_toggle_ai_sidebar(&ToggleAiSidebar, window, cx);
                                }))
                                .child(
                                    svg()
                                        .path("icons/sparkles.svg")
                                        .size(px(13.0))
                                        .text_color(if self.show_ai_sidebar {
                                            theme::pick(0x2563eb, 0x93c5fd)
                                        } else {
                                            rgb(TEXT_SECONDARY)
                                        }),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(if self.show_ai_sidebar {
                                            theme::pick(0x1d4ed8, 0xbfdbfe)
                                        } else {
                                            rgb(TEXT_SECONDARY)
                                        })
                                        .child("AI"),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Scratch-tab strip rendered above the editor/results workspace.
    fn render_tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_count = self.tabs.len();
        // Only offer the "N ▾" switcher when the strip actually overflows.
        let tabs_overflow =
            tab_count > 1 && f32::from(self.tab_scroll_handle.max_offset().width) > 0.5;
            let tabs_menu_id = SharedString::from("titlebar-tabs-menu");
            let tabs_menu_toggle = tabs_menu_id.clone();
            let tabs_menu_open = self.active_dropdown_id.as_ref() == Some(&tabs_menu_id);
            let scroll_handle_wheel = self.tab_scroll_handle.clone();

            div()
                .h(px(34.0))
                .w_full()
                .flex_none()
                .min_w(px(0.0))
                .px_2()
                .bg(rgb(0xf8fafc))
                .border_b_1()
                .border_color(rgb(BORDER_LIGHT))
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                // Horizontally scrollable tab list (trackpad swipe + mouse wheel)
                .child(
                    div()
                        .id("scratch-tabs-scroll")
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_x_scroll()
                        .track_scroll(&self.tab_scroll_handle)
                        .on_scroll_wheel(cx.listener(
                            move |_, ev: &gpui::ScrollWheelEvent, _, cx| {
                                let (dx, dy) = match ev.delta {
                                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                                    ScrollDelta::Lines(l) => (l.x * 36.0, l.y * 36.0),
                                };
                                // Translate vertical mouse-wheel scrolling into horizontal tab scrolling
                                if dx.abs() < 0.5 && dy.abs() > 0.1 {
                                    cx.stop_propagation();
                                    let cur = scroll_handle_wheel.offset();
                                    let max_w =
                                        f32::from(scroll_handle_wheel.max_offset().width)
                                            .max(0.0);
                                    let next_x = (f32::from(cur.x) + dy).clamp(-max_w, 0.0);
                                    scroll_handle_wheel.set_offset(point(px(next_x), px(0.0)));
                                    cx.notify();
                                }
                            },
                        ))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_start()
                        .gap_1()
                        .children(self.tabs.iter().enumerate().map(|(idx, tab)| {
                            let is_active = idx == self.active_tab_idx;
                            div()
                                .id(SharedString::from(format!("scratch-tab-{idx}")))
                                .flex_shrink_0()
                                .pl_2p5()
                                .pr_1p5()
                                .py_1()
                                .rounded_md()
                                .bg(if is_active {
                                    rgb(0xf1f5f9)
                                } else {
                                    rgba(0x00000000)
                                })
                                .border_1()
                                .border_color(if is_active {
                                    rgb(0xcbd5e1)
                                } else {
                                    rgba(0x00000000)
                                })
                                .hover(|s| s.bg(rgb(0xf8fafc)))
                                .cursor_pointer()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .on_mouse_down(
                                    gpui::MouseButton::Right,
                                    cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.active_dropdown_id = None;
                                        this.tab_context_menu = Some((idx, ev.position));
                                        cx.notify();
                                    }),
                                )
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                        if ev.modifiers.control {
                                            cx.stop_propagation();
                                            this.active_dropdown_id = None;
                                            this.tab_context_menu = Some((idx, ev.position));
                                            cx.notify();
                                        }
                                    }),
                                )
                                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                                    if ev.modifiers().control {
                                        return;
                                    }
                                    this.tab_context_menu = None;
                                    this.switch_to_tab(idx, cx);
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(if is_active {
                                            gpui::FontWeight::SEMIBOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(if is_active {
                                            rgb(TEXT_MAIN)
                                        } else {
                                            rgb(TEXT_SECONDARY)
                                        })
                                        .child(tab.title.clone()),
                                )
                                .when(tab_count > 1, |t| {
                                    t.child(
                                        div()
                                            .id(SharedString::from(format!("close-tab-{idx}")))
                                            .px_1()
                                            .rounded_sm()
                                            .hover(|s| {
                                                s.bg(rgb(0xe2e8f0)).text_color(rgb(0x0f172a))
                                            })
                                            .text_xs()
                                            .text_color(rgb(TEXT_MUTED))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.close_tab_at(idx, cx);
                                            }))
                                            .child("×"),
                                    )
                                })
                        })),
                )
                // Pinned `+` button (always visible even with 20+ tabs)
                .child(
                    div()
                        .id("new-scratch-tab-btn")
                        .flex_none()
                        .size(px(24.0))
                        .rounded_md()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.create_new_tab(cx);
                        }))
                        .child(
                            svg()
                                .path("icons/plus.svg")
                                .size(px(13.5))
                                .text_color(rgb(TEXT_SECONDARY)),
                        ),
                )
                // Pinned All-Tabs quick switcher (`▾`) when multiple tabs exist
                .when(tabs_overflow || tabs_menu_open, |bar| {
                    bar.child(
                        div()
                            .relative()
                            .flex_none()
                            .child(
                                div()
                                    .id("titlebar-all-tabs-btn")
                                    .h(px(24.0))
                                    .px_1p5()
                                    .rounded_md()
                                    .bg(if tabs_menu_open {
                                        rgb(0xeff6ff)
                                    } else {
                                        rgb(0xf8fafc)
                                    })
                                    .border_1()
                                    .border_color(if tabs_menu_open {
                                        rgb(0x93c5fd)
                                    } else {
                                        rgb(0xe2e8f0)
                                    })
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .text_xs()
                                    .text_color(if tabs_menu_open {
                                        rgb(0x1d4ed8)
                                    } else {
                                        rgb(TEXT_SECONDARY)
                                    })
                                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if this.active_dropdown_id.as_ref()
                                            == Some(&tabs_menu_toggle)
                                        {
                                            this.active_dropdown_id = None;
                                        } else {
                                            this.active_dropdown_id =
                                                Some(tabs_menu_toggle.clone());
                                        }
                                        cx.notify();
                                    }))
                                    .child(format!("{tab_count}"))
                                    .child(if tabs_menu_open { "▴" } else { "▾" }),
                            )
                            .when(tabs_menu_open, |wrap| {
                                let menu = div()
                                    .id("titlebar-all-tabs-menu")
                                    .occlude()
                                    .absolute()
                                    .right_0()
                                    .top(px(28.0))
                                    .w(px(220.0))
                                    .max_h(px(280.0))
                                    .overflow_y_scroll()
                                    .rounded_lg()
                                    .bg(rgb(0xffffff))
                                    .border_1()
                                    .border_color(rgb(0xcbd5e1))
                                    .shadow_lg()
                                    .p_1()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_click(|_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_scroll_wheel(|_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .children(self.tabs.iter().enumerate().map(|(idx, tab)| {
                                        let is_sel = idx == self.active_tab_idx;
                                        div()
                                            .id(SharedString::from(format!(
                                                "all-tabs-opt-{idx}"
                                            )))
                                            .px_2p5()
                                            .py_1p5()
                                            .rounded_md()
                                            .when(is_sel, |r| r.bg(rgb(0xeff6ff)))
                                            .hover(|s| s.bg(rgb(0xf1f5f9)))
                                            .cursor_pointer()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .on_mouse_down(
                                                gpui::MouseButton::Right,
                                                cx.listener(
                                                    move |this,
                                                          ev: &gpui::MouseDownEvent,
                                                          _,
                                                          cx| {
                                                        cx.stop_propagation();
                                                        this.active_dropdown_id = None;
                                                        this.tab_context_menu =
                                                            Some((idx, ev.position));
                                                        cx.notify();
                                                    },
                                                ),
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.active_dropdown_id = None;
                                                this.tab_context_menu = None;
                                                this.switch_to_tab(idx, cx);
                                            }))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .truncate()
                                                    .text_xs()
                                                    .font_weight(if is_sel {
                                                        gpui::FontWeight::SEMIBOLD
                                                    } else {
                                                        gpui::FontWeight::NORMAL
                                                    })
                                                    .text_color(if is_sel {
                                                        rgb(0x1d4ed8)
                                                    } else {
                                                        rgb(TEXT_MAIN)
                                                    })
                                                    .child(tab.title.clone()),
                                            )
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "all-tabs-close-{idx}"
                                                    )))
                                                    .flex_none()
                                                    .px_1()
                                                    .rounded_sm()
                                                    .hover(|s| {
                                                        s.bg(rgb(0xe2e8f0))
                                                            .text_color(rgb(0x0f172a))
                                                    })
                                                    .text_xs()
                                                    .text_color(rgb(TEXT_MUTED))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.close_tab_at(idx, cx);
                                                        },
                                                    ))
                                                    .child("×"),
                                            )
                                    }));
                                wrap.child(gpui::deferred(menu).with_priority(30))
                            }),
                    )
                })
    }

    /// Editor + results panes, laid out per the split setting.
    fn render_workspace(
        &self,
        hide_output_pane: bool,
        is_horizontal_split: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if hide_output_pane {
            return self.render_editor_pane(true, cx).into_any_element();
        }
        let base = div()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .overflow_hidden()
            .flex();
        if is_horizontal_split {
            base.flex_col()
                .child(self.render_editor_pane(false, cx))
                .child(self.render_pane_sash(true, cx))
                .child(self.render_right_pane(cx))
                .into_any_element()
        } else {
            base.flex_row()
                .child(self.render_editor_pane(false, cx))
                .child(self.render_pane_sash(false, cx))
                .child(self.render_right_pane(cx))
                .into_any_element()
        }
    }

    fn render_left_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(46.0))
            .h_full()
            .flex_none()
            .bg(rgb(BG_RAIL))
            .border_r_1()
            .border_color(rgb(BORDER_LIGHT))
            .flex()
            .flex_col()
            .items_center()
            .pt_2p5()
            .gap_1p5()
            // Workspace group
            .child(rail_icon_button(
                "rail-home",
                "icons/home.svg",
                self.show_landing,
                if self.show_landing { 0x2563eb } else { TEXT_MAIN },
                cx.listener(|this, hovered: &bool, _, cx| {
                    this.set_rail_hover(0, "Home / Get Started", *hovered, cx);
                }),
                cx.listener(|this, _, window, cx| {
                    this.show_landing = !this.show_landing;
                    if this.show_landing {
                        window.focus(&this.focus_handle);
                    } else {
                        this.focus_editor_next_frame = true;
                    }
                    cx.notify();
                }),
            ))
            .child(rail_icon_button(
                "rail-explorer",
                "icons/sidebar.svg",
                !self.show_landing && self.show_sidebar,
                if !self.show_landing && self.show_sidebar {
                    0x2563eb
                } else {
                    TEXT_MAIN
                },
                cx.listener(|this, hovered: &bool, _, cx| {
                    this.set_rail_hover(1, "Project Explorer (⌘ + B)", *hovered, cx);
                }),
                cx.listener(|this, _, _, cx| {
                    if this.show_landing {
                        this.show_landing = false;
                        this.show_sidebar = true;
                        this.focus_editor_next_frame = true;
                    } else {
                        this.show_sidebar = !this.show_sidebar;
                    }
                    cx.notify();
                }),
            ))
            .child(rail_icon_button(
                "rail-project",
                "icons/folder.svg",
                false,
                TEXT_MAIN,
                cx.listener(|this, hovered: &bool, _, cx| {
                    this.set_rail_hover(2, "Open Folder (⌘ + O)", *hovered, cx);
                }),
                cx.listener(|this, _, _, cx| {
                    this.open_directory_dialog(cx);
                }),
            ))
            .child(rail_icon_button(
                "rail-server",
                "icons/server.svg",
                self.active_target.is_ssh() || self.overlay == OverlayKind::SshConnections,
                if let Some((_, ssh_col)) = self.active_target.ssh_info() {
                    ssh_col
                } else {
                    TEXT_MAIN
                },
                cx.listener(|this, hovered: &bool, _, cx| {
                    this.set_rail_hover(3, "SSH Connections (⌘ + K)", *hovered, cx);
                }),
                cx.listener(|this, _, window, cx| {
                    if this.overlay == OverlayKind::SshConnections {
                        this.close_overlay(window, cx);
                    } else {
                        this.open_ssh_modal(window, cx);
                    }
                }),
            ))
            // Group divider
            .child(
                div()
                    .my_1()
                    .w(px(22.0))
                    .h(px(1.0))
                    .bg(rgb(0xe2e8f0)),
            )
            // Inspector tools group
            .child(rail_icon_button(
                "rail-logs",
                "icons/file.svg",
                self.overlay == OverlayKind::LogViewer,
                if self.overlay == OverlayKind::LogViewer {
                    0x2563eb
                } else {
                    TEXT_MAIN
                },
                cx.listener(|this, hovered: &bool, _, cx| {
                    this.set_rail_hover(5, "Logs Viewer (⌘ + L)", *hovered, cx);
                }),
                cx.listener(|this, _, window, cx| {
                    if this.overlay == OverlayKind::LogViewer {
                        this.close_overlay(window, cx);
                    } else {
                        this.open_logs(window, cx);
                    }
                }),
            ))
            .child(rail_icon_button(
                "rail-palette",
                "icons/terminal.svg",
                self.overlay == OverlayKind::CommandPalette,
                if self.overlay == OverlayKind::CommandPalette {
                    0x2563eb
                } else {
                    TEXT_MAIN
                },
                cx.listener(|this, hovered: &bool, _, cx| {
                    this.set_rail_hover(6, "Command Palette (⌘ + P)", *hovered, cx);
                }),
                cx.listener(|this, _, window, cx| {
                    this.open_palette("", window, cx);
                }),
            ))
            .child(div().flex_1())
            .child(
                div().pb_2p5().child(rail_icon_button(
                    "rail-settings",
                    "icons/settings.svg",
                    self.overlay == OverlayKind::Settings,
                    if self.overlay == OverlayKind::Settings {
                        0x2563eb
                    } else {
                        TEXT_MAIN
                    },
                    cx.listener(|this, hovered: &bool, _, cx| {
                        this.set_rail_hover(99, "Settings (⌘ + ,)", *hovered, cx);
                    }),
                    cx.listener(|this, _, window, cx| {
                        if this.overlay == OverlayKind::Settings {
                            this.close_overlay(window, cx);
                        } else {
                            this.open_settings(window, cx);
                        }
                    }),
                )),
            )
    }

    fn render_explorer_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const EXPLORER_RENDER_CAP: usize = 80;
        let filter_q = self
            .explorer_search_input
            .read(cx)
            .text()
            .trim()
            .to_lowercase();
        let has_filter = !filter_q.is_empty();

        let filtered_models: Vec<&ClassSymbol> = self
            .explorer_models
            .iter()
            .filter(|s| {
                !has_filter
                    || s.short_name.to_lowercase().contains(&filter_q)
                    || s.fqcn.to_lowercase().contains(&filter_q)
            })
            .collect();
        let total_models = filtered_models.len();
        let extra_models = total_models.saturating_sub(EXPLORER_RENDER_CAP);

        let filtered_mailables: Vec<&ClassSymbol> = self
            .explorer_mailables
            .iter()
            .filter(|s| {
                !has_filter
                    || s.short_name.to_lowercase().contains(&filter_q)
                    || s.fqcn.to_lowercase().contains(&filter_q)
            })
            .collect();
        let total_mailables = filtered_mailables.len();
        let extra_mailables = total_mailables.saturating_sub(EXPLORER_RENDER_CAP);

        div()
            .id("project-explorer-sidebar")
            .w(px(self.settings.sidebar_width.clamp(150.0, 480.0)))
            .h_full()
            .flex_none()
            .bg(rgb(0xf8fafc))
            .flex()
            .flex_col()
            .overflow_hidden()
            // Sidebar header
            .child(
                div()
                    .h(px(28.0))
                    .px_3()
                    .border_b_1()
                    .border_color(rgb(BORDER_LIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0x475569))
                            .child("EXPLORER"),
                    )
                    .child(
                        div()
                            .id("explorer-close-btn")
                            .px_1()
                            .rounded_sm()
                            .hover(|s| s.bg(rgb(0xe2e8f0)))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_sidebar = false;
                                cx.notify();
                            }))
                            .child("×"),
                    ),
            )
            // Fast filter input for projects with many Models/Mailables
            .child(
                div()
                    .px_2p5()
                    .pt_2()
                    .pb_1()
                    .child(
                        div()
                            .h(px(28.0))
                            .px_2()
                            .rounded_md()
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(if has_filter {
                                rgb(0x93c5fd)
                            } else {
                                rgb(0xe2e8f0)
                            })
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .child(
                                svg()
                                    .path("icons/search.svg")
                                    .size(px(12.5))
                                    .text_color(rgb(0x94a3b8)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .child(self.explorer_search_input.clone()),
                            )
                            .when(has_filter, |bar| {
                                bar.child(
                                    div()
                                        .id("explorer-filter-clear")
                                        .px_1()
                                        .rounded_sm()
                                        .hover(|s| s.bg(rgb(0xf1f5f9)))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(0x64748b))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.explorer_search_input.update(cx, |tf, cx| {
                                                tf.set_text("", cx);
                                            });
                                            cx.notify();
                                        }))
                                        .child("×"),
                                )
                            }),
                    ),
            )
            // Scrollable sidebar sections
            .child(
                div()
                    .id("explorer-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2p5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    // Section 1: Eloquent Models
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .px_1p5()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(0x64748b))
                                            .child("Models"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(TEXT_MUTED))
                                            .child(format!("{total_models}")),
                                    ),
                            )
                            .when(filtered_models.is_empty(), |s| {
                                s.child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_xs()
                                        .text_color(rgb(TEXT_MUTED))
                                        .child(if has_filter {
                                            "No matching models"
                                        } else {
                                            "No app/Models found"
                                        }),
                                )
                            })
                            .children(
                                filtered_models
                                    .into_iter()
                                    .take(EXPLORER_RENDER_CAP)
                                    .enumerate()
                                    .map(|(idx, sym)| {
                                        let sym_clone = (*sym).clone();
                                        let short_name = sym.short_name.clone();
                                        div()
                                            .id(SharedString::from(format!("exp-model-{idx}")))
                                            .group(SharedString::from(format!("exp-model-g-{idx}")))
                                            .relative()
                                            .w_full()
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .hover(|s| s.bg(rgb(0xeef2f6)))
                                            .cursor_pointer()
                                            .overflow_hidden()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.insert_symbol_snippet(&sym_clone, cx);
                                            }))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .overflow_hidden()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .child(
                                                        svg()
                                                            .flex_none()
                                                            .path("icons/symbol-class.svg")
                                                            .size(px(12.5))
                                                            .text_color(rgb(0xd97706)),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .truncate()
                                                            .text_xs()
                                                            .font_weight(gpui::FontWeight::MEDIUM)
                                                            .text_color(rgb(TEXT_MAIN))
                                                            .child(short_name),
                                                    ),
                                            )
                                            // "Query" hint appears only on hover, overlaid so it
                                            // never steals width from long model names.
                                            .child(
                                                div()
                                                    .absolute()
                                                    .right(px(4.0))
                                                    .top(px(2.0))
                                                    .bottom(px(2.0))
                                                    .px_1p5()
                                                    .rounded_sm()
                                                    .flex()
                                                    .items_center()
                                                    .text_xs()
                                                    .text_color(rgb(TEXT_MUTED))
                                                    .group_hover(
                                                        SharedString::from(format!(
                                                            "exp-model-g-{idx}"
                                                        )),
                                                        |s| {
                                                            s.bg(rgb(0xe2e8f0))
                                                                .text_color(rgb(TEXT_SECONDARY))
                                                        },
                                                    )
                                                    .child("Query"),
                                            )
                                    }),
                            )
                            .when(extra_models > 0, |s| {
                                s.child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_xs()
                                        .text_color(rgb(0x64748b))
                                        .child(format!("+{extra_models} more — filter above")),
                                )
                            }),
                    )
                    // Section 2: Mailables
                    .when(!filtered_mailables.is_empty(), |el| {
                        el.child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .px_1p5()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(0x64748b))
                                                .child("Mailables"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(TEXT_MUTED))
                                                .child(format!("{total_mailables}")),
                                        ),
                                )
                                .children(
                                    filtered_mailables
                                        .into_iter()
                                        .take(EXPLORER_RENDER_CAP)
                                        .enumerate()
                                        .map(|(idx, sym)| {
                                            let sym_clone = (*sym).clone();
                                            let short_name = sym.short_name.clone();
                                            div()
                                                .id(SharedString::from(format!("exp-mail-{idx}")))
                                                .w_full()
                                                .flex_shrink_0()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .hover(|s| s.bg(rgb(0xeef2f6)))
                                                .cursor_pointer()
                                                .overflow_hidden()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_1p5()
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.insert_symbol_snippet(&sym_clone, cx);
                                                }))
                                                .child(
                                                    svg()
                                                        .flex_none()
                                                        .path("icons/eye.svg")
                                                        .size(px(12.5))
                                                        .text_color(rgb(0x7c3aed)),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .truncate()
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::MEDIUM)
                                                        .text_color(rgb(TEXT_MAIN))
                                                        .child(short_name),
                                                )
                                        }),
                                )
                                .when(extra_mailables > 0, |s| {
                                    s.child(
                                        div()
                                            .px_2()
                                            .py_1()
                                            .text_xs()
                                            .text_color(rgb(0x64748b))
                                            .child(format!("+{extra_mailables} more — filter above")),
                                    )
                                }),
                        )
                    }),
            )
    }

    fn set_rail_hover(
        &mut self,
        idx: usize,
        label: &'static str,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some((idx, label))
        } else if self.hovered_rail_tooltip.map(|(i, _)| i) == Some(idx) {
            None
        } else {
            self.hovered_rail_tooltip
        };
        if self.hovered_rail_tooltip != next {
            self.hovered_rail_tooltip = next;
            cx.notify();
        }
    }

    fn render_php_missing_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tried = self.settings.php_binary.trim().to_string();
        let detail = if tried.is_empty() || tried == "php" {
            "No PHP binary was found on your PATH or in the usual Homebrew and system locations. Local projects can't run until you pick one.".to_string()
        } else {
            format!("Couldn't run PHP at '{tried}'. Local projects can't run until you pick a working PHP binary.")
        };
        div()
            .id("land-php-missing")
            .w_full()
            .px_4()
            .py_3()
            .rounded_xl()
            .bg(rgb(0xfef2f2))
            .border_1()
            .border_color(rgb(0xfecaca))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0x991b1b))
                            .child("PHP not found"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0xb91c1c))
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .id("land-php-retry")
                            .px_3()
                            .py_1p5()
                            .rounded_lg()
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(rgb(0xfecaca))
                            .hover(|s| s.bg(rgb(0xfff7f7)))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(0x991b1b))
                            .child("Detect again")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.redetect_php_binary(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("land-php-select")
                            .px_3()
                            .py_1p5()
                            .rounded_lg()
                            .bg(rgb(0xdc2626))
                            .hover(|s| s.bg(rgb(0xb91c1c)))
                            .cursor_pointer()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(fixed_rgb(0xffffff))
                            .child("Select PHP…")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_php_binary_dialog(cx);
                            })),
                    ),
            )
    }

    fn render_landing_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const SIDE_COL_W: f32 = 340.0;
        let php_missing = !is_php_detected(&self.detected_php_version);
        let php_badge = if php_missing {
            "PHP not found".to_string()
        } else {
            extract_short_php_badge(&self.detected_php_version)
        };
        let recent_count = self.projects.len();
        let ssh_count = self.ssh_store.connections.len();

        div()
            .id("landing-scroll")
            .flex_1()
            .h_full()
            .bg(rgb(0xf8fafc))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .pt_12()
            .pb_10()
            .px_8()
            .child(
                div()
                    .w_full()
                    .max_w(px(920.0))
                    .flex()
                    .flex_col()
                    .gap_5()
                    // 1. Brand header
                    .child(
                        div()
                            .pb_3()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_4()
                                    .child(
                                        div()
                                            .size(px(52.0))
                                            .flex_none()
                                            .rounded_xl()
                                            .overflow_hidden()
                                            .shadow_md()
                                            .child(
                                                img(ImageSource::Resource(Resource::Embedded(
                                                    "icons/app.png".into(),
                                                )))
                                                .size(px(52.0))
                                                .rounded_xl(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_2p5()
                                                    .child(
                                                        div()
                                                            .text_2xl()
                                                            .font_weight(gpui::FontWeight::BOLD)
                                                            .text_color(rgb(0x0f172a))
                                                            .child("Laralah"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px_2()
                                                            .py_0p5()
                                                            .rounded_full()
                                                            .bg(rgb(if php_missing {
                                                                0xfef2f2
                                                            } else {
                                                                0xeef2ff
                                                            }))
                                                            .border_1()
                                                            .border_color(rgb(if php_missing {
                                                                0xfecaca
                                                            } else {
                                                                0xc7d2fe
                                                            }))
                                                            .font_family("Menlo")
                                                            .text_xs()
                                                            .text_color(rgb(if php_missing {
                                                                0xb91c1c
                                                            } else {
                                                                0x4338ca
                                                            }))
                                                            .child(php_badge),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(rgb(0x64748b))
                                                    .child("The desktop app for Laravel Tinker"),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id("land-cmd-palette-btn")
                                            .pl_3()
                                            .pr_1p5()
                                            .py_1p5()
                                            .rounded_lg()
                                            .bg(rgb(0xffffff))
                                            .border_1()
                                            .border_color(rgb(0xe2e8f0))
                                            .shadow_xs()
                                            .hover(|s| {
                                                s.bg(rgb(0xf8fafc)).border_color(rgb(0xcbd5e1))
                                            })
                                            .cursor_pointer()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_palette("", window, cx);
                                            }))
                                            .child(
                                                svg()
                                                    .path("icons/search.svg")
                                                    .size(px(13.0))
                                                    .text_color(rgb(0x64748b)),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_color(rgb(0x334155))
                                                    .child("Search commands"),
                                            )
                                            .child(landing_keycap("⌘ P")),
                                    )
                                    .child(
                                        div()
                                            .id("land-settings-btn")
                                            .size(px(32.0))
                                            .rounded_lg()
                                            .bg(rgb(0xffffff))
                                            .border_1()
                                            .border_color(rgb(0xe2e8f0))
                                            .shadow_xs()
                                            .hover(|s| {
                                                s.bg(rgb(0xf8fafc)).border_color(rgb(0xcbd5e1))
                                            })
                                            .cursor_pointer()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_settings(window, cx);
                                            }))
                                            .child(
                                                svg()
                                                    .path("icons/settings.svg")
                                                    .size(px(15.0))
                                                    .text_color(rgb(0x475569)),
                                            ),
                                    ),
                            ),
                    )
                    // 1b. PHP missing banner
                    .when(php_missing, |el| {
                        el.child(self.render_php_missing_banner(cx))
                    })
                    // 2. Primary actions — widths match the columns below
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_4()
                            .child(
                                div().flex_1().min_w(px(0.0)).flex().child(landing_primary_tile(
                                    "land-tile-open-dir",
                                    "icons/folder.svg",
                                    0x2563eb,
                                    0xeff6ff,
                                    "Open Local Project",
                                    "Choose any Laravel or Composer directory",
                                    "⌘ O",
                                    cx.listener(|this, _, _, cx| {
                                        this.open_directory_dialog(cx);
                                    }),
                                )),
                            )
                            .child(
                                div().w(px(SIDE_COL_W)).flex_none().flex().child(
                                    landing_primary_tile(
                                        "land-tile-connect-ssh",
                                        "icons/server.svg",
                                        0x0d9488,
                                        0xf0fdfa,
                                        "Connect to Server",
                                        "Run code on Forge or SSH servers",
                                        "⌘ K",
                                        cx.listener(|this, _, window, cx| {
                                            this.open_ssh_modal(window, cx);
                                        }),
                                    ),
                                ),
                            ),
                    )
                    // 3. Recent workspaces + SSH connections
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_4()
                            // Left: Recent Workspaces
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .bg(rgb(0xffffff))
                                    .border_1()
                                    .border_color(rgb(0xe2e8f0))
                                    .rounded_xl()
                                    .shadow_xs()
                                    .overflow_hidden()
                                    .flex()
                                    .flex_col()
                                    .child(landing_card_header(
                                        "icons/clock.svg",
                                        "Recent Workspaces",
                                        recent_count,
                                        "land-recent-browse",
                                        "Browse…",
                                        cx.listener(|this, _, _, cx| {
                                            this.open_directory_dialog(cx);
                                        }),
                                    ))
                                    .when(self.projects.is_empty(), |el| {
                                        el.child(landing_empty_state(
                                            "icons/folder.svg",
                                            "No recent workspaces yet",
                                            "Open a Laravel folder with ⌘ O and it will show up here.",
                                        ))
                                    })
                                    .child(
                                        div().p_1p5().flex().flex_col().gap_0p5().children(
                                            self.projects.iter().take(8).enumerate().map(
                                                |(idx, path)| {
                                                    let name = std::path::Path::new(path)
                                                        .file_name()
                                                        .and_then(|s| s.to_str())
                                                        .unwrap_or(path)
                                                        .to_string();
                                                    let display_path = abbreviate_home_path(path);
                                                    let is_lara = is_laravel_project(path);
                                                    let path_clone = path.clone();
                                                    let group = SharedString::from(format!(
                                                        "recent-folder-g-{idx}"
                                                    ));

                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "recent-folder-{idx}"
                                                        )))
                                                        .group(group.clone())
                                                        .px_2p5()
                                                        .py_2()
                                                        .rounded_lg()
                                                        .hover(|s| s.bg(rgb(0xf1f5f9)))
                                                        .cursor_pointer()
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .gap_3()
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.open_project_path(
                                                                    path_clone.clone(),
                                                                    cx,
                                                                );
                                                            },
                                                        ))
                                                        .child(
                                                            div()
                                                                .size(px(32.0))
                                                                .flex_none()
                                                                .rounded_lg()
                                                                .bg(rgb(if is_lara {
                                                                    0xfef2f2
                                                                } else {
                                                                    0xf1f5f9
                                                                }))
                                                                .flex()
                                                                .items_center()
                                                                .justify_center()
                                                                .child(
                                                                    svg()
                                                                        .path("icons/folder.svg")
                                                                        .size(px(15.0))
                                                                        .text_color(rgb(
                                                                            if is_lara {
                                                                                0xef4444
                                                                            } else {
                                                                                0x64748b
                                                                            },
                                                                        )),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .flex()
                                                                .flex_col()
                                                                .gap_0p5()
                                                                .child(
                                                                    div()
                                                                        .flex()
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .gap_2()
                                                                        .child(
                                                                            div()
                                                                                .truncate()
                                                                                .text_sm()
                                                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                                                .text_color(rgb(0x0f172a))
                                                                                .child(name),
                                                                        )
                                                                        .when(is_lara, |b| {
                                                                            b.child(landing_tag(
                                                                                "Laravel",
                                                                                0xfef2f2,
                                                                                0xdc2626,
                                                                            ))
                                                                        }),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .truncate()
                                                                        .font_family("Menlo")
                                                                        .text_xs()
                                                                        .text_color(rgb(0x94a3b8))
                                                                        .child(display_path),
                                                                ),
                                                        )
                                                },
                                            ),
                                        ),
                                    ),
                            )
                            // Right: SSH Connections
                            .child(
                                div()
                                    .w(px(SIDE_COL_W))
                                    .flex_none()
                                    .bg(rgb(0xffffff))
                                    .border_1()
                                    .border_color(rgb(0xe2e8f0))
                                    .rounded_xl()
                                    .shadow_xs()
                                    .overflow_hidden()
                                    .flex()
                                    .flex_col()
                                    .child(landing_card_header(
                                        "icons/server.svg",
                                        "SSH Connections",
                                        ssh_count,
                                        "land-ssh-manage",
                                        "Manage…",
                                        cx.listener(|this, _, window, cx| {
                                            this.open_ssh_modal(window, cx);
                                        }),
                                    ))
                                    .when(self.ssh_store.connections.is_empty(), |el| {
                                        el.child(landing_empty_state(
                                            "icons/server.svg",
                                            "No saved servers",
                                            "Add a Forge or SSH host with ⌘ K.",
                                        ))
                                    })
                                    .child(
                                        div().p_1p5().flex().flex_col().gap_0p5().children(
                                            self.ssh_store
                                                .connections
                                                .iter()
                                                .take(6)
                                                .enumerate()
                                                .map(|(idx, prof)| {
                                                    let prof_clone = prof.clone();
                                                    let sub = format!(
                                                        "{}@{}:{}",
                                                        prof.username, prof.host, prof.port
                                                    );
                                                    let group = SharedString::from(format!(
                                                        "land-ssh-g-{idx}"
                                                    ));
                                                    let initial = prof
                                                        .label
                                                        .chars()
                                                        .next()
                                                        .map(|c| c.to_uppercase().to_string())
                                                        .unwrap_or_else(|| "?".into());
                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "land-ssh-row-{idx}"
                                                        )))
                                                        .group(group.clone())
                                                        .px_2p5()
                                                        .py_2()
                                                        .rounded_lg()
                                                        .hover(|s| s.bg(rgb(0xf1f5f9)))
                                                        .cursor_pointer()
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .gap_3()
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.connect_ssh_profile(
                                                                    &prof_clone,
                                                                    window,
                                                                    cx,
                                                                );
                                                            },
                                                        ))
                                                        .child(
                                                            div()
                                                                .size(px(32.0))
                                                                .flex_none()
                                                                .rounded_lg()
                                                                .bg(rgb(prof.color_hex))
                                                                .flex()
                                                                .items_center()
                                                                .justify_center()
                                                                .text_sm()
                                                                .font_weight(gpui::FontWeight::BOLD)
                                                                .text_color(fixed_rgb(0xffffff))
                                                                .child(initial),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .flex()
                                                                .flex_col()
                                                                .gap_0p5()
                                                                .child(
                                                                    div()
                                                                        .truncate()
                                                                        .text_sm()
                                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                                        .text_color(rgb(0x0f172a))
                                                                        .child(prof.label.clone()),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .truncate()
                                                                        .font_family("Menlo")
                                                                        .text_xs()
                                                                        .text_color(rgb(0x94a3b8))
                                                                        .child(sub),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_none()
                                                                .px_2p5()
                                                                .py_0p5()
                                                                .rounded_md()
                                                                .bg(rgb(0x2563eb))
                                                                .text_xs()
                                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                                .text_color(fixed_rgb(0xffffff))
                                                                .group_hover(group, |s| {
                                                                    s.bg(rgb(0x1d4ed8))
                                                                })
                                                                .child("Connect"),
                                                        )
                                                }),
                                        ),
                                    ),
                            ),
                    )
                    // 4. Shortcut strip
                    .child(
                        div()
                            .pt_1()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .justify_center()
                            .gap_x_5()
                            .gap_y_2()
                            .child(landing_shortcut_chip("⌃ Space", "Autocomplete"))
                            .child(landing_shortcut_chip("⌘ L", "Logs viewer"))
                            .child(landing_shortcut_chip("⌘ 3", "CLI result"))
                            .child(landing_shortcut_chip("⌘ ,", "Settings")),
                    ),
            )
    }

    fn render_editor_pane(&self, full_span: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let is_horizontal = self.settings.layout == PaneLayout::Horizontal;
        let container = if full_span {
            div().flex_1().min_w(px(0.0)).min_h(px(0.0)).h_full()
        } else if is_horizontal {
            let ratio = self.settings.stack_ratio.clamp(0.15, 0.85);
            div()
                .h(gpui::relative(ratio))
                .min_h(px(90.0))
                .min_w(px(0.0))
                .w_full()
                .flex_shrink_0()
        } else {
            let ratio = self.settings.split_ratio.clamp(0.15, 0.85);
            div()
                .w(gpui::relative(ratio))
                .min_w(px(140.0))
                .min_h(px(0.0))
                .h_full()
                .flex_shrink_0()
        };

        let driver_label = self
            .last_result
            .as_ref()
            .map(|r| r.driver_name.as_str())
            .filter(|d| !d.is_empty() && *d != "None")
            .unwrap_or("LaravelDriver");
        let driver_vars: Vec<(String, String)> = self
            .last_result
            .as_ref()
            .map(|r| r.driver_vars.clone())
            .unwrap_or_default();
        let layout_label = match self.settings.layout {
            PaneLayout::Vertical => "◫ Split",
            PaneLayout::Horizontal => "⊟ Stack",
        };

        container
            .bg(rgb(BG_WINDOW))
            .flex()
            .flex_col()
            .overflow_hidden()
            // Subtle editor context bar
            .child(
                div()
                    .h(px(28.0))
                    .flex_none()
                    .px_3()
                    .bg(rgb(0xfbfcfd))
                    .border_b_1()
                    .border_color(rgb(0xf1f5f9))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .overflow_hidden()
                            // One plain label ("PHP · Laravel") instead of a `<?php` badge
                            // plus the internal driver class name.
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(TEXT_MUTED))
                                    .child(format!(
                                        "PHP · {}",
                                        driver_label.strip_suffix("Driver").unwrap_or(driver_label)
                                    )),
                            )
                            .children(driver_vars.into_iter().take(3).enumerate().map(
                                |(idx, (var_name, _))| {
                                    let vn = var_name.clone();
                                    div()
                                        .id(SharedString::from(format!("drv-var-{idx}")))
                                        .px_1p5()
                                        .rounded_sm()
                                        .bg(rgb(0xf1f5f9))
                                        .hover(|s| s.bg(rgb(0xe2e8f0)))
                                        .cursor_pointer()
                                        .font_family("Menlo")
                                        .text_xs()
                                        .text_color(rgb(0x475569))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let cur = this.editor.read(cx).text().to_string();
                                            let next = if cur.trim().is_empty() {
                                                format!("{vn};")
                                            } else {
                                                format!("{}\n{vn};", cur.trim_end())
                                            };
                                            this.editor.update(cx, |ed, cx| ed.set_text(next, cx));
                                            this.last_editor_version =
                                                this.editor.read(cx).edit_version;
                                            this.focus_editor_next_frame = true;
                                            cx.notify();
                                        }))
                                        .child(var_name)
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .when(self.ai_editor_undo_banner, |row| {
                                row.child(
                                    div()
                                        .id("editor-ai-undo-banner")
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(rgb(0xecfdf5))
                                        .border_1()
                                        .border_color(rgb(0xa7f3d0))
                                        .hover(|s| s.bg(rgb(0xd1fae5)))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(0x047857))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.editor.update(cx, |ed, cx| {
                                                ed.undo_once(cx);
                                            });
                                            this.ai_editor_undo_banner = false;
                                            cx.notify();
                                        }))
                                        .child("✓ Written by AI · Undo (⌘Z)"),
                                )
                            })
                            .child(
                                div()
                                    .id("editor-toggle-split")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(rgb(TEXT_SECONDARY))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings.layout = match this.settings.layout {
                                            PaneLayout::Vertical => PaneLayout::Horizontal,
                                            PaneLayout::Horizontal => PaneLayout::Vertical,
                                        };
                                        this.persist_and_apply_settings(cx);
                                    }))
                                    .child(layout_label),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(self.editor.clone()),
            )
    }

    fn render_right_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_horizontal = self.settings.layout == PaneLayout::Horizontal;
        let container = if is_horizontal {
            div()
                .flex_1()
                .min_h(px(90.0))
                .min_w(px(0.0))
                .w_full()
        } else {
            div()
                .flex_1()
                .min_w(px(140.0))
                .min_h(px(0.0))
                .h_full()
        };

        let header_summary = if let Some(ref res) = self.last_result {
            if self.output_mode == OutputMode::RawCli {
                "PsySH CLI Output".to_string()
            } else {
                let has_output = !res.raw_output.trim_end_matches(['\r', '\n']).is_empty();
                let val_count = ordered_detail_items(res, false).len() + usize::from(has_output);
                let sql_cnt = res.sql_queries.len();
                if sql_cnt > 0 {
                    format!("{val_count} result(s) · {sql_cnt} SQL")
                } else {
                    format!("{val_count} result(s)")
                }
            }
        } else {
            "Output".to_string()
        };

        let has_expanded = !self.expanded_nodes.is_empty();

        container
            .bg(rgb(BG_CANVAS))
            .border_color(rgb(BORDER_LIGHT))
            .flex()
            .flex_col()
            .overflow_hidden()
            // Output pane subheader
            .child(
                div()
                    .h(px(28.0))
                    .flex_none()
                    .px_3()
                    .bg(rgb(0xf8fafc))
                    .border_b_1()
                    .border_color(rgb(BORDER_LIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(TEXT_SECONDARY))
                            .child(header_summary),
                    )
                    .when(self.last_result.is_some(), |hdr| {
                        hdr.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .when(self.output_mode == OutputMode::DetailDive, |row| {
                                    row.child(
                                        div()
                                            .id("output-toggle-expand-all")
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded_sm()
                                            .hover(|s| s.bg(rgb(0xe2e8f0)))
                                            .cursor_pointer()
                                            .text_xs()
                                            .text_color(rgb(TEXT_SECONDARY))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if !this.expanded_nodes.is_empty() {
                                                    this.expanded_nodes.clear();
                                                } else if let Some(ref res) = this.last_result {
                                                    let mut all = HashSet::new();
                                                    if let Some(ref ret) = res.return_value {
                                                        collect_all_expanded(&ret.node, &mut all);
                                                    }
                                                    for d in &res.dumps {
                                                        collect_all_expanded(&d.node, &mut all);
                                                    }
                                                    this.expanded_nodes = all;
                                                }
                                                cx.notify();
                                            }))
                                            .child(if has_expanded {
                                                "Collapse All"
                                            } else {
                                                "Expand All"
                                            }),
                                    )
                                })
                                .child(
                                    div()
                                        .id("output-clear-btn")
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .hover(|s| s.bg(rgb(0xe2e8f0)))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(TEXT_MUTED))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.last_result = None;
                                            this.expanded_nodes.clear();
                                            cx.notify();
                                        }))
                                        .child("Clear"),
                                ),
                        )
                    }),
            )
            .child(match self.last_result.as_ref() {
                None => div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(TEXT_SECONDARY))
                            .child("Ready to evaluate"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child("Press ⌘ + R or click Run in the toolbar"),
                    )
                    .into_any_element(),
                Some(res) => match self.output_mode {
                    OutputMode::TableMode => self.render_table_mode(res, cx).into_any_element(),
                    OutputMode::RawCli => self.render_raw_cli(res, cx).into_any_element(),
                    _ => self.render_detail_dive_cards(res, cx).into_any_element(),
                },
            })
    }

    fn render_detail_dive_cards(
        &self,
        res: &EvaluationResult,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let stream_items = ordered_detail_items(res, self.settings.show_sql_inline);
        let raw_output = res
            .raw_output
            .trim_end_matches(['\r', '\n'])
            .replace("\r\n", "\n");

        div()
            .id("detail-cards-scroll")
            .w_full()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .overflow_y_scroll()
            .overflow_x_hidden()
            .p_3()
            .flex()
            .flex_col()
            .gap_2p5()
            // Empty SQL notice when SQL toggle is enabled and 0 queries ran (matching screenshot)
            .when(self.settings.show_sql_inline && res.sql_queries.is_empty(), |el| {
                el.child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .bg(rgb(BG_CARD))
                        .border_1()
                        .border_color(rgb(BORDER_CARD))
                        .rounded_lg()
                        .px_3p5()
                        .py_2p5()
                        .font_family("Menlo")
                        .text_xs()
                        .text_color(rgb(TEXT_SECONDARY))
                        .child("No queries were made."),
                )
            })
            // Captured stdout (echo / print / printf ...)
            .when(!raw_output.is_empty(), |el| {
                el.child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .overflow_x_hidden()
                        .bg(rgb(BG_CARD))
                        .border_1()
                        .border_color(rgb(BORDER_CARD))
                        .rounded_lg()
                        .px_3p5()
                        .py_2p5()
                        .flex()
                        .flex_row()
                        .items_start()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .font_family("Menlo")
                                .text_xs()
                                .line_height(px(20.0))
                                .text_color(rgb(TEXT_MAIN))
                                .child(raw_output.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(rgb(TEXT_MUTED))
                                .child("Output"),
                        ),
                )
            })
            // Ordered top-to-bottom stream: SQL queries, dump()s, and final return value
            .children(stream_items.into_iter().map(|item| match item {
                DetailStreamItem::Sql(q) => div()
                    .w_full()
                    .flex_shrink_0()
                    .overflow_x_hidden()
                    .bg(rgb(BG_CARD))
                    .border_1()
                    .border_l_3()
                    .border_color(if q.is_duplicate {
                        rgb(0xf59e0b)
                    } else {
                        rgb(BORDER_CARD)
                    })
                    .rounded_lg()
                    .px_3p5()
                    .py_2p5()
                    .flex()
                    .flex_row()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap_2()
                            .child(
                                div()
                                    .flex_none()
                                    .mt_0p5()
                                    .px_1p5()
                                    .rounded_sm()
                                    .bg(if q.is_duplicate {
                                        rgb(0xfef3c7)
                                    } else {
                                        rgb(0xfaf5ff)
                                    })
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if q.is_duplicate {
                                        rgb(0xb45309)
                                    } else {
                                        rgb(COLOR_SQL_ACCENT)
                                    })
                                    .child(if q.is_duplicate { "N+1" } else { "SQL" }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .font_family("Menlo")
                                    .text_xs()
                                    .line_height(px(20.0))
                                    .text_color(rgb(TEXT_MAIN))
                                    .child(q.interpolated_sql),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .when(q.line > 0, |r| {
                                r.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(TEXT_MUTED))
                                        .child(format!("Line {}", q.line)),
                                )
                            })
                            .child(
                                div()
                                    .px_1p5()
                                    .rounded_sm()
                                    .bg(rgb(0xf8fafc))
                                    .font_family("Menlo")
                                    .text_xs()
                                    .text_color(if q.is_duplicate {
                                        rgb(0xb45309)
                                    } else {
                                        rgb(TEXT_SECONDARY)
                                    })
                                    .child(format!("{:.2}ms", q.time_ms)),
                            ),
                    )
                    .into_any_element(),
                DetailStreamItem::Value {
                    card,
                    attach_mailable_eye,
                } => self
                    .render_value_card(&card, attach_mailable_eye, res, cx)
                    .into_any_element(),
            }))
            // Exception / Parse Error card if an error occurred
            .when_some(res.exception.as_ref(), |el, exc| {
                let display_msg = if exc.message.starts_with("PHP Parse error:") {
                    exc.message.clone()
                } else {
                    format!("{}: {}", exc.class, exc.message)
                };
                el.child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .overflow_x_hidden()
                        .bg(rgb(0xfef2f2))
                        .border_1()
                        .border_color(rgb(0xfca5a5))
                        .rounded_lg()
                        .px_3p5()
                        .py_3()
                        .flex()
                        .flex_row()
                        .items_start()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .font_family("Menlo")
                                .text_xs()
                                .line_height(px(20.0))
                                .text_color(rgb(0x991b1b))
                                .child(display_msg),
                        )
                        .when(exc.line > 0, |r| {
                            r.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(0xb91c1c))
                                    .child(format!("Line {}", exc.line)),
                            )
                        }),
                )
            })
    }

    fn render_value_card(
        &self,
        card: &DumpCard,
        attach_mailable_eye: bool,
        res: &EvaluationResult,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let node = &card.node;
        let visible_children: Vec<&ObjectNode> = node
            .children
            .iter()
            .filter(|c| c.visibility.as_deref() != Some("graph_rel"))
            .collect();
        let has_children = !visible_children.is_empty();
        let is_expanded = self.expanded_nodes.contains(&node.id);
        let root_id = node.id.clone();

        let card_table = if card.kind == "return" {
            res.table
                .clone()
                .or_else(|| extract_table_from_node(node, &format!("Line {}", card.line)))
        } else {
            extract_table_from_node(node, &format!("Line {}", card.line))
        };

        let card_html = node.html_preview.clone().or_else(|| {
            if attach_mailable_eye {
                res.mailable.as_ref().map(|m| m.html.clone())
            } else {
                None
            }
        });
        let can_graph = card_html.is_none() && has_object_graph(node);
        let graph_node = node.clone();
        let has_actions = card_html.is_some() || can_graph || card_table.is_some();
        let has_right_col = card.line > 0 || has_actions;

        let line_badge_text = match card.kind.as_str() {
            "dump" | "dd" => format!("{} · Line {}", card.kind, card.line),
            _ => format!("Line {}", card.line),
        };

        div()
            .w_full()
            .flex_shrink_0()
            .overflow_x_hidden()
            .bg(rgb(BG_CARD))
            .border_1()
            .border_color(rgb(BORDER_CARD))
            .rounded_lg()
            .px_3p5()
            .py_3()
            .flex()
            .flex_col()
            .gap_1p5()
            // Top header row: value / class {#id ▸} on left (truncating cleanly), Line badge + action pills on right
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .overflow_x_hidden()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(self.render_node_header_line(
                                node,
                                None,
                                has_children,
                                is_expanded,
                                &format!("{}-root", card.id),
                                root_id,
                                cx,
                            )),
                    )
                    .when(has_right_col, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .when(has_actions, |row| {
                                    row.child(
                                        div()
                                            .flex_none()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .when_some(card_html, |icons, html| {
                                                icons.child(card_action_pill(
                                                    &format!("btn-card-mail-eye-{}", card.id),
                                                    "icons/eye.svg",
                                                    "Mail",
                                                    cx.listener(move |this, _, window, cx| {
                                                        this.open_html_preview(
                                                            html.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    }),
                                                ))
                                            })
                                            .when(can_graph, |icons| {
                                                icons.child(card_action_pill(
                                                    &format!("btn-card-eye-{}", card.id),
                                                    "icons/eye.svg",
                                                    "Graph",
                                                    cx.listener(move |this, _, window, cx| {
                                                        this.open_object_graph(
                                                            graph_node.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    }),
                                                ))
                                            })
                                            .when_some(card_table, |icons, tbl| {
                                                icons.child(card_action_pill(
                                                    &format!("btn-card-table-{}", card.id),
                                                    "icons/table.svg",
                                                    "Table",
                                                    cx.listener(move |this, _, window, cx| {
                                                        this.open_table_preview(
                                                            tbl.clone(),
                                                            window,
                                                            cx,
                                                        );
                                                    }),
                                                ))
                                            }),
                                    )
                                })
                                .when(card.line > 0, |row| {
                                    row.child(
                                        div()
                                            .flex_none()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded_sm()
                                            .bg(rgb(0xf8fafc))
                                            .text_xs()
                                            .text_color(rgb(TEXT_MUTED))
                                            .child(line_badge_text),
                                    )
                                }),
                        )
                    }),
            )
            // Expanded children indented cleanly
            .when(has_children && is_expanded, |el| {
                el.child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .overflow_x_hidden()
                        .pl_4()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .children(visible_children.into_iter().map(|child| {
                            self.render_tree_child(child, 0, &card.id, cx)
                        })),
                )
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_node_header_line(
        &self,
        node: &ObjectNode,
        prop_key: Option<&str>,
        has_children: bool,
        is_expanded: bool,
        dom_id: &str,
        toggle_id: String,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_root_header = prop_key.is_none();
        let mut row = div()
            .id(SharedString::from(dom_id.to_string()))
            .w_full()
            .min_w(px(0.0))
            .min_h(px(20.0))
            .flex_shrink_0()
            .overflow_x_hidden()
            .flex()
            .flex_row()
            .when(!is_root_header, |r| r.flex_wrap().items_start())
            .when(is_root_header, |r| r.items_center())
            .gap_1()
            .font_family("Menlo")
            .text_xs()
            .line_height(px(20.0));

        if has_children {
            row = row
                .cursor_pointer()
                .rounded_sm()
                .hover(|s| s.bg(rgb(BG_SUBTLE)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.expanded_nodes.contains(&toggle_id) {
                        this.expanded_nodes.remove(&toggle_id);
                    } else {
                        this.expanded_nodes.insert(toggle_id.clone());
                    }
                    cx.notify();
                }));
        }

        if let Some(key) = prop_key {
            row = row.child(
                div()
                    .flex_none()
                    .text_color(rgb(COLOR_PROP_KEY))
                    .child(format!("{key}:")),
            );
        }

        if let Some(ref fqcn) = node.class_name {
            let oid_tag = node
                .object_id
                .map(|id| format!("#{id}"))
                .unwrap_or_default();
            let caret = if has_children {
                if is_expanded { " ▾" } else { " ▸" }
            } else {
                ""
            };
            // Show the class basename (`Collection`, not `Illuminate\Database\Eloqu…`) so the
            // meaningful part never gets truncated away; the namespace follows in muted text.
            let (namespace, short_name) = match fqcn.rfind('\\') {
                Some(pos) => (&fqcn[..pos], &fqcn[pos + 1..]),
                None => ("", fqcn.as_str()),
            };
            let item_count = list_item_count(node);
            row = row
                .child(
                    div()
                        .flex_none()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT_MAIN))
                        .child(short_name.to_string()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(TEXT_MUTED))
                        .child(format!("{{{oid_tag}{caret}}}")),
                )
                .when_some(item_count, |r, n| {
                    r.child(
                        div()
                            .flex_none()
                            .px_1p5()
                            .rounded_sm()
                            .bg(rgb(BG_SUBTLE))
                            .text_color(rgb(TEXT_SECONDARY))
                            .child(if n == 1 {
                                "1 item".to_string()
                            } else {
                                format!("{n} items")
                            }),
                    )
                })
                .when(is_root_header && !namespace.is_empty(), |r| {
                    r.child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_color(rgb(TEXT_MUTED))
                            .child(namespace.to_string()),
                    )
                });
        } else if has_children {
            let caret = if is_expanded { "▾" } else { "▸" };
            row = row
                .child(
                    div()
                        .min_w(px(0.0))
                        .truncate()
                        .text_color(rgb(TEXT_MAIN))
                        .child(node.summary.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(TEXT_MUTED))
                        .child(format!("[{caret}]")),
                );
        } else {
            let val = node.scalar_value.as_deref().unwrap_or(&node.summary);
            let val_color = match node.type_name.as_str() {
                "int" | "float" => COLOR_NUMBER,
                "bool" => COLOR_BOOL,
                "null" => TEXT_MUTED,
                t if t.starts_with("string") => COLOR_STRING,
                _ => TEXT_MAIN,
            };
            row = row.child(
                div()
                    .min_w(px(0.0))
                    .when(is_root_header, |d| d.truncate())
                    .when(!is_root_header, |d| d.flex_1().overflow_hidden())
                    .text_color(rgb(val_color))
                    .child(val.to_string()),
            );
        }

        row.into_any_element()
    }

    fn render_tree_child(
        &self,
        node: &ObjectNode,
        depth: usize,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let visible_children: Vec<&ObjectNode> = node
            .children
            .iter()
            .filter(|c| c.visibility.as_deref() != Some("graph_rel"))
            .collect();
        let has_children = !visible_children.is_empty();
        let is_expanded = self.expanded_nodes.contains(&node.id);

        div()
            .w_full()
            .min_w(px(0.0))
            .flex_shrink_0()
            .overflow_x_hidden()
            .pl(px((depth as f32) * 14.0))
            .flex()
            .flex_col()
            .child(self.render_node_header_line(
                node,
                Some(&node.label),
                has_children,
                is_expanded,
                &format!("{scope}-{}", node.id),
                node.id.clone(),
                cx,
            ))
            .when(has_children && is_expanded, |el| {
                el.children(
                    visible_children
                        .into_iter()
                        .map(|ch| self.render_tree_child(ch, depth + 1, scope, cx)),
                )
            })
            .into_any_element()
    }

    fn render_table_mode(
        &self,
        res: &EvaluationResult,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(ref table) = res.table else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_6()
                .text_xs()
                .text_color(rgb(TEXT_MUTED))
                .child("Return an Eloquent Collection or array of rows (e.g. User::all()) for Table Mode.")
                .into_any_element();
        };

        let needle = self.table_search_input.read(cx).text().trim().to_lowercase();
        let mut indices: Vec<usize> = (0..table.rows.len())
            .filter(|&i| {
                needle.is_empty()
                    || table.rows[i]
                        .iter()
                        .any(|c| c.to_lowercase().contains(&needle))
            })
            .collect();

        if let Some(col_idx) = self.table_sort_col {
            let asc = self.table_sort_asc;
            indices.sort_by(|&a, &b| {
                let va = table.rows[a].get(col_idx).map(|s| s.as_str()).unwrap_or("");
                let vb = table.rows[b].get(col_idx).map(|s| s.as_str()).unwrap_or("");
                let cmp = match (va.parse::<f64>(), vb.parse::<f64>()) {
                    (Ok(na), Ok(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
                    _ => va.to_lowercase().cmp(&vb.to_lowercase()),
                };
                if asc { cmp } else { cmp.reverse() }
            });
        }

        let total = indices.len();
        let page_sz = self.table_page_size.max(1);
        let total_pages = total.div_ceil(page_sz).max(1);
        let page = self.table_page.min(total_pages - 1);
        let start = page * page_sz;
        let end = (start + page_sz).min(total);
        let slice = &indices[start..end];

        div()
            .flex_1()
            .flex()
            .flex_col()
            .bg(rgb(BG_CARD))
            .overflow_hidden()
            // Clean toolbar
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .bg(rgb(BG_CANVAS))
                    .border_b_1()
                    .border_color(rgb(BORDER_LIGHT))
                    .child(
                        div()
                            .flex_1()
                            .px_2p5()
                            .rounded_md()
                            .bg(rgb(BG_CARD))
                            .border_1()
                            .border_color(rgb(BORDER_CARD))
                            .child(self.table_search_input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .when_some(self.table_export_status.clone(), |row, (msg, is_err)| {
                                row.child(render_export_status_badge(&msg, is_err))
                            })
                            .child(light_button(
                                "tbl-csv",
                                "CSV",
                                cx.listener(|this, _, _, cx| this.export_table(true, cx)),
                            ))
                            .child(light_button(
                                "tbl-json",
                                "JSON",
                                cx.listener(|this, _, _, cx| this.export_table(false, cx)),
                            )),
                    ),
            )
            // Scrollable table grid
            .child(
                div()
                    .id("light-table-scroll")
                    .flex_1()
                    .overflow_scroll()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .bg(rgb(BG_CANVAS))
                            .border_b_1()
                            .border_color(rgb(BORDER_LIGHT))
                            .children(table.columns.iter().enumerate().map(|(cidx, col)| {
                                let ind = if self.table_sort_col == Some(cidx) {
                                    if self.table_sort_asc { " ▲" } else { " ▼" }
                                } else {
                                    ""
                                };
                                div()
                                    .id(SharedString::from(format!("col-h-{cidx}")))
                                    .w(px(136.0))
                                    .flex_none()
                                    .px_2p5()
                                    .py_1p5()
                                    .border_r_1()
                                    .border_color(rgb(BORDER_LIGHT))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT_SECONDARY))
                                    .truncate()
                                    .child(format!("{col}{ind}"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.table_sort_col == Some(cidx) {
                                            this.table_sort_asc = !this.table_sort_asc;
                                        } else {
                                            this.table_sort_col = Some(cidx);
                                            this.table_sort_asc = true;
                                        }
                                        cx.notify();
                                    }))
                            })),
                    )
                    .children(slice.iter().map(|&orig| {
                        let selected = self.selected_table_row == Some(orig);
                        div()
                            .id(SharedString::from(format!("row-d-{orig}")))
                            .flex()
                            .flex_row()
                            .border_b_1()
                            .border_color(rgb(BORDER_LIGHT))
                            .bg(if selected {
                                rgb(0xeff6ff)
                            } else {
                                rgb(BG_CARD)
                            })
                            .hover(|s| s.bg(rgb(BG_CANVAS)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_table_row = Some(orig);
                                cx.notify();
                            }))
                            .children(table.rows[orig].iter().map(|cell| {
                                div()
                                    .w(px(136.0))
                                    .flex_none()
                                    .px_2p5()
                                    .py_1p5()
                                    .border_r_1()
                                    .border_color(rgb(BORDER_LIGHT))
                                    .font_family("Menlo")
                                    .text_xs()
                                    .text_color(rgb(TEXT_MAIN))
                                    .truncate()
                                    .child(cell.clone())
                            }))
                    })),
            )
            // Selected row copy actions
            .when_some(self.selected_table_row, |el, r_idx| {
                let json_s = table.raw_rows_json.get(r_idx).cloned().unwrap_or_default();
                let php_s = table.raw_rows_php.get(r_idx).cloned().unwrap_or_default();
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .bg(rgb(BG_CANVAS))
                        .border_t_1()
                        .border_color(rgb(BORDER_LIGHT))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .font_family("Menlo")
                                .text_xs()
                                .text_color(rgb(TEXT_SECONDARY))
                                .truncate()
                                .child(php_s.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_1p5()
                                .child(light_button(
                                    "copy-r-json",
                                    "Copy JSON",
                                    cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            json_s.clone(),
                                        ));
                                    }),
                                ))
                                .child(light_button(
                                    "copy-r-php",
                                    "Copy PHP",
                                    cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            php_s.clone(),
                                        ));
                                    }),
                                )),
                        ),
                )
            })
            // Footer
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .bg(rgb(BG_CANVAS))
                    .border_t_1()
                    .border_color(rgb(BORDER_LIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(if total == 0 {
                                "0 rows".to_string()
                            } else {
                                format!("{}-{} of {} rows", start + 1, end, total)
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1p5()
                            .child(light_button(
                                "tbl-prev",
                                "Prev",
                                cx.listener(|this, _, _, cx| {
                                    if this.table_page > 0 {
                                        this.table_page -= 1;
                                        cx.notify();
                                    }
                                }),
                            ))
                            .child(light_button(
                                "tbl-next",
                                "Next",
                                cx.listener(move |this, _, _, cx| {
                                    if this.table_page + 1 < total_pages {
                                        this.table_page += 1;
                                        cx.notify();
                                    }
                                }),
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_raw_cli(
        &self,
        res: &EvaluationResult,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cli_lines = format_cli_lines(res);
        let full_text = format_cli_text(res);

        div()
            .id("raw-cli-container")
            .relative()
            .flex_1()
            .size_full()
            .bg(rgb(BG_WINDOW))
            .overflow_hidden()
            .child(
                div()
                    .id("raw-cli-scroll")
                    .size_full()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .px_3p5()
                    .py_2p5()
                    .flex()
                    .flex_col()
                    .children(cli_lines.into_iter().map(|line| {
                        let depth = line.indent_depth;
                        let is_blank = line.spans.is_empty();
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .min_h(px(21.0))
                            .flex_shrink_0()
                            .flex()
                            .flex_row()
                            .font_family("Menlo")
                            .text_xs()
                            .line_height(px(21.0))
                            .children((0..depth).map(|_| {
                                div()
                                    .w(px(16.0))
                                    .min_h(px(21.0))
                                    .flex_shrink_0()
                                    .border_l_1()
                                    .border_color(rgb(BORDER_LIGHT))
                            }))
                            .when(!is_blank, |row| {
                                let mut combined = String::new();
                                let mut highlights = Vec::with_capacity(line.spans.len());
                                for (kind, seg) in line.spans {
                                    let display_seg = if seg.contains('\t') {
                                        seg.replace('\t', "    ")
                                    } else {
                                        seg
                                    };
                                    if display_seg.is_empty() {
                                        continue;
                                    }
                                    let start = combined.len();
                                    combined.push_str(&display_seg);
                                    let end = combined.len();
                                    let col = match kind {
                                        CliTokenKind::ClassName => TEXT_MAIN,
                                        CliTokenKind::ObjectHandle => TEXT_MUTED,
                                        CliTokenKind::Key => TEXT_MAIN,
                                        CliTokenKind::String => COLOR_STRING,
                                        CliTokenKind::Number => COLOR_NUMBER,
                                        CliTokenKind::BoolOrNull => COLOR_BOOL,
                                        CliTokenKind::Stdout => TEXT_MAIN,
                                        CliTokenKind::Error => COLOR_DANGER,
                                        CliTokenKind::Plain => TEXT_MAIN,
                                    };
                                    let font_weight = if kind == CliTokenKind::Error {
                                        Some(gpui::FontWeight::SEMIBOLD)
                                    } else {
                                        None
                                    };
                                    highlights.push((
                                        start..end,
                                        gpui::HighlightStyle {
                                            color: Some(rgb(col).into()),
                                            font_weight,
                                            ..Default::default()
                                        },
                                    ));
                                }
                                row.child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .whitespace_normal()
                                        .text_color(rgb(TEXT_MAIN))
                                        .child(
                                            gpui::StyledText::new(combined)
                                                .with_highlights(highlights),
                                        ),
                                )
                            })
                    })),
            )
            .child(
                div()
                    .absolute()
                    .top_2()
                    .right_3()
                    .child(light_button(
                        "btn-copy-cli-output",
                        "Copy",
                        cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(full_text.clone()));
                        }),
                    )),
            )
    }

    fn render_palette_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.current_palette_entries(cx);

        div()
            .id("palette-backdrop")
            .key_context("CommandPalette")
            .on_action(cx.listener(Self::on_palette_up))
            .on_action(cx.listener(Self::on_palette_down))
            .on_action(cx.listener(Self::on_palette_confirm))
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgba(0x0f172a33))
            .flex()
            .justify_center()
            .items_start()
            .pt(px(56.0))
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlay(window, cx);
            }))
            .child(
                div()
                    .id("palette-dialog")
                    .w(px(560.0))
                    .max_h(px(420.0))
                    .bg(rgb(BG_CARD))
                    .border_1()
                    .border_color(rgb(BORDER_CARD))
                    .rounded_lg()
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .px_3p5()
                            .py_2p5()
                            .border_b_1()
                            .border_color(rgb(BORDER_LIGHT))
                            .child(self.palette_input.clone()),
                    )
                    .child(
                        div()
                            .id("palette-list")
                            .flex_1()
                            .overflow_y_scroll()
                            .p_1p5()
                            .flex()
                            .flex_col()
                            .children(entries.into_iter().enumerate().map(|(idx, item)| {
                                let selected = idx == self.palette_selected_idx;
                                let action = item.action.clone();
                                let tag_col = match item.kind {
                                    PaletteItemKind::Project => COLOR_SUCCESS,
                                    PaletteItemKind::Target => COLOR_NUMBER,
                                    PaletteItemKind::Command => COLOR_SQL_ACCENT,
                                };
                                div()
                                    .id(SharedString::from(format!("p-item-{idx}")))
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(if selected {
                                        rgb(BG_SUBTLE)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .hover(|s| s.bg(rgb(BG_SUBTLE)))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.execute_palette_action(action.clone(), window, cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2p5()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_family("Menlo")
                                                    .text_color(rgb(tag_col))
                                                    .child(item.prefix_tag),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .font_weight(gpui::FontWeight::MEDIUM)
                                                            .text_color(rgb(TEXT_MAIN))
                                                            .child(item.title),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgb(TEXT_MUTED))
                                                            .child(item.subtitle),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(TEXT_SECONDARY))
                                            .child(item.shortcut),
                                    )
                            })),
                    ),
            )
    }

    fn render_logs_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = count_by_level(&self.log_entries);
        let search = self.log_search_input.read(cx).text().to_string();
        let filtered = filter_entries_multi(&self.log_entries, &self.selected_log_levels, &search);

        let total = filtered.len();
        let page_sz = self.log_page_size.max(1);
        let total_pages = total.div_ceil(page_sz).max(1);
        let page = self.log_page.min(total_pages - 1);
        let start = page * page_sz;
        let end = (start + page_sz).min(total);
        let page_entries = &filtered[start..end];

        let active_file_label = self
            .log_files
            .get(self.active_log_file_idx)
            .map(|p| {
                log_file_display_name(
                    p,
                    &self.active_project,
                    self.custom_logs_folder.as_deref(),
                )
            })
            .unwrap_or_else(|| "No .log files".to_string());

        let levels_label = if self.selected_log_levels.is_empty() {
            "Levels".to_string()
        } else {
            format!("Levels ({})", self.selected_log_levels.len())
        };
        let levels_highlighted = !self.selected_log_levels.is_empty()
            || self.log_menu_popup == LogMenuPopup::Levels;

        let poll_label = self.log_poll_interval.label();
        let popup_state = self.log_menu_popup;

        div()
            .id("logs-backdrop")
            .key_context("LogViewer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgba(0x090d1655))
            .flex()
            .flex_col()
            .justify_end()
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlay(window, cx);
            }))
            .child(
                div()
                    .id("logs-sheet")
                    .relative()
                    .w_full()
                    .h(relative(0.92))
                    .bg(rgb(0x21252b))
                    .border_t_1()
                    .border_color(rgb(0x334155))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        if this.log_menu_popup != LogMenuPopup::None {
                            this.log_menu_popup = LogMenuPopup::None;
                            cx.notify();
                        }
                    }))
                    // 1. Centered "Logs" Header Bar
                    .child(
                        div()
                            .h(px(34.0))
                            .px_4()
                            .bg(rgb(0x21252b))
                            .border_b_1()
                            .border_color(rgb(0x2c323c))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(div().w(px(64.0)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(0xe2e8f0))
                                    .child("Logs"),
                            )
                            .child(
                                div()
                                    .w(px(64.0))
                                    .flex()
                                    .justify_end()
                                    .child(dark_close_button(
                                        "btn-close-logs",
                                        cx.listener(|this, _, window, cx| {
                                            this.close_overlay(window, cx);
                                        }),
                                    )),
                            ),
                    )
                    // 2. Controls Toolbar: [laravel.log ⌄] [Levels (N)] [Press CMD + F to focus search] [Polling Off ⌄]
                    .child(
                        div()
                            .px_4()
                            .py_2p5()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2p5()
                            // File selector dropdown button
                            .child(
                                div()
                                    .id("btn-log-file-select")
                                    .w(px(190.0))
                                    .h(px(30.0))
                                    .px_2p5()
                                    .rounded_sm()
                                    .bg(rgb(0x282c34))
                                    .border_1()
                                    .border_color(if popup_state == LogMenuPopup::Files {
                                        rgb(0x3b82f6)
                                    } else {
                                        rgb(0x3b4252)
                                    })
                                    .hover(|s| s.bg(rgb(0x2f3540)))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.log_menu_popup =
                                                if this.log_menu_popup == LogMenuPopup::Files {
                                                    LogMenuPopup::None
                                                } else {
                                                    LogMenuPopup::Files
                                                };
                                            cx.notify();
                                        }),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_xs()
                                            .text_color(rgb(0xe2e8f0))
                                            .truncate()
                                            .child(active_file_label),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x94a3b8))
                                            .child("⌄"),
                                    ),
                            )
                            // Multi-select Levels button
                            .child(
                                div()
                                    .id("btn-log-levels-select")
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .bg(rgb(0x282c34))
                                    .border_1()
                                    .border_color(if levels_highlighted {
                                        rgb(0x3b82f6)
                                    } else {
                                        rgb(0x3b4252)
                                    })
                                    .hover(|s| s.bg(rgb(0x2f3540)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(0xe2e8f0))
                                    .child(levels_label)
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.log_menu_popup =
                                                if this.log_menu_popup == LogMenuPopup::Levels {
                                                    LogMenuPopup::None
                                                } else {
                                                    LogMenuPopup::Levels
                                                };
                                            cx.notify();
                                        }),
                                    ),
                            )
                            // Full-width Search input (`Press CMD + F to focus search`)
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .bg(rgb(0x282c34))
                                    .border_1()
                                    .border_color(rgb(0x3b4252))
                                    .flex()
                                    .items_center()
                                    .child(self.log_search_input.clone()),
                            )
                            // Polling interval dropdown button (`Polling Off ⌄`)
                            .child(
                                div()
                                    .id("btn-log-polling-select")
                                    .min_w(px(124.0))
                                    .h(px(30.0))
                                    .px_3()
                                    .rounded_sm()
                                    .bg(rgb(0x282c34))
                                    .border_1()
                                    .border_color(if self.log_poll_interval != LogPollInterval::Off
                                        || popup_state == LogMenuPopup::Polling
                                    {
                                        rgb(0x3b82f6)
                                    } else {
                                        rgb(0x3b4252)
                                    })
                                    .hover(|s| s.bg(rgb(0x2f3540)))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.log_menu_popup =
                                                if this.log_menu_popup == LogMenuPopup::Polling {
                                                    LogMenuPopup::None
                                                } else {
                                                    LogMenuPopup::Polling
                                                };
                                            cx.notify();
                                        }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0xe2e8f0))
                                            .child(poll_label),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x94a3b8))
                                            .child("⌄"),
                                    ),
                            ),
                    )
                    // 3. Bordered Table (`LEVEL` | `TIME` | `DESCRIPTION`)
                    .child(
                        div()
                            .flex_1()
                            .mx_4()
                            .mb_2()
                            .rounded_sm()
                            .bg(rgb(0x242932))
                            .border_1()
                            .border_color(rgb(0x323946))
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            // Table Header
                            .child(
                                div()
                                    .px_3p5()
                                    .py_1p5()
                                    .bg(rgb(0x2a303b))
                                    .border_b_1()
                                    .border_color(rgb(0x353d4c))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_4()
                                    .child(
                                        div()
                                            .w(px(110.0))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x94a3b8))
                                            .child("LEVEL"),
                                    )
                                    .child(
                                        div()
                                            .w(px(180.0))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x94a3b8))
                                            .child("TIME"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x94a3b8))
                                            .child("DESCRIPTION"),
                                    ),
                            )
                            // Table Body
                            .child(
                                div()
                                    .id("logs-scroll-area")
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_col()
                                    .when(page_entries.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .p_8()
                                                .flex()
                                                .justify_center()
                                                .text_xs()
                                                .text_color(rgb(0x64748b))
                                                .child("No log entries match the current filter."),
                                        )
                                    })
                                    .children(page_entries.iter().map(|entry| {
                                        let level_col = match entry.level.as_str() {
                                            "ERROR" | "CRITICAL" | "ALERT" | "EMERGENCY" => {
                                                0xf87171
                                            }
                                            "WARNING" => 0xfbbf24,
                                            "DEBUG" => 0x94a3b8,
                                            _ => 0x60a5fa,
                                        };
                                        let has_extra = !entry.stack_trace.is_empty()
                                            || entry.context.is_some();
                                        let open = self.expanded_log_ids.contains(&entry.id);
                                        let eid = entry.id;

                                        let desc_preview = match entry.context.as_deref() {
                                            Some(ctx) if !open => {
                                                format!("{} {}", entry.message, ctx)
                                            }
                                            _ => entry.message.clone(),
                                        };

                                        div()
                                            .id(SharedString::from(format!("log-row-{eid}")))
                                            .px_3p5()
                                            .py_1p5()
                                            .border_b_1()
                                            .border_color(rgb(0x2d333f))
                                            .hover(|s| s.bg(rgb(0x2a303c)))
                                            .when(has_extra, |el| el.cursor_pointer())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if has_extra {
                                                    if this.expanded_log_ids.contains(&eid) {
                                                        this.expanded_log_ids.remove(&eid);
                                                    } else {
                                                        this.expanded_log_ids.insert(eid);
                                                    }
                                                    cx.notify();
                                                }
                                            }))
                                            .flex()
                                            .flex_col()
                                            .gap_1p5()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_4()
                                                    .font_family("Menlo")
                                                    .text_xs()
                                                    .child(
                                                        div()
                                                            .w(px(110.0))
                                                            .flex_none()
                                                            .text_color(rgb(level_col))
                                                            .child(entry.level.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .w(px(180.0))
                                                            .flex_none()
                                                            .text_color(rgb(0xcbd5e1))
                                                            .truncate()
                                                            .child(entry.timestamp.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .text_color(rgb(0xe2e8f0))
                                                            .truncate()
                                                            .child(desc_preview),
                                                    ),
                                            )
                                            .when(has_extra && open, |el| {
                                                el.child(
                                                    div()
                                                        .mt_1()
                                                        .p_2p5()
                                                        .rounded_sm()
                                                        .bg(rgb(0x1b1f27))
                                                        .border_1()
                                                        .border_color(rgb(0x323946))
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .when_some(
                                                            entry.context.as_ref(),
                                                            |c, json_ctx| {
                                                                c.child(
                                                                    div()
                                                                        .font_family("Menlo")
                                                                        .text_xs()
                                                                        .text_color(rgb(0x60a5fa))
                                                                        .child(json_ctx.clone()),
                                                                )
                                                            },
                                                        )
                                                        .children(entry.stack_trace.iter().map(
                                                            |l| {
                                                                div()
                                                                    .font_family("Menlo")
                                                                    .text_xs()
                                                                    .text_color(rgb(0x94a3b8))
                                                                    .child(l.clone())
                                                            },
                                                        )),
                                                )
                                            })
                                    })),
                            ),
                    )
                    // 4. Bottom Pagination Footer (`Page 1 / 5` ... `[Previous] [Next]`)
                    .child(
                        div()
                            .h(px(38.0))
                            .px_4()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x94a3b8))
                                    .child(format!("Page {} / {}", page + 1, total_pages)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id("btn-log-prev")
                                            .px_2p5()
                                            .py_1()
                                            .rounded_sm()
                                            .bg(rgb(0x282c34))
                                            .border_1()
                                            .border_color(rgb(0x353d4c))
                                            .when(page > 0, |el| {
                                                el.cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x323844)))
                                            })
                                            .text_xs()
                                            .text_color(if page > 0 {
                                                rgb(0xe2e8f0)
                                            } else {
                                                rgb(0x4b5563)
                                            })
                                            .child("Previous")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if this.log_page > 0 {
                                                    this.log_page -= 1;
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("btn-log-next")
                                            .px_2p5()
                                            .py_1()
                                            .rounded_sm()
                                            .bg(rgb(0x282c34))
                                            .border_1()
                                            .border_color(rgb(0x353d4c))
                                            .when(page + 1 < total_pages, |el| {
                                                el.cursor_pointer()
                                                    .hover(|s| s.bg(rgb(0x323844)))
                                            })
                                            .text_xs()
                                            .text_color(if page + 1 < total_pages {
                                                rgb(0xe2e8f0)
                                            } else {
                                                rgb(0x4b5563)
                                            })
                                            .child("Next")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if this.log_page + 1 < total_pages {
                                                    this.log_page += 1;
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            ),
                    )
                    // 5A. Floating Log Files Dropdown Menu (`laravel.log`, `worker.log`, `daily/...`, `Choose Logs Folder...`)
                    .when(popup_state == LogMenuPopup::Files, |el| {
                        el.child(
                            div()
                                .id("log-files-popover")
                                .absolute()
                                .top(px(76.0))
                                .left(px(16.0))
                                .min_w(px(230.0))
                                .max_h(px(280.0))
                                .overflow_y_scroll()
                                .bg(rgb(0xd1d5db))
                                .border_1()
                                .border_color(rgb(0x9ca3af))
                                .rounded_md()
                                .shadow_lg()
                                .p_1()
                                .flex()
                                .flex_col()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .children(self.log_files.iter().enumerate().map(|(i, p)| {
                                    let fname = log_file_display_name(
                                        p,
                                        &self.active_project,
                                        self.custom_logs_folder.as_deref(),
                                    );
                                    let is_sel = i == self.active_log_file_idx;
                                    macos_menu_item(
                                        &format!("lf-pop-{i}"),
                                        is_sel,
                                        &fname,
                                        cx.listener(move |this, _, _, cx| {
                                            this.active_log_file_idx = i;
                                            this.log_page = 0;
                                            this.log_menu_popup = LogMenuPopup::None;
                                            this.refresh_logs(cx);
                                        }),
                                    )
                                }))
                                .child(
                                    div()
                                        .my_1()
                                        .h(px(1.0))
                                        .bg(rgb(0x9ca3af)),
                                )
                                .child(macos_menu_item(
                                    "lf-choose-folder",
                                    false,
                                    "Choose Logs Folder...",
                                    cx.listener(|this, _, _, cx| {
                                        this.choose_logs_folder_dialog(cx);
                                    }),
                                )),
                        )
                    })
                    // 5B. Floating Multi-Select Levels Menu (`All Levels` + 8 levels)
                    .when(popup_state == LogMenuPopup::Levels, |el| {
                        let all_checked = self.selected_log_levels.is_empty();
                        el.child(
                            div()
                                .id("log-levels-popover")
                                .absolute()
                                .top(px(76.0))
                                .left(px(216.0))
                                .w(px(190.0))
                                .bg(rgb(0xd1d5db))
                                .border_1()
                                .border_color(rgb(0x9ca3af))
                                .rounded_md()
                                .shadow_lg()
                                .p_1()
                                .flex()
                                .flex_col()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .child(macos_menu_item(
                                    "ll-pop-all",
                                    all_checked,
                                    &format!("All Levels ({})", self.log_entries.len()),
                                    cx.listener(|this, _, _, cx| {
                                        this.selected_log_levels.clear();
                                        this.log_page = 0;
                                        this.log_menu_popup = LogMenuPopup::None;
                                        cx.notify();
                                    }),
                                ))
                                .child(div().my_1().h(px(1.0)).bg(rgb(0x9ca3af)))
                                .children(LogLevel::selectable().into_iter().map(|lvl| {
                                    let checked = self.selected_log_levels.contains(&lvl);
                                    let cnt = counts.get(lvl.label()).copied().unwrap_or(0);
                                    macos_menu_item(
                                        &format!("ll-pop-{}", lvl.label()),
                                        checked,
                                        &format!("{} ({cnt})", lvl.label()),
                                        cx.listener(move |this, _, _, cx| {
                                            if this.selected_log_levels.contains(&lvl) {
                                                this.selected_log_levels.remove(&lvl);
                                            } else {
                                                this.selected_log_levels.insert(lvl);
                                            }
                                            this.log_page = 0;
                                            cx.notify();
                                        }),
                                    )
                                })),
                        )
                    })
                    // 5C. Floating Polling Interval Menu (`Polling Off`, `1 second`, `5 seconds`, `10 seconds`, `30 seconds`)
                    .when(popup_state == LogMenuPopup::Polling, |el| {
                        el.child(
                            div()
                                .id("log-polling-popover")
                                .absolute()
                                .top(px(76.0))
                                .right(px(16.0))
                                .w(px(156.0))
                                .bg(rgb(0xd1d5db))
                                .border_1()
                                .border_color(rgb(0x9ca3af))
                                .rounded_md()
                                .shadow_lg()
                                .p_1()
                                .flex()
                                .flex_col()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .children(LogPollInterval::all().into_iter().map(|iv| {
                                    let is_sel = self.log_poll_interval == iv;
                                    macos_menu_item(
                                        &format!("lp-pop-{:?}", iv),
                                        is_sel,
                                        iv.label(),
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_log_poll_interval(iv, cx);
                                        }),
                                    )
                                })),
                        )
                    }),
            )
    }

    fn render_table_preview_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ref table) = self.active_table_preview else {
            return div().into_any_element();
        };

        let needle = self.table_search_input.read(cx).text().trim().to_lowercase();
        let mut indices: Vec<usize> = (0..table.rows.len())
            .filter(|&i| {
                needle.is_empty()
                    || table.rows[i]
                        .iter()
                        .any(|c| c.to_lowercase().contains(&needle))
            })
            .collect();

        if let Some(col_idx) = self.table_sort_col {
            let asc = self.table_sort_asc;
            indices.sort_by(|&a, &b| {
                let va = table.rows[a].get(col_idx).map(|s| s.as_str()).unwrap_or("");
                let vb = table.rows[b].get(col_idx).map(|s| s.as_str()).unwrap_or("");
                let cmp = match (va.parse::<f64>(), vb.parse::<f64>()) {
                    (Ok(na), Ok(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
                    _ => va.to_lowercase().cmp(&vb.to_lowercase()),
                };
                if asc { cmp } else { cmp.reverse() }
            });
        }

        let total = indices.len();
        let page_sz = self.table_page_size.max(1);
        let total_pages = total.div_ceil(page_sz).max(1);
        let page = self.table_page.min(total_pages - 1);
        let start = page * page_sz;
        let end = (start + page_sz).min(total);
        let slice = &indices[start..end];

        let compact_flags: Vec<bool> = table
            .columns
            .iter()
            .enumerate()
            .map(|(cidx, col)| is_compact_table_column(col, &table.rows, cidx))
            .collect();

        let entries_label = if total == 1 {
            "1 entry".to_string()
        } else {
            format!("{total} entries")
        };

        div()
            .id("table-preview-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgba(0x0f172a2e))
            .flex()
            .flex_col()
            .justify_end()
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlay(window, cx);
            }))
            .child(
                div()
                    .id("table-preview-sheet")
                    .w_full()
                    .h(relative(0.82))
                    .bg(rgb(BG_CARD))
                    .border_t_1()
                    .border_color(rgb(BORDER_CARD))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if this.active_dropdown_id.is_some() {
                                this.active_dropdown_id = None;
                                cx.notify();
                            }
                        }),
                    )
                    .on_click(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    // 1. Centered "Table Preview" Header Bar
                    .child(
                        div()
                            .h(px(36.0))
                            .px_4()
                            .border_b_1()
                            .border_color(rgb(BORDER_LIGHT))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(div().w(px(52.0)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(TEXT_SECONDARY))
                                    .child("Table Preview"),
                            )
                            .child(
                                div()
                                    .w(px(52.0))
                                    .flex()
                                    .justify_end()
                                    .child(light_close_button(
                                        "btn-close-table-preview",
                                        cx.listener(|this, _, window, cx| {
                                            this.close_overlay(window, cx);
                                        }),
                                    )),
                            ),
                    )
                    // 2. Search + "N entries" + Export Bar
                    .child(
                        div()
                            .px_4()
                            .py_2p5()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .px_3()
                                    .py_1()
                                    .rounded_sm()
                                    .bg(rgb(BG_CANVAS))
                                    .border_1()
                                    .border_color(rgb(BORDER_CARD))
                                    .child(self.table_search_input.clone()),
                            )
                            .when_some(self.table_export_status.clone(), |bar, (msg, is_err)| {
                                bar.child(render_export_status_badge(&msg, is_err))
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(TEXT_SECONDARY))
                                    .child(entries_label),
                            )
                            .child({
                                let export_menu_id =
                                    SharedString::from("table-preview-export-menu");
                                let export_menu_toggle = export_menu_id.clone();
                                let export_menu_open =
                                    self.active_dropdown_id.as_ref() == Some(&export_menu_id);
                                div()
                                    .relative()
                                    .flex_none()
                                    .child(
                                        div()
                                            .id("btn-table-preview-export")
                                            .px_2p5()
                                            .py_1()
                                            .rounded_md()
                                            .bg(if export_menu_open {
                                                rgb(0xeff6ff)
                                            } else {
                                                rgb(BG_CARD)
                                            })
                                            .border_1()
                                            .border_color(if export_menu_open {
                                                rgb(0x93c5fd)
                                            } else {
                                                rgb(BORDER_CARD)
                                            })
                                            .hover(|s| s.bg(rgb(BG_HOVER)))
                                            .cursor_pointer()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .text_xs()
                                            .text_color(if export_menu_open {
                                                rgb(0x1d4ed8)
                                            } else {
                                                rgb(TEXT_MAIN)
                                            })
                                            .on_mouse_down(
                                                gpui::MouseButton::Left,
                                                |_, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                if this.active_dropdown_id.as_ref()
                                                    == Some(&export_menu_toggle)
                                                {
                                                    this.active_dropdown_id = None;
                                                } else {
                                                    this.active_dropdown_id =
                                                        Some(export_menu_toggle.clone());
                                                }
                                                cx.notify();
                                            }))
                                            .child("Export")
                                            .child(
                                                div()
                                                    .text_color(if export_menu_open {
                                                        rgb(0x1d4ed8)
                                                    } else {
                                                        rgb(TEXT_MUTED)
                                                    })
                                                    .child(if export_menu_open {
                                                        "▴"
                                                    } else {
                                                        "▾"
                                                    }),
                                            ),
                                    )
                                    .when(export_menu_open, |wrap| {
                                        let menu = div()
                                            .id("table-preview-export-popover")
                                            .occlude()
                                            .absolute()
                                            .right_0()
                                            .top(px(30.0))
                                            .w(px(176.0))
                                            .rounded_lg()
                                            .bg(rgb(0xffffff))
                                            .border_1()
                                            .border_color(rgb(0xcbd5e1))
                                            .shadow_lg()
                                            .p_1()
                                            .flex()
                                            .flex_col()
                                            .gap_0p5()
                                            .on_mouse_down(
                                                gpui::MouseButton::Left,
                                                |_, _, cx| {
                                                    cx.stop_propagation();
                                                },
                                            )
                                            .on_click(|_, _, cx| {
                                                cx.stop_propagation();
                                            })
                                            .child(table_export_menu_item(
                                                "tp-exp-save-csv",
                                                "Export as CSV...",
                                                "CSV",
                                                cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.active_dropdown_id = None;
                                                    this.export_table(true, cx);
                                                }),
                                            ))
                                            .child(table_export_menu_item(
                                                "tp-exp-save-json",
                                                "Export as JSON...",
                                                "JSON",
                                                cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.active_dropdown_id = None;
                                                    this.export_table(false, cx);
                                                }),
                                            ))
                                            .child(
                                                div()
                                                    .my_1()
                                                    .h(px(1.0))
                                                    .bg(rgb(0xe2e8f0)),
                                            )
                                            .child(table_export_menu_item(
                                                "tp-exp-copy-csv",
                                                "Copy CSV",
                                                "⌘C",
                                                cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.active_dropdown_id = None;
                                                    this.copy_table_data(true, cx);
                                                }),
                                            ))
                                            .child(table_export_menu_item(
                                                "tp-exp-copy-json",
                                                "Copy JSON",
                                                "{ }",
                                                cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.active_dropdown_id = None;
                                                    this.copy_table_data(false, cx);
                                                }),
                                            ));
                                        wrap.child(gpui::deferred(menu).with_priority(30))
                                    })
                            }),
                    )
                    // 3. Column Headers
                    .child(
                        div()
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(rgb(BORDER_LIGHT))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .children(table.columns.iter().enumerate().map(|(cidx, col)| {
                                let ind = if self.table_sort_col == Some(cidx) {
                                    if self.table_sort_asc { " ▲" } else { " ▼" }
                                } else {
                                    ""
                                };
                                let compact = compact_flags[cidx];
                                div()
                                    .id(SharedString::from(format!("tp-col-h-{cidx}")))
                                    .when(compact, |el| el.w(px(80.0)).flex_none())
                                    .when(!compact, |el| el.flex_1().min_w(px(120.0)))
                                    .py_0p5()
                                    .rounded_sm()
                                    .hover(|s| s.bg(rgb(BG_SUBTLE)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT_SECONDARY))
                                    .truncate()
                                    .child(format!("{col}{ind}"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.table_sort_col == Some(cidx) {
                                            this.table_sort_asc = !this.table_sort_asc;
                                        } else {
                                            this.table_sort_col = Some(cidx);
                                            this.table_sort_asc = true;
                                        }
                                        cx.notify();
                                    }))
                            })),
                    )
                    // 4. Scrollable Data Rows
                    .child(
                        div()
                            .id("table-preview-rows-scroll")
                            .flex_1()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .when(slice.is_empty(), |el| {
                                el.child(
                                    div()
                                        .p_6()
                                        .flex()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(rgb(TEXT_MUTED))
                                        .child("No matching entries"),
                                )
                            })
                            .children(slice.iter().map(|&orig| {
                                let selected = self.selected_table_row == Some(orig);
                                div()
                                    .id(SharedString::from(format!("tp-row-{orig}")))
                                    .px_4()
                                    .py_2p5()
                                    .border_b_1()
                                    .border_color(rgb(BORDER_LIGHT))
                                    .bg(if selected {
                                        rgb(0xeff6ff)
                                    } else {
                                        rgb(BG_CARD)
                                    })
                                    .hover(|s| s.bg(rgb(BG_CANVAS)))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.selected_table_row == Some(orig) {
                                            this.selected_table_row = None;
                                        } else {
                                            this.selected_table_row = Some(orig);
                                        }
                                        cx.notify();
                                    }))
                                    .children(table.rows[orig].iter().enumerate().map(
                                        |(cidx, cell)| {
                                            let compact =
                                                compact_flags.get(cidx).copied().unwrap_or(false);
                                            div()
                                                .when(compact, |el| el.w(px(80.0)).flex_none())
                                                .when(!compact, |el| el.flex_1().min_w(px(120.0)))
                                                .text_xs()
                                                .text_color(rgb(TEXT_MAIN))
                                                .truncate()
                                                .child(cell.clone())
                                        },
                                    ))
                            })),
                    )
                    // 5. Selected Row Copy Bar (when a row is clicked)
                    .when_some(self.selected_table_row, |el, r_idx| {
                        let json_s = table
                            .raw_rows_json
                            .get(r_idx)
                            .cloned()
                            .unwrap_or_default();
                        let php_s = table
                            .raw_rows_php
                            .get(r_idx)
                            .cloned()
                            .unwrap_or_default();
                        el.child(
                            div()
                                .px_4()
                                .py_2()
                                .bg(rgb(BG_CANVAS))
                                .border_t_1()
                                .border_color(rgb(BORDER_LIGHT))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .font_family("Menlo")
                                        .text_xs()
                                        .text_color(rgb(TEXT_SECONDARY))
                                        .truncate()
                                        .child(php_s.clone()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap_1p5()
                                        .child(light_button(
                                            "tp-copy-json",
                                            "Copy JSON",
                                            cx.listener(move |this, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    json_s.clone(),
                                                ));
                                                this.table_export_status =
                                                    Some(("Copied row JSON".to_string(), false));
                                                cx.notify();
                                            }),
                                        ))
                                        .child(light_button(
                                            "tp-copy-php",
                                            "Copy PHP",
                                            cx.listener(move |this, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    php_s.clone(),
                                                ));
                                                this.table_export_status =
                                                    Some(("Copied row PHP".to_string(), false));
                                                cx.notify();
                                            }),
                                        )),
                                ),
                        )
                    })
                    // 6. Pagination Footer (when > 1 page)
                    .when(total_pages > 1, |el| {
                        el.child(
                            div()
                                .px_4()
                                .py_1p5()
                                .bg(rgb(BG_CANVAS))
                                .border_t_1()
                                .border_color(rgb(BORDER_LIGHT))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(TEXT_MUTED))
                                        .child(format!("{}-{} of {} entries", start + 1, end, total)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap_1p5()
                                        .child(light_button(
                                            "tp-prev",
                                            "Prev",
                                            cx.listener(|this, _, _, cx| {
                                                if this.table_page > 0 {
                                                    this.table_page -= 1;
                                                    cx.notify();
                                                }
                                            }),
                                        ))
                                        .child(light_button(
                                            "tp-next",
                                            "Next",
                                            cx.listener(move |this, _, _, cx| {
                                                if this.table_page + 1 < total_pages {
                                                    this.table_page += 1;
                                                    cx.notify();
                                                }
                                            }),
                                        )),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_object_graph_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom_pct = self.object_graph_view.read(cx).zoom_percent();

        div()
            .id("object-graph-backdrop")
            .key_context("ObjectGraph")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgba(0x090d164d))
            .flex()
            .flex_col()
            .justify_end()
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlay(window, cx);
            }))
            .child(
                div()
                    .id("object-graph-sheet")
                    .w_full()
                    .h(relative(0.86))
                    .bg(rgb(0x14161a))
                    .border_t_1()
                    .border_color(rgb(0x334155))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    // Dark studio header bar with centered "Object Graph" title + Zoom controls
                    .child(
                        div()
                            .h(px(36.0))
                            .px_4()
                            .bg(rgb(0x1f2430))
                            .border_b_1()
                            .border_color(rgb(0x2e3646))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .w(px(230.0))
                                    .text_xs()
                                    .text_color(rgb(0x64748b))
                                    .truncate()
                                    .child("Scroll to zoom · Drag to pan · Click relation"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(0xe2e8f0))
                                    .child("Object Graph"),
                            )
                            .child(
                                div()
                                    .w(px(230.0))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_end()
                                    .gap_1p5()
                                    .child(dark_graph_button(
                                        "btn-graph-zoom-out",
                                        "−",
                                        cx.listener(|this, _, _, cx| {
                                            this.object_graph_view.update(cx, |g, cx| g.zoom_out(cx));
                                        }),
                                    ))
                                    .child(
                                        div()
                                            .min_w(px(44.0))
                                            .flex()
                                            .justify_center()
                                            .font_family("Menlo")
                                            .text_xs()
                                            .text_color(rgb(0xcbd5e1))
                                            .child(format!("{zoom_pct}%")),
                                    )
                                    .child(dark_graph_button(
                                        "btn-graph-zoom-in",
                                        "+",
                                        cx.listener(|this, _, _, cx| {
                                            this.object_graph_view.update(cx, |g, cx| g.zoom_in(cx));
                                        }),
                                    ))
                                    .child(dark_graph_button(
                                        "btn-graph-fit",
                                        "Fit",
                                        cx.listener(|this, _, _, cx| {
                                            this.object_graph_view.update(cx, |g, cx| g.fit_view(cx));
                                        }),
                                    ))
                                    .child(dark_close_button(
                                        "btn-graph-close",
                                        cx.listener(|this, _, window, cx| {
                                            this.close_overlay(window, cx);
                                        }),
                                    )),
                            ),
                    )
                    // Interactive Blueprint Canvas
                    .child(div().flex_1().overflow_hidden().child(self.object_graph_view.clone())),
            )
    }

    fn render_html_preview_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let raw_html = self.mail_webview.read(cx).html().to_string();

        div()
            .id("html-preview-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgba(0x090d1666))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, window, cx| {
                this.close_overlay(window, cx);
            }))
            .child(
                div()
                    .id("html-preview-dialog")
                    .w(relative(0.95))
                    .h(relative(0.91))
                    .bg(rgb(0x1f2430))
                    .border_1()
                    .border_color(rgb(0x334155))
                    .rounded_lg()
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    // 1. Centered "HTML Preview" Header Bar
                    .child(
                        div()
                            .h(px(34.0))
                            .px_4()
                            .bg(rgb(0x1f2430))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(div().w(px(140.0)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(0xe2e8f0))
                                    .child("HTML Preview"),
                            )
                            .child(
                                div()
                                    .w(px(140.0))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_end()
                                    .gap_1p5()
                                    .child(dark_graph_button(
                                        "btn-copy-mail-html",
                                        "Copy HTML",
                                        cx.listener(move |_, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                raw_html.clone(),
                                            ));
                                        }),
                                    ))
                                    .child(dark_close_button(
                                        "btn-close-html-preview",
                                        cx.listener(|this, _, window, cx| {
                                            this.close_overlay(window, cx);
                                        }),
                                    )),
                            ),
                    )
                    // 2. Embedded Native macOS WKWebView Viewport
                    .child(
                        div()
                            .flex_1()
                            .px_3()
                            .bg(rgb(0x1f2430))
                            .overflow_hidden()
                            .child(
                                div()
                                    .size_full()
                                    .bg(rgb(0xedf2f7))
                                    .overflow_hidden()
                                    .child(self.mail_webview.clone()),
                            ),
                    )
                    // 3. Bottom Footer Caption (matching screenshot)
                    .child(
                        div()
                            .h(px(30.0))
                            .px_4()
                            .bg(rgb(0x1f2430))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_end()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x94a3b8))
                                    .child("Hit ⌘ + R to refresh. This will run the complete code again."),
                            ),
                    ),
            )
    }

    fn render_select_dropdown(
        &self,
        id: &str,
        value_label: &str,
        open_upward: bool,
        empty_placeholder: Option<&str>,
        options: Vec<(
            String,
            bool,
            Box<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>,
        )>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id_shared = SharedString::from(id.to_string());
        let id_toggle = id_shared.clone();
        let is_open = self.active_dropdown_id.as_ref() == Some(&id_shared);

        div()
            .relative()
            .w_full()
            .child(
                div()
                    .id(id_shared)
                    .w_full()
                    .h(px(36.0))
                    .px_3()
                    .rounded_lg()
                    .bg(rgb(0x181c23))
                    .border_1()
                    .border_color(if is_open {
                        rgb(0x3b82f6)
                    } else {
                        rgb(0x364052)
                    })
                    .hover(move |s| {
                        s.bg(rgb(0x1f2530)).border_color(if is_open {
                            rgb(0x3b82f6)
                        } else {
                            rgb(0x4b5870)
                        })
                    })
                    .cursor_pointer()
                    .overflow_hidden()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if this.active_dropdown_id.as_ref() == Some(&id_toggle) {
                            this.active_dropdown_id = None;
                        } else {
                            this.active_dropdown_id = Some(id_toggle.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(0xf1f5f9))
                            .child(value_label.to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(if is_open {
                                rgb(0x60a5fa)
                            } else {
                                rgb(0x94a3b8)
                            })
                            .child(if is_open { "▴" } else { "▾" }),
                    ),
            )
            .when(is_open, |el| {
                let menu_id = SharedString::from(format!("{id}-menu"));
                let is_empty_opts = options.is_empty();
                let placeholder_msg = empty_placeholder.map(|m| m.to_string());
                let menu = div()
                    .id(menu_id)
                    .occlude()
                    .absolute()
                    .left_0()
                    .right_0()
                    .when(open_upward, |m| m.bottom(px(40.0)))
                    .when(!open_upward, |m| m.top(px(40.0)))
                    .max_h(px(220.0))
                    .overflow_y_scroll()
                    .rounded_lg()
                    .bg(rgb(0x1e2430))
                    .border_1()
                    .border_color(rgb(0x3b4963))
                    .shadow_lg()
                    .p_1()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_scroll_wheel(|_, _, cx| {
                        cx.stop_propagation();
                    })
                    .when(is_empty_opts, |m| {
                        m.child(
                            div()
                                .px_2p5()
                                .py_2()
                                .text_xs()
                                .text_color(rgb(0x94a3b8))
                                .child(
                                    placeholder_msg
                                        .unwrap_or_else(|| "No options available".to_string()),
                                ),
                        )
                    })
                    .children(options.into_iter().enumerate().map(
                        |(idx, (opt_label, is_sel, handler))| {
                            let row_id = SharedString::from(format!("{id}-opt-{idx}"));
                            div()
                                .id(row_id)
                                .px_2p5()
                                .py_1p5()
                                .rounded_md()
                                .when(is_sel, |r| r.bg(rgba(0x2563eb38)))
                                .hover(move |s| {
                                    s.bg(if is_sel {
                                        rgba(0x2563eb52)
                                    } else {
                                        rgb(0x2a3344).into()
                                    })
                                })
                                .cursor_pointer()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.active_dropdown_id = None;
                                    handler(this, window, cx);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_xs()
                                        .font_weight(if is_sel {
                                            gpui::FontWeight::SEMIBOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(if is_sel {
                                            rgb(0xffffff)
                                        } else {
                                            rgb(0xe2e8f0)
                                        })
                                        .child(opt_label),
                                )
                                .when(is_sel, |r| {
                                    r.child(
                                        div()
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x60a5fa))
                                            .child("✓"),
                                    )
                                })
                        },
                    ));
                el.child(gpui::deferred(menu).with_priority(20))
            })
            .into_any_element()
    }

    fn render_bool_dropdown(
        &self,
        id: &str,
        current: bool,
        true_label: &str,
        false_label: &str,
        on_change: impl Fn(&mut Self, bool, &mut Window, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // On/off settings render as a macOS-style switch; the labels are kept
        // only for call-site readability.
        let _ = (true_label, false_label);
        ai_ui::render_macos_switch(
            id,
            current,
            false,
            true,
            cx.listener(move |this, _, window, cx| {
                this.active_dropdown_id = None;
                on_change(this, !current, window, cx);
                cx.notify();
            }),
        )
    }

    /// Compact segmented control for 2–3 short, mutually exclusive options.
    fn render_segmented(
        &self,
        id: &str,
        options: Vec<(
            &'static str,
            bool,
            Box<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>,
        )>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .flex_none()
            .h(px(30.0))
            .p(px(2.0))
            .rounded_lg()
            .bg(rgb(0x181c23))
            .border_1()
            .border_color(rgb(0x364052))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .children(options.into_iter().enumerate().map(|(idx, (label, is_sel, handler))| {
                div()
                    .id(SharedString::from(format!("{id}-seg-{idx}")))
                    .h_full()
                    .px_3()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_xs()
                    .when(is_sel, |el| {
                        el.bg(rgb(0x334056))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xf8fafc))
                    })
                    .when(!is_sel, |el| {
                        el.text_color(rgb(0x94a3b8))
                            .hover(|s| s.bg(rgb(0x232a36)).text_color(rgb(0xe2e8f0)))
                    })
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.active_dropdown_id = None;
                        if !is_sel {
                            handler(this, window, cx);
                        }
                        cx.notify();
                    }))
                    .child(label)
            }))
            .into_any_element()
    }

    fn render_setting_control_row(
        &self,
        key: SettingKey,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let s = &self.settings;
        match key {
            SettingKey::PhpBinary => settings_row_rich(
                "PHP binary",
                div()
                    .id("settings-php-path")
                    .w_full()
                    .h(px(36.0))
                    .pl_3()
                    .pr_1p5()
                    .rounded_lg()
                    .bg(rgb(0x181c23))
                    .border_1()
                    .border_color(rgb(0x364052))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .font_family("Menlo")
                            .text_xs()
                            .text_color(rgb(0xf1f5f9))
                            .child(middle_truncate_path(&s.php_binary, 34)),
                    )
                    .child(settings_action_btn(
                        "settings-select-php",
                        "Choose…",
                        cx.listener(|this, _, _, cx| {
                            this.select_php_binary_dialog(cx);
                        }),
                    ))
                    .into_any_element(),
                Some(php_status_caption(&self.detected_php_version)),
            ),
            SettingKey::AskBeforeClosingTab => settings_row(
                "Ask before closing tab",
                self.render_bool_dropdown(
                    "settings-ask-close",
                    s.ask_before_closing_tab,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.ask_before_closing_tab = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Confirm before closing a scratchpad tab with `⌘ W`."),
            ),
            SettingKey::WelcomeTab => settings_row(
                "Welcome tab",
                self.render_bool_dropdown(
                    "settings-welcome-tab",
                    s.show_welcome_tab,
                    "Show",
                    "Hide",
                    |this, val, _, cx| {
                        this.settings.show_welcome_tab = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Open the Get Started workspace hub on launch."),
            ),
            SettingKey::Theme => settings_row(
                "Theme",
                self.render_segmented(
                    "settings-theme",
                    [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark]
                        .into_iter()
                        .map(|mode| {
                            let on_select: Box<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)> =
                                Box::new(move |this, _, cx| {
                                    this.settings.theme = mode;
                                    crate::theme::apply_native_appearance(mode);
                                    this.persist_and_apply_settings(cx);
                                    cx.refresh_windows();
                                });
                            (mode.label(), s.theme == mode, on_select)
                        })
                        .collect(),
                    cx,
                ),
                Some("Follow the system appearance, or always use light or dark."),
            ),
            SettingKey::Layout => settings_row(
                "Layout",
                self.render_segmented(
                    "settings-layout",
                    vec![
                        (
                            "Side by side",
                            s.layout == PaneLayout::Vertical,
                            Box::new(|this, _, cx| {
                                this.settings.layout = PaneLayout::Vertical;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                        (
                            "Stacked",
                            s.layout == PaneLayout::Horizontal,
                            Box::new(|this, _, cx| {
                                this.settings.layout = PaneLayout::Horizontal;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                    ],
                    cx,
                ),
                Some("Place the output pane beside or below the editor."),
            ),
            SettingKey::Toolbar => settings_row(
                "Navigation rail",
                self.render_bool_dropdown(
                    "settings-toolbar",
                    s.show_toolbar,
                    "Show",
                    "Hide",
                    |this, val, _, cx| {
                        this.settings.show_toolbar = val;
                        this.hovered_rail_tooltip = None;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Show the icon rail on the left edge of the window."),
            ),
            SettingKey::ProjectExplorer => settings_row(
                "Project explorer",
                self.render_bool_dropdown(
                    "settings-project-explorer",
                    s.show_explorer,
                    "On",
                    "Off",
                    |this, val, _, cx| {
                        this.settings.show_explorer = val;
                        this.show_sidebar = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Show the Models & Mailables sidebar. Toggle it any time with `⌘ B`."),
            ),
            SettingKey::AlwaysOnTop => settings_row(
                "Always on top",
                self.render_bool_dropdown(
                    "settings-always-top",
                    s.always_on_top,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.always_on_top = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Keep the Laralah window floating above other windows."),
            ),
            SettingKey::AutoHideOutput => settings_row(
                "Automatically hide output",
                self.render_bool_dropdown(
                    "settings-auto-hide-out",
                    s.auto_hide_output,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.auto_hide_output = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Expand the editor to full width until code is evaluated."),
            ),
            SettingKey::AutoEvaluateCode => settings_row(
                "Auto evaluate code",
                self.render_bool_dropdown(
                    "settings-auto-eval",
                    s.auto_evaluate_code,
                    "Yes — Automatically evaluate",
                    "No — Run manually (⌘ R)",
                    |this, val, _, cx| {
                        this.settings.auto_evaluate_code = val;
                        this.auto_run = val && !this.active_target.is_ssh();
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Re-evaluate local scratchpads automatically after typing."),
            ),
            SettingKey::EvaluatingCode => settings_row(
                "Evaluating code",
                self.render_select_dropdown(
                    "settings-eval-scope",
                    s.evaluate_scope.label(),
                    false,
                    None,
                    vec![
                        (
                            EvaluateScope::SelectedIfAvailable.label().to_string(),
                            s.evaluate_scope == EvaluateScope::SelectedIfAvailable,
                            Box::new(|this, _, cx| {
                                this.settings.evaluate_scope = EvaluateScope::SelectedIfAvailable;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                        (
                            EvaluateScope::FullBuffer.label().to_string(),
                            s.evaluate_scope == EvaluateScope::FullBuffer,
                            Box::new(|this, _, cx| {
                                this.settings.evaluate_scope = EvaluateScope::FullBuffer;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                    ],
                    cx,
                ),
                Some("Choose whether active text selections run independently."),
            ),
            SettingKey::AutoLog => settings_row(
                "Auto log",
                self.render_bool_dropdown(
                    "settings-auto-log",
                    s.auto_log,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.auto_log = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Record evaluation output snapshots automatically."),
            ),
            SettingKey::OutputType => settings_row(
                "Output type",
                self.render_segmented(
                    "settings-output-type",
                    vec![
                        (
                            "Buffered",
                            s.buffered_output,
                            Box::new(|this, _, cx| {
                                this.settings.buffered_output = true;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                        (
                            "Streamed",
                            !s.buffered_output,
                            Box::new(|this, _, cx| {
                                this.settings.buffered_output = false;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                    ],
                    cx,
                ),
                Some("Buffered shows the result at once and enables Detail Dive cards and SQL capture; streamed prints line by line."),
            ),
            SettingKey::DefaultOutputType => settings_row(
                "Default output type",
                self.render_segmented(
                    "settings-default-output",
                    vec![
                        (
                            DefaultOutputSetting::DetailDive.label(),
                            s.default_output == DefaultOutputSetting::DetailDive,
                            Box::new(|this, _, cx| {
                                this.settings.default_output = DefaultOutputSetting::DetailDive;
                                this.output_mode = OutputMode::DetailDive;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                        (
                            DefaultOutputSetting::CliOutput.label(),
                            s.default_output == DefaultOutputSetting::CliOutput,
                            Box::new(|this, _, cx| {
                                this.settings.default_output = DefaultOutputSetting::CliOutput;
                                this.output_mode = OutputMode::RawCli;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                    ],
                    cx,
                ),
                Some("Initial output view used when opening a workspace or tab."),
            ),
            SettingKey::AutoExpandDetailDive => settings_row(
                "Automatically expand detail dive",
                div()
                    .w(px(210.0))
                    .child(self.render_select_dropdown(
                    "settings-expand-dive",
                    s.detail_dive_expansion.label(),
                    false,
                    None,
                    vec![
                        (
                            DetailDiveExpansion::FirstLevel.label().to_string(),
                            s.detail_dive_expansion == DetailDiveExpansion::FirstLevel,
                            Box::new(|this, _, cx| {
                                this.settings.detail_dive_expansion =
                                    DetailDiveExpansion::FirstLevel;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                        (
                            DetailDiveExpansion::CollapseAll.label().to_string(),
                            s.detail_dive_expansion == DetailDiveExpansion::CollapseAll,
                            Box::new(|this, _, cx| {
                                this.settings.detail_dive_expansion =
                                    DetailDiveExpansion::CollapseAll;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                        (
                            DetailDiveExpansion::ExpandAll.label().to_string(),
                            s.detail_dive_expansion == DetailDiveExpansion::ExpandAll,
                            Box::new(|this, _, cx| {
                                this.settings.detail_dive_expansion =
                                    DetailDiveExpansion::ExpandAll;
                                this.persist_and_apply_settings(cx);
                            }),
                        ),
                    ],
                    cx,
                ))
                    .into_any_element(),
                Some("Default expansion depth for Eloquent models and nested objects."),
            ),
            SettingKey::ExecutionTimeMemory => settings_row(
                "Execution time & memory",
                self.render_bool_dropdown(
                    "settings-exec-metrics",
                    s.show_execution_metrics,
                    "Show",
                    "Hide",
                    |this, val, _, cx| {
                        this.settings.show_execution_metrics = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Display runtime duration and peak memory in the status footer."),
            ),
            SettingKey::LineNumbers => settings_row(
                "Line numbers",
                self.render_bool_dropdown(
                    "settings-line-numbers",
                    s.show_line_numbers,
                    "Show",
                    "Hide",
                    |this, val, _, cx| {
                        this.settings.show_line_numbers = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Display line numbers in the PHP editor gutter."),
            ),
            SettingKey::LineWrap => settings_row(
                "Line wrap",
                self.render_bool_dropdown(
                    "settings-line-wrap",
                    s.line_wrap,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.line_wrap = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Soft-wrap long PHP lines to fit the editor viewport."),
            ),
            SettingKey::IndentationSize => settings_row(
                "Indentation size",
                settings_number_box("settings-indent", self.indent_size_input.clone(), 1.0, 1.0, 16.0, 0),
                Some("Number of spaces inserted per Tab press (1–16)."),
            ),
            SettingKey::InlineErrorMessages => settings_row(
                "Inline error messages",
                self.render_bool_dropdown(
                    "settings-inline-errors",
                    s.show_inline_errors,
                    "Show",
                    "Hide",
                    |this, val, _, cx| {
                        this.settings.show_inline_errors = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Highlight syntax and runtime error lines inside the editor on run."),
            ),
            SettingKey::UseVimKeymap => settings_row(
                "Use Vim keymap",
                self.render_bool_dropdown(
                    "settings-vim-keymap",
                    s.use_vim_keymap,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.use_vim_keymap = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Enable modal VIM navigation bindings in the code editor."),
            ),
            SettingKey::HistoryEntries => settings_row(
                "History entries",
                settings_number_box("settings-history", self.history_entries_input.clone(), 10.0, 10.0, 2000.0, 0),
                Some("Maximum number of scratchpad history snapshots retained."),
            ),
            SettingKey::FontFamily => {
                let font_opts = available_editor_fonts()
                    .iter()
                    .map(|f| {
                        let f_name = (*f).to_string();
                        let f_set = f_name.clone();
                        let is_sel = s.font_family == *f;
                        (
                            f_name,
                            is_sel,
                            Box::new(move |this: &mut Self, _: &mut Window, cx: &mut Context<Self>| {
                                this.settings.font_family = f_set.clone();
                                this.persist_and_apply_settings(cx);
                            })
                                as Box<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>,
                        )
                    })
                    .collect();
                settings_row(
                    "Font family",
                    div()
                    .w(px(210.0))
                    .child(self.render_select_dropdown(
                        "settings-font-family",
                        &s.font_family,
                        false,
                        None,
                        font_opts,
                        cx,
                    ))
                    .into_any_element(),
                    Some("Monospace typeface used in the PHP editor and output panes."),
                )
            }
            SettingKey::FontSize => settings_row(
                "Font size",
                settings_number_box("settings-font-size", self.font_size_input.clone(), 1.0, 9.0, 32.0, 0),
                Some("Editor font size in points (9–32)."),
            ),
            SettingKey::LineHeight => settings_row(
                "Line height",
                settings_number_box("settings-line-height", self.line_height_input.clone(), 0.1, 1.1, 3.0, 1),
                Some("Relative line-height multiplier for editor lines (1.1–3.0)."),
            ),
            SettingKey::FontLigatures => settings_row(
                "Use font ligatures",
                self.render_bool_dropdown(
                    "settings-ligatures",
                    s.use_font_ligatures,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.use_font_ligatures = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Render programming ligatures (`=>`, `->`, `!==`) when supported."),
            ),
            SettingKey::AiEnabled => settings_row(
                "Enable AI assistant",
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .child(ai_ui::render_macos_switch(
                        "settings-ai-enabled-switch",
                        s.ai_enabled,
                        false,
                        true,
                        cx.listener(|this, _, _, cx| {
                            let next = !this.settings.ai_enabled;
                            this.settings.ai_enabled = next;
                            this.settings.show_ai_sidebar = next;
                            this.show_ai_sidebar = next;
                            if !next {
                                this.ai_picker_open = false;
                                this.ai_effort_menu_open = false;
                                this.ai_sessions_menu_open = false;
                            }
                            save_settings(&this.settings);
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
                Some("Show the AI assistant sidebar on the right. Toggle it with `⌘ I`."),
            ),
            SettingKey::AiAutoWrite => settings_row(
                "Auto-write code to editor",
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .child(ai_ui::render_macos_switch(
                        "settings-ai-autowrite-switch",
                        s.ai_auto_write_editor,
                        false,
                        true,
                        cx.listener(|this, _, _, cx| {
                            this.settings.ai_auto_write_editor = !this.settings.ai_auto_write_editor;
                            save_settings(&this.settings);
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
                Some("Automatically write generated Eloquent / PHP snippets into the active scratchpad — undo with `⌘ Z`."),
            ),
            SettingKey::AiAllowSshSchema => settings_row(
                "Allow schema inspection on SSH",
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .child(ai_ui::render_macos_switch(
                        "settings-ai-ssh-schema-switch",
                        s.ai_allow_ssh_schema,
                        false,
                        true,
                        cx.listener(|this, _, _, cx| {
                            this.settings.ai_allow_ssh_schema = !this.settings.ai_allow_ssh_schema;
                            save_settings(&this.settings);
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
                Some("Let the assistant read model and table columns (read-only) on SSH connections."),
            ),
            SettingKey::AiAgentsList => self.render_ai_agents_settings_card(cx),
            SettingKey::DefaultWorkingDirectory => {
                let default_dir_label = s
                    .default_working_directory
                    .as_deref()
                    .map(|d| middle_truncate_path(d, 30))
                    .unwrap_or_else(|| "None — use last workspace".to_string());
                let has_default_dir = s.default_working_directory.is_some();
                let default_dir_color = if has_default_dir {
                    0xf1f5f9
                } else {
                    0x94a3b8
                };
                settings_row(
                    "Default working directory",
                    div()
                        .w_full()
                        .h(px(36.0))
                        .pl_3()
                        .pr_1p5()
                        .rounded_lg()
                        .bg(rgb(0x181c23))
                        .border_1()
                        .border_color(rgb(0x364052))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_1p5()
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(default_dir_color))
                                .child(default_dir_label),
                        )
                        .child(settings_action_btn(
                            "settings-select-workdir",
                            "Choose…",
                            cx.listener(|this, _, _, cx| {
                                this.select_default_workdir_dialog(cx);
                            }),
                        ))
                        .when(has_default_dir, |el| {
                            el.child(settings_action_btn(
                                "settings-clear-workdir",
                                "Clear",
                                cx.listener(|this, _, _, cx| {
                                    this.settings.default_working_directory = None;
                                    this.persist_and_apply_settings(cx);
                                }),
                            ))
                        })
                        .into_any_element(),
                    Some("The default project folder to use when opening a new tab."),
                )
            }
            SettingKey::CollisionEnabled => settings_row(
                "Collision enabled",
                self.render_bool_dropdown(
                    "settings-collision",
                    s.collision_enabled,
                    "Yes",
                    "No",
                    |this, val, _, cx| {
                        this.settings.collision_enabled = val;
                        this.persist_and_apply_settings(cx);
                    },
                    cx,
                ),
                Some("Format exceptions with Collision. Individual drivers can still override this."),
            ),
            SettingKey::AutocompletionServicePort => settings_row(
                "Autocompletion service port",
                div()
                    .w(px(96.0))
                    .child(settings_text_box(self.autocomplete_port_input.clone()))
                    .into_any_element(),
                Some("Local port reserved for PHP and Laravel symbol completion."),
            ),
            SettingKey::RecentFolders => {
                let recent_count = self.projects.len();
                let recent_caption = match recent_count {
                    0 => "None saved".to_string(),
                    1 => "1 folder".to_string(),
                    n => format!("{n} folders"),
                };
                settings_row(
                    "Recent folders",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x94a3b8))
                                .child(recent_caption),
                        )
                        .when(recent_count > 0, |el| {
                            el.child(settings_action_btn(
                                "settings-clear-recent",
                                "Clear",
                                cx.listener(|this, _, _, cx| {
                                    this.projects.clear();
                                    save_recent_folders(&[]);
                                    cx.notify();
                                }),
                            ))
                        })
                        .into_any_element(),
                    Some("Workspace history shown on the Get Started screen."),
                )
            }
        }
    }

    fn render_settings_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_tab = self.active_settings_tab;
        let query = self.settings_search_input.read(cx).text().to_string();
        let trimmed_query = query.trim().to_string();
        let is_searching = !trimmed_query.is_empty();
        let search_results =
            search_settings(&trimmed_query, &self.settings, &self.detected_php_version);
        let total_matches = search_results.total_count();

        let tab_body: gpui::AnyElement = if is_searching {
            if total_matches == 0 {
                div()
                    .w_full()
                    .mt_2()
                    .p_8()
                    .rounded_xl()
                    .bg(rgb(0x232833))
                    .border_1()
                    .border_color(rgb(0x313847))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(0xf1f5f9))
                            .child(format!("No settings matching \"{trimmed_query}\"")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x94a3b8))
                            .child("Try searching for Toolbar, AI, PHP Binary, Layout, Font, VIM, or a shortcut."),
                    )
                    .child(
                        div().mt_1().child(settings_action_btn(
                            "settings-empty-clear-btn",
                            "Clear search",
                            cx.listener(|this, _, window, cx| {
                                this.settings_search_input.update(cx, |tf, cx| {
                                    tf.set_text("", cx);
                                });
                                let fh = this.settings_search_input.focus_handle(cx);
                                window.focus(&fh);
                                cx.notify();
                            }),
                        )),
                    )
                    .into_any_element()
            } else {
                let mut groups: Vec<(SettingsTab, &'static str, Vec<SettingKey>)> = Vec::new();
                for meta in &search_results.matching_specs {
                    if let Some(last) = groups
                        .last_mut()
                        .filter(|(t, sec, _)| *t == meta.tab && *sec == meta.section)
                    {
                        last.2.push(meta.key);
                    } else {
                        groups.push((meta.tab, meta.section, vec![meta.key]));
                    }
                }

                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .children(groups.into_iter().enumerate().map(
                        |(g_idx, (grp_tab, grp_sec, keys))| {
                            let jump_id = format!("settings-jump-group-{g_idx}");
                            let jump_label = format!("Open in {} →", grp_tab.label());
                            let rows: Vec<gpui::AnyElement> = keys
                                .into_iter()
                                .map(|k| self.render_setting_control_row(k, cx))
                                .collect();
                            settings_section_card(
                                settings_breadcrumb_header(
                                    grp_tab.label(),
                                    grp_sec,
                                    &jump_id,
                                    &jump_label,
                                    cx.listener(move |this, _, _, cx| {
                                        this.active_settings_tab = grp_tab;
                                        this.settings_search_input.update(cx, |tf, cx| {
                                            tf.set_text("", cx);
                                        });
                                        cx.notify();
                                    }),
                                ),
                                rows,
                            )
                        },
                    ))
                    .when(!search_results.matching_shortcuts.is_empty(), |el| {
                        el.child(
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .gap_2p5()
                                .child(settings_breadcrumb_header(
                                    "Shortcuts",
                                    "Keyboard shortcuts",
                                    "settings-jump-shortcuts",
                                    "Open in Shortcuts →",
                                    cx.listener(|this, _, _, cx| {
                                        this.active_settings_tab = SettingsTab::Shortcuts;
                                        this.settings_search_input.update(cx, |tf, cx| {
                                            tf.set_text("", cx);
                                        });
                                        cx.notify();
                                    }),
                                ))
                                .child(render_shortcuts_card(&search_results.matching_shortcuts)),
                        )
                    })
                    .into_any_element()
            }
        } else if active_tab == SettingsTab::Shortcuts {
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_2p5()
                .child(settings_section_header("Keyboard shortcuts"))
                .child(render_shortcuts_card(shortcut_reference_rows()))
                .into_any_element()
        } else {
            let mut sections: Vec<(&'static str, Vec<SettingKey>)> = Vec::new();
            for meta in all_setting_specs().iter().filter(|m| m.tab == active_tab) {
                if let Some(last) = sections.last_mut().filter(|(sec, _)| *sec == meta.section) {
                    last.1.push(meta.key);
                } else {
                    sections.push((meta.section, vec![meta.key]));
                }
            }
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_6()
                .children(sections.into_iter().map(|(sec_title, keys)| {
                    let rows: Vec<gpui::AnyElement> = keys
                        .into_iter()
                        .map(|k| self.render_setting_control_row(k, cx))
                        .collect();
                    settings_section_card(settings_section_header(sec_title), rows)
                }))
                .into_any_element()
        };

        let match_badge_text = if total_matches == 1 {
            "1 match".to_string()
        } else {
            format!("{total_matches} matches")
        };

        div()
            .id("settings-modal-root")
            .key_context("SettingsModal")
            .absolute()
            .inset_0()
            .bg(rgb(0x1c2028))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_click(cx.listener(|this, _, _, cx| {
                if this.active_dropdown_id.is_some() {
                    this.active_dropdown_id = None;
                    cx.notify();
                }
            }))
            .flex()
            .flex_col()
            // Top bar
            .child(
                div()
                    .h(px(40.0))
                    .flex_none()
                    .pl(px(86.0))
                    .pr_4()
                    .bg(rgb(0x171a21))
                    .border_b_1()
                    .border_color(rgb(0x2a313e))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(div().w(px(52.0)))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xe2e8f0))
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .w(px(52.0))
                            .flex()
                            .justify_end()
                            .child(dark_close_button(
                                "settings-close-btn",
                                cx.listener(|this, _, window, cx| {
                                    this.close_overlay(window, cx);
                                }),
                            )),
                    ),
            )
            // Body: Left Navigation Sidebar + Right Centered Card Column
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .flex_row()
                    // Left Sidebar
                    .child(
                        div()
                            .w(px(216.0))
                            .h_full()
                            .flex_none()
                            .bg(rgb(0x171a21))
                            .border_r_1()
                            .border_color(rgb(0x2a313e))
                            .px_3()
                            .pt_5()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .when(is_searching, |sb| {
                                sb.child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .rounded_lg()
                                        .bg(rgb(0x263042))
                                        .border_1()
                                        .border_color(rgb(0x3b4963))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_2p5()
                                                .child(
                                                    svg()
                                                        .path("icons/search.svg")
                                                        .size(px(14.0))
                                                        .text_color(rgb(0x93c5fd)),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::MEDIUM)
                                                        .text_color(rgb(0xf8fafc))
                                                        .child("All Results"),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .px_1p5()
                                                .rounded_sm()
                                                .bg(rgb(0x36435a))
                                                .font_family("Menlo")
                                                .text_xs()
                                                .text_color(rgb(0xf8fafc))
                                                .child(total_matches.to_string()),
                                        ),
                                )
                                .child(div().my_1p5().h(px(1.0)).bg(rgb(0x262d3a)))
                            })
                            .children(SettingsTab::all().into_iter().map(|tab| {
                                let is_active = !is_searching && tab == active_tab;
                                let tab_matches = search_results.count_for_tab(tab);
                                let icon_path = match tab {
                                    SettingsTab::General => "icons/settings.svg",
                                    SettingsTab::Editor => "icons/code.svg",
                                    SettingsTab::Ai => "icons/sparkles.svg",
                                    SettingsTab::Advanced => "icons/layers.svg",
                                    SettingsTab::Shortcuts => "icons/terminal.svg",
                                };
                                div()
                                    .id(SharedString::from(format!(
                                        "settings-tab-{}",
                                        tab.label()
                                    )))
                                    .px_3()
                                    .py_2()
                                    .rounded_lg()
                                    .cursor_pointer()
                                    .bg(if is_active {
                                        rgb(0x263042)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .border_1()
                                    .border_color(if is_active {
                                        rgb(0x38465e)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .hover(|st| {
                                        if is_active {
                                            st.bg(rgb(0x2c374b))
                                        } else {
                                            st.bg(rgb(0x202530))
                                        }
                                    })
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.active_settings_tab = tab;
                                        if !this.settings_search_input.read(cx).text().is_empty() {
                                            this.settings_search_input.update(cx, |tf, cx| {
                                                tf.set_text("", cx);
                                            });
                                        }
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2p5()
                                            .child(
                                                svg()
                                                    .path(icon_path)
                                                    .size(px(14.5))
                                                    .text_color(if is_active {
                                                        rgb(0x60a5fa)
                                                    } else if is_searching && tab_matches == 0 {
                                                        rgb(0x4b5563)
                                                    } else {
                                                        rgb(0x94a3b8)
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(if is_active {
                                                        gpui::FontWeight::SEMIBOLD
                                                    } else {
                                                        gpui::FontWeight::NORMAL
                                                    })
                                                    .text_color(if is_active {
                                                        rgb(0xf8fafc)
                                                    } else if is_searching && tab_matches == 0 {
                                                        rgb(0x64748b)
                                                    } else {
                                                        rgb(0xcbd5e1)
                                                    })
                                                    .child(tab.label()),
                                            ),
                                    )
                                    .when(is_searching, |row| {
                                        row.child(
                                            div()
                                                .px_1p5()
                                                .rounded_sm()
                                                .bg(if tab_matches > 0 {
                                                    rgb(0x283040)
                                                } else {
                                                    rgba(0x00000000)
                                                })
                                                .font_family("Menlo")
                                                .text_xs()
                                                .text_color(if tab_matches > 0 {
                                                    rgb(0xe2e8f0)
                                                } else {
                                                    rgb(0x4b5563)
                                                })
                                                .child(tab_matches.to_string()),
                                        )
                                    })
                            })),
                    )
                    // Right Column: Pinned Search Bar + Centered Scrollable Section Cards
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            // Pinned Search Header (exact same 760px centered column as cards below)
                            .child(
                                div()
                                    .w_full()
                                    .flex_none()
                                    .px_10()
                                    .pt_6()
                                    .pb_4()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .child(
                                        div()
                                            .w_full()
                                            .max_w(px(760.0))
                                            .h(px(40.0))
                                            .px_3p5()
                                            .rounded_xl()
                                            .bg(rgb(0x232833))
                                            .border_1()
                                            .border_color(if is_searching {
                                                rgb(0x3b82f6)
                                            } else {
                                                rgb(0x343d4e)
                                            })
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2p5()
                                            .child(
                                                svg()
                                                    .path("icons/search.svg")
                                                    .size(px(15.0))
                                                    .text_color(if is_searching {
                                                        rgb(0x60a5fa)
                                                    } else {
                                                        rgb(0x94a3b8)
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .overflow_hidden()
                                                    .child(self.settings_search_input.clone()),
                                            )
                                            .when(!is_searching, |bar| {
                                                bar.child(
                                                    div()
                                                        .px_2()
                                                        .py_0p5()
                                                        .rounded_sm()
                                                        .bg(rgb(0x181c23))
                                                        .border_1()
                                                        .border_color(rgb(0x343d4e))
                                                        .font_family("Menlo")
                                                        .text_xs()
                                                        .text_color(rgb(0x94a3b8))
                                                        .child("⌘ F"),
                                                )
                                            })
                                            .when(is_searching, |bar| {
                                                bar.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0x94a3b8))
                                                        .child(match_badge_text),
                                                )
                                                .child(
                                                    div()
                                                        .id("settings-search-clear-btn")
                                                        .px_2p5()
                                                        .py_0p5()
                                                        .rounded_md()
                                                        .bg(rgb(0x2e3646))
                                                        .border_1()
                                                        .border_color(rgb(0x434e63))
                                                        .hover(|s| s.bg(rgb(0x384256)))
                                                        .cursor_pointer()
                                                        .text_xs()
                                                        .text_color(rgb(0xf1f5f9))
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.settings_search_input.update(
                                                                    cx,
                                                                    |tf, cx| {
                                                                        tf.set_text("", cx);
                                                                    },
                                                                );
                                                                let fh = this
                                                                    .settings_search_input
                                                                    .focus_handle(cx);
                                                                window.focus(&fh);
                                                                cx.notify();
                                                            },
                                                        ))
                                                        .child("Clear"),
                                                )
                                            }),
                                    ),
                            )
                            // Scrollable Settings Cards
                            .child(
                                div()
                                    .id("settings-content-scroll")
                                    .w_full()
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .px_10()
                                    .pt_1()
                                    .pb_12()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .child(div().w_full().max_w(px(760.0)).pb_8().child(tab_body)),
                            ),
                    ),
            )
    }

    fn render_status_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let raw_laravel_ver = self
            .last_result
            .as_ref()
            .map(|r| r.laravel_version.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or(&self.detected_laravel_version);
        let laravel_ver = raw_laravel_ver
            .strip_prefix("Laravel ")
            .unwrap_or(raw_laravel_ver);

        let metrics_text = if let Some(ref res) = self.last_result {
            format_footer_metrics(res.duration_ms, res.memory_mb)
        } else {
            "Ready".to_string()
        };

        let ssh_active = self.active_target.ssh_info();
        let is_ssh = ssh_active.is_some();

        // Dark high-attention bar when connected over SSH (matching SSH footer screenshot);
        // harmonious light studio chrome when working locally so the UI feels unified.
        let bar_bg = if is_ssh { 0x1f232b } else { 0xf8fafc };
        let bar_border = if is_ssh { 0x323842 } else { BORDER_LIGHT };
        let text_primary = if is_ssh { 0xd1d5db } else { 0x475569 };
        let hover_bg = if is_ssh { 0x282d37 } else { 0xeef2f6 };

        let connection_cell: gpui::AnyElement = match &self.active_target {
            ExecutionTarget::Ssh {
                name,
                docker_container,
                color_hex,
                ..
            } => {
                let c_hex = *color_hex;
                let c_name = name.clone();
                let c_docker = docker_container
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                div()
                    .h_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(
                        div()
                            .id("footer-ssh-connected")
                            .h_full()
                            .px_3()
                            .hover(move |s| s.bg(rgb(hover_bg)))
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_ssh_modal(window, cx);
                            }))
                            .child(div().size(px(9.0)).rounded_full().bg(rgb(c_hex)))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .text_xs()
                                    .child(div().text_color(rgb(0xd1d5db)).child("Connected to"))
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0xffffff))
                                            .child(c_name),
                                    )
                                    .when_some(c_docker, |el, dc| {
                                        el.child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded_sm()
                                                .bg(rgb(0x1e3a5f))
                                                .border_1()
                                                .border_color(rgb(0x2563eb))
                                                .font_family("Menlo")
                                                .text_xs()
                                                .text_color(rgb(0x93c5fd))
                                                .child(format!("docker: {dc}")),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id("footer-ssh-disconnect")
                            .mr_2()
                            .px_2()
                            .py_0p5()
                            .rounded_sm()
                            .bg(rgb(0x2b303b))
                            .border_1()
                            .border_color(rgb(0x3f4756))
                            .hover(|s| s.bg(rgb(0x374151)))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(0x9ca3af))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.disconnect_ssh(cx);
                            }))
                            .child("Disconnect"),
                    )
                    .into_any_element()
            }
            ExecutionTarget::Docker { container_name, .. } => div()
                .id("footer-docker-connected")
                .h_full()
                .px_3()
                .hover(move |s| s.bg(rgb(hover_bg)))
                .cursor_pointer()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_palette("@", window, cx);
                }))
                .child(div().size(px(8.0)).rounded_full().bg(rgb(0x0284c7)))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .child(div().text_color(rgb(text_primary)).child("Docker:"))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(TEXT_MAIN))
                                .child(container_name.clone()),
                        ),
                )
                .into_any_element(),
            ExecutionTarget::Local { project_path } => {
                let folder_name = std::path::Path::new(project_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("local")
                    .to_string();
                div()
                    .id("footer-local-status")
                    .h_full()
                    .px_3()
                    .hover(move |s| s.bg(rgb(hover_bg)))
                    .cursor_pointer()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_ssh_modal(window, cx);
                    }))
                    .child(div().size(px(7.5)).rounded_full().bg(rgb(0x10b981)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(text_primary))
                            .child(format!("Local ({folder_name})")),
                    )
                    .into_any_element()
            }
        };

        let php_missing_local = !is_php_detected(&self.detected_php_version)
            && matches!(self.active_target, ExecutionTarget::Local { .. });
        let php_short = if php_missing_local {
            "PHP not found — select…".to_string()
        } else {
            extract_short_php_badge(&self.detected_php_version)
        };

        div()
            .h(px(28.0))
            .w_full()
            .flex_none()
            .bg(rgb(bar_bg))
            .border_t_1()
            .border_color(rgb(bar_border))
            .when_some(ssh_active, |el, (_, ssh_col)| {
                el.border_b_2().border_color(rgb(ssh_col))
            })
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .h_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    // Segment 1: Laravel <version> ⓘ
                    .child(
                        div()
                            .h_full()
                            .px_3()
                            .border_r_1()
                            .border_color(rgb(bar_border))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(text_primary))
                                    .child(format!("Laravel {laravel_ver}")),
                            )
                            .child(
                                div()
                                    .size(px(13.0))
                                    .rounded_full()
                                    .bg(if is_ssh { rgb(0x334155) } else { rgb(0xe2e8f0) })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(if is_ssh {
                                        rgb(0xe2e8f0)
                                    } else {
                                        rgb(0x475569)
                                    })
                                    .child("i"),
                            ),
                    )
                    // Segment 2: Execution Duration / Memory
                    .when(self.settings.show_execution_metrics, |el| {
                        el.child(
                            div()
                                .h_full()
                                .px_3()
                                .border_r_1()
                                .border_color(rgb(bar_border))
                                .flex()
                                .flex_row()
                                .items_center()
                                .font_family("Menlo")
                                .text_xs()
                                .text_color(rgb(text_primary))
                                .child(metrics_text),
                        )
                    })
                    // Segment 3: Connected SSH / Target indicator
                    .child(connection_cell),
            )
            .child(
                div()
                    .h_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(
                        div()
                            .id("status-php-badge")
                            .h_full()
                            .px_3()
                            .flex()
                            .items_center()
                            .font_family("Menlo")
                            .text_xs()
                            .text_color(rgb(if php_missing_local {
                                0xdc2626
                            } else if is_ssh {
                                0x9ca3af
                            } else {
                                TEXT_MUTED
                            }))
                            .when(php_missing_local, |el| {
                                el.cursor_pointer()
                                    .hover(|s| s.bg(rgb(0xfef2f2)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_php_binary_dialog(cx);
                                    }))
                            })
                            .child(php_short),
                    ),
            )
    }

    fn render_ssh_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let modal_body: gpui::AnyElement = match self.ssh_modal_mode {
            SshModalMode::List => self.render_ssh_list_view(cx).into_any_element(),
            SshModalMode::Form => self.render_ssh_form_view(cx).into_any_element(),
        };

        div()
            .id("ssh-modal-backdrop")
            .key_context("SshModal")
            .absolute()
            .inset_0()
            .bg(rgba(0x0f172a88))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(48.0))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.close_overlay(window, cx);
                }),
            )
            .child(
                div()
                    .id("ssh-modal-card")
                    .w(px(650.0))
                    .max_h(px(650.0))
                    .bg(rgb(0x21242c))
                    .border_1()
                    .border_color(rgb(0x343946))
                    .rounded_lg()
                    .shadow_xl()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        let mut changed = false;
                        if this.active_dropdown_id.is_some() {
                            this.active_dropdown_id = None;
                            changed = true;
                        }
                        if this.ssh_docker_picker_open {
                            this.ssh_docker_picker_open = false;
                            changed = true;
                        }
                        if changed {
                            cx.notify();
                        }
                    }))
                    .child(modal_body),
            )
    }

    fn render_ssh_list_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.ssh_search_input.read(cx).text().trim().to_lowercase();
        let search_empty = self.ssh_search_input.read(cx).text().is_empty();
        let active_ssh_id = match &self.active_target {
            ExecutionTarget::Ssh { id, .. } => Some(id.as_str()),
            _ => None,
        };

        let matches_query = |p: &SshConnectionProfile| -> bool {
            if query.is_empty() {
                return true;
            }
            p.label.to_lowercase().contains(&query)
                || p.host.to_lowercase().contains(&query)
                || p.username.to_lowercase().contains(&query)
                || p.application_folder.to_lowercase().contains(&query)
                || (p.docker_enabled && p.docker_container.to_lowercase().contains(&query))
                || p.group
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query)
        };

        // Group connections by named groups, then Ungrouped
        let mut sections: Vec<(Option<String>, Vec<SshConnectionProfile>)> = Vec::new();
        for grp in &self.ssh_store.groups {
            let items: Vec<SshConnectionProfile> = self
                .ssh_store
                .connections
                .iter()
                .filter(|c| c.group.as_deref() == Some(grp.as_str()) && matches_query(c))
                .cloned()
                .collect();
            if !items.is_empty() || query.is_empty() {
                sections.push((Some(grp.clone()), items));
            }
        }
        let ungrouped: Vec<SshConnectionProfile> = self
            .ssh_store
            .connections
            .iter()
            .filter(|c| {
                let has_known_grp = c
                    .group
                    .as_ref()
                    .is_some_and(|g| self.ssh_store.groups.contains(g));
                !has_known_grp && matches_query(c)
            })
            .cloned()
            .collect();
        if !ungrouped.is_empty() {
            sections.push((None, ungrouped));
        }

        div()
            .flex_1()
            .overflow_hidden()
            .flex()
            .flex_col()
            // Top header bar
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(0x323842))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xf3f4f6))
                            .child("SSH Connections"),
                    )
                    .child(dark_close_button(
                        "ssh-list-close",
                        cx.listener(|this, _, window, cx| {
                            this.close_overlay(window, cx);
                        }),
                    )),
            )
            // Search bar + `+ Connection` / `+ Group` buttons matching screenshot 1
            .child(
                div()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .border_b_1()
                    .border_color(rgb(0x2d323e))
                    .flex()
                    .flex_col()
                    .gap_3()
                    // Search input with `Press [ CMD + F ] to focus search` overlay when empty
                    .child(
                        div()
                            .id("ssh-search-box")
                            .relative()
                            .h(px(38.0))
                            .px_3p5()
                            .rounded_md()
                            .bg(rgb(0x363c49))
                            .border_1()
                            .border_color(rgb(0x454d5e))
                            .flex()
                            .flex_row()
                            .items_center()
                            .cursor_text()
                            .on_click(cx.listener(|this, _, window, cx| {
                                let fh = this.ssh_search_input.focus_handle(cx);
                                window.focus(&fh);
                            }))
                            .child(self.ssh_search_input.clone())
                            .when(search_empty, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .left(px(14.0))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .text_xs()
                                        .text_color(rgb(0xd1d5db))
                                        .child("Press")
                                        .child(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded_sm()
                                                .border_1()
                                                .border_color(rgb(0xd1d5db))
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(0xffffff))
                                                .child("CMD + F"),
                                        )
                                        .child("to focus search"),
                                )
                            }),
                    )
                    // `+ Connection` & `+ Group` buttons row
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2p5()
                            .child(
                                div()
                                    .id("ssh-btn-new-connection")
                                    .px_4()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(0x2563eb))
                                    .hover(|s| s.bg(rgb(0x3b82f6)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(fixed_rgb(0xffffff))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_new_ssh_connection(window, cx);
                                    }))
                                    .child("+ Connection"),
                            )
                            .child(
                                div()
                                    .id("ssh-btn-new-group")
                                    .px_4()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(0x262c38))
                                    .border_1()
                                    .border_color(rgb(0x454d5e))
                                    .hover(|s| s.bg(rgb(0x323948)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xf3f4f6))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.ssh_creating_group = !this.ssh_creating_group;
                                        if this.ssh_creating_group {
                                            let fh = this.ssh_group_input.focus_handle(cx);
                                            window.focus(&fh);
                                        }
                                        cx.notify();
                                    }))
                                    .child("+ Group"),
                            ),
                    )
                    // Optional inline New Group bar
                    .when(self.ssh_creating_group, |el| {
                        el.child(
                            div()
                                .p_2p5()
                                .rounded_md()
                                .bg(rgb(0x2a2f3a))
                                .border_1()
                                .border_color(rgb(0x454d5e))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .h(px(30.0))
                                        .px_2p5()
                                        .rounded_sm()
                                        .bg(rgb(0x363c49))
                                        .flex()
                                        .items_center()
                                        .child(self.ssh_group_input.clone()),
                                )
                                .child(
                                    div()
                                        .id("ssh-confirm-group")
                                        .px_3()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(rgb(0x2563eb))
                                        .hover(|s| s.bg(rgb(0x3b82f6)))
                                        .cursor_pointer()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(fixed_rgb(0xffffff))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.create_ssh_group(cx);
                                        }))
                                        .child("Create"),
                                )
                                .child(settings_action_btn(
                                    "ssh-cancel-group",
                                    "Cancel",
                                    cx.listener(|this, _, _, cx| {
                                        this.ssh_creating_group = false;
                                        cx.notify();
                                    }),
                                )),
                        )
                    }),
            )
            // Scrollable connections & groups list
            .child(
                div()
                    .id("ssh-connections-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_5()
                    .py_3p5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .when(self.ssh_store.connections.is_empty(), |el| {
                        el.child(
                            div()
                                .py_8()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(rgb(0x9ca3af))
                                .child("No SSH connections yet — click '+ Connection' above to add a server."),
                        )
                    })
                    .children(sections.into_iter().map(|(grp_opt, conns)| {
                        let grp_title = grp_opt.unwrap_or_else(|| "Connections".to_string());
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(0x9ca3af))
                                    .child(grp_title),
                            )
                            .when(conns.is_empty(), |el| {
                                el.child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .bg(rgb(0x282c36))
                                        .text_xs()
                                        .text_color(rgb(0x6b7280))
                                        .child("Empty group"),
                                )
                            })
                            .children(conns.into_iter().map(|prof| {
                                let is_connected = active_ssh_id == Some(prof.id.as_str());
                                let prof_for_connect = prof.clone();
                                let prof_id_edit = prof.id.clone();
                                let prof_id_del = prof.id.clone();
                                let docker_opt =
                                    prof.active_docker_container().map(str::to_string);
                                let subtitle = match docker_opt.as_deref() {
                                    Some(c) if prof.is_local_docker() => format!(
                                        "local docker: {} · {}",
                                        c, prof.application_folder
                                    ),
                                    Some(c) => format!(
                                        "{}@{}:{} · docker: {} · {}",
                                        prof.username,
                                        prof.host,
                                        prof.port,
                                        c,
                                        prof.application_folder
                                    ),
                                    None => format!(
                                        "{}@{}:{} · {}",
                                        prof.username,
                                        prof.host,
                                        prof.port,
                                        prof.application_folder
                                    ),
                                };

                                div()
                                    .id(SharedString::from(format!("ssh-row-{}", prof.id)))
                                    .px_3p5()
                                    .py_2p5()
                                    .rounded_md()
                                    .bg(if is_connected {
                                        rgb(0x2d3342)
                                    } else {
                                        rgb(0x292e39)
                                    })
                                    .border_1()
                                    .border_color(if is_connected {
                                        rgb(prof.color_hex)
                                    } else {
                                        rgb(0x394050)
                                    })
                                    .hover(|s| s.bg(rgb(0x323846)))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                div()
                                                    .size(px(10.0))
                                                    .flex_none()
                                                    .rounded_full()
                                                    .bg(rgb(prof.color_hex)),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .overflow_hidden()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_0p5()
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .font_weight(
                                                                        gpui::FontWeight::SEMIBOLD,
                                                                    )
                                                                    .text_color(rgb(0xf3f4f6))
                                                                    .child(prof.label.clone()),
                                                            )
                                                            .when(docker_opt.is_some(), |el| {
                                                                el.child(
                                                                    div()
                                                                        .px_1p5()
                                                                        .py_0p5()
                                                                        .rounded_sm()
                                                                        .bg(rgb(0x1e3a5f))
                                                                        .border_1()
                                                                        .border_color(rgb(0x2563eb))
                                                                        .text_xs()
                                                                        .text_color(rgb(0x93c5fd))
                                                                        .child("Docker"),
                                                                )
                                                            })
                                                            .when(is_connected, |el| {
                                                                el.child(
                                                                    div()
                                                                        .px_1p5()
                                                                        .py_0p5()
                                                                        .rounded_sm()
                                                                        .bg(rgb(0x14532d))
                                                                        .text_xs()
                                                                        .text_color(rgb(0x86efac))
                                                                        .child("Connected"),
                                                                )
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .font_family("Menlo")
                                                            .text_xs()
                                                            .text_color(rgb(0x9ca3af))
                                                            .child(subtitle),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1p5()
                                            .child(if is_connected {
                                                settings_action_btn(
                                                    &format!("ssh-disc-{}", prof.id),
                                                    "Disconnect",
                                                    cx.listener(|this, _, _, cx| {
                                                        this.disconnect_ssh(cx);
                                                    }),
                                                )
                                            } else {
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "ssh-conn-{}",
                                                        prof.id
                                                    )))
                                                    .px_3()
                                                    .py_1()
                                                    .rounded_sm()
                                                    .bg(rgb(0x2563eb))
                                                    .hover(|s| s.bg(rgb(0x3b82f6)))
                                                    .cursor_pointer()
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(fixed_rgb(0xffffff))
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.connect_ssh_profile(
                                                                &prof_for_connect,
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    ))
                                                    .child("Connect")
                                                    .into_any_element()
                                            })
                                            .child(settings_action_btn(
                                                &format!("ssh-edit-{}", prof.id),
                                                "Edit",
                                                cx.listener(move |this, _, window, cx| {
                                                    this.start_edit_ssh_connection(
                                                        &prof_id_edit,
                                                        window,
                                                        cx,
                                                    );
                                                }),
                                            ))
                                            .child(settings_action_btn(
                                                &format!("ssh-del-{}", prof.id),
                                                "×",
                                                cx.listener(move |this, _, _, cx| {
                                                    this.delete_ssh_profile(&prof_id_del, cx);
                                                }),
                                            )),
                                    )
                            }))
                    })),
            )
    }

    fn render_ssh_form_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let form_title = if self.ssh_editing_id.is_some() {
            "Edit Connection"
        } else {
            "New Connection"
        };
        let local_path_text = self
            .ssh_form_local_path
            .as_deref()
            .unwrap_or("–")
            .to_string();
        let secret_label = match self.ssh_form_auth_method {
            SshAuthMethod::Password => "Password",
            _ => "Passphrase",
        };
        let group_label = self
            .ssh_form_group
            .as_deref()
            .unwrap_or("None")
            .to_string();
        let is_masked = self.ssh_passphrase_input.read(cx).is_masked();
        let is_local = self.ssh_form_kind == ConnectionKind::LocalDocker;

        div()
            .flex_1()
            .overflow_hidden()
            .flex()
            .flex_col()
            // Top bar matching screenshots 2 & 3 (`New Connection` | `[ Save & Connect ]` `[ Save ]`)
            .child(
                div()
                    .px_5()
                    .py_3p5()
                    .border_b_1()
                    .border_color(rgb(0x323842))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2p5()
                            .child(
                                div()
                                    .id("ssh-form-back")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(rgb(0x2d3340))
                                    .hover(|s| s.bg(rgb(0x394152)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(rgb(0xd1d5db))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.ssh_modal_mode = SshModalMode::List;
                                        window.focus(&this.focus_handle);
                                        cx.notify();
                                    }))
                                    .child("←"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xffffff))
                                    .child(form_title),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("ssh-btn-save-connect")
                                    .px_3p5()
                                    .py_1p5()
                                    .rounded_sm()
                                    .bg(rgb(0x262c38))
                                    .border_1()
                                    .border_color(rgb(0x454d5e))
                                    .hover(|s| s.bg(rgb(0x323948)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xf3f4f6))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_ssh_form(true, window, cx);
                                    }))
                                    .child("Save & Connect"),
                            )
                            .child(
                                div()
                                    .id("ssh-btn-save")
                                    .px_3p5()
                                    .py_1p5()
                                    .rounded_sm()
                                    .bg(rgb(0x2563eb))
                                    .hover(|s| s.bg(rgb(0x3b82f6)))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(fixed_rgb(0xffffff))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_ssh_form(false, window, cx);
                                    }))
                                    .child("Save"),
                            ),
                    ),
            )
            // Scrollable form body
            .child(
                div()
                    .id("ssh-form-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_5()
                    .py_4()
                    .flex()
                    .flex_col()
                    .gap_3p5()
                    .when_some(self.ssh_form_error.as_ref(), |el, err| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(rgb(0x451a1a))
                                .border_1()
                                .border_color(rgb(0xef4444))
                                .text_xs()
                                .text_color(rgb(0xfca5a5))
                                .child(err.clone()),
                        )
                    })
                    // 0. Connection type: SSH server or a Docker container on this machine
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(ssh_field_label("Connection Type"))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .p_0p5()
                                    .gap_0p5()
                                    .rounded_lg()
                                    .bg(rgb(0x181c23))
                                    .border_1()
                                    .border_color(rgb(0x364052))
                                    .children([ConnectionKind::Ssh, ConnectionKind::LocalDocker].into_iter().map(|kind| {
                                        let selected = self.ssh_form_kind == kind;
                                        div()
                                            .id(SharedString::from(format!("ssh-form-kind-{kind:?}")))
                                            .flex_1()
                                            .py_1p5()
                                            .rounded_md()
                                            .flex()
                                            .justify_center()
                                            .cursor_pointer()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .when(selected, |el| el.bg(rgb(0x2563eb)).text_color(fixed_rgb(0xffffff)))
                                            .when(!selected, |el| {
                                                el.text_color(rgb(0x94a3b8)).hover(|s| s.bg(rgb(0x262c38)))
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.set_ssh_form_kind(kind, window, cx);
                                            }))
                                            .child(kind.label())
                                    })),
                            ),
                    )
                    // 1. Load SSH Config
                    .when(!is_local, |el| el.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(ssh_field_label("Load SSH Config"))
                            .child({
                                let cfg_opts = self
                                    .ssh_config_hosts
                                    .iter()
                                    .map(|cfg| {
                                        let cfg_clone = cfg.clone();
                                        let desc = format!(
                                            "{}  ({}{})",
                                            cfg.alias,
                                            cfg.user
                                                .as_deref()
                                                .map(|u| format!("{u}@"))
                                                .unwrap_or_default(),
                                            cfg.hostname
                                        );
                                        let selected_lbl =
                                            format!("{} ({})", cfg.alias, cfg.hostname);
                                        let is_sel =
                                            self.ssh_selected_config_label == selected_lbl;
                                        (
                                            desc,
                                            is_sel,
                                            Box::new(
                                                move |this: &mut Self,
                                                      _: &mut Window,
                                                      cx: &mut Context<Self>| {
                                                    this.apply_ssh_config_host(&cfg_clone, cx);
                                                },
                                            )
                                                as Box<
                                                    dyn Fn(
                                                        &mut Self,
                                                        &mut Window,
                                                        &mut Context<Self>,
                                                    ),
                                                >,
                                        )
                                    })
                                    .collect();
                                self.render_select_dropdown(
                                    "ssh-load-config-select",
                                    &self.ssh_selected_config_label,
                                    false,
                                    Some("No Host entries found in ~/.ssh/config"),
                                    cfg_opts,
                                    cx,
                                )
                            }),
                    ))
                    // 2. Row: Label | Host + Port (22)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_3p5()
                            .child(
                                div()
                                    .w_1_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(ssh_field_label("Label"))
                                    .child(settings_text_box(self.ssh_label_input.clone())),
                            )
                            .when(!is_local, |el| el.child(
                                div()
                                    .w_1_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(ssh_field_label("Host"))
                                    .child({
                                        let host_in = self.ssh_host_input.clone();
                                        let host_click = host_in.clone();
                                        let port_in = self.ssh_port_input.clone();
                                        let port_click = port_in.clone();
                                        div()
                                            .h(px(36.0))
                                            .px_3()
                                            .rounded_lg()
                                            .bg(rgb(0x181c23))
                                            .border_1()
                                            .border_color(rgb(0x364052))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .h_full()
                                                    .overflow_hidden()
                                                    .cursor(gpui::CursorStyle::IBeam)
                                                    .on_mouse_down(
                                                        gpui::MouseButton::Left,
                                                        move |ev, window, cx| {
                                                            cx.stop_propagation();
                                                            host_click.update(cx, |tf, cx| {
                                                                tf.handle_mouse_down(
                                                                    ev, window, cx,
                                                                );
                                                            });
                                                        },
                                                    )
                                                    .child(host_in),
                                            )
                                            .child(
                                                div()
                                                    .w(px(46.0))
                                                    .h_full()
                                                    .pl_2()
                                                    .border_l_1()
                                                    .border_color(rgb(0x364052))
                                                    .cursor(gpui::CursorStyle::IBeam)
                                                    .on_mouse_down(
                                                        gpui::MouseButton::Left,
                                                        move |ev, window, cx| {
                                                            cx.stop_propagation();
                                                            port_click.update(cx, |tf, cx| {
                                                                tf.handle_mouse_down(
                                                                    ev, window, cx,
                                                                );
                                                            });
                                                        },
                                                    )
                                                    .child(port_in),
                                            )
                                    }),
                            )),
                    )
                    // 3. Row: Username | Application Folder (host path when Docker OFF, container path when Docker ON)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_3p5()
                            .when(!is_local, |el| el.child(
                                div()
                                    .w_1_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(ssh_field_label("Username"))
                                    .child(settings_text_box(self.ssh_user_input.clone())),
                            ))
                            .child(
                                div()
                                    .w_1_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(ssh_field_label(if self.ssh_form_docker_enabled || is_local {
                                        "Docker Application Folder"
                                    } else {
                                        "Application Folder"
                                    }))
                                    .child(settings_text_box(self.ssh_folder_input.clone())),
                            ),
                    )
                    // 3b. Docker Container Switch (On/Off) + Container ID / Name (+ Fetch helper)
                    .child({
                        let docker_on = self.ssh_form_docker_enabled;
                        let is_fetching = self.ssh_docker_fetching;
                        let picker_open = self.ssh_docker_picker_open;
                        let detected = self.ssh_detected_containers.clone();
                        let container_in = self.ssh_docker_container_input.clone();
                        let container_click = container_in.clone();

                        div()
                            .px_3p5()
                            .py_3()
                            .rounded_lg()
                            .bg(rgb(0x232833))
                            .border_1()
                            .border_color(if docker_on || is_local {
                                rgb(0x2563eb)
                            } else {
                                rgb(0x364052)
                            })
                            .flex()
                            .flex_col()
                            .gap_2p5()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .flex()
                                            .flex_col()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(0xf3f4f6))
                                                    .child(if is_local { "Local Docker Container" } else { "Docker Container" }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(0x94a3b8))
                                                    .child(if is_local {
                                                        "Runs PHP with `docker exec` on this machine. No SSH needed."
                                                    } else if docker_on {
                                                        "SSH into host and execute inside container (folder path above is from Docker)"
                                                    } else {
                                                        "Enable to run PHP / Laravel Tinker inside a Docker container on this server"
                                                    }),
                                            ),
                                    )
                                    .when(!is_local, |el| el.child(ai_ui::render_macos_switch(
                                        "ssh-form-docker-switch",
                                        docker_on,
                                        false,
                                        true,
                                        cx.listener(|this, _, window, cx| {
                                            this.toggle_ssh_form_docker(window, cx);
                                        }),
                                    ))),
                            )
                            .when(docker_on || is_local, |el| {
                                el.child(
                                    div()
                                        .pt_1()
                                        .border_t_1()
                                        .border_color(rgb(0x323948))
                                        .flex()
                                        .flex_col()
                                        .gap_1p5()
                                        .child(ssh_field_label(
                                            "Container Name or ID",
                                        ))
                                        .child(
                                            div()
                                                .h(px(36.0))
                                                .pl_3()
                                                .pr_1p5()
                                                .rounded_lg()
                                                .bg(rgb(0x181c23))
                                                .border_1()
                                                .border_color(rgb(0x364052))
                                                .cursor(gpui::CursorStyle::IBeam)
                                                .on_mouse_down(
                                                    gpui::MouseButton::Left,
                                                    move |ev, window, cx| {
                                                        cx.stop_propagation();
                                                        container_click.update(cx, |tf, cx| {
                                                            tf.handle_mouse_down(ev, window, cx);
                                                        });
                                                    },
                                                )
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .justify_between()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .h_full()
                                                        .overflow_hidden()
                                                        .child(container_in),
                                                )
                                                .child(settings_action_btn(
                                                    "ssh-fetch-docker-containers",
                                                    if is_fetching {
                                                        "Fetching…"
                                                    } else {
                                                        "Fetch"
                                                    },
                                                    cx.listener(|this, _, _, cx| {
                                                        this.fetch_ssh_docker_containers(cx);
                                                    }),
                                                )),
                                        )
                                        .when(picker_open, |sub| {
                                            sub.child(
                                                div()
                                                    .id("ssh-docker-detected-list")
                                                    .mt_1()
                                                    .max_h(px(160.0))
                                                    .overflow_y_scroll()
                                                    .rounded_lg()
                                                    .bg(rgb(0x181c23))
                                                    .border_1()
                                                    .border_color(rgb(0x3b4963))
                                                    .p_1p5()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1()
                                                    .on_mouse_down(
                                                        gpui::MouseButton::Left,
                                                        |_, _, cx| {
                                                            cx.stop_propagation();
                                                        },
                                                    )
                                                    .on_click(|_, _, cx| {
                                                        cx.stop_propagation();
                                                    })
                                                    .when(detected.is_empty(), |menu| {
                                                        menu.child(
                                                            div()
                                                                .px_2p5()
                                                                .py_2()
                                                                .text_xs()
                                                                .text_color(rgb(0x94a3b8))
                                                                .child("No running Docker containers found via `docker ps`."),
                                                        )
                                                    })
                                                    .children(detected.into_iter().map(
                                                        |ctr| {
                                                            let ctr_name = if ctr.name.is_empty() {
                                                                ctr.id.clone()
                                                            } else {
                                                                ctr.name.clone()
                                                            };
                                                            let ctr_workdir = ctr.workdir.clone();
                                                            let meta = format!(
                                                                "{} · {} · {}",
                                                                ctr.id, ctr.image, ctr.workdir
                                                            );
                                                            let row_id = SharedString::from(
                                                                format!(
                                                                    "ssh-docker-opt-{}",
                                                                    ctr.id
                                                                ),
                                                            );
                                                            div()
                                                                .id(row_id)
                                                                .px_2p5()
                                                                .py_1p5()
                                                                .rounded_md()
                                                                .hover(|s| s.bg(rgb(0x263042)))
                                                                .cursor_pointer()
                                                                .flex()
                                                                .flex_col()
                                                                .gap_0p5()
                                                                .on_click(cx.listener(
                                                                    move |this, _, _, cx| {
                                                                        let chosen_name =
                                                                            ctr_name.clone();
                                                                        let chosen_dir =
                                                                            ctr_workdir.clone();
                                                                        this.ssh_docker_container_input
                                                                            .update(
                                                                                cx,
                                                                                |tf, cx| {
                                                                                    tf.set_text(
                                                                                        chosen_name,
                                                                                        cx,
                                                                                    );
                                                                                },
                                                                            );
                                                                        if !chosen_dir.is_empty() {
                                                                            this.ssh_folder_input
                                                                                .update(
                                                                                    cx,
                                                                                    |tf, cx| {
                                                                                        tf.set_text(
                                                                                            chosen_dir,
                                                                                            cx,
                                                                                        );
                                                                                    },
                                                                                );
                                                                        }
                                                                        this.ssh_docker_picker_open =
                                                                            false;
                                                                        cx.notify();
                                                                    },
                                                                ))
                                                                .child(
                                                                    div()
                                                                        .text_xs()
                                                                        .font_weight(
                                                                            gpui::FontWeight::SEMIBOLD,
                                                                        )
                                                                        .text_color(rgb(0xf1f5f9))
                                                                        .child(if ctr.name.is_empty()
                                                                        {
                                                                            ctr.id.clone()
                                                                        } else {
                                                                            ctr.name.clone()
                                                                        }),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .truncate()
                                                                        .font_family("Menlo")
                                                                        .text_xs()
                                                                        .text_color(rgb(0x94a3b8))
                                                                        .child(meta),
                                                                )
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                            })
                    })
                    // 4. Local path + [ Select ]
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(ssh_field_label("Local path"))
                            .child(
                                div()
                                    .h(px(36.0))
                                    .pl_3()
                                    .pr_1p5()
                                    .rounded_md()
                                    .bg(rgb(0x363c49))
                                    .border_1()
                                    .border_color(rgb(0x454d5e))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .flex_1()
                                            .truncate()
                                            .text_xs()
                                            .text_color(if self.ssh_form_local_path.is_some() {
                                                rgb(0xf3f4f6)
                                            } else {
                                                rgb(0x9ca3af)
                                            })
                                            .child(local_path_text),
                                    )
                                    .when(self.ssh_form_local_path.is_some(), |el| {
                                        el.child(settings_action_btn(
                                            "ssh-clear-local-path",
                                            "Clear",
                                            cx.listener(|this, _, _, cx| {
                                                this.ssh_form_local_path = None;
                                                cx.notify();
                                            }),
                                        ))
                                    })
                                    .child(settings_action_btn(
                                        "ssh-select-local-path",
                                        "Select",
                                        cx.listener(|this, _, _, cx| {
                                            this.select_ssh_local_path_dialog(cx);
                                        }),
                                    )),
                            ),
                    )
                    // 5. Authentication Method
                    .when(!is_local, |el| el.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(ssh_field_label("Authentication Method"))
                            .child(self.render_select_dropdown(
                                "ssh-auth-method",
                                self.ssh_form_auth_method.label(),
                                false,
                                None,
                                vec![
                                    (
                                        SshAuthMethod::PrivateKey.label().to_string(),
                                        self.ssh_form_auth_method == SshAuthMethod::PrivateKey,
                                        Box::new(|this, _, _| {
                                            this.ssh_form_auth_method = SshAuthMethod::PrivateKey;
                                        }),
                                    ),
                                    (
                                        SshAuthMethod::SshAgent.label().to_string(),
                                        self.ssh_form_auth_method == SshAuthMethod::SshAgent,
                                        Box::new(|this, _, _| {
                                            this.ssh_form_auth_method = SshAuthMethod::SshAgent;
                                        }),
                                    ),
                                    (
                                        SshAuthMethod::Password.label().to_string(),
                                        self.ssh_form_auth_method == SshAuthMethod::Password,
                                        Box::new(|this, _, _| {
                                            this.ssh_form_auth_method = SshAuthMethod::Password;
                                        }),
                                    ),
                                ],
                                cx,
                            )),
                    ))
                    // 6. Private Key + [ Select ] (when Private Key auth is chosen)
                    .when(!is_local && self.ssh_form_auth_method == SshAuthMethod::PrivateKey, |el| {
                        let key_in = self.ssh_key_input.clone();
                        let key_click = key_in.clone();
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(ssh_field_label("Private Key"))
                                .child(
                                    div()
                                        .h(px(36.0))
                                        .pl_3()
                                        .pr_1p5()
                                        .rounded_lg()
                                        .bg(rgb(0x181c23))
                                        .border_1()
                                        .border_color(rgb(0x364052))
                                        .cursor(gpui::CursorStyle::IBeam)
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            move |ev, window, cx| {
                                                cx.stop_propagation();
                                                key_click.update(cx, |tf, cx| {
                                                    tf.handle_mouse_down(ev, window, cx);
                                                });
                                            },
                                        )
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .h_full()
                                                .overflow_hidden()
                                                .child(key_in),
                                        )
                                        .child(settings_action_btn(
                                            "ssh-select-key",
                                            "Select",
                                            cx.listener(|this, _, _, cx| {
                                                this.select_ssh_private_key_dialog(cx);
                                            }),
                                        )),
                                ),
                        )
                    })
                    // 7. Passphrase / Password + eye reveal toggle
                    .when(!is_local && self.ssh_form_auth_method != SshAuthMethod::SshAgent, |el| {
                        let pass_in = self.ssh_passphrase_input.clone();
                        let pass_click = pass_in.clone();
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(ssh_field_label(secret_label))
                                .child(
                                    div()
                                        .h(px(36.0))
                                        .pl_3()
                                        .pr_2p5()
                                        .rounded_lg()
                                        .bg(rgb(0x181c23))
                                        .border_1()
                                        .border_color(rgb(0x364052))
                                        .cursor(gpui::CursorStyle::IBeam)
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            move |ev, window, cx| {
                                                cx.stop_propagation();
                                                pass_click.update(cx, |tf, cx| {
                                                    tf.handle_mouse_down(ev, window, cx);
                                                });
                                            },
                                        )
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .h_full()
                                                .overflow_hidden()
                                                .child(pass_in),
                                        )
                                        .child(
                                            div()
                                                .id("ssh-toggle-passphrase-mask")
                                                .p_1()
                                                .rounded_sm()
                                                .hover(|s| s.bg(rgb(0x30394a)))
                                                .cursor_pointer()
                                                .on_mouse_down(
                                                    gpui::MouseButton::Left,
                                                    |_, _, cx| {
                                                        cx.stop_propagation();
                                                    },
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.ssh_passphrase_input.update(
                                                        cx,
                                                        |tf, cx| {
                                                            let next = !tf.is_masked();
                                                            tf.set_masked(next, cx);
                                                        },
                                                    );
                                                }))
                                                .child(
                                                    svg()
                                                        .path("icons/eye.svg")
                                                        .size(px(15.0))
                                                        .text_color(if is_masked {
                                                            rgb(0x9ca3af)
                                                        } else {
                                                            rgb(0x60a5fa)
                                                        }),
                                                ),
                                        ),
                                ),
                        )
                    })
                    // 8. Row: Color swatches | Group selector
                    .child(
                        div()
                            .pt_1()
                            .flex()
                            .flex_row()
                            .gap_3p5()
                            .child(
                                div()
                                    .w_1_2()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(ssh_field_label("Color"))
                                    .child(
                                        div()
                                            .h(px(36.0))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_2p5()
                                            .children(SSH_COLOR_SWATCHES.iter().map(
                                                |(hex, name)| {
                                                    let c = *hex;
                                                    let selected = self.ssh_form_color_hex == c;
                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "ssh-swatch-{name}"
                                                        )))
                                                        .size(px(22.0))
                                                        .rounded_full()
                                                        .bg(rgb(c))
                                                        .cursor_pointer()
                                                        .when(selected, |d| {
                                                            d.border_2()
                                                                .border_color(rgb(0xffffff))
                                                        })
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.ssh_form_color_hex = c;
                                                                cx.notify();
                                                            },
                                                        ))
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .w_1_2()
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(ssh_field_label("Group"))
                                    .child({
                                        let id_shared =
                                            SharedString::from("ssh-form-group-select");
                                        let id_toggle = id_shared.clone();
                                        let is_open =
                                            self.active_dropdown_id.as_ref() == Some(&id_shared);
                                        let filter_raw =
                                            self.ssh_group_combo_input.read(cx).text().trim().to_string();
                                        let filter_lc = filter_raw.to_lowercase();

                                        let mut group_names: Vec<String> =
                                            self.ssh_store.groups.clone();
                                        if let Some(ref current_grp) = self.ssh_form_group
                                            && !group_names.contains(current_grp)
                                        {
                                            group_names.push(current_grp.clone());
                                        }

                                        let exact_exists = group_names
                                            .iter()
                                            .any(|g| g.eq_ignore_ascii_case(&filter_raw));
                                        let filtered_groups: Vec<String> = group_names
                                            .into_iter()
                                            .filter(|g| {
                                                filter_lc.is_empty()
                                                    || g.to_lowercase().contains(&filter_lc)
                                            })
                                            .collect();
                                        let show_none = filter_lc.is_empty()
                                            || "none (ungrouped)".contains(&filter_lc);

                                        div()
                                            .relative()
                                            .w_full()
                                            .child(
                                                div()
                                                    .id(id_shared)
                                                    .w_full()
                                                    .h(px(36.0))
                                                    .px_3()
                                                    .rounded_lg()
                                                    .bg(rgb(0x181c23))
                                                    .border_1()
                                                    .border_color(if is_open {
                                                        rgb(0x3b82f6)
                                                    } else {
                                                        rgb(0x364052)
                                                    })
                                                    .hover(move |s| {
                                                        s.bg(rgb(0x1f2530)).border_color(if is_open
                                                        {
                                                            rgb(0x3b82f6)
                                                        } else {
                                                            rgb(0x4b5870)
                                                        })
                                                    })
                                                    .cursor_pointer()
                                                    .overflow_hidden()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap_2()
                                                    .on_mouse_down(
                                                        gpui::MouseButton::Left,
                                                        |_, _, cx| {
                                                            cx.stop_propagation();
                                                        },
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            if this.active_dropdown_id.as_ref()
                                                                == Some(&id_toggle)
                                                            {
                                                                this.active_dropdown_id = None;
                                                            } else {
                                                                this.ssh_group_combo_input.update(
                                                                    cx,
                                                                    |tf, cx| tf.set_text("", cx),
                                                                );
                                                                this.active_dropdown_id =
                                                                    Some(id_toggle.clone());
                                                                let fh = this
                                                                    .ssh_group_combo_input
                                                                    .focus_handle(cx);
                                                                window.focus(&fh);
                                                            }
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .truncate()
                                                            .text_xs()
                                                            .text_color(rgb(0xf1f5f9))
                                                            .child(group_label.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_none()
                                                            .text_xs()
                                                            .text_color(if is_open {
                                                                rgb(0x60a5fa)
                                                            } else {
                                                                rgb(0x94a3b8)
                                                            })
                                                            .child(if is_open { "▴" } else { "▾" }),
                                                    ),
                                            )
                                            .when(is_open, |el| {
                                                let combo_in = self.ssh_group_combo_input.clone();
                                                let combo_click = combo_in.clone();
                                                let is_none_sel = self.ssh_form_group.is_none();
                                                let create_name = filter_raw.clone();
                                                let can_create =
                                                    !create_name.is_empty() && !exact_exists;

                                                let menu = div()
                                                    .id("ssh-form-group-select-menu")
                                                    .occlude()
                                                    .absolute()
                                                    .left_0()
                                                    .right_0()
                                                    .bottom(px(40.0))
                                                    .rounded_lg()
                                                    .bg(rgb(0x1e2430))
                                                    .border_1()
                                                    .border_color(rgb(0x3b4963))
                                                    .shadow_lg()
                                                    .p_1p5()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1()
                                                    .on_mouse_down(
                                                        gpui::MouseButton::Left,
                                                        |_, _, cx| {
                                                            cx.stop_propagation();
                                                        },
                                                    )
                                                    .on_click(|_, _, cx| {
                                                        cx.stop_propagation();
                                                    })
                                                    .on_scroll_wheel(|_, _, cx| {
                                                        cx.stop_propagation();
                                                    })
                                                    .child(
                                                        div()
                                                            .h(px(30.0))
                                                            .px_2p5()
                                                            .rounded_md()
                                                            .bg(rgb(0x141820))
                                                            .border_1()
                                                            .border_color(rgb(0x334155))
                                                            .cursor(gpui::CursorStyle::IBeam)
                                                            .on_mouse_down(
                                                                gpui::MouseButton::Left,
                                                                move |ev, window, cx| {
                                                                    cx.stop_propagation();
                                                                    combo_click.update(
                                                                        cx,
                                                                        |tf, cx| {
                                                                            tf.handle_mouse_down(
                                                                                ev, window, cx,
                                                                            );
                                                                        },
                                                                    );
                                                                },
                                                            )
                                                            .flex()
                                                            .items_center()
                                                            .child(combo_in),
                                                    )
                                                    .child(
                                                        div()
                                                            .id("ssh-group-combo-scroll")
                                                            .max_h(px(176.0))
                                                            .overflow_y_scroll()
                                                            .flex()
                                                            .flex_col()
                                                            .gap_0p5()
                                                            .when(show_none, |list| {
                                                                list.child(
                                                                    div()
                                                                        .id("ssh-grp-opt-none")
                                                                        .px_2p5()
                                                                        .py_1p5()
                                                                        .rounded_md()
                                                                        .when(is_none_sel, |r| {
                                                                            r.bg(rgba(0x2563eb38))
                                                                        })
                                                                        .hover(|s| {
                                                                            s.bg(rgb(0x2a3344))
                                                                        })
                                                                        .cursor_pointer()
                                                                        .flex()
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .justify_between()
                                                                        .on_click(cx.listener(
                                                                            |this, _, _, cx| {
                                                                                cx.stop_propagation();
                                                                                this.ssh_form_group =
                                                                                    None;
                                                                                this.active_dropdown_id =
                                                                                    None;
                                                                                cx.notify();
                                                                            },
                                                                        ))
                                                                        .child(
                                                                            div()
                                                                                .text_xs()
                                                                                .text_color(
                                                                                    rgb(0xe2e8f0),
                                                                                )
                                                                                .child(
                                                                                    "None (Ungrouped)",
                                                                                ),
                                                                        )
                                                                        .when(is_none_sel, |r| {
                                                                            r.child(
                                                                                div()
                                                                                    .text_xs()
                                                                                    .font_weight(
                                                                                        gpui::FontWeight::BOLD,
                                                                                    )
                                                                                    .text_color(
                                                                                        rgb(0x60a5fa),
                                                                                    )
                                                                                    .child("✓"),
                                                                            )
                                                                        }),
                                                                )
                                                            })
                                                            .children(
                                                                filtered_groups
                                                                    .into_iter()
                                                                    .enumerate()
                                                                    .map(|(idx, grp)| {
                                                                        let is_sel = self
                                                                            .ssh_form_group
                                                                            .as_deref()
                                                                            == Some(grp.as_str());
                                                                        let grp_set = grp.clone();
                                                                        div()
                                                                            .id(SharedString::from(
                                                                                format!(
                                                                                    "ssh-grp-opt-{idx}"
                                                                                ),
                                                                            ))
                                                                            .px_2p5()
                                                                            .py_1p5()
                                                                            .rounded_md()
                                                                            .when(is_sel, |r| {
                                                                                r.bg(rgba(
                                                                                    0x2563eb38,
                                                                                ))
                                                                            })
                                                                            .hover(|s| {
                                                                                s.bg(rgb(0x2a3344))
                                                                            })
                                                                            .cursor_pointer()
                                                                            .flex()
                                                                            .flex_row()
                                                                            .items_center()
                                                                            .justify_between()
                                                                            .on_click(cx.listener(
                                                                                move |this,
                                                                                      _,
                                                                                      _,
                                                                                      cx| {
                                                                                    cx.stop_propagation();
                                                                                    this.ssh_form_group =
                                                                                        Some(
                                                                                            grp_set
                                                                                                .clone(),
                                                                                        );
                                                                                    this.active_dropdown_id =
                                                                                        None;
                                                                                    cx.notify();
                                                                                },
                                                                            ))
                                                                            .child(
                                                                                div()
                                                                                    .flex_1()
                                                                                    .truncate()
                                                                                    .text_xs()
                                                                                    .font_weight(if is_sel {
                                                                                        gpui::FontWeight::SEMIBOLD
                                                                                    } else {
                                                                                        gpui::FontWeight::NORMAL
                                                                                    })
                                                                                    .text_color(if is_sel {
                                                                                        rgb(0xffffff)
                                                                                    } else {
                                                                                        rgb(0xe2e8f0)
                                                                                    })
                                                                                    .child(grp),
                                                                            )
                                                                            .when(is_sel, |r| {
                                                                                r.child(
                                                                                    div()
                                                                                        .text_xs()
                                                                                        .font_weight(gpui::FontWeight::BOLD)
                                                                                        .text_color(rgb(0x60a5fa))
                                                                                        .child("✓"),
                                                                                )
                                                                            })
                                                                    }),
                                                            )
                                                            .when(can_create, |list| {
                                                                let new_grp = create_name.clone();
                                                                let label_txt = format!(
                                                                    "+ Create \"{create_name}\""
                                                                );
                                                                list.child(
                                                                    div()
                                                                        .id("ssh-grp-opt-create")
                                                                        .px_2p5()
                                                                        .py_1p5()
                                                                        .rounded_md()
                                                                        .hover(|s| {
                                                                            s.bg(rgb(0x2a3344))
                                                                        })
                                                                        .cursor_pointer()
                                                                        .text_xs()
                                                                        .font_weight(
                                                                            gpui::FontWeight::SEMIBOLD,
                                                                        )
                                                                        .text_color(rgb(0x93c5fd))
                                                                        .on_click(cx.listener(
                                                                            move |this, _, _, cx| {
                                                                                cx.stop_propagation();
                                                                                if !this
                                                                                    .ssh_store
                                                                                    .groups
                                                                                    .contains(
                                                                                        &new_grp,
                                                                                    )
                                                                                {
                                                                                    this.ssh_store
                                                                                        .groups
                                                                                        .push(
                                                                                            new_grp
                                                                                                .clone(),
                                                                                        );
                                                                                    save_ssh_store(
                                                                                        &this
                                                                                            .ssh_store,
                                                                                    );
                                                                                }
                                                                                this.ssh_form_group =
                                                                                    Some(
                                                                                        new_grp
                                                                                            .clone(),
                                                                                    );
                                                                                this.active_dropdown_id =
                                                                                    None;
                                                                                cx.notify();
                                                                            },
                                                                        ))
                                                                        .child(label_txt),
                                                                )
                                                            }),
                                                    );
                                                el.child(gpui::deferred(menu).with_priority(20))
                                            })
                                            .into_any_element()
                                    }),
                            ),
                    ),
            )
    }
    fn render_sidebar_sash(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_dragging = matches!(self.sash_drag, SashDragState::Sidebar { .. });
        let interactive = self.overlay == OverlayKind::None;
        div()
            .relative()
            .w(px(1.0))
            .h_full()
            .flex_none()
            .bg(if is_dragging {
                rgb(0x2563eb)
            } else {
                rgb(BORDER_LIGHT)
            })
            .when(interactive, |el| {
                el.child(
                    gpui::deferred(
                        div()
                            .id("sidebar-resize-sash")
                            .occlude()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(-3.0))
                            .w(px(7.0))
                            .cursor(gpui::CursorStyle::ResizeLeftRight)
                            .when(is_dragging, |h| h.bg(rgba(0x3b82f640)))
                            .hover(|s| s.bg(rgba(0x3b82f640)))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if event.click_count >= 2 {
                                        this.settings.sidebar_width = 216.0;
                                        this.sash_drag = SashDragState::None;
                                        save_settings(&this.settings);
                                    } else {
                                        this.sash_drag = SashDragState::Sidebar {
                                            start_x: f32::from(event.position.x),
                                            start_width: this.settings.sidebar_width,
                                        };
                                    }
                                    cx.notify();
                                }),
                            ),
                    )
                    .with_priority(5),
                )
            })
    }

    fn render_pane_sash(&self, horizontal_stack: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let interactive = self.overlay == OverlayKind::None;
        if horizontal_stack {
            let is_dragging = matches!(self.sash_drag, SashDragState::StackHorizontal { .. });
            div()
                .relative()
                .w_full()
                .h(px(1.0))
                .flex_none()
                .bg(if is_dragging {
                    rgb(0x2563eb)
                } else {
                    rgb(BORDER_LIGHT)
                })
                .when(interactive, |el| {
                    el.child(
                        gpui::deferred(
                            div()
                                .id("pane-stack-sash")
                                .occlude()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(px(-3.0))
                                .h(px(7.0))
                                .cursor(gpui::CursorStyle::ResizeUpDown)
                                .when(is_dragging, |h| h.bg(rgba(0x3b82f640)))
                                .hover(|s| s.bg(rgba(0x3b82f640)))
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        if event.click_count >= 2 {
                                            this.settings.stack_ratio = 0.5;
                                            this.sash_drag = SashDragState::None;
                                            save_settings(&this.settings);
                                        } else {
                                            this.sash_drag = SashDragState::StackHorizontal {
                                                start_y: f32::from(event.position.y),
                                                start_ratio: this.settings.stack_ratio,
                                            };
                                        }
                                        cx.notify();
                                    }),
                                ),
                        )
                        .with_priority(5),
                    )
                })
                .into_any_element()
        } else {
            let is_dragging = matches!(self.sash_drag, SashDragState::SplitVertical { .. });
            div()
                .relative()
                .w(px(1.0))
                .h_full()
                .flex_none()
                .bg(if is_dragging {
                    rgb(0x2563eb)
                } else {
                    rgb(BORDER_LIGHT)
                })
                .when(interactive, |el| {
                    el.child(
                        gpui::deferred(
                            div()
                                .id("pane-split-sash")
                                .occlude()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(-3.0))
                                .w(px(7.0))
                                .cursor(gpui::CursorStyle::ResizeLeftRight)
                                .when(is_dragging, |h| h.bg(rgba(0x3b82f640)))
                                .hover(|s| s.bg(rgba(0x3b82f640)))
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        if event.click_count >= 2 {
                                            this.settings.split_ratio = 0.5;
                                            this.sash_drag = SashDragState::None;
                                            save_settings(&this.settings);
                                        } else {
                                            this.sash_drag = SashDragState::SplitVertical {
                                                start_x: f32::from(event.position.x),
                                                start_ratio: this.settings.split_ratio,
                                            };
                                        }
                                        cx.notify();
                                    }),
                                ),
                        )
                        .with_priority(5),
                    )
                })
                .into_any_element()
        }
    }

    fn update_sash_drag(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(gpui::MouseButton::Left) {
            if self.sash_drag != SashDragState::None {
                self.sash_drag = SashDragState::None;
                save_settings(&self.settings);
                cx.notify();
            }
            return;
        }
        match self.sash_drag {
            SashDragState::None => {}
            SashDragState::Sidebar {
                start_x,
                start_width,
            } => {
                let dx = f32::from(event.position.x) - start_x;
                let max_w = (f32::from(window.viewport_size().width) - 360.0).clamp(200.0, 480.0);
                self.settings.sidebar_width = (start_width + dx).clamp(150.0, max_w);
                cx.notify();
            }
            SashDragState::AiSidebar {
                start_x,
                start_width,
            } => {
                let dx = f32::from(event.position.x) - start_x;
                let max_w = (f32::from(window.viewport_size().width) - 380.0).clamp(280.0, 580.0);
                self.settings.ai_sidebar_width = (start_width - dx).clamp(280.0, max_w);
                cx.notify();
            }
            SashDragState::SplitVertical {
                start_x,
                start_ratio,
            } => {
                let rail_w = if self.settings.show_toolbar { 46.0 } else { 0.0 };
                let side_w =
                    if !self.show_landing && self.settings.show_explorer && self.show_sidebar {
                        self.settings.sidebar_width.clamp(150.0, 480.0) + 1.0
                    } else {
                        0.0
                    };
                let ai_side_w =
                    if !self.show_landing && self.settings.ai_enabled && self.show_ai_sidebar {
                        self.settings.ai_sidebar_width.clamp(280.0, 580.0) + 1.0
                    } else {
                        0.0
                    };
                let avail_w = (f32::from(window.viewport_size().width) - rail_w - side_w - ai_side_w)
                    .max(280.0);
                let dx = f32::from(event.position.x) - start_x;
                self.settings.split_ratio = (start_ratio + dx / avail_w).clamp(0.15, 0.85);
                cx.notify();
            }
            SashDragState::StackHorizontal {
                start_y,
                start_ratio,
            } => {
                let avail_h = (f32::from(window.viewport_size().height) - 104.0).max(200.0); // titlebar + tab strip + footer
                let dy = f32::from(event.position.y) - start_y;
                self.settings.stack_ratio = (start_ratio + dy / avail_h).clamp(0.15, 0.85);
                cx.notify();
            }
        }
    }

    fn render_tab_context_menu(
        &self,
        target_idx: usize,
        pos: gpui::Point<gpui::Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab_count = self.tabs.len();
        let can_close_others = tab_count > 1;
        let can_close_right = target_idx + 1 < tab_count;
        let can_close_left = target_idx > 0 && target_idx < tab_count;
        let is_target_active = target_idx == self.active_tab_idx;

        let vp = window.viewport_size();
        let menu_w = 204.0_f32;
        let menu_h = 166.0_f32;
        let max_x = (f32::from(vp.width) - menu_w - 8.0).max(8.0);
        let max_y = (f32::from(vp.height) - menu_h - 8.0).max(8.0);
        let left_px = px(f32::from(pos.x).clamp(8.0, max_x));
        let top_px = px((f32::from(pos.y) + 2.0).clamp(8.0, max_y));

        let ctx_item = |id: &str,
                        label: &str,
                        shortcut: Option<&str>,
                        enabled: bool,
                        on_pick: Box<dyn Fn(&mut Self, &mut Context<Self>)>,
                        cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(id.to_string()))
                .px_2p5()
                .py_1p5()
                .rounded_md()
                .when(enabled, |r| {
                    r.cursor_pointer()
                        .hover(|s| s.bg(rgb(0xf1f5f9)))
                })
                .when(!enabled, |r| r.cursor_default())
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .text_xs()
                .text_color(if enabled {
                    rgb(TEXT_MAIN)
                } else {
                    rgb(0x94a3b8)
                })
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if !enabled {
                        return;
                    }
                    this.tab_context_menu = None;
                    on_pick(this, cx);
                }))
                .child(div().child(label.to_string()))
                .when_some(shortcut, |r, sc| {
                    r.child(
                        div()
                            .font_family("Menlo")
                            .text_xs()
                            .text_color(rgb(0x94a3b8))
                            .child(sc.to_string()),
                    )
                })
        };

        div()
            .id("tab-context-menu")
            .occlude()
            .absolute()
            .left(left_px)
            .top(top_px)
            .w(px(menu_w))
            .rounded_lg()
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(rgb(0xcbd5e1))
            .shadow_lg()
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_click(|_, _, cx| {
                cx.stop_propagation();
            })
            .child(ctx_item(
                "tab-ctx-close",
                "Close",
                if is_target_active { Some("⌘W") } else { None },
                target_idx < tab_count,
                Box::new(move |this, cx| this.close_tab_at(target_idx, cx)),
                cx,
            ))
            .child(ctx_item(
                "tab-ctx-close-others",
                "Close Others",
                None,
                can_close_others,
                Box::new(move |this, cx| this.close_other_tabs(target_idx, cx)),
                cx,
            ))
            .child(ctx_item(
                "tab-ctx-close-right",
                "Close Tabs to the Right",
                None,
                can_close_right,
                Box::new(move |this, cx| this.close_tabs_to_right(target_idx, cx)),
                cx,
            ))
            .child(ctx_item(
                "tab-ctx-close-left",
                "Close Tabs to the Left",
                None,
                can_close_left,
                Box::new(move |this, cx| this.close_tabs_to_left(target_idx, cx)),
                cx,
            ))
            .child(div().my_0p5().h(px(1.0)).w_full().bg(rgb(0xe2e8f0)))
            .child(ctx_item(
                "tab-ctx-close-all",
                "Close All",
                None,
                true,
                Box::new(|this, cx| this.close_all_tabs(cx)),
                cx,
            ))
    }

    fn finish_sash_drag(&mut self, cx: &mut Context<Self>) {
        if self.sash_drag != SashDragState::None {
            self.sash_drag = SashDragState::None;
            save_settings(&self.settings);
            cx.notify();
        }
    }
}

impl Render for LaralahApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::theme::set_dark(self.settings.theme.resolve_dark(window.appearance())) {
            // Child views may hold cached frames painted with the old palette.
            window.refresh();
        }
        if self.overlay != OverlayKind::HtmlPreview {
            self.mail_webview.update(cx, |wv, _| wv.hide());
        }

        if self.focus_editor_next_frame && self.overlay == OverlayKind::None && !self.show_landing {
            self.focus_editor_next_frame = false;
            let fh = self.editor.focus_handle(cx);
            window.focus(&fh);
        }

        let hide_output_pane = self.settings.auto_hide_output && self.last_result.is_none();
        let is_horizontal_split = self.settings.layout == PaneLayout::Horizontal;

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG_WINDOW))
            .text_color(rgb(TEXT_MAIN))
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let mut dirty = false;
                    if this.active_dropdown_id.is_some() {
                        this.active_dropdown_id = None;
                        dirty = true;
                    }
                    if this.tab_context_menu.is_some() {
                        this.tab_context_menu = None;
                        dirty = true;
                    }
                    if dirty {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    if this.tab_context_menu.is_some() {
                        this.tab_context_menu = None;
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_run))
            .on_action(cx.listener(Self::on_open_directory))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_ai_sidebar))
            .on_action(cx.listener(Self::on_submit_ai_prompt))
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_toggle_settings))
            .on_action(cx.listener(Self::on_toggle_ssh_modal))
            .on_action(cx.listener(Self::on_toggle_palette))
            .on_action(cx.listener(Self::on_toggle_logs))
            .on_action(cx.listener(Self::on_focus_log_search))
            .on_action(cx.listener(Self::on_focus_settings_search))
            .on_action(cx.listener(Self::on_focus_ssh_search))
            .on_action(cx.listener(Self::on_toggle_sql))
            .on_action(cx.listener(Self::on_mode_detail))
            .on_action(cx.listener(Self::on_mode_table))
            .on_action(cx.listener(Self::on_mode_raw))
            .on_action(cx.listener(Self::on_close_overlay))
            .on_action(cx.listener(Self::on_graph_zoom_in))
            .on_action(cx.listener(Self::on_graph_zoom_out))
            .on_action(cx.listener(Self::on_graph_fit))
            .child(self.render_titlebar(cx))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .flex_row()
                    .when(self.settings.show_toolbar, |el| {
                        el.child(self.render_left_rail(cx))
                    })
                    .when(
                        !self.show_landing && self.settings.show_explorer && self.show_sidebar,
                        |el| {
                            el.child(self.render_explorer_sidebar(cx))
                                .child(self.render_sidebar_sash(cx))
                        },
                    )
                    .child(if self.show_landing {
                        self.render_landing_page(cx).into_any_element()
                    } else {
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .child(self.render_tab_strip(cx))
                            .child(self.render_workspace(hide_output_pane, is_horizontal_split, cx))
                            .into_any_element()
                    })
                    .when(
                        !self.show_landing && self.settings.ai_enabled && self.show_ai_sidebar,
                        |el| {
                            el.child(self.render_ai_sidebar_sash(cx))
                                .child(self.render_ai_right_sidebar(cx))
                        },
                    ),
            )
            .when(!self.show_landing || self.active_target.is_ssh(), |el| {
                el.child(self.render_status_footer(cx))
            })
            .when(self.sash_drag != SashDragState::None, |el| {
                let drag_cursor = match self.sash_drag {
                    SashDragState::StackHorizontal { .. } => gpui::CursorStyle::ResizeUpDown,
                    _ => gpui::CursorStyle::ResizeLeftRight,
                };
                el.child(
                    div()
                        .id("sash-drag-capture-overlay")
                        .occlude()
                        .absolute()
                        .inset_0()
                        .cursor(drag_cursor)
                        .on_mouse_move(cx.listener(|this, ev, window, cx| {
                            this.update_sash_drag(ev, window, cx);
                        }))
                        .on_mouse_up(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.finish_sash_drag(cx);
                            }),
                        )
                        .on_mouse_up_out(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.finish_sash_drag(cx);
                            }),
                        ),
                )
            })
            .when_some(
                self.hovered_rail_tooltip.filter(|_| self.settings.show_toolbar),
                |el, (idx, tip)| {
                    let badge = div()
                        .absolute()
                        .left(px(52.0))
                        .px_2p5()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(0x1f2430))
                        .border_1()
                        .border_color(rgb(0x334155))
                        .shadow_md()
                        .text_xs()
                        .text_color(rgb(0xe2e8f0))
                        .child(tip);
                    if idx == 99 {
                        el.child(badge.bottom(px(40.0)))
                    } else {
                        el.child(badge.top(px(52.0 + (idx as f32) * 44.0)))
                    }
                },
            )
            .when_some(self.tab_context_menu, |el, (idx, pos)| {
                el.child(self.render_tab_context_menu(idx, pos, window, cx))
            })
            .when(self.overlay == OverlayKind::CommandPalette, |el| {
                el.child(self.render_palette_modal(cx))
            })
            .when(self.overlay == OverlayKind::LogViewer, |el| {
                el.child(theme::scoped(theme::Scope::Fixed, || {
                    self.render_logs_modal(cx).into_any_element()
                }))
            })
            .when(self.overlay == OverlayKind::TablePreview, |el| {
                el.child(self.render_table_preview_modal(cx))
            })
            .when(self.overlay == OverlayKind::ObjectGraph, |el| {
                el.child(self.render_object_graph_modal(cx))
            })
            .when(self.overlay == OverlayKind::HtmlPreview, |el| {
                el.child(theme::scoped(theme::Scope::Fixed, || {
                    self.render_html_preview_modal(cx).into_any_element()
                }))
            })
            .when(self.overlay == OverlayKind::Settings, |el| {
                el.child(theme::scoped(theme::Scope::DarkAuthored, || {
                    self.render_settings_modal(cx).into_any_element()
                }))
            })
            .when(self.overlay == OverlayKind::SshConnections, |el| {
                el.child(theme::scoped(theme::Scope::DarkAuthored, || {
                    self.render_ssh_modal(cx).into_any_element()
                }))
            })
    }
}

fn dark_close_button(
    id: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .size(px(22.0))
        .rounded_sm()
        .bg(rgb(0x283040))
        .border_1()
        .border_color(rgb(0x3f4b63))
        .hover(|s| s.bg(rgb(0x333e54)))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path("icons/close.svg")
                .size(px(13.0))
                .text_color(rgb(0xe2e8f0)),
        )
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx);
        })
}

fn light_close_button(
    id: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .size(px(24.0))
        .rounded_md()
        .bg(rgb(BG_CARD))
        .border_1()
        .border_color(rgb(BORDER_CARD))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path("icons/close.svg")
                .size(px(13.5))
                .text_color(rgb(TEXT_MAIN)),
        )
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx);
        })
}

fn dark_graph_button(
    id: &str,
    label: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .px_2()
        .py_0p5()
        .rounded_sm()
        .bg(rgb(0x283040))
        .border_1()
        .border_color(rgb(0x3f4b63))
        .hover(|s| s.bg(rgb(0x333e54)))
        .cursor_pointer()
        .text_xs()
        .text_color(rgb(0xe2e8f0))
        .child(label.to_string())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx);
        })
}

/// For list-like nodes (collections, arrays) whose children are keyed `0..n`,
/// return the number of items so the header can say "3 items".
fn list_item_count(node: &ObjectNode) -> Option<usize> {
    // The PHP serializer reports the true size as `Collection:N` (children are capped at 100).
    if let Some(n) = node
        .type_name
        .strip_prefix("Collection:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        return Some(n);
    }
    let items: Vec<&ObjectNode> = node
        .children
        .iter()
        .filter(|c| c.visibility.as_deref() != Some("graph_rel"))
        .collect();
    let is_list = !items.is_empty() && items.iter().all(|c| c.label.parse::<usize>().is_ok());
    let looks_like_collection = node
        .class_name
        .as_deref()
        .is_some_and(|c| c.ends_with("Collection") || c.ends_with("Paginator"));
    if is_list || (looks_like_collection && items.is_empty()) {
        Some(items.len())
    } else {
        None
    }
}

fn is_compact_table_column(col_name: &str, rows: &[Vec<String>], col_idx: usize) -> bool {
    let lower = col_name.to_lowercase();
    if lower == "id"
        || lower.ends_with("_id")
        || matches!(
            lower.as_str(),
            "active" | "published" | "views" | "mrr" | "total" | "country" | "plan" | "status" | "currency"
        )
    {
        return true;
    }
    if rows.is_empty() {
        return false;
    }
    rows.iter()
        .take(20)
        .all(|r| r.get(col_idx).map(|s| s.len() <= 8).unwrap_or(true))
}

fn landing_primary_tile(
    id: &str,
    icon_path: &'static str,
    accent_hex: u32,
    accent_bg_hex: u32,
    title: &str,
    subtitle: &str,
    shortcut: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .flex_1()
        .min_w(px(0.0))
        .px_4()
        .py_3p5()
        .rounded_xl()
        .bg(rgb(0xffffff))
        .border_1()
        .border_color(rgb(0xe2e8f0))
        .shadow_xs()
        .hover(move |s| s.border_color(rgb(accent_hex)).shadow_md())
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap_3p5()
        .on_click(on_click)
        .child(
            div()
                .size(px(40.0))
                .flex_none()
                .rounded_lg()
                .bg(rgb(accent_bg_hex))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(icon_path)
                        .size(px(19.0))
                        .text_color(rgb(accent_hex)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0x0f172a))
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(0x64748b))
                        .child(subtitle.to_string()),
                ),
        )
        .child(div().flex_none().child(landing_keycap(shortcut)))
        .into_any_element()
}

fn landing_keycap(keys: &str) -> gpui::AnyElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded_md()
        .bg(rgb(0xf8fafc))
        .border_1()
        .border_color(rgb(0xe2e8f0))
        .border_b_2()
        .font_family("Menlo")
        .text_xs()
        .text_color(rgb(0x64748b))
        .child(keys.to_string())
        .into_any_element()
}

fn landing_card_header(
    icon_path: &'static str,
    title: &str,
    count: usize,
    action_id: &str,
    action_label: &str,
    on_action: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    div()
        .px_4()
        .pt_3()
        .pb_2()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    svg()
                        .path(icon_path)
                        .size(px(13.0))
                        .text_color(rgb(0x94a3b8)),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0x475569))
                        .child(title.to_uppercase()),
                )
                .when(count > 0, |el| {
                    el.child(
                        div()
                            .px_1p5()
                            .rounded_full()
                            .bg(rgb(0xf1f5f9))
                            .text_xs()
                            .text_color(rgb(0x64748b))
                            .child(count.to_string()),
                    )
                }),
        )
        .child(
            div()
                .id(SharedString::from(action_id.to_string()))
                .px_2()
                .py_0p5()
                .rounded_md()
                .hover(|s| s.bg(rgb(0xeff6ff)))
                .cursor_pointer()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(0x2563eb))
                .on_click(on_action)
                .child(action_label.to_string()),
        )
        .into_any_element()
}

fn landing_tag(label: &str, bg_hex: u32, fg_hex: u32) -> gpui::AnyElement {
    div()
        .flex_none()
        .px_1p5()
        .rounded_sm()
        .bg(rgb(bg_hex))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgb(fg_hex))
        .child(label.to_string())
        .into_any_element()
}

fn landing_empty_state(icon_path: &'static str, title: &str, hint: &str) -> gpui::AnyElement {
    div()
        .px_4()
        .py_8()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(36.0))
                .rounded_full()
                .bg(rgb(0xf1f5f9))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(icon_path)
                        .size(px(16.0))
                        .text_color(rgb(0x94a3b8)),
                ),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(0x475569))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child(hint.to_string()),
        )
        .into_any_element()
}

fn landing_shortcut_chip(keys: &str, label: &str) -> gpui::AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(landing_keycap(keys))
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child(label.to_string()),
        )
        .into_any_element()
}

fn abbreviate_home_path(path: &str) -> String {
    if let Some(home) = crate::paths::home_dir_string()
        && let Some(rest) = path.strip_prefix(&home)
    {
        if rest.is_empty() {
            return "~".to_string();
        }
        return format!("~{rest}");
    }
    path.to_string()
}

fn extract_short_php_badge(detected_line: &str) -> String {
    // e.g. "Detected version: PHP 8.3.36 (cli) ..." -> "PHP 8.3.36"
    if let Some(after) = detected_line.strip_prefix("Detected version: ") {
        let parts: Vec<&str> = after.split_whitespace().collect();
        if parts.len() >= 2 && parts[0].eq_ignore_ascii_case("PHP") {
            return format!("PHP {}", parts[1]);
        }
        return after.to_string();
    }
    "PHP CLI".to_string()
}

fn recent_folders_file_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join(".laralah_recent.json")
}

fn load_recent_folders() -> Vec<String> {
    let path = recent_folders_file_path();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) else {
        return Vec::new();
    };
    list.into_iter()
        .filter(|p| std::path::Path::new(p).is_dir())
        .collect()
}

fn save_recent_folders(folders: &[String]) {
    let capped: Vec<&String> = folders.iter().take(12).collect();
    if let Ok(json) = serde_json::to_string_pretty(&capped) {
        let _ = std::fs::write(recent_folders_file_path(), json);
    }
}

fn starter_code_for_project(path: &str) -> String {
    let root = std::path::Path::new(path);
    if root.join("app").join("Models").join("User.php").is_file() {
        return "use App\\Models\\User;\n\nUser::query()->limit(5)->get();".to_string();
    }
    if root.join("artisan").is_file() {
        return "app()->version();".to_string();
    }
    "phpversion();".to_string()
}

fn macos_menu_item(
    id: &str,
    checked: bool,
    label: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .px_2p5()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .text_xs()
        .text_color(rgb(0x111827))
        .hover(|s| s.bg(rgb(0x0a84ff)).text_color(fixed_rgb(0xffffff)))
        .on_click(on_click)
        .child(
            div()
                .w(px(12.0))
                .font_weight(gpui::FontWeight::BOLD)
                .child(if checked { "✓" } else { "" }),
        )
        .child(div().flex_1().truncate().child(label.to_string()))
        .into_any_element()
}

fn rail_icon_button(
    id: &str,
    icon_path: &'static str,
    active: bool,
    color_hex: u32,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .relative()
        .size(px(34.0))
        .rounded_lg()
        .bg(if active {
            rgb(0xeff6ff)
        } else {
            rgba(0x00000000)
        })
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .on_hover(on_hover)
        .on_click(on_click)
        .when(active, |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(-6.0))
                    .w(px(3.0))
                    .h(px(18.0))
                    .rounded_r_sm()
                    .bg(rgb(color_hex)),
            )
        })
        .child(
            svg()
                .path(icon_path)
                .size(px(17.5))
                .text_color(rgb(color_hex)),
        )
}

fn card_action_pill(
    id: &str,
    icon_path: &'static str,
    label: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .px_2()
        .py_0p5()
        .rounded_md()
        .bg(rgb(0xf8fafc))
        .border_1()
        .border_color(rgb(0xe2e8f0))
        .hover(|s| s.bg(rgb(0xeef2f6)).border_color(rgb(0xcbd5e1)))
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .on_click(on_click)
        .child(
            svg()
                .path(icon_path)
                .size(px(13.0))
                .text_color(rgb(0x475569)),
        )
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(0x475569))
                .child(label.to_string()),
        )
        .into_any_element()
}

fn light_button(
    id: &str,
    label: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .px_2p5()
        .py_1()
        .rounded_md()
        .bg(rgb(BG_CARD))
        .border_1()
        .border_color(rgb(BORDER_CARD))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .cursor_pointer()
        .text_xs()
        .text_color(rgb(TEXT_MAIN))
        .child(label.to_string())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx);
        })
}

fn collect_default_expanded(node: &ObjectNode, set: &mut HashSet<String>) {
    if node.expanded_by_default {
        set.insert(node.id.clone());
    }
    for ch in &node.children {
        collect_default_expanded(ch, set);
    }
}

fn collect_all_expanded(node: &ObjectNode, set: &mut HashSet<String>) {
    if !node.children.is_empty() {
        set.insert(node.id.clone());
    }
    for ch in &node.children {
        collect_all_expanded(ch, set);
    }
}

fn format_setting_float(val: f32) -> String {
    let rounded = (val * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < 0.005 {
        format!("{}", rounded.round() as i32)
    } else {
        format!("{rounded}")
    }
}

fn format_footer_metrics(duration_ms: f64, memory_mb: f64) -> String {
    let dur = if duration_ms >= 1000.0 {
        let whole = duration_ms.round() as u64;
        format!("{}ms", format_with_commas(whole))
    } else if duration_ms >= 10.0 {
        format!("{:.0}ms", duration_ms)
    } else {
        format!("{:.1}ms", duration_ms)
    };

    let mem_kb = memory_mb * 1024.0;
    let mem = if mem_kb < 1024.0 {
        format!("{:.1}KB", mem_kb)
    } else {
        format!("{:.2}MB", memory_mb)
    };
    format!("{dur} / {mem}")
}

fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (idx, ch) in s.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn ssh_field_label(label: &str) -> gpui::AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0xe5e7eb))
        .child(label.to_string())
        .into_any_element()
}

fn prepend_buffer_use_statements(full_text: &str, selected: &str) -> String {
    let mut imports = Vec::new();
    for line in full_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("use ") && trimmed.ends_with(';') && !selected.contains(trimmed) {
            imports.push(trimmed);
        }
    }
    if imports.is_empty() {
        selected.to_string()
    } else {
        format!("{}\n\n{}", imports.join("\n"), selected)
    }
}

fn settings_section_header(title: &str) -> gpui::AnyElement {
    div()
        .w_full()
        .px_1()
        .text_sm()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(0xf1f5f9))
        .child(title.to_string())
        .into_any_element()
}

fn settings_breadcrumb_header(
    tab_label: &str,
    section_label: &str,
    jump_id: &str,
    jump_label: &str,
    on_jump: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let show_sub_section = !section_label.eq_ignore_ascii_case(tab_label);
    div()
        .w_full()
        .px_1()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .bg(rgb(0x263042))
                        .border_1()
                        .border_color(rgb(0x38465e))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(0x93c5fd))
                        .child(tab_label.to_string()),
                )
                .when(show_sub_section, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x64748b))
                            .child("›"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xf1f5f9))
                            .child(section_label.to_string()),
                    )
                })
                .when(!show_sub_section, |el| {
                    el.child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xf1f5f9))
                            .child(section_label.to_string()),
                    )
                }),
        )
        .child(
            div()
                .id(SharedString::from(jump_id.to_string()))
                .px_2p5()
                .py_1()
                .rounded_md()
                .hover(|s| s.bg(rgb(0x252c39)))
                .cursor_pointer()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .on_click(on_jump)
                .child(jump_label.to_string()),
        )
        .into_any_element()
}

fn settings_section_card(
    header: gpui::AnyElement,
    rows: Vec<gpui::AnyElement>,
) -> gpui::AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2p5()
        .child(header)
        .child(
            div()
                .w_full()
                .rounded_xl()
                .bg(rgb(0x232833))
                .border_1()
                .border_color(rgb(0x313847))
                .overflow_hidden()
                .flex()
                .flex_col()
                .children(rows.into_iter().enumerate().map(|(idx, row)| {
                    div()
                        .w_full()
                        .when(idx > 0, |r| r.border_t_1().border_color(rgb(0x2c3342)))
                        .child(row)
                })),
        )
        .into_any_element()
}

fn render_shortcuts_card(rows: &[(&'static str, &'static str)]) -> gpui::AnyElement {
    div()
        .w_full()
        .rounded_xl()
        .bg(rgb(0x232833))
        .border_1()
        .border_color(rgb(0x313847))
        .overflow_hidden()
        .flex()
        .flex_col()
        .children(rows.iter().enumerate().map(|(idx, (action_name, keys))| {
            div()
                .w_full()
                .px_5()
                .py_3()
                .when(idx > 0, |el| el.border_t_1().border_color(rgb(0x2c3342)))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(0xf1f5f9))
                        .child(*action_name),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .children(keys.split(" / ").enumerate().flat_map(|(k_idx, combo)| {
                            let sep = (k_idx > 0).then(|| {
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x64748b))
                                    .child("or")
                                    .into_any_element()
                            });
                            sep.into_iter().chain(std::iter::once(settings_keycap(combo)))
                        })),
                )
        }))
        .into_any_element()
}

/// A single key-combination badge, e.g. `⌘R`.
fn settings_keycap(combo: &str) -> gpui::AnyElement {
    div()
        .min_w(px(28.0))
        .h(px(24.0))
        .px_2()
        .rounded_md()
        .bg(rgb(0x181c23))
        .border_1()
        .border_color(rgb(0x364052))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgb(0xe2e8f0))
        .child(combo.to_string())
        .into_any_element()
}

fn settings_row(
    label: &str,
    control: gpui::AnyElement,
    caption: Option<&str>,
) -> gpui::AnyElement {
    settings_row_rich(label, control, caption.map(settings_caption))
}

fn settings_row_rich(
    label: &str,
    control: gpui::AnyElement,
    caption: Option<gpui::AnyElement>,
) -> gpui::AnyElement {
    div()
        .w_full()
        .px_5()
        .py_3()
        .min_h(px(60.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_6()
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0xf1f5f9))
                        .child(label.to_string()),
                )
                .when_some(caption, |el, cap| el.child(cap)),
        )
        // Controls size themselves and hug the right edge; only path pickers
        // and long dropdowns stretch to the full column.
        .child(
            div()
                .w(px(300.0))
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .justify_end()
                .child(control),
        )
        .into_any_element()
}

/// Caption text where `backticked` spans render as inline key/code badges.
fn settings_caption(text: &str) -> gpui::AnyElement {
    let mut plain = String::with_capacity(text.len());
    let mut highlights = Vec::new();
    for (idx, part) in text.split('`').enumerate() {
        if idx % 2 == 1 && !part.is_empty() {
            let start = plain.len();
            plain.push('\u{2009}');
            plain.push_str(part);
            plain.push('\u{2009}');
            highlights.push((
                start..plain.len(),
                gpui::HighlightStyle {
                    color: Some(rgb(0xe2e8f0).into()),
                    background_color: Some(rgb(0x2e3646).into()),
                    font_weight: Some(gpui::FontWeight::MEDIUM),
                    ..Default::default()
                },
            ));
        } else {
            plain.push_str(part);
        }
    }
    div()
        .text_xs()
        .line_height(px(17.0))
        .text_color(rgb(0x94a3b8))
        .child(gpui::StyledText::new(plain).with_highlights(highlights))
        .into_any_element()
}

/// Detected PHP version line with a success / failure indicator.
fn php_status_caption(detected: &str) -> gpui::AnyElement {
    let ok = detected.starts_with("Detected version:");
    let text = detected
        .strip_prefix("Detected version: ")
        .unwrap_or(detected)
        .to_string();
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap_1p5()
        .child(
            div()
                .flex_none()
                .text_xs()
                .line_height(px(17.0))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(if ok { 0x4ade80 } else { 0xfbbf24 }))
                .child(if ok { "✓" } else { "⚠" }),
        )
        .child(
            div()
                .flex_1()
                .text_xs()
                .line_height(px(17.0))
                .text_color(rgb(if ok { 0x94a3b8 } else { 0xfcd34d }))
                .child(text),
        )
        .into_any_element()
}

/// Shortens a path for display: `~` for home, then elides the middle so the
/// file name at the end stays visible.
fn middle_truncate_path(path: &str, max_chars: usize) -> String {
    let path = abbreviate_home_path(path);
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max_chars || max_chars < 8 {
        return path;
    }
    let tail_len = (max_chars * 3) / 5;
    let head_len = max_chars - tail_len - 1;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    format!("{head}…{tail}")
}

/// Narrow numeric field with − / + steppers. Stepping rewrites the text field,
/// which flows through the existing input observer to validate and save.
fn settings_number_box(
    id: &str,
    field: Entity<TextField>,
    step: f32,
    min: f32,
    max: f32,
    decimals: usize,
) -> gpui::AnyElement {
    let stepper = |suffix: &str, glyph: &'static str, delta: f32| {
        let field = field.clone();
        div()
            .id(SharedString::from(format!("{id}-{suffix}")))
            .flex_none()
            .w(px(26.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_sm()
            .text_color(rgb(0x94a3b8))
            .hover(|s| s.bg(rgb(0x232a36)).text_color(rgb(0xf1f5f9)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                field.update(cx, |tf, cx| {
                    let cur = tf.text().trim().parse::<f32>().unwrap_or(min);
                    let next = (cur + delta).clamp(min, max);
                    tf.set_text(format!("{next:.decimals$}"), cx);
                });
            })
            .child(glyph)
    };
    let dec = stepper("dec", "−", -step);
    let inc = stepper("inc", "+", step);
    let field_click = field.clone();
    div()
        .w(px(124.0))
        .h(px(32.0))
        .rounded_lg()
        .bg(rgb(0x181c23))
        .border_1()
        .border_color(rgb(0x364052))
        .overflow_hidden()
        .flex()
        .flex_row()
        .items_center()
        .child(dec)
        .child(
            div()
                .flex_1()
                .h_full()
                .px_2()
                .border_l_1()
                .border_r_1()
                .border_color(rgb(0x2c3342))
                .overflow_hidden()
                .cursor(gpui::CursorStyle::IBeam)
                .on_mouse_down(gpui::MouseButton::Left, move |ev, window, cx| {
                    cx.stop_propagation();
                    field_click.update(cx, |tf, cx| {
                        tf.handle_mouse_down(ev, window, cx);
                    });
                })
                .flex()
                .items_center()
                .child(div().w_full().overflow_hidden().child(field)),
        )
        .child(inc)
        .into_any_element()
}

fn settings_text_box(field: Entity<TextField>) -> gpui::AnyElement {
    let field_click = field.clone();
    div()
        .w_full()
        .h(px(36.0))
        .px_3()
        .rounded_lg()
        .bg(rgb(0x181c23))
        .border_1()
        .border_color(rgb(0x364052))
        .overflow_hidden()
        .cursor(gpui::CursorStyle::IBeam)
        .on_mouse_down(gpui::MouseButton::Left, move |ev, window, cx| {
            cx.stop_propagation();
            field_click.update(cx, |tf, cx| {
                tf.handle_mouse_down(ev, window, cx);
            });
        })
        .flex()
        .flex_row()
        .items_center()
        .child(div().w_full().h_full().overflow_hidden().child(field))
        .into_any_element()
}

fn settings_action_btn(
    id: &str,
    label: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .flex_none()
        .px_2p5()
        .py_1()
        .rounded_md()
        .bg(rgb(0x262d3b))
        .border_1()
        .border_color(rgb(0x424e63))
        .hover(|s| s.bg(rgb(0x30394a)).border_color(rgb(0x55647f)))
        .cursor_pointer()
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgb(0xf1f5f9))
        .child(label.to_string())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx);
        })
        .into_any_element()
}

fn render_export_status_badge(message: &str, is_error: bool) -> impl IntoElement {
    let (bg_hex, border_hex, text_hex, prefix) = if is_error {
        (0xfef2f2, 0xfecaca, 0xb91c1c, "✕ ")
    } else {
        (0xecfdf5, 0xa7f3d0, 0x047857, "✓ ")
    };
    div()
        .flex_none()
        .px_2()
        .py_0p5()
        .rounded_md()
        .bg(rgb(bg_hex))
        .border_1()
        .border_color(rgb(border_hex))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgb(text_hex))
        .child(format!("{prefix}{message}"))
}

fn table_export_menu_item(
    id: &str,
    label: &str,
    hint: &str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id.to_string()))
        .px_2p5()
        .py_1p5()
        .rounded_md()
        .hover(|s| s.bg(rgb(0xf1f5f9)))
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(on_click)
        .child(
            div()
                .text_xs()
                .text_color(rgb(TEXT_MAIN))
                .child(label.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(TEXT_MUTED))
                .child(hint.to_string()),
        )
}

fn table_indices_to_csv(table: &TableDataset, indices: &[usize]) -> String {
    let mut lines = Vec::with_capacity(indices.len() + 1);
    lines.push(
        table
            .columns
            .iter()
            .map(|c| csv_escape(c))
            .collect::<Vec<_>>()
            .join(","),
    );
    for &idx in indices {
        if let Some(row) = table.rows.get(idx) {
            lines.push(
                row.iter()
                    .map(|c| csv_escape(c))
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
    }
    lines.join("\n")
}

fn table_indices_to_json(table: &TableDataset, indices: &[usize]) -> String {
    let mut items = Vec::with_capacity(indices.len());
    for &idx in indices {
        if let Some(raw) = table.raw_rows_json.get(idx) {
            items.push(raw.clone());
        } else if let Some(row) = table.rows.get(idx) {
            let mut map = serde_json::Map::new();
            for (cidx, col) in table.columns.iter().enumerate() {
                let cell = row.get(cidx).cloned().unwrap_or_default();
                map.insert(col.clone(), serde_json::Value::String(cell));
            }
            items.push(
                serde_json::to_string_pretty(&serde_json::Value::Object(map))
                    .unwrap_or_else(|_| "{}".to_string()),
            );
        }
    }
    if items.is_empty() {
        "[]".to_string()
    } else {
        format!("[\n{}\n]", items.join(",\n"))
    }
}

fn csv_escape(cell: &str) -> String {
    if cell.contains(',') || cell.contains('"') || cell.contains('\n') || cell.contains('\r') {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_string()
    }
}

/// Double-clicking empty titlebar space zooms or minimizes the window, following the
/// macOS "Double-click a window's title bar to" setting. The native handler is disabled
/// in `macos_window_cursor` so double-clicks on titlebar buttons don't resize the window.
/// On Windows the same areas are marked `WindowControlArea::Drag`, so the OS handles
/// drag and double-click maximize/restore itself; Linux uses server-side decorations.
fn titlebar_double_click(event: &gpui::MouseDownEvent, window: &mut Window, _: &mut App) {
    if event.click_count == 2 {
        window.titlebar_double_click();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_table() -> TableDataset {
        TableDataset {
            source_label: "Line 1".to_string(),
            columns: vec!["id".to_string(), "name".to_string(), "note".to_string()],
            rows: vec![
                vec!["1".to_string(), "Alice".to_string(), "first, row".to_string()],
                vec!["2".to_string(), "Bob".to_string(), "quote \"here\"".to_string()],
                vec!["3".to_string(), "Cara".to_string(), "plain".to_string()],
            ],
            raw_rows_json: vec![
                r#"{"id":1,"name":"Alice"}"#.to_string(),
                r#"{"id":2,"name":"Bob"}"#.to_string(),
                r#"{"id":3,"name":"Cara"}"#.to_string(),
            ],
            raw_rows_php: vec![],
        }
    }

    #[test]
    fn table_indices_to_csv_respects_filtered_order_and_escaping() {
        let tbl = sample_table();
        let csv = table_indices_to_csv(&tbl, &[1, 0]);
        assert_eq!(
            csv,
            "id,name,note\n2,Bob,\"quote \"\"here\"\"\"\n1,Alice,\"first, row\""
        );
    }

    #[test]
    fn table_indices_to_json_respects_filtered_order_and_empty_slice() {
        let tbl = sample_table();
        assert_eq!(table_indices_to_json(&tbl, &[]), "[]");
        let json = table_indices_to_json(&tbl, &[2, 0]);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json array");
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["name"], "Cara");
        assert_eq!(parsed[1]["name"], "Alice");
    }
}
