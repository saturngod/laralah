use crate::models::ObjectNode;
use gpui::{
    div, fill, point, prelude::*, px, relative, rgb, size, App, Bounds, ContentMask, Context,
    CursorStyle, Element, ElementId, Entity, Font, FontFeatures, FontStyle, FontWeight,
    GlobalElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Style,
    TextRun, Window,
};
use std::collections::HashSet;

const CARD_WIDTH: f32 = 368.0;
const CARD_PAD_X: f32 = 13.0;
const CARD_PAD_Y: f32 = 11.0;
const ATTR_LINE_H: f32 = 17.5;
const H_GAP: f32 = 34.0;
const ARROW_TOP_LEN: f32 = 42.0;
const PILL_HEIGHT: f32 = 28.0;
const PILL_MIN_WIDTH: f32 = 142.0;
const ARROW_BOTTOM_LEN: f32 = 42.0;
const MAX_GRAPH_DEPTH: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphAttrKind {
    Number,
    String,
    BoolTrue,
    BoolFalse,
    Null,
    Other,
}

#[derive(Clone, Debug)]
pub struct GraphAttrLine {
    pub key: String,
    pub value: String,
    pub kind: GraphAttrKind,
}

#[derive(Clone, Debug)]
pub struct PositionedCard {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub attrs: Vec<GraphAttrLine>,
}

#[derive(Clone, Debug)]
pub struct PositionedPill {
    pub rel_id: String,
    pub label: String,
    pub count: usize,
    pub expanded: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub struct GraphEdge {
    pub from_x: f32,
    pub from_y: f32,
    pub to_x: f32,
    pub to_y: f32,
}

#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    pub cards: Vec<PositionedCard>,
    pub pills: Vec<PositionedPill>,
    pub edges: Vec<GraphEdge>,
    pub total_width: f32,
    pub total_height: f32,
}

struct MeasuredEntity {
    attrs: Vec<GraphAttrLine>,
    card_height: f32,
    relations: Vec<MeasuredRelation>,
    subtree_width: f32,
}

struct MeasuredRelation {
    rel_id: String,
    label: String,
    count: usize,
    expanded: bool,
    pill_width: f32,
    targets: Vec<MeasuredEntity>,
    rel_width: f32,
}

pub fn has_object_graph(node: &ObjectNode) -> bool {
    node.class_name.is_some()
        || node.type_name.starts_with("Collection:")
        || (node.type_name.starts_with("array:") && !node.children.is_empty())
}

pub fn default_expanded_relations(root: &ObjectNode) -> HashSet<String> {
    let mut set = HashSet::new();
    let roots = extract_root_entities(root);
    for (idx, entity) in roots.iter().enumerate() {
        let path = format!("r{idx}");
        collect_default_rel_ids(entity, &path, 0, &mut set);
    }
    set
}

fn collect_default_rel_ids(
    entity: &ObjectNode,
    path: &str,
    depth: usize,
    out: &mut HashSet<String>,
) {
    if depth >= MAX_GRAPH_DEPTH {
        return;
    }
    let rels = extract_entity_relations(entity);
    let has_eager = rels
        .iter()
        .any(|(ch, _)| ch.visibility.as_deref() == Some("rel"));

    let mut auto_expanded_first = false;
    for (ch, targets) in rels {
        if targets.is_empty() {
            continue;
        }
        let rel_id = format!("{path}/{}", ch.label);
        let is_eager = ch.visibility.as_deref() != Some("graph_rel");
        let should_expand = if has_eager {
            is_eager
        } else if !auto_expanded_first && depth == 0 {
            auto_expanded_first = true;
            true
        } else {
            false
        };

        if should_expand {
            out.insert(rel_id.clone());
            for (t_idx, target) in targets.iter().enumerate() {
                let child_path = format!("{rel_id}[{t_idx}]");
                collect_default_rel_ids(target, &child_path, depth + 1, out);
            }
        }
    }
}

pub fn build_object_graph_layout(
    root: &ObjectNode,
    expanded_relations: &HashSet<String>,
) -> GraphLayout {
    let roots = extract_root_entities(root);
    if roots.is_empty() {
        return GraphLayout::default();
    }

    let measured_roots: Vec<MeasuredEntity> = roots
        .iter()
        .enumerate()
        .map(|(idx, entity)| {
            let path = format!("r{idx}");
            measure_entity(entity, &path, 0, expanded_relations)
        })
        .collect();

    let mut layout = GraphLayout::default();
    let mut cursor_x = 0.0_f32;

    for (idx, m) in measured_roots.iter().enumerate() {
        if idx > 0 {
            cursor_x += H_GAP;
        }
        place_entity(m, cursor_x, 0.0, &mut layout);
        cursor_x += m.subtree_width;
    }

    layout.total_width = cursor_x.max(CARD_WIDTH);
    layout.total_height = layout
        .cards
        .iter()
        .map(|c| c.y + c.height)
        .chain(layout.pills.iter().map(|p| p.y + p.height))
        .fold(0.0_f32, f32::max);

    layout
}

fn extract_root_entities(root: &ObjectNode) -> Vec<&ObjectNode> {
    let is_collection = root.type_name.starts_with("Collection:")
        || root
            .class_name
            .as_deref()
            .is_some_and(|c| c.ends_with("\\Collection") || c == "Collection");
    let is_seq_array = root.type_name.starts_with("array:")
        && !root.children.is_empty()
        && root
            .children
            .iter()
            .filter(|c| c.type_name != "truncated")
            .enumerate()
            .all(|(i, c)| c.label == i.to_string() && !c.children.is_empty());

    if is_collection || is_seq_array {
        let items: Vec<&ObjectNode> = root
            .children
            .iter()
            .filter(|c| c.type_name != "truncated")
            .take(24)
            .collect();
        if !items.is_empty() && items.iter().all(|it| !it.children.is_empty()) {
            return items;
        }
    }

    vec![root]
}

fn is_relation_child(ch: &ObjectNode) -> bool {
    if ch.type_name == "truncated" || ch.type_name == "DateTime" || ch.type_name == "null" {
        return false;
    }
    if matches!(ch.visibility.as_deref(), Some("rel") | Some("graph_rel")) {
        return true;
    }
    if ch.type_name.starts_with("Collection:") {
        return true;
    }
    if ch.class_name.is_some() && !ch.children.is_empty() {
        return true;
    }
    if ch.type_name.starts_with("array:") && !ch.children.is_empty() {
        return true;
    }
    false
}

fn extract_entity_relations(entity: &ObjectNode) -> Vec<(&ObjectNode, Vec<&ObjectNode>)> {
    let mut out = Vec::new();
    for ch in &entity.children {
        if !is_relation_child(ch) {
            continue;
        }
        let is_collection = ch.type_name.starts_with("Collection:")
            || ch
                .class_name
                .as_deref()
                .is_some_and(|c| c.ends_with("\\Collection") || c == "Collection");
        let is_seq_obj_array = ch.type_name.starts_with("array:")
            && !ch.children.is_empty()
            && ch
                .children
                .iter()
                .filter(|c| c.type_name != "truncated")
                .enumerate()
                .all(|(i, c)| c.label == i.to_string() && !c.children.is_empty());

        let targets: Vec<&ObjectNode> = if is_collection || is_seq_obj_array {
            ch.children
                .iter()
                .filter(|c| c.type_name != "truncated" && !c.children.is_empty())
                .take(12)
                .collect()
        } else {
            vec![ch]
        };

        out.push((ch, targets));
    }
    out
}

fn extract_entity_attrs(entity: &ObjectNode) -> Vec<GraphAttrLine> {
    let mut attrs = Vec::new();
    for ch in &entity.children {
        if ch.type_name == "truncated" || is_relation_child(ch) {
            continue;
        }
        if ch.visibility.as_deref() == Some("graph_rel") {
            continue;
        }
        let (val, kind) = format_attr_value(ch);
        attrs.push(GraphAttrLine {
            key: ch.label.clone(),
            value: val,
            kind,
        });
    }

    if attrs.is_empty() {
        let (val, kind) = format_attr_value(entity);
        attrs.push(GraphAttrLine {
            key: entity
                .class_name
                .as_deref()
                .and_then(|c| c.rsplit('\\').next())
                .unwrap_or("value")
                .to_string(),
            value: val,
            kind,
        });
    }
    attrs
}

fn format_attr_value(node: &ObjectNode) -> (String, GraphAttrKind) {
    match node.type_name.as_str() {
        "int" | "float" => (
            node.scalar_value
                .as_deref()
                .unwrap_or(&node.summary)
                .to_string(),
            GraphAttrKind::Number,
        ),
        "bool" => {
            let v = node.scalar_value.as_deref().unwrap_or(&node.summary);
            if v == "true" {
                ("true".to_string(), GraphAttrKind::BoolTrue)
            } else {
                ("false".to_string(), GraphAttrKind::BoolFalse)
            }
        }
        "null" => ("null".to_string(), GraphAttrKind::Null),
        "DateTime" => (
            format!("\"{}\"", truncate_chars(&node.summary, 44)),
            GraphAttrKind::String,
        ),
        t if t.starts_with("string") => {
            let raw = node.scalar_value.as_deref().unwrap_or(&node.summary);
            let inner = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(raw)
                .replace('\n', "\\n")
                .replace('\r', "");
            (
                format!("\"{}\"", truncate_chars(&inner, 46)),
                GraphAttrKind::String,
            )
        }
        t if t.starts_with("array:0") => ("[]".to_string(), GraphAttrKind::Null),
        _ => (truncate_chars(&node.summary, 46), GraphAttrKind::Other),
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{head}...")
    }
}

fn measure_entity(
    entity: &ObjectNode,
    path: &str,
    depth: usize,
    expanded_relations: &HashSet<String>,
) -> MeasuredEntity {
    let attrs = extract_entity_attrs(entity);
    let card_height = CARD_PAD_Y * 2.0 + (attrs.len().max(1) as f32) * ATTR_LINE_H;

    let mut relations = Vec::new();
    if depth < MAX_GRAPH_DEPTH {
        for (ch, targets) in extract_entity_relations(entity) {
            let rel_id = format!("{path}/{}", ch.label);
            let count = targets.len();
            let expanded = expanded_relations.contains(&rel_id) && count > 0;
            let pill_text_len = ch.label.chars().count() + 10;
            let pill_width = ((pill_text_len as f32) * 7.8 + 38.0).max(PILL_MIN_WIDTH);

            let measured_targets: Vec<MeasuredEntity> = if expanded {
                targets
                    .iter()
                    .enumerate()
                    .map(|(t_idx, target)| {
                        let child_path = format!("{rel_id}[{t_idx}]");
                        measure_entity(target, &child_path, depth + 1, expanded_relations)
                    })
                    .collect()
            } else {
                Vec::new()
            };

            let children_span = if measured_targets.is_empty() {
                0.0
            } else {
                let sum: f32 = measured_targets.iter().map(|t| t.subtree_width).sum();
                sum + ((measured_targets.len() - 1) as f32) * H_GAP
            };

            let rel_width = pill_width.max(children_span);
            relations.push(MeasuredRelation {
                rel_id,
                label: ch.label.clone(),
                count,
                expanded,
                pill_width,
                targets: measured_targets,
                rel_width,
            });
        }
    }

    let rels_total_w = if relations.is_empty() {
        0.0
    } else {
        let sum: f32 = relations.iter().map(|r| r.rel_width).sum();
        sum + ((relations.len() - 1) as f32) * H_GAP
    };

    MeasuredEntity {
        attrs,
        card_height,
        relations,
        subtree_width: CARD_WIDTH.max(rels_total_w),
    }
}

fn place_entity(m: &MeasuredEntity, left_x: f32, top_y: f32, out: &mut GraphLayout) -> f32 {
    let card_x = left_x + (m.subtree_width - CARD_WIDTH) * 0.5;
    let card_center_x = card_x + CARD_WIDTH * 0.5;
    let card_bottom_y = top_y + m.card_height;

    out.cards.push(PositionedCard {
        x: card_x,
        y: top_y,
        width: CARD_WIDTH,
        height: m.card_height,
        attrs: m.attrs.clone(),
    });

    if m.relations.is_empty() {
        return card_center_x;
    }

    let rels_row_w: f32 = m.relations.iter().map(|r| r.rel_width).sum::<f32>()
        + ((m.relations.len() - 1) as f32) * H_GAP;
    let mut rel_cursor_x = left_x + (m.subtree_width - rels_row_w) * 0.5;
    let pill_y = card_bottom_y + ARROW_TOP_LEN;

    for rel in &m.relations {
        let pill_x = rel_cursor_x + (rel.rel_width - rel.pill_width) * 0.5;
        let pill_center_x = pill_x + rel.pill_width * 0.5;

        out.edges.push(GraphEdge {
            from_x: card_center_x,
            from_y: card_bottom_y,
            to_x: pill_center_x,
            to_y: pill_y,
        });

        out.pills.push(PositionedPill {
            rel_id: rel.rel_id.clone(),
            label: rel.label.clone(),
            count: rel.count,
            expanded: rel.expanded,
            x: pill_x,
            y: pill_y,
            width: rel.pill_width,
            height: PILL_HEIGHT,
        });

        if rel.expanded && !rel.targets.is_empty() {
            let children_span: f32 = rel.targets.iter().map(|t| t.subtree_width).sum::<f32>()
                + ((rel.targets.len() - 1) as f32) * H_GAP;
            let mut child_cursor_x = rel_cursor_x + (rel.rel_width - children_span) * 0.5;
            let child_top_y = pill_y + PILL_HEIGHT + ARROW_BOTTOM_LEN;

            for target in &rel.targets {
                let child_center_x = place_entity(target, child_cursor_x, child_top_y, out);
                out.edges.push(GraphEdge {
                    from_x: pill_center_x,
                    from_y: pill_y + PILL_HEIGHT,
                    to_x: child_center_x,
                    to_y: child_top_y,
                });
                child_cursor_x += target.subtree_width + H_GAP;
            }
        }

        rel_cursor_x += rel.rel_width + H_GAP;
    }

    card_center_x
}

// ==================== GPUI INTERACTIVE OBJECT GRAPH ENTITY ====================

pub struct ObjectGraphView {
    root_node: Option<ObjectNode>,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    is_panning: bool,
    drag_origin: Option<Point<Pixels>>,
    pan_origin: (f32, f32),
    press_pos: Option<Point<Pixels>>,
    expanded_relations: HashSet<String>,
    needs_initial_fit: bool,
    last_bounds: Option<Bounds<Pixels>>,
    pill_hit_boxes: Vec<(Bounds<Pixels>, String)>,
}

impl ObjectGraphView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            root_node: None,
            zoom: 1.0,
            pan_x: 48.0,
            pan_y: 44.0,
            is_panning: false,
            drag_origin: None,
            pan_origin: (48.0, 44.0),
            press_pos: None,
            expanded_relations: HashSet::new(),
            needs_initial_fit: true,
            last_bounds: None,
            pill_hit_boxes: Vec::new(),
        }
    }

    pub fn set_root(&mut self, node: ObjectNode, cx: &mut Context<Self>) {
        self.expanded_relations = default_expanded_relations(&node);
        self.root_node = Some(node);
        self.zoom = 1.0;
        self.pan_x = 48.0;
        self.pan_y = 44.0;
        self.needs_initial_fit = true;
        cx.notify();
    }

    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.zoom_by(1.2, None, cx);
    }

    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.zoom_by(1.0 / 1.2, None, cx);
    }

    pub fn fit_view(&mut self, cx: &mut Context<Self>) {
        self.needs_initial_fit = true;
        cx.notify();
    }

    pub fn zoom_percent(&self) -> usize {
        (self.zoom * 100.0).round() as usize
    }

    fn zoom_by(
        &mut self,
        factor: f32,
        anchor_screen: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let old_zoom = self.zoom;
        let new_zoom = (old_zoom * factor).clamp(0.30, 2.40);
        if (new_zoom - old_zoom).abs() < 0.001 {
            return;
        }

        if let Some(bounds) = self.last_bounds {
            let ax = anchor_screen
                .map(|p| f32::from(p.x - bounds.left()))
                .unwrap_or(f32::from(bounds.size.width) * 0.5);
            let ay = anchor_screen
                .map(|p| f32::from(p.y - bounds.top()))
                .unwrap_or(f32::from(bounds.size.height) * 0.5);

            let world_x = (ax - self.pan_x) / old_zoom;
            let world_y = (ay - self.pan_y) / old_zoom;

            self.pan_x = ax - world_x * new_zoom;
            self.pan_y = ay - world_y * new_zoom;
        }

        self.zoom = new_zoom;
        cx.notify();
    }

    fn apply_fit(&mut self, bounds: Bounds<Pixels>, layout: &GraphLayout) {
        let vw = f32::from(bounds.size.width);
        let vh = f32::from(bounds.size.height);
        if vw <= 40.0 || vh <= 40.0 || layout.total_width <= 1.0 || layout.total_height <= 1.0 {
            return;
        }
        let pad_x = 72.0;
        let pad_y = 64.0;
        let zx = ((vw - pad_x) / layout.total_width).min(1.05);
        let zy = ((vh - pad_y) / layout.total_height).min(1.05);
        let z = zx.min(zy).clamp(0.42, 1.0);
        self.zoom = z;
        self.pan_x = ((vw - layout.total_width * z) * 0.5).max(28.0);
        self.pan_y = ((vh - layout.total_height * z) * 0.42).clamp(28.0, 96.0);
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_panning = true;
        self.drag_origin = Some(event.position);
        self.press_pos = Some(event.position);
        self.pan_origin = (self.pan_x, self.pan_y);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_panning {
            return;
        }
        if let Some(origin) = self.drag_origin {
            let dx = f32::from(event.position.x - origin.x);
            let dy = f32::from(event.position.y - origin.y);
            self.pan_x = self.pan_origin.0 + dx;
            self.pan_y = self.pan_origin.1 + dy;
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let was_click = self
            .press_pos
            .map(|p| {
                let dx = f32::from(event.position.x - p.x).abs();
                let dy = f32::from(event.position.y - p.y).abs();
                dx < 5.0 && dy < 5.0
            })
            .unwrap_or(false);

        self.is_panning = false;
        self.drag_origin = None;
        self.press_pos = None;

        if was_click {
            if let Some((_, rel_id)) = self
                .pill_hit_boxes
                .iter()
                .find(|(rect, _)| rect.contains(&event.position))
                .cloned()
            {
                if self.expanded_relations.contains(&rel_id) {
                    self.expanded_relations.remove(&rel_id);
                } else {
                    self.expanded_relations.insert(rel_id);
                }
                cx.notify();
                return;
            }
        }
        cx.notify();
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let dy = match event.delta {
            ScrollDelta::Pixels(p) => f32::from(p.y),
            ScrollDelta::Lines(l) => l.y * 24.0,
        };
        if dy.abs() < 0.01 {
            return;
        }
        let factor = (1.0 + dy * 0.0022).clamp(0.80, 1.25);
        self.zoom_by(factor, Some(event.position), cx);
    }
}

impl Render for ObjectGraphView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = if self.is_panning {
            CursorStyle::ClosedHand
        } else {
            CursorStyle::OpenHand
        };

        div()
            .id("object-graph-surface")
            .size_full()
            .bg(rgb(0x14161a))
            .cursor(cursor)
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(ObjectGraphCanvasElement { view: cx.entity() })
    }
}

struct ObjectGraphCanvasElement {
    view: Entity<ObjectGraphView>,
}

struct ShapedCardPaint {
    rect: Bounds<Pixels>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    line_h: Pixels,
}

struct ShapedPillPaint {
    rect: Bounds<Pixels>,
    rel_id: String,
    expanded: bool,
    text_pos: Point<Pixels>,
    text_line: ShapedLine,
    line_h: Pixels,
    zoom: f32,
}

struct GraphPrepaint {
    quads: Vec<PaintQuad>,
    cards: Vec<ShapedCardPaint>,
    pills: Vec<ShapedPillPaint>,
}

impl IntoElement for ObjectGraphCanvasElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ObjectGraphCanvasElement {
    type RequestLayoutState = ();
    type PrepaintState = GraphPrepaint;

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
        let (root_opt, expanded_rels, needs_fit) = {
            let v = self.view.read(cx);
            (
                v.root_node.clone(),
                v.expanded_relations.clone(),
                v.needs_initial_fit,
            )
        };

        let Some(root) = root_opt else {
            return GraphPrepaint {
                quads: Vec::new(),
                cards: Vec::new(),
                pills: Vec::new(),
            };
        };

        let layout = build_object_graph_layout(&root, &expanded_rels);

        let (zoom, pan_x, pan_y) = self.view.update(cx, |v, _| {
            v.last_bounds = Some(bounds);
            if needs_fit {
                v.needs_initial_fit = false;
                v.apply_fit(bounds, &layout);
            }
            (v.zoom, v.pan_x, v.pan_y)
        });

        let mut quads = Vec::new();
        let left = f32::from(bounds.left());
        let top = f32::from(bounds.top());
        let vw = f32::from(bounds.size.width);
        let vh = f32::from(bounds.size.height);

        // 1. Blueprint Grid Lines (minor + major)
        let minor_step = 24.0 * zoom;
        if minor_step >= 9.0 {
            let start_x = pan_x.rem_euclid(minor_step);
            let mut gx = start_x;
            while gx < vw {
                quads.push(fill(
                    Bounds::new(point(px(left + gx), bounds.top()), size(px(1.0), px(vh))),
                    rgb(0x1b1f27),
                ));
                gx += minor_step;
            }
            let start_y = pan_y.rem_euclid(minor_step);
            let mut gy = start_y;
            while gy < vh {
                quads.push(fill(
                    Bounds::new(point(bounds.left(), px(top + gy)), size(px(vw), px(1.0))),
                    rgb(0x1b1f27),
                ));
                gy += minor_step;
            }
        }

        let major_step = 96.0 * zoom;
        if major_step >= 18.0 {
            let start_x = pan_x.rem_euclid(major_step);
            let mut gx = start_x;
            while gx < vw {
                quads.push(fill(
                    Bounds::new(point(px(left + gx), bounds.top()), size(px(1.0), px(vh))),
                    rgb(0x232833),
                ));
                gx += major_step;
            }
            let start_y = pan_y.rem_euclid(major_step);
            let mut gy = start_y;
            while gy < vh {
                quads.push(fill(
                    Bounds::new(point(bounds.left(), px(top + gy)), size(px(vw), px(1.0))),
                    rgb(0x232833),
                ));
                gy += major_step;
            }
        }

        // 2. Orthogonal Relationship Connectors + Downward Arrowheads
        let edge_col = rgb(0x64748b);
        let stroke = (1.4 * zoom).clamp(1.0, 2.5);
        for edge in &layout.edges {
            let sx = left + pan_x + edge.from_x * zoom;
            let sy = top + pan_y + edge.from_y * zoom;
            let tx = left + pan_x + edge.to_x * zoom;
            let ty = top + pan_y + edge.to_y * zoom;
            let arrow_h = (6.0 * zoom).clamp(4.0, 9.0);
            let line_end_y = (ty - arrow_h).max(sy);

            if (sx - tx).abs() < 1.0 {
                // Straight vertical connector
                quads.push(fill(
                    Bounds::new(
                        point(px(sx - stroke * 0.5), px(sy)),
                        size(px(stroke), px((line_end_y - sy).max(1.0))),
                    ),
                    edge_col,
                ));
            } else {
                // Orthogonal stepped connector
                let mid_y = (sy + ty) * 0.5;
                quads.push(fill(
                    Bounds::new(
                        point(px(sx - stroke * 0.5), px(sy)),
                        size(px(stroke), px((mid_y - sy).max(1.0))),
                    ),
                    edge_col,
                ));
                let hx0 = sx.min(tx);
                let hw = (sx - tx).abs() + stroke;
                quads.push(fill(
                    Bounds::new(
                        point(px(hx0), px(mid_y - stroke * 0.5)),
                        size(px(hw), px(stroke)),
                    ),
                    edge_col,
                ));
                quads.push(fill(
                    Bounds::new(
                        point(px(tx - stroke * 0.5), px(mid_y)),
                        size(px(stroke), px((line_end_y - mid_y).max(1.0))),
                    ),
                    edge_col,
                ));
            }

            // Crisp downward triangle arrowhead touching (tx, ty)
            let steps = 4usize;
            for step in 0..steps {
                let frac = (steps - step) as f32 / (steps as f32);
                let half_w = (4.2 * zoom * frac).max(0.8);
                let slice_h = (arrow_h / (steps as f32)).max(1.0);
                let ay = ty - arrow_h + (step as f32) * slice_h;
                quads.push(fill(
                    Bounds::new(
                        point(px(tx - half_w), px(ay)),
                        size(px(half_w * 2.0), px(slice_h + 0.5)),
                    ),
                    edge_col,
                ));
            }
        }

        // 3. Shape Entity Cards
        let mono_font = Font {
            family: "Menlo".into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        };
        let font_px = px((11.0 * zoom).clamp(5.5, 24.0));
        let line_h = px((ATTR_LINE_H * zoom).max(8.0));

        let mut shaped_cards = Vec::with_capacity(layout.cards.len());
        for card in &layout.cards {
            let cx_px = left + pan_x + card.x * zoom;
            let cy_px = top + pan_y + card.y * zoom;
            let cw_px = card.width * zoom;
            let ch_px = card.height * zoom;

            // Cull completely offscreen cards
            if cx_px + cw_px < left - 20.0
                || cx_px > left + vw + 20.0
                || cy_px + ch_px < top - 20.0
                || cy_px > top + vh + 20.0
            {
                continue;
            }

            let rect = Bounds::new(point(px(cx_px), px(cy_px)), size(px(cw_px), px(ch_px)));
            let mut lines = Vec::with_capacity(card.attrs.len());

            for (idx, attr) in card.attrs.iter().enumerate() {
                let key_part = format!("{}: ", attr.key);
                let full_str = format!("{key_part}{}", attr.value);
                let val_col = match attr.kind {
                    GraphAttrKind::Number => rgb(0xfde047),
                    GraphAttrKind::String => rgb(0xf1f5f9),
                    GraphAttrKind::BoolTrue => rgb(0x34d399),
                    GraphAttrKind::BoolFalse => rgb(0xf87171),
                    GraphAttrKind::Null => rgb(0x94a3b8),
                    GraphAttrKind::Other => rgb(0xe2e8f0),
                };
                let runs = [
                    TextRun {
                        len: key_part.len(),
                        font: mono_font.clone(),
                        color: rgb(0x60a5fa).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    },
                    TextRun {
                        len: attr.value.len(),
                        font: mono_font.clone(),
                        color: val_col.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    },
                ];
                let shaped = window.text_system().shape_line(
                    SharedString::from(full_str),
                    font_px,
                    &runs,
                    None,
                );
                let lx = px(cx_px + CARD_PAD_X * zoom);
                let ly = px(cy_px + (CARD_PAD_Y + (idx as f32) * ATTR_LINE_H) * zoom);
                lines.push((point(lx, ly), shaped));
            }

            shaped_cards.push(ShapedCardPaint {
                rect,
                lines,
                line_h,
            });
        }

        // 4. Shape Relationship Pills (`[ category  (1)  oo ]`)
        let mut shaped_pills = Vec::with_capacity(layout.pills.len());
        let pill_font_px = px((11.5 * zoom).clamp(6.0, 24.0));
        let pill_lh = px((18.0 * zoom).max(8.0));

        for pill in &layout.pills {
            let px_x = left + pan_x + pill.x * zoom;
            let px_y = top + pan_y + pill.y * zoom;
            let pw = pill.width * zoom;
            let ph = pill.height * zoom;
            let rect = Bounds::new(point(px(px_x), px(px_y)), size(px(pw), px(ph)));

            let rel_part = format!("{}  ", pill.label);
            let cnt_part = format!("({})", pill.count);
            let full_str = format!("{rel_part}{cnt_part}");
            let runs = [
                TextRun {
                    len: rel_part.len(),
                    font: Font {
                        weight: FontWeight::MEDIUM,
                        ..mono_font.clone()
                    },
                    color: rgb(0x60a5fa).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                },
                TextRun {
                    len: cnt_part.len(),
                    font: mono_font.clone(),
                    color: rgb(0xe2e8f0).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                },
            ];
            let text_line = window.text_system().shape_line(
                SharedString::from(full_str),
                pill_font_px,
                &runs,
                None,
            );
            let tx = px(px_x + 12.0 * zoom);
            let ty = px(px_y + (pill.height - 18.0) * 0.5 * zoom);

            shaped_pills.push(ShapedPillPaint {
                rect,
                rel_id: pill.rel_id.clone(),
                expanded: pill.expanded,
                text_pos: point(tx, ty),
                text_line,
                line_h: pill_lh,
                zoom,
            });
        }

        GraphPrepaint {
            quads,
            cards: shaped_cards,
            pills: shaped_pills,
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
        let mut hit_boxes = Vec::with_capacity(prepaint.pills.len());

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // 1. Grid + Connectors
            for quad in prepaint.quads.drain(..) {
                window.paint_quad(quad);
            }

            // 2. Entity Attribute Cards
            for card in prepaint.cards.drain(..) {
                // Outer 1px border + inner dark slate body
                window.paint_quad(fill(card.rect, rgb(0x52607d)));
                let inner = Bounds::new(
                    point(card.rect.left() + px(1.0), card.rect.top() + px(1.0)),
                    size(
                        (card.rect.size.width - px(2.0)).max(px(1.0)),
                        (card.rect.size.height - px(2.0)).max(px(1.0)),
                    ),
                );
                window.paint_quad(fill(inner, rgb(0x222634)));

                window.with_content_mask(Some(ContentMask { bounds: inner }), |window| {
                    for (pos, line) in card.lines {
                        let _ = line.paint(pos, card.line_h, window, cx);
                    }
                });
            }

            // 3. Relationship Pill Boxes (`[ category  (1)  👓 ]`)
            for pill in prepaint.pills.drain(..) {
                hit_boxes.push((pill.rect, pill.rel_id));

                let border_col = if pill.expanded {
                    rgb(0x64789e)
                } else {
                    rgb(0x475569)
                };
                let bg_col = if pill.expanded {
                    rgb(0x262c3d)
                } else {
                    rgb(0x1e2230)
                };

                window.paint_quad(fill(pill.rect, border_col));
                let inner = Bounds::new(
                    point(pill.rect.left() + px(1.0), pill.rect.top() + px(1.0)),
                    size(
                        (pill.rect.size.width - px(2.0)).max(px(1.0)),
                        (pill.rect.size.height - px(2.0)).max(px(1.0)),
                    ),
                );
                window.paint_quad(fill(inner, bg_col));

                let _ = pill.text_line.paint(pill.text_pos, pill.line_h, window, cx);

                // Crisp mini eyeglasses (`oo`) icon on right side of pill
                let z = pill.zoom;
                let lens = (5.5 * z).clamp(3.0, 12.0);
                let cy = f32::from(pill.rect.top()) + f32::from(pill.rect.size.height) * 0.5;
                let rx_right = f32::from(pill.rect.right()) - 11.0 * z;
                let lens_col = if pill.expanded {
                    rgb(0xe2e8f0)
                } else {
                    rgb(0x94a3b8)
                };
                let l1_x = rx_right - lens * 2.2;
                let l2_x = rx_right - lens;
                let top_y = cy - lens * 0.5;

                // Left lens + bridge + right lens
                paint_hollow_box(window, l1_x, top_y, lens, lens, lens_col, bg_col);
                window.paint_quad(fill(
                    Bounds::new(
                        point(px(l1_x + lens), px(cy - 0.6)),
                        size(px((l2_x - (l1_x + lens)).max(1.0)), px(1.2)),
                    ),
                    lens_col,
                ));
                paint_hollow_box(window, l2_x, top_y, lens, lens, lens_col, bg_col);
            }
        });

        self.view.update(cx, |v, _| {
            v.pill_hit_boxes = hit_boxes;
        });
    }
}

fn paint_hollow_box(
    window: &mut Window,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    stroke: gpui::Rgba,
    fill_bg: gpui::Rgba,
) {
    let outer = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
    window.paint_quad(fill(outer, stroke));
    if w > 2.2 && h > 2.2 {
        let inner = Bounds::new(
            point(px(x + 1.1), px(y + 1.1)),
            size(px(w - 2.2), px(h - 2.2)),
        );
        window.paint_quad(fill(inner, fill_bg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExecutionTarget;
    use crate::runner::{default_playground_path, evaluate_code};

    #[test]
    fn builds_article_collection_object_graph_with_category_relationships() {
        let pg = default_playground_path();
        let target = ExecutionTarget::Local { project_path: pg };
        let res = evaluate_code("$articles = Article::all();", &target);
        assert_eq!(res.status, "ok", "exception: {:?}", res.exception);

        let root = &res.return_value.as_ref().unwrap().node;
        let expanded = default_expanded_relations(root);
        let layout = build_object_graph_layout(root, &expanded);

        // 3 root Article cards + 3 expanded Category cards = 6 cards!
        assert_eq!(layout.cards.len(), 6);
        assert!(layout.pills.iter().any(|p| p.label == "category" && p.count == 1 && p.expanded));
        assert!(layout.pills.iter().any(|p| p.label == "author" && p.count == 1));
        assert!(layout.edges.len() >= 6);
    }
}
