use super::*;
use std::sync::atomic::AtomicBool;

const SINGLE_CLUSTER_CONFIG: &str = r#"# Begin generated Teleport configuration for proxy.example.com by tsh

# Common flags for all example.com hosts
Host *.example.com proxy.example.com
    UserKnownHostsFile "/home/alice/.tsh/known_hosts"
    IdentityFile "/home/alice/.tsh/keys/example.com/bob"
    CertificateFile "/home/alice/.tsh/keys/example.com/bob-ssh/example.com-cert.pub"

# Flags for all example.com hosts except the proxy
Host *.example.com !proxy.example.com
    Port 3022
    ProxyCommand "/tmp/tsh" proxy ssh --cluster=example.com --proxy=proxy.example.com:443 %r@%h:%p

# End generated Teleport configuration
"#;

const MULTI_CLUSTER_CONFIG: &str = r#"# Begin generated Teleport configuration for proxy.example.com by tsh

# Common flags for all root hosts
Host *.root proxy.example.com
    UserKnownHostsFile "/home/alice/.tsh/known_hosts"
    IdentityFile "/home/alice/.tsh/keys/example.com/bob"
    CertificateFile "/home/alice/.tsh/keys/example.com/bob-ssh/example.com-cert.pub"

# Flags for all root hosts except the proxy
Host *.root !proxy.example.com
    Port 3022
    ProxyCommand "/tmp/tsh" proxy ssh --cluster=root --proxy=proxy.example.com:443 %r@%h:%p
# Common flags for all leaf hosts
Host *.leaf proxy.example.com
    UserKnownHostsFile "/home/alice/.tsh/known_hosts"
    IdentityFile "/home/alice/.tsh/keys/example.com/bob"
    CertificateFile "/home/alice/.tsh/keys/example.com/bob-ssh/example.com-cert.pub"

# Flags for all leaf hosts except the proxy
Host *.leaf !proxy.example.com
    Port 3022
    ProxyCommand "/tmp/tsh" proxy ssh --cluster=leaf --proxy=proxy.example.com:443 %r@%h:%p

# End generated Teleport configuration
"#;

const USER_AND_PORT_CONFIG: &str = r#"# Begin generated Teleport configuration for proxy.example.com by tsh

# Common flags for all example.com hosts
Host *.example.com proxy.example.com
    UserKnownHostsFile "/home/alice/.tsh/known_hosts"
    IdentityFile "/home/alice/.tsh/keys/example.com/bob"
    CertificateFile "/home/alice/.tsh/keys/example.com/bob-ssh/example.com-cert.pub"
    User "testuser"

# Flags for all example.com hosts except the proxy
Host *.example.com !proxy.example.com
    Port 3232
    ProxyCommand "/tmp/tsh" proxy ssh --cluster=example.com --proxy=proxy.example.com:443 %r@%h:%p
    User "testuser"

# End generated Teleport configuration
"#;

const STATUS_JSON: &str = r#"{
  "active": {
    "profile_url": "https://proxy.example.com:443",
    "username": "bob",
    "cluster": "example.com",
    "roles": ["access"],
    "logins": ["root", "ubuntu"],
    "kubernetes_enabled": false,
    "valid_until": "2026-08-16T08:00:00Z"
  },
  "profiles": []
}"#;

const LS_JSON: &str = r#"[
  {
    "kind": "node",
    "version": "v2",
    "metadata": {
      "name": "6a3d1f5e-1111-4a2b-9c3d-000000000001",
      "labels": {"env": "prod", "role": "db", "empty": ""},
      "expires": "2026-08-15T12:00:00Z"
    },
    "spec": {
      "addr": "10.0.0.5:3022",
      "hostname": "db-1",
      "cmd_labels": {
        "arch": {"period": "1h0m0s", "command": ["uname", "-p"], "result": "x86_64"}
      },
      "rotation": {},
      "version": "16.0.0"
    }
  },
  {
    "kind": "node",
    "sub_kind": "openssh",
    "version": "v2",
    "metadata": {
      "name": "6a3d1f5e-1111-4a2b-9c3d-000000000002",
      "labels": {"env": "prod"}
    },
    "spec": {
      "addr": "10.0.0.6:22",
      "hostname": "legacy-1"
    }
  },
  {
    "kind": "node",
    "version": "v2",
    "metadata": {"name": "6a3d1f5e-1111-4a2b-9c3d-000000000003", "labels": {}},
    "spec": {"addr": "", "hostname": "tunnel-1", "use_tunnel": true}
  },
  {
    "kind": "node",
    "version": "v2",
    "metadata": {"name": "6a3d1f5e-1111-4a2b-9c3d-000000000004"},
    "spec": {"addr": "10.0.0.9:3022", "hostname": ""}
  }
]"#;

fn example_cluster() -> TshClusterConfig {
    parse_tsh_config(SINGLE_CLUSTER_CONFIG)
        .into_iter()
        .next()
        .expect("one cluster")
}

// =========================================================================
// tsh config parsing
// =========================================================================

#[test]
fn parse_tsh_config_single_cluster() {
    let clusters = parse_tsh_config(SINGLE_CLUSTER_CONFIG);
    assert_eq!(clusters.len(), 1);
    let c = &clusters[0];
    assert_eq!(c.cluster, "example.com");
    assert_eq!(c.port, Some(3022));
    assert_eq!(
        c.proxy_command.as_deref(),
        Some("\"/tmp/tsh\" proxy ssh --cluster=example.com --proxy=proxy.example.com:443 %r@%h:%p")
    );
    assert_eq!(
        c.common,
        vec![
            (
                "UserKnownHostsFile".to_string(),
                "/home/alice/.tsh/known_hosts".to_string()
            ),
            (
                "IdentityFile".to_string(),
                "/home/alice/.tsh/keys/example.com/bob".to_string()
            ),
            (
                "CertificateFile".to_string(),
                "/home/alice/.tsh/keys/example.com/bob-ssh/example.com-cert.pub".to_string()
            ),
        ]
    );
}

#[test]
fn parse_tsh_config_multiple_clusters_keeps_each_proxy_command() {
    let clusters = parse_tsh_config(MULTI_CLUSTER_CONFIG);
    let names: Vec<&str> = clusters.iter().map(|c| c.cluster.as_str()).collect();
    assert_eq!(names, ["root", "leaf"]);
    assert!(
        clusters[0]
            .proxy_command
            .as_deref()
            .unwrap()
            .contains("--cluster=root ")
    );
    assert!(
        clusters[1]
            .proxy_command
            .as_deref()
            .unwrap()
            .contains("--cluster=leaf ")
    );
    assert_eq!(clusters[1].common.len(), 3);
}

#[test]
fn parse_tsh_config_reads_custom_port_and_user() {
    let clusters = parse_tsh_config(USER_AND_PORT_CONFIG);
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].port, Some(3232));
    assert!(
        clusters[0]
            .common
            .contains(&("User".to_string(), "testuser".to_string()))
    );
}

#[test]
fn parse_tsh_config_ignores_foreign_host_blocks_and_equals_syntax() {
    let text = "Host bastion\n  HostName 1.2.3.4\n  ProxyCommand nc %h %p\n\n\
                Host *.example.com proxy.example.com\n  UserKnownHostsFile=\"/k\"\n\n\
                Host *.example.com !proxy.example.com\n  Port = 3022\n  ProxyCommand tsh proxy ssh %r@%h:%p\n";
    let clusters = parse_tsh_config(text);
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].cluster, "example.com");
    assert_eq!(
        clusters[0].common,
        vec![("UserKnownHostsFile".to_string(), "/k".to_string())]
    );
    assert_eq!(clusters[0].port, Some(3022));
    assert_eq!(
        clusters[0].proxy_command.as_deref(),
        Some("tsh proxy ssh %r@%h:%p")
    );
}

#[test]
fn parse_tsh_config_empty_input() {
    assert!(parse_tsh_config("").is_empty());
    assert!(parse_tsh_config("# only comments\n\n").is_empty());
}

#[test]
fn unquote_and_addr_port_helpers() {
    assert_eq!(unquote("\"/a b\""), "/a b");
    assert_eq!(unquote("/a"), "/a");
    assert_eq!(unquote("\""), "\"");
    assert_eq!(addr_port("10.0.0.5:3022"), Some(3022));
    assert_eq!(addr_port("10.0.0.5:22"), Some(22));
    assert_eq!(addr_port("[::1]:3022"), Some(3022));
    assert_eq!(addr_port(""), None);
    assert_eq!(addr_port("nohost"), None);
    assert_eq!(addr_port("10.0.0.5:0"), None);
    assert_eq!(addr_port("10.0.0.5:notaport"), None);
}

// =========================================================================
// Node mapping
// =========================================================================

#[test]
fn hosts_from_maps_nodes_labels_ports_and_directives() {
    let nodes: Vec<TshNode> = serde_json::from_str(LS_JSON).unwrap();
    let provider = Teleport {
        identity_file: String::new(),
    };
    let hosts = provider.hosts_from(nodes, &example_cluster());
    // The node without a hostname is skipped.
    assert_eq!(hosts.len(), 3);

    let db = &hosts[0];
    assert_eq!(db.server_id, "6a3d1f5e-1111-4a2b-9c3d-000000000001");
    assert_eq!(db.name, "db-1");
    assert_eq!(db.ip, "db-1");
    assert_eq!(db.port, Some(3022));
    assert_eq!(db.tags, vec!["arch:x86_64", "empty", "env:prod", "role:db"]);
    assert_eq!(
        db.metadata,
        vec![
            ("cluster".to_string(), "example.com".to_string()),
            ("address".to_string(), "10.0.0.5:3022".to_string()),
        ]
    );
    let keys: Vec<&str> = db.directives.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "ProxyCommand",
            "UserKnownHostsFile",
            "IdentityFile",
            "CertificateFile"
        ]
    );
    assert_eq!(
        db.directives[0].1,
        "\"/tmp/tsh\" proxy ssh --cluster=example.com --proxy=proxy.example.com:443 %r@%h:%p"
    );

    // Agentless node on port 22 keeps its own port and reports its kind.
    let legacy = &hosts[1];
    assert_eq!(legacy.name, "legacy-1");
    assert_eq!(legacy.port, Some(22));
    assert!(
        legacy
            .metadata
            .contains(&("type".to_string(), "openssh".to_string()))
    );

    // Reverse-tunnel node without an address falls back to the config port.
    let tunnel = &hosts[2];
    assert_eq!(tunnel.name, "tunnel-1");
    assert_eq!(tunnel.port, Some(3022));
    assert!(tunnel.tags.is_empty());
    assert_eq!(
        tunnel.metadata,
        vec![("cluster".to_string(), "example.com".to_string())]
    );
}

#[test]
fn hosts_from_falls_back_to_default_port_without_config_port() {
    let nodes: Vec<TshNode> = serde_json::from_str(LS_JSON).unwrap();
    let mut cfg = example_cluster();
    cfg.port = None;
    let hosts = Teleport {
        identity_file: String::new(),
    }
    .hosts_from(nodes, &cfg);
    assert_eq!(hosts[2].port, Some(DEFAULT_NODE_PORT));
}

#[test]
fn hosts_from_honors_identity_file_override_and_skips_user() {
    let nodes: Vec<TshNode> = serde_json::from_str(LS_JSON).unwrap();
    let cfg = parse_tsh_config(USER_AND_PORT_CONFIG).remove(0);
    let hosts = Teleport {
        identity_file: "~/.ssh/my_key".to_string(),
    }
    .hosts_from(nodes, &cfg);
    let keys: Vec<&str> = hosts[0]
        .directives
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(
        keys,
        ["ProxyCommand", "UserKnownHostsFile", "CertificateFile"]
    );
    // Config port applies only when the node has no address.
    assert_eq!(hosts[0].port, Some(3022));
    assert_eq!(hosts[2].port, Some(3232));
}

#[test]
fn label_tag_shapes() {
    assert_eq!(label_tag("env", "prod"), "env:prod");
    assert_eq!(label_tag("env", ""), "env");
    assert_eq!(label_tag(" env ", " prod "), "env:prod");
    assert_eq!(label_tag("", "x"), "");
}

// =========================================================================
// End to end through a stub tsh
// =========================================================================

// One stub, written once, driven by env vars per test (see the vault tests
// for why a per-test script races on ETXTBSY under parallel spawn).
#[cfg(unix)]
fn stub_bin_dir() -> &'static std::path::Path {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    static BIN_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    BIN_DIR
        .get_or_init(|| {
            let dir = tempfile::tempdir().expect("create tsh stub dir");
            let tsh = dir.path().join("tsh");
            std::fs::write(
                &tsh,
                "#!/bin/sh\n\
                 if [ -n \"$MOCK_TSH_CAPTURE\" ]; then printf '%s\\n' \"$*\" >>\"$MOCK_TSH_CAPTURE\"; fi\n\
                 if [ -n \"$MOCK_TSH_SLEEP\" ]; then sleep \"$MOCK_TSH_SLEEP\"; fi\n\
                 case \"$1\" in\n\
                   status) printf '%s' \"$MOCK_TSH_STATUS_STDOUT\"; printf '%s' \"$MOCK_TSH_STATUS_STDERR\" >&2; exit \"${MOCK_TSH_STATUS_EXIT:-0}\" ;;\n\
                   config) printf '%s' \"$MOCK_TSH_CONFIG_STDOUT\"; printf '%s' \"$MOCK_TSH_CONFIG_STDERR\" >&2; exit \"${MOCK_TSH_CONFIG_EXIT:-0}\" ;;\n\
                   ls) printf '%s' \"$MOCK_TSH_LS_STDOUT\"; printf '%s' \"$MOCK_TSH_LS_STDERR\" >&2; exit \"${MOCK_TSH_LS_EXIT:-0}\" ;;\n\
                   *) echo \"unexpected: $*\" >&2; exit 64 ;;\n\
                 esac\n",
            )
            .expect("write tsh stub");
            std::fs::set_permissions(&tsh, std::fs::Permissions::from_mode(0o755))
                .expect("chmod tsh stub");
            dir
        })
        .path()
}

#[cfg(unix)]
fn stub_env(home: &std::path::Path) -> Env {
    Env::for_test(home)
        .with_var(
            "PATH",
            format!(
                "{}:{}",
                stub_bin_dir().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .with_var("MOCK_TSH_STATUS_STDOUT", STATUS_JSON)
        .with_var("MOCK_TSH_CONFIG_STDOUT", SINGLE_CLUSTER_CONFIG)
        .with_var("MOCK_TSH_LS_STDOUT", LS_JSON)
}

#[cfg(unix)]
fn provider() -> Teleport {
    Teleport {
        identity_file: String::new(),
    }
}

#[cfg(unix)]
#[test]
fn fetch_runs_status_config_and_ls_in_order() {
    let home = tempfile::tempdir().unwrap();
    let capture = home.path().join("argv.log");
    let env = stub_env(home.path()).with_var("MOCK_TSH_CAPTURE", capture.to_string_lossy());
    let hosts = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .expect("fetch");
    assert_eq!(hosts.len(), 3);
    assert_eq!(hosts[0].name, "db-1");
    assert!(hosts[0].has_directive("ProxyCommand"));

    let calls = std::fs::read_to_string(&capture).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        [
            "status --client --format=json",
            "config",
            "ls --format=json"
        ]
    );
}

#[cfg(unix)]
#[test]
fn fetch_ignores_the_token_field() {
    let home = tempfile::tempdir().unwrap();
    let env = stub_env(home.path());
    let hosts = provider()
        .fetch_hosts_cancellable("some-token", &AtomicBool::new(false), &env)
        .expect("fetch");
    assert_eq!(hosts.len(), 3);
}

#[cfg(unix)]
#[test]
fn find_tsh_skips_empty_path_segments_and_non_executables() {
    use std::os::unix::fs::PermissionsExt;
    // An empty PATH segment must never turn into a lookup in the working dir.
    let cwd = tempfile::tempdir().unwrap();
    let plain = cwd.path().join("tsh");
    std::fs::write(&plain, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
    let env = Env::for_test("/tmp/x").with_var("PATH", ":/nonexistent-purple-tsh-path:");
    assert!(matches!(find_tsh(&env), Err(ProviderError::Execute(_))));
    // A non-executable file on PATH is not tsh either.
    let env = Env::for_test("/tmp/x").with_var("PATH", cwd.path().to_string_lossy());
    assert!(matches!(find_tsh(&env), Err(ProviderError::Execute(_))));
    // The stub dir wins once it is on PATH, in order.
    let env = Env::for_test("/tmp/x").with_var(
        "PATH",
        format!("{}:{}", cwd.path().display(), stub_bin_dir().display()),
    );
    assert_eq!(find_tsh(&env).unwrap(), stub_bin_dir().join("tsh"));
    // No PATH at all: not found.
    assert!(find_tsh(&Env::for_test("/tmp/x")).is_err());
}

#[cfg(unix)]
#[test]
fn fetch_reports_missing_tsh() {
    let env = Env::for_test("/tmp/x").with_var("PATH", "/nonexistent-purple-tsh-path");
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert!(err.to_string().contains("tsh"), "{err}");
    assert!(err.to_string().contains("not found"), "{err}");
}

#[cfg(unix)]
#[test]
fn fetch_reports_logged_out_session_without_running_ls() {
    let home = tempfile::tempdir().unwrap();
    let capture = home.path().join("argv.log");
    let env = stub_env(home.path())
        .with_var("MOCK_TSH_CAPTURE", capture.to_string_lossy())
        .with_var("MOCK_TSH_STATUS_EXIT", "1")
        .with_var("MOCK_TSH_STATUS_STDOUT", "")
        .with_var("MOCK_TSH_STATUS_STDERR", "ERROR: Not logged in.\n");
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Teleport: ERROR: Not logged in. Run `tsh login` and sync again."
    );
    let calls = std::fs::read_to_string(&capture).unwrap();
    assert_eq!(calls.lines().count(), 1, "only status may run: {calls}");
}

#[cfg(unix)]
#[test]
fn fetch_reports_expired_profile() {
    let home = tempfile::tempdir().unwrap();
    let env = stub_env(home.path())
        .with_var("MOCK_TSH_STATUS_EXIT", "1")
        .with_var("MOCK_TSH_STATUS_STDERR", "ERROR: Active profile expired.\n");
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert!(err.to_string().contains("Active profile expired"), "{err}");
    assert!(err.to_string().contains("tsh login"), "{err}");
}

#[cfg(unix)]
#[test]
fn fetch_reports_missing_active_profile() {
    let home = tempfile::tempdir().unwrap();
    let env = stub_env(home.path()).with_var("MOCK_TSH_STATUS_STDOUT", "{\"profiles\": []}");
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert!(err.to_string().contains("no active profile"), "{err}");
}

#[cfg(unix)]
#[test]
fn fetch_reports_config_without_matching_cluster() {
    let home = tempfile::tempdir().unwrap();
    let env = stub_env(home.path()).with_var("MOCK_TSH_CONFIG_STDOUT", MULTI_CLUSTER_CONFIG);
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert!(
        err.to_string().contains("no block for cluster example.com"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn fetch_reports_failing_config_and_ls() {
    let home = tempfile::tempdir().unwrap();
    let env = stub_env(home.path())
        .with_var("MOCK_TSH_CONFIG_EXIT", "1")
        .with_var("MOCK_TSH_CONFIG_STDERR", "ERROR: connection refused\n");
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "tsh config failed: ERROR: connection refused"
    );

    let env = stub_env(home.path())
        .with_var("MOCK_TSH_LS_EXIT", "1")
        .with_var("MOCK_TSH_LS_STDERR", "ERROR: access denied\n");
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert_eq!(err.to_string(), "tsh ls failed: ERROR: access denied");
}

#[cfg(unix)]
#[test]
fn fetch_reports_unparseable_ls_output() {
    let home = tempfile::tempdir().unwrap();
    let env = stub_env(home.path()).with_var("MOCK_TSH_LS_STDOUT", "not json");
    let err = provider()
        .fetch_hosts_cancellable("", &AtomicBool::new(false), &env)
        .unwrap_err();
    assert!(matches!(err, ProviderError::Parse(_)), "{err}");
}

#[cfg(unix)]
#[test]
fn fetch_honors_cancel() {
    let home = tempfile::tempdir().unwrap();
    let env = stub_env(home.path()).with_var("MOCK_TSH_SLEEP", "5");
    let cancel = AtomicBool::new(true);
    let start = std::time::Instant::now();
    let err = provider()
        .fetch_hosts_cancellable("", &cancel, &env)
        .unwrap_err();
    assert!(matches!(err, ProviderError::Cancelled), "{err}");
    assert!(start.elapsed() < Duration::from_secs(4));
}

impl ProviderHost {
    fn has_directive(&self, key: &str) -> bool {
        self.directives.iter().any(|(k, _)| k == key)
    }
}
