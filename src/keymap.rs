//! User-customizable keyboard shortcuts.
//!
//! Every rebindable command is listed once in [`shortcut_specs`] with its default
//! keystrokes. The user's overrides live in `AppSettings::custom_shortcuts`
//! (spec id -> keystrokes; an empty list means "unbound"), and [`apply`] rebuilds
//! gpui's whole keymap from the fixed editor bindings plus the effective
//! customizable ones. Keystrokes are stored in gpui syntax (`secondary-shift-p`),
//! using `secondary` for Cmd on macOS / Ctrl elsewhere so a settings file stays
//! portable between platforms.

use crate::app::{
    CloseOverlay, CloseScratchTab, FocusLogSearch, FocusSettingsSearch, FocusSshSearch,
    GraphFitView, GraphZoomIn, GraphZoomOut, ModeDetailDive, ModeRawCli, ModeTable, NewScratchTab,
    NextScratchTab, OpenDirectory, OpenPlayground, PaletteConfirm, PaletteDown, PaletteUp, PrevScratchTab, QuitApp,
    FormatCode, RunCode, SubmitAiPrompt, ToggleAiSidebar, ToggleLogs, TogglePalette, ToggleSettings,
    ToggleSidebar, ToggleSql, ToggleSshModal,
};
use gpui::{Action, App, KeyBinding, Keystroke};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, RwLock};

/// One action a shortcut triggers, and the key context it is bound in.
pub struct ShortcutTarget {
    pub build: fn() -> Box<dyn Action>,
    pub context: Option<&'static str>,
}

/// A rebindable command as shown in Settings → Shortcuts.
pub struct ShortcutSpec {
    /// Stable id used as the key in the settings file.
    pub id: &'static str,
    pub label: &'static str,
    pub defaults: &'static [&'static str],
    pub targets: &'static [ShortcutTarget],
    /// Which keys may be used without Cmd/Ctrl/Alt.
    pub bare: BareKeys,
}

/// Keys a shortcut may use without a Cmd/Ctrl/Alt modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BareKeys {
    /// None besides function keys: a bare key would swallow normal typing.
    None,
    /// Enter only (with or without Shift): for text inputs, where letters and
    /// navigation keys must keep typing and moving the caret.
    Enter,
    /// Any key: only for contexts that hold no text input.
    Any,
}

impl std::fmt::Debug for ShortcutSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShortcutSpec").field("id", &self.id).finish_non_exhaustive()
    }
}

impl ShortcutSpec {
    fn is_global(&self) -> bool {
        self.targets.iter().any(|t| t.context.is_none())
    }

    /// True when some target can fire while a text input has focus: anything but
    /// the object graph, which holds none.
    fn reaches_text_input(&self) -> bool {
        self.targets.iter().any(|t| t.context != Some("ObjectGraph"))
    }

    fn shares_context_with(&self, other: &ShortcutSpec) -> bool {
        if self.is_global() || other.is_global() {
            return true;
        }
        self.targets
            .iter()
            .any(|a| other.targets.iter().any(|b| a.context == b.context))
    }
}

macro_rules! target {
    ($action:expr) => {
        ShortcutTarget { build: || Box::new($action), context: None }
    };
    ($action:expr, $ctx:expr) => {
        ShortcutTarget { build: || Box::new($action), context: Some($ctx) }
    };
}

macro_rules! spec {
    ($id:expr, $label:expr, [$($key:expr),* $(,)?], [$($t:expr),+ $(,)?]) => {
        ShortcutSpec { id: $id, label: $label, defaults: &[$($key),*], targets: &[$($t),+], bare: BareKeys::None }
    };
    // Different defaults on macOS and on Windows/Linux, where the OS or input
    // methods claim some keys first.
    ($id:expr, $label:expr, mac [$($mk:expr),* $(,)?], other [$($ok:expr),* $(,)?], [$($t:expr),+ $(,)?]) => {
        ShortcutSpec {
            id: $id,
            label: $label,
            defaults: if cfg!(target_os = "macos") { &[$($mk),*] } else { &[$($ok),*] },
            targets: &[$($t),+],
            bare: BareKeys::None,
        }
    };
    ($id:expr, $label:expr, [$($key:expr),* $(,)?], [$($t:expr),+ $(,)?], bare $bare:ident) => {
        ShortcutSpec { id: $id, label: $label, defaults: &[$($key),*], targets: &[$($t),+], bare: BareKeys::$bare }
    };
}

pub fn shortcut_specs() -> &'static [ShortcutSpec] {
    static SPECS: &[ShortcutSpec] = &[
        spec!("run_code", "Run code", ["secondary-r"], [target!(RunCode)]),
        spec!(
            "format_code",
            "Format code",
            // Alt+Shift switches keyboard layouts on Windows.
            mac ["secondary-shift-f", "alt-shift-f"],
            other ["secondary-shift-f"],
            [target!(FormatCode)]
        ),
        spec!(
            "command_palette",
            "Command palette",
            ["secondary-p", "secondary-shift-p"],
            [target!(TogglePalette)]
        ),
        spec!("open_settings", "Open settings", ["secondary-,"], [target!(ToggleSettings)]),
        spec!("new_tab", "New scratchpad tab", ["secondary-t"], [target!(NewScratchTab)]),
        spec!("close_tab", "Close scratchpad tab", ["secondary-w"], [target!(CloseScratchTab)]),
        spec!(
            "prev_tab",
            "Previous scratchpad tab",
            // Shift+[ arrives as `{` on some layouts off macOS, so Ctrl+Shift+Tab
            // comes first there (it's the one shown in menus and hints).
            mac ["secondary-shift-[", "ctrl-shift-tab"],
            other ["ctrl-shift-tab", "secondary-shift-["],
            [target!(PrevScratchTab)]
        ),
        spec!(
            "next_tab",
            "Next scratchpad tab",
            mac ["secondary-shift-]", "ctrl-tab"],
            other ["ctrl-tab", "secondary-shift-]"],
            [target!(NextScratchTab)]
        ),
        spec!("open_folder", "Open local folder", ["secondary-o"], [target!(OpenDirectory)]),
        spec!("open_playground", "Open playground", ["secondary-shift-o"], [target!(OpenPlayground)]),
        spec!("ssh_connections", "SSH connections", ["secondary-k"], [target!(ToggleSshModal)]),
        spec!(
            "toggle_ai_sidebar",
            "Toggle AI assistant sidebar",
            ["secondary-i"],
            [target!(ToggleAiSidebar)]
        ),
        spec!(
            "toggle_explorer",
            "Toggle project explorer",
            ["secondary-b"],
            [target!(ToggleSidebar)]
        ),
        spec!("toggle_logs", "Toggle logs viewer", ["secondary-l"], [target!(ToggleLogs)]),
        spec!("toggle_sql", "Toggle inline SQL cards", ["secondary-s"], [target!(ToggleSql)]),
        spec!("mode_detail_dive", "Detail Dive view", ["secondary-1"], [target!(ModeDetailDive)]),
        spec!("mode_table", "Toggle table preview", ["secondary-2"], [target!(ModeTable)]),
        spec!("mode_cli", "Toggle CLI output", ["secondary-3"], [target!(ModeRawCli)]),
        spec!(
            "focus_search",
            "Focus search (settings, logs, SSH)",
            ["secondary-f"],
            [
                target!(FocusLogSearch, "LogViewer"),
                target!(FocusSettingsSearch, "SettingsModal"),
                target!(FocusSshSearch, "SshModal"),
            ]
        ),
        spec!(
            "trigger_autocomplete",
            "Trigger autocomplete",
            // Input-method switchers (fcitx, some Windows IMEs) take Ctrl+Space.
            mac ["ctrl-space"],
            other ["ctrl-space", "alt-/"],
            [target!(crate::editor::TriggerCompletion, "CodeEditor")]
        ),
        spec!(
            "submit_ai_prompt",
            "Send AI prompt",
            ["enter", "secondary-enter"],
            [target!(SubmitAiPrompt, "AiComposer")],
            bare Enter
        ),
        spec!(
            "graph_zoom_in",
            "Zoom object graph in",
            ["secondary-=", "secondary-+", "="],
            [target!(GraphZoomIn, "ObjectGraph")],
            bare Any
        ),
        spec!(
            "graph_zoom_out",
            "Zoom object graph out",
            ["secondary--", "-"],
            [target!(GraphZoomOut, "ObjectGraph")],
            bare Any
        ),
        spec!(
            "graph_fit",
            "Reset object graph zoom",
            ["secondary-0", "0"],
            [target!(GraphFitView, "ObjectGraph")],
            bare Any
        ),
        spec!("quit", "Quit Laralah", ["secondary-q"], [target!(QuitApp)]),
    ];
    SPECS
}

/// Shortcuts that are listed for reference but cannot be rebound.
pub fn fixed_shortcut_rows() -> &'static [(&'static str, &'static str)] {
    // Alternatives are separated by " / " and rendered as separate keycaps.
    &[
        ("Close modal or sheet", "Esc"),
        ("Copy / Cut / Paste", "⌘C / ⌘X / ⌘V"),
        ("Undo / Redo", "⌘Z / ⇧⌘Z"),
        ("Select all", "⌘A"),
    ]
}

pub fn spec_by_id(id: &str) -> Option<&'static ShortcutSpec> {
    shortcut_specs().iter().find(|s| s.id == id)
}

/// The keystrokes currently in effect for `spec`.
pub fn effective_keys<'a>(
    spec: &'a ShortcutSpec,
    overrides: &'a HashMap<String, Vec<String>>,
) -> Vec<&'a str> {
    match overrides.get(spec.id) {
        Some(keys) => keys.iter().map(String::as_str).collect(),
        None => spec.defaults.to_vec(),
    }
}

pub fn is_customized(spec: &ShortcutSpec, overrides: &HashMap<String, Vec<String>>) -> bool {
    overrides
        .get(spec.id)
        .is_some_and(|keys| !keys.iter().map(String::as_str).eq(spec.defaults.iter().copied()))
}

/// Other shortcuts that already use `keystroke` in an overlapping key context.
/// An unparsable `keystroke` is never bound, so it conflicts with nothing.
pub fn conflicts(
    spec: &ShortcutSpec,
    keystroke: &str,
    overrides: &HashMap<String, Vec<String>>,
) -> Vec<&'static ShortcutSpec> {
    let Ok(wanted) = Keystroke::parse(keystroke) else {
        return Vec::new();
    };
    shortcut_specs()
        .iter()
        .filter(|other| other.id != spec.id && spec.shares_context_with(other))
        .filter(|other| {
            effective_keys(other, overrides)
                .iter()
                .any(|k| Keystroke::parse(k).is_ok_and(|ks| ks == wanted))
        })
        .collect()
}

/// Fixed text-editing bindings from `editor::bind_editor_keys` (label, keys).
/// Inside a text input they take precedence over a customizable shortcut on
/// the same keys, because their key context is more specific.
fn fixed_text_bindings() -> &'static [(&'static str, &'static str)] {
    const COMMON: &[(&str, &str)] = &[
        ("Select all", "secondary-a"),
        ("Copy", "secondary-c"),
        ("Cut", "secondary-x"),
        ("Paste", "secondary-v"),
        ("Undo", "secondary-z"),
        ("Redo", "secondary-shift-z"),
        ("Redo", "secondary-y"),
    ];
    #[cfg(target_os = "macos")]
    const PLATFORM: &[(&str, &str)] = &[
        ("Delete to line start", "cmd-backspace"),
        ("Line start", "cmd-left"),
        ("Line end", "cmd-right"),
        ("Document start", "cmd-up"),
        ("Document end", "cmd-down"),
        ("Select to line start", "cmd-shift-left"),
        ("Select to line end", "cmd-shift-right"),
        ("Select to document start", "cmd-shift-up"),
        ("Select to document end", "cmd-shift-down"),
    ];
    #[cfg(not(target_os = "macos"))]
    const PLATFORM: &[(&str, &str)] = &[
        ("Delete word", "ctrl-backspace"),
        ("Word left", "ctrl-left"),
        ("Word right", "ctrl-right"),
        ("Select word left", "ctrl-shift-left"),
        ("Select word right", "ctrl-shift-right"),
        ("Document start", "ctrl-home"),
        ("Document end", "ctrl-end"),
        ("Select to document start", "ctrl-shift-home"),
        ("Select to document end", "ctrl-shift-end"),
        ("Cut", "shift-delete"),
        ("Copy", "ctrl-insert"),
        ("Paste", "shift-insert"),
    ];
    static ALL: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    ALL.get_or_init(|| COMMON.iter().chain(PLATFORM).copied().collect())
}

/// Everything `keystroke` would clash with for `spec`, as labels for the UI:
/// other customizable shortcuts, then fixed text-editing keys.
pub fn conflict_labels(
    spec: &ShortcutSpec,
    keystroke: &str,
    overrides: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut labels: Vec<String> = conflicts(spec, keystroke, overrides)
        .into_iter()
        .map(|other| other.label.to_string())
        .collect();
    if spec.reaches_text_input()
        && let Ok(wanted) = Keystroke::parse(keystroke)
    {
        labels.extend(
            fixed_text_bindings()
                .iter()
                .filter(|(_, keys)| Keystroke::parse(keys).is_ok_and(|ks| ks == wanted))
                .map(|(label, _)| format!("{label} in text fields")),
        );
    }
    labels
}

/// Snapshot of the overrides used to build the current keymap, for UI hints.
fn current_overrides() -> &'static RwLock<HashMap<String, Vec<String>>> {
    static CURRENT: OnceLock<RwLock<HashMap<String, Vec<String>>>> = OnceLock::new();
    CURRENT.get_or_init(Default::default)
}

/// Rebuilds the whole keymap: fixed editor/text bindings, then the customizable ones.
pub fn apply(cx: &mut App, overrides: &HashMap<String, Vec<String>>) {
    *current_overrides().write().unwrap_or_else(|e| e.into_inner()) = overrides.clone();

    cx.clear_key_bindings();
    crate::editor::bind_editor_keys(cx);
    crate::selectable::bind_selectable_keys(cx);

    let mut bindings = vec![
        KeyBinding::new("escape", CloseOverlay, None),
        KeyBinding::new("up", PaletteUp, Some("CommandPalette")),
        KeyBinding::new("down", PaletteDown, Some("CommandPalette")),
        KeyBinding::new("enter", PaletteConfirm, Some("CommandPalette")),
    ];
    for spec in shortcut_specs() {
        for key in effective_keys(spec, overrides) {
            for target in spec.targets {
                let context = target
                    .context
                    .and_then(|c| gpui::KeyBindingContextPredicate::parse(c).ok())
                    .map(std::rc::Rc::new);
                // A hand-edited settings file may hold an unparsable keystroke; skip it.
                if let Ok(binding) = KeyBinding::load(
                    key,
                    (target.build)(),
                    context,
                    false,
                    None,
                    &gpui::DummyKeyboardMapper,
                ) {
                    bindings.push(binding);
                }
            }
        }
    }
    cx.bind_keys(bindings);

    // Native menu key equivalents are read from the keymap when menus are set.
    #[cfg(target_os = "macos")]
    crate::app_menu::install_native(cx);
}

/// Display label of the first keystroke bound to the shortcut `id` (e.g. `⌘R` or
/// `Ctrl+R`), or an empty string when it is unbound.
pub fn hint(id: &str) -> &'static str {
    let Some(spec) = spec_by_id(id) else {
        return "";
    };
    let overrides = current_overrides().read().unwrap_or_else(|e| e.into_inner());
    let label = effective_keys(spec, &overrides)
        .first()
        .map(|k| display(k))
        .unwrap_or_default();
    intern(label)
}

/// Like [`hint`], but wraps the label into `template`'s `{}` placeholder; when the
/// shortcut is unbound the placeholder and surrounding parentheses are dropped.
pub fn hint_in(id: &str, template: &str) -> &'static str {
    let key = hint(id);
    let text = if key.is_empty() {
        template.replace(" ({})", "").replace("{}", "")
    } else {
        template.replace("{}", key)
    };
    intern(text)
}

/// Hint labels are used where `&'static str` is required; each distinct label is
/// leaked once, so the total stays bounded by the shortcuts the user has picked.
fn intern(s: String) -> &'static str {
    static CACHE: OnceLock<Mutex<std::collections::HashSet<&'static str>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = cache.get(s.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(s.into_boxed_str());
    cache.insert(leaked);
    leaked
}

/// Human-readable label for a gpui keystroke string, e.g. `secondary-shift-p` →
/// `⇧⌘P` on macOS or `Ctrl+Shift+P` elsewhere.
pub fn display(keystrokes: &str) -> String {
    keystrokes
        .split_whitespace()
        .map(|part| match Keystroke::parse(part) {
            Ok(ks) => display_keystroke(&ks),
            Err(_) => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_keystroke(ks: &Keystroke) -> String {
    let m = &ks.modifiers;
    let key = display_key(&ks.key);
    if cfg!(target_os = "macos") {
        let mut out = String::new();
        if m.control {
            out.push('⌃');
        }
        if m.alt {
            out.push('⌥');
        }
        if m.shift {
            out.push('⇧');
        }
        if m.platform {
            out.push('⌘');
        }
        if m.function {
            out.push_str("fn ");
        }
        out.push_str(&key);
        out
    } else {
        let mut parts: Vec<String> = Vec::new();
        if m.control {
            parts.push("Ctrl".into());
        }
        if m.alt {
            parts.push("Alt".into());
        }
        if m.shift {
            parts.push("Shift".into());
        }
        if m.platform {
            parts.push(if cfg!(target_os = "windows") { "Win" } else { "Super" }.into());
        }
        if m.function {
            parts.push("Fn".into());
        }
        parts.push(key);
        parts.join("+")
    }
}

fn display_key(key: &str) -> String {
    let mac = cfg!(target_os = "macos");
    match key {
        "space" => "Space".into(),
        "enter" => if mac { "↩" } else { "Enter" }.into(),
        "escape" => "Esc".into(),
        "tab" => "Tab".into(),
        "backspace" => if mac { "⌫" } else { "Backspace" }.into(),
        "delete" => if mac { "⌦" } else { "Delete" }.into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k if k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit()) => k.to_uppercase(),
        k => {
            let mut chars = k.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

/// Why a pressed key can't be used as a shortcut.
#[derive(Debug, PartialEq, Eq)]
pub enum RecordError {
    /// Only a modifier was pressed; keep waiting for the real key.
    ModifierOnly,
    /// A bare key would swallow normal typing.
    NeedsModifier,
    /// Only Enter may be used without a modifier here.
    NeedsModifierOrEnter,
}

/// Turns a pressed key into a portable keystroke string for `spec`.
pub fn record(spec: &ShortcutSpec, ks: &Keystroke) -> Result<String, RecordError> {
    let key = ks.key.as_str();
    if key.is_empty()
        || matches!(key, "shift" | "control" | "alt" | "platform" | "function" | "capslock")
    {
        return Err(RecordError::ModifierOnly);
    }
    let m = &ks.modifiers;
    let is_function_key = key.len() > 1
        && key.starts_with('f')
        && key[1..].chars().all(|c| c.is_ascii_digit());
    if !(m.control || m.alt || m.platform || is_function_key) {
        match spec.bare {
            BareKeys::Any => {}
            BareKeys::Enter if key == "enter" => {}
            BareKeys::Enter => return Err(RecordError::NeedsModifierOrEnter),
            BareKeys::None => return Err(RecordError::NeedsModifier),
        }
    }

    // `secondary` is Cmd on macOS and Ctrl elsewhere.
    let (secondary, other) = if cfg!(target_os = "macos") {
        (m.platform, m.control)
    } else {
        (m.control, m.platform)
    };
    let mut parts: Vec<&str> = Vec::new();
    if other {
        parts.push(if cfg!(target_os = "macos") { "ctrl" } else { "cmd" });
    }
    if m.alt {
        parts.push("alt");
    }
    if m.shift {
        parts.push("shift");
    }
    if m.function {
        parts.push("fn");
    }
    if secondary {
        parts.push("secondary");
    }
    parts.push(key);
    Ok(parts.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for spec in shortcut_specs() {
            assert!(ids.insert(spec.id), "duplicate id {}", spec.id);
            for key in spec.defaults {
                assert!(Keystroke::parse(key).is_ok(), "bad default {key} for {}", spec.id);
            }
        }
    }

    #[test]
    fn overrides_replace_defaults_and_empty_unbinds() {
        let spec = spec_by_id("run_code").unwrap();
        let mut overrides = HashMap::new();
        assert_eq!(effective_keys(spec, &overrides), vec!["secondary-r"]);
        assert!(!is_customized(spec, &overrides));

        overrides.insert("run_code".to_string(), vec!["secondary-enter".to_string()]);
        assert_eq!(effective_keys(spec, &overrides), vec!["secondary-enter"]);
        assert!(is_customized(spec, &overrides));

        overrides.insert("run_code".to_string(), vec![]);
        assert!(effective_keys(spec, &overrides).is_empty());
    }

    #[test]
    fn records_portable_keystrokes_and_rejects_bare_keys() {
        let spec = spec_by_id("run_code").unwrap();
        let secondary = if cfg!(target_os = "macos") { "cmd-shift-e" } else { "ctrl-shift-e" };
        let ks = Keystroke::parse(secondary).unwrap();
        assert_eq!(record(spec, &ks).unwrap(), "shift-secondary-e");

        let bare = Keystroke::parse("e").unwrap();
        assert_eq!(record(spec, &bare), Err(RecordError::NeedsModifier));
        assert!(record(spec, &Keystroke::parse("f5").unwrap()).is_ok());

        let graph = spec_by_id("graph_zoom_in").unwrap();
        assert_eq!(record(graph, &bare).unwrap(), "e");

        // The AI composer is a text input: bare letters must keep typing.
        let ai = spec_by_id("submit_ai_prompt").unwrap();
        assert_eq!(record(ai, &bare), Err(RecordError::NeedsModifierOrEnter));
        assert_eq!(record(ai, &Keystroke::parse("space").unwrap()), Err(RecordError::NeedsModifierOrEnter));
        assert_eq!(record(ai, &Keystroke::parse("enter").unwrap()).unwrap(), "enter");
        assert_eq!(record(ai, &Keystroke::parse("shift-enter").unwrap()).unwrap(), "shift-enter");
    }

    #[test]
    fn detects_conflicts_across_shared_contexts() {
        let run = spec_by_id("run_code").unwrap();
        let overrides = HashMap::new();
        let hits = conflicts(run, "secondary-l", &overrides);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "toggle_logs");
        assert!(conflicts(run, "secondary-shift-y", &overrides).is_empty());

        // Graph-only keys don't clash with each other's distinct contexts... but do
        // with the global ones.
        let fit = spec_by_id("graph_fit").unwrap();
        assert!(conflicts(fit, "secondary-r", &overrides).iter().any(|s| s.id == "run_code"));

        // Fixed editor keys win inside text inputs, so they're reported too; the
        // object graph has no text input, so it may reuse them.
        assert_eq!(conflict_labels(run, "secondary-c", &overrides), ["Copy in text fields"]);
        assert!(conflict_labels(fit, "secondary-c", &overrides).is_empty());

        // Unparsable keys (e.g. from a hand-edited settings file) are never bound.
        let mut bad = HashMap::new();
        bad.insert("toggle_logs".to_string(), vec!["x-y".to_string()]);
        assert!(Keystroke::parse("x-y").is_err());
        assert!(conflicts(run, "x-y", &bad).is_empty());
    }

    #[test]
    fn platform_specific_defaults() {
        let format = spec_by_id("format_code").unwrap();
        let next = spec_by_id("next_tab").unwrap();
        let complete = spec_by_id("trigger_autocomplete").unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(format.defaults, ["secondary-shift-f", "alt-shift-f"]);
            assert_eq!(next.defaults[0], "secondary-shift-]");
            assert_eq!(complete.defaults, ["ctrl-space"]);
        } else {
            assert_eq!(format.defaults, ["secondary-shift-f"]);
            assert_eq!(next.defaults[0], "ctrl-tab");
            assert_eq!(complete.defaults, ["ctrl-space", "alt-/"]);
        }
        // Each platform's set has no clashes within shared contexts, nor with
        // the fixed text-editing keys.
        let overrides = HashMap::new();
        for spec in shortcut_specs() {
            for key in spec.defaults {
                let hits = conflict_labels(spec, key, &overrides);
                assert!(hits.is_empty(), "{} {key} clashes with {hits:?}", spec.id);
            }
        }
        for (_, key) in fixed_text_bindings() {
            assert!(Keystroke::parse(key).is_ok(), "bad fixed binding {key}");
        }
    }

    #[test]
    fn displays_keystrokes_per_platform() {
        if cfg!(target_os = "macos") {
            assert_eq!(display("secondary-shift-p"), "⇧⌘P");
            assert_eq!(display("ctrl-space"), "⌃Space");
            assert_eq!(display("secondary-,"), "⌘,");
        } else {
            assert_eq!(display("secondary-shift-p"), "Ctrl+Shift+P");
            assert_eq!(display("ctrl-space"), "Ctrl+Space");
        }
    }
}
