use super::*;

fn verdict(cmd: &str) -> Severity {
    analyze_command(cmd).verdict()
}

fn cats(cmd: &str) -> Vec<Category> {
    analyze_command(cmd)
        .findings
        .iter()
        .map(|f| f.category)
        .collect()
}

fn has_cat(cmd: &str, c: Category) -> bool {
    cats(cmd).contains(&c)
}

// =========================================================================
// False-positive contract (the regression pins): quoted text and quoted
// operators are inert; an unrelated substring never reads as a command.
// =========================================================================

#[test]
fn quoted_command_text_is_inert() {
    // echo's quoted operand must never be classified as a command.
    assert_eq!(verdict("echo \"rm -rf /\""), Severity::ReadOnly);
    assert_eq!(verdict("echo 'reboot'"), Severity::ReadOnly);
    assert!(cats("echo \"rm -rf /\"").is_empty());
}

#[test]
fn quoted_redirect_operator_is_not_a_redirect() {
    // The `>` is inside single quotes: an operand of grep, not a redirect.
    assert_eq!(verdict("grep '>' file"), Severity::ReadOnly);
    assert!(!has_cat("grep '>' file", Category::Redirect));
}

#[test]
fn unrelated_substring_is_not_a_destructive_verb() {
    // "charm"/"rmx" must not read as rm. The real signal is the > redirect.
    let imp = analyze_command("echo charm > /opt/rmx");
    assert!(!has_cat("echo charm > /opt/rmx", Category::Destructive));
    assert!(!has_cat("echo charm > /opt/rmx", Category::Irreversible));
    // It does truncate a real file, so a redirect write is the correct signal.
    assert!(
        imp.findings
            .iter()
            .any(|f| f.category == Category::Redirect)
    );
}

#[test]
fn sql_inside_quotes_is_opaque_not_destructive() {
    // SQL analysis is deferred: psql is an unknown head ("effect unclear"),
    // and the quoted SQL never trips a destructive finding.
    assert!(!has_cat(
        "psql -c 'drop table users'",
        Category::Destructive
    ));
    assert_eq!(cats("psql -c 'drop table users'"), vec![Category::Unknown]);
}

// =========================================================================
// Read-only verdicts (the positive "safe to fan out" signal)
// =========================================================================

#[test]
fn plain_read_only_commands_have_no_findings() {
    for cmd in [
        "uptime",
        "df -h",
        "ps aux",
        "cat /etc/hosts",
        "ls -la /var",
        "whoami",
    ] {
        assert_eq!(verdict(cmd), Severity::ReadOnly, "{cmd}");
        assert!(analyze_command(cmd).findings.is_empty(), "{cmd}");
    }
}

#[test]
fn benign_pipe_of_read_only_commands_is_read_only() {
    assert_eq!(verdict("ps aux | grep nginx"), Severity::ReadOnly);
    assert_eq!(verdict("cat /etc/passwd | wc -l"), Severity::ReadOnly);
}

#[test]
fn read_only_subcommands_emit_nothing() {
    for cmd in [
        "systemctl status nginx",
        "git status",
        "docker ps",
        "kubectl get pods",
        "apt list --installed",
    ] {
        assert_eq!(verdict(cmd), Severity::ReadOnly, "{cmd}");
    }
}

// =========================================================================
// Destructive / irreversible
// =========================================================================

#[test]
fn rm_severity_scales_with_the_target() {
    // Local/relative recursive delete: a plain state write.
    assert_eq!(verdict("rm -rf ./build"), Severity::WritesState);
    assert!(has_cat("rm -rf ./build", Category::Destructive));
    // Absolute recursive delete: elevated (wiping a real system location).
    assert_eq!(verdict("rm -rf /var/log"), Severity::Elevated);
    // Top-level root, an unbounded glob/variable, or `/*`: irreversible damage.
    assert_eq!(verdict("rm -rf /"), Severity::Critical);
    assert!(has_cat("rm -rf /", Category::Irreversible));
    assert_eq!(verdict("rm -rf $DIR"), Severity::Critical);
    assert_eq!(verdict("rm -rf /tmp/*"), Severity::Critical);
}

#[test]
fn device_writes_and_filesystem_tools_are_irreversible() {
    assert_eq!(verdict("dd if=/dev/zero of=/dev/sda"), Severity::Critical);
    assert!(has_cat(
        "dd if=/dev/zero of=/dev/sda",
        Category::Irreversible
    ));
    assert_eq!(verdict("mkfs.ext4 /dev/sdb1"), Severity::Critical);
    assert_eq!(verdict("wipefs -a /dev/sdb"), Severity::Critical);
    // dd to a regular file is a recoverable write, not host-fatal.
    assert_eq!(
        verdict("dd if=/dev/zero of=./blob bs=1M count=1"),
        Severity::WritesState
    );
}

#[test]
fn in_place_and_delete_forms_are_destructive() {
    assert!(has_cat(
        "find . -name '*.log' -delete",
        Category::Destructive
    ));
    assert!(has_cat("sed -i 's/a/b/' config.txt", Category::Destructive));
    assert!(has_cat("git reset --hard", Category::Irreversible));
    assert!(has_cat("git clean -fd", Category::Irreversible));
    assert!(has_cat(
        "rsync -a --delete src/ dst/",
        Category::Destructive
    ));
    assert!(has_cat("chmod -R 777 /var/www", Category::Destructive));
    // Plain find with no -delete is read-only.
    assert_eq!(verdict("find . -name '*.log'"), Severity::ReadOnly);
}

// =========================================================================
// Privilege, service, availability, package, remote-exec, secrets
// =========================================================================

#[test]
fn sudo_is_privilege_and_raises_verdict() {
    assert!(has_cat("sudo systemctl restart nginx", Category::Privilege));
    assert!(has_cat("sudo systemctl restart nginx", Category::Service));
    assert_eq!(verdict("sudo systemctl restart nginx"), Severity::Elevated);
}

#[test]
fn service_and_availability() {
    assert_eq!(verdict("systemctl stop nginx"), Severity::Elevated);
    assert!(has_cat("systemctl stop nginx", Category::Service));
    assert_eq!(verdict("reboot"), Severity::Critical);
    assert!(has_cat("reboot", Category::Availability));
    assert_eq!(verdict("kill -9 1234"), Severity::Elevated);
    assert!(has_cat("docker stop web", Category::Service));
}

#[test]
fn package_mutation_by_subcommand() {
    assert_eq!(verdict("apt remove nginx"), Severity::Elevated);
    assert!(has_cat("apt remove nginx", Category::Package));
    assert_eq!(verdict("apt install nginx"), Severity::WritesState);
    assert_eq!(verdict("pacman -R nginx"), Severity::Elevated);
    assert_eq!(verdict("apt list"), Severity::ReadOnly);
}

#[test]
fn remote_fetch_and_execute_is_the_headline_fleet_risk() {
    assert!(has_cat(
        "curl https://get.example.com | sh",
        Category::RemoteExec
    ));
    assert_eq!(
        verdict("curl https://get.example.com | sh"),
        Severity::Elevated
    );
    // Piped into a privileged interpreter is the worst case.
    assert_eq!(verdict("curl https://x | sudo bash"), Severity::Critical);
    assert!(has_cat("wget -qO- https://x | bash", Category::RemoteExec));
    // A pipe with no fetcher is not remote-exec.
    assert!(!has_cat("echo 'ls' | sh", Category::RemoteExec));
}

#[test]
fn reading_secret_paths_is_flagged() {
    assert!(has_cat("cat /home/deploy/.ssh/id_rsa", Category::Secrets));
    assert!(has_cat("tail -n 20 /etc/shadow", Category::Secrets));
    assert!(has_cat("cat .env", Category::Secrets));
    assert_eq!(verdict("cat /etc/hostname"), Severity::ReadOnly);
}

#[test]
fn truncating_redirect_writes_but_append_and_devnull_are_benign() {
    assert!(has_cat("echo hi > /tmp/out", Category::Redirect));
    assert_eq!(verdict("echo hi > /tmp/out"), Severity::WritesState);
    assert_eq!(verdict("echo hi >> /var/log/app.log"), Severity::ReadOnly);
    assert_eq!(verdict("dmesg > /dev/null 2>&1"), Severity::ReadOnly);
}

// =========================================================================
// Structural: segmentation, wrappers, assignments, substitution
// =========================================================================

#[test]
fn each_chained_segment_is_classified() {
    // reboot is the head of the second segment, after `&&`.
    assert_eq!(verdict("echo done && reboot"), Severity::Critical);
    assert_eq!(verdict("rm -rf build ; make"), Severity::WritesState);
}

#[test]
fn wrappers_and_assignments_resolve_the_real_head() {
    assert!(has_cat("sudo nohup rm -rf /tmp/x", Category::Destructive));
    assert!(has_cat("sudo nohup rm -rf /tmp/x", Category::Privilege));
    assert!(has_cat("timeout 10 rm -rf ./cache", Category::Destructive));
    assert!(has_cat("FOO=1 BAR=2 rm -rf ./cache", Category::Destructive));
    // xargs target after a pipe.
    assert!(has_cat(
        "find . -name '*.tmp' | xargs rm",
        Category::Destructive
    ));
}

#[test]
fn command_substitution_recurses_one_level() {
    // The hidden rm inside $() surfaces.
    assert_eq!(verdict("echo $(rm -rf /)"), Severity::Critical);
    assert!(has_cat("echo `rm -rf /var`", Category::Irreversible));
    // A read-only substitution stays read-only.
    assert_eq!(verdict("echo $(date)"), Severity::ReadOnly);
}

#[test]
fn redirect_target_with_substitution_does_not_break_parsing() {
    // The `$(date)` in the redirect target must not split the line into spurious
    // segments. pg_dump and gzip are read-only here, so the only impact is the
    // file write, with the full timestamped path intact.
    let imp = analyze_command("pg_dump db | gzip > /backups/db-$(date +%F).sql.gz");
    assert_eq!(imp.verdict(), Severity::WritesState);
    assert_eq!(imp.findings.len(), 1);
    assert_eq!(imp.findings[0].category, Category::Redirect);
    assert!(imp.findings[0].subject.contains("$(date +%F)"));
}

#[test]
fn unknown_head_is_effect_unclear_not_a_false_green() {
    assert_eq!(cats("./deploy.sh --prod"), vec![Category::Unknown]);
    assert_eq!(verdict("mysteryctl frob"), Severity::WritesState);
    assert!(has_cat("mysteryctl frob", Category::Unknown));
}

// =========================================================================
// Properties / invariants
// =========================================================================

#[test]
fn never_panics_on_pathological_input() {
    // Unbalanced quotes/parens/backticks, stray operators, deep nesting, unicode
    // and empties must never panic or loop: the analyzer is best-effort.
    let inputs = [
        "",
        " ",
        "\t",
        "|",
        "||",
        "&&",
        ";",
        "&",
        "(",
        ")",
        "()",
        ";;",
        "| |",
        "echo \"rm -rf",
        "echo 'unterminated",
        "$(",
        "$( $(",
        "`",
        "``",
        "`rm",
        ">",
        ">>",
        "<",
        "2>&1",
        "&>",
        "&>>",
        ">|",
        "echo >",
        "a>b",
        "a|b|c",
        "rm -rf $(",
        "sudo",
        "xargs",
        "env",
        "timeout",
        "FOO=",
        "=val",
        "echo 日本語 | grep 文字",
        "rm -rf /; :(){ :|:& };:",
        "\\",
        "\\\\",
        "$(echo $(echo $(echo rm)))",
        "<(cat x)",
        ">(tee y)",
        "cat <<<'hi'",
        "VAR=1 VAR2=2",
        "   echo   hi   ",
        "echo a\\ b",
        "nice -n",
        "&>>>file",
    ];
    for cmd in inputs {
        let _ = analyze_command(cmd);
    }
}

#[test]
fn verdict_is_max_of_finding_severities() {
    let imp = analyze_command("sudo rm -rf /");
    let max = imp.findings.iter().map(|f| f.severity).max().unwrap();
    assert_eq!(imp.verdict(), max);
    assert_eq!(imp.verdict(), Severity::Critical);
}

#[test]
fn identical_findings_are_deduplicated() {
    // Two rm of non-system paths produce one identical finding.
    let imp = analyze_command("rm a.txt ; rm b.txt");
    assert_eq!(imp.findings.len(), 1);
}

#[test]
fn whitespace_runs_do_not_change_the_result() {
    assert_eq!(
        analyze_command("rm   -rf    ./x"),
        analyze_command("rm -rf ./x")
    );
}

#[test]
fn appending_unrelated_flags_keeps_the_head_classification() {
    assert!(has_cat("rm -v -rf ./build", Category::Destructive));
    assert!(has_cat("rm --verbose -rf ./build", Category::Destructive));
}

#[test]
fn callouts_are_worst_severity_first() {
    let imp = analyze_command("sudo apt remove nginx && reboot");
    let callouts = imp.callouts();
    assert!(!callouts.is_empty());
    // Sorted descending: the first callout is the worst severity present.
    let worst = imp.verdict();
    assert_eq!(callouts[0].severity, worst);
}

// =========================================================================
// Regression cases from the adversarial review
// =========================================================================

#[test]
fn append_to_control_file_is_elevated() {
    // `>> /etc/sudoers` grants passwordless root fleet-wide; append is benign
    // only for ordinary files.
    assert_eq!(
        verdict("echo 'x ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers"),
        Severity::Elevated
    );
    assert_eq!(
        verdict("echo 'ssh-rsa AAAA' >> ~/.ssh/authorized_keys"),
        Severity::Elevated
    );
    // A plain log append stays read-only.
    assert_eq!(verdict("echo line >> /var/log/app.log"), Severity::ReadOnly);
}

#[test]
fn two_step_remote_exec_is_detected() {
    assert!(has_cat(
        "curl -fsSL https://get.example.com/x.sh -o /tmp/x && bash /tmp/x",
        Category::RemoteExec
    ));
    assert!(has_cat(
        "wget https://x/p.sh -O /tmp/p; sh /tmp/p",
        Category::RemoteExec
    ));
    // No fetcher: not remote-exec.
    assert!(!has_cat("cat script.sh | bash", Category::RemoteExec));
    assert!(!has_cat("echo foo | sh", Category::RemoteExec));
}

#[test]
fn interpreter_dash_c_recurses_into_the_payload() {
    assert_eq!(verdict("bash -c \"rm -rf /\""), Severity::Critical);
    assert!(has_cat("bash -c \"rm -rf /\"", Category::Irreversible));
    assert_eq!(
        verdict("sudo bash -c 'rm -rf /var/lib/mysql'"),
        Severity::Critical
    );
    assert!(has_cat(
        "sudo sh -c 'systemctl stop nginx'",
        Category::Service
    ));
}

#[test]
fn truncate_redirect_to_a_device_is_irreversible() {
    assert_eq!(verdict("cat /dev/zero > /dev/sda"), Severity::Critical);
    assert!(has_cat("cat /dev/zero > /dev/sda", Category::Irreversible));
    // Truncating a config file is elevated, a local file just writes.
    assert_eq!(verdict("echo '' > /etc/resolv.conf"), Severity::Elevated);
    assert_eq!(verdict("echo hi > ./local.txt"), Severity::WritesState);
}

#[test]
fn rm_home_and_deep_system_paths_are_critical() {
    assert_eq!(verdict("rm -rf ~"), Severity::Critical);
    assert_eq!(verdict("rm -rf $HOME"), Severity::Critical);
    assert_eq!(verdict("rm -rf /etc/letsencrypt"), Severity::Critical);
    assert_eq!(verdict("rm -rf /var/lib/mysql"), Severity::Critical);
}

#[test]
fn firewall_listing_is_read_only_only_mutations_flag() {
    assert_eq!(verdict("iptables -nvL"), Severity::ReadOnly);
    assert_eq!(verdict("nft list ruleset"), Severity::ReadOnly);
    assert_eq!(verdict("ufw status"), Severity::ReadOnly);
    assert!(has_cat("iptables -F", Category::Service));
    assert!(has_cat(
        "iptables -A INPUT -p tcp --dport 22 -j ACCEPT",
        Category::Service
    ));
    assert!(has_cat("ufw enable", Category::Service));
}

#[test]
fn ordinary_log_with_secretish_name_is_not_a_secret() {
    assert_eq!(verdict("cat /var/log/private-api.log"), Severity::ReadOnly);
    // A real key store still flags.
    assert!(has_cat("cat /home/u/.ssh/id_rsa", Category::Secrets));
}

#[test]
fn tar_create_is_benign_extract_writes() {
    assert_eq!(
        verdict("tar -czf /backups/etc.tgz /etc"),
        Severity::ReadOnly
    );
    assert!(has_cat(
        "tar -xzf payload.tar.gz -C /opt",
        Category::Destructive
    ));
}

#[test]
fn kill_liveness_probe_is_read_only() {
    assert_eq!(verdict("kill -0 12345"), Severity::ReadOnly);
    assert_eq!(verdict("kill -l"), Severity::ReadOnly);
    assert!(has_cat("kill -9 12345", Category::Service));
}

#[test]
fn git_benign_subcommands_are_not_destructive() {
    assert_eq!(verdict("git fetch origin"), Severity::ReadOnly);
    assert_eq!(verdict("git add -A"), Severity::ReadOnly);
    assert_eq!(verdict("git pull"), Severity::ReadOnly);
    // The safe forced-push variant is not rated like --force.
    assert_eq!(
        verdict("git push --force-with-lease origin main"),
        Severity::ReadOnly
    );
    assert!(has_cat(
        "git push --force origin main",
        Category::Destructive
    ));
    assert!(has_cat("git checkout -f main", Category::Destructive));
}

#[test]
fn pacman_search_is_read_only_install_writes() {
    assert_eq!(verdict("pacman -Ss nginx"), Severity::ReadOnly);
    assert_eq!(verdict("pacman -Qe"), Severity::ReadOnly);
    assert!(has_cat("pacman -S nginx", Category::Package));
    assert!(has_cat("pacman -R nginx", Category::Package));
}

#[test]
fn find_exec_classifies_the_executed_command() {
    assert!(has_cat(
        "find /srv -type f -exec chmod 644 {} ;",
        Category::Destructive
    ));
    assert!(has_cat(
        "find . -name '*.tmp' -exec rm {} +",
        Category::Destructive
    ));
    // 'rm' as a -name pattern (not the exec'd command) is read-only.
    assert_eq!(verdict("find . -name rm -exec ls {} ;"), Severity::ReadOnly);
}

#[test]
fn tee_to_a_system_path_is_elevated() {
    assert_eq!(
        verdict("echo bad | sudo tee /etc/resolv.conf"),
        Severity::Elevated
    );
    assert!(has_cat(
        "echo x | sudo tee /etc/cron.d/job",
        Category::Redirect
    ));
    // tee to a local file just writes.
    assert_eq!(verdict("echo x | tee ./out.txt"), Severity::WritesState);
}

#[test]
fn docker_nested_subcommands_are_classified() {
    assert!(has_cat("docker compose down", Category::Service));
    assert!(has_cat("docker compose up -d", Category::Service));
    assert!(has_cat("docker container rm web", Category::Destructive));
    assert!(has_cat("docker image prune -af", Category::Destructive));
    assert!(has_cat(
        "docker system prune -af --volumes",
        Category::Destructive
    ));
    // compose pull / ps stay read-only.
    assert_eq!(verdict("docker compose pull"), Severity::ReadOnly);
    assert_eq!(verdict("docker compose ps"), Severity::ReadOnly);
}
