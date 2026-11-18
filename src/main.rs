// Release builds on Windows are GUI apps: no console window behind the main window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod app;
mod assets;
mod command_palette;
mod editor;
mod laravel_lsp;
mod log_viewer;
#[cfg(target_os = "macos")]
mod macos_window_cursor;
mod mail_webview;
mod models;
mod object_graph;
mod paths;
mod runner;
mod settings;
mod ssh;
mod theme;

use app::{
    CloseOverlay, CloseScratchTab, FocusLogSearch, FocusSettingsSearch, FocusSshSearch,
    GraphFitView, GraphZoomIn, GraphZoomOut, LaralahApp, ModeDetailDive, ModeRawCli, ModeTable,
    NewScratchTab, NextScratchTab, OpenDirectory, PaletteConfirm, PaletteDown, PaletteUp,
    PrevScratchTab, QuitApp, RunCode, SubmitAiPrompt, ToggleAiSidebar, ToggleLogs, TogglePalette,
    ToggleSettings, ToggleSidebar, ToggleSql, ToggleSshModal,
};
#[cfg(target_os = "macos")]
use assets::MACOS_APP_ICON_PNG;
use assets::EmbeddedAssets;
use editor::bind_editor_keys;
use gpui::{
    point, px, size, App, AppContext, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem,
    TitlebarOptions, WindowBounds, WindowOptions,
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

            bind_editor_keys(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-q", QuitApp, None),
                KeyBinding::new("cmd-,", ToggleSettings, None),
                KeyBinding::new("cmd-i", ToggleAiSidebar, None),
                KeyBinding::new("cmd-b", ToggleSidebar, None),
                KeyBinding::new("cmd-t", NewScratchTab, None),
                KeyBinding::new("cmd-w", CloseScratchTab, None),
                KeyBinding::new("cmd-shift-[", PrevScratchTab, None),
                KeyBinding::new("ctrl-shift-tab", PrevScratchTab, None),
                KeyBinding::new("cmd-shift-]", NextScratchTab, None),
                KeyBinding::new("ctrl-tab", NextScratchTab, None),
                KeyBinding::new("cmd-k", ToggleSshModal, None),
                KeyBinding::new("cmd-o", OpenDirectory, None),
                KeyBinding::new("cmd-r", RunCode, None),
                KeyBinding::new("cmd-p", TogglePalette, None),
                KeyBinding::new("cmd-shift-p", TogglePalette, None),
                KeyBinding::new("cmd-l", ToggleLogs, None),
                KeyBinding::new("cmd-s", ToggleSql, None),
                KeyBinding::new("cmd-1", ModeDetailDive, None),
                KeyBinding::new("cmd-2", ModeTable, None),
                KeyBinding::new("cmd-3", ModeRawCli, None),
                KeyBinding::new("escape", CloseOverlay, None),
                KeyBinding::new("up", PaletteUp, Some("CommandPalette")),
                KeyBinding::new("down", PaletteDown, Some("CommandPalette")),
                KeyBinding::new("enter", PaletteConfirm, Some("CommandPalette")),
                KeyBinding::new("enter", SubmitAiPrompt, Some("AiComposer")),
                KeyBinding::new("cmd-enter", SubmitAiPrompt, Some("AiComposer")),
                KeyBinding::new("cmd-f", FocusLogSearch, Some("LogViewer")),
                KeyBinding::new("cmd-f", FocusSettingsSearch, Some("SettingsModal")),
                KeyBinding::new("cmd-f", FocusSshSearch, Some("SshModal")),
                KeyBinding::new("cmd-=", GraphZoomIn, Some("ObjectGraph")),
                KeyBinding::new("cmd-+", GraphZoomIn, Some("ObjectGraph")),
                KeyBinding::new("=", GraphZoomIn, Some("ObjectGraph")),
                KeyBinding::new("cmd--", GraphZoomOut, Some("ObjectGraph")),
                KeyBinding::new("-", GraphZoomOut, Some("ObjectGraph")),
                KeyBinding::new("cmd-0", GraphFitView, Some("ObjectGraph")),
                KeyBinding::new("0", GraphFitView, Some("ObjectGraph")),
            ]);

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
