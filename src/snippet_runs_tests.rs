use super::*;

fn rec(ts: u64, hosts: usize, ok: usize) -> RunRecord {
    RunRecord {
        ts,
        hosts,
        ok,
        failed: hosts - ok,
    }
}

#[test]
fn empty_log_has_no_runs() {
    let log = SnippetRunLog::default();
    assert_eq!(log.run_count("deploy"), 0);
    assert!(log.for_snippet("deploy").is_empty());
    assert!(log.last_run("deploy").is_none());
    assert!(log.success_rate("deploy").is_none());
}

#[test]
fn record_appends_oldest_first() {
    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(100, 3, 3));
    log.record("deploy", rec(200, 2, 1));
    let runs = log.for_snippet("deploy");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].ts, 100);
    assert_eq!(runs[1].ts, 200);
    assert_eq!(log.last_run("deploy").unwrap().ts, 200);
}

#[test]
fn record_caps_to_max_keeping_most_recent() {
    let mut log = SnippetRunLog::default();
    for i in 0..(MAX_RUNS_PER_SNIPPET as u64 + 5) {
        log.record("deploy", rec(i, 1, 1));
    }
    let runs = log.for_snippet("deploy");
    assert_eq!(runs.len(), MAX_RUNS_PER_SNIPPET);
    // Oldest five dropped: first retained ts is 5.
    assert_eq!(runs.first().unwrap().ts, 5);
    assert_eq!(runs.last().unwrap().ts, MAX_RUNS_PER_SNIPPET as u64 + 4);
}

#[test]
fn success_rate_is_ok_over_total_attempts() {
    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(1, 4, 4)); // 4/4
    log.record("deploy", rec(2, 4, 2)); // 2/4
    // 6 ok of 8 attempts = 0.75
    assert_eq!(log.success_rate("deploy"), Some(0.75));
}

#[test]
fn runs_are_keyed_per_snippet() {
    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(1, 2, 2));
    log.record("backup-db", rec(1, 1, 0));
    assert_eq!(log.run_count("deploy"), 1);
    assert_eq!(log.run_count("backup-db"), 1);
    assert_eq!(log.success_rate("backup-db"), Some(0.0));
}

#[test]
fn record_flush_load_round_trips_to_disk() {
    // flush() is a no-op under the global demo flag, so a parallel test that
    // enables demo mode (every visual snippet test calls build_demo_app) could
    // race this one and yield an empty reload. Serialise via the cross-suite
    // lock and disable demo mode while we run, restoring the prior value after.
    let _guard = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let prior_demo = crate::demo_flag::is_demo();
    crate::demo_flag::disable();

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path());

    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(1234, 3, 2));
    log.flush(Some(&paths)).expect("flush");

    let reloaded = SnippetRunLog::load(Some(&paths));
    assert_eq!(reloaded.run_count("deploy"), 1);
    let r = reloaded.last_run("deploy").unwrap();
    assert_eq!(r.ts, 1234);
    assert_eq!(r.hosts, 3);
    assert_eq!(r.ok, 2);
    assert_eq!(r.failed, 1);

    if prior_demo {
        crate::demo_flag::enable();
    }
}

#[test]
fn rename_moves_history_to_new_name() {
    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(1, 3, 3));
    log.record("deploy", rec(2, 3, 2));
    log.rename("deploy", "ship");
    assert_eq!(log.run_count("deploy"), 0);
    assert_eq!(log.run_count("ship"), 2);
    assert_eq!(log.last_run("ship").unwrap().ts, 2);
}

#[test]
fn rename_to_same_name_is_noop() {
    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(1, 1, 1));
    log.rename("deploy", "deploy");
    assert_eq!(log.run_count("deploy"), 1);
}

#[test]
fn rename_of_unknown_snippet_is_noop() {
    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(1, 1, 1));
    log.rename("missing", "ship");
    assert_eq!(log.run_count("deploy"), 1);
    assert_eq!(log.run_count("ship"), 0);
}

#[test]
fn status_classifies_ok_partial_failed() {
    assert_eq!(rec(1, 3, 3).status(), RunStatus::Ok);
    assert_eq!(rec(1, 4, 3).status(), RunStatus::Partial);
    assert_eq!(rec(1, 2, 0).status(), RunStatus::Failed);
    // Degenerate zero-host run has no failures, so it reads as Ok.
    assert_eq!(rec(1, 0, 0).status(), RunStatus::Ok);
}

#[test]
fn tally_counts_each_status() {
    let records = [
        rec(1, 3, 3), // ok
        rec(2, 4, 3), // partial
        rec(3, 1, 0), // failed
        rec(4, 5, 5), // ok
    ];
    let t = RunTally::of(&records);
    assert_eq!(t.ok, 2);
    assert_eq!(t.partial, 1);
    assert_eq!(t.failed, 1);
}

#[test]
fn tally_of_empty_is_zero() {
    assert_eq!(RunTally::of(&[]), RunTally::default());
}

#[test]
fn load_missing_file_is_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path());
    let log = SnippetRunLog::load(Some(&paths));
    assert_eq!(log.runs.len(), 0);
}

#[test]
fn remove_drops_a_snippets_history() {
    let mut log = SnippetRunLog::default();
    log.record("deploy", rec(1, 2, 2));
    log.record("uptime", rec(2, 1, 1));
    assert_eq!(log.run_count("deploy"), 1);
    log.remove("deploy");
    assert_eq!(log.run_count("deploy"), 0);
    // Other snippets are untouched, and removing an absent name is a no-op.
    assert_eq!(log.run_count("uptime"), 1);
    log.remove("never-existed");
}
