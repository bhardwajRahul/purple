//! End-to-end tests for the XDG Base Directory layout.
//!
//! Spawns the real binary against a temp HOME so the whole chain runs in one
//! process: environment capture, category resolution, the first-run copy
//! from `~/.purple`, logging into the state directory and the CLI commands
//! that read or write the moved files.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn purple_bin() -> &'static str {
    env!("CARGO_BIN_EXE_purple")
}

struct Fixture {
    root: tempfile::TempDir,
    home: PathBuf,
    ssh_config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("purple_xdg_")
            .tempdir()
            .unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let ssh_config = root.path().join("ssh_config");
        std::fs::write(&ssh_config, "Host test\n    HostName test.example.com\n").unwrap();
        Self {
            root,
            home,
            ssh_config,
        }
    }

    fn legacy(&self) -> PathBuf {
        self.home.join(".purple")
    }

    fn base(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// Run purple with the given extra environment and arguments.
    fn run(&self, env: &[(&str, &Path)], args: &[&str]) -> Output {
        let mut cmd = Command::new(purple_bin());
        cmd.env_clear()
            .env("HOME", &self.home)
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("PURPLE_LOG", "debug")
            .arg("--config")
            .arg(&self.ssh_config)
            .args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("failed to spawn purple binary")
    }
}

fn seed_legacy(fixture: &Fixture) {
    let legacy = fixture.legacy();
    std::fs::create_dir_all(legacy.join("certs")).unwrap();
    std::fs::write(legacy.join("preferences"), "theme=Nord\n").unwrap();
    std::fs::write(
        legacy.join("history.tsv"),
        "test\t1700000000\t3\t1700000000\n",
    )
    .unwrap();
    std::fs::write(legacy.join("certs/test-cert.pub"), "cert").unwrap();
    std::fs::write(legacy.join("container_cache.jsonl"), "").unwrap();
}

#[test]
fn e2e_without_variables_everything_stays_in_dot_purple() {
    let fixture = Fixture::new();
    seed_legacy(&fixture);

    let output = fixture.run(&[], &["logs"]);
    assert!(output.status.success(), "{output:?}");
    let printed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        printed,
        fixture.legacy().join("purple.log").display().to_string()
    );
    // No category directory appeared anywhere else.
    for name in ["cfg", "data", "state", "cache"] {
        assert!(!fixture.base(name).exists(), "{name} must not be created");
    }
}

#[test]
fn e2e_xdg_variables_split_the_tree_and_seed_it_from_dot_purple() {
    let fixture = Fixture::new();
    seed_legacy(&fixture);
    let cfg = fixture.base("cfg");
    let data = fixture.base("data");
    let state = fixture.base("state");
    let cache = fixture.base("cache");
    let env: Vec<(&str, &Path)> = vec![
        ("XDG_CONFIG_HOME", &cfg),
        ("XDG_DATA_HOME", &data),
        ("XDG_STATE_HOME", &state),
        ("XDG_CACHE_HOME", &cache),
    ];

    // `purple logs` prints the resolved log path: the state directory.
    let output = fixture.run(&env, &["logs"]);
    assert!(output.status.success(), "{output:?}");
    let printed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        printed,
        state.join("purple/purple.log").display().to_string()
    );

    // Every category was seeded from the legacy tree, which is untouched.
    assert_eq!(
        std::fs::read_to_string(cfg.join("purple/preferences")).unwrap(),
        "theme=Nord\n"
    );
    assert!(data.join("purple/certs/test-cert.pub").exists());
    assert!(state.join("purple/history.tsv").exists());
    assert!(cache.join("purple/container_cache.jsonl").exists());
    assert!(fixture.legacy().join("preferences").exists());
    assert!(fixture.legacy().join("certs/test-cert.pub").exists());

    // The copies are logged into the new state directory.
    let log = std::fs::read_to_string(state.join("purple/purple.log")).unwrap_or_default();
    assert!(
        log.contains("Seeded 4 entries from ~/.purple into the split directories"),
        "migration summary missing from log:\n{log}"
    );

    // A second run copies nothing and its startup banner names all four
    // directories. `--list` reaches the banner; `logs` returns before it.
    let output = fixture.run(&env, &["--list"]);
    assert!(output.status.success(), "{output:?}");
    let log = std::fs::read_to_string(state.join("purple/purple.log")).unwrap_or_default();
    assert_eq!(
        log.matches("Seeded 4 entries").count(),
        1,
        "the second run must not seed again:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "dirs: config={} data={} state={} cache={}",
            cfg.join("purple").display(),
            data.join("purple").display(),
            state.join("purple").display(),
            cache.join("purple").display()
        )),
        "banner dirs line missing from log:\n{log}"
    );
}

#[cfg(unix)]
#[test]
fn e2e_created_category_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let cfg = fixture.base("cfg");
    let output = fixture.run(&[("XDG_CONFIG_HOME", cfg.as_path())], &["logs"]);
    assert!(output.status.success(), "{output:?}");
    let mode = std::fs::metadata(cfg.join("purple"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn e2e_purple_override_beats_xdg_and_theme_reads_from_it() {
    let fixture = Fixture::new();
    let xdg_cfg = fixture.base("cfg");
    let override_dir = fixture.base("override");
    std::fs::create_dir_all(&override_dir).unwrap();
    std::fs::write(override_dir.join("preferences"), "theme=Nord\n").unwrap();
    let env: Vec<(&str, &Path)> = vec![
        ("XDG_CONFIG_HOME", &xdg_cfg),
        ("PURPLE_CONFIG_DIR", &override_dir),
    ];

    // `purple theme list` marks the theme saved in the preferences file.
    let output = fixture.run(&env, &["theme", "list"]);
    assert!(output.status.success(), "{output:?}");
    let listed = String::from_utf8_lossy(&output.stdout);
    let nord_line = listed
        .lines()
        .find(|l| l.contains("Nord"))
        .unwrap_or_default();
    assert!(
        nord_line.trim_start().starts_with('*'),
        "Nord should be marked as the current theme, got: {listed}"
    );
    assert!(!xdg_cfg.join("purple/preferences").exists());
}

#[test]
fn e2e_first_run_after_setting_xdg_config_already_uses_the_migrated_theme() {
    // The seed lands before any command reads the config directory, so the
    // very first start with XDG_CONFIG_HOME set sees the theme saved under
    // ~/.purple.
    let fixture = Fixture::new();
    seed_legacy(&fixture);
    let cfg = fixture.base("cfg");
    let output = fixture.run(&[("XDG_CONFIG_HOME", cfg.as_path())], &["theme", "list"]);
    assert!(output.status.success(), "{output:?}");
    let listed = String::from_utf8_lossy(&output.stdout);
    let nord_line = listed
        .lines()
        .find(|l| l.contains("Nord"))
        .unwrap_or_default();
    assert!(
        nord_line.trim_start().starts_with('*'),
        "Nord should be current on the first run, got: {listed}"
    );
    assert_eq!(
        std::fs::read_to_string(cfg.join("purple/preferences")).unwrap(),
        "theme=Nord\n"
    );
}

#[test]
fn e2e_relative_xdg_path_is_ignored() {
    let fixture = Fixture::new();
    seed_legacy(&fixture);
    let output = fixture.run(
        &[("XDG_STATE_HOME", Path::new("relative/state"))],
        &["logs"],
    );
    assert!(output.status.success(), "{output:?}");
    let printed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        printed,
        fixture.legacy().join("purple.log").display().to_string()
    );
}
