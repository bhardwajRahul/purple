//! Render a ratatui `Buffer` to a standalone SVG (one rect per cell
//! background, one text run per glyph). Used by `purple gen-assets` to emit
//! crisp, on-brand screenshots that regenerate from the live binary, so
//! marketing imagery never goes stale.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// An 8-bit-per-channel RGB triple.
type Rgb = (u8, u8, u8);

/// Rendering options for [`buffer_to_svg`].
pub struct SvgOpts {
    /// Pixel width of one terminal column.
    pub cell_w: u32,
    /// Pixel height of one terminal row.
    pub cell_h: u32,
    /// Font size in pixels.
    pub font_size: u32,
    /// Full CSS `font-family` value the `<text>` elements reference, quotes
    /// and fallbacks included (e.g. `"'Berkeley Mono','JetBrains Mono',monospace"`).
    pub font_family: String,
    /// A complete `@font-face { ... }` CSS block embedding the monospace font
    /// as base64, or empty to fall back to the system `font_family`.
    pub font_face_css: String,
    /// Foreground used for cells whose `fg` is `Color::Reset`.
    pub default_fg: Rgb,
    /// Background used for the canvas and cells whose `bg` is `Color::Reset`.
    pub default_bg: Rgb,
    /// Empty margin around the cell grid, in pixels.
    pub pad: u32,
    /// Render the canvas as a rounded panel on a transparent background
    /// instead of a flush rectangle.
    pub rounded: bool,
}

/// Panel corner radius when [`SvgOpts::rounded`] is on.
const PANEL_RADIUS: u32 = 12;

impl Default for SvgOpts {
    fn default() -> Self {
        Self {
            // cell_w is exactly 0.6 * font_size, the advance width of
            // Berkeley Mono and JetBrains Mono, so textLength never has to
            // stretch or squeeze glyphs to fit the grid.
            cell_w: 9,
            cell_h: 19,
            font_size: 15,
            font_family: "'JetBrains Mono',monospace".to_string(),
            font_face_css: String::new(),
            // Soft slate terminal background with the lavender foreground.
            default_fg: (224, 214, 240),
            default_bg: (27, 28, 38),
            pad: 0,
            rounded: false,
        }
    }
}

/// Canvas metrics for a grid of `cols` x `rows` cells: full width, full
/// height and the top offset of the cell grid.
fn canvas(opts: &SvgOpts, cols: u16, rows: u16) -> (u32, u32, u32) {
    let w = u32::from(cols) * opts.cell_w + 2 * opts.pad;
    let h = u32::from(rows) * opts.cell_h + 2 * opts.pad;
    (w, h, opts.pad)
}

/// Emit the canvas background: a flush rect, or a rounded panel on a
/// transparent canvas when rounded is on.
fn emit_canvas(s: &mut String, opts: &SvgOpts, w: u32, h: u32) {
    if opts.rounded {
        s.push_str(&format!(
            "<rect width=\"{w}\" height=\"{h}\" rx=\"{PANEL_RADIUS}\" ry=\"{PANEL_RADIUS}\" fill=\"{}\"/>",
            hex(opts.default_bg)
        ));
    } else {
        s.push_str(&format!(
            "<rect width=\"{w}\" height=\"{h}\" fill=\"{}\"/>",
            hex(opts.default_bg)
        ));
    }
}

/// Serialise `buf` to a standalone SVG string.
pub fn buffer_to_svg(buf: &Buffer, opts: &SvgOpts) -> String {
    let area = buf.area;
    let (w, h, top) = canvas(opts, area.width, area.height);
    let baseline = baseline(opts);

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
    ));
    s.push_str("<defs><style>");
    s.push_str(&opts.font_face_css);
    s.push_str(&format!(
        "text{{font-family:{};font-size:{}px;white-space:pre;}}",
        opts.font_family, opts.font_size
    ));
    s.push_str("</style></defs>");
    emit_canvas(&mut s, opts, w, h);
    let shifted = opts.pad > 0 || top > 0;
    if shifted {
        s.push_str(&format!("<g transform=\"translate({},{top})\">", opts.pad));
    }
    emit_cells(&mut s, buf, opts, baseline, &|_, _| true, None);
    if shifted {
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    s
}

/// One frame of an animated SVG: the buffer, how long it holds, and whether
/// it is a self-contained keyframe (starts a scene, hides everything before
/// it) or a delta painted over the frames since the last keyframe.
pub struct AnimFrame {
    pub buf: Buffer,
    pub dur_ms: u32,
    pub keyframe: bool,
}

/// Serialise `frames` to one looping animated SVG. Scene switching is pure
/// CSS (`step-end` opacity windows), so the animation needs no JavaScript.
/// `fallback_keyframe` names the scene shown when animations are off
/// (prefers-reduced-motion); its whole frame stack stays visible.
///
/// Panics when `frames` is empty, dimensions differ between frames, the
/// first frame is not a keyframe or `fallback_keyframe` is not a keyframe.
pub fn frames_to_animated_svg(
    frames: &[AnimFrame],
    fallback_keyframe: usize,
    opts: &SvgOpts,
) -> String {
    assert!(!frames.is_empty(), "frames_to_animated_svg: no frames");
    assert!(
        frames[0].keyframe,
        "frames_to_animated_svg: first frame must be a keyframe"
    );
    assert!(
        frames.iter().all(|f| f.buf.area == frames[0].buf.area),
        "frames_to_animated_svg: all frames must share one size"
    );
    assert!(
        frames.get(fallback_keyframe).is_some_and(|f| f.keyframe),
        "frames_to_animated_svg: fallback must point at a keyframe"
    );

    let area = frames[0].buf.area;
    let (w, h, top) = canvas(opts, area.width, area.height);
    let baseline = baseline(opts);
    let total: u32 = frames.iter().map(|f| f.dur_ms).sum();

    // Window per frame: [own start, start of the next keyframe). A delta
    // stays visible until its scene ends so later deltas stack on top.
    let starts: Vec<u32> = frames
        .iter()
        .scan(0u32, |acc, f| {
            let s = *acc;
            *acc += f.dur_ms;
            Some(s)
        })
        .collect();
    let scene_end = |i: usize| -> u32 {
        frames[i + 1..]
            .iter()
            .position(|f| f.keyframe)
            .map_or(total, |off| starts[i + 1 + off])
    };
    let fallback_end = scene_end(fallback_keyframe);

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"panim\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
    ));
    s.push_str("<defs><style>");
    s.push_str(&opts.font_face_css);
    s.push_str(&format!(
        ".panim text{{font-family:{};font-size:{}px;white-space:pre;}}",
        opts.font_family, opts.font_size
    ));
    s.push_str(".panim .pf{opacity:0}.panim .pf-fb{opacity:1}");
    for (i, _) in frames.iter().enumerate() {
        let (start, end) = (starts[i], scene_end(i));
        if start == 0 && end == total {
            // Visible the whole loop: no animation needed.
            s.push_str(&format!(".panim .pf{i}{{opacity:1}}"));
            continue;
        }
        s.push_str(&format!(
            ".panim .pf{i}{{animation:pfa{i} {total}ms step-end infinite}}"
        ));
        let on = pct(start, total);
        let off = pct(end, total);
        if start == 0 {
            s.push_str(&format!(
                "@keyframes pfa{i}{{0%{{opacity:1}}{off}%{{opacity:0}}}}"
            ));
        } else if end == total {
            s.push_str(&format!("@keyframes pfa{i}{{{on}%{{opacity:1}}}}"));
        } else {
            s.push_str(&format!(
                "@keyframes pfa{i}{{{on}%{{opacity:1}}{off}%{{opacity:0}}}}"
            ));
        }
    }
    // Last so it wins the equal-specificity tie against the .pf{i} rules.
    s.push_str("@media (prefers-reduced-motion:reduce){.panim .pf{animation:none}}");
    s.push_str("</style></defs>");
    emit_canvas(&mut s, opts, w, h);
    let shifted = opts.pad > 0 || top > 0;
    if shifted {
        s.push_str(&format!("<g transform=\"translate({},{top})\">", opts.pad));
    }

    for (i, frame) in frames.iter().enumerate() {
        let fb = if starts[i] >= starts[fallback_keyframe] && starts[i] < fallback_end {
            " pf-fb"
        } else {
            ""
        };
        s.push_str(&format!("<g class=\"pf pf{i}{fb}\">"));
        if frame.keyframe {
            emit_cells(&mut s, &frame.buf, opts, baseline, &|_, _| true, None);
        } else {
            let prev = &frames[i - 1].buf;
            let mask = |x: u16, y: u16| frame.buf[(x, y)] != prev[(x, y)];
            emit_cells(&mut s, &frame.buf, opts, baseline, &mask, Some(prev));
        }
        s.push_str("</g>");
    }

    if shifted {
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    s
}

/// Glyph baseline within a cell row: ~80% down sits the glyph nicely for a
/// monospace font sized near the cell height.
fn baseline(opts: &SvgOpts) -> u32 {
    (opts.cell_h * 4) / 5
}

/// Percentage of `at` within `total`, trimmed to 4 decimals.
fn pct(at: u32, total: u32) -> String {
    let v = f64::from(at) * 100.0 / f64::from(total);
    format!("{v:.4}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// Emit the cell backgrounds and glyph runs for every cell where `mask`
/// returns true. With `prev` (delta frames), a cell whose new background is
/// the canvas default still gets a rect when the previous frame left visible
/// paint there (erasure); blank-to-glyph transitions skip it.
fn emit_cells(
    s: &mut String,
    buf: &Buffer,
    opts: &SvgOpts,
    baseline: u32,
    mask: &dyn Fn(u16, u16) -> bool,
    prev: Option<&Buffer>,
) {
    let area = buf.area;
    let needs_erase = |x: u16, y: u16| {
        prev.is_some_and(|p| {
            let cell = &p[(x, y)];
            let sym = cell.symbol();
            let (_, bg) = effective_colors(cell, opts);
            !(sym.is_empty() || sym == " ") || bg.is_some_and(|c| c != opts.default_bg)
        })
    };

    // Backgrounds first (so glyphs paint on top), merging contiguous cells
    // that share one color into a single rect.
    for y in 0..area.height {
        let mut run: Option<(u16, Rgb, u32)> = None;
        let flush = |s: &mut String, run: Option<(u16, Rgb, u32)>| {
            if let Some((start_x, rgb, len)) = run {
                s.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                    u32::from(start_x) * opts.cell_w,
                    u32::from(y) * opts.cell_h,
                    len * opts.cell_w,
                    opts.cell_h,
                    hex(rgb),
                ));
            }
        };
        for x in 0..area.width {
            if !mask(x, y) {
                flush(s, run.take());
                continue;
            }
            let cell = &buf[(x, y)];
            let (_, bg) = effective_colors(cell, opts);
            let rgb = bg.unwrap_or(opts.default_bg);
            if rgb == opts.default_bg && !needs_erase(x, y) {
                flush(s, run.take());
                continue;
            }
            match &mut run {
                Some((_, c, len)) if *c == rgb => *len += 1,
                _ => {
                    flush(s, run.take());
                    run = Some((x, rgb, 1));
                }
            }
        }
        flush(s, run.take());
    }

    // Glyphs: merge contiguous cells sharing one style into a single <text>
    // run to keep the file small. Spaces and style changes break a run.
    // Straight line glyphs merge too: ─ runs horizontally within a row,
    // │ and ┊ runs vertically across rows (collected, emitted at the end).
    let t = (f64::from(opts.cell_w) / 8.0).max(1.0);
    let mut vlines: Vec<(u16, u16, Rgb, bool)> = Vec::new();
    for y in 0..area.height {
        let mut run: Option<Run> = None;
        let mut hrun: Option<(u16, u32, Rgb, bool)> = None;
        let flush_h = |s: &mut String, hrun: Option<(u16, u32, Rgb, bool)>| {
            if let Some((start_x, len, fill, dim)) = hrun {
                let cy = f64::from(u32::from(y) * opts.cell_h) + f64::from(opts.cell_h) / 2.0;
                s.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"{}/>",
                    coord(f64::from(u32::from(start_x) * opts.cell_w)),
                    coord(cy - t / 2.0),
                    coord(f64::from(len * opts.cell_w)),
                    coord(t),
                    hex(fill),
                    dim_fill_attr(dim),
                ));
            }
        };
        for x in 0..area.width {
            if !mask(x, y) {
                flush_run(s, run.take(), opts, baseline);
                flush_h(s, hrun.take());
                continue;
            }
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            if sym.is_empty() || sym == " " {
                flush_run(s, run.take(), opts, baseline);
                flush_h(s, hrun.take());
                continue;
            }
            let (fg, _) = effective_colors(cell, opts);
            let dim = cell.modifier.contains(Modifier::DIM);
            let cp = sym.chars().next().unwrap_or(' ');
            if cp == '\u{2500}' {
                flush_run(s, run.take(), opts, baseline);
                match &mut hrun {
                    Some((_, len, fill, d)) if *fill == fg && *d == dim => *len += 1,
                    _ => {
                        flush_h(s, hrun.take());
                        hrun = Some((x, 1, fg, dim));
                    }
                }
                continue;
            }
            if cp == '\u{2502}' || cp == '\u{250A}' {
                flush_run(s, run.take(), opts, baseline);
                flush_h(s, hrun.take());
                vlines.push((x, y, fg, dim));
                continue;
            }
            // Box-drawing, block, braille and status glyphs render as crisp SVG
            // shapes (see glyph_shape) so they tile seamlessly and need no font.
            if let Some(shape) = glyph_shape(
                sym,
                f64::from(u32::from(x) * opts.cell_w),
                f64::from(u32::from(y) * opts.cell_h),
                f64::from(opts.cell_w),
                f64::from(opts.cell_h),
                &hex(fg),
                dim,
            ) {
                flush_run(s, run.take(), opts, baseline);
                flush_h(s, hrun.take());
                s.push_str(&shape);
                continue;
            }
            flush_h(s, hrun.take());
            let style = GlyphStyle {
                fg,
                bold: cell.modifier.contains(Modifier::BOLD),
                dim: cell.modifier.contains(Modifier::DIM),
                italic: cell.modifier.contains(Modifier::ITALIC),
                underline: cell.modifier.contains(Modifier::UNDERLINED),
            };
            match &mut run {
                Some(acc) if acc.style == style => {
                    acc.text.push_str(&xml_escape(sym));
                    acc.len += 1;
                }
                _ => {
                    flush_run(s, run.take(), opts, baseline);
                    run = Some(Run {
                        start_x: x,
                        row: y,
                        len: 1,
                        style,
                        text: xml_escape(sym),
                    });
                }
            }
        }
        flush_run(s, run.take(), opts, baseline);
        flush_h(s, hrun.take());
    }

    // Vertical line runs: consecutive rows in one column with one color
    // collapse into a single full-height rect.
    vlines.sort_unstable();
    let mut i = 0;
    while i < vlines.len() {
        let (x, y0, fill, dim) = vlines[i];
        let mut len: u16 = 1;
        while vlines.get(i + usize::from(len)) == Some(&(x, y0 + len, fill, dim)) {
            len += 1;
        }
        let cx = f64::from(u32::from(x) * opts.cell_w) + f64::from(opts.cell_w) / 2.0;
        s.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"{}/>",
            coord(cx - t / 2.0),
            coord(f64::from(u32::from(y0) * opts.cell_h)),
            coord(t),
            coord(f64::from(u32::from(len) * opts.cell_h)),
            hex(fill),
            dim_fill_attr(dim),
        ));
        i += usize::from(len);
    }
}

/// Shapes share the text DIM treatment: reduced opacity over the canvas.
fn dim_fill_attr(dim: bool) -> &'static str {
    if dim { " fill-opacity=\"0.6\"" } else { "" }
}

#[derive(PartialEq)]
struct GlyphStyle {
    fg: Rgb,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

struct Run {
    start_x: u16,
    row: u16,
    len: u32,
    style: GlyphStyle,
    text: String,
}

fn flush_run(s: &mut String, run: Option<Run>, opts: &SvgOpts, baseline: u32) {
    let Some(run) = run else {
        return;
    };
    let weight = if run.style.bold {
        " font-weight=\"bold\""
    } else {
        ""
    };
    let opacity = if run.style.dim {
        " fill-opacity=\"0.6\""
    } else {
        ""
    };
    let italic = if run.style.italic {
        " font-style=\"italic\""
    } else {
        ""
    };
    let underline = if run.style.underline {
        " text-decoration=\"underline\""
    } else {
        ""
    };
    s.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" fill=\"{}\"{weight}{opacity}{italic}{underline} textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\">{}</text>",
        u32::from(run.start_x) * opts.cell_w,
        u32::from(run.row) * opts.cell_h + baseline,
        hex(run.style.fg),
        run.len * opts.cell_w,
        run.text,
    ));
}

/// Effective `(fg, Option<bg>)` for a cell, applying the `REVERSED` modifier
/// (swap foreground and background). `bg == None` means "use the canvas
/// default" (a `Color::Reset` background that was not reversed).
fn effective_colors(cell: &ratatui::buffer::Cell, opts: &SvgOpts) -> (Rgb, Option<Rgb>) {
    let base_fg = resolve(cell.fg, opts, false).unwrap_or(opts.default_fg);
    let base_bg = resolve(cell.bg, opts, true);
    if cell.modifier.contains(Modifier::REVERSED) {
        (base_bg.unwrap_or(opts.default_bg), Some(base_fg))
    } else {
        (base_fg, base_bg)
    }
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Compact coordinate: whole numbers without a decimal, otherwise 2 dp trimmed.
fn coord(v: f64) -> String {
    if (v.round() - v).abs() < 1e-6 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

/// Render the box-drawing, block, braille and status glyphs purple uses as
/// crisp SVG shapes (rects, circles and stroked paths) so they tile seamlessly
/// and never depend on a fallback font. Font glyphs leave gaps at non-unit line
/// heights, and a fallback face for braille / status symbols advances wider than
/// the grid and bleeds into the next column. Returns `None` for glyphs that
/// render as `<text>` (letters, digits and the symbol glyphs the embedded font
/// covers, e.g. ✓ ⚠ ▲ ▸ ▾).
fn glyph_shape(
    sym: &str,
    x0: f64,
    y0: f64,
    cw: f64,
    ch: f64,
    fill: &str,
    dim: bool,
) -> Option<String> {
    let t = (cw / 8.0).max(1.0);
    let cx = x0 + cw / 2.0;
    let cy = y0 + ch / 2.0;
    let r = (cw * 0.4).min(cw / 2.0).min(ch / 2.0);
    let fop = dim_fill_attr(dim);
    let sop = if dim { " stroke-opacity=\"0.6\"" } else { "" };
    let rect = |x: f64, y: f64, w: f64, h: f64| {
        format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\"{fop}/>",
            coord(x),
            coord(y),
            coord(w),
            coord(h),
        )
    };
    let stroke = |d: String| {
        format!(
            "<path d=\"{d}\" fill=\"none\" stroke=\"{fill}\" stroke-width=\"{}\"{sop}/>",
            coord(t),
        )
    };
    // Status-dot radius, shared by the circle family so every dot matches.
    let dot_r = (cw * 0.34).min(ch / 2.0);
    let disc = |rr: f64| {
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{fill}\"{fop}/>",
            coord(cx),
            coord(cy),
            coord(rr),
        )
    };
    let ring = |rr: f64| {
        format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{fill}\" stroke-width=\"{}\"{sop}/>",
            coord(cx),
            coord(cy),
            coord(rr),
            coord(t),
        )
    };
    let vline = rect(cx - t / 2.0, y0, t, ch);
    let hline = rect(x0, cy - t / 2.0, cw, t);
    let h_left = rect(x0, cy - t / 2.0, cw / 2.0 + t / 2.0, t);
    let h_right = rect(cx - t / 2.0, cy - t / 2.0, cw / 2.0 + t / 2.0, t);
    let v_top = rect(cx - t / 2.0, y0, t, ch / 2.0 + t / 2.0);
    let cp = sym.chars().next()?;
    match cp {
        '\u{2500}' => Some(hline),                       // ─
        '\u{2502}' | '\u{250A}' => Some(vline),          // │ ┊
        '\u{2574}' => Some(h_left),                      // ╴
        '\u{251C}' => Some(format!("{vline}{h_right}")), // ├
        '\u{2524}' => Some(format!("{vline}{h_left}")),  // ┤
        '\u{2514}' => Some(format!("{v_top}{h_right}")), // └ (tree branch)
        // Rounded corners: a horizontal stub into a quarter arc into a vertical stub.
        '\u{256D}' => Some(stroke(format!(
            "M {},{} H {} A {},{} 0 0 0 {},{} V {}",
            coord(x0 + cw),
            coord(cy),
            coord(cx + r),
            coord(r),
            coord(r),
            coord(cx),
            coord(cy + r),
            coord(y0 + ch),
        ))), // ╭
        '\u{256E}' => Some(stroke(format!(
            "M {},{} H {} A {},{} 0 0 1 {},{} V {}",
            coord(x0),
            coord(cy),
            coord(cx - r),
            coord(r),
            coord(r),
            coord(cx),
            coord(cy + r),
            coord(y0 + ch),
        ))), // ╮
        '\u{2570}' => Some(stroke(format!(
            "M {},{} H {} A {},{} 0 0 1 {},{} V {}",
            coord(x0 + cw),
            coord(cy),
            coord(cx + r),
            coord(r),
            coord(r),
            coord(cx),
            coord(cy - r),
            coord(y0),
        ))), // ╰
        '\u{256F}' => Some(stroke(format!(
            "M {},{} H {} A {},{} 0 0 0 {},{} V {}",
            coord(x0),
            coord(cy),
            coord(cx - r),
            coord(r),
            coord(r),
            coord(cx),
            coord(cy - r),
            coord(y0),
        ))), // ╯
        '\u{2588}' => Some(rect(x0, y0, cw, ch)), // █
        '\u{2591}' => Some(format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\" fill-opacity=\"0.28\"/>",
            coord(x0),
            coord(y0),
            coord(cw),
            coord(ch),
        )), // ░
        // Lower blocks ▁..▇: a bottom-anchored bar of n eighths.
        '\u{2581}'..='\u{2587}' => {
            let h = ch * f64::from(cp as u32 - 0x2580) / 8.0;
            Some(rect(x0, y0 + ch - h, cw, h))
        }
        // Left blocks ▉..▏ (and ▌): a left-anchored bar of n eighths.
        '\u{2589}'..='\u{258F}' => {
            let w = cw * f64::from(0x2590 - cp as u32) / 8.0;
            Some(rect(x0, y0, w, ch))
        }
        // Braille patterns (sparklines, trend charts, spinners): a 2x4 dot
        // grid. Drawn as crisp in-cell dots, never font glyphs. rsvg renders
        // a fallback braille face wider than the grid advance and ignores
        // textLength, so on the text path the dots bleed into the next column.
        '\u{2800}'..='\u{28FF}' => {
            let bits = cp as u32 - 0x2800;
            let (sub_w, sub_h) = (cw / 2.0, ch / 4.0);
            let (dot_w, dot_h) = (sub_w * 0.6, sub_h * 0.6);
            let (gx, gy) = ((sub_w - dot_w) / 2.0, (sub_h - dot_h) / 2.0);
            // Bit layout per column, top row to bottom row.
            const COL_BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
            let mut dots = String::new();
            for (col, col_bits) in COL_BITS.iter().enumerate() {
                for (row, bit) in col_bits.iter().enumerate() {
                    if bits & bit != 0 {
                        dots.push_str(&rect(
                            x0 + col as f64 * sub_w + gx,
                            y0 + row as f64 * sub_h + gy,
                            dot_w,
                            dot_h,
                        ));
                    }
                }
            }
            Some(dots)
        }
        // Heavy multiplication X (ICON_ERROR): two diagonal strokes. Neither
        // brand face ships this glyph, so draw it instead of leaking to a
        // system fallback font in the rasterised imagery.
        '\u{2716}' => {
            let s = cw.min(ch) * 0.72;
            let (xl, xr) = (cx - s / 2.0, cx + s / 2.0);
            let (yt, yb) = (cy - s / 2.0, cy + s / 2.0);
            Some(format!(
                "<path d=\"M {},{} L {},{} M {},{} L {},{}\" fill=\"none\" stroke=\"{fill}\" stroke-width=\"{}\" stroke-linecap=\"round\"{sop}/>",
                coord(xl),
                coord(yt),
                coord(xr),
                coord(yb),
                coord(xr),
                coord(yt),
                coord(xl),
                coord(yb),
                coord(t * 1.5),
            ))
        }
        // Left-right arrows (tunnel indicator U+21C4): top line points right,
        // bottom line points left. Also absent from both brand faces.
        '\u{21C4}' => {
            let xl = coord(x0 + cw * 0.12);
            let xr = coord(x0 + cw * 0.88);
            let yt = coord(cy - ch * 0.10);
            let yb = coord(cy + ch * 0.10);
            let head_r = coord(x0 + cw * 0.88 - cw * 0.24);
            let head_l = coord(x0 + cw * 0.12 + cw * 0.24);
            let yt_up = coord(cy - ch * 0.10 - ch * 0.06);
            let yt_dn = coord(cy - ch * 0.10 + ch * 0.06);
            let yb_up = coord(cy + ch * 0.10 - ch * 0.06);
            let yb_dn = coord(cy + ch * 0.10 + ch * 0.06);
            let sw = coord(t);
            Some(format!(
                "<path d=\"M {xl},{yt} H {xr} M {head_r},{yt_up} L {xr},{yt} L {head_r},{yt_dn} M {xr},{yb} H {xl} M {head_l},{yb_up} L {xl},{yb} L {head_l},{yb_dn}\" fill=\"none\" stroke=\"{fill}\" stroke-width=\"{sw}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{sop}/>",
            ))
        }
        // Status circles: crisp shapes so every status dot renders identically
        // and never depends on a fallback face. ● filled, ○ outline, ◉ ring +
        // centre (target/fisheye), ◐ outline + filled left half (paused).
        '\u{25CF}' => Some(disc(dot_r)),
        '\u{25CB}' => Some(ring(dot_r)),
        '\u{25C9}' => Some(format!("{}{}", ring(dot_r), disc(dot_r * 0.42))),
        '\u{25D0}' => {
            let half = format!(
                "<path d=\"M {cxc},{top} A {rr},{rr} 0 0 0 {cxc},{bot} Z\" fill=\"{fill}\"{fop}/>",
                cxc = coord(cx),
                top = coord(cy - dot_r),
                bot = coord(cy + dot_r),
                rr = coord(dot_r),
            );
            Some(format!("{}{}", ring(dot_r), half))
        }
        // Open box (toggle / space-key hint): a squared U on the baseline.
        '\u{2423}' => {
            let l = coord(x0 + cw * 0.18);
            let r2 = coord(x0 + cw * 0.82);
            let top = coord(cy - ch * 0.10);
            let bot = coord(cy + ch * 0.16);
            let sw = coord(t);
            Some(format!(
                "<path d=\"M {l},{top} V {bot} H {r2} V {top}\" fill=\"none\" stroke=\"{fill}\" stroke-width=\"{sw}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{sop}/>",
            ))
        }
        _ => None,
    }
}

/// Resolve a ratatui [`Color`] to an RGB triple. `Color::Reset` returns
/// `None` so the caller can fall back to the canvas default.
fn resolve(color: Color, _opts: &SvgOpts, _is_bg: bool) -> Option<Rgb> {
    match color {
        Color::Reset => None,
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Indexed(i) => Some(xterm256(i)),
        named => Some(ansi16(named)),
    }
}

/// Standard VS Code terminal palette for the 16 named ANSI colors. These are
/// fallbacks: in truecolor render mode purple's cells carry `Color::Rgb`.
fn ansi16(c: Color) -> Rgb {
    match c {
        Color::Black => (0, 0, 0),
        Color::Red => (205, 49, 49),
        Color::Green => (13, 188, 121),
        Color::Yellow => (229, 229, 16),
        Color::Blue => (36, 114, 200),
        Color::Magenta => (188, 63, 188),
        Color::Cyan => (17, 168, 205),
        Color::Gray => (229, 229, 229),
        Color::DarkGray => (102, 102, 102),
        Color::LightRed => (241, 76, 76),
        Color::LightGreen => (35, 209, 139),
        Color::LightYellow => (245, 245, 67),
        Color::LightBlue => (59, 142, 234),
        Color::LightMagenta => (214, 112, 214),
        Color::LightCyan => (41, 184, 219),
        Color::White => (229, 229, 229),
        // Reset/Rgb/Indexed handled by the caller.
        _ => (229, 229, 229),
    }
}

/// Map an xterm 256-color index to RGB (16 base + 6x6x6 cube + grayscale ramp).
fn xterm256(i: u8) -> Rgb {
    match i {
        0 => (0, 0, 0),
        1 => (205, 49, 49),
        2 => (13, 188, 121),
        3 => (229, 229, 16),
        4 => (36, 114, 200),
        5 => (188, 63, 188),
        6 => (17, 168, 205),
        7 => (229, 229, 229),
        8 => (102, 102, 102),
        9 => (241, 76, 76),
        10 => (35, 209, 139),
        11 => (245, 245, 67),
        12 => (59, 142, 234),
        13 => (214, 112, 214),
        14 => (41, 184, 219),
        15 => (255, 255, 255),
        16..=231 => {
            let n = i - 16;
            let r = n / 36;
            let g = (n % 36) / 6;
            let b = n % 6;
            let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (step(r), step(g), step(b))
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    }
}

/// Escape the five XML-significant characters for text content.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::{Modifier, Style};

    fn test_opts() -> SvgOpts {
        SvgOpts {
            cell_w: 10,
            cell_h: 20,
            font_size: 16,
            font_family: "'Mono'".to_string(),
            font_face_css: String::new(),
            default_fg: (224, 214, 240),
            default_bg: (10, 10, 20),
            ..SvgOpts::default()
        }
    }

    #[test]
    fn empty_buffer_emits_sized_svg_root_with_background() {
        let buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(svg.contains("<svg"), "has svg root: {svg}");
        assert!(svg.contains("width=\"20\""), "width = cols*cell_w: {svg}");
        assert!(svg.contains("height=\"20\""), "height = rows*cell_h: {svg}");
        assert!(
            svg.contains("xmlns=\"http://www.w3.org/2000/svg\""),
            "has xmlns: {svg}"
        );
        assert!(
            svg.contains("fill=\"#0a0a14\""),
            "background fill present: {svg}"
        );
    }

    #[test]
    fn cell_with_background_emits_positioned_rect() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        buf[(1, 1)].set_symbol(" ").set_bg(Color::Rgb(0, 240, 255));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            svg.contains("<rect x=\"10\" y=\"20\" width=\"10\" height=\"20\" fill=\"#00f0ff\"/>"),
            "positioned cell bg rect: {svg}"
        );
    }

    #[test]
    fn glyph_emits_text_run_and_space_does_not() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf[(0, 0)].set_symbol("A").set_fg(Color::Rgb(255, 0, 0));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(svg.contains(">A</text>"), "glyph A rendered as text: {svg}");
        assert!(svg.contains("fill=\"#ff0000\""), "glyph fg color: {svg}");
        assert_eq!(
            svg.matches("<text").count(),
            1,
            "only non-space cells get text: {svg}"
        );
    }

    #[test]
    fn embeds_font_face_and_sets_text_font() {
        let buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let mut opts = test_opts();
        opts.font_face_css =
            "@font-face{font-family:'Mono';src:url(data:font/woff2;base64,AAAA)}".to_string();
        let svg = buffer_to_svg(&buf, &opts);
        assert!(svg.contains("<defs><style>"), "has defs/style: {svg}");
        assert!(
            svg.contains("@font-face{font-family:'Mono'"),
            "embeds font-face: {svg}"
        );
        assert!(
            svg.contains("font-family:'Mono'") && svg.contains("font-size:16px"),
            "sets text font rule: {svg}"
        );
    }

    #[test]
    fn bold_modifier_sets_font_weight() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)]
            .set_symbol("A")
            .set_style(Style::new().add_modifier(Modifier::BOLD));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(svg.contains("font-weight=\"bold\""), "bold: {svg}");
    }

    #[test]
    fn dim_modifier_sets_fill_opacity() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)]
            .set_symbol("A")
            .set_style(Style::new().add_modifier(Modifier::DIM));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(svg.contains("fill-opacity=\"0.6\""), "dim: {svg}");
    }

    #[test]
    fn reversed_modifier_swaps_fg_and_bg() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)].set_symbol("A").set_style(
            Style::new()
                .fg(Color::Rgb(0, 240, 255))
                .bg(Color::Rgb(10, 10, 20))
                .add_modifier(Modifier::REVERSED),
        );
        let svg = buffer_to_svg(&buf, &test_opts());
        // Reversed: the bg rect takes the fg color, the glyph takes the bg color.
        assert!(
            svg.contains("<rect x=\"0\" y=\"0\" width=\"10\" height=\"20\" fill=\"#00f0ff\"/>"),
            "reversed bg rect uses fg color: {svg}"
        );
        assert!(
            svg.contains("<text x=\"0\" y=\"16\" fill=\"#0a0a14\""),
            "reversed glyph uses bg color: {svg}"
        );
    }

    #[test]
    fn escapes_xml_special_glyphs() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf[(0, 0)].set_symbol("<");
        buf[(1, 0)].set_symbol("&");
        let svg = buffer_to_svg(&buf, &test_opts());
        // Adjacent default-style glyphs merge into one run.
        assert!(svg.contains(">&lt;&amp;</text>"), "escapes < and &: {svg}");
    }

    #[test]
    fn contiguous_same_style_glyphs_merge_into_one_text_run() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf[(0, 0)].set_symbol("A").set_fg(Color::Rgb(0, 240, 255));
        buf[(1, 0)].set_symbol("B").set_fg(Color::Rgb(0, 240, 255));
        buf[(2, 0)].set_symbol("C").set_fg(Color::Rgb(0, 240, 255));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(svg.contains(">ABC</text>"), "merged run text: {svg}");
        assert_eq!(svg.matches("<text").count(), 1, "one text per run: {svg}");
        // textLength spans the whole run (3 cells * 10px).
        assert!(svg.contains("textLength=\"30\""), "run textLength: {svg}");
    }

    fn glyph_svg(symbol: &str) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)]
            .set_symbol(symbol)
            .set_fg(Color::Rgb(0, 240, 255));
        buffer_to_svg(&buf, &test_opts())
    }

    #[test]
    fn vertical_box_glyph_draws_full_height_rect_not_text() {
        let svg = glyph_svg("\u{2502}"); // │
        assert_eq!(
            svg.matches("<text").count(),
            0,
            "no text for box glyph: {svg}"
        );
        // t = cell_w/8 = 1.25, full cell height 20.
        assert!(
            svg.contains("width=\"1.25\"") && svg.contains("height=\"20\""),
            "full-height thin rect: {svg}"
        );
        assert!(svg.contains("fill=\"#00f0ff\""), "uses fg color: {svg}");
    }

    #[test]
    fn horizontal_box_glyph_draws_full_width_rect() {
        let svg = glyph_svg("\u{2500}"); // ─
        assert!(
            svg.contains("width=\"10\"") && svg.contains("height=\"1.25\""),
            "full-width thin rect: {svg}"
        );
    }

    #[test]
    fn full_block_fills_the_cell() {
        let svg = glyph_svg("\u{2588}"); // █
        assert!(
            svg.contains("<rect x=\"0\" y=\"0\" width=\"10\" height=\"20\" fill=\"#00f0ff\"/>"),
            "full cell rect: {svg}"
        );
    }

    #[test]
    fn lower_half_block_anchors_to_the_bottom() {
        let svg = glyph_svg("\u{2584}"); // ▄ = 4/8 from the bottom
        assert!(
            svg.contains("<rect x=\"0\" y=\"10\" width=\"10\" height=\"10\" fill=\"#00f0ff\"/>"),
            "bottom-half rect: {svg}"
        );
    }

    #[test]
    fn left_half_block_anchors_to_the_left() {
        let svg = glyph_svg(crate::ui::design::HOST_HIGHLIGHT); // ▌ = 4/8 from the left
        assert!(
            svg.contains("<rect x=\"0\" y=\"0\" width=\"5\" height=\"20\" fill=\"#00f0ff\"/>"),
            "left-half rect: {svg}"
        );
    }

    #[test]
    fn rounded_corner_draws_an_arc_path() {
        let svg = glyph_svg("\u{256D}"); // ╭
        assert!(svg.contains("<path d=\"M "), "path present: {svg}");
        assert!(svg.contains(" A "), "arc command present: {svg}");
        assert!(svg.contains("stroke=\"#00f0ff\""), "stroked in fg: {svg}");
    }

    #[test]
    fn symbol_without_a_shape_stays_text() {
        let svg = glyph_svg(crate::ui::design::ICON_WARNING); // ⚠ has no shape, renders as text
        assert!(
            svg.contains(">\u{26A0}</text>"),
            "warning sign renders as text: {svg}"
        );
    }

    #[test]
    fn box_drawing_up_right_junction_draws_shapes_not_text() {
        let svg = glyph_svg("\u{2514}"); // └ (design::TREE_BRANCH, the providers tree leaf)
        assert_eq!(
            svg.matches("<text").count(),
            0,
            "junction renders as shapes, not a fallback-font glyph: {svg}"
        );
        // Up arm (top-half vertical) + right arm (right-half horizontal): two fg rects.
        assert!(
            svg.matches("fill=\"#00f0ff\"").count() >= 2,
            "two stroke rects for the up and right arms: {svg}"
        );
    }

    #[test]
    fn braille_renders_as_in_cell_dots_not_text() {
        let svg = glyph_svg("\u{28FF}"); // ⣿ all eight dots raised
        assert_eq!(
            svg.matches("<text").count(),
            0,
            "braille renders as dot shapes, not a fallback-font glyph: {svg}"
        );
        assert_eq!(
            svg.matches("fill=\"#00f0ff\"").count(),
            8,
            "eight dots for the full braille cell: {svg}"
        );
    }

    #[test]
    fn braille_top_left_dot_sits_in_the_top_left_sub_cell() {
        let svg = glyph_svg("\u{2801}"); // ⠁ dot 1 only
        // sub-cell 5x5, dot 0.6 of it (3x3) centred -> offset 1,1.
        assert!(
            svg.contains("<rect x=\"1\" y=\"1\" width=\"3\" height=\"3\" fill=\"#00f0ff\"/>"),
            "single top-left dot: {svg}"
        );
        assert_eq!(
            svg.matches("fill=\"#00f0ff\"").count(),
            1,
            "exactly one dot: {svg}"
        );
    }

    #[test]
    fn braille_bottom_right_dot_stays_within_the_cell() {
        let svg = glyph_svg("\u{2880}"); // dot 8 (bit 0x80) -> bottom-right sub-cell
        // x+w = 6+3 = 9 <= cell_w(10), y+h = 16+3 = 19 <= cell_h(20): no bleed.
        assert!(
            svg.contains("<rect x=\"6\" y=\"16\" width=\"3\" height=\"3\" fill=\"#00f0ff\"/>"),
            "bottom-right dot stays inside the cell bounds: {svg}"
        );
    }

    #[test]
    fn error_x_renders_as_strokes_not_text() {
        let svg = glyph_svg(crate::ui::design::ICON_ERROR); // ✖, absent from both brand faces
        assert_eq!(
            svg.matches("<text").count(),
            0,
            "error X renders as shapes, not a system-fallback glyph: {svg}"
        );
        assert!(svg.contains("<path d=\"M "), "two diagonal strokes: {svg}");
        assert!(svg.contains("stroke=\"#00f0ff\""), "stroked in fg: {svg}");
    }

    #[test]
    fn tunnel_arrows_render_as_strokes_not_text() {
        let svg = glyph_svg("\u{21C4}"); // ⇄ tunnel indicator, absent from both brand faces
        assert_eq!(
            svg.matches("<text").count(),
            0,
            "tunnel arrows render as shapes, not a system-fallback glyph: {svg}"
        );
        assert!(svg.contains("<path d=\"M "), "arrow path present: {svg}");
        assert!(svg.contains("stroke=\"#00f0ff\""), "stroked in fg: {svg}");
    }

    #[test]
    fn status_circles_render_as_shapes_not_text() {
        use crate::ui::design::{ICON_ONLINE, ICON_PAUSED, ICON_STOPPED, ICON_TARGET};
        for g in [ICON_ONLINE, ICON_STOPPED, ICON_TARGET, ICON_PAUSED] {
            let svg = glyph_svg(g);
            assert_eq!(
                svg.matches("<text").count(),
                0,
                "status circle {g} renders as shapes, not a fallback glyph: {svg}"
            );
            assert!(
                svg.contains("<circle"),
                "status circle {g} has a circle: {svg}"
            );
        }
    }

    #[test]
    fn online_dot_is_one_filled_circle() {
        let svg = glyph_svg(crate::ui::design::ICON_ONLINE); // ● dot_r = cell_w(10)*0.34 = 3.4
        assert!(
            svg.contains("<circle cx=\"5\" cy=\"10\" r=\"3.4\" fill=\"#00f0ff\"/>"),
            "centred filled dot: {svg}"
        );
        assert_eq!(
            svg.matches("<circle").count(),
            1,
            "exactly one circle: {svg}"
        );
    }

    #[test]
    fn stopped_dot_is_an_outline_circle() {
        let svg = glyph_svg(crate::ui::design::ICON_STOPPED); // ○
        assert!(svg.contains("fill=\"none\""), "outline only: {svg}");
        assert!(svg.contains("stroke=\"#00f0ff\""), "stroked in fg: {svg}");
    }

    #[test]
    fn paused_dot_fills_its_left_half() {
        let svg = glyph_svg(crate::ui::design::ICON_PAUSED); // ◐ outline ring + left semicircle
        assert!(svg.contains("<circle"), "outline ring present: {svg}");
        assert!(
            svg.contains("A 3.4,3.4 0 0 0"),
            "left-half arc (sweep flag 0): {svg}"
        );
    }

    #[test]
    fn space_hint_renders_as_a_squared_u_not_text() {
        let svg = glyph_svg(crate::ui::design::TOGGLE_HINT); // ␣ open box
        assert_eq!(svg.matches("<text").count(), 0, "no fallback glyph: {svg}");
        assert!(svg.contains("<path d=\"M "), "squared-U path: {svg}");
    }

    #[test]
    fn fg_change_breaks_the_run() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf[(0, 0)].set_symbol("A").set_fg(Color::Rgb(0, 240, 255));
        buf[(1, 0)].set_symbol("B").set_fg(Color::Rgb(255, 0, 0));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert_eq!(
            svg.matches("<text").count(),
            2,
            "two runs on fg change: {svg}"
        );
    }

    fn anim_frame(buf: Buffer, dur_ms: u32, keyframe: bool) -> AnimFrame {
        AnimFrame {
            buf,
            dur_ms,
            keyframe,
        }
    }

    #[test]
    fn animated_svg_emits_scene_groups_with_step_end_windows() {
        let mut a = Buffer::empty(Rect::new(0, 0, 2, 1));
        a[(0, 0)].set_symbol("A");
        let mut b = a.clone();
        b[(1, 0)].set_symbol("B");
        let frames = vec![anim_frame(a, 1000, true), anim_frame(b, 1000, false)];
        let svg = frames_to_animated_svg(&frames, 0, &test_opts());
        assert!(svg.contains("<svg"), "svg root: {svg}");
        assert!(
            svg.contains("<g class=\"pf pf0"),
            "group for frame 0: {svg}"
        );
        assert!(
            svg.contains("<g class=\"pf pf1"),
            "group for frame 1: {svg}"
        );
        assert!(svg.contains("step-end"), "discrete timing: {svg}");
        assert!(svg.contains("infinite"), "looping: {svg}");
        assert!(svg.contains("@keyframes"), "keyframes present: {svg}");
        assert!(svg.contains("2000ms"), "total duration is the sum: {svg}");
    }

    #[test]
    fn delta_frame_emits_only_changed_cells_and_erases_cleared_ones() {
        // Frame 0: "AB". Frame 1: "A" stays, "B" cleared to a space.
        let mut a = Buffer::empty(Rect::new(0, 0, 2, 1));
        a[(0, 0)].set_symbol("A").set_fg(Color::Rgb(0, 240, 255));
        a[(1, 0)].set_symbol("B").set_fg(Color::Rgb(0, 240, 255));
        let mut b = a.clone();
        b[(1, 0)].set_symbol(" ").set_fg(Color::Reset);
        let frames = vec![anim_frame(a, 1000, true), anim_frame(b, 1000, false)];
        let svg = frames_to_animated_svg(&frames, 0, &test_opts());

        let delta = svg.split("<g class=\"pf pf1").nth(1).expect("delta group");
        assert!(
            !delta.contains(">A</text>"),
            "unchanged cell not re-emitted: {delta}"
        );
        assert!(
            delta.contains("<rect x=\"10\" y=\"0\" width=\"10\" height=\"20\" fill=\"#0a0a14\"/>"),
            "cleared cell gets a default-bg erasure rect: {delta}"
        );
        assert!(
            !delta.contains("<text"),
            "space emits no text in the delta: {delta}"
        );
    }

    #[test]
    fn keyframe_hides_previous_scene_and_carries_fallback_stack() {
        // Scene 1: keyframe + delta (the fallback). Scene 2: keyframe.
        let mut a = Buffer::empty(Rect::new(0, 0, 2, 1));
        a[(0, 0)].set_symbol("A");
        let mut b = a.clone();
        b[(1, 0)].set_symbol("B");
        let mut c = Buffer::empty(Rect::new(0, 0, 2, 1));
        c[(0, 0)].set_symbol("C");
        let frames = vec![
            anim_frame(a, 1000, true),
            anim_frame(b, 1000, false),
            anim_frame(c, 2000, true),
        ];
        let svg = frames_to_animated_svg(&frames, 0, &test_opts());

        // Scene 1 frames stay visible until the scene-2 keyframe at 50%.
        assert!(
            svg.contains("@keyframes pfa0{0%{opacity:1}50%{opacity:0}}"),
            "keyframe 0 window runs to the next keyframe: {svg}"
        );
        assert!(
            svg.contains("@keyframes pfa1{25%{opacity:1}50%{opacity:0}}"),
            "delta stays visible until its scene ends: {svg}"
        );
        // Scene 2 runs to the end of the loop.
        assert!(
            svg.contains("@keyframes pfa2{50%{opacity:1}}"),
            "last scene holds to 100%: {svg}"
        );
        // Fallback marks the whole scene-1 stack, not scene 2.
        assert!(
            svg.contains("<g class=\"pf pf0 pf-fb\">"),
            "fb on f0: {svg}"
        );
        assert!(
            svg.contains("<g class=\"pf pf1 pf-fb\">"),
            "fb on f1: {svg}"
        );
        assert!(svg.contains("<g class=\"pf pf2\">"), "no fb on f2: {svg}");
        assert!(
            svg.contains("@media (prefers-reduced-motion:reduce){.panim .pf{animation:none}}"),
            "reduced motion turns animation off: {svg}"
        );
    }

    #[test]
    fn adjacent_same_bg_cells_merge_into_one_rect() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        for x in 0..3 {
            buf[(x, 0)].set_symbol(" ").set_bg(Color::Rgb(0, 240, 255));
        }
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            svg.contains("<rect x=\"0\" y=\"0\" width=\"30\" height=\"20\" fill=\"#00f0ff\"/>"),
            "one merged run rect: {svg}"
        );
        assert_eq!(
            svg.matches("fill=\"#00f0ff\"").count(),
            1,
            "no per-cell rects: {svg}"
        );
    }

    #[test]
    fn delta_skips_erasure_for_cells_that_were_already_blank() {
        let mut a = Buffer::empty(Rect::new(0, 0, 2, 1));
        a[(0, 0)].set_symbol("A");
        let mut b = a.clone();
        b[(1, 0)].set_symbol("B"); // appears on a blank cell: nothing to erase
        let frames = vec![anim_frame(a, 1000, true), anim_frame(b, 1000, false)];
        let svg = frames_to_animated_svg(&frames, 0, &test_opts());
        let delta = svg.split("<g class=\"pf pf1").nth(1).expect("delta group");
        assert!(delta.contains(">B</text>"), "new glyph painted: {delta}");
        assert!(
            !delta.contains("<rect"),
            "no erasure rect over a blank cell: {delta}"
        );
    }

    #[test]
    fn consecutive_horizontal_line_glyphs_merge_into_one_rect() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        for x in 0..3 {
            buf[(x, 0)]
                .set_symbol("\u{2500}")
                .set_fg(Color::Rgb(0, 240, 255));
        }
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            svg.contains(
                "<rect x=\"0\" y=\"9.38\" width=\"30\" height=\"1.25\" fill=\"#00f0ff\"/>"
            ),
            "one merged horizontal rect: {svg}"
        );
        assert_eq!(
            svg.matches("fill=\"#00f0ff\"").count(),
            1,
            "no per-cell line rects: {svg}"
        );
    }

    #[test]
    fn vertical_line_glyphs_merge_across_rows() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 3));
        for y in 0..3 {
            buf[(0, y)]
                .set_symbol("\u{2502}")
                .set_fg(Color::Rgb(0, 240, 255));
        }
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            svg.contains(
                "<rect x=\"4.38\" y=\"0\" width=\"1.25\" height=\"60\" fill=\"#00f0ff\"/>"
            ),
            "one merged vertical rect: {svg}"
        );
        assert_eq!(
            svg.matches("fill=\"#00f0ff\"").count(),
            1,
            "no per-cell line rects: {svg}"
        );
    }

    #[test]
    fn dim_line_glyphs_honor_the_dim_modifier_like_text_does() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        for x in 0..2 {
            buf[(x, 0)].set_symbol("\u{2500}").set_style(
                Style::new()
                    .fg(Color::Rgb(224, 214, 240))
                    .add_modifier(Modifier::DIM),
            );
        }
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            svg.contains("width=\"20\"") && svg.contains("fill-opacity=\"0.6\""),
            "merged dim line rect carries reduced opacity: {svg}"
        );
    }

    #[test]
    fn dim_and_bright_line_runs_do_not_merge() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf[(0, 0)].set_symbol("\u{2500}").set_style(
            Style::new()
                .fg(Color::Rgb(224, 214, 240))
                .add_modifier(Modifier::DIM),
        );
        buf[(1, 0)]
            .set_symbol("\u{2500}")
            .set_fg(Color::Rgb(224, 214, 240));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert_eq!(
            svg.matches("fill=\"#e0d6f0\"").count(),
            2,
            "two separate rects when dim differs: {svg}"
        );
    }

    #[test]
    fn dim_rounded_corner_stroke_honors_the_dim_modifier() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)].set_symbol("\u{256D}").set_style(
            Style::new()
                .fg(Color::Rgb(224, 214, 240))
                .add_modifier(Modifier::DIM),
        );
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            svg.contains("stroke-opacity=\"0.6\""),
            "dim arc stroke carries reduced opacity: {svg}"
        );
    }

    #[test]
    fn padding_grows_the_canvas_and_offsets_the_cell_grid() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf[(0, 0)].set_symbol("A");
        let mut opts = test_opts();
        opts.pad = 10;
        let svg = buffer_to_svg(&buf, &opts);
        assert!(svg.contains("width=\"40\""), "20 grid + 2*10 pad: {svg}");
        assert!(svg.contains("height=\"40\""), "20 grid + 2*10 pad: {svg}");
        assert!(
            svg.contains("<g transform=\"translate(10,10)\">"),
            "cell grid shifts by the padding: {svg}"
        );
    }

    #[test]
    fn rounded_mode_draws_a_bare_rounded_panel() {
        let buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        let mut opts = test_opts();
        opts.pad = 10;
        opts.rounded = true;
        let svg = buffer_to_svg(&buf, &opts);
        assert!(svg.contains("rx=\"12\""), "rounded panel corners: {svg}");
        for light in ["#ff5f57", "#febc2e", "#28c840"] {
            assert!(!svg.contains(light), "no traffic light {light}: {svg}");
        }
        assert!(svg.contains("height=\"40\""), "grid + padding only: {svg}");
        assert!(
            svg.contains("<g transform=\"translate(10,10)\">"),
            "grid offset is just the padding: {svg}"
        );
    }

    #[test]
    fn no_padding_keeps_legacy_flush_output() {
        let buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            !svg.contains("<g transform="),
            "flush render emits no translate group: {svg}"
        );
    }

    #[test]
    #[should_panic(expected = "share one size")]
    fn animated_svg_rejects_mismatched_frame_sizes() {
        let a = Buffer::empty(Rect::new(0, 0, 2, 1));
        let b = Buffer::empty(Rect::new(0, 0, 3, 1));
        let frames = vec![anim_frame(a, 1000, true), anim_frame(b, 1000, false)];
        frames_to_animated_svg(&frames, 0, &test_opts());
    }

    #[test]
    fn reset_fg_uses_default_and_reset_bg_emits_no_cell_rect() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        buf[(0, 0)].set_symbol("A");
        let svg = buffer_to_svg(&buf, &test_opts());
        assert!(
            svg.contains("<text x=\"0\" y=\"16\" fill=\"#e0d6f0\""),
            "reset fg falls back to default_fg: {svg}"
        );
        assert_eq!(
            svg.matches("<rect").count(),
            1,
            "reset bg emits only the canvas rect: {svg}"
        );
    }
}
