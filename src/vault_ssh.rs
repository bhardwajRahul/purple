use anyhow::{Context, Result};
use log::{debug, error, info};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime};

use crate::ssh_config::model::HostEntry;

/// One host resolved to a Vault SSH role, ready for bulk signing.
#[derive(Clone, PartialEq)]
pub struct VaultSignTarget {
    pub alias: String,
    pub role: String,
    pub certificate_file: String,
    pub pubkey: std::path::PathBuf,
    pub vault_addr: Option<String>,
}

/// Manual `Debug` so `vault_addr` (a Vault server hostname revealing
/// infrastructure topology) never appears unredacted in `{:?}` output.
impl std::fmt::Debug for VaultSignTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultSignTarget")
            .field("alias", &self.alias)
            .field("role", &self.role)
            .field("certificate_file", &self.certificate_file)
            .field("pubkey", &self.pubkey)
            .field(
                "vault_addr",
                &self.vault_addr.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Result of a certificate signing operation.
#[derive(Debug)]
pub struct SignResult {
    pub cert_path: PathBuf,
}

/// Certificate validity status.
#[derive(Debug, Clone, PartialEq)]
pub enum CertStatus {
    Valid {
        expires_at: i64,
        remaining_secs: i64,
        /// Total certificate validity window in seconds (to - from), used by
        /// the UI to compute proportional freshness thresholds.
        total_secs: i64,
    },
    Expired,
    Missing,
    Invalid(String),
}

/// Minimum remaining seconds before a cert needs renewal (5 minutes).
pub const RENEWAL_THRESHOLD_SECS: i64 = 300;

/// TTL (in seconds) for the in-memory cert status cache before we re-run
/// `ssh-keygen -L` against an on-disk certificate. Distinct from
/// `RENEWAL_THRESHOLD_SECS`: this controls how often we *re-check* a cert's
/// validity, while `RENEWAL_THRESHOLD_SECS` is the minimum lifetime below which
/// we actually request a new signature from Vault.
pub const CERT_STATUS_CACHE_TTL_SECS: u64 = 300;

/// Shorter TTL for cached `CertStatus::Invalid` entries produced by check
/// failures (e.g. unresolvable cert path). Error entries use this backoff
/// instead of the 5-minute re-check TTL so transient errors recover quickly
/// without hammering the background check thread on every poll tick.
pub const CERT_ERROR_BACKOFF_SECS: u64 = 30;

/// Validate a Vault SSH role path. Accepts ASCII alphanumerics plus `/`, `_` and `-`.
/// Rejects empty strings and values longer than 128 chars.
pub fn is_valid_role(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-')
}

/// Validate a `VAULT_ADDR` value passed to the Vault CLI as an env var.
///
/// Intentionally minimal: reject empty, control characters and whitespace.
/// We do NOT try to parse the URL here — a typo just produces a Vault CLI
/// error, which is fine. The 512-byte ceiling prevents a pathological config
/// line from ballooning the environment block.
pub fn is_valid_vault_addr(s: &str) -> bool {
    let trimmed = s.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 512
        && !trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Normalize a vault address so bare IPs and hostnames work.
///
/// Inputs with an explicit `http://` or `https://` scheme pass through
/// unchanged: the user's port choice (including its absence) is honoured
/// so the HTTP client follows the scheme default. Adding a redundant
/// `:443` breaks strict `Host`-header ACLs on HAProxy and similar
/// proxies once they drop to HTTP/1.1.
///
/// Bare hosts (no scheme) get `https://` prepended. A bare host without
/// a port falls back to Vault's `:8200` default, matching the local dev
/// pattern where `vault.local` or `192.168.1.10` is meant to point at a
/// stock Vault server. With an explicit port (`host:9200`) the user's
/// port wins.
pub fn normalize_vault_addr(s: &str) -> String {
    let trimmed = s.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return trimmed.to_string();
    }
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    let scheme_len = 8;
    let with_scheme = format!("https://{}", trimmed);
    let after_scheme = &with_scheme[scheme_len..];
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    let has_port = if let Some(bracket_end) = authority.rfind(']') {
        authority[bracket_end..].contains(':')
    } else {
        authority.contains(':')
    };
    if has_port {
        with_scheme
    } else {
        let path_start = scheme_len + authority.len();
        format!(
            "{}:8200{}",
            &with_scheme[..path_start],
            &with_scheme[path_start..]
        )
    }
}

/// Scrub a raw Vault CLI stderr for display. Drops lines containing credential-like
/// tokens (token, secret, x-vault-, cookie, authorization), joins the rest with spaces
/// and truncates to 200 chars.
pub fn scrub_vault_stderr(raw: &str) -> String {
    let filtered: String = raw
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !(lower.contains("token")
                || lower.contains("secret")
                || lower.contains("x-vault-")
                || lower.contains("cookie")
                || lower.contains("authorization"))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = filtered.trim();
    if trimmed.is_empty() {
        return "Vault SSH signing failed. Check vault status and policy".to_string();
    }
    if trimmed.chars().count() > 200 {
        trimmed.chars().take(200).collect::<String>() + "..."
    } else {
        trimmed.to_string()
    }
}

/// Return the certificate path for a given alias: `~/.purple/certs/<alias>-cert.pub`
pub fn cert_path_for(paths: Option<&crate::runtime::env::Paths>, alias: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !alias.is_empty()
            && !alias.contains('/')
            && !alias.contains('\\')
            && !alias.contains(':')
            && !alias.contains('\0')
            && !alias.contains(".."),
        "Invalid alias for cert path: '{}'",
        alias
    );
    paths
        .map(|p| p.cert_for(alias))
        .context("Could not determine home directory")
}

/// Resolve the actual certificate file path for a host.
/// Priority: CertificateFile directive > purple's default cert path.
pub fn resolve_cert_path(
    paths: Option<&crate::runtime::env::Paths>,
    alias: &str,
    certificate_file: &str,
) -> Result<PathBuf> {
    if !certificate_file.is_empty() {
        let expanded = if let Some(rest) = certificate_file.strip_prefix("~/") {
            if let Some(p) = paths {
                p.home().join(rest)
            } else {
                PathBuf::from(certificate_file)
            }
        } else {
            PathBuf::from(certificate_file)
        };
        Ok(expanded)
    } else {
        cert_path_for(paths, alias)
    }
}

/// Sign an SSH public key via Vault SSH secrets engine.
/// Runs: `vault write -field=signed_key <role> public_key=@<pubkey_path>`
/// Writes the signed certificate to `~/.purple/certs/<alias>-cert.pub`.
///
/// When `vault_addr` is `Some`, it is set as the `VAULT_ADDR` env var on the
/// `vault` subprocess, overriding whatever the parent shell has configured.
/// When `None`, the subprocess inherits the parent's env (current behavior).
/// This lets purple users configure Vault address at the provider or host
/// level without needing to launch purple from a pre-exported shell.
pub fn sign_certificate(
    env: &crate::runtime::env::Env,
    role: &str,
    pubkey_path: &Path,
    alias: &str,
    vault_addr: Option<&str>,
) -> Result<SignResult> {
    if !pubkey_path.exists() {
        anyhow::bail!(
            "Public key not found: {}. Set IdentityFile on the host or ensure ~/.ssh/id_ed25519.pub exists.",
            pubkey_path.display()
        );
    }

    if !is_valid_role(role) {
        anyhow::bail!("Invalid Vault SSH role: '{}'", role);
    }

    let cert_dest = cert_path_for(env.paths(), alias)?;

    if let Some(parent) = cert_dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| crate::messages::vault_create_dir_failed(&parent.display()))?;
    }

    // The Vault CLI receives the public key path as a UTF-8 argument. `Path::display()`
    // is lossy on non-UTF8 paths and could produce a mangled path Vault would then fail
    // to read. Require a valid UTF-8 path and fail fast with a clear message.
    let pubkey_str = pubkey_path.to_str().context(
        "public key path contains non-UTF8 bytes; vault CLI requires a valid UTF-8 path",
    )?;
    // The Vault CLI parses arguments as `key=value` KV pairs. A path containing
    // `=` would be split mid-argument and produce a cryptic parse error. The
    // check runs on the already-resolved (tilde-expanded) path because that is
    // exactly the byte sequence the CLI will see. A user with a `$HOME` path
    // that itself contains `=` will hit this early; the error message reports
    // the expanded path so they can rename the offending directory.
    if pubkey_str.contains('=') {
        anyhow::bail!(
            "Public key path '{}' contains '=' which is not supported by the Vault CLI argument format. Rename the key file or directory.",
            pubkey_str
        );
    }
    let pubkey_arg = format!("public_key=@{}", pubkey_str);
    debug!(
        "[external] Vault sign request: addr={} role={}",
        vault_addr.unwrap_or("<env>"),
        role
    );
    let mut cmd = env.command("vault");
    cmd.args(["write", "-field=signed_key", role, &pubkey_arg]);
    // Override VAULT_ADDR for this subprocess only when a value was resolved
    // from config. Otherwise leave the env untouched so `vault` keeps using
    // whatever the parent shell (or `~/.vault-token`) provides. The caller
    // (typically `resolve_vault_addr`) is expected to have validated and
    // trimmed the value already — re-checking here is cheap belt-and-braces
    // for callers that construct the `Option<&str>` manually.
    if let Some(addr) = vault_addr {
        anyhow::ensure!(
            is_valid_vault_addr(addr),
            "Invalid VAULT_ADDR '{}' for role '{}'. Check the Vault SSH Address field.",
            addr,
            role
        );
        cmd.env("VAULT_ADDR", addr);
    }
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to run vault CLI. Is vault installed and in PATH?")?;

    // Drain both pipes on background threads to prevent pipe-buffer deadlock.
    // Without this, the vault CLI can block writing to a full stderr pipe
    // (64 KB) while we poll try_wait, causing a false timeout.
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(mut h) = stdout_handle {
            if let Err(e) = std::io::Read::read_to_end(&mut h, &mut buf) {
                log::warn!("[external] Failed to read vault stdout pipe: {e}");
            }
        }
        buf
    });
    let stderr_thread = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(mut h) = stderr_handle {
            if let Err(e) = std::io::Read::read_to_end(&mut h, &mut buf) {
                log::warn!("[external] Failed to read vault stderr pipe: {e}");
            }
        }
        buf
    });

    // Wait up to 30 seconds for the vault CLI to complete. Without a timeout
    // the thread blocks indefinitely when the Vault server is unreachable
    // (e.g. wrong address, firewall, TLS handshake hanging).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // The pipe-drain threads (stdout_thread, stderr_thread)
                    // are dropped without joining here. This is intentional:
                    // kill() closes the child's pipe ends, so read_to_end
                    // returns immediately and the threads self-terminate.
                    error!(
                        "[external] Vault unreachable: {}: timed out after 30s",
                        vault_addr.unwrap_or("<env>")
                    );
                    anyhow::bail!("Vault SSH timed out. Server unreachable.");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("Failed to wait for vault CLI: {}", e);
            }
        }
    };

    let stdout_bytes = stdout_thread.join().unwrap_or_default();
    let stderr_bytes = stderr_thread.join().unwrap_or_default();
    let output = std::process::Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("permission denied") || stderr.contains("403") {
            error!(
                "[external] Vault auth failed: permission denied (role={} addr={})",
                role,
                vault_addr.unwrap_or("<env>")
            );
            anyhow::bail!("Vault SSH permission denied. Check token and policy.");
        }
        if stderr.contains("missing client token") || stderr.contains("token expired") {
            error!(
                "[external] Vault auth failed: token missing or expired (role={} addr={})",
                role,
                vault_addr.unwrap_or("<env>")
            );
            anyhow::bail!("Vault SSH token missing or expired. Run `vault login`.");
        }
        // Check "connection refused" before "dial tcp" because Go's
        // refused-connection error contains both substrings.
        if stderr.contains("connection refused") {
            error!(
                "[external] Vault unreachable: {}: connection refused",
                vault_addr.unwrap_or("<env>")
            );
            anyhow::bail!("Vault SSH connection refused.");
        }
        if stderr.contains("i/o timeout") || stderr.contains("dial tcp") {
            error!(
                "[external] Vault unreachable: {}: connection timed out",
                vault_addr.unwrap_or("<env>")
            );
            anyhow::bail!("Vault SSH connection timed out.");
        }
        if stderr.contains("no such host") {
            error!(
                "[external] Vault unreachable: {}: no such host",
                vault_addr.unwrap_or("<env>")
            );
            anyhow::bail!("Vault SSH host not found.");
        }
        if stderr.contains("server gave HTTP response to HTTPS client") {
            error!(
                "[external] Vault unreachable: {}: server returned HTTP on HTTPS connection",
                vault_addr.unwrap_or("<env>")
            );
            anyhow::bail!("Vault SSH server uses HTTP, not HTTPS. Set address to http://.");
        }
        if stderr.contains("certificate signed by unknown authority")
            || stderr.contains("tls:")
            || stderr.contains("x509:")
        {
            error!(
                "[external] Vault unreachable: {}: TLS error",
                vault_addr.unwrap_or("<env>")
            );
            anyhow::bail!("Vault SSH TLS error. Check certificate or use http://.");
        }
        error!(
            "[external] Vault SSH signing failed: {}",
            scrub_vault_stderr(&stderr)
        );
        anyhow::bail!("Vault SSH failed: {}", scrub_vault_stderr(&stderr));
    }

    let signed_key = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if signed_key.is_empty() {
        anyhow::bail!("Vault returned empty certificate for role '{}'", role);
    }

    crate::fs_util::atomic_write(&cert_dest, signed_key.as_bytes())
        .with_context(|| crate::messages::vault_write_cert_failed(&cert_dest.display()))?;

    info!("[external] Vault SSH certificate signed for {}", alias);
    Ok(SignResult {
        cert_path: cert_dest,
    })
}

/// Check the validity of an SSH certificate file via `ssh-keygen -L`.
///
/// Timezone note: `ssh-keygen -L` outputs local civil time, which `parse_ssh_datetime`
/// converts to pseudo-epoch seconds. Rather than comparing against UTC `now` (which would
/// be wrong in non-UTC zones), we compute the TTL from the parsed from/to difference
/// (timezone-independent) and measure elapsed time since the cert file was written (UTC
/// file mtime vs UTC now). This keeps both sides in the same reference frame.
pub fn check_cert_validity(env: &crate::runtime::env::Env, cert_path: &Path) -> CertStatus {
    if !cert_path.exists() {
        return CertStatus::Missing;
    }

    let output = match env
        .command("ssh-keygen")
        .args(["-L", "-f"])
        .arg(cert_path)
        .output()
    {
        Ok(o) => o,
        Err(e) => return CertStatus::Invalid(crate::messages::vault_ssh_keygen_run_failed(&e)),
    };

    if !output.status.success() {
        return CertStatus::Invalid("ssh-keygen could not read certificate".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Handle certificates signed with no expiration ("Valid: forever").
    for line in stdout.lines() {
        let t = line.trim();
        if t == "Valid: forever" || t.starts_with("Valid: from ") && t.ends_with(" to forever") {
            return CertStatus::Valid {
                expires_at: i64::MAX,
                remaining_secs: i64::MAX,
                total_secs: i64::MAX,
            };
        }
    }

    for line in stdout.lines() {
        if let Some((from, to)) = parse_valid_line(line) {
            let ttl = to - from; // Correct regardless of timezone
            // Defensive: a cert with to < from is malformed. Treat as Invalid
            // rather than propagating a negative ttl into the cache and the
            // renewal threshold calculation.
            if ttl <= 0 {
                return CertStatus::Invalid(
                    "certificate has non-positive validity window".to_string(),
                );
            }

            // Use file modification time as the signing timestamp (UTC)
            let signed_at = match std::fs::metadata(cert_path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            {
                Some(d) => d.as_secs() as i64,
                None => {
                    // Cannot determine file age. Treat as needing renewal.
                    return CertStatus::Expired;
                }
            };

            let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => d.as_secs() as i64,
                Err(_) => {
                    return CertStatus::Invalid("system clock before unix epoch".to_string());
                }
            };

            let elapsed = now - signed_at;
            let remaining = ttl - elapsed;

            if remaining <= 0 {
                return CertStatus::Expired;
            }
            let expires_at = now + remaining;
            return CertStatus::Valid {
                expires_at,
                remaining_secs: remaining,
                total_secs: ttl,
            };
        }
    }

    CertStatus::Invalid("No Valid: line found in certificate".to_string())
}

/// Parse "Valid: from YYYY-MM-DDTHH:MM:SS to YYYY-MM-DDTHH:MM:SS" from ssh-keygen -L.
fn parse_valid_line(line: &str) -> Option<(i64, i64)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("Valid:")?;
    let rest = rest.trim();
    let rest = rest.strip_prefix("from ")?;
    let (from_str, rest) = rest.split_once(" to ")?;
    let to_str = rest.trim();

    let from = parse_ssh_datetime(from_str)?;
    let to = parse_ssh_datetime(to_str)?;
    Some((from, to))
}

/// Parse YYYY-MM-DDTHH:MM:SS to Unix epoch seconds.
/// Note: ssh-keygen outputs local time. We use the same clock for comparison
/// (SystemTime::now gives wall clock), so the relative difference is correct
/// for TTL checks even though the absolute epoch may be off by the UTC offset.
fn parse_ssh_datetime(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;

    if s.as_bytes().get(4) != Some(&b'-')
        || s.as_bytes().get(7) != Some(&b'-')
        || s.as_bytes().get(10) != Some(&b'T')
        || s.as_bytes().get(13) != Some(&b':')
        || s.as_bytes().get(16) != Some(&b':')
    {
        return None;
    }

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if !(0..=23).contains(&hour) || !(0..=59).contains(&min) || !(0..=59).contains(&sec) {
        return None;
    }

    // Civil date to Unix epoch (same algorithm as chrono/time crates).
    let mut y = year;
    let m = if month <= 2 {
        y -= 1;
        month + 9
    } else {
        month - 3
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

/// Check if a certificate needs renewal.
///
/// For certificates whose total validity window is shorter than
/// `RENEWAL_THRESHOLD_SECS`, the fixed 5-minute threshold would flag a freshly
/// signed cert as needing renewal immediately, causing an infinite re-sign loop.
/// In that case we fall back to a proportional threshold (half the total).
pub fn needs_renewal(status: &CertStatus) -> bool {
    match status {
        CertStatus::Missing | CertStatus::Expired | CertStatus::Invalid(_) => true,
        CertStatus::Valid {
            remaining_secs,
            total_secs,
            ..
        } => {
            let threshold = if *total_secs > 0 && *total_secs <= RENEWAL_THRESHOLD_SECS {
                *total_secs / 2
            } else {
                RENEWAL_THRESHOLD_SECS
            };
            *remaining_secs < threshold
        }
    }
}

/// Ensure a valid certificate exists for a host. Signs a new one if needed.
/// Checks at the CertificateFile path (or purple's default) before signing.
pub fn ensure_cert(
    env: &crate::runtime::env::Env,
    role: &str,
    pubkey_path: &Path,
    alias: &str,
    certificate_file: &str,
    vault_addr: Option<&str>,
) -> Result<PathBuf> {
    let check_path = resolve_cert_path(env.paths(), alias, certificate_file)?;
    let status = check_cert_validity(env, &check_path);

    if !needs_renewal(&status) {
        info!(
            "[purple] Vault SSH certificate cache hit: alias={} role={} path={}",
            alias,
            role,
            check_path.display()
        );
        return Ok(check_path);
    }

    log::debug!(
        "[purple] Vault SSH certificate cache miss: alias={} role={} status={:?} -> signing",
        alias,
        role,
        status
    );
    let result = sign_certificate(env, role, pubkey_path, alias, vault_addr)?;
    Ok(result.cert_path)
}

/// Resolve the public key path for signing.
/// Priority: host IdentityFile + ".pub" > ~/.ssh/id_ed25519.pub fallback.
/// Returns an error when the user's home directory cannot be determined. Any
/// IdentityFile pointing outside `$HOME` is rejected and falls back to the
/// default `~/.ssh/id_ed25519.pub` to prevent reading arbitrary filesystem
/// locations via a crafted IdentityFile directive.
pub fn resolve_pubkey_path(
    paths: Option<&crate::runtime::env::Paths>,
    identity_file: &str,
) -> Result<PathBuf> {
    let home = paths
        .context("Could not determine home directory")?
        .home()
        .to_path_buf();
    let fallback = home.join(".ssh/id_ed25519.pub");

    if identity_file.is_empty() {
        return Ok(fallback);
    }

    let expanded = if let Some(rest) = identity_file.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(identity_file)
    };

    // A purely lexical `starts_with(&home)` check can be bypassed by a symlink inside
    // $HOME pointing to a path outside $HOME (e.g. ~/evil -> /etc). Canonicalize both
    // sides so symlinks are resolved, then compare. If the expanded path does not yet
    // exist (or canonicalize fails for any reason) we cannot safely reason about where
    // it actually points, so fall back to the default key path.
    let canonical_home = match std::fs::canonicalize(&home) {
        Ok(p) => p,
        Err(_) => return Ok(fallback),
    };
    if expanded.exists() {
        match std::fs::canonicalize(&expanded) {
            Ok(canonical) if canonical.starts_with(&canonical_home) => {}
            _ => return Ok(fallback),
        }
    } else if !expanded.starts_with(&home) {
        return Ok(fallback);
    }

    if expanded.extension().is_some_and(|ext| ext == "pub") {
        Ok(expanded)
    } else {
        let mut s = expanded.into_os_string();
        s.push(".pub");
        Ok(PathBuf::from(s))
    }
}

/// Resolve the effective vault role for a host.
/// Priority: host-level vault_ssh > provider-level vault_role > None.
///
/// `provider_label` selects between multiple labeled configs of the same
/// provider. None means a bare config (legacy 2-segment marker).
pub fn resolve_vault_role(
    host_vault_ssh: Option<&str>,
    provider_name: Option<&str>,
    provider_label: Option<&str>,
    provider_config: &crate::providers::config::ProviderConfig,
) -> Option<String> {
    if let Some(role) = host_vault_ssh {
        if !role.is_empty() {
            return Some(role.to_string());
        }
    }

    if let Some(name) = provider_name {
        let id = crate::providers::config::ProviderConfigId {
            provider: name.to_string(),
            label: provider_label.map(|s| s.to_string()),
        };
        let section = provider_config
            .section_by_id(&id)
            .or_else(|| provider_config.section(name));
        if let Some(section) = section {
            if !section.vault_role.is_empty() {
                return Some(section.vault_role.clone());
            }
        }
    }

    None
}

/// Resolve the effective Vault address for a host.
///
/// Precedence (highest wins): per-host `# purple:vault-addr` comment,
/// provider `vault_addr=` setting, else None (caller falls back to the
/// `vault` CLI's own env resolution).
///
/// Both layers are re-validated with `is_valid_vault_addr` even though the
/// parser paths (`HostBlock::vault_addr()` and `ProviderConfig::parse`)
/// already drop invalid values. This is defensive: a future caller that
/// constructs a `HostEntry` or `ProviderSection` in-memory (tests, migration
/// code, a new feature) won't be able to smuggle a malformed `VAULT_ADDR`
/// into `sign_certificate` through this resolver.
pub fn resolve_vault_addr(
    host_vault_addr: Option<&str>,
    provider_name: Option<&str>,
    provider_label: Option<&str>,
    provider_config: &crate::providers::config::ProviderConfig,
) -> Option<String> {
    if let Some(addr) = host_vault_addr {
        let trimmed = addr.trim();
        if !trimmed.is_empty() && is_valid_vault_addr(trimmed) {
            return Some(normalize_vault_addr(trimmed));
        }
    }

    if let Some(name) = provider_name {
        let id = crate::providers::config::ProviderConfigId {
            provider: name.to_string(),
            label: provider_label.map(|s| s.to_string()),
        };
        let section = provider_config
            .section_by_id(&id)
            .or_else(|| provider_config.section(name));
        if let Some(section) = section {
            let trimmed = section.vault_addr.trim();
            if !trimmed.is_empty() && is_valid_vault_addr(trimmed) {
                return Some(normalize_vault_addr(trimmed));
            }
        }
    }

    None
}

/// Resolve the effective ProxyJump chain for an alias by asking ssh itself.
///
/// Uses `ssh -G -F <config> <alias>` so wildcard patterns and `Match` blocks
/// contribute the same way they do at connect time. Without this, a host that
/// inherits ProxyJump from a wildcard (e.g. `Host *prod*  ProxyJump bastion`)
/// would look like it has no proxy when read from its own block alone.
///
/// Returns aliases in dependency order: proxies first, the target last. The
/// target is always present, even when ssh resolution yields nothing. Cycles
/// are broken with a visited set. Hosts referenced via ProxyJump that have no
/// matching `Host` block in the config still appear in the chain so callers
/// can decide what to do with them; existence is verified by the caller.
pub fn resolve_proxy_chain(config_path: &Path, alias: &str) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = vec![alias.to_string()];

    while let Some(current) = queue.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        chain.push(current.clone());

        let output = Command::new("ssh")
            .args(["-G", "-F"])
            .arg(config_path)
            .arg("--")
            .arg(&current)
            .output();

        let Ok(output) = output else {
            debug!("[external] ssh -G failed for {}: spawn error", current);
            continue;
        };
        if !output.status.success() {
            debug!(
                "[external] ssh -G non-zero exit for {} (code {:?})",
                current,
                output.status.code()
            );
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let lower = line.to_ascii_lowercase();
            let Some(rest) = lower.strip_prefix("proxyjump ") else {
                continue;
            };
            // ssh -G emits literal "none" when no proxy is configured.
            if rest.trim() == "none" {
                continue;
            }
            // Use the original-case slice for the value; ssh prints the
            // proxyjump value verbatim after the lower-cased key.
            // strip_prefix already guarantees line.len() >= "proxyjump ".len().
            let value = &line["proxyjump ".len()..];
            for jump in value.split(',') {
                let host = parse_proxy_jump_host(jump.trim());
                if !host.is_empty() {
                    queue.push(host.to_string());
                }
            }
        }
    }

    chain.reverse();
    chain
}

/// Extract the host portion from a single `[user@]host[:port]` ProxyJump entry.
/// Handles bracketed IPv6 hosts like `[::1]:22`.
fn parse_proxy_jump_host(jump: &str) -> &str {
    let trimmed = jump.trim();
    let after_user = trimmed.rsplit_once('@').map(|(_, h)| h).unwrap_or(trimmed);
    if let Some(rest) = after_user.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &rest[..end];
        }
    }
    after_user.split(':').next().unwrap_or(after_user)
}

/// One row in the Keys-tab Vault SSH strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveCert {
    /// Host alias the cert belongs to.
    pub alias: String,
    /// Role name from `# purple:vault-ssh <role>`.
    pub role: String,
    /// Seconds remaining on the cert.
    pub remaining_secs: i64,
    /// Total signed-cert validity window in seconds. Used by the gauge
    /// to compute `remaining/total` for the fill ratio.
    pub total_secs: i64,
}

/// True iff a host has any purple-managed Vault context: either an
/// explicit `# purple:vault-ssh` role marker, or a `CertificateFile`
/// directive pointing into purple's certs directory. The second branch
/// covers users who sign certs directly with the `vault` CLI and wire them
/// in via `CertificateFile` without setting the role marker.
pub fn has_purple_vault_context(
    host: &HostEntry,
    paths: Option<&crate::runtime::env::Paths>,
) -> bool {
    host.vault_ssh.is_some() || cert_file_in_purple_dir(&host.certificate_file, paths)
}

/// `CertificateFile` path looks like a purple-managed cert when it points
/// into the resolved certs directory (`<data>/certs/`) or into the legacy
/// `.purple/certs/`. The legacy match is a substring check so it works
/// whether the path is tilde-expanded or absolute; the resolved match
/// expands a leading `~/` against the home directory first.
pub fn cert_file_in_purple_dir(
    cert_file: &str,
    paths: Option<&crate::runtime::env::Paths>,
) -> bool {
    if cert_file.is_empty() {
        return false;
    }
    if cert_file.contains("/.purple/certs/") {
        return true;
    }
    let Some(paths) = paths else {
        return false;
    };
    let expanded = match cert_file.strip_prefix("~/") {
        Some(rest) => paths.home().join(rest),
        None => PathBuf::from(cert_file),
    };
    expanded.starts_with(paths.certs_dir())
}

/// True when any host has a purple-managed Vault context. The Keys-tab
/// strip renders iff this returns true. Even hosts whose cert is not
/// yet cached count, so the strip appears the moment the user
/// configures their first Vault role or sets a cert path.
pub fn vault_ssh_in_use(hosts: &[HostEntry], paths: Option<&crate::runtime::env::Paths>) -> bool {
    hosts.iter().any(|h| has_purple_vault_context(h, paths))
}

/// Build the strip's row list from the cert cache. Hosts that have a
/// configured role (or a purple-managed cert path) but no cached
/// `Valid` status are omitted; the gauge has nothing to fill until the
/// lazy cert check populates the cache. Sort: longest remaining first
/// so the user sees healthy certs at the top and expiring ones at the
/// bottom.
pub fn active_certs_for_strip(
    hosts: &[HostEntry],
    cache: &HashMap<String, (Instant, CertStatus, Option<SystemTime>)>,
    paths: Option<&crate::runtime::env::Paths>,
) -> Vec<ActiveCert> {
    // Recompute `remaining_secs` against the current wall clock instead
    // of using the cached snapshot. The cached number was correct only
    // at the moment the check ran; the strip is redrawn on every event
    // tick (~20× per second), so deriving from `expires_at - now` gives
    // a per-second countdown without re-running the cert validation.
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut rows: Vec<ActiveCert> = hosts
        .iter()
        .filter(|h| has_purple_vault_context(h, paths))
        .filter_map(|h| {
            let role = h.vault_ssh.clone().unwrap_or_default();
            match cache.get(&h.alias) {
                Some((
                    _,
                    CertStatus::Valid {
                        expires_at,
                        remaining_secs,
                        total_secs,
                    },
                    _,
                )) => {
                    // `expires_at == 0` is the demo sentinel for "no
                    // wall clock"; fall back to the static cached value
                    // so visual fixtures stay byte-deterministic.
                    let live_remaining = if *expires_at == 0 {
                        *remaining_secs
                    } else {
                        (*expires_at - now).max(0)
                    };
                    Some(ActiveCert {
                        alias: h.alias.clone(),
                        role,
                        remaining_secs: live_remaining,
                        total_secs: *total_secs,
                    })
                }
                _ => None,
            }
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.remaining_secs));
    rows
}

/// Compute the fill ratio (0.0..=1.0) for a Vault SSH cert TTL gauge.
/// Clamped so a cert in renewal-overlap or one whose `total_secs` was
/// recorded as `i64::MAX` ("Valid: forever") does not produce NaN.
pub fn cert_fill_ratio(remaining_secs: i64, total_secs: i64) -> f32 {
    if total_secs <= 0 || remaining_secs <= 0 {
        return 0.0;
    }
    if total_secs == i64::MAX || remaining_secs >= total_secs {
        return 1.0;
    }
    (remaining_secs as f32 / total_secs as f32).clamp(0.0, 1.0)
}

/// Format remaining certificate time for display.
pub fn format_remaining(remaining_secs: i64) -> String {
    if remaining_secs <= 0 {
        return "expired".to_string();
    }
    let hours = remaining_secs / 3600;
    let mins = (remaining_secs % 3600) / 60;
    if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

#[cfg(test)]
#[path = "vault_ssh_tests.rs"]
mod tests;
