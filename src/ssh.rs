use crate::models::ExecutionTarget;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SSH_COLOR_SWATCHES: &[(u32, &str)] = &[
    (0xef4444, "Red"),
    (0xf97316, "Orange"),
    (0xf59e0b, "Amber"),
    (0x10b981, "Emerald"),
    (0x38bdf8, "Sky"),
    (0x2563eb, "Blue"),
    (0x8b5cf6, "Violet"),
];

pub fn default_ssh_color() -> u32 {
    0xef4444
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshAuthMethod {
    #[default]
    PrivateKey,
    SshAgent,
    Password,
}

impl SshAuthMethod {
    pub fn label(&self) -> &'static str {
        match self {
            SshAuthMethod::PrivateKey => "Private Key",
            SshAuthMethod::SshAgent => "SSH Agent",
            SshAuthMethod::Password => "Password",
        }
    }
}

/// How a saved connection reaches PHP: over SSH (optionally into a Docker container on that
/// server), or straight into a Docker container on this machine with `docker exec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionKind {
    #[default]
    Ssh,
    LocalDocker,
}

impl ConnectionKind {
    pub fn label(&self) -> &'static str {
        match self {
            ConnectionKind::Ssh => "SSH",
            ConnectionKind::LocalDocker => "Local Docker",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnectionProfile {
    pub id: String,
    #[serde(default)]
    pub kind: ConnectionKind,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub application_folder: String,
    #[serde(default)]
    pub docker_enabled: bool,
    #[serde(default)]
    pub docker_container: String,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub auth_method: SshAuthMethod,
    #[serde(default = "default_private_key_path")]
    pub private_key: String,
    /// Password or key passphrase. Never written to (or read from) the JSON store on any
    /// platform: it lives only in the macOS Keychain, the Windows Credential Manager or the
    /// Linux Secret Service, or in memory for this session when no OS store is reachable.
    #[serde(skip)]
    pub passphrase: String,
    #[serde(default = "default_ssh_color")]
    pub color_hex: u32,
    #[serde(default)]
    pub group: Option<String>,
}

pub fn default_private_key_path() -> String {
    "~/.ssh/id_rsa".to_string()
}

pub fn default_ssh_application_folder() -> &'static str {
    "/home/forge/"
}

pub fn default_docker_application_folder() -> &'static str {
    "/var/www/html"
}

impl SshConnectionProfile {
    pub fn new_empty() -> Self {
        Self {
            id: generate_ssh_id(),
            kind: ConnectionKind::Ssh,
            label: String::new(),
            host: String::new(),
            port: 22,
            username: "forge".to_string(),
            application_folder: default_ssh_application_folder().to_string(),
            docker_enabled: false,
            docker_container: String::new(),
            local_path: None,
            auth_method: SshAuthMethod::PrivateKey,
            private_key: default_private_key_path(),
            passphrase: String::new(),
            color_hex: default_ssh_color(),
            group: None,
        }
    }

    pub fn is_local_docker(&self) -> bool {
        self.kind == ConnectionKind::LocalDocker
    }

    pub fn active_docker_container(&self) -> Option<&str> {
        if self.docker_enabled || self.is_local_docker() {
            let trimmed = self.docker_container.trim();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
        None
    }

    pub fn is_production_like(&self) -> bool {
        let lower = self.label.to_lowercase();
        let grp = self.group.as_deref().unwrap_or("").to_lowercase();
        self.color_hex == 0xef4444
            || lower.contains("prod")
            || grp.contains("prod")
    }

    pub fn to_execution_target(&self) -> ExecutionTarget {
        let local_docker = self.is_local_docker();
        ExecutionTarget::Ssh {
            id: self.id.clone(),
            local_docker,
            name: if !self.label.trim().is_empty() {
                self.label.trim().to_string()
            } else if local_docker {
                self.docker_container.trim().to_string()
            } else {
                format!("{}@{}", self.username, self.host)
            },
            host: if local_docker { String::new() } else { self.host.trim().to_string() },
            port: self.port,
            user: self.username.trim().to_string(),
            workdir: self.application_folder.trim().to_string(),
            docker_container: self.active_docker_container().map(str::to_string),
            is_production: self.is_production_like(),
            local_path: self.local_path.clone().filter(|s| !s.trim().is_empty()),
            auth_method: self.auth_method,
            private_key: self.private_key.trim().to_string(),
            passphrase: self.passphrase.clone(),
            color_hex: self.color_hex,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshStore {
    pub groups: Vec<String>,
    pub connections: Vec<SshConnectionProfile>,
}

pub fn generate_ssh_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    format!("ssh-{nanos}")
}

const SSH_STORE_FILE_NAME: &str = ".laralah_ssh.json";

/// The SSH store lives in the user's home directory, never in the process working
/// directory, so a `.laralah_ssh.json` dropped into some folder cannot inject profiles.
pub fn ssh_store_file_path() -> PathBuf {
    // `home_dir` handles HOME on Unix and the user profile folder on Windows.
    match crate::paths::home_dir() {
        Some(home) if home.is_dir() => home.join(SSH_STORE_FILE_NAME),
        _ => std::env::temp_dir().join(SSH_STORE_FILE_NAME),
    }
}

/// Pre-1.0 builds kept the store in the working directory.
fn legacy_ssh_store_file_path() -> Option<PathBuf> {
    let legacy = std::env::current_dir().ok()?.join(SSH_STORE_FILE_NAME);
    (legacy != ssh_store_file_path() && legacy.is_file()).then_some(legacy)
}

// OS secret storage. `set` returns Ok(true) when the secret now lives in the OS store,
// Ok(false) when no store is available (the secret is then kept in memory only),
// and Err only when a store exists but writing to it failed.
const SECRET_SERVICE: &str = "app.laralah.ssh";

#[cfg(target_os = "macos")]
mod secrets {
    use super::SECRET_SERVICE;
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    pub fn available() -> bool {
        true
    }

    pub fn get(id: &str) -> Option<String> {
        get_generic_password(SECRET_SERVICE, id)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn set(id: &str, secret: &str) -> Result<bool, String> {
        if secret.is_empty() {
            delete(id);
            return Ok(true);
        }
        set_generic_password(SECRET_SERVICE, id, secret.as_bytes())
            .map(|_| true)
            .map_err(|e| e.to_string())
    }

    pub fn delete(id: &str) {
        let _ = delete_generic_password(SECRET_SERVICE, id);
    }
}

/// Linux: the freedesktop Secret Service (GNOME Keyring, KWallet, KeePassXC...) through
/// `secret-tool` from libsecret. Secrets go over stdin, never on the command line.
#[cfg(target_os = "linux")]
mod secrets {
    use super::SECRET_SERVICE;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;

    fn tool(args: &[&str]) -> Command {
        let mut c = Command::new("secret-tool");
        c.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    /// True when `secret-tool` exists and can reach a Secret Service over D-Bus.
    pub fn available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            // A lookup of a missing item exits 1 with empty stderr; a missing D-Bus session
            // or keyring daemon reports an error on stderr.
            match tool(&["lookup", "service", SECRET_SERVICE, "account", "__laralah_probe__"])
                .output()
            {
                Ok(out) => out.stderr.iter().all(|b| b.is_ascii_whitespace()),
                Err(_) => false,
            }
        })
    }

    pub fn get(id: &str) -> Option<String> {
        if !available() {
            return None;
        }
        let out = tool(&["lookup", "service", SECRET_SERVICE, "account", id])
            .output()
            .ok()?;
        if !out.status.success() || out.stdout.is_empty() {
            return None;
        }
        String::from_utf8(out.stdout).ok()
    }

    pub fn set(id: &str, secret: &str) -> Result<bool, String> {
        if !available() {
            return Ok(false);
        }
        if secret.is_empty() {
            delete(id);
            return Ok(true);
        }
        let label = format!("Laralah SSH ({id})");
        let mut child = tool(&[
            "store", "--label", &label, "service", SECRET_SERVICE, "account", id,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(secret.as_bytes()).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(true)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    pub fn delete(id: &str) {
        if available() {
            let _ = tool(&["clear", "service", SECRET_SERVICE, "account", id]).output();
        }
    }
}

/// Windows: the Credential Manager (generic credentials, per user, DPAPI-encrypted).
#[cfg(target_os = "windows")]
mod secrets {
    use super::SECRET_SERVICE;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    fn target_name(id: &str) -> Vec<u16> {
        format!("{SECRET_SERVICE}/{id}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn available() -> bool {
        true
    }

    pub fn get(id: &str) -> Option<String> {
        let target = target_name(id);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated; on success `cred` points to a buffer that we
        // copy out of and release with `CredFree`.
        unsafe {
            if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
                return None;
            }
            let c = &*cred;
            let bytes = if c.CredentialBlob.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
            };
            CredFree(cred as *const _);
            String::from_utf8(bytes).ok()
        }
    }

    pub fn set(id: &str, secret: &str) -> Result<bool, String> {
        if secret.is_empty() {
            delete(id);
            return Ok(true);
        }
        let mut target = target_name(id);
        let mut user: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        let mut blob = secret.as_bytes().to_vec();
        let blob_len = u32::try_from(blob.len()).map_err(|_| "secret is too long".to_string())?;
        // SAFETY: all pointers reference live, correctly sized buffers for the call's duration.
        let ok = unsafe {
            let mut cred: CREDENTIALW = std::mem::zeroed();
            cred.Type = CRED_TYPE_GENERIC;
            cred.TargetName = target.as_mut_ptr();
            cred.UserName = user.as_mut_ptr();
            cred.CredentialBlobSize = blob_len;
            cred.CredentialBlob = blob.as_mut_ptr();
            cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
            CredWriteW(&cred, 0)
        };
        if ok == 0 {
            // SAFETY: plain FFI call with no arguments.
            let code = unsafe { GetLastError() };
            return Err(format!("Credential Manager write failed (error {code})"));
        }
        Ok(true)
    }

    pub fn delete(id: &str) {
        let target = target_name(id);
        // SAFETY: `target` is NUL-terminated. A missing credential is not an error here.
        unsafe {
            CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod secrets {
    // No OS secret store on this platform; secrets are kept in memory for the session only.
    pub fn available() -> bool {
        false
    }
    pub fn get(_id: &str) -> Option<String> {
        None
    }
    pub fn set(_id: &str, _secret: &str) -> Result<bool, String> {
        Ok(false)
    }
    pub fn delete(_id: &str) {}
}

// Every OS secret-store read can show a Keychain prompt, so secrets are read only when a
// connection is actually used (connect / run / edit), at most once per id per session.
fn secret_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, Option<String>>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The stored password / passphrase for a connection, read lazily from the OS store.
pub fn ssh_secret(id: &str) -> String {
    let mut cache = secret_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = cache.get(id) {
        return hit.clone().unwrap_or_default();
    }
    let value = secrets::get(id);
    cache.insert(id.to_string(), value.clone());
    value.unwrap_or_default()
}

/// Writes a connection's secret to the OS store, skipping the write when it is unchanged.
/// An empty secret removes it. Returns Ok(true) when the secret lives in the OS store.
pub fn store_ssh_secret(id: &str, secret: &str) -> Result<bool, String> {
    let mut cache = secret_cache().lock().unwrap_or_else(|e| e.into_inner());
    let wanted = (!secret.is_empty()).then(|| secret.to_string());
    if cache.get(id) == Some(&wanted) {
        return Ok(secrets::available());
    }
    let in_os_store = secrets::set(id, secret)?;
    // Without an OS store the secret is still usable for this session, never on disk.
    cache.insert(id.to_string(), wanted);
    Ok(in_os_store)
}

/// Removes a connection's stored password / passphrase from the OS keychain.
pub fn delete_ssh_secret(id: &str) {
    secrets::delete(id);
    secret_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), None);
}

/// Rejects hosts and users that `ssh` would parse as options (e.g. `-oProxyCommand=...`)
/// or that contain whitespace / control characters.
pub fn validate_ssh_destination(host: &str, user: &str) -> Result<(), String> {
    let host = host.trim();
    let user = user.trim();
    if host.is_empty() {
        return Err("SSH host is empty.".to_string());
    }
    if host.starts_with('-') {
        return Err("SSH host must not start with '-'.".to_string());
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']' | '%'))
    {
        return Err(format!("SSH host '{host}' contains invalid characters."));
    }
    if user.starts_with('-') {
        return Err("SSH username must not start with '-'.".to_string());
    }
    if user
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '@')
    {
        return Err("SSH username contains invalid characters.".to_string());
    }
    Ok(())
}

#[cfg(test)]
pub fn default_seeded_ssh_store() -> SshStore {
    SshStore {
        groups: vec!["Production".to_string(), "Staging".to_string()],
        connections: vec![
            SshConnectionProfile {
                id: "ssh-seed-prod".to_string(),
                kind: ConnectionKind::Ssh,
                label: "Laralah Production".to_string(),
                host: "207.154.198.53".to_string(),
                port: 22,
                username: "forge".to_string(),
                application_folder: "/home/forge/laralah.app".to_string(),
                docker_enabled: false,
                docker_container: String::new(),
                local_path: None,
                auth_method: SshAuthMethod::PrivateKey,
                private_key: "~/.ssh/id_rsa".to_string(),
                passphrase: String::new(),
                color_hex: 0xef4444,
                group: Some("Production".to_string()),
            },
            SshConnectionProfile {
                id: "ssh-seed-staging".to_string(),
                kind: ConnectionKind::Ssh,
                label: "Staging Forge".to_string(),
                host: "staging.laralah.internal".to_string(),
                port: 22,
                username: "forge".to_string(),
                application_folder: "/home/forge/staging.laralah.app".to_string(),
                docker_enabled: false,
                docker_container: String::new(),
                local_path: None,
                auth_method: SshAuthMethod::SshAgent,
                private_key: "~/.ssh/id_ed25519".to_string(),
                passphrase: String::new(),
                color_hex: 0xf59e0b,
                group: Some("Staging".to_string()),
            },
        ],
    }
}

fn read_store_file(path: &Path) -> Option<SshStore> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<SshStore>(&raw) {
        Ok(store) => Some(store),
        Err(err) => {
            // Keep the unreadable file instead of letting the next save overwrite it.
            let backup = path.with_extension("json.corrupt");
            let _ = std::fs::rename(path, &backup);
            eprintln!(
                "Laralah: could not parse {} ({err}); moved it to {}",
                path.display(),
                backup.display()
            );
            None
        }
    }
}

pub fn load_ssh_store() -> SshStore {
    let path = ssh_store_file_path();
    let mut needs_resave = false;

    let store = if path.is_file() {
        read_store_file(&path).unwrap_or_default()
    } else if let Some(legacy) = legacy_ssh_store_file_path() {
        match read_store_file(&legacy) {
            Some(store) => {
                needs_resave = true;
                store
            }
            None => SshStore::default(),
        }
    } else {
        SshStore::default()
    };

    // Secrets are not read here (that would show one Keychain prompt per connection at
    // launch); `ssh_secret` loads them when a connection is used. Any `passphrase` left in
    // an old JSON file is ignored, and the next save rewrites the file without it.
    if std::fs::read_to_string(&path).is_ok_and(|raw| raw.contains("\"passphrase\"")) {
        needs_resave = true;
    }

    if needs_resave && save_ssh_store_checked(&store).is_ok() {
        if let Some(legacy) = legacy_ssh_store_file_path()
            && read_store_file(&path).is_some()
        {
            // The legacy file could hold plaintext credentials; its profiles now live in
            // the home-dir store.
            let _ = std::fs::remove_file(legacy);
        }
    }
    store
}

pub fn save_ssh_store(store: &SshStore) {
    if let Err(err) = save_ssh_store_checked(store) {
        eprintln!("Laralah: failed to save SSH connections: {err}");
    }
}

fn save_ssh_store_checked(store: &SshStore) -> Result<(), String> {
    for conn in &store.connections {
        // An empty in-memory passphrase means "not loaded" (or none): leave the OS store alone.
        // Clearing a saved secret goes through `store_ssh_secret` / `delete_ssh_secret`.
        if !conn.passphrase.is_empty() {
            store_ssh_secret(&conn.id, &conn.passphrase)
                .map_err(|e| format!("Keychain error for '{}': {e}", conn.label))?;
        }
    }
    // `passphrase` is `#[serde(skip)]`, so the JSON never contains a secret.
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    write_private_file_atomic(&ssh_store_file_path(), json.as_bytes())
}

/// Writes `bytes` to `path` via a 0600 temp file in the same directory + rename.
fn write_private_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    tmp.write_all(bytes).map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfigHost {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
}

pub fn expand_tilde_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("~/")
        && let Some(home) = crate::paths::home_dir()
    {
        return Path::new(&home).join(rest).to_string_lossy().to_string();
    }
    trimmed.to_string()
}

pub fn load_system_ssh_config() -> Vec<SshConfigHost> {
    let Some(home) = crate::paths::home_dir() else {
        return Vec::new();
    };
    let config_path = Path::new(&home).join(".ssh").join("config");
    let Ok(raw) = std::fs::read_to_string(config_path) else {
        return Vec::new();
    };
    parse_ssh_config_str(&raw)
}

pub fn parse_ssh_config_str(raw: &str) -> Vec<SshConfigHost> {
    let mut results: Vec<SshConfigHost> = Vec::new();
    let mut active_aliases: Vec<String> = Vec::new();
    let mut hostname: Option<String> = None;
    let mut user: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut identity_file: Option<String> = None;

    let flush = |aliases: &mut Vec<String>,
                 hostname: &mut Option<String>,
                 user: &mut Option<String>,
                 port: &mut Option<u16>,
                 identity_file: &mut Option<String>,
                 out: &mut Vec<SshConfigHost>| {
        for alias in aliases.drain(..) {
            if alias.contains('*') || alias.contains('?') || alias.starts_with('!') {
                continue;
            }
            out.push(SshConfigHost {
                hostname: hostname.clone().unwrap_or_else(|| alias.clone()),
                alias,
                user: user.clone(),
                port: *port,
                identity_file: identity_file.clone(),
            });
        }
        *hostname = None;
        *user = None;
        *port = None;
        *identity_file = None;
    };

    for line in raw.lines() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, |c: char| c.is_whitespace() || c == '=');
        let Some(key) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(val) = parts
            .next()
            .map(|s| s.trim().trim_start_matches('=').trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
        else {
            continue;
        };

        match key.to_ascii_lowercase().as_str() {
            "host" => {
                flush(
                    &mut active_aliases,
                    &mut hostname,
                    &mut user,
                    &mut port,
                    &mut identity_file,
                    &mut results,
                );
                active_aliases = val
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
            }
            "match" => {
                flush(
                    &mut active_aliases,
                    &mut hostname,
                    &mut user,
                    &mut port,
                    &mut identity_file,
                    &mut results,
                );
            }
            "hostname" => hostname = Some(val.to_string()),
            "user" => user = Some(val.to_string()),
            "port" => port = val.parse::<u16>().ok(),
            "identityfile" => {
                if identity_file.is_none() {
                    identity_file = Some(val.to_string());
                }
            }
            _ => {}
        }
    }

    flush(
        &mut active_aliases,
        &mut hostname,
        &mut user,
        &mut port,
        &mut identity_file,
        &mut results,
    );

    results
}

pub fn shell_escape_single_quotes(input: &str) -> String {
    let escaped = input.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Builds the remote command executed on the SSH host to run a piped PHP script from stdin.
/// - Plain SSH: `cd '<workdir>' && php`
/// - SSH + Docker: `PATH="$PATH:/usr/local/bin:/snap/bin" docker exec -i -w '<workdir>' '<container>' php`
///   where `workdir` is the path inside the Docker container.
pub fn build_ssh_remote_php_command(workdir: &str, docker_container: Option<&str>) -> String {
    let clean_workdir = workdir.trim();
    if let Some(container) = docker_container.map(str::trim).filter(|c| !c.is_empty()) {
        let container_workdir = if clean_workdir.is_empty() {
            default_docker_application_folder()
        } else {
            clean_workdir
        };
        format!(
            "PATH=\"$PATH:/usr/local/bin:/snap/bin\" docker exec -i -w {} {} php",
            shell_escape_single_quotes(container_workdir),
            shell_escape_single_quotes(container),
        )
    } else {
        let host_workdir = if clean_workdir.is_empty() {
            default_ssh_application_folder()
        } else {
            clean_workdir
        };
        format!("cd {} && php", shell_escape_single_quotes(host_workdir))
    }
}

pub fn build_ssh_cli_args(
    host: &str,
    port: u16,
    user: &str,
    remote_cmd: &str,
    auth_method: SshAuthMethod,
    private_key: &str,
    has_passphrase_or_password: bool,
) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "ConnectTimeout=6".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
    ];

    if !has_passphrase_or_password {
        args.push("-o".to_string());
        args.push("BatchMode=yes".to_string());
    }

    match auth_method {
        SshAuthMethod::PrivateKey => {
            let expanded = expand_tilde_path(private_key);
            if !expanded.is_empty() {
                args.push("-i".to_string());
                args.push(expanded);
            }
        }
        SshAuthMethod::SshAgent => {}
        SshAuthMethod::Password => {
            args.push("-o".to_string());
            args.push("PreferredAuthentications=password,keyboard-interactive".to_string());
        }
    }

    args.push("-p".to_string());
    args.push(port.to_string());
    // End of options: nothing after this can be parsed as an ssh flag.
    args.push("--".to_string());
    let destination = if user.trim().is_empty() {
        host.trim().to_string()
    } else {
        format!("{}@{}", user.trim(), host.trim())
    };
    args.push(destination);
    args.push(remote_cmd.to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openssh_config_hosts_and_skips_wildcards() {
        let sample = r#"
# Global defaults
Host *
    AddKeysToAgent yes
    UseKeychain yes

Host prod-forge
    HostName 207.154.198.53
    User forge
    Port 2222
    IdentityFile ~/.ssh/id_ed25519

Host staging-api
    HostName 10.0.4.19
    User ubuntu
"#;
        let parsed = parse_ssh_config_str(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].alias, "prod-forge");
        assert_eq!(parsed[0].hostname, "207.154.198.53");
        assert_eq!(parsed[0].user.as_deref(), Some("forge"));
        assert_eq!(parsed[0].port, Some(2222));
        assert_eq!(parsed[0].identity_file.as_deref(), Some("~/.ssh/id_ed25519"));

        assert_eq!(parsed[1].alias, "staging-api");
        assert_eq!(parsed[1].hostname, "10.0.4.19");
        assert_eq!(parsed[1].user.as_deref(), Some("ubuntu"));
        assert_eq!(parsed[1].port, None);
    }

    #[test]
    fn builds_ssh_args_with_custom_port_and_identity_file() {
        let args = build_ssh_cli_args(
            "207.154.198.53",
            2222,
            "forge",
            "cd '/home/forge/app' && php",
            SshAuthMethod::PrivateKey,
            "/tmp/custom_id_rsa",
            false,
        );
        assert!(args.windows(2).any(|w| w == ["-i", "/tmp/custom_id_rsa"]));
        assert!(args.windows(2).any(|w| w == ["-p", "2222"]));
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(args.contains(&"forge@207.154.198.53".to_string()));
        let sep = args.iter().position(|a| a == "--").expect("-- separator");
        assert_eq!(args[sep + 1], "forge@207.154.198.53");
    }

    #[test]
    fn rejects_option_injection_in_ssh_destination() {
        assert!(validate_ssh_destination("207.154.198.53", "forge").is_ok());
        assert!(validate_ssh_destination("prod-forge.example.com", "").is_ok());
        assert!(validate_ssh_destination("[fe80::1%en0]", "root").is_ok());
        assert!(validate_ssh_destination("-oProxyCommand=touch /tmp/x", "forge").is_err());
        assert!(validate_ssh_destination("host evil", "forge").is_err());
        assert!(validate_ssh_destination("host\n", "forge").is_ok()); // trimmed
        assert!(validate_ssh_destination("", "forge").is_err());
        assert!(validate_ssh_destination("example.com", "-oProxyCommand=x").is_err());
        assert!(validate_ssh_destination("example.com", "a b").is_err());
        assert!(validate_ssh_destination("example.com", "a@b").is_err());
    }

    /// Needs a running Secret Service (see scripts/test-linux-docker.sh).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn secret_service_roundtrip() {
        assert!(secrets::available(), "no Secret Service reachable");
        let id = format!("ssh-test-{}", std::process::id());
        assert_eq!(secrets::set(&id, "p'w $x"), Ok(true));
        assert_eq!(secrets::get(&id).as_deref(), Some("p'w $x"));
        secrets::delete(&id);
        assert_eq!(secrets::get(&id), None);
    }

    #[test]
    fn passphrase_is_never_serialized() {
        let mut p = SshConnectionProfile::new_empty();
        p.passphrase = "hunter2".to_string();
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("hunter2"));
        assert!(!json.contains("passphrase"));
    }

    #[test]
    fn roundtrips_ssh_store_json() {
        let store = default_seeded_ssh_store();
        let json = serde_json::to_string(&store).unwrap();
        let decoded: SshStore = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, store);
        let target = decoded.connections[0].to_execution_target();
        assert!(target.is_ssh());
    }

    #[test]
    fn deserializes_legacy_ssh_profile_without_docker_fields_and_roundtrips_docker_profile() {
        let legacy_json = r#"{
            "id": "ssh-legacy-1",
            "label": "Legacy Server",
            "host": "10.0.0.12",
            "port": 22,
            "username": "forge",
            "application_folder": "/home/forge/app"
        }"#;
        let legacy: SshConnectionProfile = serde_json::from_str(legacy_json).unwrap();
        assert!(!legacy.docker_enabled);
        assert_eq!(legacy.docker_container, "");
        assert_eq!(legacy.active_docker_container(), None);
        assert_eq!(
            legacy.to_execution_target().local_or_workdir(),
            "/home/forge/app"
        );

        let mut docker_prof = legacy.clone();
        docker_prof.docker_enabled = true;
        docker_prof.docker_container = "laravel-app-1".to_string();
        docker_prof.application_folder = "/var/www/html".to_string();

        let encoded = serde_json::to_string(&docker_prof).unwrap();
        let decoded: SshConnectionProfile = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.docker_enabled);
        assert_eq!(decoded.active_docker_container(), Some("laravel-app-1"));

        let target = decoded.to_execution_target();
        assert_eq!(target.local_or_workdir(), "/var/www/html");
        assert_eq!(target.kind_badge(), "SSH DOCKER PROD");
        assert!(target.label().contains("laravel-app-1"));

        // Toggling docker_enabled off suppresses docker_container in ExecutionTarget
        let mut disabled = decoded;
        disabled.docker_enabled = false;
        assert_eq!(disabled.active_docker_container(), None);
        assert_eq!(disabled.to_execution_target().kind_badge(), "SSH PROD");
    }

    #[test]
    fn builds_ssh_remote_php_command_for_host_and_docker_container() {
        assert_eq!(
            build_ssh_remote_php_command("/home/forge/my-app", None),
            "cd '/home/forge/my-app' && php"
        );
        assert_eq!(
            build_ssh_remote_php_command("/var/www/html", Some("laravel.test")),
            "PATH=\"$PATH:/usr/local/bin:/snap/bin\" docker exec -i -w '/var/www/html' 'laravel.test' php"
        );
        assert_eq!(
            build_ssh_remote_php_command("/srv/o'reilly", Some("a1b2c3d4e5f6")),
            "PATH=\"$PATH:/usr/local/bin:/snap/bin\" docker exec -i -w '/srv/o'\\''reilly' 'a1b2c3d4e5f6' php"
        );
    }

    #[test]
    fn local_docker_profile_becomes_local_docker_target() {
        let mut p = SshConnectionProfile::new_empty();
        p.kind = ConnectionKind::LocalDocker;
        p.label = String::new();
        p.host = "ignored.example.com".to_string();
        p.docker_enabled = false;
        p.docker_container = "laravel.test".to_string();
        p.application_folder = "/var/www/html".to_string();
        p.color_hex = 0x38bdf8;
        let ExecutionTarget::Ssh {
            local_docker,
            name,
            host,
            docker_container,
            workdir,
            ..
        } = p.to_execution_target()
        else {
            panic!("expected a saved-connection target");
        };
        assert!(local_docker);
        assert_eq!(name, "laravel.test");
        assert!(host.is_empty());
        assert_eq!(docker_container.as_deref(), Some("laravel.test"));
        assert_eq!(workdir, "/var/www/html");
        assert_eq!(p.to_execution_target().kind_badge(), "DOCKER");
        assert_eq!(p.to_execution_target().label(), "Docker: laravel.test");
    }

    #[test]
    fn profiles_saved_before_connection_kinds_load_as_ssh() {
        let json = r#"{"id":"ssh-1","label":"old","host":"h","port":22,"username":"forge","application_folder":"/home/forge/app"}"#;
        let p: SshConnectionProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.kind, ConnectionKind::Ssh);
        assert!(!p.is_local_docker());
    }
}
