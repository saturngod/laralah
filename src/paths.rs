use std::path::PathBuf;

/// The current user's home folder on every desktop platform: `$HOME` on macOS/Linux and
/// the user profile folder (`%USERPROFILE%`) on Windows, where `HOME` is normally unset.
/// Returns `None` when it cannot be determined or is not an absolute path.
pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir().filter(|p| p.is_absolute() && !p.as_os_str().is_empty())
}

/// Home folder as a display string (for prefix checks and `~` abbreviation).
pub fn home_dir_string() -> Option<String> {
    home_dir().map(|p| p.to_string_lossy().to_string())
}
