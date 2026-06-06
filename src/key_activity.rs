//! Per-host SSH connection activity log.
//!
//! Records `(alias, timestamp)` events each time purple opens an SSH
//! session, exec command or tunnel for a host. Persisted to
//! `~/.purple/key_activity.json`. The Keys tab reads this log to render
//! per-key sparklines, last-touch hints and "hosts touched in last 30d"
//! metrics by pivoting events through `SshKeyInfo::linked_hosts` at
//! render time. We log per alias rather than per key fingerprint so we
//! never have to attribute connects to a specific key file; the alias
//! mapping already encodes the link.

use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use log::debug;
use serde::{Deserialize, Serialize};

use crate::fs_util;
use crate::runtime::env::Paths;

/// Retention window for events. Older rows are dropped on load and on
/// every record so the file does not grow unbounded. 90 days is the
/// longest range any rendered widget needs (30d sparkline reads the
/// most recent month, "last touch" reads the most recent of any age).
const RETENTION_DAYS: u64 = 90;

const SECS_PER_DAY: u64 = 86_400;

/// Fixed reference timestamp used by demo data seeding and by
/// render-time helpers that need a deterministic "now". Picked at the
/// cutover date so visual goldens render deterministically; the date
/// itself only matters in concert with the timestamps demo.rs seeds.
pub const DEMO_NOW_SECS: u64 = 1_778_932_800; // 2026-05-16 12:00:00 UTC

fn activity_path(paths: Option<&Paths>) -> Option<PathBuf> {
    paths.map(Paths::key_activity)
}

/// Current wall-clock epoch seconds. Demo-aware rendering uses
/// `now_for_render()` instead, which substitutes `DEMO_NOW_SECS` so
/// sparkline rendering stays byte-stable across golden runs.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Demo-aware "now" for render-time callers. Returns the frozen
/// `DEMO_NOW_SECS` when the process is in demo mode (so visual goldens
/// land byte-stable), otherwise the wall clock. Record-time callers
/// must use `now_secs()` directly and pass the result through; mixing
/// the two would let a render-time freeze leak into persisted events.
pub fn now_for_render() -> u64 {
    if crate::demo_flag::is_demo() {
        DEMO_NOW_SECS
    } else {
        now_secs()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectEvent {
    pub alias: String,
    /// Seconds since UNIX epoch.
    pub ts: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyActivityLog {
    pub events: Vec<ConnectEvent>,
}

impl KeyActivityLog {
    /// Read the log from disk, pruning anything past the retention window.
    /// Missing files yield an empty log. Corrupt files are renamed aside
    /// to `<path>.corrupt-<unix_ts>` before defaulting so a future
    /// debugger can recover the data.
    pub fn load(paths: Option<&Paths>) -> Self {
        let Some(path) = activity_path(paths) else {
            return Self::default();
        };
        match crate::fs_util::read_json_recovering::<Self>(&path, now_secs()) {
            Some(mut log) => {
                log.prune(now_secs());
                log
            }
            None => Self::default(),
        }
    }

    /// Append an event for `alias` at the supplied `now` timestamp.
    /// Prunes anything past the retention window using the same `now`
    /// so the prune cutoff matches the recorded event. Caller decides
    /// whether to flush; production call sites pass `now_secs()`.
    pub fn record(&mut self, alias: &str, now: u64) {
        self.events.push(ConnectEvent {
            alias: alias.to_string(),
            ts: now,
        });
        self.prune(now);
    }

    fn prune(&mut self, now: u64) {
        let cutoff = now.saturating_sub(RETENTION_DAYS * SECS_PER_DAY);
        self.events.retain(|e| e.ts >= cutoff);
    }

    /// Serialize to JSON and write atomically. Suppressed in demo mode so
    /// `--demo` never mutates the user's real activity log. The path is
    /// resolved from the injected `paths`; a `None` (no home) silently
    /// skips the write. The demo-suppress branch logs intent so
    /// `--demo --verbose` shows that recording is happening, just not
    /// landing on disk.
    pub fn flush(&self, paths: Option<&Paths>) -> io::Result<()> {
        if crate::demo_flag::is_demo() {
            debug!(
                "[purple] key_activity: demo mode, skipping disk flush ({} events held in memory)",
                self.events.len(),
            );
            return Ok(());
        }
        let Some(path) = activity_path(paths) else {
            return Ok(());
        };
        fs_util::write_json_pretty(&path, self)
    }

    /// One-shot record. For non-TUI call sites (CLI mode) that do not
    /// hold an in-memory log between connects. Caller passes `now`;
    /// production CLI paths pass `now_secs()`.
    pub fn record_oneshot(alias: &str, now: u64, paths: Option<&Paths>) {
        let mut log = Self::load(paths);
        log.record(alias, now);
        if let Err(e) = log.flush(paths) {
            debug!("[purple] key_activity: flush failed: {e}");
        }
    }

    /// Timestamp of the most recent event whose alias appears in `aliases`.
    pub fn last_use_for_aliases(&self, aliases: &[String]) -> Option<u64> {
        let lookup = alias_set(aliases);
        self.events
            .iter()
            .filter(|e| lookup.contains(e.alias.as_str()))
            .map(|e| e.ts)
            .max()
    }

    /// All event timestamps for the given aliases, used by the shared
    /// activity chart renderer which auto-scales the time window from
    /// the oldest entry.
    pub fn timestamps_for_aliases(&self, aliases: &[String]) -> Vec<u64> {
        let lookup = alias_set(aliases);
        self.events
            .iter()
            .filter(|e| lookup.contains(e.alias.as_str()))
            .map(|e| e.ts)
            .collect()
    }
}

/// Field-disjoint helper: record + flush the activity log without
/// holding `&mut App`. Lets the event loop record a connect while
/// another sub-state (FileBrowser, TunnelState) still holds a mutable
/// borrow on `App`, where the `App::record_key_use` method would be
/// rejected by the borrow checker. Caller passes `now`; production
/// call sites pass `now_secs()`.
pub fn record_and_flush(log: &mut KeyActivityLog, alias: &str, now: u64, paths: Option<&Paths>) {
    log.record(alias, now);
    if let Err(e) = log.flush(paths) {
        debug!("[purple] key_activity: flush failed: {e}");
    }
}

/// Build a `HashSet<&str>` lookup from an alias slice. Used once per
/// query so per-event membership check is O(1) instead of O(aliases).
fn alias_set(aliases: &[String]) -> std::collections::HashSet<&str> {
    aliases.iter().map(String::as_str).collect()
}

/// Format the gap between `now` and `ts` as a compact `Nu ago` label
/// (`N` count, `u` unit). Mirrors the rhythm Linear / GitHub use:
/// `just now`, `14m ago`, `3h ago`, `2d ago`, `3w ago`, `2mo ago`,
/// `1y ago`.
pub fn humanize_last_use(now: u64, ts: u64) -> String {
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        return "just now".to_string();
    }
    let minutes = diff / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d ago");
    }
    let weeks = days / 7;
    if weeks < 5 {
        return format!("{weeks}w ago");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo ago");
    }
    let years = days / 365;
    format!("{years}y ago")
}

/// Format a file mtime as `YYYY-MM-DD (<age> ago)` for the Created
/// label. Uses `humanize_last_use` for the age tail so the rhythm
/// matches the Last touch tile.
pub fn format_created(now: u64, mtime_ts: u64) -> String {
    let date = format_yyyy_mm_dd(mtime_ts);
    let age = humanize_last_use(now, mtime_ts);
    format!("{date} ({age})")
}

/// `YYYY-MM-DD` from a UNIX timestamp using the proleptic Gregorian
/// calendar. Avoids pulling in `chrono` just for one date format.
fn format_yyyy_mm_dd(ts: u64) -> String {
    let days_since_epoch = (ts / SECS_PER_DAY) as i64;
    let (y, m, d) = civil_from_days(days_since_epoch);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert "days since 1970-01-01" to proleptic Gregorian (year, month,
/// day). Algorithm from Howard Hinnant's date library; bounded constant
/// time, no allocations, works for any reasonable timestamp.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-crate lock: shares `demo_flag::GLOBAL_TEST_LOCK` with the
    /// preferences and visual regression suites. `now_secs()` no longer
    /// touches the demo flag, but `flush()` still early-returns when
    /// demo mode is active, so a concurrent test flipping the flag
    /// between `record()` and `flush()` would silently suppress the
    /// write. The mutex serialises every test that exercises that path.
    fn setup() -> (tempfile::TempDir, Paths, std::sync::MutexGuard<'static, ()>) {
        let guard = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::new(dir.path());
        (dir, paths, guard)
    }

    #[test]
    fn record_appends_event() {
        let (_g, _paths, _lock) = setup();
        let mut log = KeyActivityLog::default();
        log.record("prod-eu1", now_secs());
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.events[0].alias, "prod-eu1");
    }

    #[test]
    fn record_prunes_events_past_retention() {
        let (_g, _paths, _lock) = setup();
        let mut log = KeyActivityLog::default();
        let now = now_secs();
        let very_old = now - (RETENTION_DAYS + 10) * SECS_PER_DAY;
        log.events.push(ConnectEvent {
            alias: "ancient".into(),
            ts: very_old,
        });
        log.record("fresh", now);
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.events[0].alias, "fresh");
    }

    #[test]
    fn load_after_flush_roundtrips() {
        let (_g, paths, _lock) = setup();
        let mut log = KeyActivityLog::default();
        let now = now_secs();
        log.record("eric-bastion", now);
        log.record("aws-api-prod", now);
        log.flush(Some(&paths)).unwrap();
        let reloaded = KeyActivityLog::load(Some(&paths));
        assert_eq!(reloaded.events.len(), 2);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let (_g, paths, _lock) = setup();
        let log = KeyActivityLog::load(Some(&paths));
        assert!(log.events.is_empty());
    }

    #[test]
    fn last_use_returns_most_recent() {
        let (_g, _paths, _lock) = setup();
        let mut log = KeyActivityLog::default();
        log.events.push(ConnectEvent {
            alias: "h".into(),
            ts: 100,
        });
        log.events.push(ConnectEvent {
            alias: "h".into(),
            ts: 500,
        });
        log.events.push(ConnectEvent {
            alias: "h".into(),
            ts: 300,
        });
        let aliases = vec!["h".to_string()];
        assert_eq!(log.last_use_for_aliases(&aliases), Some(500));
    }

    #[test]
    fn last_use_none_for_no_matches() {
        let (_g, _paths, _lock) = setup();
        let log = KeyActivityLog::default();
        let aliases = vec!["nobody".to_string()];
        assert!(log.last_use_for_aliases(&aliases).is_none());
    }

    #[test]
    fn humanize_last_use_buckets() {
        assert_eq!(humanize_last_use(1000, 999), "just now");
        assert_eq!(humanize_last_use(1000, 600), "6m ago");
        assert_eq!(humanize_last_use(SECS_PER_DAY * 2, 0), "2d ago");
        assert_eq!(humanize_last_use(SECS_PER_DAY * 14, 0), "2w ago");
        assert_eq!(humanize_last_use(SECS_PER_DAY * 60, 0), "2mo ago");
        assert_eq!(humanize_last_use(SECS_PER_DAY * 400, 0), "1y ago");
    }

    #[test]
    fn record_oneshot_persists_to_disk() {
        let (_g, paths, _lock) = setup();
        KeyActivityLog::record_oneshot("h1", now_secs(), Some(&paths));
        let reloaded = KeyActivityLog::load(Some(&paths));
        assert_eq!(reloaded.events.len(), 1);
        assert_eq!(reloaded.events[0].alias, "h1");
    }

    #[test]
    fn civil_from_days_known_dates() {
        // 1970-01-01 is day 0.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2024-03-12 is 19794 days after 1970-01-01.
        assert_eq!(civil_from_days(19794), (2024, 3, 12));
        // 2026-05-16 is 20589 days after 1970-01-01.
        assert_eq!(civil_from_days(20589), (2026, 5, 16));
    }

    #[test]
    fn format_yyyy_mm_dd_known() {
        // 1778932800 = 2026-05-16 12:00 UTC.
        assert_eq!(format_yyyy_mm_dd(1_778_932_800), "2026-05-16");
        // 1710244800 = 2024-03-12 12:00 UTC.
        assert_eq!(format_yyyy_mm_dd(1_710_244_800), "2024-03-12");
    }

    #[test]
    fn format_created_combines_date_and_age() {
        let now = 1_778_932_800;
        let created = 1_710_244_800; // ~2y 2mo ago
        let out = format_created(now, created);
        assert!(out.starts_with("2024-03-12 ("));
        assert!(out.ends_with(" ago)"));
    }

    // --- Boundary regression tests (added during code review) ---

    #[test]
    fn humanize_boundary_60s_is_1m_not_just_now() {
        assert_eq!(humanize_last_use(1000, 940), "1m ago");
    }

    #[test]
    fn humanize_boundary_exactly_1h() {
        assert_eq!(humanize_last_use(3600, 0), "1h ago");
    }

    #[test]
    fn humanize_boundary_exactly_7d() {
        assert_eq!(humanize_last_use(SECS_PER_DAY * 7, 0), "1w ago");
    }

    #[test]
    fn humanize_boundary_35d_falls_to_months() {
        // weeks=5 short-circuits the weeks branch, so the months bucket
        // takes over. 35 days / 30 = 1 month.
        assert_eq!(humanize_last_use(SECS_PER_DAY * 35, 0), "1mo ago");
    }

    #[test]
    fn prune_keeps_event_at_exactly_retention_boundary() {
        let (_g, _paths, _lock) = setup();
        let now = 200 * SECS_PER_DAY;
        let mut log = KeyActivityLog::default();
        log.events.push(ConnectEvent {
            alias: "edge".into(),
            ts: now - RETENTION_DAYS * SECS_PER_DAY,
        });
        log.prune(now);
        assert_eq!(log.events.len(), 1);
    }

    #[test]
    fn civil_from_days_leap_day_2000() {
        // 2000-02-29 is 11017 days after 1970-01-01.
        assert_eq!(civil_from_days(11016), (2000, 2, 29));
    }

    #[test]
    fn load_corrupt_json_returns_empty_log() {
        let (_g, paths, _lock) = setup();
        let path = paths.key_activity();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not valid json {{").unwrap();
        let log = KeyActivityLog::load(Some(&paths));
        assert!(log.events.is_empty());
    }

    #[test]
    fn load_corrupt_json_preserves_file_under_corrupt_suffix() {
        let (_g, paths, _lock) = setup();
        let path = paths.key_activity();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"definitely not json").unwrap();
        let _ = KeyActivityLog::load(Some(&paths));
        // Original file must be gone.
        assert!(!path.exists(), "corrupt file should have been renamed");
        // A sibling with the .corrupt- suffix must exist with original bytes.
        let preserved: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("key_activity.json.corrupt-")
            })
            .collect();
        assert_eq!(preserved.len(), 1);
        let body = std::fs::read(preserved[0].path()).unwrap();
        assert_eq!(body, b"definitely not json");
    }

    #[test]
    fn flush_in_demo_mode_does_not_write_file() {
        let (_g, paths, _lock) = setup();
        crate::demo_flag::enable();
        let mut log = KeyActivityLog::default();
        log.record("h", now_secs());
        let result = log.flush(Some(&paths));
        crate::demo_flag::disable();

        assert!(result.is_ok());
        let path = paths.key_activity();
        assert!(
            !path.exists(),
            "demo mode must not write the activity log to disk"
        );
    }

    #[test]
    fn now_for_render_returns_demo_constant_in_demo_mode() {
        let (_g, _paths, _lock) = setup();
        crate::demo_flag::enable();
        let n = now_for_render();
        crate::demo_flag::disable();
        assert_eq!(n, DEMO_NOW_SECS);
    }

    #[test]
    fn now_for_render_returns_wall_clock_outside_demo() {
        let (_g, _paths, _lock) = setup();
        // Sanity: outside demo mode the function must NOT freeze at
        // DEMO_NOW_SECS. Compare against now_secs() which the helper
        // delegates to in the wall-clock branch.
        let before = now_secs();
        let n = now_for_render();
        let after = now_secs();
        assert!(n >= before && n <= after);
    }

    #[test]
    fn timestamps_for_aliases_filters_to_matching() {
        let mut log = KeyActivityLog::default();
        log.events.push(ConnectEvent {
            alias: "a".into(),
            ts: 100,
        });
        log.events.push(ConnectEvent {
            alias: "b".into(),
            ts: 200,
        });
        log.events.push(ConnectEvent {
            alias: "a".into(),
            ts: 300,
        });
        let ts = log.timestamps_for_aliases(&["a".to_string()]);
        assert_eq!(ts, vec![100, 300]);
    }
}
