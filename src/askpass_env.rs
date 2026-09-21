//! SSH_ASKPASS environment wiring shared by `connection`, `tunnel`, `file_browser`
//! and `snippet`. Lives in the library crate so every ssh/scp call site can route
//! through a single configuration point and a single regression test covers them all.

use std::path::Path;
use std::process::Command;

/// Alias a one-shot password belongs to. Set on a retry run together with
/// `ONESHOT_SECRET_VAR`, never in argv. The askpass subprocess hands the
/// secret out only when the hop it authenticates resolves to this alias, so
/// a ProxyJump bastion never receives the target's password.
pub(crate) const ONESHOT_ALIAS_VAR: &str = "PURPLE_ASKPASS_ONESHOT_ALIAS";

/// The one-shot password itself. Lives only in the environment of the ssh
/// child for one run, following the `PURPLE_BW_MASTER` precedent.
pub(crate) const ONESHOT_SECRET_VAR: &str = "PURPLE_ASKPASS_ONESHOT";

/// Set on every background ssh, which also carries
/// `SINGLE_PASSWORD_PROMPT_OPT`. ssh then asks once per hop, so the retry
/// marker has no loop left to break and arming one would only refuse the
/// second ssh of a two-step operation, such as the remote home lookup
/// followed by its listing. The interactive terminal path sets nothing and
/// keeps the marker.
pub(crate) const SINGLE_ATTEMPT_VAR: &str = "PURPLE_ASKPASS_SINGLE_ATTEMPT";

/// `-o` value that makes ssh fail at once instead of asking on the tty.
/// Disables the `password` and `keyboard-interactive` methods, so it is
/// only set when no password source and no session password is available.
pub(crate) const BATCH_MODE_OPT: &str = "BatchMode=yes";

/// `-o` value that limits ssh to one password attempt. A background run has
/// exactly one answer to give: a vault returns the same value each time, and
/// a password typed in the TUI is handed out once. Asking again only sends
/// an empty attempt and burns another failed login on a server that counts
/// them. The interactive terminal path keeps ssh's default of three, where
/// a person really can type a different password.
pub(crate) const SINGLE_PASSWORD_PROMPT_OPT: &str = "NumberOfPasswordPrompts=1";

/// True when a background ssh child has no way to answer a password prompt:
/// no configured source and no password typed earlier this session. The
/// caller then passes `BATCH_MODE_OPT` so ssh exits with `Permission denied`
/// instead of blocking on a hidden prompt.
pub(crate) fn needs_batch_mode(askpass: Option<&str>, session_password: Option<&str>) -> bool {
    askpass.is_none() && session_password.is_none()
}

/// Wire every authentication input a background ssh child can use: the
/// askpass program when a source or a session password exists, the one-shot
/// variables for a session password and the Bitwarden session token. Every
/// caller is a background run, so the single-attempt flag rides along with
/// the askpass program. Args and stdio stay with the caller.
pub(crate) fn configure_auth(
    cmd: &mut Command,
    alias: &str,
    config_path: &Path,
    askpass: Option<&str>,
    session_password: Option<&str>,
    bw_session: Option<&str>,
) {
    if askpass.is_some() || session_password.is_some() {
        configure_ssh_command(cmd, alias, config_path);
        cmd.env(SINGLE_ATTEMPT_VAR, "1");
    }
    if let Some(secret) = session_password {
        cmd.env(ONESHOT_ALIAS_VAR, alias)
            .env(ONESHOT_SECRET_VAR, secret);
    }
    if let Some(token) = bw_session {
        cmd.env("BW_SESSION", token);
    }
}

/// Configure an `ssh` or `scp` [`Command`] so the child process invokes purple
/// as its SSH_ASKPASS program. Sets:
///
/// - `SSH_ASKPASS` to the current purple binary (falling back to argv\[0\]).
/// - `SSH_ASKPASS_REQUIRE=force` so OpenSSH invokes askpass regardless of whether
///   a TTY is attached or `DISPLAY`/`WAYLAND_DISPLAY` is set. OpenSSH's `prefer`
///   mode gates askpass on a non-empty `DISPLAY` or `WAYLAND_DISPLAY` (see
///   `readpass.c` in openssh-portable); inside a headless ssh session on Linux
///   both are empty, so `prefer` would silently no-op and ssh would fall back to
///   the TTY prompt, bypassing purple's vault lookup entirely.
/// - `PURPLE_ASKPASS_MODE`, `PURPLE_HOST_ALIAS`, `PURPLE_CONFIG_PATH` so the
///   askpass subprocess (re-entering purple) can look up the right host config.
///
/// Only the env vars are set; stdio, args and working directory are left to the
/// caller. `BW_SESSION` is also the caller's concern since not every call site
/// forwards it explicitly.
pub(crate) fn configure_ssh_command(cmd: &mut Command, alias: &str, config_path: &Path) {
    let exe = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::args().next())
        .unwrap_or_else(|| "purple".to_string());
    cmd.env("SSH_ASKPASS", &exe)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("PURPLE_ASKPASS_MODE", "1")
        .env("PURPLE_HOST_ALIAS", alias)
        .env("PURPLE_CONFIG_PATH", config_path.as_os_str());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// Snapshot the env vars configured on a Command into a HashMap for inspection.
    /// Skips entries whose value is `None` (those are env removals, not additions).
    fn snapshot_envs(cmd: &Command) -> HashMap<OsString, OsString> {
        cmd.get_envs()
            .filter_map(|(k, v)| v.map(|val| (k.to_os_string(), val.to_os_string())))
            .collect()
    }

    #[test]
    fn sets_ssh_askpass_require_to_force() {
        // Regression test for GitHub issue #19: purple previously used `prefer`,
        // which silently no-ops when DISPLAY and WAYLAND_DISPLAY are empty. `force`
        // bypasses that gate. This test locks the value so a future change back to
        // `prefer` (or any other value) fails CI.
        let mut cmd = Command::new("ssh");
        configure_ssh_command(&mut cmd, "myhost", &PathBuf::from("/tmp/cfg"));
        let envs = snapshot_envs(&cmd);
        assert_eq!(
            envs.get(&OsString::from("SSH_ASKPASS_REQUIRE")),
            Some(&OsString::from("force")),
            "SSH_ASKPASS_REQUIRE must be 'force' to work in headless ssh sessions"
        );
    }

    #[test]
    fn sets_ssh_askpass_to_current_exe() {
        let mut cmd = Command::new("ssh");
        configure_ssh_command(&mut cmd, "myhost", &PathBuf::from("/tmp/cfg"));
        let envs = snapshot_envs(&cmd);
        let askpass = envs
            .get(&OsString::from("SSH_ASKPASS"))
            .expect("SSH_ASKPASS must be set");
        assert!(
            !askpass.is_empty(),
            "SSH_ASKPASS must point at a non-empty path"
        );
    }

    #[test]
    fn sets_purple_context_vars() {
        let mut cmd = Command::new("ssh");
        configure_ssh_command(&mut cmd, "myhost", &PathBuf::from("/tmp/my/ssh_config"));
        let envs = snapshot_envs(&cmd);
        assert_eq!(
            envs.get(&OsString::from("PURPLE_ASKPASS_MODE")),
            Some(&OsString::from("1"))
        );
        assert_eq!(
            envs.get(&OsString::from("PURPLE_HOST_ALIAS")),
            Some(&OsString::from("myhost"))
        );
        assert_eq!(
            envs.get(&OsString::from("PURPLE_CONFIG_PATH")),
            Some(&OsString::from("/tmp/my/ssh_config"))
        );
    }

    #[test]
    fn passes_alias_with_spaces_and_slashes_unmodified() {
        let mut cmd = Command::new("ssh");
        configure_ssh_command(&mut cmd, "my host/with slash", &PathBuf::from("/tmp/cfg"));
        let envs = snapshot_envs(&cmd);
        assert_eq!(
            envs.get(&OsString::from("PURPLE_HOST_ALIAS")),
            Some(&OsString::from("my host/with slash"))
        );
    }

    #[test]
    fn does_not_set_bw_session() {
        // BW_SESSION forwarding is the caller's responsibility (connection.rs
        // forwards it explicitly; tunnel.rs relies on inheritance). The helper
        // must not touch it.
        let mut cmd = Command::new("ssh");
        configure_ssh_command(&mut cmd, "myhost", &PathBuf::from("/tmp/cfg"));
        let envs = snapshot_envs(&cmd);
        assert!(!envs.contains_key(&OsString::from("BW_SESSION")));
    }

    #[test]
    fn needs_batch_mode_only_without_source_and_session_password() {
        assert!(needs_batch_mode(None, None));
        assert!(!needs_batch_mode(Some("keychain"), None));
        assert!(!needs_batch_mode(None, Some("hunter2")));
        assert!(!needs_batch_mode(Some("bw:item"), Some("hunter2")));
    }

    #[test]
    fn configure_auth_without_inputs_sets_nothing() {
        let mut cmd = Command::new("ssh");
        configure_auth(&mut cmd, "h", &PathBuf::from("/tmp/cfg"), None, None, None);
        assert!(snapshot_envs(&cmd).is_empty());
    }

    #[test]
    fn configure_auth_with_source_wires_askpass_only() {
        let mut cmd = Command::new("ssh");
        configure_auth(
            &mut cmd,
            "h",
            &PathBuf::from("/tmp/cfg"),
            Some("keychain"),
            None,
            None,
        );
        let envs = snapshot_envs(&cmd);
        assert_eq!(
            envs.get(&OsString::from("SSH_ASKPASS_REQUIRE")),
            Some(&OsString::from("force"))
        );
        assert!(!envs.contains_key(&OsString::from(ONESHOT_SECRET_VAR)));
        assert!(!envs.contains_key(&OsString::from(ONESHOT_ALIAS_VAR)));
    }

    #[test]
    fn configure_auth_with_session_password_wires_askpass_and_oneshot() {
        // A session password travels through askpass too, so the askpass
        // program must be wired even when the host has no source.
        let mut cmd = Command::new("ssh");
        configure_auth(
            &mut cmd,
            "web1",
            &PathBuf::from("/tmp/cfg"),
            None,
            Some("hunter2"),
            None,
        );
        let envs = snapshot_envs(&cmd);
        assert_eq!(
            envs.get(&OsString::from("SSH_ASKPASS_REQUIRE")),
            Some(&OsString::from("force"))
        );
        assert_eq!(
            envs.get(&OsString::from(ONESHOT_ALIAS_VAR)),
            Some(&OsString::from("web1"))
        );
        assert_eq!(
            envs.get(&OsString::from(ONESHOT_SECRET_VAR)),
            Some(&OsString::from("hunter2"))
        );
    }

    #[test]
    fn configure_auth_forwards_bw_session() {
        let mut cmd = Command::new("ssh");
        configure_auth(
            &mut cmd,
            "h",
            &PathBuf::from("/tmp/cfg"),
            Some("bw:item"),
            None,
            Some("tok"),
        );
        let envs = snapshot_envs(&cmd);
        assert_eq!(
            envs.get(&OsString::from("BW_SESSION")),
            Some(&OsString::from("tok"))
        );
    }

    #[test]
    fn oneshot_secret_never_lands_in_argv() {
        let mut cmd = Command::new("ssh");
        configure_auth(
            &mut cmd,
            "h",
            &PathBuf::from("/tmp/cfg"),
            None,
            Some("hunter2"),
            None,
        );
        assert!(cmd.get_args().all(|a| a != "hunter2"));
    }
}
