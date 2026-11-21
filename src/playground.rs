//! The Playground: a throwaway Laravel app that Laralah creates and manages for the
//! user, so they can try PHP and Laravel code without opening a project of their own.
//!
//! It lives in `~/.laralah/playground`, uses the latest stable `laravel/laravel`
//! release from Composer, and keeps its data in a SQLite file
//! (`database/database.sqlite`), so it needs nothing besides PHP and Composer.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Folder the playground app is installed into.
pub fn playground_dir() -> PathBuf {
    crate::paths::home_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join(".laralah")
        .join("playground")
}

/// True for the playground folder (so it can be titled and listed differently).
pub fn is_playground_path(path: &str) -> bool {
    Path::new(path) == playground_dir()
}

/// The playground is usable once Composer has finished and the app can boot.
pub fn is_playground_ready() -> bool {
    let dir = playground_dir();
    dir.join("artisan").is_file()
        && dir.join("vendor").join("autoload.php").is_file()
        && dir.join(".env").is_file()
}

/// Folders to search for `composer` (and to put on `PATH` for Composer itself, whose
/// launcher runs `php` through `env`). A desktop app started from the Dock or Finder
/// does not get the login shell's `PATH`, so the usual install locations are added.
fn tool_search_dirs(php_bin: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(parent) = Path::new(php_bin).parent().filter(|p| !p.as_os_str().is_empty()) {
        dirs.push(parent.to_path_buf());
    }
    if let Ok(path_var) = std::env::var("PATH") {
        dirs.extend(std::env::split_paths(&path_var));
    }
    if let Some(home) = crate::paths::home_dir() {
        dirs.push(home.join("Library/Application Support/Herd/bin"));
        dirs.push(home.join(".config/herd-lite/bin"));
        dirs.push(home.join(".composer/vendor/bin"));
        dirs.push(home.join(".config/composer/vendor/bin"));
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        dirs.push(PathBuf::from(d));
    }
    if cfg!(windows) {
        if let Ok(pd) = std::env::var("ProgramData") {
            dirs.push(PathBuf::from(pd).join("ComposerSetup").join("bin"));
        }
        if let Ok(app) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(app).join("Composer").join("vendor").join("bin"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// True when opening the playground has to fetch Laravel with Composer first (as
/// opposed to opening it, or only finishing the local SQLite setup).
pub fn needs_download() -> bool {
    let dir = playground_dir();
    !(dir.join("artisan").is_file() && dir.join("vendor").join("autoload.php").is_file())
}

/// The playground folder as people write it on their platform: `~/.laralah/playground`
/// on macOS and Linux, and the full `C:\Users\…\.laralah\playground` on Windows,
/// where `~` means nothing in Explorer.
pub fn display_dir() -> String {
    display_path_for(&playground_dir(), crate::paths::home_dir().as_deref(), cfg!(windows))
}

fn display_path_for(dir: &Path, home: Option<&Path>, windows: bool) -> String {
    if !windows
        && let Some(home) = home
        && let Ok(rest) = dir.strip_prefix(home)
    {
        let parts: Vec<String> = rest.iter().map(|c| c.to_string_lossy().to_string()).collect();
        return format!("~/{}", parts.join("/"));
    }
    dir.to_string_lossy().to_string()
}

/// How to start Composer: either its launcher directly, or `php composer.phar`.
enum ComposerCmd {
    Launcher(PathBuf),
    Phar(PathBuf),
}

fn find_composer(dirs: &[PathBuf]) -> Option<ComposerCmd> {
    let launchers: &[&str] = if cfg!(windows) {
        &["composer.bat", "composer.cmd", "composer.exe"]
    } else {
        &["composer"]
    };
    for dir in dirs {
        for name in launchers {
            let p = dir.join(name);
            if p.is_file() {
                return Some(ComposerCmd::Launcher(p));
            }
        }
    }
    for dir in dirs {
        let p = dir.join("composer.phar");
        if p.is_file() {
            return Some(ComposerCmd::Phar(p));
        }
    }
    None
}

fn path_env(dirs: &[PathBuf]) -> Option<std::ffi::OsString> {
    std::env::join_paths(dirs.iter().filter(|d| d.is_dir())).ok()
}

/// Runs a command to completion and turns a failure into an error carrying the tail
/// of its output, which is where Composer and Artisan explain what went wrong.
fn run_checked(mut cmd: Command, what: &str) -> Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().with_context(|| format!("could not start {what}"))?;
    if out.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&out.stderr).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&out.stdout).to_string();
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    bail!("{what} failed:\n{tail}");
}

/// Sets `KEY=value` in a `.env` file, replacing an existing line (commented or not).
fn set_env_value(env: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut lines: Vec<String> = env
        .lines()
        .map(|line| {
            let bare = line.trim_start().trim_start_matches('#').trim_start();
            if !found && bare.starts_with(&format!("{key}=")) {
                found = true;
                format!("{key}={value}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("{key}={value}"));
    }
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// Comments out a `.env` key so Laravel falls back to its default.
fn comment_env_value(env: &str, key: &str) -> String {
    let mut s = env
        .lines()
        .map(|line| {
            if line.trim_start().starts_with(&format!("{key}=")) {
                format!("# {}", line.trim_start())
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    s.push('\n');
    s
}

/// Points the app at its SQLite file and keeps everything else local (file sessions
/// and cache aren't needed: the database driver works fine on SQLite).
fn configure_env(dir: &Path) -> Result<()> {
    let env_path = dir.join(".env");
    if !env_path.is_file() {
        std::fs::copy(dir.join(".env.example"), &env_path)
            .context("could not create .env from .env.example")?;
    }
    let mut env = std::fs::read_to_string(&env_path).context("could not read .env")?;
    env = set_env_value(&env, "APP_NAME", "Playground");
    env = set_env_value(&env, "APP_ENV", "local");
    env = set_env_value(&env, "DB_CONNECTION", "sqlite");
    for key in ["DB_HOST", "DB_PORT", "DB_DATABASE", "DB_USERNAME", "DB_PASSWORD"] {
        env = comment_env_value(&env, key);
    }
    env = set_env_value(&env, "MAIL_MAILER", "log");
    std::fs::write(&env_path, env).context("could not write .env")?;

    let db = dir.join("database").join("database.sqlite");
    if !db.is_file() {
        std::fs::create_dir_all(dir.join("database")).ok();
        std::fs::write(&db, b"").context("could not create database/database.sqlite")?;
    }
    Ok(())
}

fn artisan(php_bin: &str, dir: &Path, path: &Option<std::ffi::OsString>, args: &[&str]) -> Command {
    let mut c = Command::new(php_bin);
    c.arg("artisan").args(args).current_dir(dir);
    if let Some(p) = path {
        c.env("PATH", p);
    }
    c
}

/// Finishes setup inside an installed app: `.env`, app key, SQLite file, migrations.
/// Safe to run again on an existing playground.
fn finish_setup(php_bin: &str, dir: &Path, path: &Option<std::ffi::OsString>) -> Result<()> {
    configure_env(dir)?;
    let env = std::fs::read_to_string(dir.join(".env")).unwrap_or_default();
    let has_key = env
        .lines()
        .any(|l| l.starts_with("APP_KEY=") && l.trim_start_matches("APP_KEY=").trim().len() > 8);
    if !has_key {
        run_checked(
            artisan(php_bin, dir, path, &["key:generate", "--force", "--no-interaction"]),
            "php artisan key:generate",
        )?;
    }
    run_checked(
        artisan(php_bin, dir, path, &["package:discover", "--no-interaction"]),
        "php artisan package:discover",
    )?;
    run_checked(
        artisan(php_bin, dir, path, &["migrate", "--force", "--no-interaction"]),
        "php artisan migrate",
    )?;
    Ok(())
}

/// Creates the playground with the latest stable Laravel (or repairs a half-finished
/// one). `progress` is told about each step so the UI can show it.
pub fn install_playground(php_bin: &str, progress: impl Fn(&str)) -> Result<PathBuf> {
    let php_bin = if php_bin.trim().is_empty() {
        crate::runner::resolve_php_binary()
    } else {
        php_bin.trim().to_string()
    };
    let dir = playground_dir();
    let dirs = tool_search_dirs(&php_bin);
    let path = path_env(&dirs);

    // Composer finished before but setup didn't: just finish it.
    if dir.join("artisan").is_file() && dir.join("vendor").join("autoload.php").is_file() {
        progress("Setting up SQLite…");
        finish_setup(&php_bin, &dir, &path)?;
        return Ok(dir);
    }

    let Some(composer) = find_composer(&dirs) else {
        bail!(
            "Composer was not found. Install it from https://getcomposer.org (or with Laravel Herd), then try again."
        );
    };

    let parent = dir.parent().context("playground folder has no parent")?;
    std::fs::create_dir_all(parent).context("could not create ~/.laralah")?;
    // Install next to the final folder and move it in place at the end, so a failed or
    // interrupted download never leaves a broken playground behind.
    let staging = parent.join("playground.installing");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).context("could not clear an old unfinished install")?;
    }

    progress("Downloading Laravel…");
    let mut cmd = match &composer {
        ComposerCmd::Launcher(p) => Command::new(p),
        ComposerCmd::Phar(p) => {
            let mut c = Command::new(&php_bin);
            c.arg(p);
            c
        }
    };
    cmd.args([
        "create-project",
        "laravel/laravel",
        &staging.to_string_lossy(),
        "--prefer-dist",
        "--no-interaction",
        "--no-progress",
        // Laravel's own post-create scripts may also run npm; do the PHP side here.
        "--no-scripts",
    ])
    .current_dir(parent)
    .env("COMPOSER_NO_INTERACTION", "1");
    if let Some(p) = &path {
        cmd.env("PATH", p);
    }
    if let Err(e) = run_checked(cmd, "composer create-project") {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    progress("Setting up SQLite…");
    if let Err(e) = finish_setup(&php_bin, &staging, &path) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    if dir.exists() {
        std::fs::remove_dir_all(&dir).context("could not replace the old playground")?;
    }
    std::fs::rename(&staging, &dir).context("could not move the playground into place")?;
    // The SQLite path in config is relative to the app, so the move is safe; clear any
    // cached config that might still name the staging folder.
    let _ = std::fs::remove_file(dir.join("bootstrap").join("cache").join("config.php"));
    Ok(dir)
}

/// Drops every table and runs the migrations again, for a clean database.
pub fn reset_playground_database(php_bin: &str) -> Result<()> {
    let php_bin = if php_bin.trim().is_empty() {
        crate::runner::resolve_php_binary()
    } else {
        php_bin.trim().to_string()
    };
    let dir = playground_dir();
    if !is_playground_ready() {
        bail!("The playground is not installed yet.");
    }
    let path = path_env(&tool_search_dirs(&php_bin));
    run_checked(
        artisan(&php_bin, &dir, &path, &["migrate:fresh", "--force", "--no-interaction"]),
        "php artisan migrate:fresh",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_env_value_replaces_commented_and_plain_keys() {
        let env = "APP_NAME=Laravel\n# DB_CONNECTION=mysql\nDB_HOST=127.0.0.1\n";
        let out = set_env_value(env, "DB_CONNECTION", "sqlite");
        assert!(out.contains("DB_CONNECTION=sqlite"));
        assert!(!out.contains("mysql"));
        let out = set_env_value(&out, "APP_NAME", "Playground");
        assert!(out.starts_with("APP_NAME=Playground\n"));
        let out = set_env_value(&out, "MAIL_MAILER", "log");
        assert!(out.ends_with("MAIL_MAILER=log\n"));
    }

    #[test]
    fn comment_env_value_only_touches_the_key() {
        let env = "DB_HOST=127.0.0.1\nDB_HOSTNAME=x\n";
        let out = comment_env_value(env, "DB_HOST");
        assert_eq!(out, "# DB_HOST=127.0.0.1\nDB_HOSTNAME=x\n");
    }

    #[test]
    fn display_path_uses_tilde_off_windows_only() {
        let home = Path::new("/Users/me");
        let dir = home.join(".laralah").join("playground");
        assert_eq!(display_path_for(&dir, Some(home), false), "~/.laralah/playground");
        assert_eq!(
            display_path_for(&dir, Some(home), true),
            dir.to_string_lossy().to_string()
        );
        assert_eq!(
            display_path_for(Path::new("/elsewhere/playground"), Some(home), false),
            "/elsewhere/playground"
        );
    }

    #[test]
    fn playground_path_is_recognized() {
        let p = playground_dir().to_string_lossy().to_string();
        assert!(is_playground_path(&p));
        assert!(!is_playground_path("/tmp/other"));
    }
}

#[cfg(test)]
mod install_tests {
    /// Real install (network + Composer). Run with a throwaway HOME:
    /// `HOME=/tmp/x cargo test playground_real_install -- --ignored`
    #[test]
    #[ignore]
    fn playground_real_install() {
        let dir = super::install_playground("", |m| eprintln!("{m}")).expect("install");
        assert!(super::is_playground_ready());
        assert!(dir.join("database/database.sqlite").metadata().unwrap().len() > 0);
        super::reset_playground_database("").expect("reset");
    }
}
