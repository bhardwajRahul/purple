//! Snippet run events. Accumulates per-host stdout/stderr/exit_code into
//! `app.snippets.output` and tracks completion progress for the snippet
//! output overlay.

use crate::app::{self, App};

/// Handle `AppEvent::SnippetHostDone`.
pub(crate) fn handle_snippet_host_done(
    app: &mut App,
    run_id: u64,
    alias: String,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
) {
    // Only the live run touches connection history and the sort order. A stale
    // or superseded run's late SnippetHostDone (the overlay was closed, or a
    // newer run replaced this one) must not record history or re-sort the host
    // list under the user.
    let is_current = app.snippets.output().is_some_and(|s| s.run_id == run_id);
    if is_current && exit_code == Some(0) {
        app.history.record(&alias);
        app.record_key_use(&alias, crate::key_activity::now_secs());
        app.apply_sort();
    }
    if is_current && let Some(state) = app.snippets.output_mut() {
        state.results.push(app::SnippetHostOutput {
            alias,
            stdout,
            stderr,
            exit_code,
        });
    }
}

/// Handle `AppEvent::SnippetProgress`.
pub(crate) fn handle_snippet_progress(app: &mut App, run_id: u64, completed: usize, total: usize) {
    if let Some(state) = app.snippets.output_mut()
        && state.run_id == run_id
    {
        state.completed = completed;
        state.total = total;
    }
}

/// Handle `AppEvent::SnippetAllDone`. Marks the run finished and, on the
/// first transition to done, records the run in the per-snippet ledger
/// (host count + success tally) so the detail TRACK RECORD verdict and
/// trend chart reflect it. Recording is idempotent per run via the
/// `all_done` guard.
pub(crate) fn handle_snippet_all_done(app: &mut App, run_id: u64) {
    let counts = match app.snippets.output_mut() {
        Some(state) if state.run_id == run_id && !state.all_done => {
            state.all_done = true;
            // Record the targeted host count (RunRecord.hosts is documented as
            // "hosts targeted"), matching the terminal-mode path. On a cancelled
            // run state.total exceeds the completed count, so hosts that never
            // ran count as failures. Fall back to the completed count if total
            // was never populated.
            let hosts = state.total.max(state.results.len());
            let ok = state
                .results
                .iter()
                .filter(|r| r.exit_code == Some(0))
                .count();
            Some((hosts, ok))
        }
        _ => None,
    };
    let Some((hosts, ok)) = counts else {
        return;
    };
    let Some(name) = app.snippets.output_snippet_name().map(str::to_string) else {
        return;
    };
    if hosts == 0 || name.is_empty() {
        return;
    }
    let now = crate::key_activity::now_secs();
    let paths = app.env().paths().cloned();
    app.snippets
        .runs_mut()
        .record_run_and_flush(&name, hosts, ok, now, paths.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, SnippetHostOutput, SnippetOutputState};
    use crate::ssh_config::model::SshConfigFile;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn app_with_output(name: &str, exit_codes: &[Option<i32>]) -> App {
        let scratch = tempfile::tempdir().expect("tempdir").keep();
        let config = SshConfigFile {
            elements: SshConfigFile::parse_content(""),
            path: scratch.join("cfg"),
            crlf: false,
            bom: false,
        };
        let mut app = App::new(config);
        let results = exit_codes
            .iter()
            .enumerate()
            .map(|(i, &ec)| SnippetHostOutput {
                alias: format!("h{i}"),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: ec,
            })
            .collect();
        app.snippets.set_output(Some(SnippetOutputState {
            run_id: 7,
            results,
            scroll_offset: 0,
            completed: exit_codes.len(),
            total: exit_codes.len(),
            all_done: false,
            cancel: Arc::new(AtomicBool::new(false)),
        }));
        app.snippets.set_output_snippet_name(Some(name.to_string()));
        app
    }

    #[test]
    fn all_done_records_run_with_success_tally() {
        let mut app = app_with_output("deploy", &[Some(0), Some(1), Some(0)]);
        handle_snippet_all_done(&mut app, 7);
        let runs = app.snippets.runs();
        assert_eq!(runs.run_count("deploy"), 1);
        let r = runs.last_run("deploy").unwrap();
        assert_eq!(r.hosts, 3);
        assert_eq!(r.ok, 2);
        assert_eq!(r.failed, 1);
    }

    #[test]
    fn all_done_records_targeted_host_count_on_cancelled_run() {
        // 3 hosts completed but 5 were targeted (run cancelled): RunRecord.hosts
        // is the targeted count, so the 2 that never ran count as failures.
        let mut app = app_with_output("deploy", &[Some(0), Some(0), Some(1)]);
        if let Some(state) = app.snippets.output_mut() {
            state.total = 5;
        }
        handle_snippet_all_done(&mut app, 7);
        let r = app.snippets.runs().last_run("deploy").unwrap();
        assert_eq!(r.hosts, 5);
        assert_eq!(r.ok, 2);
        assert_eq!(r.failed, 3);
    }

    #[test]
    fn all_done_is_idempotent_per_run() {
        let mut app = app_with_output("deploy", &[Some(0)]);
        handle_snippet_all_done(&mut app, 7);
        handle_snippet_all_done(&mut app, 7); // already all_done: no second record
        assert_eq!(app.snippets.runs().run_count("deploy"), 1);
    }

    #[test]
    fn all_done_ignores_stale_run_id() {
        let mut app = app_with_output("deploy", &[Some(0)]);
        handle_snippet_all_done(&mut app, 999);
        assert_eq!(app.snippets.runs().run_count("deploy"), 0);
    }

    #[test]
    fn host_done_current_run_records_history_and_result() {
        let mut app = app_with_output("deploy", &[]);
        handle_snippet_host_done(
            &mut app,
            7,
            "h-live".into(),
            String::new(),
            String::new(),
            Some(0),
        );
        assert!(app.history.entry("h-live").is_some());
        assert_eq!(app.snippets.output().unwrap().results.len(), 1);
    }

    #[test]
    fn host_done_stale_run_id_records_nothing() {
        // A superseded run's late success must not touch connection history,
        // re-sort the host list or push into the live run's results.
        let mut app = app_with_output("deploy", &[]);
        handle_snippet_host_done(
            &mut app,
            999,
            "h-stale".into(),
            String::new(),
            String::new(),
            Some(0),
        );
        assert!(app.history.entry("h-stale").is_none());
        assert_eq!(app.snippets.output().unwrap().results.len(), 0);
    }
}
