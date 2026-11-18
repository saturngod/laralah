#![cfg(target_os = "macos")]
#![allow(unexpected_cfgs)]

use cocoa::appkit::NSApplication;
use cocoa::base::{BOOL, NO, YES, id, nil};
use cocoa::foundation::{NSPoint, NSRect, NSUInteger};
use objc::runtime::{Class, Imp, Method, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

/// Interior window-edge resize margin in Cocoa points where macOS WindowServer
/// intercepts mouse-down to resize a resizable `NSWindow`.
const INNER_RESIZE_PT: f64 = 3.5;

/// Exterior window-edge resize halo in Cocoa points outside `NSWindow.frame`
/// where macOS WindowServer also allows resizing.
const OUTER_RESIZE_PT: f64 = 4.5;

/// Distance along each edge from a window corner treated as a diagonal corner resize zone.
const CORNER_RESIZE_PT: f64 = 12.0;

const NS_WINDOW_STYLE_MASK_RESIZABLE: NSUInteger = 1 << 3;
const NS_WINDOW_STYLE_MASK_FULL_SCREEN: NSUInteger = 1 << 14;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeZone {
    InsideContent = 0,
    ResizeNorthSouth = 1,
    ResizeEastWest = 2,
    ResizeNWSE = 3,
    ResizeNESW = 4,
    OutsideWindow = 5,
}

impl EdgeZone {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::ResizeNorthSouth,
            2 => Self::ResizeEastWest,
            3 => Self::ResizeNWSE,
            4 => Self::ResizeNESW,
            5 => Self::OutsideWindow,
            _ => Self::InsideContent,
        }
    }

    pub(crate) fn is_resize(self) -> bool {
        matches!(
            self,
            Self::ResizeNorthSouth | Self::ResizeEastWest | Self::ResizeNWSE | Self::ResizeNESW
        )
    }
}

/// Pure geometry classifier for a point `(mx, my)` relative to a window frame
/// `(x, y, width, height)` in bottom-left Cocoa screen coordinates.
pub(crate) fn classify_window_edge_zone(
    mx: f64,
    my: f64,
    win_x: f64,
    win_y: f64,
    win_w: f64,
    win_h: f64,
) -> EdgeZone {
    if win_w <= 0.0 || win_h <= 0.0 {
        return EdgeZone::InsideContent;
    }

    let left = win_x;
    let right = win_x + win_w;
    let bottom = win_y;
    let top = win_y + win_h;

    if mx < left - OUTER_RESIZE_PT
        || mx > right + OUTER_RESIZE_PT
        || my < bottom - OUTER_RESIZE_PT
        || my > top + OUTER_RESIZE_PT
    {
        return EdgeZone::OutsideWindow;
    }

    let near_left = mx < left + INNER_RESIZE_PT;
    let near_right = mx > right - INNER_RESIZE_PT;
    let near_bottom = my < bottom + INNER_RESIZE_PT;
    let near_top = my > top - INNER_RESIZE_PT;

    if !(near_left || near_right || near_bottom || near_top) {
        return EdgeZone::InsideContent;
    }

    let corner_left = mx < left + CORNER_RESIZE_PT;
    let corner_right = mx > right - CORNER_RESIZE_PT;
    let corner_bottom = my < bottom + CORNER_RESIZE_PT;
    let corner_top = my > top - CORNER_RESIZE_PT;

    if (near_top && corner_left)
        || (near_left && corner_top)
        || (near_bottom && corner_right)
        || (near_right && corner_bottom)
    {
        return EdgeZone::ResizeNWSE;
    }

    if (near_top && corner_right)
        || (near_right && corner_top)
        || (near_bottom && corner_left)
        || (near_left && corner_bottom)
    {
        return EdgeZone::ResizeNESW;
    }

    if near_bottom || near_top {
        EdgeZone::ResizeNorthSouth
    } else {
        EdgeZone::ResizeEastWest
    }
}

unsafe extern "C" {
    fn class_getInstanceMethod(cls: *const Class, name: Sel) -> *mut Method;
    fn method_getImplementation(method: *const Method) -> Imp;
    fn method_getTypeEncoding(method: *const Method) -> *const std::os::raw::c_char;
    fn class_addMethod(
        cls: *mut Class,
        name: Sel,
        imp: Imp,
        types: *const std::os::raw::c_char,
    ) -> BOOL;
    fn method_setImplementation(method: *mut Method, imp: Imp) -> Imp;
}

type CursorSetFn = unsafe extern "C" fn(*mut Object, Sel);
type ViewMouseEventFn = unsafe extern "C" fn(*mut Object, Sel, id);
type WindowBoolGetterFn = unsafe extern "C" fn(*mut Object, Sel) -> BOOL;
type ShouldZoomAtPointFn = unsafe extern "C" fn(*mut Object, Sel, NSPoint) -> BOOL;
type HandleDoubleClickModifiersFn = unsafe extern "C" fn(*mut Object, Sel, NSUInteger) -> BOOL;
type HandleDoubleClickEventFn = unsafe extern "C" fn(*mut Object, Sel, id) -> BOOL;
type GestureShouldAttemptFn = unsafe extern "C" fn(*mut Object, Sel, id, id) -> BOOL;

static INSTALL_ONCE: Once = Once::new();
static DISABLE_TITLEBAR_DBL_CLICK_ONCE: Once = Once::new();

static ORIG_NSCURSOR_SET: AtomicUsize = AtomicUsize::new(0);
static ORIG_GPUIVIEW_MOUSE_MOVED: AtomicUsize = AtomicUsize::new(0);
static ORIG_GPUIVIEW_MOUSE_DOWN: AtomicUsize = AtomicUsize::new(0);

static ORIG_HANDLE_DBL_CLICK_MODS: AtomicUsize = AtomicUsize::new(0);
static ORIG_TITLED_FRAME_DBL_CLICK_GESTURE: AtomicUsize = AtomicUsize::new(0);
static ORIG_TITLEBAR_CONTAINER_DBL_CLICK: AtomicUsize = AtomicUsize::new(0);
static ORIG_THEME_FRAME_GESTURE_SHOULD_ATTEMPT: AtomicUsize = AtomicUsize::new(0);
static ORIG_THEME_FRAME_HANDLE_DBL_TAP: AtomicUsize = AtomicUsize::new(0);

static CURSOR_ARROW: AtomicUsize = AtomicUsize::new(0);
static CURSOR_NS: AtomicUsize = AtomicUsize::new(0);
static CURSOR_EW: AtomicUsize = AtomicUsize::new(0);
static CURSOR_NWSE: AtomicUsize = AtomicUsize::new(0);
static CURSOR_NESW: AtomicUsize = AtomicUsize::new(0);

static LAST_CONTENT_CURSOR: AtomicUsize = AtomicUsize::new(0);
static IN_EDGE_RESIZE_ZONE: AtomicBool = AtomicBool::new(false);
static WAS_MOUSE_PRESSED: AtomicBool = AtomicBool::new(false);
static LATCHED_ZONE: AtomicU8 = AtomicU8::new(EdgeZone::InsideContent as u8);

fn is_cached_resize_cursor(ptr: usize) -> bool {
    ptr != 0
        && (ptr == CURSOR_NS.load(Ordering::Relaxed)
            || ptr == CURSOR_EW.load(Ordering::Relaxed)
            || ptr == CURSOR_NWSE.load(Ordering::Relaxed)
            || ptr == CURSOR_NESW.load(Ordering::Relaxed))
}

fn cursor_for_zone(zone: EdgeZone) -> id {
    let ptr = match zone {
        EdgeZone::ResizeNorthSouth => CURSOR_NS.load(Ordering::Relaxed),
        EdgeZone::ResizeEastWest => CURSOR_EW.load(Ordering::Relaxed),
        EdgeZone::ResizeNWSE => CURSOR_NWSE.load(Ordering::Relaxed),
        EdgeZone::ResizeNESW => CURSOR_NESW.load(Ordering::Relaxed),
        _ => CURSOR_ARROW.load(Ordering::Relaxed),
    };
    ptr as id
}

unsafe fn apply_raw_cursor(cursor: id) {
    if cursor == nil {
        return;
    }
    let raw = ORIG_NSCURSOR_SET.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    unsafe {
        let current: id = msg_send![class!(NSCursor), currentCursor];
        if current != cursor {
            let orig: CursorSetFn = std::mem::transmute(raw);
            orig(cursor as *mut Object, sel!(set));
        }
    }
}

unsafe fn compute_unlatched_zone() -> EdgeZone {
    unsafe {
        let app = NSApplication::sharedApplication(nil);
        if app == nil {
            return EdgeZone::InsideContent;
        }
        let key_window: id = msg_send![app, keyWindow];
        if key_window == nil {
            return EdgeZone::InsideContent;
        }
        let Some(gpui_win_cls) = Class::get("GPUIWindow") else {
            return EdgeZone::InsideContent;
        };
        let is_gpui_window: BOOL = msg_send![key_window, isKindOfClass: gpui_win_cls];
        if is_gpui_window == NO {
            return EdgeZone::InsideContent;
        }

        let style_mask: NSUInteger = msg_send![key_window, styleMask];
        if (style_mask & NS_WINDOW_STYLE_MASK_RESIZABLE) == 0
            || (style_mask & NS_WINDOW_STYLE_MASK_FULL_SCREEN) != 0
        {
            return EdgeZone::InsideContent;
        }

        let frame: NSRect = msg_send![key_window, frame];
        let mouse: NSPoint = msg_send![class!(NSEvent), mouseLocation];
        classify_window_edge_zone(
            mouse.x,
            mouse.y,
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        )
    }
}

unsafe fn current_edge_zone() -> EdgeZone {
    let pressed: NSUInteger = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };
    if pressed == 0 {
        WAS_MOUSE_PRESSED.store(false, Ordering::Relaxed);
        let zone = unsafe { compute_unlatched_zone() };
        LATCHED_ZONE.store(zone as u8, Ordering::Relaxed);
        zone
    } else if !WAS_MOUSE_PRESSED.swap(true, Ordering::Relaxed) {
        let zone = unsafe { compute_unlatched_zone() };
        LATCHED_ZONE.store(zone as u8, Ordering::Relaxed);
        zone
    } else {
        EdgeZone::from_u8(LATCHED_ZONE.load(Ordering::Relaxed))
    }
}

extern "C" fn swizzled_nscursor_set(this: *mut Object, cmd: Sel) {
    let this_ptr = this as usize;
    if this_ptr != 0 && !is_cached_resize_cursor(this_ptr) {
        LAST_CONTENT_CURSOR.store(this_ptr, Ordering::Relaxed);
    }

    let zone = unsafe { current_edge_zone() };
    match zone {
        EdgeZone::InsideContent => {
            let raw = ORIG_NSCURSOR_SET.load(Ordering::Relaxed);
            if raw != 0 {
                unsafe {
                    let orig: CursorSetFn = std::mem::transmute(raw);
                    orig(this, cmd);
                }
            }
        }
        zone if zone.is_resize() => {
            IN_EDGE_RESIZE_ZONE.store(true, Ordering::Relaxed);
            unsafe { apply_raw_cursor(cursor_for_zone(zone)) };
        }
        EdgeZone::OutsideWindow => {
            // Do not let GPUI's active-window redraws overwrite the system cursor
            // when the pointer is outside our GPUIWindow.
        }
        _ => {}
    }
}

extern "C" fn swizzled_gpuiview_mouse_down(this: *mut Object, cmd: Sel, event: id) {
    // Any mouseDown: delivered to GPUIView is an interior content interaction
    // (window-frame resizes are intercepted by AppKit before reaching contentView).
    WAS_MOUSE_PRESSED.store(true, Ordering::Relaxed);
    LATCHED_ZONE.store(EdgeZone::InsideContent as u8, Ordering::Relaxed);
    IN_EDGE_RESIZE_ZONE.store(false, Ordering::Relaxed);

    let raw = ORIG_GPUIVIEW_MOUSE_DOWN.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let orig: ViewMouseEventFn = std::mem::transmute(raw);
            orig(this, cmd, event);
        }
    }
}

extern "C" fn swizzled_gpuiview_mouse_moved(this: *mut Object, cmd: Sel, event: id) {
    let zone = unsafe { current_edge_zone() };

    let raw = ORIG_GPUIVIEW_MOUSE_MOVED.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let orig: ViewMouseEventFn = std::mem::transmute(raw);
            orig(this, cmd, event);
        }
    }

    match zone {
        zone if zone.is_resize() => {
            IN_EDGE_RESIZE_ZONE.store(true, Ordering::Relaxed);
            unsafe { apply_raw_cursor(cursor_for_zone(zone)) };
        }
        EdgeZone::InsideContent => {
            if IN_EDGE_RESIZE_ZONE.swap(false, Ordering::Relaxed) {
                let target = LAST_CONTENT_CURSOR.load(Ordering::Relaxed);
                let fallback = CURSOR_ARROW.load(Ordering::Relaxed);
                let chosen = if target != 0 { target } else { fallback };
                unsafe { apply_raw_cursor(chosen as id) };
            }
        }
        EdgeZone::OutsideWindow => {
            if IN_EDGE_RESIZE_ZONE.swap(false, Ordering::Relaxed) {
                unsafe { apply_raw_cursor(CURSOR_ARROW.load(Ordering::Relaxed) as id) };
            }
        }
        _ => {}
    }
}

unsafe fn resolve_and_retain_cursor(primary_sel: Sel, fallback_sel: Sel) -> usize {
    unsafe {
        let cls = class!(NSCursor);
        let has_primary: BOOL = msg_send![cls, respondsToSelector: primary_sel];
        let cur: id = if has_primary == YES {
            msg_send![cls, performSelector: primary_sel]
        } else {
            msg_send![cls, performSelector: fallback_sel]
        };
        if cur != nil {
            let _: id = msg_send![cur, retain];
        }
        cur as usize
    }
}

/// Prevents GPUI 0.2.2 from clobbering macOS's native window-edge resize cursors
/// back to the arrow/pointer cursor when hovering or dragging window borders.
pub fn install_window_edge_cursor_guard() {
    INSTALL_ONCE.call_once(|| unsafe {
        let arrow = resolve_and_retain_cursor(sel!(arrowCursor), sel!(arrowCursor));
        let ns = resolve_and_retain_cursor(
            sel!(_windowResizeNorthSouthCursor),
            sel!(resizeUpDownCursor),
        );
        let ew = resolve_and_retain_cursor(
            sel!(_windowResizeEastWestCursor),
            sel!(resizeLeftRightCursor),
        );
        let nwse = resolve_and_retain_cursor(
            sel!(_windowResizeNorthWestSouthEastCursor),
            sel!(crosshairCursor),
        );
        let nesw = resolve_and_retain_cursor(
            sel!(_windowResizeNorthEastSouthWestCursor),
            sel!(crosshairCursor),
        );

        CURSOR_ARROW.store(arrow, Ordering::Relaxed);
        CURSOR_NS.store(ns, Ordering::Relaxed);
        CURSOR_EW.store(ew, Ordering::Relaxed);
        CURSOR_NWSE.store(nwse, Ordering::Relaxed);
        CURSOR_NESW.store(nesw, Ordering::Relaxed);
        LAST_CONTENT_CURSOR.store(arrow, Ordering::Relaxed);

        let cursor_cls = class!(NSCursor) as *const Class;
        let set_method = class_getInstanceMethod(cursor_cls, sel!(set));
        if !set_method.is_null() {
            let new_imp: Imp = std::mem::transmute(swizzled_nscursor_set as CursorSetFn);
            let prev_imp = method_setImplementation(set_method, new_imp);
            ORIG_NSCURSOR_SET.store(prev_imp as usize, Ordering::Relaxed);
        }

        if let Some(gpui_view_cls) = Class::get("GPUIView") {
            let view_cls_ptr = gpui_view_cls as *const Class;

            let moved_method = class_getInstanceMethod(view_cls_ptr, sel!(mouseMoved:));
            if !moved_method.is_null() {
                let new_imp: Imp =
                    std::mem::transmute(swizzled_gpuiview_mouse_moved as ViewMouseEventFn);
                let prev_imp = method_setImplementation(moved_method, new_imp);
                ORIG_GPUIVIEW_MOUSE_MOVED.store(prev_imp as usize, Ordering::Relaxed);
            }

            let down_method = class_getInstanceMethod(view_cls_ptr, sel!(mouseDown:));
            if !down_method.is_null() {
                let new_imp: Imp =
                    std::mem::transmute(swizzled_gpuiview_mouse_down as ViewMouseEventFn);
                let prev_imp = method_setImplementation(down_method, new_imp);
                ORIG_GPUIVIEW_MOUSE_DOWN.store(prev_imp as usize, Ordering::Relaxed);
            }
        }
    });
}

unsafe fn is_gpui_window_ptr(win: id) -> bool {
    if win == nil {
        return false;
    }
    let Some(gpui_win_cls) = Class::get("GPUIWindow") else {
        return false;
    };
    let is_gpui: BOOL = unsafe { msg_send![win, isKindOfClass: gpui_win_cls] };
    is_gpui == YES
}

unsafe fn is_frame_for_gpui_window(frame_view: *mut Object) -> bool {
    if frame_view.is_null() {
        return false;
    }
    let win: id = unsafe { msg_send![frame_view, window] };
    unsafe { is_gpui_window_ptr(win) }
}

unsafe fn is_double_click_window_gesture(frame_view: *mut Object, gesture: id) -> bool {
    if gesture == nil {
        return false;
    }
    unsafe {
        if !frame_view.is_null() {
            let has_zoom_check: BOOL =
                msg_send![frame_view, respondsToSelector: sel!(gestureRecognizerIsDoubleClickZoomGR:)];
            if has_zoom_check == YES {
                let is_zoom_gr: BOOL =
                    msg_send![frame_view, gestureRecognizerIsDoubleClickZoomGR: gesture];
                if is_zoom_gr == YES {
                    return true;
                }
            }
        }
        let has_action: BOOL = msg_send![gesture, respondsToSelector: sel!(action)];
        if has_action == YES {
            let action: Sel = msg_send![gesture, action];
            if action == sel!(_handleTitledFrameDoubleClickGesture:)
                || action == sel!(handleDoubleTapOrClickGesture:)
            {
                return true;
            }
        }
    }
    false
}

extern "C" fn gpui_window_return_no(_this: *mut Object, _cmd: Sel) -> BOOL {
    NO
}

extern "C" fn gpui_window_should_zoom_at_location(
    _this: *mut Object,
    _cmd: Sel,
    _location: NSPoint,
) -> BOOL {
    NO
}

extern "C" fn swizzled_titled_frame_double_click_mods(
    this: *mut Object,
    cmd: Sel,
    modifiers: NSUInteger,
) -> BOOL {
    if unsafe { is_frame_for_gpui_window(this) } {
        return NO;
    }
    let raw = ORIG_HANDLE_DBL_CLICK_MODS.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let orig: HandleDoubleClickModifiersFn = std::mem::transmute(raw);
            return orig(this, cmd, modifiers);
        }
    }
    NO
}

extern "C" fn swizzled_titled_frame_double_click_gesture(
    this: *mut Object,
    cmd: Sel,
    gesture: id,
) {
    if unsafe { is_frame_for_gpui_window(this) } {
        return;
    }
    let raw = ORIG_TITLED_FRAME_DBL_CLICK_GESTURE.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let orig: ViewMouseEventFn = std::mem::transmute(raw);
            orig(this, cmd, gesture);
        }
    }
}

extern "C" fn swizzled_titlebar_container_double_click(
    this: *mut Object,
    cmd: Sel,
    event: id,
) -> BOOL {
    if unsafe { is_frame_for_gpui_window(this) } {
        return NO;
    }
    let raw = ORIG_TITLEBAR_CONTAINER_DBL_CLICK.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let orig: HandleDoubleClickEventFn = std::mem::transmute(raw);
            return orig(this, cmd, event);
        }
    }
    NO
}

extern "C" fn swizzled_theme_frame_gesture_should_attempt(
    this: *mut Object,
    cmd: Sel,
    gesture: id,
    event: id,
) -> BOOL {
    if unsafe { is_frame_for_gpui_window(this) && is_double_click_window_gesture(this, gesture) } {
        return NO;
    }
    let raw = ORIG_THEME_FRAME_GESTURE_SHOULD_ATTEMPT.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let orig: GestureShouldAttemptFn = std::mem::transmute(raw);
            return orig(this, cmd, gesture, event);
        }
    }
    NO
}

extern "C" fn swizzled_theme_frame_handle_double_tap(
    this: *mut Object,
    cmd: Sel,
    gesture: id,
) {
    if unsafe { is_frame_for_gpui_window(this) } {
        return;
    }
    let raw = ORIG_THEME_FRAME_HANDLE_DBL_TAP.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let orig: ViewMouseEventFn = std::mem::transmute(raw);
            orig(this, cmd, gesture);
        }
    }
}

unsafe fn override_or_swizzle_method(cls: *const Class, sel: Sel, new_imp: Imp) -> usize {
    unsafe {
        let method = class_getInstanceMethod(cls, sel);
        if method.is_null() {
            return 0;
        }
        let orig_imp = method_getImplementation(method);
        let types = method_getTypeEncoding(method);
        if class_addMethod(cls as *mut Class, sel, new_imp, types) == NO {
            method_setImplementation(method, new_imp);
        }
        orig_imp as usize
    }
}

/// Disables macOS's double-click-on-titlebar (maximize / zoom / fill / restore)
/// and double-click snap-resize gestures on `GPUIWindow`, while preserving
/// single-click window dragging, traffic-light buttons, and edge drag-resizing.
pub fn disable_titlebar_double_click_zoom() {
    DISABLE_TITLEBAR_DBL_CLICK_ONCE.call_once(|| unsafe {
        if let Some(gpui_win_cls) = Class::get("GPUIWindow") {
            let cls_ptr = gpui_win_cls as *const Class;
            let no_getter_imp: Imp =
                std::mem::transmute(gpui_window_return_no as WindowBoolGetterFn);
            let no_zoom_pt_imp: Imp =
                std::mem::transmute(gpui_window_should_zoom_at_location as ShouldZoomAtPointFn);

            override_or_swizzle_method(
                cls_ptr,
                sel!(titlebarHandlesDoubleClickWhenTransparent),
                no_getter_imp,
            );
            override_or_swizzle_method(cls_ptr, sel!(_shouldSnapSizeOnDoubleClick), no_getter_imp);
            override_or_swizzle_method(
                cls_ptr,
                sel!(_shouldZoomInDragRegionAtLocation:),
                no_zoom_pt_imp,
            );
        }

        if let Some(titled_frame_cls) = Class::get("NSTitledFrame") {
            let cls_ptr = titled_frame_cls as *const Class;

            let mods_imp: Imp = std::mem::transmute(
                swizzled_titled_frame_double_click_mods as HandleDoubleClickModifiersFn,
            );
            let prev = override_or_swizzle_method(
                cls_ptr,
                sel!(_handlePossibleDoubleClickWithModifiers:),
                mods_imp,
            );
            ORIG_HANDLE_DBL_CLICK_MODS.store(prev, Ordering::Relaxed);

            let gesture_imp: Imp =
                std::mem::transmute(swizzled_titled_frame_double_click_gesture as ViewMouseEventFn);
            let prev = override_or_swizzle_method(
                cls_ptr,
                sel!(_handleTitledFrameDoubleClickGesture:),
                gesture_imp,
            );
            ORIG_TITLED_FRAME_DBL_CLICK_GESTURE.store(prev, Ordering::Relaxed);

            let container_imp: Imp = std::mem::transmute(
                swizzled_titlebar_container_double_click as HandleDoubleClickEventFn,
            );
            let prev = override_or_swizzle_method(
                cls_ptr,
                sel!(_handlePossibleDoubleClickForTitlebarContainerViewWithEvent:),
                container_imp,
            );
            ORIG_TITLEBAR_CONTAINER_DBL_CLICK.store(prev, Ordering::Relaxed);
        }

        if let Some(theme_frame_cls) = Class::get("NSThemeFrame") {
            let cls_ptr = theme_frame_cls as *const Class;

            let attempt_imp: Imp = std::mem::transmute(
                swizzled_theme_frame_gesture_should_attempt as GestureShouldAttemptFn,
            );
            let prev = override_or_swizzle_method(
                cls_ptr,
                sel!(gestureRecognizer:shouldAttemptToRecognizeWithEvent:),
                attempt_imp,
            );
            ORIG_THEME_FRAME_GESTURE_SHOULD_ATTEMPT.store(prev, Ordering::Relaxed);

            let tap_imp: Imp =
                std::mem::transmute(swizzled_theme_frame_handle_double_tap as ViewMouseEventFn);
            let prev = override_or_swizzle_method(
                cls_ptr,
                sel!(handleDoubleTapOrClickGesture:),
                tap_imp,
            );
            ORIG_THEME_FRAME_HANDLE_DBL_TAP.store(prev, Ordering::Relaxed);
        }
    });
}

/// Applies per-window AppKit flags and disables double-click zoom/snap gesture
/// recognizers on all currently open `GPUIWindow` instances.
pub fn configure_gpui_windows() {
    unsafe {
        let app = NSApplication::sharedApplication(nil);
        if app == nil {
            return;
        }
        let windows: id = msg_send![app, windows];
        if windows == nil {
            return;
        }
        let count: NSUInteger = msg_send![windows, count];
        for i in 0..count {
            let win: id = msg_send![windows, objectAtIndex: i];
            configure_single_gpui_window(win);
        }
    }
}

pub(crate) unsafe fn configure_single_gpui_window(win: id) {
    unsafe {
        if !is_gpui_window_ptr(win) {
            return;
        }
        let has_setter: BOOL =
            msg_send![win, respondsToSelector: sel!(setTitlebarHandlesDoubleClickWhenTransparent:)];
        if has_setter == YES {
            let _: () = msg_send![win, setTitlebarHandlesDoubleClickWhenTransparent: NO];
        }

        let content_view: id = msg_send![win, contentView];
        if content_view == nil {
            return;
        }
        let frame_view: id = msg_send![content_view, superview];
        if frame_view == nil {
            return;
        }
        let gestures: id = msg_send![frame_view, gestureRecognizers];
        if gestures == nil {
            return;
        }
        let g_count: NSUInteger = msg_send![gestures, count];
        for idx in 0..g_count {
            let gr: id = msg_send![gestures, objectAtIndex: idx];
            if is_double_click_window_gesture(frame_view, gr) {
                let _: () = msg_send![gr, setEnabled: NO];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EdgeZone, classify_window_edge_zone};

    #[test]
    fn bottom_edge_inside_and_outside_halo_stay_resize_north_south() {
        // Window at (100, 200) size 1220x760 -> bottom is y = 200.0
        for &dy in &[-4.0, -2.0, -0.5, 0.0, 0.5, 2.0, 3.0] {
            let zone = classify_window_edge_zone(500.0, 200.0 + dy, 100.0, 200.0, 1220.0, 760.0);
            assert_eq!(
                zone,
                EdgeZone::ResizeNorthSouth,
                "expected ResizeNorthSouth at dy={dy}"
            );
        }
    }

    #[test]
    fn interior_footer_above_resize_margin_is_inside_content() {
        // 8pt above bottom edge (inside the 28px status footer)
        let zone = classify_window_edge_zone(500.0, 208.0, 100.0, 200.0, 1220.0, 760.0);
        assert_eq!(zone, EdgeZone::InsideContent);

        // Traffic lights area (x = +20, y = top - 16)
        let traffic = classify_window_edge_zone(120.0, 944.0, 100.0, 200.0, 1220.0, 760.0);
        assert_eq!(traffic, EdgeZone::InsideContent);
    }

    #[test]
    fn corners_and_side_edges_classify_accurately() {
        // Left & right edges
        assert_eq!(
            classify_window_edge_zone(101.0, 500.0, 100.0, 200.0, 1220.0, 760.0),
            EdgeZone::ResizeEastWest
        );
        assert_eq!(
            classify_window_edge_zone(1321.0, 500.0, 100.0, 200.0, 1220.0, 760.0),
            EdgeZone::ResizeEastWest
        );

        // Bottom-left (NESW) & bottom-right (NWSE) corners
        assert_eq!(
            classify_window_edge_zone(102.0, 201.0, 100.0, 200.0, 1220.0, 760.0),
            EdgeZone::ResizeNESW
        );
        assert_eq!(
            classify_window_edge_zone(1319.0, 201.0, 100.0, 200.0, 1220.0, 760.0),
            EdgeZone::ResizeNWSE
        );

        // Far below the window
        assert_eq!(
            classify_window_edge_zone(500.0, 180.0, 100.0, 200.0, 1220.0, 760.0),
            EdgeZone::OutsideWindow
        );
    }

    #[test]
    fn gpui_window_disables_titlebar_double_click_zoom_and_snap_resize() {
        use cocoa::appkit::{NSApplication, NSBackingStoreBuffered, NSWindowStyleMask};
        use cocoa::base::{BOOL, NO, YES, id, nil};
        use cocoa::foundation::{NSPoint, NSRect, NSSize, NSUInteger};
        use objc::runtime::{Class, Object};
        use objc::{class, msg_send, sel, sel_impl};

        super::disable_titlebar_double_click_zoom();

        unsafe {
            let gpui_win_cls = Class::get("GPUIWindow").expect("GPUIWindow class registered");
            let cls_ptr = gpui_win_cls as *const Class;

            // Verify class-level method overrides on GPUIWindow (safe on any thread)
            let transp_m = super::class_getInstanceMethod(
                cls_ptr,
                sel!(titlebarHandlesDoubleClickWhenTransparent),
            );
            assert!(!transp_m.is_null());
            let transp_fn: super::WindowBoolGetterFn =
                std::mem::transmute(super::method_getImplementation(transp_m));
            assert_eq!(
                transp_fn(
                    std::ptr::null_mut::<Object>(),
                    sel!(titlebarHandlesDoubleClickWhenTransparent)
                ),
                NO
            );

            let snap_m = super::class_getInstanceMethod(cls_ptr, sel!(_shouldSnapSizeOnDoubleClick));
            assert!(!snap_m.is_null());
            let snap_fn: super::WindowBoolGetterFn =
                std::mem::transmute(super::method_getImplementation(snap_m));
            assert_eq!(
                snap_fn(
                    std::ptr::null_mut::<Object>(),
                    sel!(_shouldSnapSizeOnDoubleClick)
                ),
                NO
            );

            let zoom_m = super::class_getInstanceMethod(
                cls_ptr,
                sel!(_shouldZoomInDragRegionAtLocation:),
            );
            assert!(!zoom_m.is_null());
            let zoom_fn: super::ShouldZoomAtPointFn =
                std::mem::transmute(super::method_getImplementation(zoom_m));
            assert_eq!(
                zoom_fn(
                    std::ptr::null_mut::<Object>(),
                    sel!(_shouldZoomInDragRegionAtLocation:),
                    NSPoint::new(400.0, 590.0)
                ),
                NO
            );

            // AppKit requires NSWindow instantiation on the main thread (run via `--test-threads=1`).
            let is_main_thread: BOOL = msg_send![class!(NSThread), isMainThread];
            if is_main_thread != YES {
                return;
            }

            let _app = NSApplication::sharedApplication(nil);
            let raw_win: id = msg_send![gpui_win_cls, alloc];
            let style_mask = NSWindowStyleMask::NSTitledWindowMask
                | NSWindowStyleMask::NSClosableWindowMask
                | NSWindowStyleMask::NSMiniaturizableWindowMask
                | NSWindowStyleMask::NSResizableWindowMask
                | NSWindowStyleMask::NSFullSizeContentViewWindowMask;
            let rect = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(800.0, 600.0));
            let win: id = msg_send![
                raw_win,
                initWithContentRect: rect
                styleMask: style_mask.bits()
                backing: NSBackingStoreBuffered
                defer: NO
            ];
            assert_ne!(win, nil);

            super::configure_single_gpui_window(win);

            let handles_transp: BOOL = msg_send![win, titlebarHandlesDoubleClickWhenTransparent];
            assert_eq!(handles_transp, NO);

            let should_snap: BOOL = msg_send![win, _shouldSnapSizeOnDoubleClick];
            assert_eq!(should_snap, NO);

            let should_zoom: BOOL = msg_send![
                win,
                _shouldZoomInDragRegionAtLocation: NSPoint::new(400.0, 590.0)
            ];
            assert_eq!(should_zoom, NO);

            let content_view: id = msg_send![win, contentView];
            let frame_view: id = msg_send![content_view, superview];
            assert_ne!(frame_view, nil);

            let dbl_click_handled: BOOL = msg_send![
                frame_view,
                _handlePossibleDoubleClickWithModifiers: 0usize as NSUInteger
            ];
            assert_eq!(dbl_click_handled, NO);

            let gestures: id = msg_send![frame_view, gestureRecognizers];
            if gestures != nil {
                let g_count: NSUInteger = msg_send![gestures, count];
                let mut saw_pan = false;
                for idx in 0..g_count {
                    let gr: id = msg_send![gestures, objectAtIndex: idx];
                    if super::is_double_click_window_gesture(frame_view, gr) {
                        let enabled: BOOL = msg_send![gr, isEnabled];
                        assert_eq!(
                            enabled, NO,
                            "double-click gesture recognizer should be disabled"
                        );
                        let should_attempt: BOOL = msg_send![
                            frame_view,
                            gestureRecognizer: gr
                            shouldAttemptToRecognizeWithEvent: nil
                        ];
                        assert_eq!(should_attempt, NO);
                    } else {
                        let action: objc::runtime::Sel = msg_send![gr, action];
                        if action == sel!(handlePanGesture:) {
                            let enabled: BOOL = msg_send![gr, isEnabled];
                            assert_eq!(enabled, YES, "pan gesture recognizer must stay enabled");
                            saw_pan = true;
                        }
                    }
                }
                assert!(
                    saw_pan,
                    "expected window drag/resize pan gesture recognizers to remain enabled"
                );
            }
            // Intentionally leak `win` in this test so GPUIWindow's custom `dealloc`
            // does not read an unpopulated `windowState` ivar.
        }
    }
}
