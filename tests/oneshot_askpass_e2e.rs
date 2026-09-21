//! End-to-end test for the one-shot askpass channel.
//!
//! When the user types a password in the TUI and asks purple not to store
//! it, the password travels to ssh in two environment variables for that one
//! run. ssh hands it to purple in askpass mode, which prints it back on
//! stdout. This spawns the real binary the way ssh would, with a cleared
//! environment. It asserts that the password comes back for the host it was
//! typed for and that a different host on the way there does not get it.
//! That the secret never reaches argv is asserted where the command is
//! built, in `askpass_env` and `key_push`.

#![cfg(unix)]

use std::process::Command;

fn purple_bin() -> &'static str {
    env!("CARGO_BIN_EXE_purple")
}

struct Fixture {
    home: tempfile::TempDir,
    config_path: std::path::PathBuf,
    _config_dir: tempfile::TempDir,
}

/// A config with the target plus a bastion it jumps through, so the
/// per-hop scoping can be exercised.
fn setup() -> Fixture {
    let home = tempfile::Builder::new()
        .prefix("purple_oneshot_home_")
        .tempdir()
        .unwrap();
    let config_dir = tempfile::Builder::new()
        .prefix("purple_oneshot_cfg_")
        .tempdir()
        .unwrap();
    let config_path = config_dir.path().join("config");
    std::fs::write(
        &config_path,
        "Host target\n    HostName target.example.com\n    ProxyJump bastion\n\
         \nHost bastion\n    HostName bastion.example.com\n",
    )
    .unwrap();
    Fixture {
        home,
        config_path,
        _config_dir: config_dir,
    }
}

/// A config with one host and no ProxyJump, so an unparsed prompt has only
/// one host it could belong to.
fn setup_single_hop() -> Fixture {
    let home = tempfile::Builder::new()
        .prefix("purple_oneshot_home_")
        .tempdir()
        .unwrap();
    let config_dir = tempfile::Builder::new()
        .prefix("purple_oneshot_cfg_")
        .tempdir()
        .unwrap();
    let config_path = config_dir.path().join("config");
    std::fs::write(&config_path, "Host solo\n    HostName solo.example.com\n").unwrap();
    Fixture {
        home,
        config_path,
        _config_dir: config_dir,
    }
}

/// A config whose target jumps through a host that has no `Host` block of
/// its own. ssh reaches that bastion all the same, but it contributes no
/// alias to the chain, so chain size alone cannot spot it.
fn setup_bare_proxy_jump() -> Fixture {
    let home = tempfile::Builder::new()
        .prefix("purple_oneshot_home_")
        .tempdir()
        .unwrap();
    let config_dir = tempfile::Builder::new()
        .prefix("purple_oneshot_cfg_")
        .tempdir()
        .unwrap();
    let config_path = config_dir.path().join("config");
    std::fs::write(
        &config_path,
        "Host target\n    HostName target.example.com\n    ProxyJump ops@jump.example.com\n",
    )
    .unwrap();
    Fixture {
        home,
        config_path,
        _config_dir: config_dir,
    }
}

/// Run purple in askpass mode the way ssh does, with the one-shot pair set
/// for `oneshot_alias` and the given prompt as argv[1].
fn run_askpass(
    f: &Fixture,
    oneshot_alias: &str,
    secret: &str,
    prompt: &str,
) -> std::process::Output {
    run_askpass_for(f, "target", oneshot_alias, secret, prompt)
}

/// Run purple in askpass mode with a configured source instead of a
/// one-shot password, the way a background run with `# purple:askpass`
/// reaches it. `single_attempt` mirrors what `configure_auth` sets.
fn run_askpass_with_source(f: &Fixture, host_alias: &str, single_attempt: bool) -> bool {
    let mut cmd = Command::new(purple_bin());
    cmd.env_clear()
        .env("PURPLE_ASKPASS_MODE", "1")
        .env("PURPLE_HOST_ALIAS", host_alias)
        .env("PURPLE_CONFIG_PATH", &f.config_path)
        .env("HOME", f.home.path())
        .arg("ops@target.example.com's password: ");
    if single_attempt {
        cmd.env("PURPLE_ASKPASS_SINGLE_ATTEMPT", "1");
    }
    // No source resolves in these fixtures, so purple exits non-zero either
    // way. What the marker decides is whether a file was left behind.
    let _ = cmd.output().expect("failed to spawn purple binary");
    let state = f.home.path().join(".purple");
    std::fs::read_dir(&state)
        .map(|rd| {
            rd.flatten().any(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|s| s.starts_with(".askpass_"))
            })
        })
        .unwrap_or(false)
}

/// Same, with an explicit `PURPLE_HOST_ALIAS` for fixtures whose target is
/// not called `target`.
fn run_askpass_for(
    f: &Fixture,
    host_alias: &str,
    oneshot_alias: &str,
    secret: &str,
    prompt: &str,
) -> std::process::Output {
    Command::new(purple_bin())
        .env_clear()
        .env("PURPLE_ASKPASS_MODE", "1")
        .env("PURPLE_HOST_ALIAS", host_alias)
        .env("PURPLE_CONFIG_PATH", &f.config_path)
        .env("HOME", f.home.path())
        .env("PURPLE_ASKPASS_ONESHOT_ALIAS", oneshot_alias)
        .env("PURPLE_ASKPASS_ONESHOT", secret)
        .arg(prompt)
        .output()
        .expect("failed to spawn purple binary")
}

#[test]
fn the_password_comes_back_for_the_host_it_was_typed_for() {
    let f = setup();
    let out = run_askpass(
        &f,
        "target",
        "hunter2",
        "ops@target.example.com's password: ",
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2");
}

#[test]
fn a_proxy_jump_hop_never_receives_the_targets_password() {
    // ssh fires askpass once per hop. The prompt names the bastion, so the
    // one-shot pair scoped to the target must not answer it. With no source
    // configured for the bastion, purple exits non-zero and ssh falls back.
    let f = setup();
    let out = run_askpass(
        &f,
        "target",
        "hunter2",
        "ops@bastion.example.com's password: ",
    );
    assert!(!out.status.success(), "the bastion must not be answered");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("hunter2"),
        "the target's password leaked to the bastion"
    );
}

#[test]
fn a_password_with_spaces_and_punctuation_round_trips_intact() {
    let f = setup();
    let secret = "c0rrect horse $taple #1!";
    let out = run_askpass(&f, "target", secret, "ops@target.example.com's password: ");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), secret);
}

#[test]
fn a_passphrase_prompt_is_still_refused() {
    // A key passphrase is not the host password, so the one-shot pair must
    // not answer it. purple exits before any source lookup.
    let f = setup();
    let out = run_askpass(
        &f,
        "target",
        "hunter2",
        "Enter passphrase for key '/home/u/.ssh/id_ed25519': ",
    );
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

#[test]
fn a_host_key_question_is_still_refused() {
    let f = setup();
    let out = run_askpass(
        &f,
        "target",
        "hunter2",
        "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
    );
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

#[test]
fn repeated_calls_are_answered_because_ssh_is_the_one_that_counts_attempts() {
    // No marker gates the one-shot. A single operation can take two ssh
    // runs (the remote home lookup and then its listing), and both must be
    // answered. What keeps a wrong password to one login attempt is
    // `-o NumberOfPasswordPrompts=1` on the command purple builds, asserted
    // where each command is built.
    let f = setup();
    let prompt = "ops@target.example.com's password: ";
    for run in 1..=3 {
        let out = run_askpass(&f, "target", "hunter2", prompt);
        assert!(out.status.success(), "run {run} was refused");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2");
    }
}

#[test]
fn an_unparsed_prompt_on_a_direct_host_is_still_answered() {
    // A keyboard-interactive server sends a prompt that names no host. The
    // connection has one hop, so there is no other host the answer could
    // belong to.
    let f = setup_single_hop();
    let out = run_askpass_for(&f, "solo", "solo", "hunter2", "Password: ");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2");
}

#[test]
fn an_unparsed_prompt_on_a_proxy_jump_chain_is_withheld() {
    // The prompt names no host and the connection has two hops, so purple
    // cannot tell whether the bastion or the target is asking. Handing the
    // target's password over on a guess would give it to the bastion.
    let f = setup();
    let out = run_askpass(&f, "target", "hunter2", "Password: ");
    assert!(!out.status.success(), "an unnamed hop must not be answered");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

#[test]
fn a_bare_proxy_jump_hop_never_receives_the_targets_password() {
    // `ProxyJump ops@jump.example.com` with no `Host jump.example.com`
    // block is an ordinary way to write a bastion. It adds no alias to the
    // chain, so the prompt naming it resolves to nothing and the target
    // becomes the fallback. Answering on a parsed-but-unplaced host would
    // hand the target's password straight to that bastion.
    let f = setup_bare_proxy_jump();
    let out = run_askpass(&f, "target", "hunter2", "ops@jump.example.com's password: ");
    assert!(
        !out.status.success(),
        "a host purple cannot place must not be answered"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("hunter2"),
        "the target's password leaked to an unplaced host"
    );
}

#[test]
fn an_unparsed_prompt_behind_a_bare_proxy_jump_is_withheld() {
    // Same config, and this time the prompt names nobody. The chain holds
    // one alias, so a size check would call this direct and answer it. The
    // `ProxyJump` directive is what proves a bastion is in the path.
    let f = setup_bare_proxy_jump();
    let out = run_askpass(&f, "target", "hunter2", "Password: ");
    assert!(
        !out.status.success(),
        "a one-hop chain that still jumps must not be answered blind"
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

#[test]
fn a_background_run_arms_no_retry_marker() {
    // ssh there asks once per hop, so the loop the marker breaks cannot
    // form. Arming one would refuse the second ssh of a two-step
    // operation, such as the remote home lookup followed by its listing.
    let f = setup();
    let armed = run_askpass_with_source(&f, "target", true);
    assert!(!armed, "a background run must leave no marker behind");
}

#[test]
fn a_terminal_run_still_arms_the_retry_marker() {
    // The interactive path keeps ssh's own three attempts, so the marker
    // is what stops a wrong password from looping.
    let f = setup();
    let armed = run_askpass_with_source(&f, "target", false);
    assert!(armed, "the terminal path keeps its retry guard");
}
