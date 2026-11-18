//! App-wide light/dark theming.
//!
//! The UI is authored against a light palette (hex literals passed to `rgb` /
//! `rgba`). Rather than threading a theme struct through every render
//! function, UI modules import the theme-aware `rgb` / `rgba` from here. When
//! dark mode is active those remap each light-palette color to its dark
//! counterpart: an explicit table for the core palette, and an HSL-based
//! transform for everything else (neutrals are inverted, tinted backgrounds
//! become deep tints, accents are lifted slightly for contrast).
//!
//! Use `fixed_rgb` for colors that must never change (e.g. white text on a
//! saturated accent button).

use gpui::{Rgba, WindowAppearance};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

/// How colors built in the current render scope were authored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Authored for the light theme; remapped in dark mode.
    Normal,
    /// Authored dark (e.g. the settings panel); kept as-is in dark mode and
    /// remapped to a light version in light mode.
    DarkAuthored,
    /// Never remapped.
    Fixed,
}

thread_local! {
    static SCOPE: Cell<Scope> = const { Cell::new(Scope::Normal) };
}

/// Restores the previous scope when dropped.
pub struct ScopeGuard(Scope);

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        SCOPE.with(|s| s.set(self.0));
    }
}

pub fn enter_scope(scope: Scope) -> ScopeGuard {
    ScopeGuard(SCOPE.with(|s| s.replace(scope)))
}

/// Builds elements with colors interpreted in `scope`. Element builders run
/// eagerly, so every `rgb` call inside `f` sees the scope.
pub fn scoped<R>(scope: Scope, f: impl FnOnce() -> R) -> R {
    let _guard = enter_scope(scope);
    f()
}

fn current_scope() -> Scope {
    SCOPE.with(|s| s.get())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn label(&self) -> &'static str {
        match self {
            ThemeMode::System => "System",
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }

    pub fn resolve_dark(&self, appearance: WindowAppearance) -> bool {
        match self {
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
            ThemeMode::System => matches!(
                appearance,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        }
    }
}

/// Sets the native (AppKit) appearance so window chrome, menus and native
/// controls match. `System` clears the override so the app follows macOS.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn apply_native_appearance(mode: ThemeMode) {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let appearance: id = match mode {
            ThemeMode::System => nil,
            ThemeMode::Light | ThemeMode::Dark => {
                let name = if mode == ThemeMode::Dark {
                    "NSAppearanceNameDarkAqua"
                } else {
                    "NSAppearanceNameAqua"
                };
                let ns_name = NSString::alloc(nil).init_str(name);
                let appearance: id = msg_send![class!(NSAppearance), appearanceNamed: ns_name];
                let _: () = msg_send![ns_name, release];
                appearance
            }
        };
        let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
        if ns_app != nil {
            let _: () = msg_send![ns_app, setAppearance: appearance];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_native_appearance(_mode: ThemeMode) {}

static DARK: AtomicBool = AtomicBool::new(false);

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// Sets the active palette. Returns true when it changed.
pub fn set_dark(dark: bool) -> bool {
    DARK.swap(dark, Ordering::Relaxed) != dark
}

/// Theme-aware replacement for `gpui::rgb`.
pub fn rgb(hex: u32) -> Rgba {
    gpui::rgb(map_rgb(hex))
}

/// Theme-aware replacement for `gpui::rgba` (0xRRGGBBAA).
pub fn rgba(hex: u32) -> Rgba {
    let alpha = hex & 0xff;
    let color = hex >> 8;
    // Translucent dark colors are shadows / scrims; keep them dark.
    if alpha == 0 || (alpha < 0xff && lightness(color) < 0.25) {
        return gpui::rgba(hex);
    }
    gpui::rgba((map_rgb(color) << 8) | alpha)
}

/// An explicit per-theme color, for spots where the automatic mapping is not
/// good enough (e.g. accent text on tinted toolbar pills).
pub fn pick(light: u32, dark: u32) -> Rgba {
    gpui::rgb(if is_dark() { dark } else { light })
}

/// A color that stays the same in every theme.
pub fn fixed_rgb(hex: u32) -> Rgba {
    gpui::rgb(hex)
}

/// Maps a 0xRRGGBB color for the active theme.
pub fn map_rgb(hex: u32) -> u32 {
    match (current_scope(), is_dark()) {
        (Scope::Normal, true) => dark_variant(hex),
        (Scope::DarkAuthored, false) => light_variant(hex),
        _ => hex,
    }
}

/// Light-theme counterpart of a color authored for a dark surface.
fn light_variant(hex: u32) -> u32 {
    let hex = hex & 0xffffff;
    if let Some(mapped) = light_table(hex) {
        return mapped;
    }
    let (h, s, l) = to_hsl(hex);
    let (r, g, b) = channels(hex);
    let chroma = r.max(g).max(b) - r.min(g).min(b);

    let (s2, l2) = if chroma < 0.19 {
        (s.min(0.25), invert_neutral_to_light(l))
    } else if l < 0.3 {
        // Deep tinted surface -> pale tint.
        (s.min(0.85), 0.95)
    } else if l > 0.6 {
        // Light accent meant for dark surfaces -> deeper accent.
        (s, 0.42 + (l - 0.6) * 0.3)
    } else {
        (s, l)
    };
    from_hsl(h, s2, l2)
}

fn invert_neutral_to_light(l: f32) -> f32 {
    const POINTS: [(f32, f32); 9] = [
        (0.0, 1.0),
        (0.1, 0.985),
        (0.2, 0.93),
        (0.35, 0.80),
        (0.5, 0.62),
        (0.65, 0.46),
        (0.8, 0.30),
        (0.9, 0.20),
        (1.0, 0.10),
    ];
    for w in POINTS.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if l <= x1 {
            return y0 + (l - x0) / (x1 - x0) * (y1 - y0);
        }
    }
    0.10
}

fn light_table(hex: u32) -> Option<u32> {
    Some(match hex {
        // Surfaces of the dark-authored panels
        0x232833 => 0xffffff,
        0x21242c => 0xffffff,
        0x21252b => 0xffffff,
        0x1f2430 => 0xffffff,
        0x1c2028 => 0xf8fafc,
        0x1f232b => 0xf8fafc,
        0x202530 => 0xf8fafc,
        0x181c23 => 0xf1f5f9,
        0x171a21 => 0xf6f8fa,
        0x334056 => 0xffffff,
        0x364052 => 0xe2e8f0,
        0x343d4e => 0xe2e8f0,
        // Text
        0xffffff => 0x0f172a,
        0xf8fafc => 0x0f172a,
        0xf3f4f6 => 0x111827,
        0xf1f5f9 => 0x1e293b,
        0xe2e8f0 => 0x1e293b,
        0xe5e7eb => 0x1f2937,
        0xd1d5db => 0x374151,
        0xcbd5e1 => 0x334155,
        0x9ca3af => 0x6b7280,
        0x94a3b8 => 0x64748b,
        0x64748b => 0x94a3b8,
        _ => return None,
    })
}

fn dark_variant(hex: u32) -> u32 {
    let hex = hex & 0xffffff;
    if let Some(mapped) = dark_table(hex) {
        return mapped;
    }
    let (h, s, l) = to_hsl(hex);
    let (r, g, b) = channels(hex);
    let chroma = r.max(g).max(b) - r.min(g).min(b);

    let (s2, l2) = if l > 0.85 && s > 0.6 {
        // Light tinted surface (e.g. error/info backgrounds) -> deep tint.
        (s.min(0.45), 0.13 + (1.0 - l) * 0.6)
    } else if chroma < 0.19 {
        // Neutral gray -> invert lightness into a comfortable range.
        (s.min(0.18), 0.085 + (1.0 - l) * 0.83)
    } else if l < 0.4 {
        // Dark accent (text on tinted backgrounds) -> light accent.
        (s, 0.72 + (0.4 - l) * 0.3)
    } else if l > 0.85 {
        (s, 0.25)
    } else {
        // Mid accent: lift so it reads on dark and deep-tinted surfaces.
        (s, (l + 0.15).clamp(0.6, 0.75))
    };
    from_hsl(h, s2, l2)
}

fn dark_table(hex: u32) -> Option<u32> {
    Some(match hex {
        // Surfaces
        0xffffff => 0x16181d,
        0xfbfbfc => 0x1a1d23,
        0xfafafa => 0x1a1d23,
        0xf8fafc => 0x1b1e24,
        0xf6f8fa => 0x111317,
        0xf3f4f6 => 0x21252c,
        0xf1f5f9 => 0x23272f,
        0xeef2f6 => 0x2a2f38,
        0xe5e7eb => 0x2c313a,
        0xe2e8f0 => 0x323844,
        0xd1d5db => 0x3f4652,
        0xcbd5e1 => 0x475569,
        // Text
        0x000000 => 0xf8fafc,
        0x0f172a => 0xf1f5f9,
        0x111827 => 0xf3f4f6,
        0x1e293b => 0xe2e8f0,
        0x1f2937 => 0xe5e7eb,
        0x334155 => 0xcbd5e1,
        0x374151 => 0xd1d5db,
        0x475569 => 0xa8b4c4,
        0x4b5563 => 0xa8b0bd,
        0x64748b => 0x94a3b8,
        0x6b7280 => 0x9ca3af,
        0x94a3b8 => 0x6f7a8c,
        0x9ca3af => 0x7b8494,
        _ => return None,
    })
}

fn channels(hex: u32) -> (f32, f32, f32) {
    (
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

fn lightness(hex: u32) -> f32 {
    to_hsl(hex).2
}

fn to_hsl(hex: u32) -> (f32, f32, f32) {
    let (r, g, b) = channels(hex);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> u32 {
    let l = l.clamp(0.0, 1.0);
    let s = s.clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| (((v + m) * 255.0).round().clamp(0.0, 255.0)) as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips() {
        for hex in [0x2563eb, 0xef4444, 0x10b981, 0x64748b, 0xfef2f2, 0x7c3aed] {
            let (h, s, l) = to_hsl(hex);
            let back = from_hsl(h, s, l);
            for shift in [16, 8, 0] {
                let a = ((hex >> shift) & 0xff) as i32;
                let b = ((back >> shift) & 0xff) as i32;
                assert!((a - b).abs() <= 1, "{hex:06x} -> {back:06x}");
            }
        }
    }

    #[test]
    fn dark_authored_scope_flips_only_in_light_mode() {
        set_dark(false);
        let _g = enter_scope(Scope::DarkAuthored);
        assert_eq!(map_rgb(0x232833), 0xffffff);
        assert!(lightness(map_rgb(0x2a313e)) > 0.85);
        set_dark(true);
        assert_eq!(map_rgb(0x232833), 0x232833);
        set_dark(false);
    }

    #[test]
    fn dark_variants_flip_surfaces_and_text() {
        assert!(lightness(dark_variant(0xffffff)) < 0.15);
        assert!(lightness(dark_variant(0x1e293b)) > 0.8);
        // Tinted error background becomes a deep red, not gray.
        let err = dark_variant(0xfef2f2);
        let (r, g, _) = channels(err);
        assert!(lightness(err) < 0.25 && r > g);
        // Accent blue stays blue.
        let (_, s, _) = to_hsl(dark_variant(0x2563eb));
        assert!(s > 0.6);
    }
}
