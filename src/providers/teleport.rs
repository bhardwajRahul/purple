use std::collections::HashMap;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use log::debug;
use serde::Deserialize;

use super::{Provider, ProviderError, ProviderHost, ProviderMetadata};
use crate::runtime::env::Env;

/// Teleport nodes, read from the local `tsh` CLI. There is no API token:
/// `tsh login` holds the session and every sync goes through `tsh status`,
/// `tsh config` and `tsh ls`. Each host gets the `ProxyCommand` and key paths
/// straight from `tsh config`, so plain `ssh` connects through the proxy.
pub struct Teleport {
    /// The provider's IdentityFile setting. When set it wins over the
    /// tsh-managed key that `tsh config` names.
    pub identity_file: String,
}

/// Upper bound on one `tsh` invocation. Generous, since `tsh config` and
/// `tsh ls` both talk to the proxy.
const TSH_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the wait loop checks for cancel and exit.
const TSH_POLL: Duration = Duration::from_millis(100);
/// Port the Teleport SSH service listens on when a node reports no address.
const DEFAULT_NODE_PORT: u16 = 3022;

// =========================================================================
// `tsh status --client --format=json`
// =========================================================================

#[derive(Deserialize, Default)]
struct TshStatus {
    #[serde(default)]
    active: Option<TshProfile>,
}

#[derive(Deserialize, Default, Clone)]
struct TshProfile {
    #[serde(default)]
    profile_url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    cluster: String,
    #[serde(default)]
    logins: Vec<String>,
}

// =========================================================================
// `tsh ls --format=json`
// =========================================================================

#[derive(Deserialize)]
struct TshNode {
    #[serde(default)]
    sub_kind: String,
    #[serde(default)]
    metadata: TshNodeMeta,
    #[serde(default)]
    spec: TshNodeSpec,
}

#[derive(Deserialize, Default)]
struct TshNodeMeta {
    #[serde(default)]
    name: String,
    #[serde(default)]
    labels: HashMap<String, String>,
}

#[derive(Deserialize, Default)]
struct TshNodeSpec {
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    addr: String,
    #[serde(default)]
    cmd_labels: HashMap<String, TshCmdLabel>,
}

#[derive(Deserialize, Default)]
struct TshCmdLabel {
    #[serde(default)]
    result: String,
}

// =========================================================================
// `tsh config`
// =========================================================================

/// The two `Host` blocks `tsh config` prints per cluster, folded into one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TshClusterConfig {
    cluster: String,
    /// Directives from the common block (`Host *.<cluster> <proxy>`).
    common: Vec<(String, String)>,
    /// `Port` from the proxied block (`Host *.<cluster> !<proxy>`).
    port: Option<u16>,
    /// `ProxyCommand` from the proxied block, verbatim.
    proxy_command: Option<String>,
}

/// Parse `tsh config` output into one entry per cluster.
///
/// The output has, per cluster, a common block `Host *.<cluster> <proxy>`
/// with key paths and a proxied block `Host *.<cluster> !<proxy>` with
/// `Port` and `ProxyCommand`. Single-token values lose their surrounding
/// quotes; `ProxyCommand` keeps the rest of the line as written.
fn parse_tsh_config(text: &str) -> Vec<TshClusterConfig> {
    let mut clusters: Vec<TshClusterConfig> = Vec::new();
    // (cluster index, is the proxied block)
    let mut current: Option<(usize, bool)> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = match split_directive(line) {
            Some(kv) => kv,
            None => continue,
        };
        if key.eq_ignore_ascii_case("host") {
            let mut tokens = rest.split_whitespace();
            let pattern = tokens.next().unwrap_or_default();
            let second = tokens.next().unwrap_or_default();
            let Some(cluster) = pattern.strip_prefix("*.") else {
                current = None;
                continue;
            };
            let proxied = second.starts_with('!');
            let idx = match clusters.iter().position(|c| c.cluster == cluster) {
                Some(i) => i,
                None => {
                    clusters.push(TshClusterConfig {
                        cluster: cluster.to_string(),
                        ..Default::default()
                    });
                    clusters.len() - 1
                }
            };
            current = Some((idx, proxied));
            continue;
        }
        let Some((idx, proxied)) = current else {
            continue;
        };
        let entry = &mut clusters[idx];
        if key.eq_ignore_ascii_case("proxycommand") {
            entry.proxy_command = Some(rest.to_string());
        } else if key.eq_ignore_ascii_case("port") {
            entry.port = unquote(rest).parse().ok();
        } else if !proxied {
            entry
                .common
                .push((canonical_key(key), unquote(rest).to_string()));
        }
    }
    clusters
}

/// Split `Key value` or `Key=value` into the key and the untrimmed rest.
fn split_directive(line: &str) -> Option<(&str, &str)> {
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let key = &line[..end];
    if key.is_empty() {
        return None;
    }
    let rest = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    Some((key, rest.trim()))
}

/// Drop one pair of surrounding double quotes.
fn unquote(value: &str) -> &str {
    let v = value.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

/// Spell a directive the way ssh_config(5) does, so the written line reads
/// as OpenSSH prints it and later syncs compare like with like.
fn canonical_key(key: &str) -> String {
    const KNOWN: &[&str] = &[
        "UserKnownHostsFile",
        "IdentityFile",
        "CertificateFile",
        "HostKeyAlgorithms",
        "User",
        "Port",
        "ProxyCommand",
    ];
    KNOWN
        .iter()
        .find(|k| k.eq_ignore_ascii_case(key))
        .map(|k| (*k).to_string())
        .unwrap_or_else(|| key.to_string())
}

// =========================================================================
// Running tsh
// =========================================================================

/// Locate `tsh` on the resolved PATH.
fn find_tsh(env: &Env) -> Result<PathBuf, ProviderError> {
    let path = env.var("PATH").unwrap_or_default();
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let candidate = PathBuf::from(dir).join("tsh");
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(ProviderError::Execute(
        "Teleport CLI (tsh) not found. Install it from https://goteleport.com/download or add it to PATH."
            .to_string(),
    ))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Output of one `tsh` run.
struct TshOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Run `tsh <args>` with stdin closed, a timeout and cancel support. A
/// closed stdin keeps an expired session from waiting on a login prompt.
/// The child gets its own process group so cancel and timeout also take
/// down anything tsh spawned; otherwise a grandchild holding the pipes
/// keeps the readers alive.
fn run_tsh(
    env: &Env,
    tsh: &std::path::Path,
    args: &[&str],
    cancel: &AtomicBool,
) -> Result<TshOutput, ProviderError> {
    let mut cmd = env.command(&tsh.to_string_lossy());
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ProviderError::Execute(format!("Failed to run tsh: {}", e)))?;

    // Drain both pipes on their own threads so a chatty tsh never fills a
    // pipe buffer and blocks before we get to wait().
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut p) = stdout_pipe {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut p) = stderr_pipe {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let label = args.first().copied().unwrap_or("");
    let outcome: Result<bool, ProviderError> = loop {
        if cancel.load(Ordering::Relaxed) {
            kill_process_group(&mut child);
            break Err(ProviderError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Ok(None) => {
                if start.elapsed() >= TSH_TIMEOUT {
                    kill_process_group(&mut child);
                    break Err(ProviderError::Execute(format!(
                        "tsh {label} timed out after {}s.",
                        TSH_TIMEOUT.as_secs()
                    )));
                }
                std::thread::sleep(TSH_POLL);
            }
            Err(e) => {
                kill_process_group(&mut child);
                break Err(ProviderError::Execute(format!(
                    "Failed to wait for tsh {label}: {}",
                    e
                )));
            }
        }
    };

    // Join the readers in every case so the threads never outlive the call.
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    let success = outcome?;
    Ok(TshOutput {
        success,
        stdout,
        stderr,
    })
}

/// Kill the child and everything in its process group, then reap it.
fn kill_process_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as libc::pid_t;
        if pgid > 0 {
            // SAFETY: plain syscall on a pid we own; a stale pgid only yields ESRCH.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Trim tsh's stderr down to something a toast can show.
fn stderr_summary(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

// =========================================================================
// Provider impl
// =========================================================================

impl Provider for Teleport {
    fn name(&self) -> &str {
        "teleport"
    }

    fn short_label(&self) -> &str {
        "tp"
    }

    fn fetch_hosts_cancellable(
        &self,
        _token: &str,
        cancel: &AtomicBool,
        env: &Env,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        let tsh = find_tsh(env)?;

        // Session check first: `--client` needs no network and fails fast when
        // the user is logged out or the cert expired, so we never sit behind
        // a login prompt.
        let status = run_tsh(env, &tsh, &["status", "--client", "--format=json"], cancel)?;
        if !status.success {
            let reason = stderr_summary(&status.stderr);
            let reason = if reason.is_empty() {
                "not logged in".to_string()
            } else {
                reason.trim_end_matches('.').to_string()
            };
            return Err(ProviderError::Execute(format!(
                "Teleport: {reason}. Run `tsh login` and sync again."
            )));
        }
        let profile = serde_json::from_str::<TshStatus>(&status.stdout)
            .map_err(|e| ProviderError::Parse(format!("Failed to parse tsh status: {}", e)))?
            .active
            .filter(|p| !p.cluster.is_empty())
            .ok_or_else(|| {
                ProviderError::Execute(
                    "Teleport: no active profile. Run `tsh login` and sync again.".to_string(),
                )
            })?;
        debug!(
            "[external] teleport: active profile user={} cluster={} proxy={} logins={}",
            profile.username,
            profile.cluster,
            profile.profile_url,
            profile.logins.len()
        );

        let config = run_tsh(env, &tsh, &["config"], cancel)?;
        if !config.success {
            return Err(ProviderError::Execute(format!(
                "tsh config failed: {}",
                stderr_summary(&config.stderr)
            )));
        }
        let cluster_config = parse_tsh_config(&config.stdout)
            .into_iter()
            .find(|c| c.cluster == profile.cluster)
            .ok_or_else(|| {
                ProviderError::Parse(format!(
                    "tsh config has no block for cluster {}",
                    profile.cluster
                ))
            })?;
        if cluster_config.proxy_command.is_none() {
            return Err(ProviderError::Parse(format!(
                "tsh config has no ProxyCommand for cluster {}",
                profile.cluster
            )));
        }

        let listing = run_tsh(env, &tsh, &["ls", "--format=json"], cancel)?;
        if !listing.success {
            return Err(ProviderError::Execute(format!(
                "tsh ls failed: {}",
                stderr_summary(&listing.stderr)
            )));
        }
        let nodes: Vec<TshNode> = serde_json::from_str(&listing.stdout)
            .map_err(|e| ProviderError::Parse(format!("Failed to parse tsh ls output: {}", e)))?;
        let hosts = self.hosts_from(nodes, &cluster_config);
        debug!(
            "[external] teleport: {} host(s) from cluster {}",
            hosts.len(),
            cluster_config.cluster
        );
        Ok(hosts)
    }
}

impl Teleport {
    /// Map `tsh ls` nodes onto provider hosts carrying the `tsh config`
    /// transport directives. Nodes without a hostname are skipped.
    fn hosts_from(&self, nodes: Vec<TshNode>, cfg: &TshClusterConfig) -> Vec<ProviderHost> {
        let mut hosts = Vec::new();
        for node in nodes {
            if node.spec.hostname.is_empty() || node.metadata.name.is_empty() {
                continue;
            }

            let mut tags: Vec<String> = node
                .metadata
                .labels
                .iter()
                .map(|(k, v)| label_tag(k, v))
                .chain(
                    node.spec
                        .cmd_labels
                        .iter()
                        .map(|(k, v)| label_tag(k, &v.result)),
                )
                .filter(|t| !t.is_empty())
                .collect();
            tags.sort();
            tags.dedup();

            let mut metadata = ProviderMetadata::new();
            metadata.push("cluster", cfg.cluster.clone());
            if !node.spec.addr.is_empty() {
                metadata.push("address", node.spec.addr.clone());
            }
            if !node.sub_kind.is_empty() {
                metadata.push("type", node.sub_kind.clone());
            }

            let port = addr_port(&node.spec.addr)
                .or(cfg.port)
                .unwrap_or(DEFAULT_NODE_PORT);

            let mut directives: Vec<(String, String)> = Vec::new();
            if let Some(cmd) = &cfg.proxy_command {
                directives.push(("ProxyCommand".to_string(), cmd.clone()));
            }
            for (key, value) in &cfg.common {
                if key.eq_ignore_ascii_case("User") {
                    continue;
                }
                if key.eq_ignore_ascii_case("IdentityFile") && !self.identity_file.is_empty() {
                    continue;
                }
                directives.push((key.clone(), value.clone()));
            }

            hosts.push(ProviderHost {
                server_id: node.metadata.name,
                name: node.spec.hostname.clone(),
                ip: node.spec.hostname,
                tags,
                metadata: metadata.finish(),
                port: Some(port),
                directives,
            });
        }
        hosts
    }
}

/// `key:value` or just `key` when the value is empty.
fn label_tag(key: &str, value: &str) -> String {
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() {
        String::new()
    } else if value.is_empty() {
        key.to_string()
    } else {
        format!("{}:{}", key, value)
    }
}

/// The port of a `host:port` node address, if it has one.
fn addr_port(addr: &str) -> Option<u16> {
    addr.rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .filter(|p| *p != 0)
}

#[cfg(test)]
#[path = "teleport_tests.rs"]
mod tests;
