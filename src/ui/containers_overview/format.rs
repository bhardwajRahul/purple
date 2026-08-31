use super::*;

/// Format `HostConfig.Memory` bytes as a panel value (`512 MB`, `1.5 GB`).
/// Sub-megabyte values render as raw bytes since they only happen on
/// pathological misconfigurations.
pub(crate) fn format_memory_bytes(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        let g = bytes as f64 / GB as f64;
        if g.fract().abs() < 0.05 {
            format!("{:.0} GB", g)
        } else {
            format!("{:.1} GB", g)
        }
    } else if bytes >= MB {
        let m = bytes as f64 / MB as f64;
        format!("{:.0} MB", m)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Format `NanoCpus` as a fractional core count (`1.5 cores`). Whole-core
/// values drop the trailing `.0`.
pub(crate) fn format_cpu_nanos(nanos: u64) -> String {
    let cores = nanos as f64 / 1_000_000_000.0;
    if cores.fract().abs() < 0.05 {
        format!("{:.0} cores", cores)
    } else {
        format!("{:.1} cores", cores)
    }
}

/// Format a nanosecond duration as the largest natural unit (`30s`, `5m`,
/// `2h`). Used for healthcheck intervals.
pub(crate) fn format_duration_ns(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    if secs >= 3600 {
        format!("{}h", secs / 3600)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else if secs > 0 {
        format!("{}s", secs)
    } else {
        format!("{}ms", ns / 1_000_000)
    }
}

/// Format a `Healthcheck.Test` array into a readable command. Docker
/// stores it as `["CMD", "curl", "-fs", url]` or `["CMD-SHELL", "..."]`.
/// `["NONE"]` means the image disabled the inherited healthcheck.
pub(crate) fn format_health_test(test: &[String]) -> String {
    if test.is_empty() {
        return String::new();
    }
    match test[0].as_str() {
        "CMD" | "CMD-SHELL" => test[1..].join(" "),
        "NONE" => "disabled".to_string(),
        _ => test.join(" "),
    }
}

/// Convert a docker/podman RFC3339 timestamp (`2026-05-09T08:00:00.123Z`)
/// into the compact "YYYY-MM-DD HH:MM:SS" form the detail panel renders.
/// Returns `None` for empty strings and the Go zero-time
/// `0001-01-01T00:00:00Z` that docker emits when a field is unset.
pub(crate) fn format_iso_timestamp(s: &str) -> Option<String> {
    if s.is_empty() || s.starts_with("0001-") {
        return None;
    }
    let main = s.split('.').next().unwrap_or(s);
    let main = main.trim_end_matches('Z');
    Some(main.replace('T', " "))
}

/// Truncate a path-like string to fit the 48-column detail panel,
/// preserving the right-hand segment (the leaf is usually more
/// informative than the prefix). Reuse for any panel value that may
/// exceed the column budget.
pub(crate) fn truncate_panel_value(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut = max.saturating_sub(2);
    let take_from_end = cut;
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(take_from_end);
    let suffix: String = chars[start..].iter().collect();
    format!("…{}", suffix)
}

/// Snap a row count down to the latest card-close (`╰`) within `cap`
/// so the panel never clips a card mid-content.
pub(crate) fn snap_top_to_card_boundary(lines: &[Line<'static>], cap: u16) -> u16 {
    let cap_us = cap as usize;
    // Walk backwards looking for ╰ (close border) lines. Each one
    // sits at index `i`; including it means keeping `i + 1` rows.
    let mut best: Option<usize> = None;
    for (i, line) in lines.iter().enumerate().take(cap_us) {
        let starts_with_close = line
            .spans
            .first()
            .map(|s| s.content.starts_with('\u{2570}'))
            .unwrap_or(false);
        if starts_with_close {
            best = Some(i + 1);
        }
    }
    best.map(|n| n as u16).unwrap_or(cap)
}

/// Hard-wrap on character boundary onto up to `max_lines` lines of
/// `width` cols; trailing `…` when the input still overflows.
pub(crate) fn wrap_to_lines(s: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() && out.len() < max_lines {
        let remaining = chars.len() - start;
        let take = remaining.min(width);
        let is_last_slot = out.len() + 1 == max_lines;
        let overflows = is_last_slot && take < remaining;
        let end = start + take;
        let chunk: String = if overflows {
            // Reserve one column for the trailing `…`.
            let cut = end.saturating_sub(1);
            let mut s: String = chars[start..cut].iter().collect();
            s.push('\u{2026}');
            s
        } else {
            chars[start..end].iter().collect()
        };
        out.push(chunk);
        start = end;
    }
    out
}

/// Pad-or-truncate a path-like string to `w` columns, preserving the
/// leaf via left-truncation. Sibling of `pad_or_truncate` which
/// right-truncates; both produce exactly `w` cols of output.
pub(crate) fn pad_or_truncate_path(s: &str, w: usize) -> String {
    let cur = s.width();
    if cur == w {
        return s.to_string();
    }
    if cur < w {
        return format!("{}{}", s, " ".repeat(w - cur));
    }
    let truncated = truncate_panel_value(s, w);
    let tw = truncated.width();
    if tw < w {
        format!("{}{}", truncated, " ".repeat(w - tw))
    } else {
        truncated
    }
}

/// Compress a long sha256 image digest into `sha256:abcdef…1234` so
/// the 48-col panel has space for the row alignment without dropping
/// the prefix that identifies the algorithm.
pub(crate) fn short_digest(d: &str) -> String {
    if let Some(hex) = d.strip_prefix("sha256:")
        && hex.len() > 12
    {
        return format!("sha256:{}…{}", &hex[..6], &hex[hex.len() - 4..]);
    }
    d.to_string()
}

/// Trim a 64-char container ID to the 12-char short form `docker ps`
/// shows. Leaves shorter IDs untouched so test fixtures stay readable.
pub(crate) fn short_id(id: &str) -> String {
    if id.len() <= 12 {
        id.to_string()
    } else {
        id[..12].to_string()
    }
}
