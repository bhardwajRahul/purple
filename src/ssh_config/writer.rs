use std::fs;
use std::time::SystemTime;

use anyhow::{Context, Result};
use log::{debug, error};

use super::model::{ConfigElement, SshConfigFile};
use crate::fs_util;

impl SshConfigFile {
    /// Write the config back to disk.
    /// Creates a backup before writing and uses atomic write (temp file + rename).
    /// Resolves symlinks so the rename targets the real file, not the link.
    /// Acquires an advisory lock to prevent concurrent writes from multiple
    /// purple processes or background sync threads.
    pub fn write(&self) -> Result<()> {
        if crate::demo_flag::is_demo() {
            debug!("[purple] ssh_config.write skipped (demo mode)");
            return Ok(());
        }
        // Resolve symlinks so we write through to the real file
        let target_path = fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());
        debug!(
            "[config] ssh_config.write: target={} elements={}",
            target_path.display(),
            self.elements.len()
        );

        // Acquire advisory lock (blocks until available)
        let _lock = fs_util::FileLock::acquire(&target_path)
            .inspect_err(|e| {
                debug!(
                    "[config] ssh_config.write: lock acquire failed: {} ({})",
                    target_path.display(),
                    e
                );
            })
            .context("Failed to acquire config lock")?;

        // Create backup if the file exists, keep only last 5.
        // Use the canonical `target_path` so backups land next to the real
        // file rather than next to a symlink. Without this, a user with
        // `~/.ssh/config -> /Volumes/dotfiles/ssh_config` would see backups
        // accumulate in `~/.ssh/` even though the actual file lives on the
        // dotfiles volume. Recovery becomes confusing.
        if target_path.exists() {
            self.create_backup(&target_path)
                .context("Failed to create backup of SSH config")?;
            self.prune_backups(&target_path, 5).ok();
        }

        // Persist with one blank line between every top-level block. serialize()
        // stays byte-for-byte round-trip faithful for undo and comparison; only
        // the on-disk file is normalized, healing configs whose blocks were
        // glued together before purple managed them.
        let raw = self.serialized_lines();
        let normalized = ensure_block_separators(&raw);
        let healed = normalized.len() - raw.len();
        if healed > 0 {
            debug!(
                "[config] ssh_config.write: inserted {healed} block separator(s) in {}",
                target_path.display()
            );
        }
        let content = self.lines_to_string(&normalized);

        fs_util::atomic_write(&target_path, content.as_bytes())
            .map_err(|err| {
                error!(
                    "[purple] SSH config write failed: {}: {err}",
                    target_path.display()
                );
                err
            })
            .with_context(|| format!("Failed to write SSH config to {}", target_path.display()))?;

        debug!(
            "[config] ssh_config.write: wrote {} bytes to {}",
            content.len(),
            target_path.display()
        );

        // Lock released on drop
        Ok(())
    }

    /// Serialize the config to a string.
    /// Collapses consecutive blank lines to prevent accumulation after deletions.
    /// Round-trip faithful: blank-line layout is preserved exactly as parsed.
    pub fn serialize(&self) -> String {
        self.lines_to_string(&self.serialized_lines())
    }

    /// Flatten the element tree to content lines (no line endings), collapsing
    /// runs of blank lines to at most one. Shared by `serialize` and `write`.
    fn serialized_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        for element in &self.elements {
            match element {
                ConfigElement::GlobalLine(line) => {
                    lines.push(line.clone());
                }
                ConfigElement::HostBlock(block) => {
                    lines.push(block.raw_host_line.clone());
                    for directive in &block.directives {
                        lines.push(directive.raw_line.clone());
                    }
                }
                ConfigElement::Include(include) => {
                    lines.push(include.raw_line.clone());
                }
            }
        }

        // Collapse consecutive blank lines (keep at most one)
        let mut collapsed = Vec::with_capacity(lines.len());
        let mut prev_blank = false;
        for line in lines {
            let is_blank = line.trim().is_empty();
            if is_blank && prev_blank {
                continue;
            }
            prev_blank = is_blank;
            collapsed.push(line);
        }
        collapsed
    }

    /// Join content lines with the file's line ending, restoring the BOM and
    /// guaranteeing exactly one trailing newline.
    fn lines_to_string(&self, lines: &[String]) -> String {
        let line_ending = if self.crlf { "\r\n" } else { "\n" };
        let mut result = String::new();
        // Restore UTF-8 BOM if the original file had one
        if self.bom {
            result.push('\u{FEFF}');
        }
        for line in lines {
            result.push_str(line);
            result.push_str(line_ending);
        }
        // Ensure files always end with exactly one newline
        // (check lines instead of result, since BOM makes result non-empty)
        if lines.is_empty() {
            result.push_str(line_ending);
        }
        result
    }

    /// Create a timestamped backup of the current config file.
    /// Backup files are created with chmod 600 to match the source file's sensitivity.
    fn create_backup(&self, target_path: &std::path::Path) -> Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let backup_name = format!(
            "{}.bak.{}",
            target_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            timestamp
        );
        let backup_path = target_path.with_file_name(backup_name);
        fs::copy(target_path, &backup_path).with_context(|| {
            format!(
                "Failed to copy {} to {}",
                target_path.display(),
                backup_path.display()
            )
        })?;

        // Set backup permissions to 600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(&backup_path, fs::Permissions::from_mode(0o600)) {
                debug!(
                    "[config] Failed to set backup permissions on {}: {e}",
                    backup_path.display()
                );
            }
        }

        Ok(())
    }

    /// Remove old backups, keeping only the most recent `keep` files.
    fn prune_backups(&self, target_path: &std::path::Path, keep: usize) -> Result<()> {
        let parent = target_path.parent().context("No parent directory")?;
        let prefix = format!(
            "{}.bak.",
            target_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let mut backups: Vec<_> = fs::read_dir(parent)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .collect();
        // Sort by mtime so prune is robust against future timestamp-digit-width
        // changes. Filename sort would silently break if the millisecond
        // suffix length ever grew.
        backups.sort_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });
        if backups.len() > keep {
            for old in &backups[..backups.len() - keep] {
                if let Err(e) = fs::remove_file(old.path()) {
                    debug!(
                        "[config] Failed to prune old backup {}: {e}",
                        old.path().display()
                    );
                }
            }
        }
        Ok(())
    }
}

/// True when `line` begins a top-level block (`Host`/`Match` at column 0).
/// Keyword match is case-insensitive, matching the parser's own detection.
fn is_block_start(line: &str) -> bool {
    if line.starts_with(char::is_whitespace) {
        return false;
    }
    match line.split_whitespace().next() {
        Some(kw) => kw.eq_ignore_ascii_case("Host") || kw.eq_ignore_ascii_case("Match"),
        None => false,
    }
}

/// Insert a single blank line before each top-level block that runs directly
/// into the previous line, so persisted configs never have glued-together Host
/// blocks. A block kept glued to a preceding top-level comment (a group header
/// or hand-written label) is left as-is. Operates on collapsed lines, so it can
/// never create consecutive blanks.
fn ensure_block_separators(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    for line in lines {
        if is_block_start(line)
            && let Some(prev) = out.last()
        {
            let prev_blank = prev.trim().is_empty();
            let prev_top_level_comment =
                !prev.starts_with(char::is_whitespace) && prev.trim_start().starts_with('#');
            if !prev_blank && !prev_top_level_comment {
                out.push(String::new());
            }
        }
        out.push(line.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh_config::model::HostEntry;

    fn parse_str(content: &str) -> SshConfigFile {
        SshConfigFile {
            elements: SshConfigFile::parse_content(content),
            path: tempfile::tempdir()
                .expect("tempdir")
                .keep()
                .join("test_config"),
            crlf: crate::ssh_config::parser::detect_crlf_majority(content),
            bom: false,
        }
    }

    #[test]
    fn test_round_trip_basic() {
        let content = "\
Host myserver
  HostName 192.168.1.10
  User admin
  Port 2222
";
        let config = parse_str(content);
        assert_eq!(config.serialize(), content);
    }

    #[test]
    fn test_round_trip_with_comments() {
        let content = "\
# My SSH config
# Generated by hand

Host alpha
  HostName alpha.example.com
  # Deploy user
  User deploy

Host beta
  HostName beta.example.com
  User root
";
        let config = parse_str(content);
        assert_eq!(config.serialize(), content);
    }

    #[test]
    fn test_round_trip_with_globals_and_wildcards() {
        let content = "\
# Global settings
Host *
  ServerAliveInterval 60
  ServerAliveCountMax 3

Host production
  HostName prod.example.com
  User deployer
  IdentityFile ~/.ssh/prod_key
";
        let config = parse_str(content);
        assert_eq!(config.serialize(), content);
    }

    #[test]
    fn test_add_host_serializes() {
        let mut config = parse_str("Host existing\n  HostName 10.0.0.1\n");
        config.add_host(&HostEntry {
            alias: "newhost".to_string(),
            hostname: "10.0.0.2".to_string(),
            user: "admin".to_string(),
            port: 22,
            ..Default::default()
        });
        let output = config.serialize();
        assert!(output.contains("Host newhost"));
        assert!(output.contains("HostName 10.0.0.2"));
        assert!(output.contains("User admin"));
        // Port 22 is default, should not be written
        assert!(!output.contains("Port 22"));
    }

    #[test]
    fn test_delete_host_serializes() {
        let content = "\
Host alpha
  HostName alpha.example.com

Host beta
  HostName beta.example.com
";
        let mut config = parse_str(content);
        config.delete_host("alpha");
        let output = config.serialize();
        assert!(!output.contains("Host alpha"));
        assert!(output.contains("Host beta"));
    }

    #[test]
    fn test_update_host_serializes() {
        let content = "\
Host myserver
  HostName 10.0.0.1
  User old_user
";
        let mut config = parse_str(content);
        config.update_host(
            "myserver",
            &HostEntry {
                alias: "myserver".to_string(),
                hostname: "10.0.0.2".to_string(),
                user: "new_user".to_string(),
                port: 22,
                ..Default::default()
            },
        );
        let output = config.serialize();
        assert!(output.contains("HostName 10.0.0.2"));
        assert!(output.contains("User new_user"));
        assert!(!output.contains("old_user"));
    }

    #[test]
    fn test_update_host_preserves_unknown_directives() {
        let content = "\
Host myserver
  HostName 10.0.0.1
  User admin
  ForwardAgent yes
  LocalForward 8080 localhost:80
  Compression yes
";
        let mut config = parse_str(content);
        config.update_host(
            "myserver",
            &HostEntry {
                alias: "myserver".to_string(),
                hostname: "10.0.0.2".to_string(),
                user: "admin".to_string(),
                port: 22,
                ..Default::default()
            },
        );
        let output = config.serialize();
        assert!(output.contains("HostName 10.0.0.2"));
        assert!(output.contains("ForwardAgent yes"));
        assert!(output.contains("LocalForward 8080 localhost:80"));
        assert!(output.contains("Compression yes"));
    }

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|l| (*l).to_string()).collect()
    }

    #[test]
    fn ensure_block_separators_splits_glued_hosts() {
        let input = lines(&["Host a", "  HostName 1", "Host b", "  HostName 2"]);
        let out = ensure_block_separators(&input);
        assert_eq!(
            out,
            lines(&["Host a", "  HostName 1", "", "Host b", "  HostName 2"])
        );
    }

    #[test]
    fn ensure_block_separators_leaves_separated_hosts() {
        let input = lines(&["Host a", "  HostName 1", "", "Host b", "  HostName 2"]);
        let out = ensure_block_separators(&input);
        assert_eq!(out, input, "already-separated input must be untouched");
    }

    #[test]
    fn ensure_block_separators_keeps_group_header_glue() {
        // A top-level comment (group header) directly above a Host stays glued:
        // that separation is intentional, not the bug.
        let input = lines(&["# purple:group DigitalOcean", "Host a", "  HostName 1"]);
        let out = ensure_block_separators(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn ensure_block_separators_splits_three_glued_hosts() {
        let input = lines(&["Host a", "  HostName 1", "Host b", "  HostName 2", "Host c"]);
        let out = ensure_block_separators(&input);
        assert_eq!(
            out,
            lines(&[
                "Host a",
                "  HostName 1",
                "",
                "Host b",
                "  HostName 2",
                "",
                "Host c",
            ])
        );
    }

    #[test]
    fn ensure_block_separators_splits_glued_match_block() {
        let input = lines(&["Host a", "  HostName 1", "Match host b", "  User x"]);
        let out = ensure_block_separators(&input);
        assert_eq!(
            out,
            lines(&["Host a", "  HostName 1", "", "Match host b", "  User x"])
        );
    }

    #[test]
    fn ensure_block_separators_no_leading_blank() {
        let input = lines(&["Host a", "  HostName 1"]);
        let out = ensure_block_separators(&input);
        assert_eq!(out, input, "must not insert a blank before the first block");
    }

    /// Serialize against demo-flag mutators (asset generation, the visual
    /// suite) and pin demo off. `SshConfigFile::write` no-ops in demo mode, so
    /// a parallel demo-mode test flipping the global flag would make these
    /// on-disk reads observe a file that was never written.
    fn ondisk_guard() -> std::sync::MutexGuard<'static, ()> {
        let g = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::demo_flag::disable();
        g
    }

    #[test]
    fn write_normalization_is_idempotent() {
        let _g = ondisk_guard();
        // Writing a healed config and re-parsing it must produce the same bytes
        // on a second write. Mirrors the fuzz round-trip/idempotency invariant
        // for the glued-hosts mutation class.
        let glued = "Host a\n  HostName 1\nhost b\n  HostName 2\nMatch host c\n  User x\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let config = SshConfigFile {
            elements: SshConfigFile::parse_content(glued),
            path: path.clone(),
            crlf: false,
            bom: false,
        };
        config.write().unwrap();
        let first = std::fs::read_to_string(&path).unwrap();

        let reparsed = SshConfigFile {
            elements: SshConfigFile::parse_content(&first),
            path: path.clone(),
            crlf: false,
            bom: false,
        };
        reparsed.write().unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second, "write normalization must be idempotent");
        assert!(!first.contains("\n\n\n"), "no triple blanks:\n{first}");
    }

    #[test]
    fn ensure_block_separators_case_insensitive_keyword() {
        // SSH keywords are case-insensitive; lowercase `host`/`match` must heal
        // too, matching the parser's own case-insensitive detection.
        let input = lines(&["host a", "  HostName 1", "MATCH host b", "  User x"]);
        let out = ensure_block_separators(&input);
        assert_eq!(
            out,
            lines(&["host a", "  HostName 1", "", "MATCH host b", "  User x"])
        );
    }

    #[test]
    fn write_normalizes_glued_hosts_on_disk_serialize_stays_pure() {
        let _g = ondisk_guard();
        // serialize() must stay byte-for-byte round-trip faithful (glued stays
        // glued), but the persisted file gets a blank line between the blocks.
        let glued = "Host a\n  HostName 1.1.1.1\nHost b\n  HostName 2.2.2.2\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let config = SshConfigFile {
            elements: SshConfigFile::parse_content(glued),
            path: path.clone(),
            crlf: false,
            bom: false,
        };

        // serialize() is unchanged: still glued.
        assert_eq!(config.serialize(), glued);

        // write() normalizes: blank line between the two blocks on disk.
        config.write().unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            on_disk,
            "Host a\n  HostName 1.1.1.1\n\nHost b\n  HostName 2.2.2.2\n"
        );
    }

    #[test]
    fn write_normalizes_glued_hosts_preserves_crlf() {
        let _g = ondisk_guard();
        let glued = "Host a\r\n  HostName 1.1.1.1\r\nHost b\r\n  HostName 2.2.2.2\r\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let config = SshConfigFile {
            elements: SshConfigFile::parse_content(glued),
            path: path.clone(),
            crlf: true,
            bom: false,
        };
        config.write().unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            on_disk,
            "Host a\r\n  HostName 1.1.1.1\r\n\r\nHost b\r\n  HostName 2.2.2.2\r\n"
        );
    }
}
