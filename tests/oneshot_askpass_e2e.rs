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

/// A config written the older way, with the bastion named inside a
/// `ProxyCommand` instead of a `ProxyJump`. The bastion is a real second
/// machine, yet it appears in no directive purple parses into the chain.
fn setup_proxy_command() -> Fixture {
    fixture_with(
        "Host target\n    HostName target.example.com\n    ProxyCommand ssh -W %h:%p bastion\n\nHost bastion\n    HostName bastion.example.com\n",
    )
}

/// Build a fixture around one config body.
fn fixture_with(config: &str) -> Fixture {
    let home = tempfile::Builder::new()
        .prefix("purple_oneshot_home_")
        .tempdir()
        .unwrap();
    let config_dir = tempfile::Builder::new()
        .prefix("purple_oneshot_cfg_")
        .tempdir()
        .unwrap();
    let config_path = config_dir.path().join("config");
    std::fs::write(&config_path, config).unwrap();
    Fixture {
        home,
        config_path,
        _config_dir: config_dir,
    }
}

/// The password a configured source hands back in the source-path tests.
const SOURCE_SECRET: &str = "source-secret";

/// A config carrying a custom-command source on the target, so the source
/// path can be exercised without a keychain or a vault. `route` is the
/// directive that puts a bastion in front of it, empty for a direct host.
fn setup_with_source(route: &str) -> Fixture {
    fixture_with(&format!(
        "Host target\n    HostName target.example.com\n{route}    # purple:askpass echo {SOURCE_SECRET}\n\nHost bastion\n    HostName bastion.example.com\n"
    ))
}

/// Run purple in askpass mode with a configured source and no one-shot pair,
/// the way ssh reaches it for a host carrying `# purple:askpass`. PATH is
/// kept because a custom-command source runs through the shell.
fn run_askpass_source_only(f: &Fixture, prompt: &str) -> std::process::Output {
    Command::new(purple_bin())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("PURPLE_ASKPASS_MODE", "1")
        .env("PURPLE_HOST_ALIAS", "target")
        .env("PURPLE_CONFIG_PATH", &f.config_path)
        .env("HOME", f.home.path())
        .arg(prompt)
        .output()
        .expect("failed to spawn purple binary")
}

/// Run purple in askpass mode with `PATH` intact, so the `ssh -G` probe that
/// settles whether the connection proxies can actually run. Everything else
/// matches `run_askpass`.
fn run_askpass_with_ssh_on_path(
    f: &Fixture,
    host_alias: &str,
    prompt: &str,
) -> std::process::Output {
    Command::new(purple_bin())
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("PURPLE_ASKPASS_MODE", "1")
        .env("PURPLE_HOST_ALIAS", host_alias)
        .env("PURPLE_CONFIG_PATH", &f.config_path)
        .env("HOME", f.home.path())
        .env("PURPLE_ASKPASS_ONESHOT_ALIAS", host_alias)
        .env("PURPLE_ASKPASS_ONESHOT", "hunter2")
        .arg(prompt)
        .output()
        .expect("failed to spawn purple binary")
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

// --- a bastion named inside a ProxyCommand ---
//
// `ProxyCommand ssh -W %h:%p bastion` is the older way of writing what
// `ProxyJump bastion` writes today, and it reaches a second machine just the
// same. The bastion appears in no directive that contributes to the chain,
// so the connection has to be read as one that jumps.

#[test]
fn a_proxy_command_bastion_never_receives_the_targets_password() {
    // The prompt names nobody, which is what a keyboard-interactive server
    // sends. Reading the chain as direct would hand the bastion the
    // password the user typed for the target.
    let f = setup_proxy_command();
    let out = run_askpass(&f, "target", "hunter2", "Password: ");
    assert!(
        !out.status.success(),
        "a connection that proxies must not be answered blind"
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

#[test]
fn a_proxy_command_bastion_is_refused_when_it_names_itself_too() {
    let f = setup_proxy_command();
    let out = run_askpass(
        &f,
        "target",
        "hunter2",
        "ops@bastion.example.com's password: ",
    );
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

// --- routes purple's own model cannot see ---
//
// A `Match` block, a directive above the first `Host` line or a
// canonicalized name each change where ssh actually connects. None of them
// reaches the host model. ssh reports its own reading. That is the one the
// gate follows.

#[test]
fn a_bastion_named_only_in_a_match_block_is_still_refused() {
    let f = fixture_with(
        "Host db\n    HostName db.internal\n\nMatch host db.internal\n    ProxyCommand ssh -W %h:%p bastion.example.com\n",
    );
    let out = run_askpass_with_ssh_on_path(&f, "db", "ops@bastion.example.com's password: ");
    assert!(
        !out.status.success(),
        "a bastion ssh routes through must not be answered"
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

#[test]
fn a_host_ssh_reports_as_direct_is_answered_on_a_prompt_it_cannot_place() {
    // Nothing proxies, so the one machine in the connection is the one
    // asking, whatever name ssh spells out for it.
    let f = fixture_with("Host solo\n    HostName solo.example.com\n");
    let out = run_askpass_with_ssh_on_path(&f, "solo", "ops@some.other.name's password: ");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2");
}

// --- the marker that tells the TUI a prompt could not be placed ---

/// True when the fixture's state directory holds the withheld marker for
/// `alias`, which is what the TUI reads to decide against asking.
fn withheld_marker_exists(f: &Fixture, alias: &str) -> bool {
    let wanted = format!(".askpass_withheld.{alias}");
    std::fs::read_dir(f.home.path().join(".purple"))
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_str().is_some_and(|s| s == wanted))
        })
        .unwrap_or(false)
}

#[test]
fn a_withheld_prompt_leaves_a_marker_for_the_tui() {
    // Asking the user for a password here would be asking for something
    // that meets the same wall, so the TUI has to be able to tell this
    // apart from an ordinary refusal.
    let f = setup_proxy_command();
    let out = run_askpass(&f, "target", "hunter2", "Password: ");
    assert!(!out.status.success());
    assert!(
        withheld_marker_exists(&f, "target"),
        "the TUI needs to know the prompt could not be placed"
    );
}

#[test]
fn an_answered_prompt_leaves_no_withheld_marker() {
    let f = setup_single_hop();
    let out = run_askpass_for(&f, "solo", "solo", "hunter2", "Password: ");
    assert!(out.status.success());
    assert!(!withheld_marker_exists(&f, "solo"));
}

// --- the configured-source path ---
//
// A source reaches ssh through the same askpass call as a typed password
// and lands on the same machine, so it is released on the same evidence.

#[test]
fn a_source_is_withheld_from_a_prompt_naming_a_bastion_it_cannot_place() {
    // The prompt names the bastion, which sits in no directive that reaches
    // the chain. Falling back to the target here would hand the target's
    // own source straight to the machine in front of it.
    let f = setup_with_source("    ProxyCommand ssh -W %h:%p bastion\n");
    let out = run_askpass_source_only(&f, "ops@bastion.example.com's password: ");
    assert!(!out.status.success(), "the hop cannot be placed");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(SOURCE_SECRET));
}

#[test]
fn a_source_is_withheld_from_an_unnamed_prompt_behind_a_proxy_command() {
    let f = setup_with_source("    ProxyCommand ssh -W %h:%p bastion\n");
    let out = run_askpass_source_only(&f, "Password: ");
    assert!(!out.status.success(), "the hop cannot be placed");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(SOURCE_SECRET));
}

#[test]
fn a_source_is_withheld_from_an_unnamed_prompt_behind_a_jump_host() {
    let f = setup_with_source("    ProxyJump bastion\n");
    let out = run_askpass_source_only(&f, "Password: ");
    assert!(!out.status.success(), "the hop cannot be placed");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(SOURCE_SECRET));
}

#[test]
fn a_source_still_answers_a_prompt_that_names_the_target() {
    // The everyday case behind a bastion: ssh spells out whose password it
    // wants, so the hop is placed and the source is released.
    let f = setup_with_source("    ProxyJump bastion\n");
    let out = run_askpass_source_only(&f, "ops@target.example.com's password: ");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), SOURCE_SECRET);
}

#[test]
fn a_wildcard_proxy_command_puts_a_bastion_in_front_of_every_host() {
    // `Host *` with a ProxyCommand is how a whole estate is routed through
    // one machine. The target names nothing itself, so the pattern is the
    // only place the bastion appears.
    let f = fixture_with(
        "Host *\n    ProxyCommand ssh -W %h:%p bastion\n\nHost target\n    HostName target.example.com\n",
    );
    let out = run_askpass(&f, "target", "hunter2", "Password: ");
    assert!(
        !out.status.success(),
        "the pattern routes through a bastion"
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("hunter2"));
}

#[test]
fn a_host_opting_out_of_a_wildcard_proxy_command_is_answered() {
    // `ProxyCommand none` above the pattern is how one machine is taken
    // back out of a route that covers everything. ssh takes the first value
    // it obtains, so the opt-out has to come first to have any effect. The
    // host then reaches its target directly, and an unnamed prompt has only
    // one host it could belong to.
    let f = fixture_with(
        "Host target\n    HostName target.example.com\n    ProxyCommand none\n\nHost *\n    ProxyCommand ssh -W %h:%p bastion\n",
    );
    let out = run_askpass(&f, "target", "hunter2", "Password: ");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hunter2");
}

#[test]
fn a_source_still_answers_an_unnamed_prompt_on_a_direct_host() {
    // Nothing proxies, so there is no second machine the prompt could have
    // come from. A keyboard-interactive server keeps working.
    let f = setup_with_source("");
    let out = run_askpass_source_only(&f, "Password: ");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), SOURCE_SECRET);
}
