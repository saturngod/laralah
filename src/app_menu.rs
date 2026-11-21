//! The application menu, defined once and shown two ways: as the native menu
//! bar on macOS, and as the hamburger menu in the top bar on Windows and Linux
//! (which have no menu bar of their own in this app).

use crate::app::{
    CloseScratchTab, FormatCode, ModeDetailDive, ModeRawCli, ModeTable, NewScratchTab,
    NextScratchTab, OpenDirectory, OpenPlayground, PrevScratchTab, QuitApp, RunCode, ToggleAiSidebar, ToggleLogs,
    TogglePalette, ToggleSettings, ToggleSidebar, ToggleSql, ToggleSshModal,
};
use crate::editor::{Copy, Cut, Paste, Redo, SelectAll, TriggerCompletion, Undo};
use gpui::{Action, OsAction};

/// Where a menu item's shortcut label comes from.
#[derive(Clone, Copy)]
pub enum MenuShortcut {
    /// A rebindable shortcut from Settings → Shortcuts, by spec id.
    Spec(&'static str),
    /// A fixed binding, in gpui keystroke syntax.
    Keys(&'static str),
}

#[derive(Clone, Copy)]
pub struct AppMenuItem {
    pub label: &'static str,
    pub build: fn() -> Box<dyn Action>,
    pub shortcut: Option<MenuShortcut>,
    /// Lets native text fields handle the command (macOS menu bar only).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub os_action: Option<OsAction>,
}

impl AppMenuItem {
    /// Display label for the shortcut (`⇧⌘F`, `Ctrl+Shift+F`), empty when unbound.
    pub fn shortcut_label(&self) -> String {
        match self.shortcut {
            Some(MenuShortcut::Spec(id)) => crate::keymap::hint(id).to_string(),
            Some(MenuShortcut::Keys(keys)) => crate::keymap::display(keys),
            None => String::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum AppMenuEntry {
    Item(AppMenuItem),
    Separator,
}

pub struct AppMenu {
    pub name: &'static str,
    pub entries: Vec<AppMenuEntry>,
}

macro_rules! item {
    ($label:expr, $action:expr) => {
        AppMenuEntry::Item(AppMenuItem {
            label: $label,
            build: || Box::new($action),
            shortcut: None,
            os_action: None,
        })
    };
    ($label:expr, $action:expr, spec $id:expr) => {
        AppMenuEntry::Item(AppMenuItem {
            label: $label,
            build: || Box::new($action),
            shortcut: Some(MenuShortcut::Spec($id)),
            os_action: None,
        })
    };
    ($label:expr, $action:expr, keys $keys:expr, $os:expr) => {
        AppMenuEntry::Item(AppMenuItem {
            label: $label,
            build: || Box::new($action),
            shortcut: Some(MenuShortcut::Keys($keys)),
            os_action: Some($os),
        })
    };
}

const SEP: AppMenuEntry = AppMenuEntry::Separator;

/// Which kind of menu the entries are built for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuStyle {
    /// The macOS menu bar: Settings and Quit live in the "Laralah" app menu.
    MacMenuBar,
    /// The in-window hamburger menu (Windows/Linux): Settings and Exit end File.
    Hamburger,
}

/// The menus in display order.
pub fn app_menus(style: MenuStyle) -> Vec<AppMenu> {
    let mac = style == MenuStyle::MacMenuBar;
    let mut menus = Vec::new();

    if mac {
        menus.push(AppMenu {
            name: "Laralah",
            entries: vec![
                item!("Settings…", ToggleSettings, spec "open_settings"),
                SEP,
                item!("Quit Laralah", QuitApp, spec "quit"),
            ],
        });
    }

    let mut file = vec![
        item!("New Tab", NewScratchTab, spec "new_tab"),
        item!("Open Folder…", OpenDirectory, spec "open_folder"),
        item!("Open Playground", OpenPlayground, spec "open_playground"),
        item!("Connect to Server…", ToggleSshModal, spec "ssh_connections"),
        SEP,
        item!("Close Tab", CloseScratchTab, spec "close_tab"),
    ];
    if !mac {
        file.extend([
            SEP,
            item!("Settings…", ToggleSettings, spec "open_settings"),
            SEP,
            item!("Exit", QuitApp, spec "quit"),
        ]);
    }
    menus.push(AppMenu { name: "File", entries: file });

    menus.push(AppMenu {
        name: "Edit",
        entries: vec![
            item!("Undo", Undo, keys "secondary-z", OsAction::Undo),
            item!(
                "Redo",
                Redo,
                keys if cfg!(target_os = "macos") { "secondary-shift-z" } else { "secondary-y" },
                OsAction::Redo
            ),
            SEP,
            item!("Cut", Cut, keys "secondary-x", OsAction::Cut),
            item!("Copy", Copy, keys "secondary-c", OsAction::Copy),
            item!("Paste", Paste, keys "secondary-v", OsAction::Paste),
            SEP,
            item!("Select All", SelectAll, keys "secondary-a", OsAction::SelectAll),
        ],
    });

    menus.push(AppMenu {
        name: "Editor",
        entries: vec![
            item!("Run Code", RunCode, spec "run_code"),
            item!("Format Code", FormatCode, spec "format_code"),
            SEP,
            item!("Trigger Autocomplete", TriggerCompletion, spec "trigger_autocomplete"),
            SEP,
            item!("Previous Tab", PrevScratchTab, spec "prev_tab"),
            item!("Next Tab", NextScratchTab, spec "next_tab"),
        ],
    });

    menus.push(AppMenu {
        name: "View",
        entries: vec![
            item!("Command Palette…", TogglePalette, spec "command_palette"),
            SEP,
            item!("Project Explorer", ToggleSidebar, spec "toggle_explorer"),
            item!("AI Assistant", ToggleAiSidebar, spec "toggle_ai_sidebar"),
            item!("Logs", ToggleLogs, spec "toggle_logs"),
            item!("SQL Queries", ToggleSql, spec "toggle_sql"),
            SEP,
            item!("Detail Dive", ModeDetailDive, spec "mode_detail_dive"),
            item!("Table Preview", ModeTable, spec "mode_table"),
            item!("CLI Output", ModeRawCli, spec "mode_cli"),
        ],
    });

    menus
}

/// Installs the menus as the macOS menu bar. Key equivalents come from the
/// current keymap, so this runs again whenever shortcuts are rebound.
#[cfg(target_os = "macos")]
pub fn install_native(cx: &mut gpui::App) {
    use gpui::{Menu, MenuItem};
    let menus = app_menus(MenuStyle::MacMenuBar)
        .into_iter()
        .map(|menu| Menu {
            name: menu.name.into(),
            items: menu
                .entries
                .into_iter()
                .map(|entry| match entry {
                    AppMenuEntry::Separator => MenuItem::separator(),
                    AppMenuEntry::Item(item) => MenuItem::Action {
                        name: item.label.into(),
                        action: (item.build)(),
                        os_action: item.os_action,
                    },
                })
                .collect(),
        })
        .collect();
    cx.set_menus(menus);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_have_expected_sections_and_known_shortcut_ids() {
        for style in [MenuStyle::MacMenuBar, MenuStyle::Hamburger] {
            check_menus(style);
        }
        let hamburger: Vec<_> = app_menus(MenuStyle::Hamburger).iter().map(|m| m.name).collect();
        assert_eq!(hamburger, ["File", "Edit", "Editor", "View"]);
        assert_eq!(app_menus(MenuStyle::MacMenuBar)[0].name, "Laralah");
    }

    fn check_menus(style: MenuStyle) {
        let menus = app_menus(style);
        let names: Vec<_> = menus.iter().map(|m| m.name).collect();
        for wanted in ["File", "Edit", "Editor", "View"] {
            assert!(names.contains(&wanted), "missing {wanted} menu");
        }
        for menu in &menus {
            assert!(!matches!(menu.entries.first(), Some(AppMenuEntry::Separator)));
            assert!(!matches!(menu.entries.last(), Some(AppMenuEntry::Separator)));
            for entry in &menu.entries {
                if let AppMenuEntry::Item(AppMenuItem { shortcut: Some(MenuShortcut::Spec(id)), label, .. }) = entry {
                    assert!(crate::keymap::spec_by_id(id).is_some(), "{label}: unknown shortcut id {id}");
                }
            }
        }
        // Settings and Quit appear exactly once on every platform.
        let count = |label: &str| {
            menus
                .iter()
                .flat_map(|m| &m.entries)
                .filter(|e| matches!(e, AppMenuEntry::Item(i) if i.label.starts_with(label)))
                .count()
        };
        assert_eq!(count("Settings"), 1);
        assert_eq!(count("Quit") + count("Exit"), 1);
    }
}
