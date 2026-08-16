//! End-to-end tests for `purple provider add` credential validation.
//!
//! Spawns the real binary against a temp HOME so the token gate runs in the
//! process that owns it. `handle_provider_command` rejects by calling
//! `std::process::exit`, which a unit test cannot observe, so the exit status
//! and the saved `~/.purple/providers` file are the assertions here.

#![cfg(unix)]

use std::process::{Command, Output};

fn purple_bin() -> &'static str {
    env!("CARGO_BIN_EXE_purple")
}

struct Fixture {
    home: tempfile::TempDir,
    ssh_config: std::path::PathBuf,
    _config_dir: tempfile::TempDir,
}

fn setup() -> Fixture {
    let home = tempfile::Builder::new()
        .prefix("purple_provider_home_")
        .tempdir()
        .unwrap();
    let config_dir = tempfile::Builder::new()
        .prefix("purple_provider_cfg_")
        .tempdir()
        .unwrap();
    let ssh_config = config_dir.path().join("config");
    std::fs::write(&ssh_config, "Host test\n    HostName test.example.com\n").unwrap();
    Fixture {
        home,
        ssh_config,
        _config_dir: config_dir,
    }
}

/// Run `purple provider add ...` with no credentials in the environment.
fn provider_add(fixture: &Fixture, args: &[&str]) -> Output {
    Command::new(purple_bin())
        .env_clear()
        .env("HOME", fixture.home.path())
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PURPLE_LOG", "debug")
        .arg("--config")
        .arg(&fixture.ssh_config)
        .arg("provider")
        .arg("add")
        .args(args)
        .output()
        .expect("failed to spawn purple binary")
}

fn read_log(fixture: &Fixture) -> String {
    std::fs::read_to_string(fixture.home.path().join(".purple").join("purple.log"))
        .unwrap_or_default()
}

fn saved_config(fixture: &Fixture) -> String {
    std::fs::read_to_string(fixture.home.path().join(".purple").join("providers"))
        .unwrap_or_default()
}

#[test]
fn e2e_provider_add_aws_saves_without_token_or_profile() {
    // Credentials come from AWS_ACCESS_KEY_ID / _SECRET_ACCESS_KEY /
    // _SESSION_TOKEN at sync time, so the save must not demand them here.
    let fixture = setup();
    let output = provider_add(&fixture, &["aws", "--regions", "eu-central-1"]);
    assert!(
        output.status.success(),
        "provider add aws should succeed without credentials. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = saved_config(&fixture);
    assert!(saved.contains("[aws]"), "aws section missing: {saved:?}");
    assert!(
        saved.contains("regions=eu-central-1"),
        "regions not saved: {saved:?}"
    );
    // The save is a state change, so it records what landed on disk. The
    // token is reported as set or empty, never by value.
    let log = read_log(&fixture);
    assert!(
        log.contains("provider saved: [aws]") && log.contains("token=empty"),
        "save not recorded in the log: {log}"
    );
}

#[test]
fn e2e_provider_add_aws_saves_with_profile_only() {
    let fixture = setup();
    let output = provider_add(
        &fixture,
        &["aws", "--profile", "default", "--regions", "eu-central-1"],
    );
    assert!(
        output.status.success(),
        "provider add aws --profile should succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = saved_config(&fixture);
    assert!(
        saved.contains("profile=default"),
        "profile not saved: {saved:?}"
    );
}

#[test]
fn e2e_provider_add_local_cli_providers_save_without_token() {
    // Tailscale falls back to its local CLI and Teleport has no token at all.
    // Both used to hit the token prompt before reaching their own gate.
    for provider in ["tailscale", "teleport"] {
        let fixture = setup();
        let output = provider_add(&fixture, &[provider]);
        assert!(
            output.status.success(),
            "provider add {provider} should succeed without a token. stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let saved = saved_config(&fixture);
        assert!(
            saved.contains(&format!("[{provider}]")),
            "{provider} section missing: {saved:?}"
        );
    }
}

#[test]
fn e2e_provider_add_aws_warns_when_a_requested_token_resolves_empty() {
    // The save is legal: AWS reads credentials elsewhere. A script piping a
    // blank secret looks the same from here, so it must not pass in silence.
    for args in [
        vec!["aws", "--regions", "eu-central-1", "--token", ""],
        vec!["aws", "--regions", "eu-central-1", "--token-stdin"],
    ] {
        let fixture = setup();
        let output = provider_add(&fixture, &args);
        assert!(
            output.status.success(),
            "{args:?} should save. stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("empty token"),
            "{args:?} saved without warning. stderr: {stderr}"
        );
    }
}

#[test]
fn e2e_provider_add_clears_a_stored_token_with_an_empty_one() {
    // Moving an AWS config off a static key onto a profile has to work from
    // the CLI, without hand-editing ~/.purple/providers.
    let fixture = setup();
    let first = provider_add(
        &fixture,
        &["aws", "--regions", "eu-central-1", "--token", "AKID:SECRET"],
    );
    assert!(first.status.success(), "setup add failed");
    assert!(saved_config(&fixture).contains("token=AKID:SECRET"));

    let second = provider_add(
        &fixture,
        &[
            "aws",
            "--regions",
            "eu-central-1",
            "--profile",
            "default",
            "--token",
            "",
        ],
    );
    assert!(
        second.status.success(),
        "clearing the token failed. stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let saved = saved_config(&fixture);
    assert!(
        saved.lines().any(|l| l == "token="),
        "token not cleared: {saved:?}"
    );
    assert!(
        !saved.contains("AKID:SECRET"),
        "old token still on disk: {saved:?}"
    );
    // Losing a stored credential is the part worth naming out loud.
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("replaces the one stored"),
        "clearing a stored token passed unremarked. stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
fn e2e_provider_add_blank_purple_token_keeps_a_stored_credential() {
    // `export PURPLE_TOKEN="$UNSET"` used to skip the stored-token fallback,
    // so an unrelated update wrote the blank straight over a real key.
    let fixture = setup();
    let first = provider_add(
        &fixture,
        &["aws", "--regions", "us-east-1", "--token", "AKID:SECRET"],
    );
    assert!(first.status.success(), "setup add failed");

    let second = Command::new(purple_bin())
        .env_clear()
        .env("HOME", fixture.home.path())
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PURPLE_TOKEN", "")
        .arg("--config")
        .arg(&fixture.ssh_config)
        .args(["provider", "add", "aws", "--regions", "eu-west-1"])
        .output()
        .expect("failed to spawn purple binary");
    assert!(
        second.status.success(),
        "update failed. stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let saved = saved_config(&fixture);
    assert!(
        saved.contains("token=AKID:SECRET"),
        "a blank PURPLE_TOKEN wiped the stored credential: {saved:?}"
    );
    assert!(
        saved.contains("regions=eu-west-1"),
        "the requested change did not land: {saved:?}"
    );
}

#[test]
fn e2e_provider_add_ignores_an_exported_but_blank_purple_token() {
    // `export PURPLE_TOKEN="$UNSET"` leaves the variable present and blank.
    // That is ambient shell state, not a request for a token, so a provider
    // that reads credentials elsewhere still saves.
    for provider in ["teleport", "tailscale"] {
        let fixture = setup();
        let output = Command::new(purple_bin())
            .env_clear()
            .env("HOME", fixture.home.path())
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("PURPLE_TOKEN", "")
            .arg("--config")
            .arg(&fixture.ssh_config)
            .args(["provider", "add", provider])
            .output()
            .expect("failed to spawn purple binary");
        assert!(
            output.status.success(),
            "a blank PURPLE_TOKEN must not block {provider}. stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            saved_config(&fixture).contains(&format!("[{provider}]")),
            "{provider} section missing"
        );
    }
}

#[test]
fn e2e_provider_add_keeps_labeled_configs_apart() {
    // Two accounts of the same provider. The stored-value fallback matches on
    // the exact id, so adding the second one must not inherit the first one's
    // credential and quietly sync the wrong account.
    let fixture = setup();
    let prod = provider_add(
        &fixture,
        &[
            "aws",
            "--label",
            "prod",
            "--prefix",
            "aws-prod",
            "--token",
            "AKIDPROD:SECRETPROD",
            "--regions",
            "us-east-1",
        ],
    );
    assert!(prod.status.success(), "prod add failed");

    let staging = provider_add(
        &fixture,
        &[
            "aws",
            "--label",
            "staging",
            "--prefix",
            "aws-stg",
            "--regions",
            "eu-west-1",
        ],
    );
    assert!(
        staging.status.success(),
        "staging add failed. stderr: {}",
        String::from_utf8_lossy(&staging.stderr)
    );

    let saved = saved_config(&fixture);
    let staging_block = saved
        .split("[aws:staging]")
        .nth(1)
        .expect("staging section missing");
    assert!(
        !staging_block.contains("AKIDPROD:SECRETPROD"),
        "prod credential leaked into staging: {saved:?}"
    );
    assert!(
        saved.contains("AKIDPROD:SECRETPROD"),
        "prod credential should be untouched: {saved:?}"
    );
}

#[test]
fn e2e_provider_add_still_requires_a_token_elsewhere() {
    // Negative control: optionality is per provider, not global.
    let fixture = setup();
    let output = provider_add(&fixture, &["digitalocean"]);
    assert!(
        !output.status.success(),
        "provider add digitalocean must fail without a token. stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        saved_config(&fixture).is_empty(),
        "a rejected add must not write a provider config"
    );
}
