//! Drunken Bishop randomart card. Renders the visual fingerprint in
//! either a canonical 17x9 grid or an upgraded 25x13 grid on taller
//! terminals, with a twinkle effect that reshuffles which cells fire
//! every `TWINKLE_BUCKET` animation ticks.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};

use crate::ssh_keys::SshKeyInfo;
use crate::ui::{design, theme};

/// Drunken Bishop dimensions: (interior cols, interior rows). Two
/// variants picked from based on the terminal height. Both axes must
/// be odd so the bishop walk starts cleanly in the centre. We do not
/// scale larger than 25x13 because the bishop walk only emits 256
/// moves: a grid much larger than that visits a small fraction of its
/// cells and looks sparse.
pub(super) const BISHOP_CANONICAL: (usize, usize) = (17, 9);
pub(super) const BISHOP_LARGE: (usize, usize) = (25, 13);

/// Minimum terminal height required to upgrade the bishop variant to
/// `LARGE`. Below this the hero shares room with the top bar, vault
/// strip, linked hosts grid and footer.
const LARGE_MIN_TERMINAL_H: u16 = 30;

/// Per-side horizontal padding inside the randomart card. Combined with
/// the rounded border it produces visible breathing room around the art.
pub(super) const RANDOMART_PAD_H: u16 = 2;
pub(super) const RANDOMART_PAD_V: u16 = 1;

/// Pace of the twinkle effect. The bishop pattern reshuffles every
/// `TWINKLE_BUCKET` animation ticks instead of every tick. Calibrated
/// between strobe (1) and barely-moving (24): at 10 the reshuffle lands
/// around three quarters of a second on the active 80 ms tick path and
/// stays readable as a starfield rather than a flicker.
const TWINKLE_BUCKET: u64 = 10;

/// Pick the bishop variant from the terminal height. Bishop dimensions
/// and `hero_h` are derived from this single decision point so they
/// cannot drift out of sync if a variant or padding is tweaked.
pub(super) fn pick_bishop_size(terminal_height: u16) -> (usize, usize) {
    if terminal_height >= LARGE_MIN_TERMINAL_H {
        BISHOP_LARGE
    } else {
        BISHOP_CANONICAL
    }
}

/// Middle card: rounded border without a title (the Keys list on the
/// left already names the active key) and the randomart visual centred
/// inside. No inner frame around the art; the card border is the only
/// outline.
pub(super) fn render_randomart_card(
    frame: &mut Frame,
    key: &SshKeyInfo,
    area: Rect,
    size: (usize, usize),
    spinner_tick: u64,
) {
    let block = design::main_block_line(Line::default()).padding(Padding::new(
        RANDOMART_PAD_H,
        RANDOMART_PAD_H,
        RANDOMART_PAD_V,
        RANDOMART_PAD_V,
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    render_bishop(frame, key, inner, size, spinner_tick);
}

/// Render the Drunken Bishop visual fingerprint at the requested grid
/// size, centred within `area`. The art renders raw without an inner
/// frame; the surrounding card border is the only outline.
fn render_bishop(
    frame: &mut Frame,
    key: &SshKeyInfo,
    area: Rect,
    size: (usize, usize),
    spinner_tick: u64,
) {
    let lines = build_bishop_lines(key, size, spinner_tick);
    if lines.is_empty() {
        let fallback: Vec<Line> = key
            .bishop_lines()
            .iter()
            .map(|l| Line::from(Span::styled((*l).to_string(), theme::muted())))
            .collect();
        if fallback.is_empty() {
            let placeholder = Paragraph::new(Line::from(Span::styled(
                "(no visual fingerprint)",
                theme::muted(),
            )));
            frame.render_widget(placeholder, area);
        } else {
            const CANONICAL_W: u16 = 19;
            let h = (fallback.len() as u16).min(area.height);
            let left_pad = area.width.saturating_sub(CANONICAL_W) / 2;
            let top_pad = area.height.saturating_sub(h) / 2;
            let centered = Rect::new(
                area.x + left_pad,
                area.y + top_pad,
                CANONICAL_W.min(area.width),
                h,
            );
            frame.render_widget(Clear, centered);
            frame.render_widget(Paragraph::new(fallback), centered);
        }
        return;
    }
    let total_w = size.0 as u16;
    let total_h = lines.len() as u16;
    let w = total_w.min(area.width);
    let h = total_h.min(area.height);
    let left_pad = area.width.saturating_sub(w) / 2;
    let top_pad = area.height.saturating_sub(h) / 2;
    let rect = Rect::new(area.x + left_pad, area.y + top_pad, w, h);
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines), rect);
}

fn build_bishop_lines(
    key: &SshKeyInfo,
    size: (usize, usize),
    spinner_tick: u64,
) -> Vec<Line<'static>> {
    let Some(bytes) = crate::ssh_keys::decode_fingerprint(&key.fingerprint) else {
        return Vec::new();
    };
    let (cols, rows) = size;
    let grid = crate::ssh_keys::drunken_bishop_grid(&bytes, cols, rows);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(rows);
    for (r, row) in grid.iter().enumerate() {
        let spans: Vec<Span<'static>> = row
            .iter()
            .enumerate()
            .map(|(c, &count)| {
                let ch = crate::ssh_keys::bishop_char(count);
                Span::styled(ch.to_string(), bishop_char_style(count, r, c, spinner_tick))
            })
            .collect();
        lines.push(Line::from(spans));
    }
    lines
}

/// Twinkle gate: cell is muted by default, bold otherwise; ~1-in-3 of
/// the bold cells flips to brand accent. Routes through `theme::*` so
/// the palette swaps cleanly with the active theme.
fn bishop_char_style(count: u8, row: usize, col: usize, tick: u64) -> Style {
    if count == 0 {
        return theme::muted();
    }
    if twinkle(row, col, tick) {
        if twinkle_accent(row, col, tick) {
            return theme::accent_bold();
        }
        return theme::bold();
    }
    theme::muted()
}

/// Deterministic pseudo-random gate that fires for roughly 1 in 12
/// (row, col) cells per twinkle bucket: dense enough that the
/// starfield reads as a constellation rather than a single spark.
fn twinkle(row: usize, col: usize, tick: u64) -> bool {
    let bucket = tick / TWINKLE_BUCKET;
    let seed = (row as u64)
        .wrapping_mul(0xA5A5_5A5A)
        .wrapping_add((col as u64).wrapping_mul(0x9E37_79B9))
        .wrapping_add(bucket.wrapping_mul(0xDEAD_BEEF));
    let h = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> 33).is_multiple_of(12)
}

/// Companion gate to `twinkle`: when a cell is twinkling, roughly one
/// in every three cells flips to the theme accent colour instead of
/// bold white, so the effect breathes between white and the brand
/// accent rather than a uniform flash. Different multipliers from
/// `twinkle` so the two are statistically independent.
fn twinkle_accent(row: usize, col: usize, tick: u64) -> bool {
    let bucket = tick / TWINKLE_BUCKET;
    let seed = (row as u64)
        .wrapping_mul(0xC0FF_EE17)
        .wrapping_add((col as u64).wrapping_mul(0xBADD_CAFE))
        .wrapping_add(bucket.wrapping_mul(0xFEED_FACE));
    let h = seed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (h >> 33).is_multiple_of(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_bishop_size_short_terminal_picks_canonical() {
        assert_eq!(pick_bishop_size(24), BISHOP_CANONICAL);
        assert_eq!(pick_bishop_size(29), BISHOP_CANONICAL);
    }

    #[test]
    fn pick_bishop_size_mid_terminal_picks_large() {
        assert_eq!(pick_bishop_size(LARGE_MIN_TERMINAL_H), BISHOP_LARGE);
        assert_eq!(pick_bishop_size(35), BISHOP_LARGE);
    }

    #[test]
    fn pick_bishop_size_tall_terminal_caps_at_large() {
        assert_eq!(pick_bishop_size(60), BISHOP_LARGE);
        assert_eq!(pick_bishop_size(120), BISHOP_LARGE);
    }

    #[test]
    fn twinkle_is_deterministic_for_same_inputs() {
        // Same (row, col, tick) must always yield the same boolean. The
        // animation depends on this so the visual goldens stay byte-stable
        // across re-renders within a single tick.
        let a = twinkle(3, 5, 7);
        let b = twinkle(3, 5, 7);
        assert_eq!(a, b);
    }

    #[test]
    fn twinkle_accent_fires_at_a_minority_rate() {
        // Sample over a deterministic cross-section: the function should
        // return true on a strict minority of cells. The accent rate is
        // documented as "~1-in-3 of bold cells" so the sampled rate must
        // be well under 50%.
        let mut accent_count = 0usize;
        let mut total = 0usize;
        for row in 0..9 {
            for col in 0..17 {
                for tick in 0..20 {
                    if twinkle_accent(row, col, tick) {
                        accent_count += 1;
                    }
                    total += 1;
                }
            }
        }
        let pct = (accent_count * 100) / total;
        assert!(
            pct < 50,
            "twinkle_accent fired on {}% of samples; expected minority",
            pct
        );
    }
}
