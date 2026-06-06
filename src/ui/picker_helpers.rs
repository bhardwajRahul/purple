//! Shared building blocks for the host picker family (`tunnel_host_picker`,
//! `container_host_picker`, and the `snippet_host_picker` search strip).
//! Centralises the search-input strip, separator, row rendering, overlay
//! geometry and selection clamping so the host pickers stay visually identical
//! without copy-pasting the same code into each file.
//!
//! Pickers with a different shape (theme, tag, key_push, bulk_tag_editor)
//! intentionally do not flow through these helpers: their row layout, footer
//! state and overlay sizing diverge enough that a generic widget would either
//! bloat the API or lose features.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::theme;

/// Cap on simultaneously visible rows. Mirrors the jump bar so both
/// pickers scroll at the same rate on tall terminals.
const MAX_VISIBLE_ROWS: u16 = 16;

/// Render the search strip at the top of a host picker overlay: a brand-badge
/// `/` prefix, then the query (or a muted "type to filter hosts..." placeholder
/// when empty), with a thin cursor while editing. Shared by every host picker
/// so the affordance is identical across tunnel, container and snippet.
/// `show_cursor` is true while the filter input is actively being edited.
pub(super) fn render_search_input(frame: &mut Frame, area: Rect, query: &str, show_cursor: bool) {
    let line = if query.is_empty() {
        Line::from(vec![
            Span::styled(" / ", theme::brand_badge()),
            Span::raw(" "),
            Span::styled("type to filter hosts...", theme::muted()),
        ])
    } else {
        let mut spans = vec![
            Span::styled(" / ", theme::brand_badge()),
            Span::raw(" "),
            Span::raw(query.to_string()),
        ];
        if show_cursor {
            spans.push(Span::styled("_", theme::accent()));
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// Single-row muted separator that sits between the search input and
/// the list. Uses `\u{2500}` so the rule matches the rounded borders
/// the design system uses elsewhere.
pub(super) fn render_picker_separator(frame: &mut Frame, area: Rect) {
    let sep_width = (area.width as usize).saturating_sub(1);
    let sep = Line::from(Span::styled(
        format!(" {}", "\u{2500}".repeat(sep_width)),
        theme::muted(),
    ));
    frame.render_widget(Paragraph::new(sep), area);
}

/// Single picker row: 2-space leading gutter, alias in bold, hostname
/// dimmed. Hostname is truncated to fit the remaining display columns
/// via `super::truncate`, which is column-aware (CJK / fullwidth
/// characters consume the cells they actually occupy on screen). Plain
/// `chars().take(n)` would overshoot the budget when the hostname
/// contains wide characters.
pub(super) fn build_alias_hostname_row(
    alias: &str,
    hostname: &str,
    content_w: usize,
) -> ListItem<'static> {
    ListItem::new(Line::from(build_alias_hostname_spans(
        alias, hostname, content_w,
    )))
}

/// Span builder for `build_alias_hostname_row`. Extracted as a private
/// function so unit tests can inspect the row text without going
/// through ratatui's `ListItem` (whose `content` field is private).
fn build_alias_hostname_spans(alias: &str, hostname: &str, content_w: usize) -> Vec<Span<'static>> {
    let leading = 2;
    let gap = 2;
    let alias_w = alias.width().min(content_w.saturating_sub(leading));
    let remaining = content_w
        .saturating_sub(leading)
        .saturating_sub(alias_w)
        .saturating_sub(gap);
    let hostname_truncated = super::truncate(hostname, remaining);
    vec![
        Span::raw("  "),
        Span::styled(alias.to_string(), theme::bold()),
        Span::raw("  "),
        Span::styled(hostname_truncated, theme::muted()),
    ]
}

/// Centred overlay rectangle for an alias+hostname picker. Delegates
/// to `host_picker_overlay_dimensions` for the width/height math so
/// the geometry can be unit-tested without a `Frame`.
pub(super) fn host_picker_overlay_area(frame: &Frame, visible_count: usize) -> Rect {
    let frame_area = frame.area();
    let (width, height) =
        host_picker_overlay_dimensions(frame_area.width, frame_area.height, visible_count);
    super::centered_rect_fixed(width, height, frame_area)
}

/// Pure geometry for the alias+hostname picker overlay. Width is
/// `max(48, 60% of terminal)` clamped to `terminal - 4`, matching the
/// jump bar so the affordance feels uniform. Height grows with the
/// visible row count up to `MAX_VISIBLE_ROWS` and is clamped to leave
/// room for the footer below the block.
fn host_picker_overlay_dimensions(term_w: u16, term_h: u16, visible_count: usize) -> (u16, u16) {
    let list_height = (visible_count as u16).clamp(1, MAX_VISIBLE_ROWS);
    // border(2) + input(1) + separator(1) + list. Footer below the block.
    let total_height = 2 + 1 + 1 + list_height;
    let dynamic_width = 48u16.max(term_w * 60 / 100);
    let overlay_width = dynamic_width.min(term_w.saturating_sub(4));
    let height = total_height.min(term_h.saturating_sub(3));
    (overlay_width, height)
}

/// Keep the `ListState` selection inside `[0, visible_count)`. Returns
/// the picker to the first row when the previous selection has fallen
/// off the end (filtering, deletion). No-op when the selection is
/// already valid so renders are idempotent. Callers must guard against
/// `visible_count == 0` before invoking; passing zero would park the
/// selection on an empty list.
pub(super) fn clamp_picker_selection(state: &mut ListState, visible_count: usize) {
    let sel = state.selected();
    let new_sel = match sel {
        Some(i) if i < visible_count => Some(i),
        _ => Some(0),
    };
    if new_sel != sel {
        state.select(new_sel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_in_range_selection() {
        let mut state = ListState::default();
        state.select(Some(2));
        clamp_picker_selection(&mut state, 5);
        assert_eq!(state.selected(), Some(2));
    }

    #[test]
    fn clamp_resets_out_of_range_selection_to_zero() {
        let mut state = ListState::default();
        state.select(Some(7));
        clamp_picker_selection(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn clamp_none_selection_becomes_zero() {
        let mut state = ListState::default();
        clamp_picker_selection(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn clamp_at_boundary_index_equals_count_is_reset() {
        // Selection equal to visible_count is one-past-the-end and must
        // reset (rows are indexed `[0, visible_count)`).
        let mut state = ListState::default();
        state.select(Some(3));
        clamp_picker_selection(&mut state, 3);
        assert_eq!(state.selected(), Some(0));
    }

    fn flatten_spans(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn build_row_pads_alias_and_appends_hostname() {
        let spans = build_alias_hostname_spans("prod", "10.0.0.1", 60);
        let text = flatten_spans(&spans);
        assert!(text.contains("prod"));
        assert!(text.contains("10.0.0.1"));
    }

    #[test]
    fn build_row_truncates_hostname_to_remaining_budget() {
        // content_w 18: leading(2) + alias("ab"=2) + gap(2) leaves 12
        // columns for the hostname. The 26-char hostname must be
        // truncated and end in the ellipsis super::truncate uses.
        let spans = build_alias_hostname_spans("ab", "very-long-host.example.com", 18);
        let text = flatten_spans(&spans);
        assert!(text.ends_with('\u{2026}'), "expected ellipsis, got: {text}");
    }

    #[test]
    fn build_row_handles_zero_width_budget_without_panic() {
        // content_w smaller than leading: every saturating subtraction
        // collapses to zero. The function must return an empty hostname
        // section instead of panicking.
        let _ = build_alias_hostname_spans("alias", "host", 0);
    }

    #[test]
    fn overlay_dimensions_clamp_height_at_max_visible_rows() {
        // 100 visible rows but the list is capped at MAX_VISIBLE_ROWS (16).
        // Height = border(2) + input(1) + separator(1) + 16 = 20.
        let (_w, h) = host_picker_overlay_dimensions(120, 80, 100);
        assert_eq!(h, 2 + 1 + 1 + MAX_VISIBLE_ROWS);
    }

    #[test]
    fn overlay_dimensions_floor_height_at_one_row_when_empty() {
        // visible_count 0 still reserves 1 row so the empty-state hint
        // has somewhere to render.
        let (_w, h) = host_picker_overlay_dimensions(120, 80, 0);
        assert_eq!(h, 2 + 1 + 1 + 1);
    }

    #[test]
    fn overlay_dimensions_width_is_60_percent_of_wide_terminal() {
        // 120 wide: 60% = 72, which beats the 48 floor, and leaves room
        // for the 4-col inset.
        let (w, _h) = host_picker_overlay_dimensions(120, 80, 5);
        assert_eq!(w, 72);
    }

    #[test]
    fn overlay_dimensions_width_floors_at_48_on_narrow_terminal() {
        // 50 wide: 60% = 30 which loses to the 48 floor, then clamped
        // by terminal - 4 = 46.
        let (w, _h) = host_picker_overlay_dimensions(50, 30, 5);
        assert_eq!(w, 46);
    }

    #[test]
    fn overlay_dimensions_height_clamped_by_short_terminal() {
        // 8 row terminal: max usable height is 8 - 3 = 5. The natural
        // total of 4 + 5 = 9 exceeds that, so we clamp.
        let (_w, h) = host_picker_overlay_dimensions(120, 8, 5);
        assert_eq!(h, 5);
    }
}
