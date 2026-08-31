use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use log::warn;

use crate::fs_util;
use crate::runtime::env::Paths;

/// Timestamps older than this are pruned on load and after each record().
const RETENTION_SECS: u64 = 365 * 86400;

/// Hard cap on stored timestamps per host to bound memory and serialisation cost.
const MAX_TIMESTAMPS: usize = 10_000;

/// A single history entry for a host.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub alias: String,
    pub last_connected: u64,
    pub count: u32,
    /// Individual connection timestamps (last 365 days) for activity charts.
    pub timestamps: Vec<u64>,
}

/// Connection history tracking.
#[derive(Debug, Clone, Default)]
pub struct ConnectionHistory {
    entries: HashMap<String, HistoryEntry>,
    path: PathBuf,
}

impl ConnectionHistory {
    /// Load connection history from disk.
    pub fn load(paths: Option<&Paths>) -> Self {
        let path = match Self::history_path(paths) {
            Some(p) => p,
            None => return Self::default(),
        };
        if !path.exists() {
            return Self {
                entries: HashMap::new(),
                path,
            };
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!("[config] Failed to read connection history: {e}");
                }
                return Self {
                    entries: HashMap::new(),
                    path,
                };
            }
        };
        let mut entries = HashMap::new();
        for line in content.lines() {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() >= 3
                && let (Ok(ts), Ok(count)) = (parts[1].parse::<u64>(), parts[2].parse::<u32>())
            {
                let timestamps = if parts.len() == 4 && !parts[3].is_empty() {
                    parts[3]
                        .split(',')
                        .filter_map(|s| s.parse::<u64>().ok())
                        .collect()
                } else {
                    Vec::new()
                };
                entries.insert(
                    parts[0].to_string(),
                    HistoryEntry {
                        alias: parts[0].to_string(),
                        last_connected: ts,
                        count,
                        timestamps,
                    },
                );
            }
        }
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(RETENTION_SECS);
        for entry in entries.values_mut() {
            entry.timestamps.retain(|&t| t >= cutoff);
            if entry.timestamps.len() > MAX_TIMESTAMPS {
                let excess = entry.timestamps.len() - MAX_TIMESTAMPS;
                entry.timestamps.drain(..excess);
            }
        }
        Self { entries, path }
    }

    /// Create a ConnectionHistory from pre-built entries (for demo use).
    pub fn from_entries(entries: HashMap<String, HistoryEntry>) -> Self {
        Self {
            entries,
            path: PathBuf::new(),
        }
    }

    pub fn entries(&self) -> &HashMap<String, HistoryEntry> {
        &self.entries
    }

    pub fn entry(&self, alias: &str) -> Option<&HistoryEntry> {
        self.entries.get(alias)
    }

    pub fn upsert_entry(&mut self, entry: HistoryEntry) {
        self.entries.insert(entry.alias.clone(), entry);
    }

    /// Record a connection to a host.
    pub fn record(&mut self, alias: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let entry = self
            .entries
            .entry(alias.to_string())
            .or_insert(HistoryEntry {
                alias: alias.to_string(),
                last_connected: 0,
                count: 0,
                timestamps: Vec::new(),
            });
        entry.last_connected = now;
        entry.count = entry.count.saturating_add(1);
        entry.timestamps.push(now);
        let cutoff = now.saturating_sub(RETENTION_SECS);
        entry.timestamps.retain(|&t| t >= cutoff);
        if entry.timestamps.len() > MAX_TIMESTAMPS {
            let excess = entry.timestamps.len() - MAX_TIMESTAMPS;
            entry.timestamps.drain(..excess);
        }
        if let Err(e) = self.save() {
            warn!("[config] Failed to save connection history: {e}");
        }
    }

    /// Move a host's entry from `old_alias` to `new_alias`. Called from the
    /// host-form rename path so connection counts and timestamps survive a
    /// rename. When both keys carry entries (defensive, should not occur in
    /// practice because SSH config writes reject collisions) the two are
    /// merged: counts sum, the most recent `last_connected` wins, and the
    /// timestamp lists are concatenated then pruned by the same retention
    /// and cap rules used on load.
    ///
    /// Returns `true` when the file changed.
    pub fn rename(&mut self, old_alias: &str, new_alias: &str) -> bool {
        if old_alias == new_alias {
            return false;
        }
        let Some(mut moved) = self.entries.remove(old_alias) else {
            return false;
        };
        moved.alias = new_alias.to_string();
        if let Some(existing) = self.entries.remove(new_alias) {
            moved.count = moved.count.saturating_add(existing.count);
            moved.last_connected = moved.last_connected.max(existing.last_connected);
            moved.timestamps.extend(existing.timestamps);
            moved.timestamps.sort_unstable();
            moved.timestamps.dedup();
            let cutoff = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(RETENTION_SECS);
            moved.timestamps.retain(|&t| t >= cutoff);
            if moved.timestamps.len() > MAX_TIMESTAMPS {
                let excess = moved.timestamps.len() - MAX_TIMESTAMPS;
                moved.timestamps.drain(..excess);
            }
        }
        self.entries.insert(new_alias.to_string(), moved);
        if let Err(e) = self.save() {
            warn!("[config] Failed to save connection history after rename: {e}");
        }
        true
    }

    /// Last connected timestamp for a host (0 if never connected).
    pub fn last_connected(&self, alias: &str) -> u64 {
        self.entries.get(alias).map_or(0, |e| e.last_connected)
    }

    /// Frecency score: count weighted by recency.
    pub fn frecency_score(&self, alias: &str) -> f64 {
        let entry = match self.entries.get(alias) {
            Some(e) => e,
            None => return 0.0,
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let age_hours = (now.saturating_sub(entry.last_connected)) as f64 / 3600.0;
        let recency = 1.0 / (1.0 + age_hours / 24.0);
        entry.count as f64 * recency
    }

    /// Format a timestamp as a human-readable "time ago" string.
    pub fn format_time_ago(timestamp: u64) -> String {
        if timestamp == 0 {
            return String::new();
        }
        // In demo mode read from a frozen reference clock so visual goldens
        // do not flake when render time straddles a minute boundary after
        // demo-data build time.
        let now = if crate::demo_flag::is_demo() {
            crate::demo_flag::now_secs()
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        };
        let diff = now.saturating_sub(timestamp);
        if diff < 60 {
            "<1m".to_string()
        } else if diff < 3600 {
            format!("{}m", diff / 60)
        } else if diff < 86400 {
            format!("{}h", diff / 3600)
        } else if diff < 604800 {
            format!("{}d", diff / 86400)
        } else {
            format!("{}w", diff / 604800)
        }
    }

    fn save(&self) -> std::io::Result<()> {
        if crate::demo_flag::is_demo() {
            return Ok(());
        }
        // Sort by alias for deterministic output
        let mut sorted: Vec<_> = self.entries.values().collect();
        sorted.sort_by(|a, b| a.alias.cmp(&b.alias));
        let mut content = String::new();
        for (i, e) in sorted.iter().enumerate() {
            if i > 0 {
                content.push('\n');
            }
            content.push_str(&e.alias);
            content.push('\t');
            content.push_str(&e.last_connected.to_string());
            content.push('\t');
            content.push_str(&e.count.to_string());
            if !e.timestamps.is_empty() {
                content.push('\t');
                let ts_strs: Vec<String> = e.timestamps.iter().map(|t| t.to_string()).collect();
                content.push_str(&ts_strs.join(","));
            }
        }
        if !content.is_empty() {
            content.push('\n');
        }
        fs_util::atomic_write(&self.path, content.as_bytes())
    }

    fn history_path(paths: Option<&Paths>) -> Option<PathBuf> {
        paths.map(Paths::history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frecency_score_unknown_alias() {
        let history = ConnectionHistory::default();
        assert_eq!(history.frecency_score("unknown"), 0.0);
    }

    #[test]
    fn test_format_time_ago_zero() {
        assert_eq!(ConnectionHistory::format_time_ago(0), "");
    }

    #[test]
    fn test_timestamps_parsing_roundtrip() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let tsv = format!(
            "myhost\t{}\t5\t{},{},{}",
            now,
            now - 100,
            now - 200,
            now - 300
        );
        let dir = std::env::temp_dir().join(format!(
            "purple_test_history_{:?}",
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.tsv");
        std::fs::write(&path, &tsv).unwrap();

        let mut history = ConnectionHistory {
            entries: HashMap::new(),
            path: path.clone(),
        };
        let content = std::fs::read_to_string(&path).unwrap();
        for line in content.lines() {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() >= 3
                && let (Ok(ts), Ok(count)) = (parts[1].parse::<u64>(), parts[2].parse::<u32>())
            {
                let timestamps = if parts.len() == 4 && !parts[3].is_empty() {
                    parts[3]
                        .split(',')
                        .filter_map(|s| s.parse::<u64>().ok())
                        .collect()
                } else {
                    Vec::new()
                };
                history.entries.insert(
                    parts[0].to_string(),
                    HistoryEntry {
                        alias: parts[0].to_string(),
                        last_connected: ts,
                        count,
                        timestamps,
                    },
                );
            }
        }

        let entry = history.entries.get("myhost").unwrap();
        assert_eq!(entry.count, 5);
        assert_eq!(entry.timestamps.len(), 3);
        assert_eq!(entry.timestamps[0], now - 100);

        // Save and reload to verify roundtrip
        history.save().unwrap();
        let reloaded = std::fs::read_to_string(&path).unwrap();
        assert!(reloaded.contains("myhost"));
        assert!(reloaded.contains(&(now - 100).to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_timestamps_retention_prunes_old() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let old = now - 400 * 86400; // 400 days ago — beyond 365-day retention
        let recent = now - 10 * 86400; // 10 days ago — within retention

        let dir = std::env::temp_dir().join(format!(
            "purple_test_retention_{:?}",
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.tsv");
        let tsv = format!("host1\t{}\t2\t{},{}", now, old, recent);
        std::fs::write(&path, &tsv).unwrap();

        // Simulate load with retention pruning
        let mut entries = HashMap::new();
        let cutoff = now.saturating_sub(RETENTION_SECS);
        entries.insert(
            "host1".to_string(),
            HistoryEntry {
                alias: "host1".to_string(),
                last_connected: now,
                count: 2,
                timestamps: vec![old, recent],
            },
        );
        for entry in entries.values_mut() {
            entry.timestamps.retain(|&t| t >= cutoff);
        }

        let entry = entries.get("host1").unwrap();
        assert_eq!(entry.timestamps.len(), 1, "old timestamp should be pruned");
        assert_eq!(entry.timestamps[0], recent);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_timestamps_cap() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut timestamps: Vec<u64> = (0..MAX_TIMESTAMPS + 500)
            .map(|i| now - (i as u64))
            .collect();
        timestamps.sort();

        let cutoff = now.saturating_sub(RETENTION_SECS);
        timestamps.retain(|&t| t >= cutoff);
        if timestamps.len() > MAX_TIMESTAMPS {
            let excess = timestamps.len() - MAX_TIMESTAMPS;
            timestamps.drain(..excess);
        }

        assert!(timestamps.len() <= MAX_TIMESTAMPS);
        // Should keep the most recent timestamps
        assert_eq!(*timestamps.last().unwrap(), now);
    }

    #[test]
    fn test_retention_keeps_nine_months() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let nine_months = now - 270 * 86400;
        let six_months = now - 180 * 86400;
        let recent = now - 86400;

        let cutoff = now.saturating_sub(RETENTION_SECS);
        let mut timestamps = vec![nine_months, six_months, recent];
        timestamps.retain(|&t| t >= cutoff);

        assert_eq!(
            timestamps.len(),
            3,
            "9-month-old timestamps must be retained"
        );
        assert_eq!(timestamps[0], nine_months);
    }

    #[test]
    fn test_retention_prunes_beyond_one_year() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let thirteen_months = now - 400 * 86400;
        let recent = now - 86400;

        let cutoff = now.saturating_sub(RETENTION_SECS);
        let mut timestamps = vec![thirteen_months, recent];
        timestamps.retain(|&t| t >= cutoff);

        assert_eq!(timestamps.len(), 1, "13-month-old timestamp must be pruned");
        assert_eq!(timestamps[0], recent);
    }

    #[test]
    fn test_timestamps_empty_fourth_column() {
        // A 3-column line (no timestamps) should parse with empty timestamps
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let line = format!("oldhost\t{}\t10", now);
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        assert_eq!(parts.len(), 3);
        let timestamps: Vec<u64> = if parts.len() == 4 && !parts[3].is_empty() {
            parts[3]
                .split(',')
                .filter_map(|s| s.parse::<u64>().ok())
                .collect()
        } else {
            Vec::new()
        };
        assert!(timestamps.is_empty());
    }

    #[test]
    fn test_format_time_ago_recent() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(ConnectionHistory::format_time_ago(now), "<1m");
        assert_eq!(ConnectionHistory::format_time_ago(now - 300), "5m");
        assert_eq!(ConnectionHistory::format_time_ago(now - 7200), "2h");
        assert_eq!(ConnectionHistory::format_time_ago(now - 172800), "2d");
    }

    fn make_entry(alias: &str, last: u64, count: u32, timestamps: Vec<u64>) -> HistoryEntry {
        HistoryEntry {
            alias: alias.to_string(),
            last_connected: last,
            count,
            timestamps,
        }
    }

    #[test]
    fn rename_moves_entry_under_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.tsv");
        let mut history = ConnectionHistory {
            entries: HashMap::new(),
            path: path.clone(),
        };
        let now = 1_700_000_000;
        history.entries.insert(
            "web-old".to_string(),
            make_entry("web-old", now, 7, vec![now - 60, now]),
        );

        assert!(history.rename("web-old", "web-new"));
        assert!(!history.entries.contains_key("web-old"));
        let moved = history.entries.get("web-new").expect("entry under new key");
        assert_eq!(moved.alias, "web-new");
        assert_eq!(moved.count, 7);
        assert_eq!(moved.last_connected, now);
        assert_eq!(moved.timestamps, vec![now - 60, now]);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with("web-new\t"));
        assert!(!saved.contains("web-old"));
    }

    #[test]
    fn rename_merges_when_new_key_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.tsv");
        let mut history = ConnectionHistory {
            entries: HashMap::new(),
            path,
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        history.entries.insert(
            "a".to_string(),
            make_entry("a", now - 100, 3, vec![now - 200, now - 100]),
        );
        history.entries.insert(
            "b".to_string(),
            make_entry("b", now - 50, 5, vec![now - 100, now - 50]),
        );

        assert!(history.rename("a", "b"));
        let merged = history.entries.get("b").expect("merged entry");
        assert_eq!(merged.count, 8, "counts sum on collision");
        assert_eq!(
            merged.last_connected,
            now - 50,
            "most recent timestamp wins"
        );
        // Shared `now - 100` timestamp must be deduplicated.
        assert_eq!(merged.timestamps, vec![now - 200, now - 100, now - 50]);
        assert!(!history.entries.contains_key("a"));
    }

    #[test]
    fn rename_noop_when_same_alias() {
        let mut history = ConnectionHistory::default();
        history
            .entries
            .insert("a".to_string(), make_entry("a", 1, 1, vec![1]));
        assert!(!history.rename("a", "a"));
        assert!(history.entries.contains_key("a"));
    }

    #[test]
    fn rename_noop_when_old_absent() {
        let mut history = ConnectionHistory::default();
        assert!(!history.rename("ghost", "phantom"));
        assert!(history.entries.is_empty());
    }
}
