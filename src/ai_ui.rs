use super::*;
use crate::ai::discovery::{check_cli_provider, detect_providers_full, list_models_for_provider};
use crate::ai::harness::{AiHarnessJobInput, execute_ai_harness_job};
use crate::ai::{
    AiChatMessage, AiChatSession, AiMessageStatus, AiModel, AiToolStep, AiUiEvent,
    CliAgentRuntimeState, CliProviderId, format_effort_label,
};
use std::sync::atomic::{AtomicBool, Ordering};

struct AiTooltip {
    text: SharedString,
}

impl AiTooltip {
    fn view(text: impl Into<SharedString>, cx: &mut App) -> gpui::AnyView {
        let text = text.into();
        cx.new(|_| Self { text }).into()
    }
}

impl Render for AiTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let _scope = crate::theme::enter_scope(crate::theme::Scope::Fixed);
        div()
            .px_2p5()
            .py_1()
            .rounded_md()
            .bg(rgb(0x1f2430))
            .border_1()
            .border_color(rgb(0x334155))
            .shadow_md()
            .text_xs()
            .whitespace_nowrap()
            .text_color(rgb(0xe2e8f0))
            .child(self.text.clone())
    }
}

/// Drops the provider name from a model label inside that provider's own
/// panel ("Claude Opus 5.5" → "Opus 5.5"), so rows don't truncate.
fn picker_model_label(provider: CliProviderId, label: &str) -> String {
    let prefix = format!("{} ", provider.short_label());
    label
        .strip_prefix(prefix.as_str())
        .filter(|rest| !rest.trim().is_empty())
        .unwrap_or(label)
        .to_string()
}

/// "2.1.326 (Claude Code)" → "v2.1.326"; falls back to the raw string.
fn picker_version_label(raw: &str) -> String {
    raw.split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(|t| format!("v{t}"))
        .unwrap_or_else(|| raw.to_string())
}

fn picker_source_label(source: &str) -> &'static str {
    match source {
        "live" => "Up to date",
        "cached" => "Cached",
        _ => "Built-in list",
    }
}

/// ("opus", [5, 5]) for "claude-opus-5-5"; None for aliases and odd ids.
fn claude_family_version(id: &str) -> Option<(&str, Vec<u32>)> {
    let mut parts = id.strip_prefix("claude-")?.split('-');
    let family = parts.next()?;
    let version: Vec<u32> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    (!version.is_empty()).then_some((family, version))
}

/// Splits the Claude catalog into the newest model of each family, older
/// releases (newest first), and bare aliases such as `sonnet`.
fn group_claude_models(models: Vec<AiModel>) -> (Vec<AiModel>, Vec<AiModel>, Vec<AiModel>) {
    let mut newest: HashMap<String, Vec<u32>> = HashMap::new();
    for m in &models {
        if let Some((family, ver)) = claude_family_version(&m.id) {
            let entry = newest.entry(family.to_string()).or_default();
            if ver > *entry {
                *entry = ver;
            }
        }
    }
    let (mut latest, mut older, mut aliases) = (Vec::new(), Vec::new(), Vec::new());
    for m in models {
        match claude_family_version(&m.id) {
            Some((family, ver)) if newest.get(family) == Some(&ver) => latest.push(m),
            Some(_) => older.push(m),
            None if m.id.starts_with("claude-") => older.push(m),
            None => aliases.push(m),
        }
    }
    older.sort_by(|a, b| {
        let va = claude_family_version(&a.id).map(|(_, v)| v).unwrap_or_default();
        let vb = claude_family_version(&b.id).map(|(_, v)| v).unwrap_or_default();
        vb.cmp(&va).then_with(|| a.id.cmp(&b.id))
    });
    (latest, older, aliases)
}

fn picker_section_header(title: &'static str) -> gpui::AnyElement {
    div()
        .px_2p5()
        .pt_2()
        .pb_1()
        .text_size(px(10.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(TEXT_MUTED))
        .child(title.to_uppercase())
        .into_any_element()
}

pub(super) fn render_macos_switch(
    id: &str,
    on: bool,
    compact: bool,
    dark_surface: bool,
    on_toggle: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let (track_w, track_h, thumb_sz) = if compact {
        (30.0, 17.0, 13.0)
    } else {
        (38.0, 22.0, 18.0)
    };
    let off_bg = if dark_surface { 0x3b4456 } else { 0xcbd5e1 };
    let off_hover = if dark_surface { 0x475268 } else { 0x94a3b8 };

    div()
        .id(SharedString::from(id.to_string()))
        .w(px(track_w))
        .h(px(track_h))
        .p(px(2.0))
        .rounded_full()
        .flex_none()
        .cursor_pointer()
        .bg(rgb(if on { 0x2563eb } else { off_bg }))
        .hover(move |s| s.bg(rgb(if on { 0x1d4ed8 } else { off_hover })))
        .flex()
        .flex_row()
        .items_center()
        .when(on, |el| el.justify_end())
        .when(!on, |el| el.justify_start())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_toggle(ev, window, cx);
        })
        .child(
            div()
                .size(px(thumb_sz))
                .rounded_full()
                .bg(fixed_rgb(0xffffff))
                .shadow_xs(),
        )
        .into_any_element()
}

pub(super) fn is_safe_readonly_php(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    let mutating_tokens = [
        "->delete(",
        "::destroy(",
        "->forceDelete(",
        "->truncate(",
        "::truncate(",
        "->update(",
        "::update(",
        "->insert(",
        "::insert(",
        "::create(",
        "->create(",
        "->save(",
        "db::statement(",
        "db::unprepared(",
        "schema::drop",
        "artisan::call(",
        "unlink(",
        "file_put_contents(",
    ];
    !mutating_tokens.iter().any(|tok| lower.contains(tok))
}

/// Appends an AI-generated PHP snippet to the bottom of an existing editor buffer while
/// hoisting and deduplicating any top-level `use ...;` imports so PHP syntax stays valid.
pub(super) fn merge_and_append_php_snippet(existing: &str, incoming: &str) -> String {
    let existing_norm = existing.replace("\r\n", "\n").replace('\r', "\n");
    let incoming_norm = incoming.replace("\r\n", "\n").replace('\r', "\n");
    let incoming_trimmed = incoming_norm.trim();

    if incoming_trimmed.is_empty() {
        return existing_norm;
    }
    if existing_norm.trim().is_empty() {
        return incoming_trimmed.to_string();
    }

    let mut extracted_fqcns: Vec<String> = Vec::new();
    let mut alias_use_lines: Vec<String> = Vec::new();
    let mut body_lines: Vec<&str> = Vec::new();
    let mut in_header = true;

    for raw_line in incoming_trimmed.lines() {
        let t = raw_line.trim();
        if in_header {
            if t.starts_with("<?") || t.starts_with("declare(") {
                continue;
            }
            if t.is_empty() {
                continue;
            }
            if t.starts_with("use ") && t.ends_with(';') && !t.contains('(') && !t.contains('{') {
                let inner = t[4..t.len() - 1].trim();
                if inner.contains(" as ") || inner.starts_with("function ") || inner.starts_with("const ") {
                    alias_use_lines.push(format!("use {inner};"));
                } else {
                    for part in inner.split(',') {
                        let fqcn = part.trim().trim_start_matches('\\');
                        if !fqcn.is_empty() {
                            extracted_fqcns.push(fqcn.to_string());
                        }
                    }
                }
                continue;
            }
            in_header = false;
        }
        body_lines.push(raw_line);
    }

    let mut merged = existing_norm.trim_end().to_string();
    for fqcn in extracted_fqcns {
        let (next, _) = insert_use_import(&merged, merged.len(), &fqcn);
        merged = next;
    }
    for alias_stmt in alias_use_lines {
        if !merged.lines().any(|l| l.trim() == alias_stmt) {
            merged = format!("{alias_stmt}\n{merged}");
        }
    }

    let body = body_lines.join("\n");
    let body_trimmed = body.trim();
    if body_trimmed.is_empty() {
        merged
    } else {
        format!("{}\n\n{}", merged.trim_end(), body_trimmed)
    }
}

impl LaralahApp {
    pub(super) fn bootstrap_ai_discovery(&mut self, cx: &mut Context<Self>) {
        let tx = self.ai_ui_tx.clone();
        let initial_agent = self.active_ai_provider();
        if let Some(st) = self.ai_agents.get_mut(&initial_agent) {
            st.loading_models = true;
        }

        std::thread::spawn(move || {
            let statuses = detect_providers_full();
            let _ = tx.send(AiUiEvent::ProvidersDetected {
                statuses: statuses.clone(),
            });

            // Probe version & model list for the active provider if available (or Claude catalog)
            let is_avail = statuses
                .iter()
                .find(|(id, _)| *id == initial_agent)
                .map(|(_, ok)| *ok)
                .unwrap_or(false);

            if is_avail {
                let (avail, conn, ver, msg) = check_cli_provider(initial_agent);
                let _ = tx.send(AiUiEvent::CliChecked {
                    provider: initial_agent,
                    available: avail,
                    connected: conn,
                    version: ver,
                    message: msg,
                });
            }

            if is_avail || initial_agent == CliProviderId::ClaudeCode {
                let res = list_models_for_provider(initial_agent, false);
                let _ = tx.send(AiUiEvent::ModelsLoaded {
                    provider: initial_agent,
                    result: res,
                });
            } else {
                let _ = tx.send(AiUiEvent::ModelsLoaded {
                    provider: initial_agent,
                    result: Err(format!(
                        "{} CLI not installed.",
                        initial_agent.short_label()
                    )),
                });
            }
        });
        cx.notify();
    }

    pub(super) fn spawn_ai_event_listener(
        &self,
        rx: std::sync::mpsc::Receiver<AiUiEvent>,
        cx: &mut Context<Self>,
    ) {
        let rx_arc = Arc::new(Mutex::new(rx));
        cx.spawn(async move |this, cx| {
            loop {
                let rx_clone = Arc::clone(&rx_arc);
                let next_evt = cx
                    .background_executor()
                    .spawn(async move {
                        let guard = rx_clone.lock().ok()?;
                        guard.recv().ok()
                    })
                    .await;

                let Some(evt) = next_evt else {
                    break;
                };

                let alive = this
                    .update(cx, |app, cx| {
                        app.handle_ai_ui_event(evt, cx);
                    })
                    .is_ok();

                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_ai_ui_event(&mut self, evt: AiUiEvent, cx: &mut Context<Self>) {
        match evt {
            AiUiEvent::ProvidersDetected { statuses } => {
                for (id, avail) in statuses {
                    let entry = self
                        .ai_agents
                        .entry(id)
                        .or_insert_with(|| CliAgentRuntimeState::new(avail));
                    entry.available = avail;
                }
                // If current selected agent is neither installed nor explicitly chosen with models,
                // prefer an enabled & installed agent on first boot.
                let cur = self.active_ai_provider();
                let cur_avail = self.ai_agents.get(&cur).is_some_and(|s| s.available);
                if !cur_avail
                    && !self.settings.ai_selected_models.contains_key(cur.as_str())
                    && let Some(installed) = self
                        .enabled_ai_providers()
                        .into_iter()
                        .find(|p| self.ai_agents.get(p).is_some_and(|s| s.available))
                {
                    self.settings.ai_selected_agent = installed.as_str().to_string();
                    self.ensure_agent_models_loaded(installed, false, cx);
                }
                cx.notify();
            }
            AiUiEvent::CliChecked {
                provider,
                available,
                connected,
                version,
                message,
            } => {
                if let Some(st) = self.ai_agents.get_mut(&provider) {
                    st.checking = false;
                    st.available = available;
                    st.connected = Some(connected);
                    if version.is_some() {
                        st.version = version;
                    }
                    st.check_message = Some(message);
                }
                cx.notify();
            }
            AiUiEvent::ModelsLoaded { provider, result } => {
                if let Some(st) = self.ai_agents.get_mut(&provider) {
                    st.loading_models = false;
                    match result {
                        Ok(list) => {
                            if !self
                                .settings
                                .ai_selected_models
                                .contains_key(provider.as_str())
                                && let Some(def) = list
                                    .models
                                    .iter()
                                    .find(|m| m.is_default)
                                    .or_else(|| list.models.first())
                            {
                                self.settings
                                    .ai_selected_models
                                    .insert(provider.as_str().to_string(), def.id.clone());
                                save_settings(&self.settings);
                            }
                            st.model_list = Some(list);
                            st.model_error = None;
                        }
                        Err(err) => {
                            st.model_error = Some(err);
                        }
                    }
                }
                cx.notify();
            }
            AiUiEvent::TurnToolStep { msg_id, step } => {
                if let Some(msg) = self.find_ai_message_mut(msg_id) {
                    if let Some(existing) = msg.tool_steps.iter_mut().find(|s| s.id == step.id) {
                        *existing = step;
                    } else {
                        msg.tool_steps.push(step);
                    }
                    cx.notify();
                }
            }
            AiUiEvent::TurnStreamDelta { msg_id, full_text } => {
                if let Some(msg) = self.find_ai_message_mut(msg_id) {
                    msg.streaming_text = full_text;
                    cx.notify();
                }
            }
            AiUiEvent::TurnResetStream { msg_id } => {
                if let Some(msg) = self.find_ai_message_mut(msg_id) {
                    msg.streaming_text.clear();
                    cx.notify();
                }
            }
            AiUiEvent::TurnCompleted {
                msg_id,
                explanation,
                php_code,
                lint_passed,
                project_key,
                cached_models,
                cached_tables,
            } => {
                self.ai_active_cancel = None;
                if !cached_models.is_empty() {
                    let map = self.ai_schema_cache.entry(project_key.clone()).or_default();
                    for m in cached_models {
                        map.insert(m.fqcn.clone(), m);
                    }
                }
                if let Some(tables) = cached_tables {
                    self.ai_tables_cache.insert(project_key, tables);
                }

                let auto_write = self.settings.ai_auto_write_editor;
                let mut wrote = false;
                if auto_write && let Some(ref code) = php_code {
                    self.append_ai_code_to_editor(code, false, cx);
                    wrote = true;
                }

                if let Some(msg) = self.find_ai_message_mut(msg_id) {
                    msg.streaming_text.clear();
                    msg.explanation = explanation;
                    msg.php_code = php_code;
                    msg.lint_passed = lint_passed;
                    msg.written_to_editor = wrote;
                    msg.status = AiMessageStatus::Done;
                }
                cx.notify();
            }
            AiUiEvent::TurnFailed { msg_id, error } => {
                self.ai_active_cancel = None;
                if let Some(msg) = self.find_ai_message_mut(msg_id) {
                    msg.status = AiMessageStatus::Failed(error);
                }
                cx.notify();
            }
            AiUiEvent::TurnCancelled { msg_id } => {
                self.ai_active_cancel = None;
                if let Some(msg) = self.find_ai_message_mut(msg_id) {
                    msg.status = AiMessageStatus::Cancelled;
                }
                cx.notify();
            }
        }
    }

    pub(super) fn find_ai_message_mut(&mut self, msg_id: u64) -> Option<&mut AiChatMessage> {
        for session in &mut self.ai_sessions {
            if let Some(msg) = session.messages.iter_mut().find(|m| m.id == msg_id) {
                return Some(msg);
            }
        }
        None
    }

    pub(super) fn active_ai_messages(&self) -> &[AiChatMessage] {
        self.ai_sessions
            .get(self.ai_active_session_idx)
            .map(|s| s.messages.as_slice())
            .unwrap_or(&[])
    }

    pub(super) fn open_new_ai_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_active_ai_run(cx);
        let sid = self.ai_next_session_id;
        self.ai_next_session_id += 1;
        self.ai_sessions.push(AiChatSession::new(sid));
        self.ai_active_session_idx = self.ai_sessions.len() - 1;
        self.ai_picker_open = false;
        self.ai_effort_menu_open = false;
        self.ai_sessions_menu_open = false;
        let fh = self.ai_prompt_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    pub(super) fn clear_active_ai_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_active_ai_run(cx);
        if let Some(session) = self.ai_sessions.get_mut(self.ai_active_session_idx) {
            session.messages.clear();
            session.title = format!("Session {}", session.id);
        }
        self.ai_picker_open = false;
        self.ai_effort_menu_open = false;
        self.ai_sessions_menu_open = false;
        let fh = self.ai_prompt_input.focus_handle(cx);
        window.focus(&fh);
        cx.notify();
    }

    pub(super) fn close_ai_session_at(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.ai_sessions.len() {
            return;
        }
        if self.ai_sessions.len() <= 1 {
            self.cancel_active_ai_run(cx);
            self.ai_sessions[0].messages.clear();
            self.ai_sessions[0].title = format!("Session {}", self.ai_sessions[0].id);
            self.ai_sessions_menu_open = false;
            cx.notify();
            return;
        }
        if idx == self.ai_active_session_idx {
            self.cancel_active_ai_run(cx);
        }
        self.ai_sessions.remove(idx);
        if self.ai_active_session_idx >= self.ai_sessions.len() {
            self.ai_active_session_idx = self.ai_sessions.len() - 1;
        } else if idx < self.ai_active_session_idx {
            self.ai_active_session_idx -= 1;
        }
        cx.notify();
    }

    pub(super) fn enabled_ai_providers(&self) -> Vec<CliProviderId> {
        let list: Vec<CliProviderId> = CliProviderId::all()
            .into_iter()
            .filter(|id| {
                self.settings
                    .ai_enabled_agents
                    .get(id.as_str())
                    .copied()
                    .unwrap_or(true)
            })
            .collect();
        if list.is_empty() {
            vec![CliProviderId::ClaudeCode]
        } else {
            list
        }
    }

    pub(super) fn active_ai_provider(&self) -> CliProviderId {
        let enabled = self.enabled_ai_providers();
        if let Some(parsed) = CliProviderId::parse(&self.settings.ai_selected_agent)
            && enabled.contains(&parsed)
        {
            return parsed;
        }
        enabled
            .into_iter()
            .next()
            .unwrap_or(CliProviderId::ClaudeCode)
    }

    pub(super) fn active_ai_model_info(&self, provider: CliProviderId) -> (String, String, Vec<String>) {
        let saved_id = self
            .settings
            .ai_selected_models
            .get(provider.as_str())
            .cloned();

        if let Some(st) = self.ai_agents.get(&provider)
            && let Some(ref list) = st.model_list
        {
            if let Some(ref sid) = saved_id {
                if let Some(found) = list.models.iter().find(|m| &m.id == sid) {
                    return (
                        found.id.clone(),
                        found.label.clone(),
                        found.reasoning_efforts.clone(),
                    );
                }
                return (sid.clone(), sid.clone(), Vec::new());
            }
            if let Some(def) = list
                .models
                .iter()
                .find(|m| m.is_default)
                .or_else(|| list.models.first())
            {
                return (
                    def.id.clone(),
                    def.label.clone(),
                    def.reasoning_efforts.clone(),
                );
            }
        }

        if let Some(sid) = saved_id {
            return (sid.clone(), sid, Vec::new());
        }

        match provider {
            CliProviderId::ClaudeCode => (
                "claude-haiku-5-5".to_string(),
                "Claude Haiku 5.5".to_string(),
                vec![
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "xhigh".into(),
                    "max".into(),
                ],
            ),
            CliProviderId::Codex => (
                "gpt-5.6-sol".to_string(),
                "GPT-5.6 Sol".to_string(),
                vec!["low".into(), "medium".into(), "high".into()],
            ),
            CliProviderId::Agy => (
                "gemini-4-pro".to_string(),
                "Gemini 4 Pro".to_string(),
                vec!["low".into(), "medium".into(), "high".into()],
            ),
            CliProviderId::Qoder => ("auto".to_string(), "auto".to_string(), Vec::new()),
            CliProviderId::Opencode => (
                "anthropic/claude-sonnet-4-6".to_string(),
                "Claude Sonnet".to_string(),
                Vec::new(),
            ),
        }
    }

    pub(super) fn trigger_cli_check(&mut self, provider: CliProviderId, cx: &mut Context<Self>) {
        if let Some(st) = self.ai_agents.get_mut(&provider) {
            if st.checking {
                return;
            }
            st.checking = true;
            st.check_message = Some("Checking `--version`…".to_string());
        }
        let tx = self.ai_ui_tx.clone();
        std::thread::spawn(move || {
            let (available, connected, version, message) = check_cli_provider(provider);
            let _ = tx.send(AiUiEvent::CliChecked {
                provider,
                available,
                connected,
                version,
                message,
            });
            if available {
                let res = list_models_for_provider(provider, false);
                let _ = tx.send(AiUiEvent::ModelsLoaded {
                    provider,
                    result: res,
                });
            }
        });
        cx.notify();
    }

    pub(super) fn ensure_agent_models_loaded(
        &mut self,
        provider: CliProviderId,
        refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let should_probe_version = self
            .ai_agents
            .get(&provider)
            .is_some_and(|s| s.available && s.version.is_none() && !s.checking);

        if let Some(st) = self.ai_agents.get_mut(&provider) {
            if st.loading_models {
                return;
            }
            if !refresh && st.model_list.is_some() {
                return;
            }
            st.loading_models = true;
            st.model_error = None;
            if should_probe_version {
                st.checking = true;
            }
        }

        let tx = self.ai_ui_tx.clone();
        std::thread::spawn(move || {
            if should_probe_version {
                let (avail, conn, ver, msg) = check_cli_provider(provider);
                let _ = tx.send(AiUiEvent::CliChecked {
                    provider,
                    available: avail,
                    connected: conn,
                    version: ver,
                    message: msg,
                });
            }
            let res = list_models_for_provider(provider, refresh);
            let _ = tx.send(AiUiEvent::ModelsLoaded {
                provider,
                result: res,
            });
        });
        cx.notify();
    }

    pub(super) fn on_toggle_ai_sidebar(
        &mut self,
        _: &ToggleAiSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.settings.ai_enabled {
            return;
        }
        self.show_ai_sidebar = !self.show_ai_sidebar;
        self.settings.show_ai_sidebar = self.show_ai_sidebar;
        if !self.show_ai_sidebar {
            self.ai_picker_open = false;
            self.ai_effort_menu_open = false;
            self.ai_sessions_menu_open = false;
        }
        save_settings(&self.settings);

        if self.show_ai_sidebar && !self.show_landing {
            let provider = self.active_ai_provider();
            self.ensure_agent_models_loaded(provider, false, cx);
            let fh = self.ai_prompt_input.focus_handle(cx);
            window.focus(&fh);
        }
        cx.notify();
    }

    pub(super) fn on_submit_ai_prompt(
        &mut self,
        _: &SubmitAiPrompt,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_ai_prompt(cx);
    }

    pub(super) fn cancel_active_ai_run(&mut self, cx: &mut Context<Self>) {
        if let Some(ref cancel) = self.ai_active_cancel {
            cancel.store(true, Ordering::SeqCst);
        }
        cx.notify();
    }

    pub(super) fn submit_ai_prompt(&mut self, cx: &mut Context<Self>) {
        if self.ai_active_cancel.is_some() {
            return;
        }
        let prompt = self.ai_prompt_input.read(cx).text().trim().to_string();
        if prompt.is_empty() {
            return;
        }

        self.ai_prompt_input.update(cx, |tf, cx| {
            tf.set_text("", cx);
        });
        self.ai_picker_open = false;
        self.ai_effort_menu_open = false;
        self.ai_sessions_menu_open = false;

        let provider = self.active_ai_provider();
        let (model_id, model_label, supported_efforts) = self.active_ai_model_info(provider);
        let saved_effort = self
            .settings
            .ai_selected_efforts
            .get(provider.as_str())
            .cloned()
            .filter(|e| !e.is_empty() && !e.eq_ignore_ascii_case("default"))
            .filter(|e| {
                supported_efforts.is_empty()
                    || supported_efforts
                        .iter()
                        .any(|sup| sup.eq_ignore_ascii_case(e))
            });

        let msg_id = self.ai_next_msg_id;
        self.ai_next_msg_id += 1;

        if self.ai_sessions.is_empty() {
            let sid = self.ai_next_session_id;
            self.ai_next_session_id += 1;
            self.ai_sessions.push(AiChatSession::new(sid));
            self.ai_active_session_idx = 0;
        }
        if self.ai_active_session_idx >= self.ai_sessions.len() {
            self.ai_active_session_idx = self.ai_sessions.len() - 1;
        }

        let prior_turns: Vec<(String, String)> = self.ai_sessions[self.ai_active_session_idx]
            .messages
            .iter()
            .filter(|m| m.status == AiMessageStatus::Done)
            .map(|m| {
                let ans = match (&m.explanation.is_empty(), &m.php_code) {
                    (false, Some(code)) => format!("{}\n```php\n{code}\n```", m.explanation),
                    (true, Some(code)) => format!("```php\n{code}\n```"),
                    _ => m.explanation.clone(),
                };
                (m.user_prompt.clone(), ans)
            })
            .collect();

        let active_sess = &mut self.ai_sessions[self.ai_active_session_idx];
        if active_sess.messages.is_empty() {
            let short_title: String = prompt.chars().take(36).collect();
            active_sess.title = if prompt.chars().count() > 36 {
                format!("{}…", short_title.trim_end())
            } else {
                short_title
            };
        }
        active_sess.messages.push(AiChatMessage {
            id: msg_id,
            user_prompt: prompt.clone(),
            agent: provider,
            model_label: model_label.clone(),
            effort: saved_effort.clone(),
            tool_steps: Vec::new(),
            streaming_text: String::new(),
            explanation: String::new(),
            php_code: None,
            written_to_editor: false,
            lint_passed: true,
            status: AiMessageStatus::Running,
        });

        let editor_code = if self.settings.ai_include_editor_context {
            Some(self.editor.read(cx).text().to_string())
        } else {
            None
        };

        let last_exception = if self.settings.ai_include_editor_context {
            self.last_result
                .as_ref()
                .and_then(|r| r.exception.as_ref())
                .map(|ex| format!("{}: {} (line {})", ex.class, ex.message, ex.line))
        } else {
            None
        };

        let indexed_models: Vec<String> = self
            .explorer_models
            .iter()
            .map(|m| m.fqcn.clone())
            .collect();

        let known_schemas = self
            .ai_schema_cache
            .get(&self.active_project)
            .cloned()
            .unwrap_or_default();
        let known_tables = self.ai_tables_cache.get(&self.active_project).cloned();

        let cancel = Arc::new(AtomicBool::new(false));
        self.ai_active_cancel = Some(Arc::clone(&cancel));

        let job_input = AiHarnessJobInput {
            msg_id,
            provider,
            model: model_id,
            effort: saved_effort,
            user_prompt: prompt,
            prior_turns,
            project_path: self.active_project.clone(),
            target: self.active_target.clone(),
            php_binary: self.settings.php_binary.clone(),
            laravel_version: self.detected_laravel_version.clone(),
            php_version: extract_short_php_badge(&self.detected_php_version),
            indexed_models,
            editor_code,
            last_exception,
            allow_ssh_schema: self.settings.ai_allow_ssh_schema,
            known_schemas,
            known_tables,
        };

        let tx = self.ai_ui_tx.clone();
        std::thread::spawn(move || {
            execute_ai_harness_job(job_input, &cancel, &tx);
        });

        cx.notify();
    }

    pub(super) fn append_ai_code_to_editor(
        &mut self,
        code: &str,
        run_immediately: bool,
        cx: &mut Context<Self>,
    ) {
        self.show_landing = false;
        self.editor.update(cx, |ed, cx| {
            let merged = merge_and_append_php_snippet(ed.text(), code);
            ed.set_text_and_reveal(merged, cx);
            ed.apply_auto_imports(cx);
        });
        // Sync last_editor_version so debounced auto-run only runs when safe & intended
        self.last_editor_version = self.editor.read(cx).edit_version;
        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
            tab.code = self.editor.read(cx).text().to_string();
        }
        self.ai_editor_undo_banner = true;

        if run_immediately {
            self.run_evaluation(cx);
        } else if self.auto_run && !self.active_target.is_ssh() && is_safe_readonly_php(code) {
            self.schedule_auto_run(cx);
        }
        cx.notify();
    }

    pub(super) fn replace_ai_code_in_editor(
        &mut self,
        code: &str,
        run_immediately: bool,
        cx: &mut Context<Self>,
    ) {
        self.show_landing = false;
        self.editor.update(cx, |ed, cx| {
            ed.set_text(code, cx);
            ed.apply_auto_imports(cx);
        });
        self.last_editor_version = self.editor.read(cx).edit_version;
        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
            tab.code = self.editor.read(cx).text().to_string();
        }
        self.ai_editor_undo_banner = true;

        if run_immediately {
            self.run_evaluation(cx);
        } else if self.auto_run && !self.active_target.is_ssh() && is_safe_readonly_php(code) {
            self.schedule_auto_run(cx);
        }
        cx.notify();
    }

    // ==================== SETTINGS -> AI ASSISTANT RENDERING ====================

    pub(super) fn render_ai_agents_settings_card(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let enabled_count = self.enabled_ai_providers().len();

        div()
            .w_full()
            .px_5()
            .py_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    // The section header above already names this card, so
                    // lead with the explanation instead of repeating the title.
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .line_height(px(17.0))
                            .text_color(rgb(0x94a3b8))
                            .child("Choose which installed coding agents appear in the AI assistant picker."),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(rgb(0x181c23))
                            .border_1()
                            .border_color(rgb(0x2e3646))
                            .text_xs()
                            .text_color(rgb(0x93c5fd))
                            .child(format!("{enabled_count} active")),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .rounded_lg()
                    .bg(rgb(0x181c23))
                    .border_1()
                    .border_color(rgb(0x2e3646))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .children(CliProviderId::all().into_iter().enumerate().map(
                        |(idx, provider)| {
                            let is_on = self
                                .settings
                                .ai_enabled_agents
                                .get(provider.as_str())
                                .copied()
                                .unwrap_or(true);
                            let rt = self.ai_agents.get(&provider);
                            let available = rt.is_some_and(|s| s.available);
                            let connected = rt.and_then(|s| s.connected);
                            let checking = rt.is_some_and(|s| s.checking);
                            let status_badge = match (available, connected) {
                                (_, Some(true)) => "● Connected",
                                (true, Some(false)) => "⚠ Check failed",
                                (true, None) => "● Detected",
                                (false, _) => "○ Not found",
                            };
                            let subtitle = rt
                                .and_then(|s| s.check_message.clone())
                                .or_else(|| rt.and_then(|s| s.version.clone()))
                                .unwrap_or_else(|| {
                                    format!("Binary: {}", provider.binaries().join(" / "))
                                });

                            div()
                                .w_full()
                                .px_3p5()
                                .py_3()
                                .when(idx > 0, |r| r.border_t_1().border_color(rgb(0x252c39)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .size(px(32.0))
                                                .rounded_lg()
                                                .bg(rgb(0x232833))
                                                .border_1()
                                                .border_color(rgb(0x364052))
                                                .flex_none()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    svg()
                                                        .path(provider.icon_path())
                                                        .size(px(16.0))
                                                        .text_color(rgb(if is_on {
                                                            0xf1f5f9
                                                        } else {
                                                            0x64748b
                                                        })),
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
                                                                .text_xs()
                                                                .font_weight(
                                                                    gpui::FontWeight::SEMIBOLD,
                                                                )
                                                                .text_color(rgb(0xf1f5f9))
                                                                .child(provider.label()),
                                                        )
                                                        .child(
                                                            div()
                                                                .px_1p5()
                                                                .py_0p5()
                                                                .rounded_sm()
                                                                .bg(rgb(if available {
                                                                    0x064e3b
                                                                } else {
                                                                    0x262d3b
                                                                }))
                                                                .text_xs()
                                                                .text_color(rgb(if available {
                                                                    0x6ee7b7
                                                                } else {
                                                                    0x94a3b8
                                                                }))
                                                                .child(status_badge),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .font_family("Menlo")
                                                        .text_xs()
                                                        .text_color(rgb(0x94a3b8))
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
                                        .gap_3()
                                        .child(settings_action_btn(
                                            &format!("ai-check-{}", provider.as_str()),
                                            if checking { "Checking…" } else { "Check" },
                                            cx.listener(move |this, _, _, cx| {
                                                this.trigger_cli_check(provider, cx);
                                            }),
                                        ))
                                        .child(render_macos_switch(
                                            &format!("ai-switch-{}", provider.as_str()),
                                            is_on,
                                            false,
                                            true,
                                            cx.listener(move |this, _, _, cx| {
                                                let currently_on = this
                                                    .settings
                                                    .ai_enabled_agents
                                                    .get(provider.as_str())
                                                    .copied()
                                                    .unwrap_or(true);
                                                // Keep at least one agent enabled
                                                if currently_on
                                                    && this.enabled_ai_providers().len() <= 1
                                                {
                                                    return;
                                                }
                                                let next = !currently_on;
                                                this.settings
                                                    .ai_enabled_agents
                                                    .insert(provider.as_str().to_string(), next);
                                                if !next
                                                    && this.settings.ai_selected_agent
                                                        == provider.as_str()
                                                    && let Some(first_on) =
                                                        this.enabled_ai_providers().first().copied()
                                                {
                                                    this.settings.ai_selected_agent =
                                                        first_on.as_str().to_string();
                                                    this.ensure_agent_models_loaded(
                                                        first_on, false, cx,
                                                    );
                                                }
                                                save_settings(&this.settings);
                                                cx.notify();
                                            }),
                                        )),
                                )
                        },
                    )),
            )
            .into_any_element()
    }

    // ==================== RIGHT RESIZABLE AI SIDEBAR & SPLIT PICKER ====================

    pub(super) fn render_ai_sidebar_sash(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_dragging = matches!(self.sash_drag, SashDragState::AiSidebar { .. });
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
                            .id("ai-sidebar-resize-sash")
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
                                        this.settings.ai_sidebar_width = 340.0;
                                        this.sash_drag = SashDragState::None;
                                        save_settings(&this.settings);
                                    } else {
                                        this.sash_drag = SashDragState::AiSidebar {
                                            start_x: f32::from(event.position.x),
                                            start_width: this.settings.ai_sidebar_width,
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

    pub(super) fn render_ai_right_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.settings.ai_sidebar_width.clamp(280.0, 580.0);
        let provider = self.active_ai_provider();
        let (_, model_label, supported_efforts) = self.active_ai_model_info(provider);
        let effort_token = self
            .settings
            .ai_selected_efforts
            .get(provider.as_str())
            .cloned()
            .filter(|e| !e.is_empty())
            .filter(|e| {
                supported_efforts.is_empty()
                    || supported_efforts
                        .iter()
                        .any(|sup| sup.eq_ignore_ascii_case(e))
            });
        let effort_badge = effort_token
            .as_deref()
            .map(format_effort_label)
            .unwrap_or_else(|| "Default".to_string());

        let is_busy = self.ai_active_cancel.is_some();
        let project_short = std::path::Path::new(&self.active_project)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("laravel")
            .to_string();

        let prompt_field_click = self.ai_prompt_input.clone();

        div()
            .id("ai-assistant-right-sidebar")
            .relative()
            .w(px(width))
            .h_full()
            .flex_none()
            .bg(rgb(BG_CARD))
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.ai_picker_open || this.ai_effort_menu_open || this.ai_sessions_menu_open
                    {
                        this.ai_picker_open = false;
                        this.ai_effort_menu_open = false;
                        this.ai_sessions_menu_open = false;
                        cx.notify();
                    }
                }),
            )
            // 1. Top Header: Agent + Model Selector Pill + New Chat + Collapse
            .child(
                div()
                    .h(px(36.0))
                    .px_2p5()
                    .bg(rgb(0xf8fafc))
                    .border_b_1()
                    .border_color(rgb(BORDER_LIGHT))
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_1p5()
                    .child(
                        div()
                            .id("ai-agent-model-trigger")
                            .flex_1()
                            .min_w(px(0.0))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(if self.ai_picker_open {
                                0xeff6ff
                            } else {
                                0xffffff
                            }))
                            .border_1()
                            .border_color(rgb(if self.ai_picker_open {
                                0x93c5fd
                            } else {
                                0xe2e8f0
                            }))
                            .hover(|s| s.border_color(rgb(0x94a3b8)))
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_1p5()
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.ai_picker_open = !this.ai_picker_open;
                                this.ai_effort_menu_open = false;
                                this.ai_sessions_menu_open = false;
                                if this.ai_picker_open {
                                    let p = this.active_ai_provider();
                                    this.ensure_agent_models_loaded(p, false, cx);
                                    let fh = this.ai_model_search_input.focus_handle(cx);
                                    window.focus(&fh);
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        svg()
                                            .path(provider.icon_path())
                                            .size(px(13.5))
                                            .text_color(rgb(0x2563eb)),
                                    )
                                    // Model name only: the provider icon already says which
                                    // agent this is, and "Claude · Cl…" wasted the width.
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .truncate()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(TEXT_MAIN))
                                            .child(if model_label.trim().is_empty() {
                                                provider.short_label().to_string()
                                            } else {
                                                model_label.to_string()
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .when(!supported_efforts.is_empty(), |r| {
                                        r.child(
                                            div()
                                                .px_1p5()
                                                .rounded_sm()
                                                .bg(rgb(0xf1f5f9))
                                                .text_xs()
                                                .text_color(rgb(0x475569))
                                                .child(effort_badge),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(TEXT_MUTED))
                                            .child(if self.ai_picker_open { "▴" } else { "▾" }),
                                    ),
                            ),
                    )
                    .child({
                        let has_messages = !self.active_ai_messages().is_empty();
                        let session_count = self.ai_sessions.len();
                        let sessions_open = self.ai_sessions_menu_open;

                        div()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_0p5()
                            // Sessions Switcher (`clock.svg` + count badge when >1)
                            .child(
                                div()
                                    .id("ai-sessions-menu-btn")
                                    .h(px(24.0))
                                    .px_1p5()
                                    .rounded_md()
                                    .bg(if sessions_open {
                                        rgb(0xeff6ff)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .cursor_pointer()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.ai_sessions_menu_open = !this.ai_sessions_menu_open;
                                        this.ai_picker_open = false;
                                        this.ai_effort_menu_open = false;
                                        cx.notify();
                                    }))
                                    .tooltip(|_, cx| AiTooltip::view("Chat sessions", cx))
                                    .child(
                                        svg()
                                            .path("icons/clock.svg")
                                            .size(px(13.0))
                                            .text_color(rgb(if sessions_open {
                                                0x2563eb
                                            } else {
                                                TEXT_SECONDARY
                                            })),
                                    )
                                    .when(session_count > 1, |b| {
                                        b.child(
                                            div()
                                                .text_xs()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(if sessions_open {
                                                    0x1d4ed8
                                                } else {
                                                    TEXT_SECONDARY
                                                }))
                                                .child(format!("{session_count}")),
                                        )
                                    }),
                            )
                            // Open New Session (`+`)
                            .child(
                                div()
                                    .id("ai-new-session-btn")
                                    .size(px(24.0))
                                    .rounded_md()
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.open_new_ai_session(window, cx);
                                    }))
                                    .tooltip(|_, cx| AiTooltip::view("Open new session", cx))
                                    .child(
                                        svg()
                                            .path("icons/plus.svg")
                                            .size(px(13.5))
                                            .text_color(rgb(TEXT_SECONDARY)),
                                    ),
                            )
                            // Clear Active Chat (`trash.svg`)
                            .child(
                                div()
                                    .id("ai-clear-chat-btn")
                                    .size(px(24.0))
                                    .rounded_md()
                                    .when(has_messages, |b| {
                                        b.hover(|s| s.bg(rgb(0xfee2e2))).cursor_pointer()
                                    })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        if has_messages {
                                            this.clear_active_ai_session(window, cx);
                                        }
                                    }))
                                    .tooltip(|_, cx| AiTooltip::view("Clear chat", cx))
                                    .child(
                                        svg()
                                            .path("icons/trash.svg")
                                            .size(px(13.0))
                                            .text_color(rgb(if has_messages {
                                                TEXT_SECONDARY
                                            } else {
                                                0xcbd5e1
                                            })),
                                    ),
                            )
                    }),
            )
            // 2. Scrollable Chat Conversation Area
            .child(self.render_ai_chat_thread(cx))
            // 3. Composer Footer (Context Chips + Auto-Write macOS Switch + Input)
            .child(
                div()
                    .key_context("AiComposer")
                    .flex_none()
                    .p_2p5()
                    .bg(rgb(0xf8fafc))
                    .border_t_1()
                    .border_color(rgb(BORDER_LIGHT))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_1p5()
                            .child({
                                let include_editor = self.settings.ai_include_editor_context;
                                let project_tip = SharedString::from(format!(
                                    "Workspace: {}",
                                    self.active_project
                                ));
                                let include_tip: &'static str = if include_editor {
                                    "Include editor code (On)"
                                } else {
                                    "Include editor code (Off)"
                                };

                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .id("ai-ctx-project-chip")
                                            .h(px(22.0))
                                            .px_1p5()
                                            .rounded_md()
                                            .bg(rgb(0xffffff))
                                            .border_1()
                                            .border_color(rgb(0xe2e8f0))
                                            .max_w(px(140.0))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .overflow_hidden()
                                            .tooltip(move |_, cx| {
                                                AiTooltip::view(project_tip.clone(), cx)
                                            })
                                            .child(
                                                svg()
                                                    .path("icons/folder.svg")
                                                    .size(px(12.0))
                                                    .flex_none()
                                                    .text_color(rgb(TEXT_MUTED)),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(rgb(TEXT_SECONDARY))
                                                    .child(project_short),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("ai-ctx-buffer-chip")
                                            .flex_none()
                                            .h(px(22.0))
                                            .w(px(24.0))
                                            .rounded_md()
                                            .bg(rgb(if include_editor {
                                                0xeff6ff
                                            } else {
                                                0xffffff
                                            }))
                                            .border_1()
                                            .border_color(rgb(if include_editor {
                                                0x93c5fd
                                            } else {
                                                0xe2e8f0
                                            }))
                                            .hover(move |s| {
                                                s.bg(rgb(if include_editor {
                                                    0xdbeafe
                                                } else {
                                                    0xf1f5f9
                                                }))
                                            })
                                            .cursor_pointer()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings.ai_include_editor_context =
                                                    !this.settings.ai_include_editor_context;
                                                save_settings(&this.settings);
                                                cx.notify();
                                            }))
                                            .tooltip(move |_, cx| AiTooltip::view(include_tip, cx))
                                            .child(
                                                svg()
                                                    .path("icons/code.svg")
                                                    .size(px(13.5))
                                                    .text_color(rgb(if include_editor {
                                                        0x2563eb
                                                    } else {
                                                        TEXT_MUTED
                                                    })),
                                            ),
                                    )
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(TEXT_SECONDARY))
                                            .child("Auto-write"),
                                    )
                                    .child(render_macos_switch(
                                        "ai-composer-autowrite-switch",
                                        self.settings.ai_auto_write_editor,
                                        true,
                                        false,
                                        cx.listener(|this, _, _, cx| {
                                            this.settings.ai_auto_write_editor =
                                                !this.settings.ai_auto_write_editor;
                                            save_settings(&this.settings);
                                            cx.notify();
                                        }),
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .id("ai-prompt-input-box")
                                    .flex_1()
                                    .h(px(34.0))
                                    .px_2p5()
                                    .rounded_lg()
                                    .bg(rgb(0xffffff))
                                    .border_1()
                                    .border_color(rgb(0xcbd5e1))
                                    .cursor(gpui::CursorStyle::IBeam)
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        move |ev, window, cx| {
                                            cx.stop_propagation();
                                            prompt_field_click.update(cx, |tf, cx| {
                                                tf.handle_mouse_down(ev, window, cx);
                                            });
                                        },
                                    )
                                    .flex()
                                    .items_center()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .w_full()
                                            .h_full()
                                            .overflow_hidden()
                                            .child(self.ai_prompt_input.clone()),
                                    ),
                            )
                            .child(if is_busy {
                                div()
                                    .id("ai-stop-btn")
                                    .size(px(34.0))
                                    .rounded_lg()
                                    .bg(rgb(0xef4444))
                                    .hover(|s| s.bg(rgb(0xdc2626)))
                                    .cursor_pointer()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_active_ai_run(cx);
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/stop.svg")
                                            .size(px(13.0))
                                            .text_color(fixed_rgb(0xffffff)),
                                    )
                                    .into_any_element()
                            } else {
                                div()
                                    .id("ai-send-btn")
                                    .size(px(34.0))
                                    .rounded_lg()
                                    .bg(rgb(0x2563eb))
                                    .hover(|s| s.bg(rgb(0x1d4ed8)))
                                    .cursor_pointer()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit_ai_prompt(cx);
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/send.svg")
                                            .size(px(15.0))
                                            .text_color(fixed_rgb(0xffffff)),
                                    )
                                    .into_any_element()
                            }),
                    ),
            )
            // 4. Split Agent + Model + Effort Popover (when open)
            .when(self.ai_picker_open, |el| {
                el.child(self.render_ai_agent_model_popover(cx))
            })
            // 5. Sessions Switcher Popover (when open)
            .when(self.ai_sessions_menu_open, |el| {
                el.child(self.render_ai_sessions_popover(cx))
            })
    }

    fn render_ai_sessions_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.ai_sessions.len();
        let active_idx = self.ai_active_session_idx;

        div()
            .id("ai-sessions-popover")
            .occlude()
            .absolute()
            .right(px(8.0))
            .top(px(40.0))
            .w(px(248.0))
            .max_h(px(320.0))
            .rounded_xl()
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(rgb(0xcbd5e1))
            .shadow_xl()
            .overflow_hidden()
            .flex()
            .flex_col()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_click(|_, _, cx| {
                cx.stop_propagation();
            })
            .on_scroll_wheel(|_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .px_3()
                    .py_2()
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
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_MAIN))
                            .child(format!("Chat Sessions ({total})")),
                    )
                    .child(
                        div()
                            .id("sessions-popover-new-btn")
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(rgb(0x2563eb))
                            .hover(|s| s.bg(rgb(0x1d4ed8)))
                            .cursor_pointer()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(fixed_rgb(0xffffff))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_new_ai_session(window, cx);
                            }))
                            .child("+ New Session"),
                    ),
            )
            .child(
                div()
                    .id("ai-sessions-list-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .p_1p5()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .children(self.ai_sessions.iter().enumerate().rev().map(|(idx, sess)| {
                        let is_active = idx == active_idx;
                        let msg_count = sess.messages.len();
                        div()
                            .id(SharedString::from(format!("ai-session-row-{idx}")))
                            .px_2p5()
                            .py_1p5()
                            .rounded_lg()
                            .when(is_active, |r| r.bg(rgb(0xeff6ff)))
                            .hover(|s| s.bg(rgb(0xf1f5f9)))
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.ai_active_session_idx = idx;
                                this.ai_sessions_menu_open = false;
                                let fh = this.ai_prompt_input.focus_handle(cx);
                                window.focus(&fh);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .font_weight(if is_active {
                                                gpui::FontWeight::SEMIBOLD
                                            } else {
                                                gpui::FontWeight::NORMAL
                                            })
                                            .text_color(rgb(if is_active {
                                                0x1d4ed8
                                            } else {
                                                TEXT_MAIN
                                            }))
                                            .child(sess.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(TEXT_MUTED))
                                            .child(format!("{msg_count} prompt(s)")),
                                    ),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("ai-session-del-{idx}")))
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .hover(|s| s.bg(rgb(0xfee2e2)).text_color(rgb(0xb91c1c)))
                                    .text_xs()
                                    .text_color(rgb(TEXT_MUTED))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.close_ai_session_at(idx, cx);
                                    }))
                                    .child("×"),
                            )
                    })),
            )
    }

    fn render_picker_model_row(
        &self,
        idx: usize,
        m: AiModel,
        is_checked: bool,
        provider: CliProviderId,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mid = m.id.clone();
        let label = picker_model_label(provider, &m.label);
        let tip: SharedString = m.id.clone().into();

        div()
            .id(SharedString::from(format!("picker-model-row-{idx}")))
            .h(px(30.0))
            .px_2p5()
            .rounded_lg()
            .when(is_checked, |r| r.bg(rgb(0xf1f5f9)))
            .when(!is_checked, |r| r.hover(|s| s.bg(rgb(0xf8fafc))))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap_2p5()
            .tooltip(move |_, cx| AiTooltip::view(tip.clone(), cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.settings
                    .ai_selected_models
                    .insert(provider.as_str().to_string(), mid.clone());
                this.ai_picker_open = false;
                this.ai_effort_menu_open = false;
                save_settings(&this.settings);
                let fh = this.ai_prompt_input.focus_handle(cx);
                window.focus(&fh);
                cx.notify();
            }))
            .child(
                div()
                    .size(px(15.0))
                    .rounded_full()
                    .flex_none()
                    .border_1()
                    .border_color(rgb(if is_checked { 0x0f172a } else { 0xcbd5e1 }))
                    .when(is_checked, |c| c.bg(rgb(0x0f172a)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(is_checked, |c| {
                        c.child(
                            svg()
                                .path("icons/check.svg")
                                .size(px(9.0))
                                .text_color(rgb(0xffffff)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_xs()
                    .font_weight(if is_checked {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(rgb(TEXT_MAIN))
                    .child(label),
            )
            .when(m.is_default, |r| {
                r.child(
                    div()
                        .flex_none()
                        .text_size(px(10.5))
                        .text_color(rgb(TEXT_MUTED))
                        .child("default"),
                )
            })
            .into_any_element()
    }

    fn render_ai_agent_model_popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled_providers = self.enabled_ai_providers();
        let active_provider = self.active_ai_provider();
        let (selected_model_id, _, supported_efforts) =
            self.active_ai_model_info(active_provider);

        let rt = self.ai_agents.get(&active_provider);
        let available = rt.is_some_and(|s| s.available);
        let loading = rt.is_some_and(|s| s.loading_models);
        let list_source = rt
            .and_then(|s| s.model_list.as_ref().map(|l| l.source.clone()))
            .unwrap_or_else(|| "catalog".to_string());
        let list_message = rt
            .and_then(|s| s.model_list.as_ref().map(|l| l.message.clone()))
            .filter(|m| !m.is_empty());
        let version_subtitle = match rt.and_then(|s| s.version.as_deref()) {
            Some(ver) => format!(
                "{} · {}",
                picker_version_label(ver),
                picker_source_label(&list_source)
            ),
            None if available || active_provider == CliProviderId::ClaudeCode => {
                picker_source_label(&list_source).to_string()
            }
            None => format!("`{}` not found on PATH", active_provider.binaries()[0]),
        };
        let refresh_tip: SharedString = match list_message {
            Some(msg) => format!("{msg} Click to refresh."),
            None => "Refresh model list".to_string(),
        }
        .into();

        let search_q = self
            .ai_model_search_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        let search_lower = search_q.to_lowercase();

        let models: Vec<AiModel> = rt
            .and_then(|s| s.model_list.as_ref().map(|l| l.models.clone()))
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                search_lower.is_empty()
                    || m.label.to_lowercase().contains(&search_lower)
                    || m.id.to_lowercase().contains(&search_lower)
            })
            .collect();

        let exact_match_exists = models
            .iter()
            .any(|m| m.id.eq_ignore_ascii_case(&search_q) || m.label.eq_ignore_ascii_case(&search_q));

        let cur_effort = self
            .settings
            .ai_selected_efforts
            .get(active_provider.as_str())
            .cloned()
            .filter(|e| !e.is_empty())
            .filter(|e| {
                supported_efforts.is_empty()
                    || supported_efforts
                        .iter()
                        .any(|sup| sup.eq_ignore_ascii_case(e))
            });
        let effort_enabled = !supported_efforts.is_empty();
        let effort_label = if effort_enabled {
            cur_effort
                .as_deref()
                .map(format_effort_label)
                .unwrap_or_else(|| "Default".to_string())
        } else {
            "Not supported".to_string()
        };

        let mut effort_choices: Vec<Option<String>> = vec![None];
        for eff in &supported_efforts {
            effort_choices.push(Some(eff.clone()));
        }

        let search_field_click = self.ai_model_search_input.clone();

        let no_models = models.is_empty();
        // Claude gets grouped sections; other providers (and any search) stay flat.
        let mut model_rows: Vec<gpui::AnyElement> = Vec::new();
        let mut row_idx = 0usize;
        let mut push_rows = |rows: &mut Vec<gpui::AnyElement>, list: Vec<AiModel>, cx: &mut Context<Self>| {
            for m in list {
                let checked = m.id == selected_model_id;
                rows.push(self.render_picker_model_row(row_idx, m, checked, active_provider, cx));
                row_idx += 1;
            }
        };
        if active_provider == CliProviderId::ClaudeCode && search_lower.is_empty() {
            let (latest, older, aliases) = group_claude_models(models);
            if !latest.is_empty() {
                model_rows.push(picker_section_header("Latest"));
                push_rows(&mut model_rows, latest, cx);
            }
            if !older.is_empty() {
                let pinned_open = older.iter().any(|m| m.id == selected_model_id);
                let expanded = pinned_open || self.ai_picker_show_older;
                let count = older.len();
                model_rows.push(
                    div()
                        .id("picker-older-toggle")
                        .mt_1()
                        .px_2p5()
                        .py_1()
                        .rounded_md()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .text_size(px(10.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT_MUTED))
                        .when(!pinned_open, |el| {
                            el.cursor_pointer()
                                .hover(|s| s.bg(rgb(0xf8fafc)).text_color(rgb(TEXT_SECONDARY)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.ai_picker_show_older = !this.ai_picker_show_older;
                                    cx.notify();
                                }))
                        })
                        .child(format!("OLDER MODELS ({count})"))
                        .child(div().flex_1())
                        .when(!pinned_open, |el| {
                            el.child(
                                svg()
                                    .path(if expanded {
                                        "icons/chevron-down.svg"
                                    } else {
                                        "icons/chevron-right.svg"
                                    })
                                    .size(px(11.0))
                                    .text_color(rgb(TEXT_MUTED)),
                            )
                        })
                        .into_any_element(),
                );
                if expanded {
                    push_rows(&mut model_rows, older, cx);
                }
            }
            if !aliases.is_empty() {
                model_rows.push(picker_section_header("Aliases"));
                push_rows(&mut model_rows, aliases, cx);
            }
        } else {
            push_rows(&mut model_rows, models, cx);
        }

        div()
            .id("ai-split-picker-popover")
            .occlude()
            .absolute()
            .left(px(8.0))
            .right(px(8.0))
            .top(px(40.0))
            .bottom(px(12.0))
            .max_h(px(470.0))
            .rounded_xl()
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(rgb(0xcbd5e1))
            .shadow_xl()
            .overflow_hidden()
            .flex()
            .flex_row()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if this.ai_effort_menu_open {
                    this.ai_effort_menu_open = false;
                    cx.notify();
                }
            }))
            .on_scroll_wheel(|_, _, cx| {
                cx.stop_propagation();
            })
            // Left Strip: Enabled AI Agents + Bottom Settings Sliders Shortcut
            .child(
                div()
                    .w(px(46.0))
                    .h_full()
                    .flex_none()
                    .bg(rgb(0xf8fafc))
                    .border_r_1()
                    .border_color(rgb(BORDER_LIGHT))
                    .py_2p5()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1p5()
                            .children(enabled_providers.into_iter().map(|prov| {
                                let is_sel = prov == active_provider;
                                let is_avail = self
                                    .ai_agents
                                    .get(&prov)
                                    .is_some_and(|s| s.available);
                                let tip: SharedString = if is_avail {
                                    prov.label().into()
                                } else {
                                    format!("{} · not installed", prov.label()).into()
                                };

                                div()
                                    .id(SharedString::from(format!(
                                        "picker-agent-{}",
                                        prov.as_str()
                                    )))
                                    .tooltip(move |_, cx| AiTooltip::view(tip.clone(), cx))
                                    .relative()
                                    .size(px(34.0))
                                    .rounded_lg()
                                    .bg(if is_sel {
                                        rgb(0xffffff)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .border_1()
                                    .border_color(if is_sel {
                                        rgb(0x2563eb)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .when(is_sel, |b| b.shadow_xs())
                                    .hover(|s| s.bg(rgb(0xffffff)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.settings.ai_selected_agent =
                                            prov.as_str().to_string();
                                        this.ai_effort_menu_open = false;
                                        save_settings(&this.settings);
                                        this.ensure_agent_models_loaded(prov, false, cx);
                                        cx.notify();
                                    }))
                                    .child(
                                        svg()
                                            .path(prov.icon_path())
                                            .size(px(16.5))
                                            .text_color(rgb(if is_sel {
                                                0x0f172a
                                            } else if is_avail {
                                                0x475569
                                            } else {
                                                0x94a3b8
                                            })),
                                    )
                                    .when(is_avail, |btn| {
                                        btn.child(
                                            div()
                                                .absolute()
                                                .right(px(2.0))
                                                .bottom(px(2.0))
                                                .size(px(6.0))
                                                .rounded_full()
                                                .bg(rgb(0x10b981))
                                                .border_1()
                                                .border_color(rgb(0xffffff)),
                                        )
                                    })
                            })),
                    )
                    .child(
                        div()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(BORDER_LIGHT))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .id("picker-open-ai-settings")
                                    .size(px(32.0))
                                    .rounded_lg()
                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.ai_picker_open = false;
                                        this.ai_effort_menu_open = false;
                                        this.active_settings_tab = SettingsTab::Ai;
                                        this.open_settings(window, cx);
                                    }))
                                    .tooltip(|_, cx| AiTooltip::view("AI agent settings", cx))
                                    .child(
                                        svg()
                                            .path("icons/sliders.svg")
                                            .size(px(15.0))
                                            .text_color(rgb(TEXT_SECONDARY)),
                                    ),
                            ),
                    ),
            )
            // Right Panel: Agent Header + Search Models + Radio List + Effort Footer
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    // Agent Header
                    .child(
                        div()
                            .px_3p5()
                            .pt_3()
                            .pb_2()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_start()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(0x0f172a))
                                            .child(active_provider.short_label()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(rgb(TEXT_SECONDARY))
                                            .child(version_subtitle),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .id("picker-refresh-models")
                                            .size(px(24.0))
                                            .rounded_md()
                                            .hover(|s| s.bg(rgb(BG_HOVER)))
                                            .cursor_pointer()
                                            .tooltip(move |_, cx| {
                                                AiTooltip::view(refresh_tip.clone(), cx)
                                            })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.ensure_agent_models_loaded(
                                                    active_provider,
                                                    true,
                                                    cx,
                                                );
                                            }))
                                            .child(
                                                svg()
                                                    .path("icons/refresh.svg")
                                                    .size(px(13.0))
                                                    .text_color(rgb(TEXT_SECONDARY)),
                                            ),
                                    ),
                            ),
                    )
                    // Search models input
                    .child(
                        div().px_3().pb_2().flex_none().child(
                            div()
                                .id("picker-search-models-box")
                                .h(px(30.0))
                                .px_2p5()
                                .rounded_lg()
                                .bg(rgb(0xf1f5f9))
                                .border_1()
                                .border_color(rgb(0xe2e8f0))
                                .cursor(gpui::CursorStyle::IBeam)
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    move |ev, window, cx| {
                                        cx.stop_propagation();
                                        search_field_click.update(cx, |tf, cx| {
                                            tf.handle_mouse_down(ev, window, cx);
                                        });
                                    },
                                )
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    svg()
                                        .path("icons/search.svg")
                                        .size(px(12.5))
                                        .text_color(rgb(TEXT_MUTED)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .h_full()
                                        .overflow_hidden()
                                        .child(self.ai_model_search_input.clone()),
                                ),
                        ),
                    )
                    // Scrollable Models List
                    .child(
                        div()
                            .id("picker-models-scroll")
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .px_2()
                            .py_1()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .when(loading, |el| {
                                el.child(
                                    div()
                                        .p_3()
                                        .text_xs()
                                        .text_color(rgb(TEXT_SECONDARY))
                                        .child("Loading models…"),
                                )
                            })
                            .when(
                                !loading && !available && active_provider != CliProviderId::ClaudeCode,
                                |el| {
                                    el.child(
                                        div()
                                            .m_1p5()
                                            .p_3()
                                            .rounded_lg()
                                            .bg(rgb(0xf8fafc))
                                            .border_1()
                                            .border_color(rgb(BORDER_LIGHT))
                                            .flex()
                                            .flex_col()
                                            .gap_1p5()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(TEXT_MAIN))
                                                    .child(format!(
                                                        "{} CLI not detected",
                                                        active_provider.label()
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(TEXT_SECONDARY))
                                                    .child(format!(
                                                        "Install `{}` on your PATH, then click Refresh or Check in Settings.",
                                                        active_provider.binaries()[0]
                                                    )),
                                            ),
                                    )
                                },
                            )
                            .when_some(
                                rt.and_then(|s| s.model_error.clone()).filter(|_| !loading && no_models),
                                |el, err_msg| {
                                    el.child(
                                        div()
                                            .m_1p5()
                                            .p_2p5()
                                            .rounded_lg()
                                            .bg(rgb(0xfef2f2))
                                            .border_1()
                                            .border_color(rgb(0xfecaca))
                                            .text_xs()
                                            .text_color(rgb(0xb91c1c))
                                            .child(err_msg),
                                    )
                                },
                            )
                            .children(model_rows)
                            .when(!search_q.is_empty() && !exact_match_exists, |el| {
                                let custom_id = search_q.clone();
                                el.child(
                                    div()
                                        .id("picker-use-custom-model")
                                        .mt_1()
                                        .px_2p5()
                                        .py_1p5()
                                        .rounded_lg()
                                        .bg(rgb(0xeff6ff))
                                        .border_1()
                                        .border_color(rgb(0xbfdbfe))
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(rgb(0x1d4ed8))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.settings.ai_selected_models.insert(
                                                active_provider.as_str().to_string(),
                                                custom_id.clone(),
                                            );
                                            this.ai_picker_open = false;
                                            save_settings(&this.settings);
                                            let fh = this.ai_prompt_input.focus_handle(cx);
                                            window.focus(&fh);
                                            cx.notify();
                                        }))
                                        .child(format!("Use custom model ID \"{search_q}\"")),
                                )
                            }),
                    )
                    // Pinned Bottom Effort Bar (matching reference screenshot)
                    .child(
                        div()
                            .h(px(40.0))
                            .px_3p5()
                            .bg(rgb(0xf8fafc))
                            .border_t_1()
                            .border_color(rgb(BORDER_LIGHT))
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(TEXT_SECONDARY))
                                    .child("Effort"),
                            )
                            .child(
                                div()
                                    .id("picker-effort-btn")
                                    .min_w(px(110.0))
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(if effort_enabled { 0xffffff } else { 0xf8fafc }))
                                    .border_1()
                                    .border_color(rgb(if effort_enabled { 0xcbd5e1 } else { BORDER_LIGHT }))
                                    .when(!effort_enabled, |b| {
                                        b.tooltip(|_, cx| {
                                            AiTooltip::view("This model has no effort levels", cx)
                                        })
                                    })
                                    .when(!supported_efforts.is_empty(), |b| {
                                        b.hover(|s| s.border_color(rgb(0x94a3b8))).cursor_pointer()
                                    })
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if !supported_efforts.is_empty() {
                                            this.ai_effort_menu_open = !this.ai_effort_menu_open;
                                            cx.notify();
                                        }
                                    }))
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(if effort_enabled { TEXT_MAIN } else { TEXT_MUTED }))
                                            .child(effort_label),
                                    )
                                    .when(effort_enabled, |b| {
                                        b.child(
                                            svg()
                                                .path("icons/chevron-down.svg")
                                                .size(px(11.0))
                                                .text_color(rgb(TEXT_MUTED)),
                                        )
                                    }),
                            ),
                    )
                    // Effort Popup Menu (above bottom Effort bar)
                    .when(self.ai_effort_menu_open && !effort_choices.is_empty(), |el| {
                        el.child(
                            div()
                                .id("picker-effort-popup")
                                .occlude()
                                .absolute()
                                .right(px(12.0))
                                .bottom(px(44.0))
                                .w(px(136.0))
                                .p_1()
                                .rounded_lg()
                                .bg(rgb(0xffffff))
                                .border_1()
                                .border_color(rgb(0xcbd5e1))
                                .shadow_lg()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .children(effort_choices.into_iter().enumerate().map(
                                    |(e_idx, opt_tok)| {
                                        let is_cur = match (&cur_effort, &opt_tok) {
                                            (None, None) => true,
                                            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                                            _ => false,
                                        };
                                        let lbl = opt_tok
                                            .as_deref()
                                            .map(format_effort_label)
                                            .unwrap_or_else(|| "Default".to_string());

                                        div()
                                            .id(SharedString::from(format!("eff-opt-{e_idx}")))
                                            .px_2p5()
                                            .py_1()
                                            .rounded_md()
                                            .when(is_cur, |r| r.bg(rgb(0xeff6ff)))
                                            .hover(|s| s.bg(rgb(0xf1f5f9)))
                                            .cursor_pointer()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .text_xs()
                                            .text_color(rgb(if is_cur {
                                                0x1d4ed8
                                            } else {
                                                TEXT_MAIN
                                            }))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                match &opt_tok {
                                                    Some(tok) => {
                                                        this.settings.ai_selected_efforts.insert(
                                                            active_provider.as_str().to_string(),
                                                            tok.clone(),
                                                        );
                                                    }
                                                    None => {
                                                        this.settings
                                                            .ai_selected_efforts
                                                            .remove(active_provider.as_str());
                                                    }
                                                }
                                                this.ai_effort_menu_open = false;
                                                save_settings(&this.settings);
                                                cx.notify();
                                            }))
                                            .child(lbl)
                                            .when(is_cur, |r| {
                                                r.child(
                                                    svg()
                                                        .path("icons/check.svg")
                                                        .size(px(10.0))
                                                        .text_color(rgb(0x1d4ed8)),
                                                )
                                            })
                                    },
                                )),
                        )
                    }),
            )
    }

    fn render_ai_chat_thread(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let messages = self.active_ai_messages();
        if messages.is_empty() {
            let active_session_title = self
                .ai_sessions
                .get(self.ai_active_session_idx)
                .map(|s| s.title.clone())
                .unwrap_or_else(|| "Session 1".to_string());
            let show_session_pill = self.ai_sessions.len() > 1;
            let examples = [
                "How many users registered yesterday?",
                "List the 10 latest records with their relations",
                "Explain or optimize the query in my editor",
            ];

            return div()
                .id("ai-chat-empty-state")
                .flex_1()
                .min_h(px(0.0))
                .px_4()
                .pt_6()
                .pb_4()
                .flex()
                .flex_col()
                .justify_start()
                .gap_3()
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
                                .justify_between()
                                .gap_1p5()
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_1p5()
                                        .child(
                                            svg()
                                                .path("icons/sparkles.svg")
                                                .size(px(15.0))
                                                .text_color(rgb(0x2563eb)),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(rgb(TEXT_MAIN))
                                                .child("Ask Laralah AI"),
                                        ),
                                )
                                .when(show_session_pill, |r| {
                                    r.child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_md()
                                            .bg(rgb(0xeff6ff))
                                            .border_1()
                                            .border_color(rgb(0xbfdbfe))
                                            .text_xs()
                                            .text_color(rgb(0x1d4ed8))
                                            .child(active_session_title),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .line_height(px(18.0))
                                .text_color(rgb(TEXT_SECONDARY))
                                .child("Describe what you want to query or inspect. Laralah automatically checks your Eloquent models & columns and writes ready-to-run code to the editor."),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .children(examples.into_iter().enumerate().map(|(idx, ex)| {
                            let prompt_str = ex.to_string();
                            div()
                                .id(SharedString::from(format!("ai-starter-{idx}")))
                                .px_2p5()
                                .py_1p5()
                                .rounded_md()
                                .bg(rgb(0xf8fafc))
                                .border_1()
                                .border_color(rgb(BORDER_LIGHT))
                                .hover(|s| s.bg(rgb(0xeff6ff)).border_color(rgb(0xbfdbfe)))
                                .cursor_pointer()
                                .text_xs()
                                .text_color(rgb(TEXT_MAIN))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.ai_prompt_input.update(cx, |tf, cx| {
                                        tf.set_text(&prompt_str, cx);
                                    });
                                    let fh = this.ai_prompt_input.focus_handle(cx);
                                    window.focus(&fh);
                                    cx.notify();
                                }))
                                .child(format!("“{ex}”"))
                        })),
                )
                .into_any_element();
        }

        div()
            .id("ai-chat-scroll-area")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.ai_chat_scroll)
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .children(
                messages
                    .iter()
                    .map(|msg| self.render_ai_message_card(msg, cx)),
            )
            .into_any_element()
    }

    fn render_ai_message_card(
        &self,
        msg: &AiChatMessage,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let msg_id = msg.id;
        let is_copied = self.ai_copied_msg_id == Some(msg_id);

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            // User prompt bubble
            .child(
                div()
                    .ml_5()
                    .px_3()
                    .py_2()
                    .rounded_xl()
                    .bg(rgb(0xf1f5f9))
                    .border_1()
                    .border_color(rgb(0xe2e8f0))
                    .text_xs()
                    .line_height(px(18.0))
                    .text_color(rgb(TEXT_MAIN))
                    .child(msg.user_prompt.clone()),
            )
            // Tool / Schema Inspection Step Pills
            .when(!msg.tool_steps.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(msg.tool_steps.iter().map(|step: &AiToolStep| {
                            div()
                                .px_2p5()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(if step.is_error { 0xfef2f2 } else { 0xf8fafc }))
                                .border_1()
                                .border_color(rgb(if step.is_error {
                                    0xfecaca
                                } else {
                                    BORDER_LIGHT
                                }))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .truncate()
                                        .text_xs()
                                        .text_color(rgb(if step.is_error {
                                            0xb91c1c
                                        } else {
                                            TEXT_SECONDARY
                                        }))
                                        .child(step.summary.clone()),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(if step.is_error {
                                            0xef4444
                                        } else if step.done {
                                            0x10b981
                                        } else {
                                            0x2563eb
                                        }))
                                        .child(if step.is_error {
                                            "!"
                                        } else if step.done {
                                            "✓"
                                        } else {
                                            "…"
                                        }),
                                )
                        })),
                )
            })
            // Streaming live preview (while running)
            .when(msg.status == AiMessageStatus::Running, |el| {
                let live_txt = if msg.streaming_text.trim().is_empty() {
                    format!("{} ({}) is thinking…", msg.agent.short_label(), msg.model_label)
                } else {
                    msg.streaming_text.clone()
                };
                el.child(
                    div()
                        .p_2p5()
                        .rounded_lg()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(0xe2e8f0))
                        .text_xs()
                        .line_height(px(18.0))
                        .text_color(rgb(if msg.streaming_text.trim().is_empty() {
                            TEXT_MUTED
                        } else {
                            TEXT_MAIN
                        }))
                        .child(live_txt),
                )
            })
            // Completed Assistant Card
            .when(msg.status == AiMessageStatus::Done, |el| {
                let code_opt = msg.php_code.clone();
                let written = msg.written_to_editor;
                let meta_line = match &msg.effort {
                    Some(eff) => format!(
                        "{} · {} · {}",
                        msg.agent.short_label(),
                        msg.model_label,
                        format_effort_label(eff)
                    ),
                    None => format!("{} · {}", msg.agent.short_label(), msg.model_label),
                };

                el.child(
                    div()
                        .p_2p5()
                        .rounded_xl()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(BORDER_CARD))
                        .shadow_xs()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(rgb(TEXT_MUTED))
                                        .child(meta_line),
                                )
                                .when(msg.php_code.is_some() && msg.lint_passed, |r| {
                                    r.child(
                                        div()
                                            .flex_none()
                                            .text_xs()
                                            .text_color(rgb(0x10b981))
                                            .child("php -l ✓"),
                                    )
                                }),
                        )
                        .when(!msg.explanation.is_empty(), |card| {
                            card.child(
                                div()
                                    .text_xs()
                                    .line_height(px(18.0))
                                    .text_color(rgb(TEXT_MAIN))
                                    .child(msg.explanation.clone()),
                            )
                        })
                        .when_some(code_opt, |card, code_str| {
                            let c_write = code_str.clone();
                            let c_insert = code_str.clone();
                            let c_run = code_str.clone();
                            let c_copy = code_str.clone();
                            let can_direct_run =
                                !self.active_target.is_ssh() && is_safe_readonly_php(&code_str);

                            card.child(
                                div()
                                    .rounded_lg()
                                    .bg(rgb(0xf8fafc))
                                    .border_1()
                                    .border_color(rgb(0xe2e8f0))
                                    .overflow_hidden()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "ai-code-scroll-{msg_id}"
                                            )))
                                            .p_2p5()
                                            .overflow_x_scroll()
                                            .font_family("Menlo")
                                            .text_xs()
                                            .line_height(px(18.0))
                                            .text_color(rgb(0x0f172a))
                                            .child(code_str),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_1p5()
                                            .bg(rgb(0xf1f5f9))
                                            .border_t_1()
                                            .border_color(rgb(0xe2e8f0))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .gap_1p5()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .id(SharedString::from(format!(
                                                                "ai-write-ed-{msg_id}"
                                                            )))
                                                            .px_2()
                                                            .py_0p5()
                                                            .rounded_sm()
                                                            .bg(rgb(if written {
                                                                0x10b981
                                                            } else {
                                                                0x2563eb
                                                            }))
                                                            .hover(|s| s.opacity(0.9))
                                                            .cursor_pointer()
                                                            .text_xs()
                                                            .font_weight(gpui::FontWeight::MEDIUM)
                                                            .text_color(fixed_rgb(0xffffff))
                                                            .tooltip(|_, cx| {
                                                                AiTooltip::view(
                                                                    "Append snippet to bottom of editor",
                                                                    cx,
                                                                )
                                                            })
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.append_ai_code_to_editor(
                                                                        &c_write, false, cx,
                                                                    );
                                                                    if let Some(m) = this
                                                                        .find_ai_message_mut(msg_id)
                                                                    {
                                                                        m.written_to_editor = true;
                                                                    }
                                                                    cx.notify();
                                                                },
                                                            ))
                                                            .child(if written {
                                                                "✓ Appended"
                                                            } else {
                                                                "Append"
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .id(SharedString::from(format!(
                                                                "ai-replace-ed-{msg_id}"
                                                            )))
                                                            .px_2()
                                                            .py_0p5()
                                                            .rounded_sm()
                                                            .bg(rgb(0xffffff))
                                                            .border_1()
                                                            .border_color(rgb(0xcbd5e1))
                                                            .hover(|s| s.bg(rgb(BG_HOVER)))
                                                            .cursor_pointer()
                                                            .text_xs()
                                                            .text_color(rgb(TEXT_MAIN))
                                                            .tooltip(|_, cx| {
                                                                AiTooltip::view(
                                                                    "Overwrite entire editor with this snippet",
                                                                    cx,
                                                                )
                                                            })
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.replace_ai_code_in_editor(
                                                                        &c_insert, false, cx,
                                                                    );
                                                                    if let Some(m) = this
                                                                        .find_ai_message_mut(msg_id)
                                                                    {
                                                                        m.written_to_editor = true;
                                                                    }
                                                                    cx.notify();
                                                                },
                                                            ))
                                                            .child("Replace"),
                                                    )
                                                    .when(can_direct_run, |r| {
                                                        r.child(
                                                            div()
                                                                .id(SharedString::from(format!(
                                                                    "ai-run-ed-{msg_id}"
                                                                )))
                                                                .px_2()
                                                                .py_0p5()
                                                                .rounded_sm()
                                                                .bg(rgb(0x0f172a))
                                                                .hover(|s| s.bg(rgb(0x1e293b)))
                                                                .cursor_pointer()
                                                                .text_xs()
                                                                .text_color(rgb(0xffffff))
                                                                .on_click(cx.listener(
                                                                    move |this, _, _, cx| {
                                                                        if written {
                                                                            this.run_evaluation(cx);
                                                                        } else {
                                                                            this.append_ai_code_to_editor(
                                                                                &c_run, true, cx,
                                                                            );
                                                                            if let Some(m) = this
                                                                                .find_ai_message_mut(
                                                                                    msg_id,
                                                                                )
                                                                            {
                                                                                m.written_to_editor =
                                                                                    true;
                                                                            }
                                                                        }
                                                                    },
                                                                ))
                                                                .child("▶ Run"),
                                                        )
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "ai-copy-ed-{msg_id}"
                                                    )))
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded_sm()
                                                    .hover(|s| s.bg(rgb(0xe2e8f0)))
                                                    .cursor_pointer()
                                                    .text_xs()
                                                    .text_color(rgb(TEXT_SECONDARY))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            cx.write_to_clipboard(
                                                                ClipboardItem::new_string(
                                                                    c_copy.clone(),
                                                                ),
                                                            );
                                                            this.ai_copied_msg_id = Some(msg_id);
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .child(if is_copied {
                                                        "Copied"
                                                    } else {
                                                        "Copy"
                                                    }),
                                            ),
                                    ),
                            )
                        }),
                )
            })
            // Failed / Cancelled state
            .when_some(
                match &msg.status {
                    AiMessageStatus::Failed(e) => Some((e.clone(), true)),
                    AiMessageStatus::Cancelled => Some(("Cancelled.".to_string(), false)),
                    _ => None,
                },
                |el, (txt, is_err)| {
                    el.child(
                        div()
                            .p_2p5()
                            .rounded_lg()
                            .bg(rgb(if is_err { 0xfef2f2 } else { 0xf8fafc }))
                            .border_1()
                            .border_color(rgb(if is_err { 0xfecaca } else { BORDER_LIGHT }))
                            .text_xs()
                            .line_height(px(17.0))
                            .text_color(rgb(if is_err { 0xb91c1c } else { TEXT_MUTED }))
                            .child(txt),
                    )
                },
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AiModel, CliProviderId, group_claude_models, merge_and_append_php_snippet,
        picker_model_label, picker_version_label,
    };

    #[test]
    fn model_picker_labels_and_grouping() {
        assert_eq!(picker_model_label(CliProviderId::ClaudeCode, "Claude Opus 5.5"), "Opus 5.5");
        assert_eq!(picker_model_label(CliProviderId::Codex, "gpt-5"), "gpt-5");
        assert_eq!(picker_version_label("2.1.326 (Claude Code)"), "v2.1.326");
        assert_eq!(picker_version_label("codex-cli 0.50.0"), "v0.50.0");

        let mk = |id: &str| AiModel {
            id: id.to_string(),
            label: id.to_string(),
            detail: None,
            is_default: false,
            reasoning_efforts: vec![],
        };
        let (latest, older, aliases) = group_claude_models(
            ["claude-fable-5-2", "claude-opus-5", "claude-opus-5-5", "claude-opus-4-8", "claude-fable-5-1", "sonnet"]
                .into_iter()
                .map(mk)
                .collect(),
        );
        let ids = |v: &[AiModel]| v.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&latest), ["claude-fable-5-2", "claude-opus-5-5"]);
        assert_eq!(ids(&older), ["claude-fable-5-1", "claude-opus-5", "claude-opus-4-8"]);
        assert_eq!(ids(&aliases), ["sonnet"]);
    }

    #[test]
    fn appends_ai_php_snippet_and_hoists_use_imports_without_duplicates() {
        assert_eq!(
            merge_and_append_php_snippet("   \n", "use App\\Models\\User;\n\nUser::count();"),
            "use App\\Models\\User;\n\nUser::count();"
        );

        let existing = "use App\\Models\\User;\n\nUser::count();";
        let incoming = "<?php\nuse App\\Models\\User;\nuse Carbon\\Carbon;\n\nUser::whereDate('created_at', Carbon::yesterday())->count();";
        let merged = merge_and_append_php_snippet(existing, incoming);

        assert_eq!(
            merged,
            "use App\\Models\\User;\nuse Carbon\\Carbon;\n\nUser::count();\n\nUser::whereDate('created_at', Carbon::yesterday())->count();"
        );
    }
}
