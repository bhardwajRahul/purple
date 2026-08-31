//! Key handler for the one-shot container logs overlay
//! (`Screen::ContainerLogs`). The overlay opens via `l` on the
//! containers tab and shows the last 200 lines of `<runtime> logs`
//! over a single SSH call. There is no live follow; refresh re-fires
//! the same call (`r`).

use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyEvent};

use super::ctx::Nav;
use crate::app::{
    App, ContainerLogsRequest, ContainerLogsSearch, ContainerState, HostState, Screen,
};
use crate::event::AppEvent;

/// Number of trailing log lines requested per fetch. Sized to fit a
/// typical 50-row terminal twice over while keeping the SSH stream
/// bounded.
pub const DEFAULT_TAIL: usize = 200;

/// Scroll value that tail-anchors `body_len` lines in a `viewport_h`-row
/// area. Returns 0 when the body fits in the viewport so short logs
/// render flush-top without leaving blank rows below.
pub(crate) fn tail_scroll(body_len: usize, viewport_h: u16) -> u16 {
    body_len.saturating_sub(viewport_h as usize) as u16
}

/// Scroll value that puts the line at index `line` near the centre of
/// a `viewport_h`-row viewport. Used when Tab/Shift+Tab or live
/// typing moves the cursor between matches: centring keeps surrounding
/// context visible on both sides of the match.
pub(crate) fn center_scroll(line: usize, viewport_h: u16, body_len: usize) -> u16 {
    let half = (viewport_h as usize) / 2;
    let target = line.saturating_sub(half);
    let max = body_len.saturating_sub(viewport_h as usize);
    target.min(max) as u16
}

/// Smart-case match test. Lowercase-only query matches case-
/// insensitively; any uppercase rune flips the search to case-
/// sensitive (vim's `'smartcase'` default).
pub(crate) fn matches_line(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    if needle.chars().any(|c| c.is_uppercase()) {
        haystack.contains(needle)
    } else {
        // Case-insensitive ASCII fast path. Non-ASCII bytes compare
        // verbatim, which is acceptable for log search where the
        // payload is overwhelmingly ASCII.
        ascii_ci_find(haystack.as_bytes(), needle.as_bytes()).is_some()
    }
}

/// Byte indices of every non-overlapping match of `needle` in
/// `haystack` under the smart-case rule. Public so the renderer can
/// highlight matches inline at exact byte boundaries.
pub(crate) fn match_indices_smart(haystack: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    if needle.chars().any(|c| c.is_uppercase()) {
        haystack.match_indices(needle).map(|(idx, _)| idx).collect()
    } else {
        ascii_ci_match_indices(haystack.as_bytes(), needle.as_bytes())
    }
}

fn ascii_ci_find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    ascii_ci_match_indices(hay, needle).into_iter().next()
}

fn ascii_ci_match_indices(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    'outer: while i + needle.len() <= hay.len() {
        for j in 0..needle.len() {
            if !hay[i + j].eq_ignore_ascii_case(&needle[j]) {
                i += 1;
                continue 'outer;
            }
        }
        out.push(i);
        // Non-overlapping: skip past this match. ASCII-only needle
        // means a match never starts inside a UTF-8 continuation
        // byte, so byte positions remain char-boundary-safe.
        i += needle.len();
    }
    out
}

/// Recompute matches against a refreshed body. Public so the event
/// loop can re-sync after `r` lands new lines without exposing the
/// match cache directly.
pub(crate) fn refresh_search(body: &[String], search: &mut ContainerLogsSearch) {
    recompute_matches(body, search);
}

/// Re-centre the viewport on the current match. Public so the event
/// loop can keep the cursor visible after a refresh.
pub(crate) fn recenter_on_match(
    body_len: usize,
    last_render_height: u16,
    search: &ContainerLogsSearch,
    scroll: &mut u16,
) {
    scroll_to_current_match(body_len, last_render_height, search, scroll);
}

/// Recompute `search.matches` against `body` using `search.query`,
/// then reset `current` to 0 so the cursor lands on the first hit.
/// Mirrors `app::search::apply_filter` which also resets the host
/// list selection to the first row after every refining keystroke;
/// keeping logs search consistent with that behaviour means the
/// viewport always scrolls to the first match as you type, instead
/// of trying to preserve a stale `current` index that no longer
/// points where the user was looking.
fn recompute_matches(body: &[String], search: &mut ContainerLogsSearch) {
    search.matches = body
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            if matches_line(line, &search.query) {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    search.current = 0;
}

/// Scroll the viewport so the current match sits near the centre.
/// No-op when the body fits the viewport (max_scroll == 0).
fn scroll_to_current_match(
    body_len: usize,
    last_render_height: u16,
    search: &ContainerLogsSearch,
    scroll: &mut u16,
) {
    if let Some(line) = search.matches.get(search.current) {
        *scroll = center_scroll(*line, last_render_height, body_len);
    }
}

/// The slice of App the container-logs overlay touches: the screen (which
/// carries the logs body, scroll, search and coordinates inside the
/// `ContainerLogs` variant), the container cache/queue and the host list
/// (read-only, for the per-host askpass on refresh). It never reaches into
/// tunnels, providers or any other domain.
struct ContainerLogsCtx<'a> {
    screen: &'a mut Screen,
    container_state: &'a mut ContainerState,
    hosts: &'a HostState,
}

impl Nav for ContainerLogsCtx<'_> {
    fn screen_mut(&mut self) -> &mut Screen {
        self.screen
    }
}

pub(super) fn handle_key(app: &mut App, key: KeyEvent, _events_tx: &mpsc::Sender<AppEvent>) {
    let mut ctx = ContainerLogsCtx {
        screen: &mut app.screen,
        container_state: &mut app.container_state,
        hosts: &app.hosts_state,
    };
    container_logs_key(&mut ctx, key);
}

fn container_logs_key(ctx: &mut ContainerLogsCtx, key: KeyEvent) {
    if !matches!(*ctx.screen, Screen::ContainerLogs) {
        return;
    }
    // State was pruned out from under us (host removed via reload_hosts
    // while the overlay was open). Drop to HostList so the user is not
    // stranded on `Screen::ContainerLogs` with no body and no working
    // Esc.
    if ctx.container_state.logs_view().is_none() {
        ctx.set_screen(Screen::HostList);
        return;
    }
    let view = ctx
        .container_state
        .logs_view_mut()
        .expect("logs_view presence just checked");
    let body = &mut view.body;
    let scroll = &mut view.scroll;
    let alias = &view.alias;
    let container_id = &view.container_id;
    let container_name = &view.container_name;
    let last_render_height = &view.last_render_height;
    let search = &mut view.search;

    // Modeless search: while active, every keystroke either edits the
    // query (chars / cursor / delete) or steps through matches (Tab /
    // Shift+Tab). Esc exits search. Scroll keys are intentionally
    // swallowed; the user navigates results via Tab or quits search
    // with Esc and uses j/k/g/G. Matches are recomputed live.
    if let Some(s) = search.as_mut() {
        match key.code {
            KeyCode::Esc => {
                log::debug!("[purple] container_logs: search closed");
                *search = None;
            }
            KeyCode::Tab if !s.matches.is_empty() => {
                s.current = (s.current + 1) % s.matches.len();
                scroll_to_current_match(body.len(), *last_render_height, s, scroll);
            }
            KeyCode::BackTab if !s.matches.is_empty() => {
                s.current = if s.current == 0 {
                    s.matches.len() - 1
                } else {
                    s.current - 1
                };
                scroll_to_current_match(body.len(), *last_render_height, s, scroll);
            }
            KeyCode::Backspace => {
                s.delete_char_before_cursor();
                recompute_matches(body, s);
                scroll_to_current_match(body.len(), *last_render_height, s, scroll);
            }
            KeyCode::Delete => {
                s.delete_char_at_cursor();
                recompute_matches(body, s);
                scroll_to_current_match(body.len(), *last_render_height, s, scroll);
            }
            KeyCode::Left => s.move_left(),
            KeyCode::Right => s.move_right(),
            KeyCode::Home => s.move_home(),
            KeyCode::End => s.move_end(),
            KeyCode::Char(c) => {
                s.insert_char(c);
                recompute_matches(body, s);
                scroll_to_current_match(body.len(), *last_render_height, s, scroll);
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            log::debug!("[purple] container_logs: closed");
            ctx.container_state.clear_logs_view();
            ctx.set_screen(Screen::HostList);
        }
        KeyCode::Char('?') => {
            // Help dispatcher reads `app.top_page` to pick the
            // tab-specific help; the containers tab block already
            // documents `l logs`. Returning to HostList preserves
            // the tab the user was on.
            ctx.set_screen(Screen::Help {
                return_screen: Box::new(Screen::HostList),
            });
        }
        KeyCode::Char('/') => {
            log::debug!("[purple] container_logs: search opened");
            *search = Some(ContainerLogsSearch::default());
        }
        KeyCode::Char('j') | KeyCode::Down => {
            *scroll = scroll.saturating_add(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            *scroll = scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            *scroll = scroll.saturating_add(20);
        }
        KeyCode::PageUp => {
            *scroll = scroll.saturating_sub(20);
        }
        KeyCode::Char('g') => {
            *scroll = 0;
        }
        KeyCode::Char('G') => {
            // Tail-anchor: align the bottom of the body with the bottom
            // of the visible area so the most recent line sits at the
            // last visible row, with N preceding lines filling the gap.
            *scroll = tail_scroll(body.len(), *last_render_height);
        }
        KeyCode::Char('r') => {
            // Re-queue a fresh fetch with the same coordinates. Cleared
            // body + reset scroll so the loading indicator is visible
            // while the SSH call runs.
            let alias = alias.clone();
            let container_id = container_id.clone();
            let container_name = container_name.clone();
            ctx.requeue_logs_fetch(alias, container_id, container_name);
        }
        _ => {}
    }
}

impl ContainerLogsCtx<'_> {
    fn requeue_logs_fetch(&mut self, alias: String, container_id: String, container_name: String) {
        let Some(entry) = self.container_state.cache_entry(&alias) else {
            log::debug!(
                "[purple] container_logs: refresh aborted, no cache for alias={}",
                alias
            );
            return;
        };
        let runtime = entry.runtime;
        let askpass = self
            .hosts
            .list()
            .iter()
            .find(|h| h.alias == alias)
            .and_then(|h| h.askpass.clone());

        if matches!(*self.screen, Screen::ContainerLogs)
            && let Some(view) = self.container_state.logs_view_mut()
        {
            view.body.clear();
            view.scroll = 0;
            view.error = None;
            view.fetched_at = 0;
            // Wipe stale line indices: the body just emptied, so any old
            // match positions now point past the end. The completion path
            // in event_loop calls `refresh_search` to repopulate against
            // the fresh body, but until then the search bar would show
            // a stale "(3 of 5)" badge tied to the previous fetch.
            if let Some(s) = view.search.as_mut() {
                s.matches.clear();
                s.current = 0;
            }
        }
        log::debug!(
            "[purple] container_logs: refresh queued alias={} id={}",
            alias,
            container_id
        );
        self.container_state.queue_logs(ContainerLogsRequest {
            alias,
            askpass,
            runtime,
            container_id,
            container_name,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_case_lowercase_query_matches_uppercase_haystack() {
        assert!(matches_line("ERROR: Connection refused", "error"));
        assert!(matches_line("Error", "error"));
    }

    #[test]
    fn smart_case_uppercase_query_is_case_sensitive() {
        assert!(matches_line("ERROR: Connection refused", "ERROR"));
        assert!(!matches_line("error", "ERROR"));
        assert!(!matches_line("Error", "ERROR"));
    }

    #[test]
    fn empty_query_never_matches() {
        assert!(!matches_line("anything", ""));
    }

    #[test]
    fn match_indices_returns_every_hit() {
        let positions = match_indices_smart("foo bar foo baz foo", "foo");
        assert_eq!(positions, vec![0, 8, 16]);
    }

    #[test]
    fn match_indices_smart_case_insensitive_is_ascii_safe() {
        let positions = match_indices_smart("Foo FOO foo", "foo");
        assert_eq!(positions, vec![0, 4, 8]);
    }

    #[test]
    fn match_indices_non_overlapping() {
        // `aaa` in `aaaaa` yields 2 non-overlapping matches at 0 and 3.
        let positions = match_indices_smart("aaaaa", "aaa");
        assert_eq!(positions, vec![0]);
    }

    #[test]
    fn center_scroll_anchors_match_in_middle() {
        // Target line 50, viewport 20 → half = 10, so scroll = 40.
        assert_eq!(center_scroll(50, 20, 200), 40);
    }

    #[test]
    fn center_scroll_clamps_to_zero_for_top_match() {
        assert_eq!(center_scroll(2, 20, 200), 0);
    }

    #[test]
    fn center_scroll_clamps_to_max_when_match_near_tail() {
        // body_len 100, viewport 20 → max scroll = 80.
        // Match at line 95 would want scroll = 85, clamped to 80.
        assert_eq!(center_scroll(95, 20, 100), 80);
    }

    #[test]
    fn recompute_matches_resets_current_to_first_hit() {
        // Consistent with app::search::apply_filter: every refining
        // moves the cursor back to the first match so the viewport
        // always scrolls to the top hit.
        let body = vec![
            "error 1".to_string(),
            "ok".to_string(),
            "error 2".to_string(),
        ];
        let mut search = ContainerLogsSearch {
            query: "error".to_string(),
            current: 1,
            ..Default::default()
        };
        recompute_matches(&body, &mut search);
        assert_eq!(search.matches, vec![0, 2]);
        assert_eq!(search.current, 0, "current resets to first match");
    }

    #[test]
    fn recompute_matches_resets_current_when_hits_shrink_to_none() {
        let body = vec!["foo".to_string(), "bar".to_string()];
        let mut search = ContainerLogsSearch {
            query: "qux".to_string(),
            current: 5,
            ..Default::default()
        };
        recompute_matches(&body, &mut search);
        assert!(search.matches.is_empty());
        assert_eq!(search.current, 0);
    }
}
