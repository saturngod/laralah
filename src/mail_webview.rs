#![allow(unexpected_cfgs, deprecated)]

use gpui::{
    div, prelude::*, relative, App, Bounds, Context, Element, ElementId, Entity,
    GlobalElementId, LayoutId, Pixels, Render, Style, Window,
};

#[cfg(target_os = "macos")]
#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
mod mac {
    use cocoa::base::{id, nil, BOOL, NO, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
    use gpui::{Bounds, Pixels, Window};
    use objc::runtime::Class;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub struct NativeWkWebView {
        webview_ptr: usize,
        loaded_version: u64,
    }

    impl Default for NativeWkWebView {
        fn default() -> Self {
            Self {
                webview_ptr: 0,
                loaded_version: 0,
            }
        }
    }

    impl NativeWkWebView {
        pub fn sync(
            &mut self,
            window: &mut Window,
            bounds: Bounds<Pixels>,
            html: &str,
            version: u64,
        ) {
            let Ok(wh) = HasWindowHandle::window_handle(window) else {
                return;
            };
            let RawWindowHandle::AppKit(appkit) = wh.as_raw() else {
                return;
            };

            unsafe {
                let parent_view = appkit.ns_view.as_ptr() as id;
                if parent_view == nil {
                    return;
                }

                let bx = f32::from(bounds.left()) as f64;
                let by = f32::from(bounds.top()) as f64;
                let bw = (f32::from(bounds.size.width) as f64).max(1.0);
                let bh = (f32::from(bounds.size.height) as f64).max(1.0);

                let is_flipped: BOOL = msg_send![parent_view, isFlipped];
                let cocoa_y = if is_flipped == YES {
                    by
                } else {
                    let p_bounds: NSRect = msg_send![parent_view, bounds];
                    (p_bounds.size.height - by - bh).max(0.0)
                };
                let frame = NSRect::new(NSPoint::new(bx, cocoa_y), NSSize::new(bw, bh));

                if self.webview_ptr == 0 {
                    let Some(cfg_cls) = Class::get("WKWebViewConfiguration") else {
                        return;
                    };
                    let Some(wv_cls) = Class::get("WKWebView") else {
                        return;
                    };
                    let cfg: id = msg_send![cfg_cls, alloc];
                    let cfg: id = msg_send![cfg, init];
                    let wv: id = msg_send![wv_cls, alloc];
                    let wv: id = msg_send![wv, initWithFrame: frame configuration: cfg];
                    let _: () = msg_send![cfg, release];
                    if wv == nil {
                        return;
                    }
                    self.webview_ptr = wv as usize;
                }

                let wv = self.webview_ptr as id;
                let superview: id = msg_send![wv, superview];
                if superview != parent_view {
                    if superview != nil {
                        let _: () = msg_send![wv, removeFromSuperview];
                    }
                    let _: () = msg_send![parent_view, addSubview: wv];
                }
                let _: () = msg_send![wv, setHidden: NO];
                let _: () = msg_send![wv, setFrame: frame];

                if self.loaded_version != version {
                    self.loaded_version = version;
                    let ns_html = NSString::alloc(nil).init_str(html);
                    let _: id = msg_send![wv, loadHTMLString: ns_html baseURL: nil];
                    let _: () = msg_send![ns_html, release];
                }
            }
        }

        pub fn hide(&mut self) {
            if self.webview_ptr == 0 {
                return;
            }
            unsafe {
                let wv = self.webview_ptr as id;
                let _: () = msg_send![wv, setHidden: YES];
                let superview: id = msg_send![wv, superview];
                if superview != nil {
                    let _: () = msg_send![wv, removeFromSuperview];
                }
            }
        }
    }

    impl Drop for NativeWkWebView {
        fn drop(&mut self) {
            if self.webview_ptr != 0 {
                unsafe {
                    let wv = self.webview_ptr as id;
                    let superview: id = msg_send![wv, superview];
                    if superview != nil {
                        let _: () = msg_send![wv, removeFromSuperview];
                    }
                    let _: () = msg_send![wv, release];
                }
                self.webview_ptr = 0;
            }
        }
    }
}

pub struct MailWebView {
    html: String,
    version: u64,
    #[cfg(target_os = "macos")]
    native: mac::NativeWkWebView,
}

impl MailWebView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            html: String::new(),
            version: 0,
            #[cfg(target_os = "macos")]
            native: mac::NativeWkWebView::default(),
        }
    }

    pub fn set_html(&mut self, html: String, cx: &mut Context<Self>) {
        self.html = html;
        self.version = self.version.wrapping_add(1).max(1);
        cx.notify();
    }

    pub fn html(&self) -> &str {
        &self.html
    }

    pub fn hide(&mut self) {
        #[cfg(target_os = "macos")]
        self.native.hide();
    }
}

impl Render for MailWebView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("native-mail-webview-host")
            .size_full()
            .bg(crate::theme::rgb(0xedf2f7))
            .child(MailWebViewElement { view: cx.entity() })
    }
}

struct MailWebViewElement {
    view: Entity<MailWebView>,
}

impl IntoElement for MailWebViewElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MailWebViewElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(target_os = "macos")]
        {
            self.view.update(cx, |v, _| {
                let html = v.html.clone();
                let ver = v.version;
                v.native.sync(window, bounds, &html, ver);
            });
        }
    }
}
