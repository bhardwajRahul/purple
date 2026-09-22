use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, UNIX_EPOCH};

use log::debug;

use crate::ssh_config::model::HostEntry;

mod native;

/// Resolve the SSH directory (`~/.ssh`) from the injected paths. Returns
/// `None` when the home directory is unknown, so callers can short-circuit
/// cleanly. Tests pass a sandboxed `Paths` instead of touching the real `~/.ssh`.
pub fn resolve_ssh_dir(paths: Option<&crate::runtime::env::Paths>) -> Option<PathBuf> {
    paths.map(crate::runtime::env::Paths::ssh_dir)
}

/// Information about an SSH key found on disk.
#[derive(Debug, Clone)]
pub struct SshKeyInfo {
    /// Display name (filename without path, e.g. "id_ed25519")
    pub name: String,
    /// Display path with tilde (e.g. "~/.ssh/id_ed25519")
    pub display_path: String,
    /// Key type (e.g. "ED25519", "RSA", "sk-ED25519")
    pub key_type: String,
    /// Key bits (e.g. "256", "4096")
    pub bits: String,
    /// SHA256 fingerprint
    pub fingerprint: String,
    /// Comment from the public key
    pub comment: String,
    /// Host aliases that reference this key via IdentityFile
    pub linked_hosts: Vec<String>,
    /// Drunken Bishop visual fingerprint in the `ssh-keygen -lv` layout.
    /// 11 lines (top border + 9 content + bottom border), joined with `\n`.
    /// Empty when no art block could be produced.
    pub bishop_art: String,
    /// Strength score 0..=100. Composed of algorithm strength, key size and
    /// on-disk encryption. Hardware-bound `sk-*` keys score highest;
    /// deprecated DSA and short RSA score lowest.
    pub strength_score: u8,
    /// Private key on disk is passphrase-encrypted. Read from the key file
    /// header, or from the `ssh-keygen -y -P "" -f <key>` exit status for
    /// other formats. False when the private key is missing or not a
    /// regular file.
    pub encrypted: bool,
    /// Public key fingerprint matches an entry returned by `ssh-add -l`.
    pub agent_loaded: bool,
    /// File is an OpenSSH user certificate. Detected via `-cert.pub`
    /// filename suffix or a `-cert` substring in the ssh-keygen-reported
    /// key type (see `detect_certificate`).
    pub is_certificate: bool,
    /// File mtime of the private key (or pubkey when private is missing),
    /// expressed as seconds since UNIX epoch. None when the file system
    /// cannot report a timestamp. Powers the `Modified` field on the
    /// Keys tab hero panel; mtime is the most portable proxy because
    /// birthtime is not exposed by every supported filesystem.
    pub mtime_ts: Option<u64>,
}

impl SshKeyInfo {
    /// Format type with bits (e.g. "ED25519" or "RSA 4096").
    pub fn type_display(&self) -> String {
        if self.bits.is_empty() {
            self.key_type.clone()
        } else {
            format!("{} {}", self.key_type, self.bits)
        }
    }

    /// Drunken Bishop art split into rendering-ready lines. Empty Vec when
    /// `bishop_art` is empty (e.g. when ssh-keygen failed at discovery).
    pub fn bishop_lines(&self) -> Vec<&str> {
        if self.bishop_art.is_empty() {
            Vec::new()
        } else {
            self.bishop_art.lines().collect()
        }
    }
}

/// Character ladder for the Drunken Bishop random-art. Index 0 is the
/// unvisited cell; counter values 1..=14 map to ascending visit density;
/// 15 marks the bishop's start position (S) and 16 the end position (E).
const BISHOP_CHARS: &[u8] = b" .o+=*BOX@%&#/^SE";

const BISHOP_COUNTER_CAP: u8 = 14;
const BISHOP_S_INDEX: u8 = 15;
const BISHOP_E_INDEX: u8 = 16;

/// Decode an OpenSSH `SHA256:<base64>` fingerprint string into its raw
/// hash bytes. OpenSSH emits unpadded base64 with potentially non-zero
/// trailing bits (synthetic fingerprints from demo fixtures sometimes
/// fall in this bucket), so we configure a lenient engine that accepts
/// both padded and unpadded input. Returns `None` when the `SHA256:`
/// prefix is missing or the body is not decodable.
pub fn decode_fingerprint(fp_str: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
    let b64 = fp_str.strip_prefix("SHA256:")?;
    let config = GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true);
    let engine = GeneralPurpose::new(&base64::alphabet::STANDARD, config);
    engine.decode(b64).ok()
}

/// Generate the Drunken Bishop visit grid from a fingerprint. Mirrors the
/// `key_fingerprint_randomart` walk in OpenSSH `sshkey.c`: starting at
/// the grid centre, the bishop steps diagonally based on 2-bit pairs read
/// LSB-first from each fingerprint byte. Each visited cell increments its
/// counter (capped so unique cells stay distinguishable), and the start
/// and end positions are tagged with `S` and `E` markers.
///
/// `cols` and `rows` are the interior cell dimensions. Use 17×9 to match
/// the canonical OpenSSH output, or scale up for a more prominent visual.
pub fn drunken_bishop_grid(fp_bytes: &[u8], cols: usize, rows: usize) -> Vec<Vec<u8>> {
    let mut grid = vec![vec![0u8; cols]; rows];
    let mut x = cols / 2;
    let mut y = rows / 2;
    let start = (x, y);
    for &byte in fp_bytes {
        let mut b = byte;
        for _ in 0..4 {
            let dx: isize = if b & 0x1 == 0 { -1 } else { 1 };
            let dy: isize = if b & 0x2 == 0 { -1 } else { 1 };
            x = (x as isize + dx).clamp(0, cols as isize - 1) as usize;
            y = (y as isize + dy).clamp(0, rows as isize - 1) as usize;
            if grid[y][x] < BISHOP_COUNTER_CAP {
                grid[y][x] += 1;
            }
            b >>= 2;
        }
    }
    grid[start.1][start.0] = BISHOP_S_INDEX;
    grid[y][x] = BISHOP_E_INDEX;
    grid
}

/// Map a Drunken Bishop counter value to its display character.
pub fn bishop_char(counter: u8) -> char {
    let idx = counter.min(BISHOP_E_INDEX) as usize;
    BISHOP_CHARS[idx] as char
}

/// Translate a UI-space selection index into an `app.keys.list` index,
/// honoring an active search filter. Returns `None` when the selection
/// is out of range for the current filter. Centralised here so every
/// call site (copy, push, detail-pane render) maps selection back to
/// `app.keys.list` through one code path; a divergent implementation in any
/// of them would silently point at the wrong key.
pub fn resolve_selection(keys: &[SshKeyInfo], query: Option<&str>, sel: usize) -> Option<usize> {
    let filtered = filtered_key_indices(keys, query);
    filtered.get(sel).copied()
}

/// Indices into `keys` whose `name` or `comment` contains `query`
/// (case-insensitive substring). Returns all indices when the query is
/// empty or `None`, so callers can render the unfiltered list using the
/// same code path. Pure function so the search handler can call it
/// repeatedly per keystroke without touching the App.
pub fn filtered_key_indices(keys: &[SshKeyInfo], query: Option<&str>) -> Vec<usize> {
    match query {
        None | Some("") => (0..keys.len()).collect(),
        Some(q) => {
            let needle = q.to_ascii_lowercase();
            keys.iter()
                .enumerate()
                .filter(|(_, k)| {
                    k.name.to_ascii_lowercase().contains(&needle)
                        || k.comment.to_ascii_lowercase().contains(&needle)
                })
                .map(|(i, _)| i)
                .collect()
        }
    }
}

/// Discover SSH keys in the given directory and cross-reference with host entries.
///
/// Runs `ssh-add -l` once at the start so each key knows whether its
/// fingerprint is currently loaded in the agent. The result is a snapshot:
/// keys added to or removed from the agent after this call will not show
/// up until the next discover_keys() invocation (host reload).
pub fn discover_keys(
    paths: Option<&crate::runtime::env::Paths>,
    ssh_dir: &Path,
    hosts: &[HostEntry],
) -> Vec<SshKeyInfo> {
    let entries = match std::fs::read_dir(ssh_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let started = Instant::now();
    let home = paths.map(|p| p.home().to_path_buf());
    let agent_fingerprints = agent_loaded_fingerprints();

    let mut keys: Vec<SshKeyInfo> = entries
        .filter_map(|e| e.ok())
        .filter(is_public_key_file)
        .filter_map(|e| {
            read_key_info(
                ssh_dir,
                &e.path(),
                home.as_deref(),
                hosts,
                &agent_fingerprints,
            )
        })
        .collect();

    keys.sort_by(|a, b| a.name.cmp(&b.name));
    debug!(
        "[purple] discover_keys: found {} key(s) in {}, {} loaded in agent, took {}ms",
        keys.len(),
        ssh_dir.display(),
        agent_fingerprints.len(),
        started.elapsed().as_millis()
    );
    keys
}

/// Fingerprints (SHA256 form, including the `SHA256:` prefix) of every key
/// currently loaded in the running ssh-agent. Empty when the agent has no
/// identities, is not reachable, or `ssh-add` is missing. Each failure
/// path emits one debug line so a user reporting "agent column always
/// reads `not loaded`" has a trace pointing at the cause.
fn agent_loaded_fingerprints() -> HashSet<String> {
    let output = Command::new("ssh-add").arg("-l").output();
    match output {
        Ok(o) if o.status.success() => parse_agent_list(&String::from_utf8_lossy(&o.stdout)),
        Ok(o) => {
            let code = o.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&o.stderr);
            log::debug!(
                "[external] ssh-add -l non-zero exit={code} stderr={}",
                stderr.trim().lines().next().unwrap_or("<empty>"),
            );
            HashSet::new()
        }
        Err(e) => {
            log::debug!("[external] ssh-add spawn failed: {e}");
            HashSet::new()
        }
    }
}

/// Parse `ssh-add -l` stdout into a fingerprint set. Each line has the
/// format `<bits> SHA256:<hash> <comment> (<TYPE>)`; we extract column 2.
/// Lines that do not start with a numeric bit count are skipped (covers
/// the "The agent has no identities." string and any future banner).
fn parse_agent_list(stdout: &str) -> HashSet<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() >= 2 && parts[1].starts_with("SHA256:") {
                Some(parts[1].to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Compute the strength score for a key. Pure function so we can unit-test
/// every algorithm/bit combo without subprocess calls. Hardware-bound `sk-*`
/// keys are floored at 90 since the private material never leaves the token;
/// deprecated DSA and short RSA collapse to single-digit scores.
fn strength_score_for(key_type: &str, bits: &str, encrypted: bool) -> u8 {
    // OpenSSH spells hardware-key types as `sk-ed25519` / `sk-ecdsa-...`,
    // but ssh-keygen output sometimes uppercases the prefix. One
    // case-insensitive prefix check covers both.
    let is_sk = key_type.to_ascii_lowercase().starts_with("sk-");
    let base: i16 = if is_sk {
        95
    } else {
        match key_type.to_ascii_uppercase().as_str() {
            "DSA" => 5,
            "RSA" => match bits.parse::<u32>().unwrap_or(0) {
                0..=1023 => 5,
                1024..=2047 => 15,
                2048..=3071 => 55,
                3072..=4095 => 75,
                _ => 80,
            },
            "ECDSA" => match bits.parse::<u32>().unwrap_or(0) {
                256 => 70,
                384 => 80,
                521 => 85,
                _ => 60,
            },
            "ED25519" => 90,
            _ => 50,
        }
    };
    let modifier: i16 = if encrypted { 5 } else { -10 };
    (base + modifier).clamp(0, 100) as u8
}

/// Detect whether a private key file is passphrase-encrypted. Known formats
/// are read from the file header. Other formats fall back to deriving the
/// public key with an empty passphrase: success means unencrypted, failure
/// means encrypted (or unreadable). Returns false when the private key is
/// absent or not a regular file, so unbacked .pub files are not flagged as
/// encrypted and a FIFO or device cannot block the scan.
fn private_key_encrypted(private_path: &Path) -> bool {
    if !private_path.is_file() {
        return false;
    }
    if let Some(encrypted) = native::private_key_encrypted(private_path) {
        return encrypted;
    }
    debug!(
        "[purple] key scan: ssh-keygen fallback for encryption check of {}",
        private_path.display()
    );
    keygen_private_key_encrypted(private_path)
}

fn keygen_private_key_encrypted(private_path: &Path) -> bool {
    let output = Command::new("ssh-keygen")
        .arg("-y")
        .args(["-P", ""])
        .arg("-f")
        .arg(private_path)
        .output();
    match output {
        Ok(o) => !o.status.success(),
        Err(_) => false,
    }
}

/// Extract the Drunken Bishop ASCII block from `ssh-keygen -lv` stdout.
/// Returns the 11 art lines joined with `\n`, or an empty string when the
/// expected `+--...--+` border + 9 content rows + closing border are not
/// all present. The filter matches any line that opens AND closes with
/// either `+` (border) or `|` (content), which is robust to header
/// variations across OpenSSH versions.
fn parse_bishop_block(stdout: &str) -> String {
    let art_lines: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            let t = l.trim_end();
            (t.starts_with('+') && t.ends_with('+')) || (t.starts_with('|') && t.ends_with('|'))
        })
        .collect();
    if art_lines.len() == 11 {
        art_lines.join("\n")
    } else {
        String::new()
    }
}

/// Check if a directory entry looks like a public key file.
fn is_public_key_file(entry: &std::fs::DirEntry) -> bool {
    let name = entry.file_name();
    let name = name.to_string_lossy();

    // Must end in .pub
    if !name.ends_with(".pub") {
        return false;
    }

    // Skip known non-key files
    let skip = ["authorized_keys.pub", "known_hosts.pub"];
    if skip.contains(&name.as_ref()) {
        return false;
    }

    // Use std::fs::metadata, not DirEntry::file_type or DirEntry::metadata:
    // both of those use lstat and report the symlink itself (is_file = false).
    // std::fs::metadata uses stat, follows the chain, and reports the target.
    std::fs::metadata(entry.path())
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// What `ssh-keygen -lv -E sha256` reports about a public key file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicKeyFacts {
    bits: String,
    fingerprint: String,
    comment: String,
    key_type: String,
    bishop_art: String,
}

/// Read the public key facts in-process. Formats the in-process reader
/// does not cover go through `ssh-keygen -lv` instead.
fn public_key_facts(pub_path: &Path) -> Option<PublicKeyFacts> {
    if let Some(facts) = native::inspect_public_key(pub_path) {
        return Some(facts);
    }
    debug!(
        "[purple] key scan: ssh-keygen fallback for {}",
        pub_path.display()
    );
    keygen_public_key_facts(pub_path)
}

fn keygen_public_key_facts(pub_path: &Path) -> Option<PublicKeyFacts> {
    let output = Command::new("ssh-keygen")
        .arg("-lv")
        .arg("-f")
        .arg(pub_path)
        .args(["-E", "sha256"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?.trim();

    // Format: "<bits> <fingerprint> <comment> (<type>)"
    let (bits, fingerprint, comment, key_type) = parse_keygen_output(first_line)?;
    Some(PublicKeyFacts {
        bits,
        fingerprint,
        comment,
        key_type,
        bishop_art: parse_bishop_block(&stdout),
    })
}

/// Read key metadata (fingerprint + Drunken Bishop) and cross-reference
/// with hosts, agent state and on-disk encryption.
fn read_key_info(
    ssh_dir: &Path,
    pub_path: &Path,
    home: Option<&Path>,
    hosts: &[HostEntry],
    agent_fingerprints: &HashSet<String>,
) -> Option<SshKeyInfo> {
    let PublicKeyFacts {
        bits,
        fingerprint,
        comment,
        key_type,
        bishop_art,
    } = public_key_facts(pub_path)?;

    // Derive the private key name (strip .pub)
    let pub_name = pub_path.file_name()?.to_string_lossy();
    let name = pub_name
        .strip_suffix(".pub")
        .unwrap_or(&pub_name)
        .to_string();

    // Private key path (without .pub extension)
    let private_path = ssh_dir.join(&name);

    // Display path: use ~ if ssh_dir is under home
    let display_path = match home {
        Some(home) if ssh_dir.starts_with(home) => {
            let relative = ssh_dir.strip_prefix(home).unwrap();
            format!("~/{}/{}", relative.display(), name)
        }
        _ => private_path.display().to_string(),
    };

    // Find hosts that reference this key
    let linked_hosts = find_linked_hosts(&private_path, &display_path, hosts);

    let is_certificate = detect_certificate(&pub_name, &key_type);

    // Probe encryption status via empty-passphrase pubkey derivation.
    // Cert files have no encrypted-private-key counterpart, so skip.
    let encrypted = if is_certificate {
        false
    } else {
        private_key_encrypted(&private_path)
    };

    // Agent match by fingerprint (already SHA256-prefixed in both sides).
    let agent_loaded = agent_fingerprints.contains(&fingerprint);

    let strength_score = strength_score_for(&key_type, &bits, encrypted);

    let mtime_ts = file_mtime_ts(&private_path, pub_path);

    Some(SshKeyInfo {
        name,
        display_path,
        key_type,
        bits,
        fingerprint,
        comment,
        linked_hosts,
        bishop_art,
        strength_score,
        encrypted,
        agent_loaded,
        is_certificate,
        mtime_ts,
    })
}

/// File mtime of the private key, falling back to the pub key when the
/// private file is missing or unreadable. Returns seconds since UNIX
/// epoch. mtime is the most portable proxy for "key created" on Unix;
/// btime exists on some filesystems but Rust's stable std cannot read
/// it portably without a third-party crate.
fn file_mtime_ts(private_path: &Path, pub_path: &Path) -> Option<u64> {
    let from = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    };
    from(private_path).or_else(|| from(pub_path))
}

/// Detect whether a `.pub` file holds an OpenSSH user certificate.
///
/// Two paths trigger detection:
/// 1. Filename ends in `-cert.pub` (the convention `ssh-keygen -s` emits).
/// 2. `ssh-keygen -l` reported a cert variant in the type column, e.g.
///    `ED25519-CERT-V01@openssh.com`. This branch catches Vault SSH certs
///    and other signed pub keys the user renamed away from `-cert.pub`.
fn detect_certificate(pub_name: &str, key_type: &str) -> bool {
    pub_name.ends_with("-cert.pub") || key_type.to_ascii_lowercase().contains("-cert")
}

/// Parse ssh-keygen -lf output line into (bits, fingerprint, comment, type).
fn parse_keygen_output(line: &str) -> Option<(String, String, String, String)> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return None;
    }

    let bits = parts[0].to_string();
    let fingerprint = parts[1].to_string();

    // The rest is "<comment> (<type>)". Extract type from the end.
    let rest = parts[2];
    let (comment, key_type) = if let Some(paren_start) = rest.rfind('(') {
        let comment = rest[..paren_start].trim().to_string();
        let key_type = rest[paren_start + 1..].trim_end_matches(')').to_string();
        (comment, key_type)
    } else {
        (rest.to_string(), String::new())
    };

    Some((bits, fingerprint, comment, key_type))
}

/// Find host aliases that reference a given key path via IdentityFile.
/// Hosts without an explicit IdentityFile are linked to all keys (SSH tries them all).
fn find_linked_hosts(full_path: &Path, display_path: &str, hosts: &[HostEntry]) -> Vec<String> {
    // Only count explicit IdentityFile matches. SSH technically falls
    // back to trying every available key when no IdentityFile is set,
    // but rendering that as "this host is linked to every key" pollutes
    // every key's Linked Hosts grid with the same untargeted hosts.
    hosts
        .iter()
        .filter(|h| {
            if h.identity_file.is_empty() {
                return false;
            }
            h.identity_file == display_path || Path::new(&h.identity_file) == full_path
        })
        .map(|h| h.alias.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keygen_output_ed25519() {
        let line = "256 SHA256:abcdef1234567890 user@host (ED25519)";
        let (bits, fp, comment, key_type) = parse_keygen_output(line).unwrap();
        assert_eq!(bits, "256");
        assert_eq!(fp, "SHA256:abcdef1234567890");
        assert_eq!(comment, "user@host");
        assert_eq!(key_type, "ED25519");
    }

    #[test]
    fn test_parse_keygen_output_rsa() {
        let line = "4096 SHA256:xyz9876543210 deploy@prod.example.com (RSA)";
        let (bits, fp, comment, key_type) = parse_keygen_output(line).unwrap();
        assert_eq!(bits, "4096");
        assert_eq!(fp, "SHA256:xyz9876543210");
        assert_eq!(comment, "deploy@prod.example.com");
        assert_eq!(key_type, "RSA");
    }

    #[test]
    fn test_parse_keygen_output_no_comment() {
        let line = "256 SHA256:fingerprint (ED25519)";
        let (bits, fp, comment, key_type) = parse_keygen_output(line).unwrap();
        assert_eq!(bits, "256");
        assert_eq!(fp, "SHA256:fingerprint");
        assert_eq!(comment, "");
        assert_eq!(key_type, "ED25519");
    }

    #[test]
    fn test_parse_keygen_output_comment_with_spaces() {
        let line = "256 SHA256:fingerprint eko@MacBook Pro (ED25519)";
        let (bits, fp, comment, key_type) = parse_keygen_output(line).unwrap();
        assert_eq!(bits, "256");
        assert_eq!(fp, "SHA256:fingerprint");
        assert_eq!(comment, "eko@MacBook Pro");
        assert_eq!(key_type, "ED25519");
    }

    #[test]
    fn test_parse_keygen_output_no_type_parens() {
        let line = "256 SHA256:fingerprint user@host";
        let (bits, fp, comment, key_type) = parse_keygen_output(line).unwrap();
        assert_eq!(bits, "256");
        assert_eq!(fp, "SHA256:fingerprint");
        assert_eq!(comment, "user@host");
        assert_eq!(key_type, "");
    }

    #[test]
    fn test_parse_keygen_output_too_short() {
        assert!(parse_keygen_output("256 SHA256:fp").is_none());
        assert!(parse_keygen_output("").is_none());
    }

    #[test]
    fn test_find_linked_hosts_display_path() {
        let hosts = vec![
            HostEntry {
                alias: "prod".to_string(),
                identity_file: "~/.ssh/id_ed25519".to_string(),
                ..Default::default()
            },
            HostEntry {
                alias: "staging".to_string(),
                identity_file: "~/.ssh/other_key".to_string(),
                ..Default::default()
            },
        ];
        let linked = find_linked_hosts(
            Path::new("/home/user/.ssh/id_ed25519"),
            "~/.ssh/id_ed25519",
            &hosts,
        );
        assert_eq!(linked, vec!["prod"]);
    }

    #[test]
    fn test_find_linked_hosts_full_path() {
        let hosts = vec![HostEntry {
            alias: "server".to_string(),
            identity_file: "/home/user/.ssh/deploy_key".to_string(),
            ..Default::default()
        }];
        let linked = find_linked_hosts(
            Path::new("/home/user/.ssh/deploy_key"),
            "~/.ssh/deploy_key",
            &hosts,
        );
        assert_eq!(linked, vec!["server"]);
    }

    #[test]
    fn test_find_linked_hosts_no_identity_file_does_not_link() {
        // Hosts without an explicit IdentityFile are excluded so the
        // Linked Hosts grid stays accurate per key instead of showing
        // every untargeted host under every key.
        let hosts = vec![HostEntry {
            alias: "server".to_string(),
            identity_file: String::new(),
            ..Default::default()
        }];
        let linked =
            find_linked_hosts(Path::new("/home/user/.ssh/id_rsa"), "~/.ssh/id_rsa", &hosts);
        assert!(linked.is_empty());
    }

    #[test]
    fn test_find_linked_hosts_wrong_identity_file() {
        let hosts = vec![HostEntry {
            alias: "server".to_string(),
            identity_file: "~/.ssh/other_key".to_string(),
            ..Default::default()
        }];
        let linked =
            find_linked_hosts(Path::new("/home/user/.ssh/id_rsa"), "~/.ssh/id_rsa", &hosts);
        assert!(linked.is_empty());
    }

    fn sample_key() -> SshKeyInfo {
        SshKeyInfo {
            name: "id_ed25519".to_string(),
            display_path: "~/.ssh/id_ed25519".to_string(),
            key_type: "ED25519".to_string(),
            bits: "256".to_string(),
            fingerprint: "SHA256:8x2k7HhPqQfvN5jJrUvWxTsXmnQ4LpBkEoYzNcAdGhI".to_string(),
            comment: "eric@MacBook".to_string(),
            linked_hosts: Vec::new(),
            bishop_art: String::new(),
            strength_score: 95,
            encrypted: true,
            agent_loaded: true,
            is_certificate: false,
            mtime_ts: None,
        }
    }

    #[test]
    fn test_type_display() {
        let key = sample_key();
        assert_eq!(key.type_display(), "ED25519 256");

        let key2 = SshKeyInfo {
            bits: String::new(),
            ..key
        };
        assert_eq!(key2.type_display(), "ED25519");
    }

    #[test]
    fn detect_certificate_via_filename_suffix() {
        assert!(detect_certificate("id_ed25519-cert.pub", "ED25519"));
    }

    #[test]
    fn detect_certificate_via_key_type_full_oid() {
        // ssh-keygen emits this form when a signed pub key is fed in even
        // though the filename omits the conventional `-cert.pub` suffix.
        assert!(detect_certificate(
            "id_ed25519-vault.pub",
            "ED25519-CERT-V01@openssh.com"
        ));
    }

    #[test]
    fn detect_certificate_via_key_type_short() {
        assert!(detect_certificate(
            "id_ed25519-breakglass.pub",
            "ED25519-CERT"
        ));
    }

    #[test]
    fn detect_certificate_rejects_plain_key() {
        assert!(!detect_certificate("id_ed25519.pub", "ED25519"));
    }

    #[test]
    fn detect_certificate_rejects_unrelated_dash_cert_in_name() {
        // A filename containing `cert` but not the `-cert.pub` suffix and a
        // non-cert key_type must not be flagged as a certificate.
        assert!(!detect_certificate("my-cert-backup.pub", "RSA"));
    }

    #[test]
    fn drunken_bishop_matches_openssh_canonical_17x9() {
        // Fingerprint generated with `ssh-keygen -t ed25519`; the bishop
        // block below is the exact `ssh-keygen -lv -E sha256` output for
        // that key. Locks the algorithm against OpenSSH's reference impl.
        let fp = decode_fingerprint("SHA256:1LayGj+CVIvJfOnQqADAT52DoJHhSa30feF/23wbRuE")
            .expect("decode fingerprint");
        let grid = drunken_bishop_grid(&fp, 17, 9);
        let rendered: Vec<String> = grid
            .iter()
            .map(|row| row.iter().map(|&c| bishop_char(c)).collect())
            .collect();
        assert_eq!(
            rendered,
            vec![
                "+=o o .          ",
                "*+.+ + . .       ",
                "+o= . o o o    . ",
                ".. o ..+ . .  . .",
                ".  o *.oS .    E ",
                ".   O =  + .  .  ",
                " . o =. . . +  o ",
                "  . . o+.  . o...",
                "      ....    ...",
            ]
        );
    }

    #[test]
    fn drunken_bishop_scales_to_larger_grid() {
        // At a larger grid the walk still starts at center and produces a
        // sparser pattern (same step count over more cells). Just sanity-
        // check that the dimensions match the request and that the center
        // tile carries the S marker as expected.
        let fp = decode_fingerprint("SHA256:1LayGj+CVIvJfOnQqADAT52DoJHhSa30feF/23wbRuE")
            .expect("decode fingerprint");
        let grid = drunken_bishop_grid(&fp, 25, 13);
        assert_eq!(grid.len(), 13);
        assert!(grid.iter().all(|row| row.len() == 25));
        assert_eq!(grid[6][12], BISHOP_S_INDEX);
    }

    #[test]
    fn decode_fingerprint_rejects_other_hash_prefixes() {
        assert!(decode_fingerprint("MD5:abcd").is_none());
        assert!(decode_fingerprint("plain-text").is_none());
    }

    #[test]
    fn test_bishop_lines_empty() {
        let key = SshKeyInfo {
            bishop_art: String::new(),
            ..sample_key()
        };
        assert!(key.bishop_lines().is_empty());
    }

    #[test]
    fn test_bishop_lines_split() {
        let key = SshKeyInfo {
            bishop_art: "+--[ED25519 256]--+\n|       .o*+      |\n+----[SHA256]-----+".to_string(),
            ..sample_key()
        };
        assert_eq!(key.bishop_lines().len(), 3);
        assert_eq!(key.bishop_lines()[1], "|       .o*+      |");
    }

    #[test]
    fn test_parse_agent_list_two_keys() {
        let stdout = "256 SHA256:abc1 eric@host (ED25519)\n4096 SHA256:def2 work@laptop (RSA)\n";
        let set = parse_agent_list(stdout);
        assert_eq!(set.len(), 2);
        assert!(set.contains("SHA256:abc1"));
        assert!(set.contains("SHA256:def2"));
    }

    #[test]
    fn test_parse_agent_list_empty_agent() {
        let stdout = "The agent has no identities.\n";
        let set = parse_agent_list(stdout);
        assert!(set.is_empty());
    }

    #[test]
    fn test_parse_agent_list_banner_skipped() {
        let stdout = "Could not open a connection to your authentication agent.\n";
        let set = parse_agent_list(stdout);
        assert!(set.is_empty());
    }

    #[test]
    fn test_strength_score_ed25519() {
        assert_eq!(strength_score_for("ED25519", "256", true), 95);
        assert_eq!(strength_score_for("ED25519", "256", false), 80);
    }

    #[test]
    fn test_strength_score_sk_ed25519() {
        assert_eq!(strength_score_for("sk-ED25519", "256", true), 100);
        assert_eq!(strength_score_for("sk-ED25519", "256", false), 85);
    }

    #[test]
    fn test_strength_score_rsa_buckets() {
        assert_eq!(strength_score_for("RSA", "1024", true), 20);
        assert_eq!(strength_score_for("RSA", "2048", true), 60);
        assert_eq!(strength_score_for("RSA", "3072", true), 80);
        assert_eq!(strength_score_for("RSA", "4096", true), 85);
        assert_eq!(strength_score_for("RSA", "8192", true), 85);
    }

    #[test]
    fn test_strength_score_dsa_is_low() {
        assert_eq!(strength_score_for("DSA", "1024", true), 10);
        assert_eq!(strength_score_for("DSA", "1024", false), 0);
    }

    #[test]
    fn test_strength_score_ecdsa_buckets() {
        assert_eq!(strength_score_for("ECDSA", "256", true), 75);
        assert_eq!(strength_score_for("ECDSA", "384", true), 85);
        assert_eq!(strength_score_for("ECDSA", "521", true), 90);
    }

    #[test]
    fn test_strength_score_unknown_type() {
        assert_eq!(strength_score_for("WEIRD", "256", true), 55);
        assert_eq!(strength_score_for("", "0", false), 40);
    }

    #[test]
    fn test_parse_bishop_block_typical_output() {
        let stdout = "\
256 SHA256:abc eric@host (ED25519)
+--[ED25519 256]--+
|                 |
|                 |
|      . .  . ... |
|       o o..ooo.o|
|      . S =.oo+==|
|     . o   B +E*B|
|      . . O =.=.+|
|     ..  = B o.oo|
|      .oo.+.=o.. |
+----[SHA256]-----+
";
        let art = parse_bishop_block(stdout);
        assert_eq!(art.lines().count(), 11);
        assert!(art.starts_with("+--[ED25519 256]--+"));
        assert!(art.ends_with("+----[SHA256]-----+"));
    }

    #[test]
    fn test_parse_bishop_block_missing_returns_empty() {
        let stdout = "256 SHA256:abc eric@host (ED25519)\n";
        assert!(parse_bishop_block(stdout).is_empty());
    }

    #[test]
    fn test_parse_bishop_block_truncated_returns_empty() {
        let stdout = "+--[ED25519 256]--+\n|   |\n+--+\n";
        assert!(parse_bishop_block(stdout).is_empty());
    }

    fn search_corpus() -> Vec<SshKeyInfo> {
        vec![
            SshKeyInfo {
                name: "id_ed25519".into(),
                comment: "eric@mac".into(),
                ..sample_key()
            },
            SshKeyInfo {
                name: "yubikey_work".into(),
                comment: "yubi@work".into(),
                ..sample_key()
            },
            SshKeyInfo {
                name: "customer-x".into(),
                comment: "eric@customer".into(),
                ..sample_key()
            },
        ]
    }

    #[test]
    fn filtered_key_indices_none_returns_all() {
        let keys = search_corpus();
        let idx = filtered_key_indices(&keys, None);
        assert_eq!(idx, vec![0, 1, 2]);
    }

    #[test]
    fn filtered_key_indices_empty_returns_all() {
        let keys = search_corpus();
        let idx = filtered_key_indices(&keys, Some(""));
        assert_eq!(idx, vec![0, 1, 2]);
    }

    #[test]
    fn filtered_key_indices_matches_name() {
        let keys = search_corpus();
        let idx = filtered_key_indices(&keys, Some("yubi"));
        assert_eq!(idx, vec![1]);
    }

    #[test]
    fn filtered_key_indices_matches_comment() {
        let keys = search_corpus();
        let idx = filtered_key_indices(&keys, Some("eric"));
        assert_eq!(idx, vec![0, 2]);
    }

    #[test]
    fn filtered_key_indices_case_insensitive() {
        let keys = search_corpus();
        let idx = filtered_key_indices(&keys, Some("ERIC"));
        assert_eq!(idx, vec![0, 2]);
    }

    #[test]
    fn filtered_key_indices_no_match() {
        let keys = search_corpus();
        let idx = filtered_key_indices(&keys, Some("nonexistent"));
        assert!(idx.is_empty());
    }

    #[test]
    fn resolve_selection_unfiltered_is_identity() {
        let keys = search_corpus();
        assert_eq!(resolve_selection(&keys, None, 0), Some(0));
        assert_eq!(resolve_selection(&keys, None, 2), Some(2));
        assert_eq!(resolve_selection(&keys, None, 99), None);
    }

    #[test]
    fn resolve_selection_filtered_maps_back_to_underlying() {
        let keys = search_corpus();
        // "eric" matches indices 0 (id_ed25519, eric@mac) and 2 (customer-x, eric@customer).
        assert_eq!(resolve_selection(&keys, Some("eric"), 0), Some(0));
        assert_eq!(resolve_selection(&keys, Some("eric"), 1), Some(2));
        assert_eq!(resolve_selection(&keys, Some("eric"), 2), None);
    }

    #[test]
    fn resolve_selection_no_match_returns_none() {
        let keys = search_corpus();
        assert_eq!(resolve_selection(&keys, Some("xyzzy"), 0), None);
    }

    #[cfg(unix)]
    fn read_only_entry(dir: &Path, name: &str) -> std::fs::DirEntry {
        std::fs::read_dir(dir)
            .expect("read_dir")
            .filter_map(Result::ok)
            .find(|e| e.file_name() == name)
            .expect("entry not found")
    }

    #[cfg(unix)]
    #[test]
    fn test_is_public_key_file_accepts_regular_pub_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_ed25519.pub");
        std::fs::write(&path, b"ssh-ed25519 AAAA").unwrap();
        let entry = read_only_entry(dir.path(), "id_ed25519.pub");
        assert!(is_public_key_file(&entry));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_public_key_file_accepts_symlink_to_regular_pub_file() {
        use std::os::unix::fs::symlink;
        let target_dir = tempfile::tempdir().unwrap();
        let link_dir = tempfile::tempdir().unwrap();
        let target = target_dir.path().join("id_ed25519.pub");
        std::fs::write(&target, b"ssh-ed25519 AAAA").unwrap();
        let link = link_dir.path().join("id_ed25519.pub");
        symlink(&target, &link).unwrap();
        let entry = read_only_entry(link_dir.path(), "id_ed25519.pub");
        assert!(is_public_key_file(&entry));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_public_key_file_rejects_broken_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("id_ed25519.pub");
        symlink(dir.path().join("does_not_exist.pub"), &link).unwrap();
        let entry = read_only_entry(dir.path(), "id_ed25519.pub");
        assert!(!is_public_key_file(&entry));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_public_key_file_rejects_symlink_to_directory() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("realdir");
        std::fs::create_dir(&real_dir).unwrap();
        let link = dir.path().join("id_ed25519.pub");
        symlink(&real_dir, &link).unwrap();
        let entry = read_only_entry(dir.path(), "id_ed25519.pub");
        assert!(!is_public_key_file(&entry));
    }

    // --- file_mtime_ts coverage (added during code review) ---

    #[test]
    fn file_created_ts_returns_private_key_mtime_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let priv_path = dir.path().join("id_ed25519");
        let pub_path = dir.path().join("id_ed25519.pub");
        std::fs::write(&priv_path, b"PRIVATE").unwrap();
        std::fs::write(&pub_path, b"ssh-ed25519 AAAA").unwrap();
        let ts = file_mtime_ts(&priv_path, &pub_path).expect("private mtime");
        assert!(ts > 0);
    }

    #[test]
    fn file_created_ts_falls_back_to_pubkey_when_private_missing() {
        let dir = tempfile::tempdir().unwrap();
        let priv_path = dir.path().join("does_not_exist");
        let pub_path = dir.path().join("id_ed25519.pub");
        std::fs::write(&pub_path, b"ssh-ed25519 AAAA").unwrap();
        let ts = file_mtime_ts(&priv_path, &pub_path).expect("pubkey mtime");
        assert!(ts > 0);
    }

    #[test]
    fn file_created_ts_returns_none_when_both_missing() {
        let dir = tempfile::tempdir().unwrap();
        let priv_path = dir.path().join("nope_priv");
        let pub_path = dir.path().join("nope_pub.pub");
        assert!(file_mtime_ts(&priv_path, &pub_path).is_none());
    }
}
