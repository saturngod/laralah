use crate::laravel_lsp::{
    CompletionPopupState, LaravelLanguageService, insert_use_import,
};
use crate::models::EditorDiagnostic;
use crate::settings::AppSettings;
use gpui::{
    actions, anchored, deferred, div, fill, point, prelude::*, px, relative, size, svg, App, Bounds,
    ClipboardItem, ContentMask, Context, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, Font, FontFeatures,
    FontStyle, FontWeight, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use crate::theme::{rgb, rgba};
use std::ops::Range;
use std::sync::Arc;

actions!(
    editor,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteToLineStart,
        Enter,
        Indent,
        Outdent,
        FocusNextField,
        FocusPrevField,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        LineStart,
        LineEnd,
        DocStart,
        DocEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectLineStart,
        SelectLineEnd,
        SelectDocStart,
        SelectDocEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        TriggerCompletion,
        DismissCompletion,
        DismissContextMenu,
    ]
);

pub fn bind_editor_keys(cx: &mut App) {
    // `secondary` is Cmd on macOS and Ctrl on Windows/Linux. Plain `cmd` maps
    // to the Windows/Super key off macOS, so clipboard and undo shortcuts must
    // use `secondary` to work everywhere.
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("EditorInput")),
        KeyBinding::new("delete", Delete, Some("EditorInput")),
        KeyBinding::new("alt-backspace", DeleteWordLeft, Some("EditorInput")),
        KeyBinding::new("enter", Enter, Some("CodeEditor")),
        KeyBinding::new("tab", Indent, Some("CodeEditor")),
        KeyBinding::new("shift-tab", Outdent, Some("CodeEditor")),
        KeyBinding::new("tab", FocusNextField, Some("EditorInput && !CodeEditor")),
        KeyBinding::new("shift-tab", FocusPrevField, Some("EditorInput && !CodeEditor")),
        KeyBinding::new("left", Left, Some("EditorInput")),
        KeyBinding::new("right", Right, Some("EditorInput")),
        KeyBinding::new("up", Up, Some("CodeEditor")),
        KeyBinding::new("down", Down, Some("CodeEditor")),
        KeyBinding::new("alt-left", WordLeft, Some("EditorInput")),
        KeyBinding::new("alt-right", WordRight, Some("EditorInput")),
        KeyBinding::new("home", LineStart, Some("EditorInput")),
        KeyBinding::new("end", LineEnd, Some("EditorInput")),
        KeyBinding::new("shift-left", SelectLeft, Some("EditorInput")),
        KeyBinding::new("shift-right", SelectRight, Some("EditorInput")),
        KeyBinding::new("shift-up", SelectUp, Some("CodeEditor")),
        KeyBinding::new("shift-down", SelectDown, Some("CodeEditor")),
        KeyBinding::new("alt-shift-left", SelectWordLeft, Some("EditorInput")),
        KeyBinding::new("alt-shift-right", SelectWordRight, Some("EditorInput")),
        KeyBinding::new("shift-home", SelectLineStart, Some("EditorInput")),
        KeyBinding::new("shift-end", SelectLineEnd, Some("EditorInput")),
        KeyBinding::new("secondary-a", SelectAll, Some("EditorInput")),
        KeyBinding::new("secondary-c", Copy, Some("EditorInput")),
        KeyBinding::new("secondary-x", Cut, Some("EditorInput")),
        KeyBinding::new("secondary-v", Paste, Some("EditorInput")),
        KeyBinding::new("secondary-z", Undo, Some("EditorInput")),
        KeyBinding::new("secondary-shift-z", Redo, Some("EditorInput")),
        KeyBinding::new("secondary-y", Redo, Some("EditorInput")),
        KeyBinding::new("ctrl-space", TriggerCompletion, Some("CodeEditor")),
        KeyBinding::new("escape", DismissCompletion, Some("CompletionOpen")),
        KeyBinding::new("escape", DismissContextMenu, Some("ContextMenuOpen")),
    ]);

    // Line/document navigation follows each platform's own conventions.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-backspace", DeleteToLineStart, Some("EditorInput")),
        KeyBinding::new("cmd-left", LineStart, Some("EditorInput")),
        KeyBinding::new("cmd-right", LineEnd, Some("EditorInput")),
        KeyBinding::new("cmd-up", DocStart, Some("CodeEditor")),
        KeyBinding::new("cmd-down", DocEnd, Some("CodeEditor")),
        KeyBinding::new("cmd-shift-left", SelectLineStart, Some("EditorInput")),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, Some("EditorInput")),
        KeyBinding::new("cmd-shift-up", SelectDocStart, Some("CodeEditor")),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, Some("CodeEditor")),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, Some("EditorInput")),
        KeyBinding::new("ctrl-left", WordLeft, Some("EditorInput")),
        KeyBinding::new("ctrl-right", WordRight, Some("EditorInput")),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some("EditorInput")),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some("EditorInput")),
        KeyBinding::new("ctrl-home", DocStart, Some("CodeEditor")),
        KeyBinding::new("ctrl-end", DocEnd, Some("CodeEditor")),
        KeyBinding::new("ctrl-shift-home", SelectDocStart, Some("CodeEditor")),
        KeyBinding::new("ctrl-shift-end", SelectDocEnd, Some("CodeEditor")),
        KeyBinding::new("shift-delete", Cut, Some("EditorInput")),
        KeyBinding::new("ctrl-insert", Copy, Some("EditorInput")),
        KeyBinding::new("shift-insert", Paste, Some("EditorInput")),
    ]);
}

/// Commands offered by the editor right-click menu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl EditCommand {
    fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
        }
    }

    #[cfg(target_os = "macos")]
    fn shortcut(self) -> &'static str {
        match self {
            Self::Undo => "⌘Z",
            Self::Redo => "⇧⌘Z",
            Self::Cut => "⌘X",
            Self::Copy => "⌘C",
            Self::Paste => "⌘V",
            Self::SelectAll => "⌘A",
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn shortcut(self) -> &'static str {
        match self {
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Y",
            Self::Cut => "Ctrl+X",
            Self::Copy => "Ctrl+C",
            Self::Paste => "Ctrl+V",
            Self::SelectAll => "Ctrl+A",
        }
    }
}

/// An open right-click menu: where it was opened (window coordinates) and
/// which entries are enabled, as `None` for a separator.
struct ContextMenuState {
    position: Point<Pixels>,
    entries: Vec<Option<(EditCommand, bool)>>,
}

trait ContextMenuHost: Sized + 'static {
    fn run_edit_command(&mut self, cmd: EditCommand, window: &mut Window, cx: &mut Context<Self>);
    fn close_context_menu(&mut self, cx: &mut Context<Self>);
}

fn clipboard_has_text(cx: &App) -> bool {
    cx.read_from_clipboard()
        .and_then(|c| c.text())
        .is_some_and(|t| !t.is_empty())
}

fn render_context_menu<V: ContextMenuHost>(
    menu: &ContextMenuState,
    cx: &mut Context<V>,
) -> gpui::AnyElement {
    let items = menu.entries.iter().enumerate().map(|(idx, entry)| {
        let Some((cmd, enabled)) = *entry else {
            return div()
                .h(px(1.0))
                .my(px(4.0))
                .mx(px(8.0))
                .bg(rgb(0xe2e8f0))
                .into_any_element();
        };
        div()
            .id(SharedString::from(format!("editor-context-item-{idx}")))
            .h(px(26.0))
            .mx(px(4.0))
            .px(px(10.0))
            .rounded_sm()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(24.0))
            .text_color(if enabled { rgb(0x0f172a) } else { rgb(0xcbd5e1) })
            .when(enabled, |el| {
                el.cursor_pointer()
                    .hover(|s| s.bg(gpui::rgb(0x2f81f7)).text_color(gpui::rgb(0xffffff)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.close_context_menu(cx);
                            this.run_edit_command(cmd, window, cx);
                        }),
                    )
            })
            .when(!enabled, |el| {
                el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            })
            .child(cmd.label())
            .child(div().opacity(0.6).child(cmd.shortcut()))
            .into_any_element()
    });

    deferred(
        anchored()
            .position(menu.position)
            .snap_to_window_with_margin(px(8.0))
            .child(
                div()
                    .id("editor-context-menu")
                    .min_w(px(190.0))
                    .py(px(4.0))
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(0xe2e8f0))
                    .rounded_md()
                    .shadow_lg()
                    .text_sm()
                    .cursor_default()
                    .flex()
                    .flex_col()
                    .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.close_context_menu(cx);
                    }))
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .children(items),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

const GUTTER_WIDTH: f32 = 46.0;
const TEXT_LEFT_PAD: f32 = 12.0;
const TEXT_RIGHT_PAD: f32 = 36.0;
const TOP_PAD: f32 = 14.0;
const LINE_HEIGHT: f32 = 24.0;
const FONT_SIZE: f32 = 13.5;
const H_SCROLLBAR_THICKNESS: f32 = 12.0;
const V_SCROLLBAR_THICKNESS: f32 = 12.0;
const SCROLLBAR_MIN_THUMB: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollbarAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug)]
struct ScrollbarDragState {
    axis: ScrollbarAxis,
    grab_offset: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScrollbarMetrics {
    h_track: Option<Bounds<Pixels>>,
    v_track: Option<Bounds<Pixels>>,
    max_scroll_x: f32,
    max_scroll_y: f32,
    usable_code_w: f32,
    usable_code_h: f32,
}

#[derive(Clone)]
struct CachedLine {
    start_byte: usize,
    end_byte: usize,
    y_top: f32,
    row_span: usize,
    shaped: ShapedLine,
}

pub struct CodeEditor {
    focus_handle: FocusHandle,
    text: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    preferred_col: Option<usize>,
    undo_stack: Vec<(String, Range<usize>)>,
    redo_stack: Vec<(String, Range<usize>)>,
    scroll_x: f32,
    scroll_y: f32,
    pending_ensure_cursor_visible: bool,
    scrollbar_metrics: ScrollbarMetrics,
    hovered_scrollbar: Option<ScrollbarAxis>,
    scrollbar_drag: Option<ScrollbarDragState>,
    is_selecting: bool,
    last_bounds: Option<Bounds<Pixels>>,
    last_lines: Vec<CachedLine>,
    diagnostic: Option<EditorDiagnostic>,
    lsp: Option<Arc<LaravelLanguageService>>,
    completion_popup: Option<CompletionPopupState>,
    context_menu: Option<ContextMenuState>,
    show_line_numbers: bool,
    indentation_size: usize,
    show_inline_errors: bool,
    history_limit: usize,
    font_family: SharedString,
    font_size: f32,
    line_height: f32,
    pub edit_version: u64,
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CodeEditor {
    pub fn new(initial_text: impl Into<String>, cx: &mut Context<Self>) -> Self {
        let text = initial_text.into().replace("\r\n", "\n").replace('\r', "\n");
        let len = text.len();
        Self {
            focus_handle: cx.focus_handle(),
            text,
            selected_range: len..len,
            selection_reversed: false,
            marked_range: None,
            preferred_col: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            pending_ensure_cursor_visible: false,
            scrollbar_metrics: ScrollbarMetrics::default(),
            hovered_scrollbar: None,
            scrollbar_drag: None,
            is_selecting: false,
            last_bounds: None,
            last_lines: Vec::new(),
            diagnostic: None,
            lsp: None,
            completion_popup: None,
            context_menu: None,
            show_line_numbers: true,
            indentation_size: 4,
            show_inline_errors: true,
            history_limit: 150,
            font_family: "Menlo".into(),
            font_size: FONT_SIZE,
            line_height: LINE_HEIGHT,
            edit_version: 0,
        }
    }

    pub fn apply_settings(&mut self, settings: &AppSettings, cx: &mut Context<Self>) {
        self.show_line_numbers = settings.show_line_numbers;
        self.indentation_size = settings.indentation_size.clamp(1, 8);
        self.show_inline_errors = settings.show_inline_errors;
        self.history_limit = settings.history_entries.clamp(10, 2000);
        self.font_family = SharedString::from(settings.font_family.clone());
        self.font_size = settings.font_size.clamp(10.0, 28.0);
        self.line_height = (self.font_size * settings.line_height_ratio.clamp(1.2, 2.5)).max(16.0);
        cx.notify();
    }

    pub fn selected_text(&self) -> Option<&str> {
        if self.selected_range.is_empty() {
            return None;
        }
        let slice = &self.text[self.selected_range.clone()];
        if slice.trim().is_empty() {
            None
        } else {
            Some(slice)
        }
    }

    fn gutter_width(&self) -> f32 {
        if self.show_line_numbers {
            GUTTER_WIDTH
        } else {
            16.0
        }
    }

    pub fn set_language_service(
        &mut self,
        lsp: Arc<LaravelLanguageService>,
        cx: &mut Context<Self>,
    ) {
        self.lsp = Some(lsp);
        self.completion_popup = None;
        cx.notify();
    }

    /// Scans the buffer for unimported Models, Mailables, Project Classes, or Facades
    /// (such as `Article::...` or `new InvoiceShipped(...)`) and automatically inserts `use ...;`
    /// statements at the top of the file while preserving cursor position.
    pub fn apply_auto_imports(&mut self, cx: &mut Context<Self>) -> Vec<String> {
        let Some(ref lsp) = self.lsp else {
            return Vec::new();
        };
        let cur = self.cursor_offset();
        let (updated, new_cur, added) = lsp.auto_import_missing(&self.text, cur);
        if !added.is_empty() && updated != self.text {
            self.push_undo();
            self.text = updated;
            let clamped = clamp_char_boundary(&self.text, new_cur);
            self.selected_range = clamped..clamped;
            self.selection_reversed = false;
            self.ensure_cursor_visible();
            cx.notify();
        }
        added
    }

    fn refresh_completions(&mut self) {
        if !self.selected_range.is_empty() {
            self.completion_popup = None;
            return;
        }
        let Some(ref lsp) = self.lsp else {
            self.completion_popup = None;
            return;
        };
        self.completion_popup = lsp.complete(&self.text, self.cursor_offset());
    }

    fn accept_completion(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(popup) = self.completion_popup.take() else {
            return;
        };
        let Some(item) = popup.items.get(idx).cloned() else {
            return;
        };

        let start = clamp_char_boundary(&self.text, popup.replace_range.start);
        let end = clamp_char_boundary(&self.text, popup.replace_range.end).max(start);

        self.push_undo();
        self.text.replace_range(start..end, &item.insert_text);
        let mut new_cursor = start + item.insert_text.len();

        if let Some(ref fqcn) = item.auto_import_fqcn {
            let (with_import, shifted_cursor) = insert_use_import(&self.text, new_cursor, fqcn);
            self.text = with_import;
            new_cursor = shifted_cursor;
        }

        let clamped = clamp_char_boundary(&self.text, new_cursor);
        self.selected_range = clamped..clamped;
        self.selection_reversed = false;
        self.marked_range = None;
        self.preferred_col = None;
        if item.insert_text.ends_with("::") || item.insert_text.ends_with("->") {
            self.refresh_completions();
        }
        self.notify_text_edited(cx);
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, new_text: impl Into<String>, cx: &mut Context<Self>) {
        let normalized = new_text.into().replace("\r\n", "\n").replace('\r', "\n");
        self.push_undo();
        let end = normalized.len();
        self.text = normalized;
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.preferred_col = None;
        self.completion_popup = None;
        self.diagnostic = None;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.pending_ensure_cursor_visible = false;
        self.scrollbar_drag = None;
        cx.notify();
    }

    pub fn set_text_and_reveal(&mut self, new_text: impl Into<String>, cx: &mut Context<Self>) {
        let normalized = new_text.into().replace("\r\n", "\n").replace('\r', "\n");
        self.push_undo();
        let end = normalized.len();
        self.text = normalized;
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.preferred_col = None;
        self.completion_popup = None;
        self.scrollbar_drag = None;
        self.notify_text_edited(cx);
    }

    pub fn undo_once(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((prev_text, prev_sel)) = self.undo_stack.pop() {
            self.redo_stack
                .push((self.text.clone(), self.selected_range.clone()));
            self.text = prev_text;
            let start = clamp_char_boundary(&self.text, prev_sel.start);
            let end = clamp_char_boundary(&self.text, prev_sel.end);
            self.selected_range = start..end;
            self.marked_range = None;
            self.notify_text_edited(cx);
            true
        } else {
            false
        }
    }

    pub fn set_diagnostic(
        &mut self,
        diag: Option<EditorDiagnostic>,
        cx: &mut Context<Self>,
    ) {
        if self.diagnostic != diag {
            self.diagnostic = diag;
            cx.notify();
        }
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn push_undo(&mut self) {
        if self
            .undo_stack
            .last()
            .map(|(t, _)| t == &self.text)
            .unwrap_or(false)
        {
            return;
        }
        let limit = self.history_limit.max(10);
        if self.undo_stack.len() >= limit {
            self.undo_stack.remove(0);
        }
        self.undo_stack
            .push((self.text.clone(), self.selected_range.clone()));
        self.redo_stack.clear();
    }

    fn notify_text_edited(&mut self, cx: &mut Context<Self>) {
        // Never show syntax/parse errors while typing; clear any prior run error on edit.
        self.diagnostic = None;
        self.edit_version = self.edit_version.wrapping_add(1);
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn ensure_cursor_visible(&mut self) {
        self.pending_ensure_cursor_visible = true;
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let viewport_h = if self.scrollbar_metrics.usable_code_h > 1.0 {
            self.scrollbar_metrics.usable_code_h
        } else {
            f32::from(bounds.size.height)
        };
        let lh = self.line_height;
        let cur = self.cursor_offset();
        let (line_idx, col_chars) = byte_to_line_col(&self.text, cur);

        if viewport_h > lh {
            let cursor_top = self
                .last_lines
                .get(line_idx)
                .map(|l| l.y_top)
                .unwrap_or(TOP_PAD + (line_idx as f32) * lh);
            let max_y = self
                .scrollbar_metrics
                .max_scroll_y
                .max(cursor_top + lh + 64.0 - viewport_h);
            self.scroll_y =
                adjust_scroll_y_for_cursor(self.scroll_y, cursor_top, lh, viewport_h, max_y);
        }

        let gw = self.gutter_width();
        let usable_w = if self.scrollbar_metrics.usable_code_w > 1.0 {
            self.scrollbar_metrics.usable_code_w
        } else {
            (f32::from(bounds.size.width) - gw).max(40.0)
        };
        let approx_char_w = self.font_size * 0.6;
        let cursor_rel_x = self
            .last_lines
            .get(line_idx)
            .and_then(|l| {
                if cur >= l.start_byte && cur <= l.end_byte {
                    Some(f32::from(l.shaped.x_for_index(cur - l.start_byte)))
                } else {
                    None
                }
            })
            .unwrap_or((col_chars as f32) * approx_char_w);
        let max_x = self
            .scrollbar_metrics
            .max_scroll_x
            .max(TEXT_LEFT_PAD + cursor_rel_x + TEXT_RIGHT_PAD - usable_w);
        self.scroll_x = adjust_scroll_x_for_cursor(self.scroll_x, cursor_rel_x, usable_w, max_x);
    }

    fn move_to(&mut self, offset: usize, keep_preferred_col: bool, cx: &mut Context<Self>) {
        let clamped = clamp_char_boundary(&self.text, offset);
        self.selected_range = clamped..clamped;
        self.selection_reversed = false;
        self.completion_popup = None;
        if !keep_preferred_col {
            self.preferred_col = None;
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, keep_preferred_col: bool, cx: &mut Context<Self>) {
        let clamped = clamp_char_boundary(&self.text, offset);
        if self.selection_reversed {
            self.selected_range.start = clamped;
        } else {
            self.selected_range.end = clamped;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        if !keep_preferred_col {
            self.preferred_col = None;
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn edit_range(&mut self, range: Range<usize>, replacement: &str, cx: &mut Context<Self>) {
        let start = clamp_char_boundary(&self.text, range.start);
        let end = clamp_char_boundary(&self.text, range.end).max(start);
        self.push_undo();
        self.text.replace_range(start..end, replacement);
        let new_cursor = start + replacement.len();
        self.selected_range = new_cursor..new_cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.preferred_col = None;
        self.notify_text_edited(cx);
    }

    // ---- Actions ----

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = prev_char_boundary(&self.text, self.cursor_offset());
            self.move_to(prev, false, cx);
        } else {
            self.move_to(self.selected_range.start, false, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = next_char_boundary(&self.text, self.cursor_offset());
            self.move_to(next, false, cx);
        } else {
            self.move_to(self.selected_range.end, false, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref mut popup) = self.completion_popup
            && !popup.items.is_empty()
        {
            if popup.selected_idx == 0 {
                popup.selected_idx = popup.items.len() - 1;
            } else {
                popup.selected_idx -= 1;
            }
            cx.notify();
            return;
        }
        let target = self.vertical_move_offset(-1);
        self.move_to(target, true, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref mut popup) = self.completion_popup
            && !popup.items.is_empty()
        {
            popup.selected_idx = (popup.selected_idx + 1) % popup.items.len();
            cx.notify();
            return;
        }
        let target = self.vertical_move_offset(1);
        self.move_to(target, true, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let prev = prev_char_boundary(&self.text, self.cursor_offset());
        self.select_to(prev, false, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let next = next_char_boundary(&self.text, self.cursor_offset());
        self.select_to(next, false, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_move_offset(-1);
        self.select_to(target, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_move_offset(1);
        self.select_to(target, true, cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = prev_word_boundary(&self.text, self.cursor_offset());
        self.move_to(target, false, cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_word_boundary(&self.text, self.cursor_offset());
        self.move_to(target, false, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = prev_word_boundary(&self.text, self.cursor_offset());
        self.select_to(target, false, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_word_boundary(&self.text, self.cursor_offset());
        self.select_to(target, false, cx);
    }

    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let target = smart_line_start(&self.text, self.cursor_offset());
        self.move_to(target, false, cx);
    }

    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let target = line_end_offset(&self.text, self.cursor_offset());
        self.move_to(target, false, cx);
    }

    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let target = smart_line_start(&self.text, self.cursor_offset());
        self.select_to(target, false, cx);
    }

    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let target = line_end_offset(&self.text, self.cursor_offset());
        self.select_to(target, false, cx);
    }

    fn doc_start(&mut self, _: &DocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
    }

    fn doc_end(&mut self, _: &DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), false, cx);
    }

    fn select_doc_start(&mut self, _: &SelectDocStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, false, cx);
    }

    fn select_doc_end(&mut self, _: &SelectDocEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.len(), false, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.text.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.edit_range(self.selected_range.clone(), "", cx);
        } else {
            let cur = self.cursor_offset();
            if cur > 0 {
                let prev = prev_char_boundary(&self.text, cur);
                self.edit_range(prev..cur, "", cx);
            }
        }
        self.refresh_completions();
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.edit_range(self.selected_range.clone(), "", cx);
        } else {
            let cur = self.cursor_offset();
            if cur < self.text.len() {
                let next = next_char_boundary(&self.text, cur);
                self.edit_range(cur..next, "", cx);
            }
        }
        self.refresh_completions();
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.edit_range(self.selected_range.clone(), "", cx);
        } else {
            let cur = self.cursor_offset();
            let prev = prev_word_boundary(&self.text, cur);
            if prev < cur {
                self.edit_range(prev..cur, "", cx);
            }
        }
        self.refresh_completions();
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.edit_range(self.selected_range.clone(), "", cx);
        } else {
            let cur = self.cursor_offset();
            let start = line_start_offset(&self.text, cur);
            if start < cur {
                self.edit_range(start..cur, "", cx);
            }
        }
        self.refresh_completions();
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref popup) = self.completion_popup {
            let idx = popup.selected_idx;
            self.accept_completion(idx, cx);
            return;
        }
        let start = self.selected_range.start;
        let lstart = line_start_offset(&self.text, start);
        let line_prefix = &self.text[lstart..start];
        let indent: String = line_prefix.chars().take_while(|c| *c == ' ').collect();
        let trimmed = line_prefix.trim_end();
        let indent_unit = " ".repeat(self.indentation_size);
        let extra = if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
            indent_unit.as_str()
        } else {
            ""
        };
        let insert = format!("\n{indent}{extra}");
        self.edit_range(self.selected_range.clone(), &insert, cx);
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref popup) = self.completion_popup {
            let idx = popup.selected_idx;
            self.accept_completion(idx, cx);
            return;
        }
        let indent_unit = " ".repeat(self.indentation_size);
        if self.selected_range.is_empty()
            || !self.text[self.selected_range.clone()].contains('\n')
        {
            self.edit_range(self.selected_range.clone(), &indent_unit, cx);
            return;
        }
        // Multi-line block indent
        let block_start = line_start_offset(&self.text, self.selected_range.start);
        let block_end = line_end_offset(&self.text, self.selected_range.end);
        let indented = self.text[block_start..block_end]
            .split('\n')
            .map(|l| format!("{indent_unit}{l}"))
            .collect::<Vec<_>>()
            .join("\n");
        self.push_undo();
        self.text.replace_range(block_start..block_end, &indented);
        self.selected_range = block_start..block_start + indented.len();
        self.notify_text_edited(cx);
    }

    fn trigger_completion(&mut self, _: &TriggerCompletion, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_completions();
        cx.notify();
    }

    fn dismiss_completion(&mut self, _: &DismissCompletion, _: &mut Window, cx: &mut Context<Self>) {
        if self.completion_popup.take().is_some() {
            cx.notify();
        }
    }

    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        let indent_sz = self.indentation_size;
        let block_start = line_start_offset(&self.text, self.selected_range.start);
        let block_end = line_end_offset(&self.text, self.selected_range.end);
        let outdented = self.text[block_start..block_end]
            .split('\n')
            .map(|l| {
                let spaces = l.chars().take_while(|c| *c == ' ').count().min(indent_sz);
                &l[spaces..]
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.push_undo();
        self.text.replace_range(block_start..block_end, &outdented);
        self.selected_range = block_start..block_start + outdented.len();
        self.notify_text_edited(cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let slice = if !self.selected_range.is_empty() {
            &self.text[self.selected_range.clone()]
        } else {
            let cur = self.cursor_offset();
            let s = line_start_offset(&self.text, cur);
            let e = line_end_with_newline(&self.text, cur);
            &self.text[s..e]
        };
        if !slice.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let range = if !self.selected_range.is_empty() {
            self.selected_range.clone()
        } else {
            let cur = self.cursor_offset();
            line_start_offset(&self.text, cur)..line_end_with_newline(&self.text, cur)
        };
        if !range.is_empty() {
            let copied = self.text[range.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(copied));
            self.edit_range(range, "", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(clip) = cx.read_from_clipboard().and_then(|c| c.text()) {
            let normalized = clip.replace("\r\n", "\n").replace('\r', "\n");
            self.edit_range(self.selected_range.clone(), &normalized, cx);
            self.apply_auto_imports(cx);
            self.completion_popup = None;
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((prev_text, prev_sel)) = self.undo_stack.pop() {
            self.redo_stack
                .push((self.text.clone(), self.selected_range.clone()));
            self.text = prev_text;
            let start = clamp_char_boundary(&self.text, prev_sel.start);
            let end = clamp_char_boundary(&self.text, prev_sel.end);
            self.selected_range = start..end;
            self.marked_range = None;
            self.notify_text_edited(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((next_text, next_sel)) = self.redo_stack.pop() {
            self.undo_stack
                .push((self.text.clone(), self.selected_range.clone()));
            self.text = next_text;
            let start = clamp_char_boundary(&self.text, next_sel.start);
            let end = clamp_char_boundary(&self.text, next_sel.end);
            self.selected_range = start..end;
            self.marked_range = None;
            self.notify_text_edited(cx);
        }
    }

    fn vertical_move_offset(&mut self, delta_lines: isize) -> usize {
        let ranges = line_byte_ranges(&self.text);
        if ranges.is_empty() {
            return 0;
        }
        let cur = self.cursor_offset();
        let (cur_line, cur_col) = byte_to_line_col(&self.text, cur);
        let desired_col = *self.preferred_col.get_or_insert(cur_col);

        if delta_lines < 0 && cur_line == 0 {
            return 0;
        }
        let next_line = (cur_line as isize + delta_lines).clamp(0, ranges.len() as isize - 1) as usize;
        if delta_lines > 0 && cur_line == ranges.len() - 1 {
            return self.text.len();
        }
        let (ls, le) = ranges[next_line];
        let line_str = &self.text[ls..le];
        ls + char_col_to_byte(line_str, desired_col)
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds else {
            return self.cursor_offset();
        };
        if self.last_lines.is_empty() {
            return 0;
        }

        let doc_y = f32::from(position.y - bounds.top()) + self.scroll_y;
        if doc_y < 0.0 {
            return 0;
        }
        if let Some(last) = self.last_lines.last()
            && doc_y > last.y_top + (last.row_span as f32) * self.line_height
        {
            return self.text.len();
        }
        let line_idx = if doc_y < TOP_PAD {
            0
        } else {
            self.last_lines
                .iter()
                .rposition(|cl| doc_y >= cl.y_top)
                .unwrap_or(0)
        };

        let cached = &self.last_lines[line_idx];
        let gutter_right = bounds.left() + px(self.gutter_width());
        if !self.is_selecting && position.x < gutter_right {
            return clamp_char_boundary(&self.text, cached.start_byte);
        }
        let text_left = gutter_right + px(TEXT_LEFT_PAD - self.scroll_x);
        let rel_x = position.x - text_left;
        let local_idx = if rel_x <= px(0.0) {
            0
        } else if rel_x >= cached.shaped.width {
            cached.end_byte - cached.start_byte
        } else {
            cached
                .shaped
                .closest_index_for_x(rel_x)
                .min(cached.end_byte - cached.start_byte)
        };
        clamp_char_boundary(&self.text, cached.start_byte + local_idx)
    }

    fn drag_select_to_point(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(bounds) = self.last_bounds {
            let bottom_edge = if self.scrollbar_metrics.h_track.is_some() {
                bounds.bottom() - px(H_SCROLLBAR_THICKNESS)
            } else {
                bounds.bottom()
            };
            if position.y < bounds.top() && self.scroll_y > 0.0 {
                let delta = f32::from(bounds.top() - position.y).clamp(4.0, 36.0);
                self.scroll_y = (self.scroll_y - delta).max(0.0);
            } else if position.y > bottom_edge {
                let max_scroll_y = self.scrollbar_metrics.max_scroll_y;
                let delta = f32::from(position.y - bottom_edge).clamp(4.0, 36.0);
                self.scroll_y = (self.scroll_y + delta).min(max_scroll_y);
            }

            let code_left = bounds.left() + px(self.gutter_width());
            let code_right = if self.scrollbar_metrics.v_track.is_some() {
                bounds.right() - px(V_SCROLLBAR_THICKNESS)
            } else {
                bounds.right()
            };
            if position.x < code_left && self.scroll_x > 0.0 {
                let delta = f32::from(code_left - position.x).clamp(4.0, 42.0);
                self.scroll_x = (self.scroll_x - delta).max(0.0);
            } else if position.x > code_right {
                let max_scroll_x = self.scrollbar_metrics.max_scroll_x;
                let delta = f32::from(position.x - code_right).clamp(4.0, 42.0);
                self.scroll_x = (self.scroll_x + delta).min(max_scroll_x);
            }
        }
        let idx = self.index_for_mouse_position(position);
        self.select_to(idx, false, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus_handle);

        // Hit-test horizontal scrollbar track/thumb
        if let Some(h_track) = self.scrollbar_metrics.h_track
            && h_track.contains(&event.position)
        {
            let track_start = f32::from(h_track.left());
            let track_len = f32::from(h_track.size.width);
            let (thumb_start, thumb_len) = compute_scrollbar_thumb_1d(
                track_start,
                track_len,
                self.scroll_x,
                self.scrollbar_metrics.max_scroll_x,
                self.scrollbar_metrics.usable_code_w,
            );
            let mouse_x = f32::from(event.position.x);
            let grab_offset = if mouse_x >= thumb_start && mouse_x <= thumb_start + thumb_len {
                mouse_x - thumb_start
            } else {
                let centered = thumb_len * 0.5;
                self.scroll_x = scroll_from_thumb_drag_1d(
                    track_start,
                    track_len,
                    thumb_len,
                    mouse_x,
                    centered,
                    self.scrollbar_metrics.max_scroll_x,
                );
                centered
            };
            self.scrollbar_drag = Some(ScrollbarDragState {
                axis: ScrollbarAxis::Horizontal,
                grab_offset,
            });
            self.is_selecting = false;
            cx.notify();
            return;
        }

        // Hit-test vertical scrollbar track/thumb
        if let Some(v_track) = self.scrollbar_metrics.v_track
            && v_track.contains(&event.position)
        {
            let track_start = f32::from(v_track.top());
            let track_len = f32::from(v_track.size.height);
            let (thumb_start, thumb_len) = compute_scrollbar_thumb_1d(
                track_start,
                track_len,
                self.scroll_y,
                self.scrollbar_metrics.max_scroll_y,
                self.scrollbar_metrics.usable_code_h,
            );
            let mouse_y = f32::from(event.position.y);
            let grab_offset = if mouse_y >= thumb_start && mouse_y <= thumb_start + thumb_len {
                mouse_y - thumb_start
            } else {
                let centered = thumb_len * 0.5;
                self.scroll_y = scroll_from_thumb_drag_1d(
                    track_start,
                    track_len,
                    thumb_len,
                    mouse_y,
                    centered,
                    self.scrollbar_metrics.max_scroll_y,
                );
                centered
            };
            self.scrollbar_drag = Some(ScrollbarDragState {
                axis: ScrollbarAxis::Vertical,
                grab_offset,
            });
            self.is_selecting = false;
            cx.notify();
            return;
        }

        // Ignore clicks inside bottom-right scrollbar intersection corner when both scrollbars exist
        if self.scrollbar_metrics.h_track.is_some()
            && self.scrollbar_metrics.v_track.is_some()
            && let Some(bounds) = self.last_bounds
            && event.position.x >= bounds.right() - px(V_SCROLLBAR_THICKNESS)
            && event.position.y >= bounds.bottom() - px(H_SCROLLBAR_THICKNESS)
        {
            return;
        }

        let idx = self.index_for_mouse_position(event.position);

        if event.click_count >= 3 {
            let s = line_start_offset(&self.text, idx);
            let e = line_end_with_newline(&self.text, idx);
            self.selected_range = s..e;
            self.selection_reversed = false;
            self.is_selecting = false;
            cx.notify();
        } else if event.click_count == 2 {
            let (s, e) = word_range_at(&self.text, idx);
            self.selected_range = s..e;
            self.selection_reversed = false;
            self.is_selecting = false;
            cx.notify();
        } else if event.modifiers.shift {
            self.is_selecting = true;
            self.select_to(idx, false, cx);
        } else {
            self.is_selecting = true;
            self.move_to(idx, false, cx);
        }
    }

    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(drag) = self.scrollbar_drag {
            if event.pressed_button != Some(MouseButton::Left) {
                self.scrollbar_drag = None;
                cx.notify();
                return;
            }
            match drag.axis {
                ScrollbarAxis::Horizontal => {
                    if let Some(h_track) = self.scrollbar_metrics.h_track {
                        let track_start = f32::from(h_track.left());
                        let track_len = f32::from(h_track.size.width);
                        let (_, thumb_len) = compute_scrollbar_thumb_1d(
                            track_start,
                            track_len,
                            self.scroll_x,
                            self.scrollbar_metrics.max_scroll_x,
                            self.scrollbar_metrics.usable_code_w,
                        );
                        let next_x = scroll_from_thumb_drag_1d(
                            track_start,
                            track_len,
                            thumb_len,
                            f32::from(event.position.x),
                            drag.grab_offset,
                            self.scrollbar_metrics.max_scroll_x,
                        );
                        if (next_x - self.scroll_x).abs() > 0.01 {
                            self.scroll_x = next_x;
                            cx.notify();
                        }
                    }
                }
                ScrollbarAxis::Vertical => {
                    if let Some(v_track) = self.scrollbar_metrics.v_track {
                        let track_start = f32::from(v_track.top());
                        let track_len = f32::from(v_track.size.height);
                        let (_, thumb_len) = compute_scrollbar_thumb_1d(
                            track_start,
                            track_len,
                            self.scroll_y,
                            self.scrollbar_metrics.max_scroll_y,
                            self.scrollbar_metrics.usable_code_h,
                        );
                        let next_y = scroll_from_thumb_drag_1d(
                            track_start,
                            track_len,
                            thumb_len,
                            f32::from(event.position.y),
                            drag.grab_offset,
                            self.scrollbar_metrics.max_scroll_y,
                        );
                        if (next_y - self.scroll_y).abs() > 0.01 {
                            self.scroll_y = next_y;
                            cx.notify();
                        }
                    }
                }
            }
            return;
        }

        if self.is_selecting {
            if event.pressed_button == Some(MouseButton::Left) {
                self.drag_select_to_point(event.position, cx);
            } else {
                self.is_selecting = false;
            }
            return;
        }

        let new_hover = if self
            .last_bounds
            .is_some_and(|b| b.contains(&event.position))
        {
            if self
                .scrollbar_metrics
                .h_track
                .is_some_and(|t| t.contains(&event.position))
            {
                Some(ScrollbarAxis::Horizontal)
            } else if self
                .scrollbar_metrics
                .v_track
                .is_some_and(|t| t.contains(&event.position))
            {
                Some(ScrollbarAxis::Vertical)
            } else {
                None
            }
        } else {
            None
        };

        if new_hover != self.hovered_scrollbar {
            self.hovered_scrollbar = new_hover;
            cx.notify();
        }
    }

    fn handle_mouse_up(&mut self, cx: &mut Context<Self>) {
        let was_dragging_sb = self.scrollbar_drag.take().is_some();
        self.is_selecting = false;
        if was_dragging_sb {
            cx.notify();
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lh = self.line_height;
        let (raw_dx, raw_dy) = match event.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
            ScrollDelta::Lines(l) => (l.x * lh, l.y * lh),
        };
        let (dx, dy) = resolve_wheel_deltas(raw_dx, raw_dy, event.modifiers.shift);
        if dx.abs() < 0.01 && dy.abs() < 0.01 {
            return;
        }
        cx.stop_propagation();

        let max_scroll_x = self.scrollbar_metrics.max_scroll_x;
        let max_scroll_y = if self.last_bounds.is_some() {
            self.scrollbar_metrics.max_scroll_y
        } else {
            let line_count = self.text.split('\n').count().max(1);
            let content_h = TOP_PAD * 2.0 + (line_count as f32) * lh + 64.0;
            let viewport_h = f32::from(window.viewport_size().height);
            (content_h - viewport_h).max(0.0)
        };

        let next_x = (self.scroll_x - dx).clamp(0.0, max_scroll_x);
        let next_y = (self.scroll_y - dy).clamp(0.0, max_scroll_y);
        if (next_x - self.scroll_x).abs() > 0.01 || (next_y - self.scroll_y).abs() > 0.01 {
            self.scroll_x = next_x;
            self.scroll_y = next_y;
            cx.notify();
        }
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_from_utf16(&self.text, &range_utf16);
        actual_range.replace(range_to_utf16(&self.text, &range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: range_to_utf16(&self.text, &self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|r| range_to_utf16(&self.text, r))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.text, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let normalized = new_text.replace("\r\n", "\n").replace('\r', "\n");
        self.edit_range(range, &normalized, cx);

        // Automatically inject `use ...;` when user finishes typing `ClassName::` or `new ClassName(`
        let cur = self.cursor_offset();
        if (normalized == ":" && cur >= 2 && &self.text[cur - 2..cur] == "::")
            || normalized == "("
        {
            self.apply_auto_imports(cx);
        }

        self.refresh_completions();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.text, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());

        let normalized = new_text.replace("\r\n", "\n").replace('\r', "\n");
        let start = clamp_char_boundary(&self.text, range.start);
        let end = clamp_char_boundary(&self.text, range.end).max(start);
        self.text.replace_range(start..end, &normalized);

        if !normalized.is_empty() {
            self.marked_range = Some(start..start + normalized.len());
        } else {
            self.marked_range = None;
        }

        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r16| {
                let sub = &self.text[start..start + normalized.len()];
                let local = range_from_utf16(sub, r16);
                (start + local.start)..(start + local.end)
            })
            .unwrap_or_else(|| (start + normalized.len())..(start + normalized.len()));

        self.refresh_completions();
        self.notify_text_edited(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = range_from_utf16(&self.text, &range_utf16);
        let (line_idx, _) = byte_to_line_col(&self.text, range.start);
        let cached = self.last_lines.get(line_idx)?;
        let local = range.start.saturating_sub(cached.start_byte);
        let code_left = bounds.left() + px(self.gutter_width());
        let x = (code_left + px(TEXT_LEFT_PAD - self.scroll_x) + cached.shaped.x_for_index(local))
            .clamp(code_left, bounds.right());
        let y = bounds.top() + px(cached.y_top - self.scroll_y);
        Some(Bounds::new(point(x, y), size(px(2.0), px(self.line_height))))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let utf8_idx = self.index_for_mouse_position(point);
        Some(offset_to_utf16(&self.text, utf8_idx))
    }
}

impl Render for CodeEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_popup = self.completion_popup.is_some();
        let key_ctx = match (has_popup, self.context_menu.is_some()) {
            (_, true) => "CodeEditor EditorInput ContextMenuOpen",
            (true, false) => "CodeEditor EditorInput CompletionOpen",
            (false, false) => "CodeEditor EditorInput",
        };

        let popup_overlay = self.render_completion_popup(cx);
        let context_menu = self
            .context_menu
            .as_ref()
            .map(|menu| render_context_menu(menu, cx));
        let editor_cursor = if self.scrollbar_drag.is_some() || self.hovered_scrollbar.is_some() {
            CursorStyle::Arrow
        } else {
            CursorStyle::IBeam
        };

        div()
            .id("code-editor-surface")
            .relative()
            .size_full()
            .bg(rgb(0xffffff))
            .key_context(key_ctx)
            .track_focus(&self.focus_handle)
            .cursor(editor_cursor)
            .overflow_hidden()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::doc_start))
            .on_action(cx.listener(Self::doc_end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::select_doc_start))
            .on_action(cx.listener(Self::select_doc_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::trigger_completion))
            .on_action(cx.listener(Self::dismiss_completion))
            .on_action(cx.listener(|this, _: &DismissContextMenu, _, cx| {
                this.close_context_menu(cx)
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(CodeEditorElement {
                editor: cx.entity(),
            })
            .when_some(popup_overlay, |el, pop| el.child(pop))
            .when_some(context_menu, |el, menu| el.child(menu))
    }
}

impl CodeEditor {
    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus_handle);
        self.completion_popup = None;
        // Keep a selection the click lands inside; otherwise move the caret
        // to the click, like native editors do.
        let idx = self.index_for_mouse_position(event.position);
        let sel = &self.selected_range;
        if sel.is_empty() || idx < sel.start || idx > sel.end {
            self.selected_range = idx..idx;
            self.selection_reversed = false;
            self.preferred_col = None;
        }
        let has_sel = !self.selected_range.is_empty();
        self.context_menu = Some(ContextMenuState {
            position: event.position,
            entries: vec![
                Some((EditCommand::Undo, !self.undo_stack.is_empty())),
                Some((EditCommand::Redo, !self.redo_stack.is_empty())),
                None,
                Some((EditCommand::Cut, has_sel)),
                Some((EditCommand::Copy, has_sel)),
                Some((EditCommand::Paste, clipboard_has_text(cx))),
                None,
                Some((EditCommand::SelectAll, !self.text.is_empty())),
            ],
        });
        cx.notify();
    }
}

impl ContextMenuHost for CodeEditor {
    fn run_edit_command(&mut self, cmd: EditCommand, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            EditCommand::Undo => self.undo(&Undo, window, cx),
            EditCommand::Redo => self.redo(&Redo, window, cx),
            EditCommand::Cut => self.cut(&Cut, window, cx),
            EditCommand::Copy => self.copy(&Copy, window, cx),
            EditCommand::Paste => self.paste(&Paste, window, cx),
            EditCommand::SelectAll => self.select_all(&SelectAll, window, cx),
        }
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
        }
    }
}

impl CodeEditor {
    fn render_completion_popup(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        // Designed as a dark popup; keep its colors in both themes.
        let _scope = crate::theme::enter_scope(crate::theme::Scope::Fixed);
        let popup = self.completion_popup.as_ref()?;
        if popup.items.is_empty() {
            return None;
        }

        let anchor_byte = clamp_char_boundary(&self.text, popup.replace_range.start);
        let (line_idx, col_chars) = byte_to_line_col(&self.text, anchor_byte);
        let gw = self.gutter_width();
        let lh = self.line_height;
        let approx_x = gw
            + TEXT_LEFT_PAD
            - self.scroll_x
            + self
                .last_lines
                .get(line_idx)
                .map(|cl| {
                    let local = anchor_byte
                        .saturating_sub(cl.start_byte)
                        .min(cl.end_byte - cl.start_byte);
                    f32::from(cl.shaped.x_for_index(local))
                })
                .unwrap_or((col_chars as f32) * 8.1);

        let popup_width = 420.0_f32;
        let max_x = self
            .last_bounds
            .map(|b| (f32::from(b.size.width) - popup_width - 8.0).max(gw))
            .unwrap_or(approx_x);
        let left_px = px(approx_x.clamp(gw, max_x));

        let visible_rows = popup.items.len().min(11) as f32;
        let est_height = (visible_rows * 26.0 + 6.0).min(292.0);
        let line_y_top = self
            .last_lines
            .get(line_idx)
            .map(|cl| cl.y_top)
            .unwrap_or(TOP_PAD + (line_idx as f32) * lh);
        let below_y = line_y_top + lh - self.scroll_y + 2.0;
        let top_px = if let Some(bounds) = self.last_bounds {
            let editor_h = f32::from(bounds.size.height);
            if below_y + est_height > editor_h && line_y_top > est_height {
                let above_y = line_y_top - self.scroll_y - est_height - 2.0;
                px(above_y.max(4.0))
            } else {
                px(below_y.max(TOP_PAD + LINE_HEIGHT))
            }
        } else {
            px(below_y.max(TOP_PAD + LINE_HEIGHT))
        };

        let selected_idx = popup.selected_idx;

        Some(
            div()
                .id("lsp-completion-popup")
                .absolute()
                .left(left_px)
                .top(top_px)
                .w(px(popup_width))
                .max_h(px(292.0))
                .overflow_y_scroll()
                .bg(rgb(0x22272e))
                .border_1()
                .border_color(rgb(0x3b4252))
                .rounded_sm()
                .shadow_lg()
                .py(px(2.0))
                .flex()
                .flex_col()
                .cursor_default()
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .on_scroll_wheel(|_, _, cx| {
                    cx.stop_propagation();
                })
                .children(popup.items.iter().enumerate().map(|(idx, item)| {
                    let is_sel = idx == selected_idx;
                    let icon_col = if is_sel {
                        0xffffff
                    } else {
                        item.kind.color_hex()
                    };
                    let split_at = clamp_char_boundary(&item.label, item.matched_prefix_len);
                    let matched_prefix = item.label[..split_at].to_string();
                    let rest_label = item.label[split_at..].to_string();
                    let ns_suffix = item
                        .namespace_context
                        .as_ref()
                        .map(|ns| format!(" ({ns})"));

                    div()
                        .id(SharedString::from(format!("lsp-item-{idx}")))
                        .h(px(26.0))
                        .px(px(8.0))
                        .bg(if is_sel {
                            rgb(0x2f81f7)
                        } else {
                            rgba(0x00000000)
                        })
                        .when(!is_sel, |el| el.hover(|s| s.bg(rgb(0x2d333b))))
                        .cursor_pointer()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.accept_completion(idx, cx);
                            }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .w(px(14.0))
                                        .h(px(14.0))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            svg()
                                                .path(item.kind.icon_path())
                                                .size(px(13.0))
                                                .text_color(rgb(icon_col)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .font_family("Menlo")
                                        .text_xs()
                                        .when(!matched_prefix.is_empty(), |el| {
                                            el.child(
                                                div()
                                                    .flex_shrink_0()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(if is_sel {
                                                        rgb(0xffffff)
                                                    } else {
                                                        rgb(0x58a6ff)
                                                    })
                                                    .child(matched_prefix),
                                            )
                                        })
                                        .when(!rest_label.is_empty(), |el| {
                                            el.child(
                                                div()
                                                    .truncate()
                                                    .font_weight(if is_sel {
                                                        FontWeight::SEMIBOLD
                                                    } else {
                                                        FontWeight::NORMAL
                                                    })
                                                    .text_color(if is_sel {
                                                        rgb(0xffffff)
                                                    } else {
                                                        rgb(0xe6edf3)
                                                    })
                                                    .child(rest_label),
                                            )
                                        })
                                        .when_some(ns_suffix, |el, ns| {
                                            el.child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .truncate()
                                                    .text_color(if is_sel {
                                                        rgb(0xdbeafe)
                                                    } else {
                                                        rgb(0x9ca3af)
                                                    })
                                                    .child(ns),
                                            )
                                        }),
                                ),
                        )
                        .when(!item.detail.is_empty(), |el| {
                            el.child(
                                div()
                                    .flex_shrink_0()
                                    .max_w(px(140.0))
                                    .font_family("Menlo")
                                    .text_xs()
                                    .text_color(if is_sel {
                                        rgb(0xeff6ff)
                                    } else {
                                        rgb(0x9ca3af)
                                    })
                                    .truncate()
                                    .child(item.detail.clone()),
                            )
                        })
                }))
                .into_any_element(),
        )
    }
}

struct CodeEditorElement {
    editor: Entity<CodeEditor>,
}

struct EditorPrepaint {
    lines: Vec<CachedLine>,
    line_nums: Vec<(f32, ShapedLine)>,
    diag_texts: Vec<(Point<Pixels>, ShapedLine)>,
    gutter_diag_quads: Vec<PaintQuad>,
    overview_diag_quads: Vec<PaintQuad>,
    scrollbar_quads: Vec<PaintQuad>,
    selections: Vec<PaintQuad>,
    active_line_bg: Option<PaintQuad>,
    cursor_quad: Option<PaintQuad>,
    scroll_x: f32,
    scroll_y: f32,
    scrollbar_metrics: ScrollbarMetrics,
    gutter_width: f32,
    line_height: f32,
}

impl IntoElement for CodeEditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CodeEditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let ed = self.editor.read(cx);
        let text = ed.text.clone();
        let selected_range = ed.selected_range.clone();
        let cursor_offset = ed.cursor_offset();
        let mut scroll_x = ed.scroll_x;
        let mut scroll_y = ed.scroll_y;
        let pending_ensure_cursor = ed.pending_ensure_cursor_visible;
        let hovered_sb = ed.hovered_scrollbar;
        let dragging_sb = ed.scrollbar_drag.map(|d| d.axis);
        let diagnostic = ed.diagnostic.clone();
        let font_family = ed.font_family.clone();
        let font_size = ed.font_size;
        let line_height = ed.line_height;
        let gutter_width = ed.gutter_width();
        let show_line_numbers = ed.show_line_numbers;
        let show_inline_errors = ed.show_inline_errors;

        let mono_font = Font {
            family: font_family,
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };

        let ranges = line_byte_ranges(&text);
        let (cursor_line, _) = byte_to_line_col(&text, cursor_offset);
        let total_w = f32::from(bounds.size.width).max(1.0);
        let total_h = f32::from(bounds.size.height).max(1.0);
        let raw_code_w = (total_w - gutter_width).max(1.0);
        let doc_err_right_limit = (raw_code_w - 14.0).max(TEXT_LEFT_PAD + 80.0);

        let mut lines = Vec::with_capacity(ranges.len());
        let mut line_nums = Vec::with_capacity(ranges.len());
        let mut raw_diag_texts: Vec<(f32, f32, ShapedLine)> = Vec::new();
        let mut diag_rows: Vec<(f32, usize)> = Vec::new();

        let mut running_y_top = TOP_PAD;
        let mut max_line_right = TEXT_LEFT_PAD;

        // Pass 1: shape all lines, gutter numbers, and inline diagnostics
        for (idx, (start_byte, end_byte)) in ranges.into_iter().enumerate() {
            let line_str = &text[start_byte..end_byte];
            let line_diag = diagnostic.as_ref().filter(|d| d.line == idx + 1);

            let mut runs = highlight_php_line(line_str, &mono_font);
            if let Some(d) = line_diag {
                runs = apply_wavy_error_underline(runs, line_str, d.token_hint.as_deref());
            }

            let shaped = window.text_system().shape_line(
                SharedString::from(line_str.to_string()),
                px(font_size),
                &runs,
                None,
            );

            let y_top = running_y_top;
            let mut row_span = 1usize;
            let mut line_right = TEXT_LEFT_PAD + f32::from(shaped.width);

            if let Some(d) = line_diag {
                let wrapped = if show_inline_errors {
                    let first_doc_x =
                        (TEXT_LEFT_PAD + f32::from(shaped.width) + 9.0).max(TEXT_LEFT_PAD);
                    wrap_inline_error_rows(
                        &d.message,
                        first_doc_x,
                        TEXT_LEFT_PAD,
                        doc_err_right_limit,
                    )
                } else {
                    Vec::new()
                };
                if !wrapped.is_empty() {
                    row_span = wrapped.iter().map(|(r, _, _)| r + 1).max().unwrap_or(1);
                }
                diag_rows.push((y_top, row_span));

                for (r_idx, doc_rx_f, seg_text) in wrapped {
                    let err_run = TextRun {
                        len: seg_text.len(),
                        font: mono_font.clone(),
                        color: rgb(0xef4444).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped_err = window.text_system().shape_line(
                        SharedString::from(seg_text),
                        px(font_size),
                        &[err_run],
                        None,
                    );
                    line_right = line_right.max(doc_rx_f + f32::from(shaped_err.width));
                    let doc_ry = y_top + (r_idx as f32) * line_height;
                    raw_diag_texts.push((doc_rx_f, doc_ry, shaped_err));
                }
            } else {
                if show_line_numbers {
                    let num_str = format!("{}", idx + 1);
                    let num_color = if idx == cursor_line {
                        rgb(0x475569)
                    } else {
                        rgb(0x9ca3af)
                    };
                    let num_run = TextRun {
                        len: num_str.len(),
                        font: mono_font.clone(),
                        color: num_color.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped_num = window.text_system().shape_line(
                        SharedString::from(num_str),
                        px((font_size - 1.5).max(10.0)),
                        &[num_run],
                        None,
                    );
                    line_nums.push((y_top, shaped_num));
                }

            }

            max_line_right = max_line_right.max(line_right);
            lines.push(CachedLine {
                start_byte,
                end_byte,
                y_top,
                row_span,
                shaped,
            });
            running_y_top += (row_span as f32) * line_height;
        }

        // Resolve scrollbar presence & exact scroll limits
        let mut has_h = compute_max_scroll_x(max_line_right, raw_code_w) > 0.5;
        let has_v = compute_max_scroll_y(
            running_y_top,
            if has_h {
                (total_h - H_SCROLLBAR_THICKNESS).max(1.0)
            } else {
                total_h
            },
        ) > 0.5;
        if has_v && !has_h {
            has_h =
                compute_max_scroll_x(max_line_right, (raw_code_w - V_SCROLLBAR_THICKNESS).max(1.0))
                    > 0.5;
        }

        let usable_code_w = if has_v {
            (raw_code_w - V_SCROLLBAR_THICKNESS).max(1.0)
        } else {
            raw_code_w
        };
        let usable_code_h = if has_h {
            (total_h - H_SCROLLBAR_THICKNESS).max(1.0)
        } else {
            total_h
        };
        let max_scroll_x = compute_max_scroll_x(max_line_right, usable_code_w);
        let max_scroll_y = compute_max_scroll_y(running_y_top, usable_code_h);

        scroll_x = scroll_x.clamp(0.0, max_scroll_x);
        scroll_y = scroll_y.clamp(0.0, max_scroll_y);

        if pending_ensure_cursor
            && let Some(cl) = lines.get(cursor_line)
        {
            let c_local = cursor_offset
                .saturating_sub(cl.start_byte)
                .min(cl.end_byte - cl.start_byte);
            let cursor_rel_x = f32::from(cl.shaped.x_for_index(c_local));
            scroll_x =
                adjust_scroll_x_for_cursor(scroll_x, cursor_rel_x, usable_code_w, max_scroll_x);
            if usable_code_h > line_height {
                scroll_y = adjust_scroll_y_for_cursor(
                    scroll_y,
                    cl.y_top,
                    line_height,
                    usable_code_h,
                    max_scroll_y,
                );
            }
        }

        // Pass 2: build screen-positioned quads using finalized (scroll_x, scroll_y)
        let code_left = bounds.left() + px(gutter_width);
        let text_left = code_left + px(TEXT_LEFT_PAD - scroll_x);

        let mut selections = Vec::new();
        let mut cursor_quad = None;
        let mut active_line_bg = None;
        let mut gutter_diag_quads = Vec::new();
        let mut overview_diag_quads = Vec::new();

        for (idx, cl) in lines.iter().enumerate() {
            let y = bounds.top() + px(cl.y_top - scroll_y);
            let line_has_diag = diagnostic.as_ref().is_some_and(|d| d.line == idx + 1);

            if idx == cursor_line && selected_range.is_empty() {
                active_line_bg = Some(fill(
                    Bounds::new(
                        point(bounds.left(), y),
                        size(bounds.size.width, px(line_height)),
                    ),
                    if line_has_diag {
                        rgb(0xfef2f2)
                    } else {
                        rgb(0xf8fafc)
                    },
                ));
            }

            if !selected_range.is_empty()
                && selected_range.start <= cl.end_byte
                && selected_range.end >= cl.start_byte
                && selected_range.end != cl.start_byte
            {
                let line_len = cl.end_byte - cl.start_byte;
                let s_local = selected_range
                    .start
                    .saturating_sub(cl.start_byte)
                    .min(line_len);
                let e_local = selected_range
                    .end
                    .saturating_sub(cl.start_byte)
                    .min(line_len);
                let x0 = text_left + cl.shaped.x_for_index(s_local);
                let extra_nl = if selected_range.end > cl.end_byte {
                    px(7.0)
                } else {
                    px(0.0)
                };
                let x1 = (text_left + cl.shaped.x_for_index(e_local) + extra_nl).max(x0 + px(2.0));
                selections.push(fill(
                    Bounds::from_corners(point(x0, y), point(x1, y + px(line_height))),
                    rgba(0x3b82f62e),
                ));
            }

            if idx == cursor_line && selected_range.is_empty() {
                let c_local = cursor_offset
                    .saturating_sub(cl.start_byte)
                    .min(cl.end_byte - cl.start_byte);
                let cx_pos = text_left + cl.shaped.x_for_index(c_local);
                cursor_quad = Some(fill(
                    Bounds::new(
                        point(cx_pos, y + px(2.0)),
                        size(px(2.0), px(line_height - 4.0)),
                    ),
                    rgb(0x2563eb),
                ));
            }
        }

        for (y_top, row_span) in diag_rows {
            let y = bounds.top() + px(y_top - scroll_y);
            for r in 0..row_span {
                let ry = y + px((r as f32) * line_height);
                gutter_diag_quads.push(fill(
                    Bounds::new(
                        point(
                            bounds.left() + px(gutter_width * 0.5 - 2.5),
                            ry + px(line_height * 0.5 - 2.5),
                        ),
                        size(px(5.0), px(5.0)),
                    ),
                    rgb(0xef4444),
                ));
            }
            overview_diag_quads.push(fill(
                Bounds::new(
                    point(bounds.right() - px(3.5), y + px(2.0)),
                    size(px(3.5), px((row_span as f32) * line_height - 4.0)),
                ),
                rgb(0xef4444),
            ));
        }

        let diag_texts: Vec<(Point<Pixels>, ShapedLine)> = raw_diag_texts
            .into_iter()
            .map(|(doc_rx_f, doc_ry, shaped_err)| {
                (
                    point(
                        code_left + px(doc_rx_f - scroll_x),
                        bounds.top() + px(doc_ry - scroll_y),
                    ),
                    shaped_err,
                )
            })
            .collect();

        // Build horizontal & vertical scrollbar geometry and paint quads
        let mut scrollbar_quads = Vec::new();
        let mut h_track_bounds = None;
        let mut v_track_bounds = None;

        if has_h && usable_code_w > 16.0 && total_h > H_SCROLLBAR_THICKNESS + 8.0 {
            let track_left = f32::from(code_left);
            let track_top = f32::from(bounds.bottom()) - H_SCROLLBAR_THICKNESS;
            let track_w = usable_code_w;
            let track = Bounds::new(
                point(px(track_left), px(track_top)),
                size(px(track_w), px(H_SCROLLBAR_THICKNESS)),
            );
            let (thumb_x, thumb_w) = compute_scrollbar_thumb_1d(
                track_left,
                track_w,
                scroll_x,
                max_scroll_x,
                usable_code_w,
            );
            let is_active = dragging_sb == Some(ScrollbarAxis::Horizontal);
            let is_hover = hovered_sb == Some(ScrollbarAxis::Horizontal);
            let thumb_h = if is_active || is_hover { 8.0 } else { 6.0 };
            let thumb_y = track_top + (H_SCROLLBAR_THICKNESS - thumb_h) * 0.5;
            let thumb = Bounds::new(point(px(thumb_x), px(thumb_y)), size(px(thumb_w), px(thumb_h)));
            let thumb_col = if is_active {
                rgba(0x475569e6)
            } else if is_hover {
                rgba(0x64748bcc)
            } else {
                rgba(0x94a3b899)
            };

            scrollbar_quads.push(fill(track, rgba(0xf8fafce8)));
            scrollbar_quads.push(fill(
                Bounds::new(point(px(track_left), px(track_top)), size(px(track_w), px(1.0))),
                rgb(0xf1f5f9),
            ));
            scrollbar_quads.push(fill(thumb, thumb_col).corner_radii(px(thumb_h * 0.5)));

            h_track_bounds = Some(track);
        }

        if has_v && usable_code_h > 16.0 && raw_code_w > V_SCROLLBAR_THICKNESS + 8.0 {
            let track_left = f32::from(bounds.right()) - V_SCROLLBAR_THICKNESS;
            let track_top = f32::from(bounds.top());
            let track_h = usable_code_h;
            let track = Bounds::new(
                point(px(track_left), px(track_top)),
                size(px(V_SCROLLBAR_THICKNESS), px(track_h)),
            );
            let (thumb_y, thumb_h) = compute_scrollbar_thumb_1d(
                track_top,
                track_h,
                scroll_y,
                max_scroll_y,
                usable_code_h,
            );
            let is_active = dragging_sb == Some(ScrollbarAxis::Vertical);
            let is_hover = hovered_sb == Some(ScrollbarAxis::Vertical);
            let thumb_w = if is_active || is_hover { 8.0 } else { 6.0 };
            let thumb_x = track_left + (V_SCROLLBAR_THICKNESS - thumb_w) * 0.5;
            let thumb = Bounds::new(point(px(thumb_x), px(thumb_y)), size(px(thumb_w), px(thumb_h)));
            let thumb_col = if is_active {
                rgba(0x475569e6)
            } else if is_hover {
                rgba(0x64748bcc)
            } else {
                rgba(0x94a3b899)
            };

            scrollbar_quads.push(fill(track, rgba(0xf8fafce8)));
            scrollbar_quads.push(fill(
                Bounds::new(point(px(track_left), px(track_top)), size(px(1.0), px(track_h))),
                rgb(0xf1f5f9),
            ));
            scrollbar_quads.push(fill(thumb, thumb_col).corner_radii(px(thumb_w * 0.5)));

            v_track_bounds = Some(track);
        }

        if h_track_bounds.is_some() && v_track_bounds.is_some() {
            scrollbar_quads.push(fill(
                Bounds::new(
                    point(
                        bounds.right() - px(V_SCROLLBAR_THICKNESS),
                        bounds.bottom() - px(H_SCROLLBAR_THICKNESS),
                    ),
                    size(px(V_SCROLLBAR_THICKNESS), px(H_SCROLLBAR_THICKNESS)),
                ),
                rgba(0xf8fafce8),
            ));
        }

        let scrollbar_metrics = ScrollbarMetrics {
            h_track: h_track_bounds,
            v_track: v_track_bounds,
            max_scroll_x,
            max_scroll_y,
            usable_code_w,
            usable_code_h,
        };

        EditorPrepaint {
            lines,
            line_nums,
            diag_texts,
            gutter_diag_quads,
            overview_diag_quads,
            scrollbar_quads,
            selections,
            active_line_bg,
            cursor_quad,
            scroll_x,
            scroll_y,
            scrollbar_metrics,
            gutter_width,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        let is_focused = focus_handle.is_focused(window);
        if is_focused && let Some(bg) = prepaint.active_line_bg.take() {
            window.paint_quad(bg);
        }

        let lh = px(prepaint.line_height);
        let code_left = bounds.left() + px(prepaint.gutter_width);
        let text_left = code_left + px(TEXT_LEFT_PAD - prepaint.scroll_x);

        // Gutter line numbers & gutter red dots (horizontally pinned)
        for (y_top, shaped_num) in prepaint.line_nums.drain(..) {
            let y = bounds.top() + px(y_top - prepaint.scroll_y);
            if y + lh < bounds.top() - lh || y > bounds.bottom() + lh {
                continue;
            }
            let num_x = bounds.left() + px(prepaint.gutter_width - 10.0) - shaped_num.width;
            let _ = shaped_num.paint(point(num_x, y + px(1.0)), lh, window, cx);
        }

        for q in prepaint.gutter_diag_quads.drain(..) {
            window.paint_quad(q);
        }

        if prepaint.scroll_x > 0.5 {
            window.paint_quad(fill(
                Bounds::new(
                    point(code_left - px(1.0), bounds.top()),
                    size(px(1.0), bounds.size.height),
                ),
                rgb(0xe2e8f0),
            ));
        }

        // Horizontally & vertically clipped code content area
        let code_clip_bounds = Bounds::new(
            point(code_left, bounds.top()),
            size(
                (bounds.size.width - px(prepaint.gutter_width)).max(px(1.0)),
                bounds.size.height.max(px(1.0)),
            ),
        );

        window.with_content_mask(Some(ContentMask { bounds: code_clip_bounds }), |window| {
            for sel in prepaint.selections.drain(..) {
                window.paint_quad(sel);
            }

            for cached in &prepaint.lines {
                let y = bounds.top() + px(cached.y_top - prepaint.scroll_y);
                if y + lh < bounds.top() - lh || y > bounds.bottom() + lh {
                    continue;
                }
                let _ = cached.shaped.paint(point(text_left, y), lh, window, cx);
            }

            for (pos, err_line) in prepaint.diag_texts.drain(..) {
                if pos.y + lh < bounds.top() - lh || pos.y > bounds.bottom() + lh {
                    continue;
                }
                let _ = err_line.paint(pos, lh, window, cx);
            }

            if is_focused && let Some(cursor) = prepaint.cursor_quad.take() {
                window.paint_quad(cursor);
            }
        });

        for q in prepaint.overview_diag_quads.drain(..) {
            window.paint_quad(q);
        }

        for q in prepaint.scrollbar_quads.drain(..) {
            window.paint_quad(q);
        }

        let cached_lines = std::mem::take(&mut prepaint.lines);
        let final_scroll_x = prepaint.scroll_x;
        let final_scroll_y = prepaint.scroll_y;
        let final_metrics = prepaint.scrollbar_metrics;
        self.editor.update(cx, |ed, _| {
            ed.last_bounds = Some(bounds);
            ed.last_lines = cached_lines;
            ed.scroll_x = final_scroll_x;
            ed.scroll_y = final_scroll_y;
            ed.pending_ensure_cursor_visible = false;
            ed.scrollbar_metrics = final_metrics;
        });

        let ed_move = self.editor.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble {
                ed_move.update(cx, |ed, cx| {
                    ed.handle_mouse_move(event, cx);
                });
            }
        });

        let ed_up = self.editor.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble {
                ed_up.update(cx, |ed, cx| {
                    ed.handle_mouse_up(cx);
                });
            }
        });
    }
}

fn compute_max_scroll_x(max_line_right: f32, usable_code_w: f32) -> f32 {
    if max_line_right + 14.0 <= usable_code_w {
        0.0
    } else {
        (max_line_right + TEXT_RIGHT_PAD - usable_code_w).max(0.0)
    }
}

fn compute_max_scroll_y(running_y_top: f32, usable_code_h: f32) -> f32 {
    if running_y_top + TOP_PAD <= usable_code_h {
        0.0
    } else {
        (running_y_top + 64.0 - usable_code_h).max(0.0)
    }
}

fn resolve_wheel_deltas(dx: f32, dy: f32, shift_held: bool) -> (f32, f32) {
    if shift_held && dx.abs() < 0.01 && dy.abs() > 0.01 {
        (dy, 0.0)
    } else {
        (dx, dy)
    }
}

fn adjust_scroll_x_for_cursor(
    scroll_x: f32,
    cursor_rel_x: f32,
    usable_code_w: f32,
    max_scroll_x: f32,
) -> f32 {
    let doc_cursor_x = TEXT_LEFT_PAD + cursor_rel_x;
    let left_keep = TEXT_LEFT_PAD + 16.0;
    let right_keep = 28.0;
    let mut next = scroll_x;
    if doc_cursor_x < next + left_keep {
        next = (doc_cursor_x - left_keep).max(0.0);
    } else if doc_cursor_x + right_keep > next + usable_code_w {
        next = (doc_cursor_x + right_keep - usable_code_w).max(0.0);
    }
    next.clamp(0.0, max_scroll_x.max(0.0))
}

fn adjust_scroll_y_for_cursor(
    scroll_y: f32,
    cursor_top: f32,
    line_height: f32,
    usable_code_h: f32,
    max_scroll_y: f32,
) -> f32 {
    let cursor_bottom = cursor_top + line_height + 12.0;
    let mut next = scroll_y;
    if cursor_top < next + TOP_PAD {
        next = (cursor_top - TOP_PAD).max(0.0);
    } else if cursor_bottom > next + usable_code_h {
        next = (cursor_bottom - usable_code_h).max(0.0);
    }
    next.clamp(0.0, max_scroll_y.max(0.0))
}

fn compute_scrollbar_thumb_1d(
    track_start: f32,
    track_len: f32,
    scroll: f32,
    max_scroll: f32,
    viewport_len: f32,
) -> (f32, f32) {
    let inner_start = track_start + 2.0;
    let inner_len = (track_len - 4.0).max(4.0);
    let content_len = (viewport_len + max_scroll.max(0.0)).max(1.0);
    let thumb_len = ((viewport_len / content_len) * inner_len)
        .clamp(SCROLLBAR_MIN_THUMB.min(inner_len), inner_len);
    let movable = (inner_len - thumb_len).max(0.0);
    let ratio = if max_scroll > 0.0 {
        (scroll / max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (inner_start + ratio * movable, thumb_len)
}

fn scroll_from_thumb_drag_1d(
    track_start: f32,
    track_len: f32,
    thumb_len: f32,
    mouse_pos: f32,
    grab_offset: f32,
    max_scroll: f32,
) -> f32 {
    let inner_start = track_start + 2.0;
    let inner_len = (track_len - 4.0).max(4.0);
    let movable = (inner_len - thumb_len).max(0.0);
    if movable <= 0.0 || max_scroll <= 0.0 {
        return 0.0;
    }
    let desired_start = mouse_pos - grab_offset;
    let ratio = ((desired_start - inner_start) / movable).clamp(0.0, 1.0);
    ratio * max_scroll
}

fn apply_wavy_error_underline(
    runs: Vec<TextRun>,
    line_str: &str,
    token_hint: Option<&str>,
) -> Vec<TextRun> {
    if line_str.is_empty() {
        return runs;
    }

    let (err_start, err_end) = token_hint
        .filter(|h| !h.is_empty())
        .and_then(|hint| line_str.find(hint).map(|pos| (pos, pos + hint.len())))
        .unwrap_or_else(|| {
            let lead = line_str.len() - line_str.trim_start().len();
            let end = line_str.trim_end().len();
            if end > lead {
                (lead, end)
            } else {
                (0, line_str.len())
            }
        });

    if err_end <= err_start {
        return runs;
    }

    let wavy = UnderlineStyle {
        thickness: px(1.5),
        color: Some(rgb(0xef4444).into()),
        wavy: true,
    };

    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut cursor = 0usize;
    for run in runs {
        let r_start = cursor;
        let r_end = cursor + run.len;
        cursor = r_end;

        if r_end <= err_start || r_start >= err_end {
            out.push(run);
            continue;
        }

        if r_start < err_start {
            let mut before = run.clone();
            before.len = err_start - r_start;
            out.push(before);
        }

        let mid_start = r_start.max(err_start);
        let mid_end = r_end.min(err_end);
        if mid_end > mid_start {
            let mut mid = run.clone();
            mid.len = mid_end - mid_start;
            mid.underline = Some(wavy);
            out.push(mid);
        }

        if r_end > err_end {
            let mut after = run;
            after.len = r_end - err_end;
            out.push(after);
        }
    }

    out
}

fn wrap_inline_error_rows(
    msg: &str,
    first_x: f32,
    cont_x: f32,
    right_limit_x: f32,
) -> Vec<(usize, f32, String)> {
    const CHAR_ADVANCE: f32 = 8.2;
    let words: Vec<&str> = msg.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let mut rows: Vec<(usize, f32, String)> = Vec::new();
    let mut row_idx = 0usize;
    let mut cur_x = first_x;
    let mut cur_str = String::new();

    for word in words {
        let avail_chars = ((right_limit_x - cur_x).max(0.0) / CHAR_ADVANCE).floor() as usize;
        let needed = if cur_str.is_empty() {
            word.chars().count()
        } else {
            cur_str.chars().count() + 1 + word.chars().count()
        };

        if needed <= avail_chars {
            if !cur_str.is_empty() {
                cur_str.push(' ');
            }
            cur_str.push_str(word);
        } else {
            if !cur_str.is_empty() {
                rows.push((row_idx, cur_x, std::mem::take(&mut cur_str)));
            }
            row_idx += 1;
            cur_x = cont_x;
            cur_str.push_str(word);
        }
    }

    if !cur_str.is_empty() {
        rows.push((row_idx, cur_x, cur_str));
    }

    rows
}

// ==================== SINGLE-LINE TEXT FIELD ====================

pub struct TextField {
    focus_handle: FocusHandle,
    text: String,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    is_selecting: bool,
    scroll_x: f32,
    last_bounds: Option<Bounds<Pixels>>,
    last_layout: Option<ShapedLine>,
    text_color_hex: u32,
    masked: bool,
    context_menu: Option<ContextMenuState>,
    pub edit_version: u64,
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextField {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            text: String::new(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            is_selecting: false,
            scroll_x: 0.0,
            last_bounds: None,
            last_layout: None,
            text_color_hex: 0x0f172a,
            masked: false,
            context_menu: None,
            edit_version: 0,
        }
    }

    pub fn with_text_color(mut self, hex: u32) -> Self {
        self.text_color_hex = hex;
        self
    }

    pub fn with_masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    pub fn set_masked(&mut self, masked: bool, cx: &mut Context<Self>) {
        self.masked = masked;
        cx.notify();
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        let s = value.into().replace(['\r', '\n'], " ");
        let end = s.len();
        self.text = s;
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.is_selecting = false;
        self.scroll_x = 0.0;
        self.edit_version = self.edit_version.wrapping_add(1);
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn mapped_index(&self, byte_idx: usize) -> usize {
        if self.masked {
            let c = clamp_char_boundary(&self.text, byte_idx);
            self.text[..c].chars().count()
        } else {
            clamp_char_boundary(&self.text, byte_idx)
        }
    }

    fn sync_scroll_to_cursor(&mut self) {
        if self.text.is_empty() {
            self.scroll_x = 0.0;
            return;
        }
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return;
        };
        let viewport_w = f32::from(bounds.size.width).max(1.0);
        let line_w = f32::from(line.width);
        let max_scroll = (line_w - viewport_w + 8.0).max(0.0);
        let cur_x = f32::from(line.x_for_index(self.mapped_index(self.cursor_offset())));
        if cur_x < self.scroll_x + 4.0 {
            self.scroll_x = (cur_x - 4.0).max(0.0);
        } else if cur_x > self.scroll_x + viewport_w - 6.0 {
            self.scroll_x = (cur_x - viewport_w + 6.0).max(0.0);
        }
        self.scroll_x = self.scroll_x.clamp(0.0, max_scroll);
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let c = clamp_char_boundary(&self.text, offset);
        self.selected_range = c..c;
        self.selection_reversed = false;
        self.sync_scroll_to_cursor();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let c = clamp_char_boundary(&self.text, offset);
        if self.selection_reversed {
            self.selected_range.start = c;
        } else {
            self.selected_range.end = c;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.sync_scroll_to_cursor();
        cx.notify();
    }

    fn edit_range(&mut self, range: Range<usize>, replacement: &str, cx: &mut Context<Self>) {
        let start = clamp_char_boundary(&self.text, range.start);
        let end = clamp_char_boundary(&self.text, range.end).max(start);
        let clean = replacement.replace(['\r', '\n'], " ");
        self.text.replace_range(start..end, &clean);
        let pos = start + clean.len();
        self.selected_range = pos..pos;
        self.selection_reversed = false;
        self.marked_range = None;
        self.sync_scroll_to_cursor();
        self.edit_version = self.edit_version.wrapping_add(1);
        cx.notify();
    }

    fn index_for_x(&self, position: Point<Pixels>) -> usize {
        if self.text.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return self.text.len();
        };
        let rel_x = (position.x - bounds.left()) + px(self.scroll_x);
        if rel_x <= px(0.0) {
            return 0;
        }
        if rel_x >= line.width {
            return self.text.len();
        }
        let idx = line.closest_index_for_x(rel_x);
        if self.masked {
            char_col_to_byte(&self.text, idx)
        } else {
            clamp_char_boundary(&self.text, idx)
        }
    }

    fn drag_select_to_point(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) {
            let viewport_w = f32::from(bounds.size.width).max(1.0);
            let max_scroll = (f32::from(line.width) - viewport_w + 8.0).max(0.0);
            if position.x < bounds.left() && self.scroll_x > 0.0 {
                let step = f32::from(bounds.left() - position.x).clamp(6.0, 48.0);
                self.scroll_x = (self.scroll_x - step).max(0.0);
            } else if position.x > bounds.right() && self.scroll_x < max_scroll {
                let step = f32::from(position.x - bounds.right()).clamp(6.0, 48.0);
                self.scroll_x = (self.scroll_x + step).min(max_scroll);
            }
        }
        let idx = self.index_for_x(position);
        self.select_to(idx, cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(prev_char_boundary(&self.text, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(next_char_boundary(&self.text, self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }
    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(prev_word_boundary(&self.text, self.cursor_offset()), cx);
    }
    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(next_word_boundary(&self.text, self.cursor_offset()), cx);
    }
    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }
    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), cx);
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_char_boundary(&self.text, self.cursor_offset()), cx);
    }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_char_boundary(&self.text, self.cursor_offset()), cx);
    }
    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_word_boundary(&self.text, self.cursor_offset()), cx);
    }
    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_word_boundary(&self.text, self.cursor_offset()), cx);
    }
    fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }
    fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.len(), cx);
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.text.len();
        self.selection_reversed = false;
        cx.notify();
    }
    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.edit_range(self.selected_range.clone(), "", cx);
        } else {
            let c = self.cursor_offset();
            if c > 0 {
                self.edit_range(prev_char_boundary(&self.text, c)..c, "", cx);
            }
        }
    }
    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.edit_range(self.selected_range.clone(), "", cx);
        } else {
            let c = self.cursor_offset();
            if c < self.text.len() {
                self.edit_range(c..next_char_boundary(&self.text, c), "", cx);
            }
        }
    }
    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let c = self.cursor_offset();
        let p = prev_word_boundary(&self.text, c);
        self.edit_range(p..c, "", cx);
    }
    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let c = self.cursor_offset();
        self.edit_range(0..c, "", cx);
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text[self.selected_range.clone()].to_string(),
            ));
            self.edit_range(self.selected_range.clone(), "", cx);
        }
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(clip) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.edit_range(self.selected_range.clone(), &clip, cx);
        }
    }
    pub fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        let idx = self.index_for_x(event.position);
        if event.click_count >= 3 {
            self.selected_range = 0..self.text.len();
            self.selection_reversed = false;
            self.is_selecting = false;
            cx.notify();
        } else if event.click_count == 2 {
            let (s, e) = word_range_at(&self.text, idx);
            if s < e {
                self.selected_range = s..e;
            } else {
                self.selected_range = 0..self.text.len();
            }
            self.selection_reversed = false;
            self.is_selecting = false;
            cx.notify();
        } else if event.modifiers.shift {
            self.is_selecting = true;
            self.select_to(idx, cx);
        } else {
            self.is_selecting = true;
            self.move_to(idx, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.handle_mouse_down(event, window, cx);
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return;
        };
        let viewport_w = f32::from(bounds.size.width).max(1.0);
        let max_scroll = (f32::from(line.width) - viewport_w + 8.0).max(0.0);
        if max_scroll <= 0.5 {
            return;
        }
        let (raw_dx, raw_dy) = match event.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
            ScrollDelta::Lines(l) => (l.x * 20.0, l.y * 20.0),
        };
        let (dx, _) = resolve_wheel_deltas(raw_dx, raw_dy, event.modifiers.shift);
        if dx.abs() < 0.05 {
            return;
        }
        cx.stop_propagation();
        let next_x = (self.scroll_x - dx).clamp(0.0, max_scroll);
        if (next_x - self.scroll_x).abs() > 0.01 {
            self.scroll_x = next_x;
            cx.notify();
        }
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_from_utf16(&self.text, &range_utf16);
        actual_range.replace(range_to_utf16(&self.text, &range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: range_to_utf16(&self.text, &self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|r| range_to_utf16(&self.text, r))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.text, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.edit_range(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| range_from_utf16(&self.text, r))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let clean = new_text.replace(['\r', '\n'], " ");
        let start = clamp_char_boundary(&self.text, range.start);
        let end = clamp_char_boundary(&self.text, range.end).max(start);
        self.text.replace_range(start..end, &clean);
        self.marked_range = (!clean.is_empty()).then_some(start..start + clean.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r16| {
                let local = range_from_utf16(&self.text[start..start + clean.len()], r16);
                (start + local.start)..(start + local.end)
            })
            .unwrap_or_else(|| (start + clean.len())..(start + clean.len()));
        self.edit_version = self.edit_version.wrapping_add(1);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = range_from_utf16(&self.text, &range_utf16);
        let sx = px(self.scroll_x);
        Some(Bounds::from_corners(
            point(
                bounds.left() + layout.x_for_index(self.mapped_index(range.start)) - sx,
                bounds.top(),
            ),
            point(
                bounds.left() + layout.x_for_index(self.mapped_index(range.end)) - sx,
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let utf8 = self.index_for_x(point);
        Some(offset_to_utf16(&self.text, utf8))
    }
}

impl TextField {
    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.focus_handle);
        let idx = self.index_for_x(event.position);
        let sel = &self.selected_range;
        if sel.is_empty() || idx < sel.start || idx > sel.end {
            self.selected_range = idx..idx;
            self.selection_reversed = false;
        }
        // Never offer to copy a masked (password) field's contents.
        let can_copy = !self.selected_range.is_empty() && !self.masked;
        self.context_menu = Some(ContextMenuState {
            position: event.position,
            entries: vec![
                Some((EditCommand::Cut, can_copy)),
                Some((EditCommand::Copy, can_copy)),
                Some((EditCommand::Paste, clipboard_has_text(cx))),
                None,
                Some((EditCommand::SelectAll, !self.text.is_empty())),
            ],
        });
        cx.notify();
    }
}

impl ContextMenuHost for TextField {
    fn run_edit_command(&mut self, cmd: EditCommand, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            EditCommand::Cut => self.cut(&Cut, window, cx),
            EditCommand::Copy => self.copy(&Copy, window, cx),
            EditCommand::Paste => self.paste(&Paste, window, cx),
            EditCommand::SelectAll => self.select_all(&SelectAll, window, cx),
            EditCommand::Undo | EditCommand::Redo => {}
        }
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for TextField {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context_menu = self
            .context_menu
            .as_ref()
            .map(|menu| render_context_menu(menu, cx));
        div()
            .flex()
            .items_center()
            .size_full()
            .min_h(px(26.0))
            .overflow_hidden()
            .key_context(if context_menu.is_some() {
                "EditorInput ContextMenuOpen"
            } else {
                "EditorInput"
            })
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(|this, _: &DismissContextMenu, _, cx| {
                this.close_context_menu(cx)
            }))
            .on_action(|_: &FocusNextField, window, _| window.focus_next())
            .on_action(|_: &FocusPrevField, window, _| window.focus_prev())
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(TextFieldElement {
                field: cx.entity(),
            })
            .when_some(context_menu, |el, menu| el.child(menu))
    }
}

struct TextFieldElement {
    field: Entity<TextField>,
}

struct FieldPrepaint {
    line: ShapedLine,
    scroll_x: f32,
    selection: Option<PaintQuad>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for TextFieldElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextFieldElement {
    type RequestLayoutState = ();
    type PrepaintState = FieldPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = px(22.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let _scope = text_field_scope(self.field.read(cx).text_color_hex);
        let f = self.field.read(cx);
        let style = window.text_style();
        let is_empty = f.text.is_empty();
        let display_str: SharedString = if is_empty {
            f.placeholder.clone()
        } else if f.masked {
            SharedString::from("*".repeat(f.text.chars().count()))
        } else {
            SharedString::from(f.text.clone())
        };
        let color = if is_empty {
            rgb(0x94a3b8).into()
        } else {
            rgb(f.text_color_hex).into()
        };

        let base_run = TextRun {
            len: display_str.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs = if !is_empty && !f.masked && let Some(ref mr) = f.marked_range {
            vec![
                TextRun {
                    len: mr.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: mr.end.saturating_sub(mr.start),
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_str.len().saturating_sub(mr.end),
                    ..base_run
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect()
        } else {
            vec![base_run]
        };

        let shaped = window
            .text_system()
            .shape_line(display_str, px(13.0), &runs, None);

        let map_idx = |byte_idx: usize| -> usize {
            if f.masked {
                let c = clamp_char_boundary(&f.text, byte_idx);
                f.text[..c].chars().count()
            } else {
                clamp_char_boundary(&f.text, byte_idx)
            }
        };

        let viewport_w = f32::from(bounds.size.width).max(1.0);
        let max_scroll = (f32::from(shaped.width) - viewport_w + 8.0).max(0.0);
        let scroll_x = if is_empty {
            0.0
        } else {
            f.scroll_x.clamp(0.0, max_scroll)
        };
        let sx = px(scroll_x);

        let is_dark_fg = f.text_color_hex > 0x808080;
        let sel_fill = if is_dark_fg {
            rgba(0x3b82f666)
        } else {
            rgba(0x2563eb38)
        };
        let cur_fill = if is_dark_fg {
            rgb(0x60a5fa)
        } else {
            rgb(0x2563eb)
        };

        let cur = if is_empty { 0 } else { map_idx(f.cursor_offset()) };
        let (selection, cursor) = if !is_empty && !f.selected_range.is_empty() {
            let x0 = (bounds.left() + shaped.x_for_index(map_idx(f.selected_range.start)) - sx)
                .max(bounds.left());
            let x1 = (bounds.left() + shaped.x_for_index(map_idx(f.selected_range.end)) - sx)
                .min(bounds.right());
            let sel_quad = (x1 > x0).then(|| {
                fill(
                    Bounds::from_corners(
                        point(x0, bounds.top() + px(1.0)),
                        point(x1, bounds.bottom() - px(1.0)),
                    ),
                    sel_fill,
                )
            });
            (sel_quad, None)
        } else {
            let x = if is_empty {
                bounds.left()
            } else {
                bounds.left() + shaped.x_for_index(cur) - sx
            };
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(x, bounds.top() + px(2.0)),
                        size(px(1.8), bounds.size.height - px(4.0)),
                    ),
                    cur_fill,
                )),
            )
        };

        FieldPrepaint {
            line: shaped,
            scroll_x,
            selection,
            cursor,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let _scope = text_field_scope(self.field.read(cx).text_color_hex);
        let focus_handle = self.field.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );
        if let Some(sel) = prepaint.selection.take() {
            window.paint_quad(sel);
        }
        let _ = prepaint.line.paint(
            point(bounds.left() - px(prepaint.scroll_x), bounds.top() + px(2.0)),
            px(20.0),
            window,
            cx,
        );
        if focus_handle.is_focused(window) && let Some(cur) = prepaint.cursor.take() {
            window.paint_quad(cur);
        }
        let shaped_clone = prepaint.line.clone();
        let clamped_scroll = prepaint.scroll_x;
        self.field.update(cx, |f, _| {
            f.last_bounds = Some(bounds);
            f.last_layout = Some(shaped_clone);
            f.scroll_x = clamped_scroll;
        });

        let field_move = self.field.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble && field_move.read(cx).is_selecting {
                field_move.update(cx, |f, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        f.drag_select_to_point(event.position, cx);
                    } else {
                        f.is_selecting = false;
                    }
                });
            }
        });

        let field_up = self.field.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble && field_up.read(cx).is_selecting {
                field_up.update(cx, |f, _| {
                    f.is_selecting = false;
                });
            }
        });
    }
}

// ==================== PHP SYNTAX TOKENIZER (LIGHT THEME) ====================

fn highlight_php_line(line: &str, base_font: &Font) -> Vec<TextRun> {
    if line.is_empty() {
        return Vec::new();
    }

    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut i = 0;

    let push_run = |runs: &mut Vec<TextRun>, run_len: usize, color_hex: u32, italic: bool| {
        if run_len == 0 {
            return;
        }
        let font = if italic {
            Font {
                style: FontStyle::Italic,
                ..base_font.clone()
            }
        } else {
            base_font.clone()
        };
        runs.push(TextRun {
            len: run_len,
            font,
            color: rgb(color_hex).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    };

    while i < len {
        let b = bytes[i];

        // Line comment `//...`
        if i + 1 < len && b == b'/' && bytes[i + 1] == b'/' {
            push_run(&mut runs, len - i, 0x94a3b8, true);
            break;
        }

        // Block comment `/*...*/`
        if i + 1 < len && b == b'/' && bytes[i + 1] == b'*' {
            if let Some(end_rel) = line[i + 2..].find("*/") {
                let total = 2 + end_rel + 2;
                push_run(&mut runs, total, 0x94a3b8, true);
                i += total;
                continue;
            } else {
                push_run(&mut runs, len - i, 0x94a3b8, true);
                break;
            }
        }

        // Strings `'...'` or `"..."`
        if b == b'\'' || b == b'"' {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            push_run(&mut runs, i - start, 0x059669, false);
            continue;
        }

        // PHP variable `$foo`
        if b == b'$' && i + 1 < len && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            let start = i;
            i += 2;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            push_run(&mut runs, i - start, 0xe11d48, false);
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_') {
                i += 1;
            }
            push_run(&mut runs, i - start, 0x2563eb, false);
            continue;
        }

        // Identifiers / Keywords / Classes / Methods
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let is_kw = matches!(
                word,
                "use"
                    | "namespace"
                    | "new"
                    | "return"
                    | "fn"
                    | "function"
                    | "class"
                    | "if"
                    | "else"
                    | "elseif"
                    | "foreach"
                    | "for"
                    | "while"
                    | "as"
                    | "match"
                    | "true"
                    | "false"
                    | "null"
                    | "echo"
                    | "public"
                    | "protected"
                    | "private"
                    | "static"
            );
            let is_call = i < len && bytes[i] == b'(';
            let is_static_lhs = i + 1 < len && bytes[i] == b':' && bytes[i + 1] == b':';
            let is_upper_class = word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);

            let color = if is_kw {
                0xa21caf
            } else if is_call || is_static_lhs || is_upper_class {
                0x0284c7
            } else {
                0x1e293b
            };
            push_run(&mut runs, i - start, color, false);
            continue;
        }

        // Advance one UTF-8 character for operators, whitespace, or non-ASCII chars
        let ch_len = line[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        push_run(&mut runs, ch_len, 0x334155, false);
        i += ch_len;
    }

    runs
}

// ==================== UTF-8 / LINE HELPERS ====================

fn clamp_char_boundary(s: &str, idx: usize) -> usize {
    let mut c = idx.min(s.len());
    while c > 0 && !s.is_char_boundary(c) {
        c -= 1;
    }
    c
}

fn prev_char_boundary(s: &str, idx: usize) -> usize {
    let c = clamp_char_boundary(s, idx);
    if c == 0 {
        return 0;
    }
    s[..c].char_indices().next_back().map(|(b, _)| b).unwrap_or(0)
}

fn next_char_boundary(s: &str, idx: usize) -> usize {
    let c = clamp_char_boundary(s, idx);
    if c >= s.len() {
        return s.len();
    }
    let ch_len = s[c..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
    (c + ch_len).min(s.len())
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

fn prev_word_boundary(s: &str, idx: usize) -> usize {
    let c = clamp_char_boundary(s, idx);
    if c == 0 {
        return 0;
    }
    let chars: Vec<(usize, char)> = s[..c].char_indices().collect();
    let mut k = chars.len();
    while k > 0 && chars[k - 1].1.is_whitespace() {
        k -= 1;
    }
    if k == 0 {
        return 0;
    }
    let word_mode = is_word_char(chars[k - 1].1);
    while k > 0 && !chars[k - 1].1.is_whitespace() && is_word_char(chars[k - 1].1) == word_mode {
        k -= 1;
    }
    chars.get(k).map(|(b, _)| *b).unwrap_or(0)
}

fn next_word_boundary(s: &str, idx: usize) -> usize {
    let c = clamp_char_boundary(s, idx);
    if c >= s.len() {
        return s.len();
    }
    let chars: Vec<(usize, char)> = s[c..].char_indices().collect();
    let mut k = 0;
    while k < chars.len() && chars[k].1.is_whitespace() {
        k += 1;
    }
    if k >= chars.len() {
        return s.len();
    }
    let word_mode = is_word_char(chars[k].1);
    while k < chars.len() && !chars[k].1.is_whitespace() && is_word_char(chars[k].1) == word_mode {
        k += 1;
    }
    chars.get(k).map(|(b, _)| c + *b).unwrap_or(s.len())
}

fn word_range_at(s: &str, idx: usize) -> (usize, usize) {
    let c = clamp_char_boundary(s, idx);
    let mut start = c;
    for (b, ch) in s[..c].char_indices().rev() {
        if is_word_char(ch) {
            start = b;
        } else {
            break;
        }
    }
    let mut end = c;
    for (rel, ch) in s[c..].char_indices() {
        if is_word_char(ch) {
            end = c + rel + ch.len_utf8();
        } else {
            break;
        }
    }
    (start, end)
}

fn line_start_offset(s: &str, idx: usize) -> usize {
    let c = clamp_char_boundary(s, idx);
    s[..c].rfind('\n').map(|p| p + 1).unwrap_or(0)
}

fn smart_line_start(s: &str, idx: usize) -> usize {
    let ls = line_start_offset(s, idx);
    let le = line_end_offset(s, idx);
    let non_ws = s[ls..le]
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(b, _)| ls + b)
        .unwrap_or(ls);
    if idx == non_ws { ls } else { non_ws }
}

fn line_end_offset(s: &str, idx: usize) -> usize {
    let c = clamp_char_boundary(s, idx);
    s[c..].find('\n').map(|rel| c + rel).unwrap_or(s.len())
}

fn line_end_with_newline(s: &str, idx: usize) -> usize {
    let le = line_end_offset(s, idx);
    if le < s.len() && s.as_bytes()[le] == b'\n' {
        le + 1
    } else {
        le
    }
}

fn line_byte_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (idx, b) in s.bytes().enumerate() {
        if b == b'\n' {
            out.push((start, idx));
            start = idx + 1;
        }
    }
    out.push((start, s.len()));
    out
}

fn byte_to_line_col(s: &str, byte_offset: usize) -> (usize, usize) {
    let c = clamp_char_boundary(s, byte_offset);
    let mut line = 0usize;
    let mut line_start = 0usize;
    for (idx, b) in s[..c].bytes().enumerate() {
        if b == b'\n' {
            line += 1;
            line_start = idx + 1;
        }
    }
    let col = s[line_start..c].chars().count();
    (line, col)
}

fn char_col_to_byte(line_str: &str, col: usize) -> usize {
    line_str
        .char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line_str.len())
}

fn offset_to_utf16(s: &str, utf8_offset: usize) -> usize {
    let target = clamp_char_boundary(s, utf8_offset);
    let mut utf16 = 0usize;
    for (idx, ch) in s.char_indices() {
        if idx >= target {
            break;
        }
        utf16 += ch.len_utf16();
    }
    utf16
}

fn offset_from_utf16(s: &str, utf16_offset: usize) -> usize {
    let mut utf16 = 0usize;
    for (idx, ch) in s.char_indices() {
        if utf16 >= utf16_offset {
            return idx;
        }
        utf16 += ch.len_utf16();
    }
    s.len()
}

fn range_to_utf16(s: &str, range: &Range<usize>) -> Range<usize> {
    offset_to_utf16(s, range.start)..offset_to_utf16(s, range.end)
}

fn range_from_utf16(s: &str, range_utf16: &Range<usize>) -> Range<usize> {
    let a = offset_from_utf16(s, range_utf16.start);
    let b = offset_from_utf16(s, range_utf16.end);
    a.min(b)..a.max(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_utf8_roundtrip_handles_multibyte_and_php_code() {
        let s = "$user = User::find(42); // 🚀 test\n$name = 'Café';";
        for (byte_idx, _) in s.char_indices() {
            let u16_idx = offset_to_utf16(s, byte_idx);
            let back = offset_from_utf16(s, u16_idx);
            assert_eq!(back, byte_idx);
        }
        assert_eq!(offset_from_utf16(s, offset_to_utf16(s, s.len())), s.len());
    }

    #[test]
    fn word_boundaries_and_smart_home_work_like_standard_editor() {
        let s = "    $wasAdded = $user->attachApp();";
        // Smart home jumps to first non-whitespace (4), then to 0
        assert_eq!(smart_line_start(s, 12), 4);
        assert_eq!(smart_line_start(s, 4), 0);

        let (ws, we) = word_range_at(s, 6);
        assert_eq!(&s[ws..we], "$wasAdded");
    }

    #[test]
    fn php_syntax_highlighter_preserves_exact_byte_lengths() {
        let font = Font {
            family: "Menlo".into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let samples = [
            "// Get a user from your database",
            "$user = User::find(42); // inline comment",
            "    /* block */ ->where('plan', '!=', \"starter\")",
            "Mail::to($user->email)->send(new WelcomeEmail());",
        ];
        for line in samples {
            let runs = highlight_php_line(line, &font);
            let total: usize = runs.iter().map(|r| r.len).sum();
            assert_eq!(total, line.len(), "mismatch on line: {line}");
        }
    }

    #[test]
    fn horizontal_scroll_limits_and_wheel_deltas_work_for_long_lines() {
        // Fits comfortably -> no horizontal scroll
        assert_eq!(compute_max_scroll_x(220.0, 400.0), 0.0);

        // Overflows viewport -> positive max_scroll_x with right breathing room
        let max_x = compute_max_scroll_x(640.0, 400.0);
        assert!((max_x - (640.0 + TEXT_RIGHT_PAD - 400.0)).abs() < 0.01);

        // Trackpad horizontal swipe preserves (dx, dy)
        assert_eq!(resolve_wheel_deltas(-32.0, -4.0, false), (-32.0, -4.0));

        // Shift + vertical scroll wheel maps vertical delta into horizontal scroll
        assert_eq!(resolve_wheel_deltas(0.0, -48.0, true), (-48.0, 0.0));
        // If OS already translated Shift+wheel into dx, don't overwrite it
        assert_eq!(resolve_wheel_deltas(-48.0, 0.0, true), (-48.0, 0.0));
    }

    #[test]
    fn cursor_visibility_and_scrollbar_thumb_drag_roundtrip() {
        let usable_w = 360.0;
        let max_x = compute_max_scroll_x(720.0, usable_w);

        // Moving cursor past right edge scrolls horizontally so cursor stays visible
        let sx = adjust_scroll_x_for_cursor(0.0, 500.0, usable_w, max_x);
        assert!(sx > 150.0 && sx <= max_x);
        // Cursor already visible does not jump scroll_x
        assert_eq!(adjust_scroll_x_for_cursor(sx, 420.0, usable_w, max_x), sx);
        // Moving cursor back to start of line resets scroll_x to 0
        assert_eq!(adjust_scroll_x_for_cursor(sx, 0.0, usable_w, max_x), 0.0);

        // Scrollbar thumb grab & drag roundtrip
        let track_start = 46.0;
        let track_len = usable_w;
        let initial_scroll = 140.0;
        let (thumb_start, thumb_len) =
            compute_scrollbar_thumb_1d(track_start, track_len, initial_scroll, max_x, usable_w);
        let grab_offset = 12.0;
        let unchanged = scroll_from_thumb_drag_1d(
            track_start,
            track_len,
            thumb_len,
            thumb_start + grab_offset,
            grab_offset,
            max_x,
        );
        assert!((unchanged - initial_scroll).abs() < 0.05);

        // Dragging past right end clamps to max_x; dragging past left end clamps to 0
        assert!(
            (scroll_from_thumb_drag_1d(
                track_start,
                track_len,
                thumb_len,
                track_start + track_len + 100.0,
                grab_offset,
                max_x,
            ) - max_x)
                .abs()
                < 0.01
        );
        assert_eq!(
            scroll_from_thumb_drag_1d(track_start, track_len, thumb_len, 0.0, grab_offset, max_x),
            0.0
        );
    }
}

/// Text fields with light text live on dark-authored panels (settings, SSH),
/// so their colors follow that panel's theme scope.
fn text_field_scope(text_color_hex: u32) -> crate::theme::ScopeGuard {
    let scope = if text_color_hex > 0x808080 {
        crate::theme::Scope::DarkAuthored
    } else {
        crate::theme::Scope::Normal
    };
    crate::theme::enter_scope(scope)
}
