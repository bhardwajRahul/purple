//! Centralized user-facing messages.
//!
//! Every string the user can see (toasts, CLI output, error messages) lives
//! here. Handler, CLI and UI code reference these constants and functions
//! instead of inlining string literals. This makes copy consistent, auditable
//! and future-proof for i18n.
//!
//! Domain submodules (host, tunnel, provider, vault, snippet, container,
//! picker) own their own slice of the surface and are glob-re-exported
//! below so callers continue to write `crate::messages::X` unchanged.

pub mod container;
pub mod host;
pub mod picker;
pub mod provider;
pub mod snippet;
pub mod tunnel;
pub mod vault;

pub use container::*;
pub use host::*;
pub use picker::*;
pub use provider::*;
pub use snippet::*;
pub use tunnel::*;
pub use vault::*;

// ── General / shared ────────────────────────────────────────────────

pub const FAILED_TO_SAVE: &str = "Failed to save";
pub fn failed_to_save(e: &impl std::fmt::Display) -> String {
    format!("{}: {}", FAILED_TO_SAVE, e)
}

pub const CONFIG_CHANGED_EXTERNALLY: &str =
    "Config changed externally. Press Esc and re-open to pick up changes.";

// ── Demo mode ───────────────────────────────────────────────────────

pub const DEMO_CONNECTION_DISABLED: &str = "Demo mode. Connection disabled.";
pub const DEMO_SYNC_DISABLED: &str = "Demo mode. Sync disabled.";
pub const DEMO_TUNNELS_DISABLED: &str = "Demo mode. Tunnels disabled.";
pub const DEMO_VAULT_SIGNING_DISABLED: &str = "Demo mode. Vault SSH signing disabled.";
pub const DEMO_FILE_BROWSER_DISABLED: &str = "Demo mode. File browser disabled.";
pub const DEMO_CONTAINER_REFRESH_DISABLED: &str = "Demo mode. Container refresh disabled.";
pub const DEMO_CONTAINER_ACTIONS_DISABLED: &str = "Demo mode. Container actions disabled.";
pub const DEMO_EXECUTION_DISABLED: &str = "Demo mode. Execution disabled.";
pub const DEMO_PROVIDER_CHANGES_DISABLED: &str = "Demo mode. Provider config changes disabled.";

// ── Ping ────────────────────────────────────────────────────────────

pub fn pinging_host(alias: &str, show_hint: bool) -> String {
    if show_hint {
        format!("Pinging {}... (Shift+P pings all)", alias)
    } else {
        format!("Pinging {}...", alias)
    }
}

pub fn bastion_not_found(alias: &str) -> String {
    format!("Bastion {} not found in config.", alias)
}

// ── Clipboard subprocess errors ─────────────────────────────────────
//
// Surfaced when `pbcopy`/`xclip`/`wl-copy` fails to spawn, write to its
// stdin, or be reaped. The cmd name is the binary the platform picked.

pub fn clipboard_run_failed(cmd: &str) -> String {
    format!("Failed to run {}.", cmd)
}

pub fn clipboard_write_failed(cmd: &str) -> String {
    format!("Failed to write to {}.", cmd)
}

pub fn clipboard_wait_failed(cmd: &str) -> String {
    format!("Failed to wait for {}.", cmd)
}

pub fn clipboard_exited_error(cmd: &str) -> String {
    format!("{} exited with error.", cmd)
}

// ── Import errors ───────────────────────────────────────────────────
//
// Bubble up to the CLI via `eprintln!("{}", e)` when the user runs
// `purple import` against a missing or unreadable file.

pub fn import_open_failed(path: &impl std::fmt::Display, e: &impl std::fmt::Display) -> String {
    format!("Can't open {}: {}", path, e)
}

pub fn import_known_hosts_open_failed(e: &impl std::fmt::Display) -> String {
    format!("Can't open known_hosts: {}", e)
}

pub const IMPORT_HOME_DIR_UNKNOWN: &str = "Could not determine home directory.";
pub const IMPORT_KNOWN_HOSTS_MISSING: &str = "~/.ssh/known_hosts not found.";

// ── Import ──────────────────────────────────────────────────────────

pub fn imported_hosts(imported: usize, skipped: usize) -> String {
    format!(
        "Imported {} host{}, skipped {} duplicate{}.",
        imported,
        if imported == 1 { "" } else { "s" },
        skipped,
        if skipped == 1 { "" } else { "s" }
    )
}

pub fn all_hosts_exist(skipped: usize) -> String {
    if skipped == 1 {
        "Host already exists.".to_string()
    } else {
        format!("All {} hosts already exist.", skipped)
    }
}

// ── Connection ──────────────────────────────────────────────────────

pub fn opened_in_tmux(alias: &str) -> String {
    format!("Opened {} in new tmux window.", alias)
}

pub fn tmux_error(e: &impl std::fmt::Display) -> String {
    format!("tmux: {}", e)
}

pub fn connection_failed(alias: &str) -> String {
    format!("Connection to {} failed.", alias)
}

/// Stderr line printed when the ssh subprocess itself failed to spawn or
/// wait (e.g. binary missing, signal interrupted), distinct from a
/// non-zero exit code which the user sees via the toast.
pub fn connection_spawn_failed(e: &impl std::fmt::Display) -> String {
    format!("Connection failed: {}", e)
}

/// Toast shown when ssh exited non-zero with a captured stderr line we
/// can show. The reason is the trimmed last meaningful line of ssh stderr.
pub fn ssh_failed_with_reason(alias: &str, reason: &str) -> String {
    format!("SSH to {} failed. {}", alias, reason)
}

/// Toast shown when ssh exited non-zero with no captured stderr to relay.
/// The exit code is the only signal we have left.
pub fn ssh_exited_with_code(alias: &str, code: i32) -> String {
    format!("SSH to {} exited with code {}.", alias, code)
}

// ── Transfer ────────────────────────────────────────────────────────

pub const TRANSFER_COMPLETE: &str = "Transfer complete.";

// ── Background / event loop ─────────────────────────────────────────

/// Per-provider sync progress line with a leading spinner frame so
/// `event_loop::handle_tick` animates the prefix while the message is
/// on screen. Format: `⠋ Proxmox VE: Resolving IPs (1/5)...`. Mirrors
/// the spinner contract used by `synced_progress` so the footer keeps
/// animating even when granular per-provider progress overrides the
/// batch summary mid-sync.
pub fn provider_progress(spinner: &str, name: &str, message: &str) -> String {
    format!("{} {}: {}", spinner, name, message)
}

// ── Relative age (detail panel "checked" suffix) ────────────────────

pub const AGE_JUST_NOW: &str = "just now";

/// Compact relative age: "just now", "12s ago", "3m ago", "2h ago",
/// "2d ago". Used in the detail panel so the reader can tell stale
/// data from fresh.
pub fn relative_age(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 5 {
        AGE_JUST_NOW.to_string()
    } else if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

// ── Config reload ───────────────────────────────────────────────────

pub fn config_reloaded(count: usize) -> String {
    format!("Config reloaded. {} hosts.", count)
}

// ── Sync background ─────────────────────────────────────────────────

/// In-progress sync line for the footer. Format:
/// `⠋ Syncing AWS, Hetzner · 1/3 (+12 ~3 -1)`.
/// Active provider names lead so the user immediately sees which provider
/// is currently in flight (especially relevant when one provider is slow).
/// `done/total` follows as a counter. The leading character is a braille
/// spinner frame rotated on every tick. The `(+a ~u -s)` suffix is omitted
/// when all counts are zero.
///
/// Callers MUST only invoke this when `active_names` is non-empty (i.e.
/// at least one provider is still in flight). The only call site is
/// `main::set_sync_summary`, which enters this branch via `still_syncing`,
/// itself gated on `!providers.syncing.is_empty()` — so `active_names`
/// (built from `syncing.keys()`) is guaranteed non-empty.
pub fn synced_progress(
    spinner: &str,
    active_names: &str,
    done: usize,
    total: usize,
    added: usize,
    updated: usize,
    stale: usize,
) -> String {
    debug_assert!(
        !active_names.is_empty(),
        "synced_progress must only be called while a provider is still in flight"
    );
    let diff = sync_diff_suffix(added, updated, stale);
    format!(
        "{} Syncing {} \u{00B7} {}/{}{}",
        spinner, active_names, done, total, diff
    )
}

/// Final sync summary for the footer once all providers in the batch have
/// completed. Format: `Synced 5/5 · AWS, DO, Vultr, Hetzner, Linode (+12 ~3 -1)`.
/// No spinner prefix, no auto-tick: the message expires by length-proportional
/// timeout once the batch is done.
pub fn synced_done(
    done: usize,
    total: usize,
    names: &str,
    added: usize,
    updated: usize,
    stale: usize,
) -> String {
    let diff = sync_diff_suffix(added, updated, stale);
    format!("Synced {}/{} \u{00B7} {}{}", done, total, names, diff)
}

fn sync_diff_suffix(added: usize, updated: usize, stale: usize) -> String {
    let parts: Vec<String> = [(added, '+'), (updated, '~'), (stale, '-')]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, sign)| format!("{}{}", sign, n))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(" "))
    }
}

pub const SYNC_THREAD_SPAWN_FAILED: &str = "Failed to start sync thread.";

pub const SYNC_UNKNOWN_PROVIDER: &str = "Unknown provider.";

pub fn sync_skipped_external_change() -> &'static str {
    "Config changed on disk during sync. Re-run sync after reviewing your edits."
}

// ── Clipboard ───────────────────────────────────────────────────────

pub const NO_CLIPBOARD_TOOL: &str =
    "No clipboard tool found. Install pbcopy (macOS), wl-copy (Wayland), or xclip/xsel (X11).";

// ── MCP server ──────────────────────────────────────────────────────

pub const MCP_TOOL_DENIED_READ_ONLY: &str = "Tool denied. Server started with --read-only. Restart without --read-only to enable state-changing tools.";

/// Bare message body. Callers add the `[purple]` fault-domain prefix at
/// their `warn!` / `error!` site; the `eprintln!` startup diagnostic emits
/// this body directly without the tag.
pub fn mcp_audit_init_failed(path: &impl std::fmt::Display, e: &impl std::fmt::Display) -> String {
    format!(
        "Failed to initialise MCP audit log at {}: {}. Continuing without audit logging.",
        path, e
    )
}

/// Bare message body. Callers add `[purple]` at the log macro site.
pub fn mcp_audit_write_failed(e: &impl std::fmt::Display) -> String {
    format!("Failed to write MCP audit entry: {}", e)
}

/// Returned to the MCP client as `isError` content when the SSH config path
/// does not point to an existing file. Surfaces the bug class where a
/// missing-file silently yields an empty host list.
pub fn mcp_config_file_not_found(path: &impl std::fmt::Display) -> String {
    format!("SSH config file not found: {}", path)
}

/// Logged when `dirs::home_dir()` cannot resolve a home for the audit log
/// default. Auditing is silently disabled in this state, so the operator
/// needs an explicit cue.
pub const MCP_AUDIT_HOME_DIR_UNAVAILABLE: &str = "Could not determine home directory; MCP audit log disabled. Set --audit-log <PATH> explicitly to enable auditing.";

// ── Jump ─────────────────────────────────────────────────

/// Placeholder shown in the jump bar input when the query is empty.
pub const PALETTE_PLACEHOLDER: &str = "Find anything";
/// Empty-state copy when the current query has no matches.
pub const PALETTE_NO_RESULTS: &str = "No matches.";
/// Toast shown when the user dispatches a snippet from the jump bar while
/// no host is selected (the snippet picker needs at least one target).
pub const PALETTE_SNIPPET_NEEDS_HOST: &str =
    "Pick a host first, then run a snippet from the jump bar.";
/// Suffix appended to the truncated row list when the visible window is
/// smaller than the result list.
pub fn jump_more_rows(n: usize) -> String {
    format!("+{n} more (scroll down)")
}

// ── CLI messages ────────────────────────────────────────────────────

#[path = "messages/cli.rs"]
pub mod cli;
pub mod footer;

// ── Update messages ─────────────────────────────────────────────────

pub mod update {
    pub const WHATS_NEW_HINT: &str = "Press n inside purple to see what's new.";
    pub const DONE: &str = "done.";
    pub const CHECKSUM_OK: &str = "ok.";
    pub const SUDO_WARNING: &str =
        "Running via sudo. Consider fixing directory permissions instead.";

    /// Two-space-indented progress prefixes printed before each step.
    /// Trailing space is intentional so the success/fail glyph or
    /// `DONE` constant follows on the same line, matching the visual
    /// rhythm of the updater output.
    pub const STEP_CHECKING: &str = "  Checking for updates... ";
    pub const STEP_VERIFYING_CHECKSUM: &str = "  Verifying checksum... ";
    pub const STEP_INSTALLING: &str = "  Installing... ";

    pub fn already_on(current: &str) -> String {
        format!("already on v{} (latest).", current)
    }

    pub fn available(latest: &str, current: &str) -> String {
        format!("v{} available (current: v{}).", latest, current)
    }

    /// Two-space-indented progress prefix for the download step. Matches
    /// the trailing-space convention of the other STEP_* constants so
    /// the next print resumes on the same line.
    pub fn step_downloading(version: &str) -> String {
        format!("  Downloading v{}... ", version)
    }

    /// Indented sudo warning rendered before the download step. The
    /// caller passes a pre-bolded bang (`!`) so the line reads
    /// `  ! Running via sudo. ...` with the `!` emphasized.
    pub fn sudo_warning_line(bold_bang: &str) -> String {
        format!("  {} {}", bold_bang, SUDO_WARNING)
    }

    pub fn header(bold_name: &str) -> String {
        format!("\n  {} updater\n", bold_name)
    }

    pub fn binary_path(path: &std::path::Path) -> String {
        format!("  Binary: {}", path.display())
    }

    pub fn installed_at(bold_version: &str, path: &std::path::Path) -> String {
        format!("\n  {} installed at {}.", bold_version, path.display())
    }

    pub fn whats_new_hint_indented() -> String {
        format!("\n  {}", WHATS_NEW_HINT)
    }
}

// ── Askpass / password prompts ───────────────────────────────────────

pub mod askpass {
    pub const BW_NOT_FOUND: &str = "Bitwarden CLI (bw) not found. SSH will prompt for password.";
    pub const BW_NOT_LOGGED_IN: &str = "Bitwarden vault not logged in. Run 'bw login' first.";
    pub const EMPTY_PASSWORD: &str = "Empty password. SSH will prompt for password.";
    pub const PASSWORD_IN_KEYCHAIN: &str = "Password stored in keychain.";

    pub fn read_failed(e: &impl std::fmt::Display) -> String {
        format!("Failed to read password: {}", e)
    }

    pub fn unlock_failed_retry(e: &impl std::fmt::Display) -> String {
        format!("Unlock failed: {}. Try again.", e)
    }

    pub fn unlock_failed_prompt(e: &impl std::fmt::Display) -> String {
        format!("Unlock failed: {}. SSH will prompt for password.", e)
    }

    /// CLI prompt shown by the inline askpass path when the user has no
    /// stored credential yet. The trailing space is intentional — the
    /// reader echoes user input directly after.
    pub fn password_prompt(alias: &str) -> String {
        format!("Password for {}: ", alias)
    }

    /// CLI prompt shown when keychain storage is the sink. Reminds the
    /// user that the entry will be persisted, not just used once.
    pub fn keychain_password_prompt(alias: &str) -> String {
        format!("Password for {} (stored in keychain): ", alias)
    }

    /// Stderr line emitted when the keychain `add-generic-password` call
    /// failed. The user falls back to ssh's own prompt on the next try.
    pub fn keychain_store_failed(e: &impl std::fmt::Display) -> String {
        format!(
            "Failed to store in keychain: {}. SSH will prompt for password.",
            e
        )
    }

    pub const PROTON_NOT_FOUND: &str =
        "Proton Pass CLI (pass-cli) not found. SSH will prompt for password.";

    pub const PROTON_LOGIN_PROMPT: &str = "Proton Pass PAT: ";

    pub const PROTON_LOGIN_SUCCESS: &str = "Logged in to Proton Pass.";

    pub fn proton_login_failed_retry(e: &impl std::fmt::Display) -> String {
        format!("Proton Pass login failed: {}. Try again.", e)
    }

    pub fn proton_login_failed_prompt(e: &impl std::fmt::Display) -> String {
        format!(
            "Proton Pass login failed: {}. SSH will prompt for password.",
            e
        )
    }
}

// ── Logging ─────────────────────────────────────────────────────────

pub mod logging {
    pub fn init_failed(e: &impl std::fmt::Display) -> String {
        format!("[purple] Failed to initialize logger: {}", e)
    }

    pub const SSH_VERSION_FAILED: &str = "[purple] Failed to detect SSH version. Is ssh installed?";
}

// ── Form field hints / placeholders ─────────────────────────────────
//
// Dimmed placeholder text shown in empty form fields. Centralized here
// so every user-visible string lives in one place and is auditable.

pub mod hints {
    // ── Shared ──────────────────────────────────────────────────────
    // Picker hints mention "Space" because per the design system keyboard
    // invariants, Enter always submits a form; pickers open on Space.
    // Keep these strings in sync with scripts/check-keybindings.sh.
    pub const IDENTITY_FILE_PICK: &str = "Space to pick a key";
    pub const DEFAULT_SSH_USER: &str = "root";

    // ── Host form ───────────────────────────────────────────────────
    pub const HOST_ALIAS: &str = "e.g. prod or db-01";
    pub const HOST_ALIAS_PATTERN: &str = "10.0.0.* or *.example.com";
    pub const HOST_HOSTNAME: &str = "192.168.1.1 or example.com";
    pub const HOST_PORT: &str = "22";
    pub const HOST_PROXY_JUMP: &str = "Space to pick a host";
    pub const HOST_VAULT_SSH: &str = "e.g. ssh-client-signer/sign/my-role (auth via vault login)";
    pub const HOST_VAULT_SSH_PICKER: &str = "Space to pick a role or type one";
    pub const HOST_VAULT_ADDR: &str =
        "e.g. http://127.0.0.1:8200 (inherits from provider or env when empty)";
    pub const HOST_TAGS: &str = "e.g. prod, staging, us-east (comma-separated)";
    pub const HOST_ASKPASS_PICK: &str = "Space to pick a source";

    pub fn askpass_default(default: &str) -> String {
        format!("default: {}", default)
    }

    pub fn inherits_from(value: &str, provider: &str) -> String {
        format!("inherits {} from {}", value, provider)
    }

    // ── Tunnel form ─────────────────────────────────────────────────
    pub const TUNNEL_BIND_PORT: &str = "8080";
    pub const TUNNEL_REMOTE_HOST: &str = "localhost";
    pub const TUNNEL_REMOTE_PORT: &str = "80";

    // ── Snippet form ────────────────────────────────────────────────
    pub const SNIPPET_NAME: &str = "check-disk";
    pub const SNIPPET_COMMAND: &str = "df -h";
    pub const SNIPPET_OPTIONAL: &str = "(optional)";
    /// Default-hosts field placeholder (the field opens a picker on Space).
    pub const SNIPPET_DEFAULT_HOSTS: &str = "Space to pick hosts";

    // ── Provider form ───────────────────────────────────────────────
    pub const PROVIDER_URL: &str = "https://pve.example.com:8006";
    pub const PROVIDER_TOKEN_DEFAULT: &str = "your-api-token";
    pub const PROVIDER_TOKEN_PROXMOX: &str = "user@pam!token=secret";
    pub const PROVIDER_TOKEN_AWS: &str = "AccessKeyId:Secret[:SessionToken] (or use Profile)";
    pub const PROVIDER_TOKEN_GCP: &str = "/path/to/service-account.json (or access token)";
    pub const PROVIDER_TOKEN_AZURE: &str = "/path/to/service-principal.json (or access token)";
    pub const PROVIDER_TOKEN_TAILSCALE: &str = "API key (leave empty for local CLI)";
    pub const PROVIDER_TOKEN_ORACLE: &str = "~/.oci/config";
    pub const PROVIDER_TOKEN_OVH: &str = "app_key:app_secret:consumer_key";
    pub const PROVIDER_PROFILE: &str = "Name from ~/.aws/credentials (or use Token)";
    pub const PROVIDER_PROJECT_DEFAULT: &str = "my-gcp-project-id";
    pub const PROVIDER_PROJECT_OVH: &str = "Public Cloud project ID";
    pub const PROVIDER_COMPARTMENT: &str = "ocid1.compartment.oc1..aaaa...";
    pub const PROVIDER_REGIONS_DEFAULT: &str = "Space to select regions";
    pub const PROVIDER_REGIONS_GCP: &str = "Space to select zones (empty = all)";
    pub const PROVIDER_REGIONS_SCALEWAY: &str = "Space to select zones";
    // Azure regions is a text input (not a picker), so no key is mentioned.
    pub const PROVIDER_REGIONS_AZURE: &str = "comma-separated subscription IDs";
    pub const PROVIDER_REGIONS_OVH: &str = "Space to select endpoint (default: EU)";
    pub const PROVIDER_USER_AWS: &str = "ec2-user";
    pub const PROVIDER_USER_GCP: &str = "ubuntu";
    pub const PROVIDER_USER_AZURE: &str = "azureuser";
    pub const PROVIDER_USER_ORACLE: &str = "opc";
    pub const PROVIDER_USER_OVH: &str = "ubuntu";
    pub const PROVIDER_VAULT_ROLE: &str =
        "e.g. ssh-client-signer/sign/my-role (vault login; inherited)";
    pub const PROVIDER_VAULT_ADDR: &str = "e.g. http://127.0.0.1:8200 (inherited by all hosts)";
    pub const PROVIDER_ALIAS_PREFIX_DEFAULT: &str = "prefix";
    pub const PROVIDER_LABEL: &str = "short name, e.g. server1 or work";
}

#[cfg(test)]
mod hints_tests {
    use super::hints;

    #[test]
    fn askpass_default_formats() {
        assert_eq!(hints::askpass_default("keychain"), "default: keychain");
    }

    #[test]
    fn askpass_default_formats_empty() {
        assert_eq!(hints::askpass_default(""), "default: ");
    }

    #[test]
    fn inherits_from_formats() {
        assert_eq!(
            hints::inherits_from("role/x", "aws"),
            "inherits role/x from aws"
        );
    }

    #[test]
    fn picker_hints_mention_space_not_enter() {
        // Per the keyboard invariants, pickers open on Space.
        // If these assertions fail, audit scripts/check-keybindings.sh too.
        for s in [
            hints::IDENTITY_FILE_PICK,
            hints::HOST_PROXY_JUMP,
            hints::HOST_VAULT_SSH_PICKER,
            hints::HOST_ASKPASS_PICK,
            hints::PROVIDER_REGIONS_DEFAULT,
            hints::PROVIDER_REGIONS_GCP,
            hints::PROVIDER_REGIONS_SCALEWAY,
            hints::PROVIDER_REGIONS_OVH,
        ] {
            assert!(
                s.starts_with("Space "),
                "picker hint must mention Space: {s}"
            );
            assert!(!s.contains("Enter "), "picker hint must not say Enter: {s}");
        }
    }
}

#[path = "messages/whats_new.rs"]
pub mod whats_new;

#[path = "messages/whats_new_toast.rs"]
pub mod whats_new_toast;

#[cfg(test)]
mod relative_age_tests {
    use super::relative_age;
    use std::time::Duration;

    #[test]
    fn relative_age_boundaries() {
        assert_eq!(relative_age(Duration::from_secs(0)), "just now");
        assert_eq!(relative_age(Duration::from_secs(4)), "just now");
        assert_eq!(relative_age(Duration::from_secs(5)), "5s ago");
        assert_eq!(relative_age(Duration::from_secs(59)), "59s ago");
        assert_eq!(relative_age(Duration::from_secs(60)), "1m ago");
        assert_eq!(relative_age(Duration::from_secs(3599)), "59m ago");
        assert_eq!(relative_age(Duration::from_secs(3600)), "1h ago");
        assert_eq!(relative_age(Duration::from_secs(86399)), "23h ago");
        assert_eq!(relative_age(Duration::from_secs(86400)), "1d ago");
        assert_eq!(relative_age(Duration::from_secs(86400 * 7)), "7d ago");
    }
}
