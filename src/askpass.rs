use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context, Result};
use log::{debug, error, warn};

use crate::ssh_config::model::SshConfigFile;

/// A password source option for the picker overlay.
pub struct PasswordSourceOption {
    pub label: &'static str,
    pub value: &'static str,
    pub hint: &'static str,
}

pub const PASSWORD_SOURCES: &[PasswordSourceOption] = &[
    PasswordSourceOption {
        label: "OS Keychain",
        value: "keychain",
        hint: "keychain",
    },
    PasswordSourceOption {
        label: "1Password",
        value: "op://",
        hint: "op://Vault/Item/field",
    },
    PasswordSourceOption {
        label: "Bitwarden",
        value: "bw:",
        hint: "bw:item-name",
    },
    PasswordSourceOption {
        label: "pass",
        value: "pass:",
        hint: "pass:path/to/entry",
    },
    // Vault KV secrets engine (key/value store). Distinct from the Vault SSH
    // secrets engine used for signed SSH certificates, which has its own
    // "Vault SSH role" field on the host form.
    PasswordSourceOption {
        label: "HashiCorp Vault KV",
        value: "vault:",
        hint: "vault:secret/path#field",
    },
    PasswordSourceOption {
        label: "Proton Pass",
        value: "proton:",
        hint: "proton:Vault/Item/field",
    },
    PasswordSourceOption {
        label: "Custom command",
        value: "cmd:",
        hint: "cmd %a %h",
    },
    PasswordSourceOption {
        label: "None",
        value: "",
        hint: "(remove)",
    },
];

/// Handle an SSH_ASKPASS invocation. Called when purple is invoked as an askpass program.
/// Reads the password source from the host's `# purple:askpass` comment and retrieves it.
pub fn handle(env: &crate::runtime::env::Env) -> Result<()> {
    // Initialize file-only logging for askpass subprocess
    // verbose is determined by PURPLE_LOG env var only (no CLI flag in subprocess)
    crate::logging::init(false, false, env);

    let alias = env.var("PURPLE_HOST_ALIAS").unwrap_or_default().to_string();
    let config_path = env
        .var("PURPLE_CONFIG_PATH")
        .unwrap_or_default()
        .to_string();

    // Check the prompt (argv[1]) to skip passphrase and host key verification prompts
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let prompt_lower = prompt.to_ascii_lowercase();
    if prompt_lower.contains("passphrase")
        || prompt_lower.contains("yes/no")
        || prompt_lower.contains("(yes/no/")
    {
        // Not a password prompt. Exit with error so SSH falls back to interactive.
        std::process::exit(1);
    }

    if alias.is_empty() || config_path.is_empty() {
        std::process::exit(1);
    }

    // Parse config first so we can resolve the prompt's host to the right entry.
    // With ProxyJump, ssh fires askpass for each hop. The prompt argv[1] tells us
    // which hop is being authenticated; PURPLE_HOST_ALIAS only knows the final
    // target. Resolving the prompt host to its own alias scopes the keychain
    // lookup to the correct entry per hop.
    let config = SshConfigFile::parse_with_env(&PathBuf::from(&config_path), env)
        .context("Failed to parse SSH config")?;

    // Restrict prompt-based resolution to PURPLE_HOST_ALIAS and the hosts
    // reachable via its ProxyJump chain. Without this scope, a malicious
    // server could send a keyboard-interactive prompt formatted like a
    // password prompt for an unrelated host (`attacker@victim's password:`)
    // and exfiltrate that host's credential. Chain membership ensures we
    // only ever supply credentials for hosts the user has wired into this
    // connection.
    let chain = build_proxy_chain(&config, &alias);
    let resolved_alias = parse_password_prompt_host(&prompt)
        .and_then(|h| find_alias_for_host(&config, h, &chain))
        .unwrap_or_else(|| alias.clone());

    // Retry detection: if we've been called recently for this resolved alias,
    // the password was wrong. Exit with error so SSH falls back to interactive.
    // The marker is keyed on the resolved alias so retries on one ProxyJump hop
    // do not block askpass on the next hop.
    let marker = marker_path(env.paths(), &resolved_alias);
    if let Some(marker_path) = &marker {
        if is_recent_marker(marker_path) {
            debug!("[purple] Askpass retry detected for {resolved_alias}");
            let _ = std::fs::remove_file(marker_path);
            std::process::exit(1);
        }
        if let Err(e) = std::fs::create_dir_all(marker_path.parent().unwrap()) {
            debug!("[purple] Failed to create askpass marker directory: {e}");
        }
        if let Err(e) = crate::fs_util::atomic_write(marker_path, b"") {
            debug!("[purple] Failed to write askpass marker: {e}");
        }
    }

    let source = find_askpass_source(&config, env.paths(), &resolved_alias);

    let source = match source {
        Some(s) => s,
        None => std::process::exit(1),
    };

    debug!("[purple] Askpass invoked for alias={resolved_alias} source={source}");

    let hostname = find_hostname(&config, &resolved_alias);
    match retrieve_password(env, &source, &resolved_alias, &hostname) {
        Ok(password) => {
            debug!("[purple] Askpass retrieved password for {resolved_alias} via {source}");
            print!("{}", password);
            Ok(())
        }
        Err(err) => {
            warn!("[external] Password retrieval failed via {source}");
            debug!("[external] Password retrieval detail: {err}");
            if let Some(m) = &marker {
                let _ = std::fs::remove_file(m);
            }
            std::process::exit(1);
        }
    }
}

/// Extract the host being authenticated from an OpenSSH password prompt.
/// OpenSSH builds prompts as `<user>@<host>'s password:` (see `userauth_passwd`
/// in openssh-portable). IPv6 hosts are rendered with brackets (`user@[::1]`),
/// which we strip so the result matches a plain `HostName` entry. Returns
/// `None` for keyboard-interactive prompts or any other format we cannot parse,
/// so the caller falls back to PURPLE_HOST_ALIAS.
fn parse_password_prompt_host(prompt: &str) -> Option<&str> {
    let idx = prompt.find("'s password")?;
    let head = &prompt[..idx];
    let (_, host) = head.rsplit_once('@')?;
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if host.is_empty() { None } else { Some(host) }
}

/// Find the alias whose entry matches `host` by alias or hostname, restricted
/// to entries in `permitted`. Alias match takes priority over hostname match
/// in a single pass. Used to map the SSH prompt's host (which may be a bastion
/// in a ProxyJump chain) back to the entry that owns its `# purple:askpass`
/// config. The `permitted` scope blocks malicious-server attempts to coax a
/// credential lookup for an unrelated host in `~/.ssh/config`.
fn find_alias_for_host(
    config: &SshConfigFile,
    host: &str,
    permitted: &HashSet<String>,
) -> Option<String> {
    let mut by_hostname: Option<String> = None;
    for entry in config.host_entries() {
        if !permitted.contains(&entry.alias) {
            continue;
        }
        if entry.alias.eq_ignore_ascii_case(host) {
            return Some(entry.alias.clone());
        }
        if by_hostname.is_none() && entry.hostname.eq_ignore_ascii_case(host) {
            by_hostname = Some(entry.alias.clone());
        }
    }
    by_hostname
}

/// Build the set of aliases reachable from `target` via its ProxyJump chain,
/// including `target` itself. ProxyJump values can be comma-separated and
/// formatted `[user@]host[:port]`, including bracketed IPv6 hosts. Cycles are
/// broken by the visited-set; entries that reference unknown hosts contribute
/// nothing to the chain.
fn build_proxy_chain(config: &SshConfigFile, target: &str) -> HashSet<String> {
    let entries = config.host_entries();
    let mut chain: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = vec![target.to_string()];
    while let Some(current) = queue.pop() {
        if !chain.insert(current.clone()) {
            continue;
        }
        let Some(entry) = entries.iter().find(|e| e.alias == current) else {
            continue;
        };
        if entry.proxy_jump.is_empty() {
            continue;
        }
        for jump in entry.proxy_jump.split(',') {
            let host = parse_proxy_jump_host(jump);
            if host.is_empty() {
                continue;
            }
            for e in &entries {
                if e.alias.eq_ignore_ascii_case(host) || e.hostname.eq_ignore_ascii_case(host) {
                    queue.push(e.alias.clone());
                }
            }
        }
    }
    chain
}

/// Extract the host portion from a single ProxyJump entry of the form
/// `[user@]host[:port]`. Handles bracketed IPv6 hosts (`[::1]:22`).
fn parse_proxy_jump_host(jump: &str) -> &str {
    let trimmed = jump.trim();
    let after_user = trimmed.rsplit_once('@').map(|(_, h)| h).unwrap_or(trimmed);
    if let Some(rest) = after_user.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        return &rest[..end];
    }
    after_user.split(':').next().unwrap_or(after_user)
}

/// Find the askpass source for a host. Checks per-host config, then global default.
fn find_askpass_source(
    config: &SshConfigFile,
    paths: Option<&crate::runtime::env::Paths>,
    alias: &str,
) -> Option<String> {
    // Per-host source
    for entry in config.host_entries() {
        if entry.alias == alias
            && let Some(ref source) = entry.askpass
        {
            return Some(source.clone());
        }
    }
    // Global default from preferences file
    load_askpass_default_direct(paths)
}

/// Read askpass default directly from ~/.purple/preferences without depending on the
/// preferences module (which requires crate::app and isn't available in askpass subprocess).
fn load_askpass_default_direct(paths: Option<&crate::runtime::env::Paths>) -> Option<String> {
    let path = paths?.preferences();
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == "askpass"
        {
            let val = v.trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Find the hostname for an alias (for %h substitution).
fn find_hostname(config: &SshConfigFile, alias: &str) -> String {
    for entry in config.host_entries() {
        if entry.alias == alias {
            return entry.hostname.clone();
        }
    }
    alias.to_string()
}

/// Retrieve a password from the given source.
fn retrieve_password(
    env: &crate::runtime::env::Env,
    source: &str,
    alias: &str,
    hostname: &str,
) -> Result<String> {
    if source == "keychain" {
        return retrieve_from_keychain(env, alias);
    }
    if let Some(uri) = source.strip_prefix("op://") {
        return retrieve_from_1password(env, &format!("op://{}", uri));
    }
    if let Some(entry) = source.strip_prefix("pass:") {
        return retrieve_from_pass(env, entry);
    }
    if let Some(item_id) = source.strip_prefix("bw:") {
        return retrieve_from_bitwarden(env, item_id);
    }
    if let Some(rest) = source.strip_prefix("vault:") {
        return retrieve_from_vault(env, rest);
    }
    if let Some(spec) = source.strip_prefix("proton:") {
        return retrieve_from_proton_pass(env, spec);
    }
    // Custom command (with or without cmd: prefix)
    let cmd = source.strip_prefix("cmd:").unwrap_or(source);
    retrieve_from_command(env, cmd, alias, hostname)
}

/// Retrieve from OS keychain (macOS: Keychain, Linux: secret-tool).
fn retrieve_from_keychain(env: &crate::runtime::env::Env, alias: &str) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let output = env
            .command("security")
            .args([
                "find-generic-password",
                "-a",
                alias,
                "-s",
                "purple-ssh",
                "-w",
            ])
            .output()
            .context("Failed to run security command")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!(
                "[external] askpass keychain lookup failed: alias={} exit={} stderr={}",
                alias,
                output.status.code().unwrap_or(-1),
                stderr.trim().lines().next().unwrap_or("<empty>"),
            );
            anyhow::bail!("Keychain lookup failed");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let output = env
            .command("secret-tool")
            .args(["lookup", "application", "purple-ssh", "host", alias])
            .output()
            .context("Failed to run secret-tool")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!(
                "[external] askpass secret-tool lookup failed: alias={} exit={} stderr={}",
                alias,
                output.status.code().unwrap_or(-1),
                stderr.trim().lines().next().unwrap_or("<empty>"),
            );
            anyhow::bail!("Secret-tool lookup failed");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Check if a password exists in the OS keychain for this alias.
pub fn keychain_has_password(env: &crate::runtime::env::Env, alias: &str) -> bool {
    retrieve_from_keychain(env, alias).is_ok()
}

/// Retrieve a password from the OS keychain. Public for keychain migration on alias rename.
pub fn retrieve_keychain_password(env: &crate::runtime::env::Env, alias: &str) -> Result<String> {
    retrieve_from_keychain(env, alias)
}

/// Store a password in the OS keychain.
pub fn store_in_keychain(
    env: &crate::runtime::env::Env,
    alias: &str,
    password: &str,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = env
            .command("security")
            .args([
                "add-generic-password",
                "-U",
                "-a",
                alias,
                "-s",
                "purple-ssh",
                "-w",
                password,
            ])
            .status()
            .context("Failed to run security command")?;
        if !status.success() {
            anyhow::bail!("Failed to store password in Keychain");
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut child = env
            .command("secret-tool")
            .args([
                "store",
                "--label",
                &format!("purple-ssh: {}", alias),
                "application",
                "purple-ssh",
                "host",
                alias,
            ])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Failed to run secret-tool")?;
        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            stdin.write_all(password.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("Failed to store password with secret-tool");
        }
        Ok(())
    }
}

/// Remove a password from the OS keychain.
pub fn remove_from_keychain(env: &crate::runtime::env::Env, alias: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = env
            .command("security")
            .args(["delete-generic-password", "-a", alias, "-s", "purple-ssh"])
            .status()
            .context("Failed to run security command")?;
        if !status.success() {
            anyhow::bail!("No password found for '{}' in Keychain", alias);
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let status = env
            .command("secret-tool")
            .args(["clear", "application", "purple-ssh", "host", alias])
            .status()
            .context("Failed to run secret-tool")?;
        if !status.success() {
            anyhow::bail!("Failed to remove password with secret-tool");
        }
        Ok(())
    }
}

/// Retrieve from 1Password CLI.
fn retrieve_from_1password(env: &crate::runtime::env::Env, uri: &str) -> Result<String> {
    let result = env
        .command("op")
        .args(["read", uri, "--no-newline"])
        .output();
    let output = match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            error!("[config] Password manager binary not found: op");
            return Err(e).context("Failed to run 1Password CLI (op)");
        }
        other => other.context("Failed to run 1Password CLI (op)")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "[external] askpass 1Password lookup failed: uri={} exit={} stderr={}",
            uri,
            output.status.code().unwrap_or(-1),
            stderr.trim().lines().next().unwrap_or("<empty>"),
        );
        anyhow::bail!("1Password lookup failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Retrieve from pass (password-store). Returns the first line.
fn retrieve_from_pass(env: &crate::runtime::env::Env, entry: &str) -> Result<String> {
    let result = env.command("pass").args(["show", entry]).output();
    let output = match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            error!("[config] Password manager binary not found: pass");
            return Err(e).context("Failed to run pass");
        }
        other => other.context("Failed to run pass")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "[external] askpass pass lookup failed: entry={} exit={} stderr={}",
            entry,
            output.status.code().unwrap_or(-1),
            stderr.trim().lines().next().unwrap_or("<empty>"),
        );
        anyhow::bail!("pass lookup failed");
    }
    let full = String::from_utf8_lossy(&output.stdout);
    Ok(full.lines().next().unwrap_or("").to_string())
}

/// Retrieve from Bitwarden CLI. The item_id can be an item ID or search term.
/// Uses `bw get password <item_id>` which requires an unlocked vault (BW_SESSION).
fn retrieve_from_bitwarden(env: &crate::runtime::env::Env, item_id: &str) -> Result<String> {
    let result = env
        .command("bw")
        .args(["get", "password", item_id])
        .output();
    let output = match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            error!("[config] Password manager binary not found: bw");
            return Err(e).context("Failed to run Bitwarden CLI (bw)");
        }
        other => other.context("Failed to run Bitwarden CLI (bw)")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "[external] askpass Bitwarden lookup failed: item={} exit={} stderr={}",
            item_id,
            output.status.code().unwrap_or(-1),
            stderr.trim().lines().next().unwrap_or("<empty>"),
        );
        anyhow::bail!("Bitwarden lookup failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Retrieve from the HashiCorp Vault KV secrets engine via the `vault` CLI.
/// Spec format: `path#field` or just `path` (defaults to `password`).
/// Distinct from the Vault SSH secrets engine (see src/vault_ssh.rs), which
/// signs SSH certificates rather than storing passwords.
fn retrieve_from_vault(env: &crate::runtime::env::Env, spec: &str) -> Result<String> {
    let (path, field) = match spec.rsplit_once('#') {
        Some((p, f)) => (p, f),
        None => (spec, "password"),
    };
    let result = env
        .command("vault")
        .args(["kv", "get", &format!("-field={}", field), path])
        .output();
    let output = match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            error!("[config] Password manager binary not found: vault");
            return Err(e).context("Failed to run vault CLI");
        }
        other => other.context("Failed to run vault CLI")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "[external] askpass Vault KV lookup failed: path={} field={} exit={} stderr={}",
            path,
            field,
            output.status.code().unwrap_or(-1),
            stderr.trim().lines().next().unwrap_or("<empty>"),
        );
        anyhow::bail!("Vault lookup failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Retrieve via custom command. Supports %h (hostname) and %a (alias) substitution.
/// Values are shell-escaped to prevent metacharacter injection.
fn retrieve_from_command(
    env: &crate::runtime::env::Env,
    cmd: &str,
    alias: &str,
    hostname: &str,
) -> Result<String> {
    let safe_alias = crate::snippet::shell_escape(alias);
    let safe_hostname = crate::snippet::shell_escape(hostname);
    let expanded = cmd.replace("%a", &safe_alias).replace("%h", &safe_hostname);
    let output = env
        .command("sh")
        .args(["-c", &expanded])
        .output()
        .context("Failed to run custom askpass command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "[external] askpass custom command failed: alias={} exit={} stderr={}",
            alias,
            output.status.code().unwrap_or(-1),
            stderr.trim().lines().next().unwrap_or("<empty>"),
        );
        anyhow::bail!("Custom askpass command failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get the path for the retry marker file.
/// Sanitizes the alias to prevent path traversal (replaces `/` and `\` with `_`).
fn marker_path(paths: Option<&crate::runtime::env::Paths>, alias: &str) -> Option<PathBuf> {
    paths.map(|p| p.askpass_marker(alias))
}

/// Check if a marker file exists and is recent (< 60 seconds old).
fn is_recent_marker(path: &PathBuf) -> bool {
    if let Ok(meta) = std::fs::metadata(path)
        && let Ok(modified) = meta.modified()
        && let Ok(elapsed) = SystemTime::now().duration_since(modified)
    {
        return elapsed.as_secs() < 60;
    }
    false
}

/// Clean up retry markers after a successful connection. ProxyJump connections
/// create one marker per hop and the parent process only knows the final
/// target alias, so we clear every `.askpass_*` marker in the state directory
/// on success.
/// Each marker has a 60s expiry; this just keeps rapid reconnects snappy and
/// prevents a stranded bastion marker from blocking the next attempt.
pub fn cleanup_marker(paths: Option<&crate::runtime::env::Paths>, _alias: &str) {
    let Some(paths) = paths else {
        return;
    };
    let Ok(read) = std::fs::read_dir(paths.state_dir()) else {
        return;
    };
    for entry in read.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|s| s.starts_with(".askpass_"))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Parse an askpass source string and return a description for display.
#[allow(dead_code)]
pub fn describe_source(source: &str) -> &str {
    if source == "keychain" {
        "OS Keychain"
    } else if source.starts_with("op://") {
        "1Password"
    } else if source.starts_with("proton:") {
        "Proton Pass"
    } else if source.starts_with("pass:") {
        "pass"
    } else if source.starts_with("bw:") {
        "Bitwarden"
    } else if source.starts_with("vault:") {
        "HashiCorp Vault KV"
    } else {
        "Custom command"
    }
}

/// Bitwarden vault status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BwStatus {
    Unlocked,
    Locked,
    NotAuthenticated,
    NotInstalled,
}

/// Parse the Bitwarden vault status from `bw status` JSON output.
fn parse_bw_status(stdout: &str) -> BwStatus {
    if let Some(status) = stdout
        .split("\"status\":")
        .nth(1)
        .and_then(|s| s.split('"').nth(1))
    {
        match status {
            "unlocked" => BwStatus::Unlocked,
            "locked" => BwStatus::Locked,
            "unauthenticated" => BwStatus::NotAuthenticated,
            _ => BwStatus::Locked,
        }
    } else {
        BwStatus::NotInstalled
    }
}

/// Check the Bitwarden vault status by running `bw status`.
pub fn bw_vault_status(env: &crate::runtime::env::Env) -> BwStatus {
    let output = match env.command("bw").arg("status").output() {
        Ok(o) => o,
        Err(_) => return BwStatus::NotInstalled,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_bw_status(&stdout)
}

/// Unlock the Bitwarden vault with the given master password.
/// Passes the password via env var to avoid exposure in `ps` output.
/// Returns the session token on success.
pub fn bw_unlock(env: &crate::runtime::env::Env, password: &str) -> Result<String> {
    let output = env
        .command("bw")
        .args(["unlock", "--passwordenv", "PURPLE_BW_MASTER", "--raw"])
        .env("PURPLE_BW_MASTER", password)
        .output()
        .context("Failed to run Bitwarden CLI (bw)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Bitwarden unlock failed: {}", stderr.trim());
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        anyhow::bail!("Bitwarden unlock returned empty session token");
    }
    Ok(token)
}

/// Proton Pass CLI authentication status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProtonStatus {
    Authenticated,
    NotAuthenticated,
    NotInstalled,
}

/// Check whether `pass-cli` is installed and the user is logged in. Uses
/// `pass-cli test` (not `info`) because in pass-cli 2.x `info` exits 0 even
/// without a session and only reports the error on stderr. `test` is the
/// command that actually exits non-zero when authentication is missing.
pub fn proton_status(env: &crate::runtime::env::Env) -> ProtonStatus {
    let result = env.command("pass-cli").arg("test").output();
    let status = match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProtonStatus::NotInstalled,
        Err(_) => ProtonStatus::NotAuthenticated,
        Ok(out) if out.status.success() => ProtonStatus::Authenticated,
        Ok(_) => ProtonStatus::NotAuthenticated,
    };
    debug!("[external] Proton Pass status: {status:?}");
    status
}

/// Log in to Proton Pass with a Personal Access Token.
/// PAT is supplied via the `PROTON_PASS_PERSONAL_ACCESS_TOKEN` env var so it
/// never appears in argv. Returns an error wrapping pass-cli's stderr on
/// non-zero exit so the prompt loop can surface it.
pub fn proton_login(env: &crate::runtime::env::Env, pat: &str) -> Result<()> {
    if pat.is_empty() {
        anyhow::bail!("empty PAT");
    }
    let output = env
        .command("pass-cli")
        .arg("login")
        .env("PROTON_PASS_PERSONAL_ACCESS_TOKEN", pat)
        .output()
        .context("Failed to run Proton Pass CLI (pass-cli)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!("[external] Proton Pass login failed: {}", stderr.trim());
        anyhow::bail!("{}", stderr.trim());
    }
    debug!("[external] Proton Pass login succeeded");
    Ok(())
}

/// Parse a `proton:Vault/Item/field` askpass spec into its three components.
/// Vault and item segments cannot contain `/`; the field segment is everything
/// after the second `/`. All three segments must be non-empty.
fn parse_proton_spec(spec: &str) -> Result<(&str, &str, &str)> {
    let (vault, rest) = spec
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("Proton Pass spec must be Vault/Item/field"))?;
    let (item, field) = rest
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("Proton Pass spec must be Vault/Item/field"))?;
    if vault.is_empty() || item.is_empty() || field.is_empty() {
        anyhow::bail!("Proton Pass spec segments must be non-empty");
    }
    Ok((vault, item, field))
}

/// Retrieve a secret from Proton Pass via `pass-cli item view`. The askpass
/// spec `proton:Vault/Item/field` is mapped to name-based lookup flags
/// (`--vault-name`, `--item-title`, `--field`) rather than the URI form, so
/// purple users can refer to their vaults and items by human-readable names
/// instead of opaque share/item IDs.
fn retrieve_from_proton_pass(env: &crate::runtime::env::Env, spec: &str) -> Result<String> {
    let (vault, item, field) = parse_proton_spec(spec)?;
    let result = env
        .command("pass-cli")
        .args([
            "item",
            "view",
            "--vault-name",
            vault,
            "--item-title",
            item,
            "--field",
            field,
        ])
        .output();
    let output = match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            error!("[config] Password manager binary not found: pass-cli");
            return Err(e).context("Failed to run Proton Pass CLI (pass-cli)");
        }
        other => other.context("Failed to run Proton Pass CLI (pass-cli)")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("[external] Proton Pass lookup failed: {}", stderr.trim());
        anyhow::bail!("Proton Pass lookup failed");
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        warn!("[external] Proton Pass returned empty secret");
        anyhow::bail!("Proton Pass returned empty secret");
    }
    debug!("[external] Proton Pass lookup succeeded");
    Ok(value)
}

#[cfg(test)]
#[path = "askpass_tests.rs"]
mod tests;
