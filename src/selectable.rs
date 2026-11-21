//! Read-only, selectable text for the output panes (Detail Dive, CLI, SQL).
//!
//! GPUI's text elements cannot be selected. [`SelectableText`] wraps a
//! [`StyledText`] and registers itself with a shared [`TextSelection`], which
//! tracks one selection that can span many text blocks (one per CLI line, one
//! per value, ...). The region that hosts the blocks is wired up with
//! [`selectable_region`] so it gets ⌘C / ⌘A and a right-click menu.

use crate::theme::{rgb, rgba};
use gpui::{
    anchored, deferred, div, fill, prelude::*, px, App, Bounds, ClipboardItem,
    CursorStyle, DispatchPhase, Element, ElementId, FocusHandle, GlobalElementId, HighlightStyle,
    Hitbox, HitboxBehavior, InspectorElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StyledText, TextLayout, Window,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

// The editor's own Copy / Select All actions, so the Edit menu (macOS menu bar
// and the in-window menu) finds a handler while an output pane has focus.
use crate::editor::{Copy as CopySelection, SelectAll as SelectAllText};

const KEY_CONTEXT: &str = "SelectableText";

pub fn bind_selectable_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-c", CopySelection, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-a", SelectAllText, Some(KEY_CONTEXT)),
    ]);
}

/// One painted text block, as laid out in a frame.
struct Block {
    key: SharedString,
    text: SharedString,
    /// Prepended when the block is copied from its start (e.g. CLI indentation).
    copy_prefix: SharedString,
    layout: TextLayout,
    bounds: Bounds<Pixels>,
}

#[derive(Clone, Debug, PartialEq)]
struct Pos {
    key: SharedString,
    offset: usize,
}

struct Inner {
    focus: FocusHandle,
    /// Blocks of the last fully painted frame (used while painting the next one).
    prev: Vec<Block>,
    /// Blocks registered so far in the current frame; complete between frames,
    /// which is when input events are handled.
    cur: Vec<Block>,
    anchor: Option<Pos>,
    head: Option<Pos>,
    dragging: bool,
    /// The current mouse-down landed on a text block.
    claimed: bool,
    /// The last press made a selection, so a click handler under it should not fire.
    suppress_click: bool,
    menu: Option<Point<Pixels>>,
}

/// Shared selection state for one selectable region. Cheap to clone.
#[derive(Clone)]
pub struct TextSelection(Rc<RefCell<Inner>>);

impl TextSelection {
    pub fn new(focus: FocusHandle) -> Self {
        Self(Rc::new(RefCell::new(Inner {
            focus,
            prev: Vec::new(),
            cur: Vec::new(),
            anchor: None,
            head: None,
            dragging: false,
            claimed: false,
            suppress_click: false,
            menu: None,
        })))
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.0.borrow().focus.clone()
    }

    /// Call once per render of the region, before any block is built.
    pub fn begin_frame(&self) {
        let mut inner = self.0.borrow_mut();
        inner.prev = std::mem::take(&mut inner.cur);
    }

    pub fn clear(&self) {
        let mut inner = self.0.borrow_mut();
        inner.anchor = None;
        inner.head = None;
        inner.dragging = false;
        inner.menu = None;
    }

    /// True (once) if the press that just ended selected text; click handlers on
    /// rows that also hold selectable text check this to avoid toggling.
    pub fn take_suppress_click(&self) -> bool {
        std::mem::take(&mut self.0.borrow_mut().suppress_click)
    }

    pub fn has_selection(&self) -> bool {
        !self.selected_text().is_empty()
    }

    /// Selected text between frames, when input events are handled.
    pub fn selected_text(&self) -> String {
        self.text_in(false)
    }

    /// `from_prev` reads the last painted frame; use it while rendering, when the
    /// current frame's blocks are not registered yet.
    fn text_in(&self, from_prev: bool) -> String {
        let inner = self.0.borrow();
        let blocks = if from_prev { &inner.prev } else { &inner.cur };
        let Some(((si, so), (ei, eo))) = ordered(blocks, &inner.anchor, &inner.head) else {
            return String::new();
        };
        let mut out = String::new();
        for i in si..=ei {
            let b = &blocks[i];
            let len = b.text.len();
            let start = if i == si { clamp_boundary(&b.text, so) } else { 0 };
            let end = if i == ei { clamp_boundary(&b.text, eo) } else { len };
            if i > si {
                let same_row = (b.bounds.top() - blocks[i - 1].bounds.top()).abs() < px(2.0);
                out.push(if same_row { ' ' } else { '\n' });
            }
            if start == 0 && !(i == ei && end == 0) {
                out.push_str(&b.copy_prefix);
            }
            if start < end {
                out.push_str(&b.text[start..end]);
            }
        }
        out
    }

    pub fn copy(&self, cx: &mut App) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn select_all(&self) {
        let mut inner = self.0.borrow_mut();
        let (Some(first), Some(last)) = (inner.cur.first(), inner.cur.last()) else {
            return;
        };
        let anchor = Pos { key: first.key.clone(), offset: 0 };
        let head = Pos { key: last.key.clone(), offset: last.text.len() };
        inner.anchor = Some(anchor);
        inner.head = Some(head);
    }

    fn open_menu(&self, position: Point<Pixels>) {
        self.0.borrow_mut().menu = Some(position);
    }

    fn close_menu(&self) -> bool {
        self.0.borrow_mut().menu.take().is_some()
    }

    /// Records a block for this frame; returns its paint order.
    fn register(&self, block: Block) -> usize {
        let mut inner = self.0.borrow_mut();
        inner.cur.push(block);
        inner.cur.len() - 1
    }

    /// The part of block `key` that is selected, using the previous frame's order.
    fn range_for(&self, key: &SharedString, text: &str) -> Option<Range<usize>> {
        let inner = self.0.borrow();
        let blocks = &inner.prev;
        let ((si, so), (ei, eo)) = ordered(blocks, &inner.anchor, &inner.head)?;
        let i = blocks.iter().position(|b| &b.key == key)?;
        if i < si || i > ei {
            return None;
        }
        let start = if i == si { clamp_boundary(text, so) } else { 0 };
        let end = if i == ei { clamp_boundary(text, eo) } else { text.len() };
        (start < end).then_some(start..end)
    }

    fn on_block_mouse_down(&self, key: &SharedString, text: &str, offset: usize, event: &MouseDownEvent) {
        let mut inner = self.0.borrow_mut();
        inner.claimed = true;
        if inner.menu.is_some() {
            // The menu closes itself; keep the selection it acts on.
            return;
        }
        let offset = clamp_boundary(text, offset);
        let head = Pos { key: key.clone(), offset };
        match event.click_count {
            2 => {
                let r = word_range(text, offset);
                inner.anchor = Some(Pos { key: key.clone(), offset: r.start });
                inner.head = Some(Pos { key: key.clone(), offset: r.end });
            }
            n if n >= 3 => {
                inner.anchor = Some(Pos { key: key.clone(), offset: 0 });
                inner.head = Some(Pos { key: key.clone(), offset: text.len() });
            }
            _ => {
                if !(event.modifiers.shift && inner.anchor.is_some()) {
                    inner.anchor = Some(head.clone());
                }
                inner.head = Some(head);
            }
        }
        inner.dragging = true;
    }

    /// A press in the region that missed every text block: beside a line (an
    /// indent gutter, past its end) it starts a selection on that row, like a
    /// native text view; anywhere else it clears the selection.
    fn on_region_mouse_down(&self, event: &MouseDownEvent) -> bool {
        let mut inner = self.0.borrow_mut();
        if inner.menu.is_some() {
            return false;
        }
        let y = event.position.y;
        let on_row = inner
            .cur
            .iter()
            .any(|b| y >= b.bounds.top() && y <= b.bounds.bottom());
        if on_row && let Some(pos) = resolve_point(&inner.cur, event.position) {
            if !(event.modifiers.shift && inner.anchor.is_some()) {
                inner.anchor = Some(pos.clone());
            }
            inner.head = Some(pos);
            inner.dragging = true;
            return true;
        }
        let had = inner.anchor.is_some();
        inner.anchor = None;
        inner.head = None;
        inner.dragging = false;
        had
    }

    fn on_drag(&self, position: Point<Pixels>) -> bool {
        let mut inner = self.0.borrow_mut();
        if !inner.dragging {
            return false;
        }
        let Some(pos) = resolve_point(&inner.cur, position) else {
            return false;
        };
        if inner.head.as_ref() == Some(&pos) {
            return false;
        }
        inner.head = Some(pos);
        true
    }

    fn on_mouse_up(&self) {
        let was_dragging = std::mem::take(&mut self.0.borrow_mut().dragging);
        if was_dragging && self.has_selection() {
            self.0.borrow_mut().suppress_click = true;
        }
    }

    pub fn render_context_menu(&self) -> Option<gpui::AnyElement> {
        let position = self.0.borrow().menu?;
        let has_sel = !self.text_in(true).is_empty();
        let item = |id: &'static str, label: &'static str, shortcut: String, enabled: bool, sel: TextSelection, run: fn(&TextSelection, &mut App)| {
            div()
                .id(id)
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
                })
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    sel.close_menu();
                    if enabled {
                        run(&sel, cx);
                    }
                    window.refresh();
                })
                .child(label)
                .child(div().opacity(0.6).child(shortcut))
        };
        let close = self.clone();
        Some(
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(8.0))
                    .child(
                        div()
                            .id("selectable-context-menu")
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
                            .on_mouse_down_out(move |_, window, _| {
                                if close.close_menu() {
                                    window.refresh();
                                }
                            })
                            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                            .child(item("selectable-menu-copy", "Copy", crate::keymap::display("secondary-c"), has_sel, self.clone(), |s, cx| s.copy(cx)))
                            .child(item("selectable-menu-select-all", "Select All", crate::keymap::display("secondary-a"), true, self.clone(), |s, _| s.select_all())),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// Wires a container up as the host of `sel`'s blocks: focus, ⌘C / ⌘A,
/// clearing the selection on clicks outside text, and the right-click menu.
pub fn selectable_region<E>(el: E, sel: &TextSelection) -> E
where
    E: InteractiveElement + ParentElement,
{
    let (s1, s2, s3, s4, s5) = (sel.clone(), sel.clone(), sel.clone(), sel.clone(), sel.clone());
    el.track_focus(&sel.focus_handle())
        .key_context(KEY_CONTEXT)
        .on_action(move |_: &CopySelection, _, cx| s1.copy(cx))
        .on_action(move |_: &SelectAllText, window, _| {
            s2.select_all();
            window.refresh();
        })
        .capture_any_mouse_down(move |_, _, _| {
            let mut inner = s3.0.borrow_mut();
            inner.claimed = false;
            inner.suppress_click = false;
        })
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            if !s4.0.borrow().claimed && s4.on_region_mouse_down(event) {
                window.focus(&s4.focus_handle());
                window.refresh();
            }
        })
        .on_mouse_down(MouseButton::Right, move |event, window, _| {
            s5.open_menu(event.position);
            window.focus(&s5.focus_handle());
            window.refresh();
        })
        .children(sel.render_context_menu())
}

/// A text block that takes part in `sel`. `key` must be unique within the
/// region and stable across renders.
pub struct SelectableText {
    sel: TextSelection,
    key: SharedString,
    text: SharedString,
    copy_prefix: SharedString,
    styled: StyledText,
}

pub fn selectable_text(
    sel: &TextSelection,
    key: impl Into<SharedString>,
    text: impl Into<SharedString>,
) -> SelectableText {
    let text = text.into();
    SelectableText {
        sel: sel.clone(),
        key: key.into(),
        styled: StyledText::new(text.clone()),
        text,
        copy_prefix: SharedString::default(),
    }
}

impl SelectableText {
    pub fn with_highlights(
        mut self,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        self.styled = StyledText::new(self.text.clone()).with_highlights(highlights);
        self
    }

    pub fn copy_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.copy_prefix = prefix.into();
        self
    }
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.styled.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled.prepaint(id, inspector_id, bounds, state, window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let order = self.sel.register(Block {
            key: self.key.clone(),
            text: self.text.clone(),
            copy_prefix: self.copy_prefix.clone(),
            layout: layout.clone(),
            bounds,
        });

        if let Some(range) = self.sel.range_for(&self.key, &self.text) {
            paint_selection(&layout, bounds, range, window);
        }
        self.styled.paint(id, inspector_id, bounds, state, &mut (), window, cx);

        window.set_cursor_style(CursorStyle::IBeam, hitbox);

        let sel = self.sel.clone();
        let key = self.key.clone();
        let text = self.text.clone();
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
            if phase != DispatchPhase::Capture
                || event.button != MouseButton::Left
                || !hitbox.is_hovered(window)
            {
                return;
            }
            let offset = layout
                .index_for_position(event.position)
                .unwrap_or_else(|closest| closest);
            sel.on_block_mouse_down(&key, &text, offset, event);
            window.focus(&sel.focus_handle());
            window.refresh();
        });

        // Drag and release are tracked once per region, not per block.
        if order == 0 {
            let sel = self.sel.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    sel.0.borrow_mut().dragging = false;
                    return;
                }
                if sel.on_drag(event.position) {
                    window.refresh();
                }
            });
            let sel = self.sel.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, _| {
                if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                    sel.on_mouse_up();
                }
            });
        }
    }
}

fn paint_selection(layout: &TextLayout, bounds: Bounds<Pixels>, range: Range<usize>, window: &mut Window) {
    let (Some(start), Some(end)) = (
        layout.position_for_index(range.start),
        layout.position_for_index(range.end),
    ) else {
        return;
    };
    let lh = layout.line_height();
    let color = rgba(0x3b82f63d);
    let rect = |x0: Pixels, x1: Pixels, y: Pixels| {
        Bounds::from_corners(gpui::point(x0, y), gpui::point(x1.max(x0 + px(1.0)), y + lh))
    };
    if (start.y - end.y).abs() < px(0.5) {
        window.paint_quad(fill(rect(start.x, end.x, start.y), color));
        return;
    }
    window.paint_quad(fill(rect(start.x, bounds.right(), start.y), color));
    let mut y = start.y + lh;
    while y + px(0.5) < end.y {
        window.paint_quad(fill(rect(bounds.left(), bounds.right(), y), color));
        y += lh;
    }
    window.paint_quad(fill(rect(bounds.left(), end.x, end.y), color));
}

/// Anchor/head as (block index, offset), start first.
fn ordered(
    blocks: &[Block],
    anchor: &Option<Pos>,
    head: &Option<Pos>,
) -> Option<((usize, usize), (usize, usize))> {
    let (anchor, head) = (anchor.as_ref()?, head.as_ref()?);
    let ai = blocks.iter().position(|b| b.key == anchor.key)?;
    let hi = blocks.iter().position(|b| b.key == head.key)?;
    let a = (ai, anchor.offset);
    let h = (hi, head.offset);
    Some(if a <= h { (a, h) } else { (h, a) })
}

/// The text position nearest to `point`.
fn resolve_point(blocks: &[Block], point: Point<Pixels>) -> Option<Pos> {
    if let Some(b) = blocks.iter().rev().find(|b| b.bounds.contains(&point)) {
        let offset = b.layout.index_for_position(point).unwrap_or_else(|c| c);
        return Some(Pos { key: b.key.clone(), offset: clamp_boundary(&b.text, offset) });
    }
    let dist = |b: &Block| {
        let dy = if point.y < b.bounds.top() {
            b.bounds.top() - point.y
        } else if point.y > b.bounds.bottom() {
            point.y - b.bounds.bottom()
        } else {
            px(0.0)
        };
        let dx = if point.x < b.bounds.left() {
            b.bounds.left() - point.x
        } else if point.x > b.bounds.right() {
            point.x - b.bounds.right()
        } else {
            px(0.0)
        };
        (dy, dx)
    };
    let b = blocks
        .iter()
        .min_by(|a, b| dist(a).partial_cmp(&dist(b)).unwrap_or(std::cmp::Ordering::Equal))?;
    let offset = if point.y < b.bounds.top() {
        0
    } else if point.y > b.bounds.bottom() {
        b.text.len()
    } else if point.x < b.bounds.left() {
        let probe = gpui::point(b.bounds.left(), point.y);
        b.layout.index_for_position(probe).unwrap_or_else(|c| c)
    } else {
        let probe = gpui::point(b.bounds.right() - px(0.5), point.y);
        b.layout.index_for_position(probe).unwrap_or_else(|c| c)
    };
    Some(Pos { key: b.key.clone(), offset: clamp_boundary(&b.text, offset) })
}

fn clamp_boundary(text: &str, offset: usize) -> usize {
    let mut i = offset.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn word_range(text: &str, offset: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$' || c == '\\';
    let offset = clamp_boundary(text, offset);
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(offset, |(i, _)| i);
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(i, _)| offset + i);
    if start == end {
        // Not on a word: select the single character under the pointer.
        let next = text[offset..].chars().next().map_or(offset, |c| offset + c.len_utf8());
        return offset..next;
    }
    start..end
}
