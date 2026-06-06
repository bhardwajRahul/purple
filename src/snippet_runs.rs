//! Per-snippet run ledger persisted to `~/.purple/snippet_runs.json`.
//!
//! Each recorded run captures when a snippet was executed, how many hosts it
//! targeted and how many succeeded. The Snippets detail panel reads this log to
//! feed the TRACK RECORD card (the reliability verdict in the title border and
//! the inset run-history trend chart) so an operator can judge whether a snippet
//! is safe to run across a fleet.
//!
//! Keyed by snippet name. Records are capped per snippet to the most recent
//! `MAX_RUNS_PER_SNIPPET`. Corrupt files are preserved aside before defaulting,
//! mirroring `key_activity`.

use std::collections::HashMap;
use std::io;

use log::{debug, warn};
use serde::{Deserialize, Serialize};

use crate::fs_util;
use crate::runtime::env::Paths;

/// Most recent runs retained per snippet. Bounds the file and the trend chart.
const MAX_RUNS_PER_SNIPPET: usize = 30;

/// One snippet execution against a set of hosts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunRecord {
    /// Seconds since UNIX epoch.
    pub ts: u64,
    /// Number of hosts targeted.
    pub hosts: usize,
    /// Hosts that exited 0.
    pub ok: usize,
    /// Hosts that failed (non-zero exit, no exit or launch error).
    pub failed: usize,
}

/// Outcome of a single run, the single source of truth shared by the RUN
/// HISTORY bar colour and its summary line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// Every targeted host exited 0.
    Ok,
    /// Some hosts succeeded, some failed.
    Partial,
    /// No host succeeded.
    Failed,
}

impl RunRecord {
    /// Classify the run. A run with no failures is `Ok` (including the
    /// degenerate zero-host run), one with at least one success and one
    /// failure is `Partial`, otherwise `Failed`.
    pub fn status(&self) -> RunStatus {
        if self.failed == 0 {
            RunStatus::Ok
        } else if self.ok > 0 {
            RunStatus::Partial
        } else {
            RunStatus::Failed
        }
    }
}

/// Per-status run counts across a snippet's history. Feeds the TRACK RECORD
/// summary line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunTally {
    pub ok: usize,
    pub partial: usize,
    pub failed: usize,
}

impl RunTally {
    /// Tally `records` by [`RunStatus`].
    pub fn of(records: &[RunRecord]) -> Self {
        let mut t = Self::default();
        for r in records {
            match r.status() {
                RunStatus::Ok => t.ok += 1,
                RunStatus::Partial => t.partial += 1,
                RunStatus::Failed => t.failed += 1,
            }
        }
        t
    }
}

/// The run ledger. `runs[name]` holds the recent runs for a snippet, oldest
/// first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetRunLog {
    pub runs: HashMap<String, Vec<RunRecord>>,
}

fn log_path(paths: Option<&Paths>) -> Option<std::path::PathBuf> {
    paths.map(|p| p.snippet_runs())
}

impl SnippetRunLog {
    /// Read the ledger from disk. Missing files yield an empty ledger. A
    /// corrupt file is renamed to `<path>.corrupt-<unix_ts>` before defaulting.
    pub fn load(paths: Option<&Paths>) -> Self {
        let Some(path) = log_path(paths) else {
            return Self::default();
        };
        fs_util::read_json_recovering::<Self>(&path, crate::key_activity::now_secs())
            .unwrap_or_default()
    }

    /// Build a run record from a host tally, append it and flush to disk.
    /// Shared by the in-TUI and terminal-mode run-completion sites so the
    /// record-build + log + flush pattern lives in one place. Logs the record
    /// on success and any flush error with the `[config]` fault tag.
    pub fn record_run_and_flush(
        &mut self,
        name: &str,
        hosts: usize,
        ok: usize,
        now: u64,
        paths: Option<&Paths>,
    ) {
        let failed = hosts.saturating_sub(ok);
        debug!(
            "[purple] snippet run recorded: name={name:?} hosts={hosts} ok={ok} failed={failed}"
        );
        self.record(
            name,
            RunRecord {
                ts: now,
                hosts,
                ok,
                failed,
            },
        );
        if let Err(e) = self.flush(paths) {
            warn!("[config] snippet_runs flush failed: {e}");
        }
    }

    /// Append a run for `name`, capping the per-snippet history to the most
    /// recent `MAX_RUNS_PER_SNIPPET`.
    pub fn record(&mut self, name: &str, record: RunRecord) {
        let entry = self.runs.entry(name.to_string()).or_default();
        entry.push(record);
        if entry.len() > MAX_RUNS_PER_SNIPPET {
            let overflow = entry.len() - MAX_RUNS_PER_SNIPPET;
            entry.drain(0..overflow);
        }
    }

    /// Move the run history from `old` to `new` when a snippet is renamed, so a
    /// rename keeps its TRACK RECORD verdict and trend chart. No-op when
    /// `old` has no history. Any history already under `new` is overwritten.
    pub fn rename(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        if let Some(history) = self.runs.remove(old) {
            self.runs.insert(new.to_string(), history);
        }
    }

    /// Drop a snippet's entire run history, called when the snippet is deleted
    /// so a recreated name starts with a clean TRACK RECORD instead of
    /// inheriting the deleted snippet's verdict and trend chart. No-op when the
    /// name has no history.
    pub fn remove(&mut self, name: &str) {
        self.runs.remove(name);
    }

    /// Serialize to JSON and write atomically. Suppressed in demo mode so
    /// `--demo` never mutates the user's real ledger.
    pub fn flush(&self, paths: Option<&Paths>) -> io::Result<()> {
        if crate::demo_flag::is_demo() {
            debug!(
                "[purple] snippet_runs: demo mode, skipping disk flush ({} snippets held in memory)",
                self.runs.len(),
            );
            return Ok(());
        }
        let Some(path) = log_path(paths) else {
            return Ok(());
        };
        fs_util::write_json_pretty(&path, self)
    }

    /// Recent runs for `name`, oldest first. Empty slice when never run.
    pub fn for_snippet(&self, name: &str) -> &[RunRecord] {
        self.runs.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Total recorded runs for `name`.
    pub fn run_count(&self, name: &str) -> usize {
        self.for_snippet(name).len()
    }

    /// Most recent run for `name`.
    pub fn last_run(&self, name: &str) -> Option<&RunRecord> {
        self.for_snippet(name).last()
    }

    /// Success rate across all recorded host-executions for `name`, as a
    /// fraction in `0.0..=1.0`. `None` when there are no recorded attempts.
    pub fn success_rate(&self, name: &str) -> Option<f64> {
        let runs = self.for_snippet(name);
        let attempts: usize = runs.iter().map(|r| r.hosts).sum();
        if attempts == 0 {
            return None;
        }
        let ok: usize = runs.iter().map(|r| r.ok).sum();
        Some(ok as f64 / attempts as f64)
    }
}

#[cfg(test)]
#[path = "snippet_runs_tests.rs"]
mod tests;
