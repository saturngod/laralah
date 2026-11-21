//! Platform-aware labels for keyboard shortcuts.
//!
//! Key bindings use gpui's `secondary` modifier (Cmd on macOS, Ctrl elsewhere), so
//! the hints shown in the UI must follow the same rule. Labels are written once in
//! macOS notation (`⌘R`, `⇧⌘P`, `⌘ + B`, `Cmd+O`) and [`label`] rewrites them to
//! `Ctrl+R`, `Ctrl+Shift+P`, `Ctrl+B`, `Ctrl+O` off macOS.

/// Returns `mac` unchanged on macOS, or its Windows/Linux spelling elsewhere.
///
/// The result is `&'static str` so it can go anywhere a string literal could.
/// Converted labels are cached, so each distinct label is allocated only once.
#[cfg(target_os = "macos")]
pub fn label(mac: &'static str) -> &'static str {
    mac
}

#[cfg(not(target_os = "macos"))]
pub fn label(mac: &'static str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    *cache.entry(mac).or_insert_with(|| {
        let leaked: &'static str = Box::leak(to_ctrl(mac).into_boxed_str());
        leaked
    })
}

/// Rewrites a macOS shortcut label into Ctrl/Shift notation.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn to_ctrl(mac: &str) -> String {
    mac.replace("⇧⌘", "Ctrl+Shift+")
        .replace("⌘ + ", "Ctrl+")
        .replace("⌘ ", "Ctrl+")
        .replace('⌘', "Ctrl+")
        .replace("Cmd+", "Ctrl+")
        .replace('⌃', "Ctrl+")
        .replace('⇧', "Shift+")
}

#[cfg(test)]
mod tests {
    use super::to_ctrl;

    #[test]
    fn rewrites_mac_labels_to_ctrl() {
        assert_eq!(to_ctrl("⌘R"), "Ctrl+R");
        assert_eq!(to_ctrl("⇧⌘P"), "Ctrl+Shift+P");
        assert_eq!(to_ctrl("⌘P / ⇧⌘P"), "Ctrl+P / Ctrl+Shift+P");
        assert_eq!(to_ctrl("Project Explorer (⌘ + B)"), "Project Explorer (Ctrl+B)");
        assert_eq!(to_ctrl("⌘ O"), "Ctrl+O");
        assert_eq!(to_ctrl("Cmd+,"), "Ctrl+,");
        assert_eq!(to_ctrl("⌃Space"), "Ctrl+Space");
        assert_eq!(to_ctrl("⌘+ / ⌘- / ⌘0"), "Ctrl++ / Ctrl+- / Ctrl+0");
        assert_eq!(to_ctrl("Esc"), "Esc");
    }
}
