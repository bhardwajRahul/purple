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
fn e2e_provider_add_netbox_saves_url_token_and_filter() {
    let fixture = setup();
    let output = provider_add(
        &fixture,
        &[
            "netbox",
            "--url",
            "https://netbox.example.com",
            "--token",
            "tk-netbox",
            "--filter",
            "tag=ssh",
        ],
    );
    assert!(
        output.status.success(),
        "provider add netbox should succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = saved_config(&fixture);
    assert!(
        saved.contains("[netbox]"),
        "netbox section missing: {saved:?}"
    );
    assert!(
        saved.contains("url=https://netbox.example.com"),
        "url not saved: {saved:?}"
    );
    assert!(
        saved.contains("token=tk-netbox"),
        "token not saved: {saved:?}"
    );
    assert!(
        saved.contains("filter=tag=ssh"),
        "filter not saved: {saved:?}"
    );
}

#[test]
fn e2e_provider_add_netbox_requires_url() {
    let fixture = setup();
    let output = provider_add(&fixture, &["netbox", "--token", "tk"]);
    assert!(
        !output.status.success(),
        "provider add netbox must fail without --url. stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--url"),
        "error must name the missing flag. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        saved_config(&fixture).is_empty(),
        "a rejected add must not write a provider config"
    );
}

#[test]
fn e2e_provider_add_netbox_keeps_stored_url_and_filter_on_update() {
    // An update that only touches the token must carry the stored url and
    // filter forward, mirroring the Proxmox url fallback.
    let fixture = setup();
    let first = provider_add(
        &fixture,
        &[
            "netbox",
            "--url",
            "https://netbox.example.com",
            "--token",
            "tk-old",
            "--filter",
            "tag=ssh",
        ],
    );
    assert!(first.status.success(), "setup add failed");

    let second = provider_add(&fixture, &["netbox", "--token", "tk-new"]);
    assert!(
        second.status.success(),
        "token-only update failed. stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let saved = saved_config(&fixture);
    assert!(
        saved.contains("token=tk-new"),
        "token not updated: {saved:?}"
    );
    assert!(
        saved.contains("url=https://netbox.example.com"),
        "stored url lost on update: {saved:?}"
    );
    assert!(
        saved.contains("filter=tag=ssh"),
        "stored filter lost on update: {saved:?}"
    );
}

#[test]
fn e2e_provider_add_filter_ignored_for_other_providers() {
    let fixture = setup();
    let output = provider_add(
        &fixture,
        &["hetzner", "--token", "tk", "--filter", "tag=ssh"],
    );
    assert!(
        output.status.success(),
        "provider add hetzner should succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--filter"),
        "the ignored flag must be named on stderr. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !saved_config(&fixture).contains("filter="),
        "filter must not be saved for a non-NetBox provider"
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

// ── Session Manager and the multi-account hint ───────────────────────

#[test]
fn e2e_provider_add_aws_saves_the_ssm_mode() {
    let fixture = setup();
    let output = provider_add(
        &fixture,
        &["aws", "--regions", "eu-central-1", "--ssm", "auto"],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = saved_config(&fixture);
    assert!(saved.contains("ssm=auto"), "mode not saved: {saved:?}");
    // The mode is part of what a save changed, so it lands in the record.
    assert!(
        read_log(&fixture).contains("ssm=auto"),
        "mode not recorded in the log"
    );
}

#[test]
fn e2e_provider_add_aws_ssm_off_is_not_written() {
    // Off is the default, so writing it would put a line in every AWS config
    // that says nothing.
    let fixture = setup();
    let output = provider_add(
        &fixture,
        &["aws", "--regions", "eu-central-1", "--ssm", "off"],
    );
    assert!(output.status.success());
    let saved = saved_config(&fixture);
    assert!(
        !saved.contains("ssm="),
        "off should not be written: {saved:?}"
    );
}

#[test]
fn e2e_provider_add_rejects_an_unknown_ssm_mode() {
    // A typo must not read as off, which would quietly leave every host on
    // its IP address.
    let fixture = setup();
    let output = provider_add(
        &fixture,
        &["aws", "--regions", "eu-central-1", "--ssm", "enabled"],
    );
    assert!(!output.status.success(), "a typo must not save");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("off, auto or always"), "stderr: {stderr}");
    assert!(saved_config(&fixture).is_empty(), "nothing should be saved");
}

#[test]
fn e2e_provider_add_ssm_mode_survives_an_update_that_omits_it() {
    let fixture = setup();
    provider_add(
        &fixture,
        &["aws", "--regions", "eu-central-1", "--ssm", "always"],
    );
    let output = provider_add(&fixture, &["aws", "--user", "ubuntu"]);
    assert!(output.status.success());
    let saved = saved_config(&fixture);
    assert!(
        saved.contains("ssm=always"),
        "mode lost on update: {saved:?}"
    );
    assert!(
        saved.contains("user=ubuntu"),
        "update not applied: {saved:?}"
    );
}

#[test]
fn e2e_provider_add_warns_that_ssm_is_aws_only() {
    let fixture = setup();
    let output = provider_add(
        &fixture,
        &["digitalocean", "--token", "dop_v1_x", "--ssm", "auto"],
    );
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--ssm is only used by the AWS provider"),
        "stderr: {stderr}"
    );
    assert!(
        !saved_config(&fixture).contains("ssm="),
        "mode must be dropped"
    );
}

#[test]
fn e2e_provider_add_names_a_route_that_works_when_it_replaces_a_config() {
    // Replacing in place is the documented behavior but it reads as data
    // loss, which is what issue #138 reported. The second add says once how
    // to keep both, and the route it names has to be one the CLI accepts:
    // `--label` on a provider that still has a bare config is refused two
    // guards later, so pointing at it would advise a command that cannot run.
    let fixture = setup();
    let first = provider_add(&fixture, &["aws", "--regions", "eu-central-1"]);
    assert!(first.status.success());
    assert!(
        !String::from_utf8_lossy(&first.stdout).contains("keep two side by side"),
        "a first add has nothing to replace, so it must stay quiet"
    );

    let second = provider_add(&fixture, &["aws", "--regions", "us-east-1"]);
    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("keep two side by side"),
        "hint missing: {stdout}"
    );
    assert!(
        !stdout.contains("--label"),
        "the hint must not name a flag this config refuses: {stdout}"
    );

    // The refusal the old hint walked into, asserted so the two stay aligned.
    let labeled = provider_add(
        &fixture,
        &["aws", "--label", "second", "--regions", "us-east-1"],
    );
    assert!(
        !labeled.status.success(),
        "a labeled add must still be refused while a bare config exists"
    );
    let stderr = String::from_utf8_lossy(&labeled.stderr);
    assert!(stderr.contains("bare config"), "stderr: {stderr}");
}

#[test]
fn e2e_provider_add_warns_when_session_manager_has_no_profile() {
    // The proxy command can carry a --profile and nothing else, so an inline
    // key pair never reaches the session. Saving is still allowed: the aws
    // CLI may well find credentials of its own.
    let fixture = setup();
    let out = provider_add(
        &fixture,
        &[
            "aws",
            "--token",
            "AKIAAAAAAAAAAAAAAAAA:secret",
            "--regions",
            "eu-central-1",
            "--ssm",
            "auto",
        ],
    );
    assert!(out.status.success(), "the save must still go through");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Session Manager is on without a profile"),
        "stderr: {stderr}"
    );
}

#[test]
fn e2e_provider_add_stays_quiet_when_session_manager_has_a_profile() {
    let fixture = setup();
    let out = provider_add(
        &fixture,
        &[
            "aws",
            "--profile",
            "default",
            "--regions",
            "eu-central-1",
            "--ssm",
            "auto",
        ],
    );
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("Session Manager is on without a profile"),
        "stderr: {stderr}"
    );
}

#[test]
fn e2e_provider_add_keeps_two_aws_accounts_apart() {
    // The multi-account shape issue #138 asked for, end to end.
    let fixture = setup();
    let prod = provider_add(
        &fixture,
        &[
            "aws",
            "--label",
            "prod",
            "--profile",
            "org-prod",
            "--regions",
            "eu-west-1",
        ],
    );
    assert!(
        prod.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&prod.stderr)
    );
    let dev = provider_add(
        &fixture,
        &[
            "aws",
            "--label",
            "dev",
            "--profile",
            "org-dev",
            "--regions",
            "eu-west-1",
        ],
    );
    assert!(
        dev.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&dev.stderr)
    );

    let saved = saved_config(&fixture);
    assert!(saved.contains("[aws:prod]"), "got: {saved}");
    assert!(saved.contains("[aws:dev]"), "got: {saved}");
    assert!(saved.contains("profile=org-prod"), "got: {saved}");
    assert!(saved.contains("profile=org-dev"), "got: {saved}");
    // Distinct prefixes keep the two accounts' aliases apart.
    assert!(saved.contains("alias_prefix=aws-prod"), "got: {saved}");
    assert!(saved.contains("alias_prefix=aws-dev"), "got: {saved}");
}

#[test]
fn e2e_provider_add_refuses_a_profile_name_it_cannot_put_in_a_proxy_command() {
    // The name goes into a shell line purple writes, so it is refused at save
    // time rather than on the next sync.
    let fixture = setup();
    let out = provider_add(
        &fixture,
        &[
            "aws",
            "--profile",
            "my work",
            "--regions",
            "eu-central-1",
            "--ssm",
            "auto",
        ],
    );
    assert!(!out.status.success(), "an unsafe name must be refused");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("ProxyCommand"), "stderr: {stderr}");
    assert!(
        !saved_config(&fixture).contains("my work"),
        "nothing may be written"
    );
}

#[test]
fn e2e_provider_add_allows_the_same_name_with_session_manager_off() {
    // The name only has to be shell-safe because it goes into a command; with
    // Session Manager off it never does.
    let fixture = setup();
    let out = provider_add(
        &fixture,
        &["aws", "--profile", "my work", "--regions", "eu-central-1"],
    );
    assert!(out.status.success(), "off must not validate the name");
}
