// Release builds on Windows are GUI apps: no console window behind the main window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod app;
mod assets;
mod command_palette;
mod editor;
mod keymap;
mod laravel_lsp;
mod log_viewer;
#[cfg(target_os = "macos")]
mod macos_window_cursor;
mod mail_webview;
mod models;
mod object_graph;
mod paths;
mod runner;
mod selectable;
mod shortcut;
mod settings;
mod ssh;
mod theme;

use app::LaralahApp;
#[cfg(target_os = "macos")]
use app::{QuitApp, ToggleSettings};
#[cfg(target_os = "macos")]
use assets::MACOS_APP_ICON_PNG;
use assets::EmbeddedAssets;
#[cfg(target_os = "macos")]
use gpui::{Menu, MenuItem};
use gpui::{
    point, px, size, App, AppContext, Application, Bounds, Focusable, TitlebarOptions,
    WindowBounds, WindowOptions,
};

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn configure_macos_app(cx: &mut App) {
    macos_window_cursor::install_window_edge_cursor_guard();
    macos_window_cursor::disable_titlebar_double_click_zoom();

    use cocoa::appkit::{NSApplication, NSImage};
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let process_info: id = msg_send![class!(NSProcessInfo), processInfo];
        if process_info != nil {
            let name = NSString::alloc(nil).init_str("Laralah");
            let _: () = msg_send![process_info, setProcessName: name];
        }

        let data: id = msg_send![
            class!(NSData),
            dataWithBytes: MACOS_APP_ICON_PNG.as_ptr() as *const std::ffi::c_void
            length: MACOS_APP_ICON_PNG.len()
        ];
        if data != nil {
            let image: id = NSImage::alloc(nil).initWithData_(data);
            if image != nil {
                let ns_app = NSApplication::sharedApplication(nil);
                ns_app.setApplicationIconImage_(image);
                let _: () = msg_send![image, release];
            }
        }
    }

    cx.set_menus(vec![Menu {
        name: "Laralah".into(),
        items: vec![
            MenuItem::action("Settings...", ToggleSettings),
            MenuItem::separator(),
            MenuItem::action("Quit Laralah", QuitApp),
        ],
    }]);
}

fn main() {
    if runner::run_askpass_if_requested() {
        return;
    }

    Application::new()
        .with_assets(EmbeddedAssets)
        .run(|cx: &mut App| {
            #[cfg(target_os = "macos")]
            configure_macos_app(cx);

            let bounds = Bounds::centered(None, size(px(1220.0), px(760.0)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        app_id: Some("Laralah".to_string()),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Laralah".into()),
                            appears_transparent: true,
                            traffic_light_position: Some(point(px(14.0), px(11.0))),
                        }),
                        ..Default::default()
                    },
                    |_, cx| cx.new(LaralahApp::new),
                )
                .expect("failed to open Laralah window");

            #[cfg(target_os = "macos")]
            macos_window_cursor::configure_gpui_windows();

            window
                .update(cx, |view, window, cx| {
                    theme::apply_native_appearance(view.theme_mode());
                    cx.observe_window_appearance(window, |_, window, _| window.refresh())
                        .detach();
                    let fh = view.focus_handle(cx);
                    window.focus(&fh);
                    cx.activate(true);
                })
                .ok();
        });
}
