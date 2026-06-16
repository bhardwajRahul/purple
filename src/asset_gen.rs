//! Generate marketing SVG screenshots from the live TUI render path.
//!
//! Reuses `demo::build_demo_app` and `ui::render` so the imagery is produced
//! from the same code users run, then serialises each screen's ratatui buffer
//! to SVG via [`crate::ui::svg_export`]. Driven by the hidden `purple
//! gen-assets` subcommand in the release imagery pipeline.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use ratatui::buffer::Buffer;

use crate::animation::AnimationState;
use crate::app::{App, JumpMode, Screen, TopPage, ViewMode};
use crate::demo;
use crate::ui;
use crate::ui::svg_export::{AnimFrame, SvgOpts, buffer_to_svg, frames_to_animated_svg};

/// One named screen state to render, at a chosen terminal size. With `crop`,
/// only that cell region `(x, y, w, h)` of the rendered buffer ships: the
/// zoom-* detail shots are cut from the same live render as the full screens.
struct Scene {
    name: &'static str,
    width: u16,
    height: u16,
    setup: fn(&mut App),
    crop: Option<(u16, u16, u16, u16)>,
}

const SCENES: &[Scene] = &[
    Scene {
        name: "hosts",
        width: 120,
        height: 34,
        setup: setup_hosts,
        crop: None,
    },
    Scene {
        name: "search",
        width: 100,
        height: 30,
        setup: setup_search,
        crop: None,
    },
    Scene {
        name: "jump",
        width: 100,
        height: 30,
        setup: setup_jump,
        crop: None,
    },
    Scene {
        name: "providers",
        width: 100,
        height: 30,
        setup: setup_providers,
        crop: None,
    },
    Scene {
        name: "tunnels",
        width: 120,
        height: 36,
        setup: setup_tunnels,
        crop: None,
    },
    Scene {
        name: "containers",
        width: 130,
        height: 40,
        setup: setup_containers,
        crop: None,
    },
    Scene {
        name: "snippets",
        width: 160,
        height: 40,
        setup: setup_snippets,
        crop: None,
    },
    Scene {
        name: "keys",
        width: 130,
        height: 40,
        setup: setup_keys,
        crop: None,
    },
    // OG/social share card: the hosts hero view sized to ~1.91:1 (the og:image
    // aspect) so social platforms show it without cropping.
    Scene {
        name: "og-card",
        width: 124,
        height: 30,
        setup: setup_hosts,
        crop: None,
    },
    // Zoomed detail shots, cut from the same renders as the full screens.
    Scene {
        name: "zoom-host-detail",
        width: 120,
        height: 34,
        setup: setup_hosts,
        crop: Some((79, 3, 41, 19)),
    },
    Scene {
        name: "zoom-jump",
        width: 100,
        height: 30,
        setup: setup_jump,
        crop: Some((13, 7, 74, 16)),
    },
    Scene {
        name: "zoom-providers",
        width: 100,
        height: 30,
        setup: setup_providers,
        crop: Some((15, 5, 68, 19)),
    },
    Scene {
        name: "zoom-tunnel-live",
        width: 90,
        height: 32,
        setup: setup_tunnels,
        crop: Some((0, 17, 90, 12)),
    },
    Scene {
        name: "zoom-container-fleet",
        width: 90,
        height: 40,
        setup: setup_containers,
        crop: Some((0, 6, 90, 14)),
    },
    Scene {
        name: "zoom-snippet-impact",
        width: 160,
        height: 40,
        setup: setup_snippets,
        crop: Some((87, 10, 73, 18)),
    },
    Scene {
        name: "zoom-key-randomart",
        width: 100,
        height: 40,
        setup: setup_keys,
        crop: Some((28, 9, 72, 17)),
    },
    Scene {
        name: "zoom-vault-ttl",
        width: 90,
        height: 40,
        setup: setup_keys,
        crop: Some((0, 3, 90, 6)),
    },
];

/// Copy the `(x, y, w, h)` cell region of `src` into a fresh buffer.
fn crop_buffer(src: &Buffer, (x, y, w, h): (u16, u16, u16, u16)) -> Buffer {
    let mut out = Buffer::empty(ratatui::layout::Rect::new(0, 0, w, h));
    for dy in 0..h {
        for dx in 0..w {
            out[(dx, dy)] = src[(x + dx, y + dy)].clone();
        }
    }
    out
}

fn setup_hosts(app: &mut App) {
    app.hosts_state.set_view_mode(ViewMode::Detailed);
}

fn setup_search(app: &mut App) {
    app.start_search_with("aws");
}

fn setup_jump(app: &mut App) {
    app.open_jump(JumpMode::Hosts);
}

fn setup_providers(app: &mut App) {
    app.screen = Screen::Providers;
}

fn setup_tunnels(app: &mut App) {
    app.top_page = TopPage::Tunnels;
    demo::seed_tunnel_live_snapshots(app);
    app.ui.tunnels_overview_state_mut().select(Some(0));
}

fn setup_containers(app: &mut App) {
    app.top_page = TopPage::Containers;
    let first_container = crate::ui::containers_overview::visible_items(app)
        .iter()
        .position(|i| i.as_container().is_some())
        .unwrap_or(0);
    app.ui
        .containers_overview_state_mut()
        .select(Some(first_container));
}

fn setup_snippets(app: &mut App) {
    app.top_page = TopPage::Snippets;
    let deploy = app
        .snippets
        .store()
        .snippets
        .iter()
        .position(|s| s.name == "deploy")
        .unwrap_or(0);
    app.snippets.list_state_mut().select(Some(deploy));
}

fn setup_keys(app: &mut App) {
    app.top_page = TopPage::Keys;
    app.keys.list_state_mut().select(Some(0));
}

/// Render every curated scene to an SVG file in `out_dir`. When `font_dir` is
/// `Some`, the embedded base64 `@font-face` is read from
/// `ui-mono-regular.woff2` / `ui-mono-bold.woff2` there so box-drawing and
/// glyphs render identically inside an `<img>`-embedded SVG.
pub fn generate(out_dir: &Path, font_dir: Option<&Path>) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(out_dir)?;

    // Force the brand look regardless of the host terminal: truecolor + Purple.
    ui::theme::set_color_mode(2);
    ui::theme::set_theme(ui::theme::ThemeDef::purple());

    let font_face_css = match font_dir {
        Some(dir) => build_font_css(dir),
        None => String::new(),
    };

    let mut written = Vec::with_capacity(SCENES.len());
    for scene in SCENES {
        let mut app = demo::build_demo_app();
        (scene.setup)(&mut app);

        let backend = TestBackend::new(scene.width, scene.height);
        let mut terminal = Terminal::new(backend).map_err(|e| io::Error::other(e.to_string()))?;
        let mut anim = AnimationState::default();
        terminal
            .draw(|f| ui::render(f, &mut app, &mut anim))
            .map_err(|e| io::Error::other(e.to_string()))?;
        let mut buf = terminal.backend().buffer().clone();
        if let Some(region) = scene.crop {
            buf = crop_buffer(&buf, region);
        }

        // Berkeley Mono is the brand face; JetBrains Mono (embedded) fills the
        // symbol glyphs Berkeley lacks (✓ ⚠ ▲ ▸ ▾) the way a terminal's font
        // fallback would. Box-drawing, blocks, braille and status icons render
        // as crisp SVG shapes (see ui::svg_export), so they need no font. The
        // rasterizer resolves Berkeley from the render environment. Full screens
        // render as a rounded padded panel; zoom crops stay flush cutouts.
        let rounded = scene.crop.is_none();
        let opts = SvgOpts {
            font_family: "'Berkeley Mono','JetBrains Mono',monospace".to_string(),
            font_face_css: font_face_css.clone(),
            rounded,
            pad: if rounded { 14 } else { 0 },
            ..SvgOpts::default()
        };
        let svg = buffer_to_svg(&buf, &opts);
        let path = out_dir.join(format!("{}.svg", scene.name));
        crate::fs_util::atomic_write(&path, svg.as_bytes())?;
        log::debug!(
            "[purple] gen-assets wrote {} ({} bytes)",
            path.display(),
            svg.len()
        );
        written.push(path);
    }
    Ok(written)
}

/// One terminal size for every hero frame; the animation cross-fades scenes
/// on a single canvas.
const HERO_W: u16 = 120;
const HERO_H: u16 = 34;

/// Render the current app state to a buffer at the hero size.
fn hero_buffer(app: &mut App) -> io::Result<Buffer> {
    let backend = TestBackend::new(HERO_W, HERO_H);
    let mut terminal = Terminal::new(backend).map_err(|e| io::Error::other(e.to_string()))?;
    let mut anim = AnimationState::default();
    terminal
        .draw(|f| ui::render(f, app, &mut anim))
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(terminal.backend().buffer().clone())
}

/// The hero storyboard: hosts with a search typing itself, then the four
/// other top pages, each with its detail panel open.
fn hero_frames() -> io::Result<Vec<AnimFrame>> {
    let mut frames = Vec::with_capacity(9);

    let mut app = demo::build_demo_app();
    setup_hosts(&mut app);
    frames.push(AnimFrame {
        buf: hero_buffer(&mut app)?,
        dur_ms: 1600,
        keyframe: true,
    });
    for (query, dur_ms) in [("p", 200), ("pr", 200), ("pro", 200), ("prod", 2600)] {
        app.start_search_with(query);
        frames.push(AnimFrame {
            buf: hero_buffer(&mut app)?,
            dur_ms,
            keyframe: false,
        });
    }

    let pages: [fn(&mut App); 4] = [setup_tunnels, setup_containers, setup_snippets, setup_keys];
    for setup in pages {
        let mut app = demo::build_demo_app();
        setup(&mut app);
        frames.push(AnimFrame {
            buf: hero_buffer(&mut app)?,
            dur_ms: 3600,
            keyframe: true,
        });
    }
    Ok(frames)
}

/// Render the looping hero animation (all five tabs, detail panels open, a
/// search typing itself on the hosts tab) to one animated SVG at `out_path`.
/// The reduced-motion fallback is the typed-out hosts scene.
pub fn generate_hero(out_path: &Path, font_dir: Option<&Path>) -> io::Result<()> {
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }

    ui::theme::set_color_mode(2);
    ui::theme::set_theme(ui::theme::ThemeDef::purple());

    let font_face_css = font_dir.map(build_font_css).unwrap_or_default();
    let opts = SvgOpts {
        font_family: "'Berkeley Mono','JetBrains Mono',monospace".to_string(),
        font_face_css,
        rounded: true,
        pad: 14,
        ..SvgOpts::default()
    };
    let frames = hero_frames()?;
    let svg = frames_to_animated_svg(&frames, 0, &opts);
    crate::fs_util::atomic_write(out_path, svg.as_bytes())?;
    log::debug!(
        "[purple] gen-assets wrote {} ({} bytes, {} frames)",
        out_path.display(),
        svg.len(),
        frames.len()
    );
    Ok(())
}

/// Build the `@font-face` CSS embedding the regular and bold woff2 files as
/// base64. The files are a glyph-subset of JetBrains Mono v2.304 (OFL-1.1, see
/// `assets/fonts/OFL.txt`). Missing files are skipped silently (the SVG falls
/// back to the system monospace named in [`SvgOpts::font_family`]).
fn build_font_css(font_dir: &Path) -> String {
    let mut css = String::new();
    if let Ok(bytes) = fs::read(font_dir.join("ui-mono-regular.woff2")) {
        css.push_str(&font_face(400, &bytes));
    }
    if let Ok(bytes) = fs::read(font_dir.join("ui-mono-bold.woff2")) {
        css.push_str(&font_face(700, &bytes));
    }
    css
}

fn font_face(weight: u32, bytes: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        "@font-face{{font-family:'JetBrains Mono';font-style:normal;font-weight:{weight};src:url(data:font/woff2;base64,{b64}) format('woff2');}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_buffer_copies_exactly_the_requested_region() {
        let mut src = Buffer::empty(ratatui::layout::Rect::new(0, 0, 10, 5));
        src[(2, 1)].set_symbol("A");
        src[(5, 3)].set_symbol("B");
        src[(0, 0)].set_symbol("X"); // outside the crop
        let out = crop_buffer(&src, (2, 1, 4, 3));
        assert_eq!(out.area.width, 4);
        assert_eq!(out.area.height, 3);
        assert_eq!(out[(0, 0)].symbol(), "A", "top-left maps to crop origin");
        assert_eq!(out[(3, 2)].symbol(), "B", "bottom-right inside crop");
        let all: String = (0..4)
            .flat_map(|x| (0..3).map(move |y| (x, y)))
            .map(|(x, y)| out[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(!all.contains('X'), "outside cells are not copied");
    }

    #[test]
    fn generate_hero_writes_one_looping_animated_svg() {
        let _lock = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        let out = tmp.path().join("hero.svg");
        let font_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        generate_hero(&out, Some(&font_dir)).expect("generate_hero");

        let body = fs::read_to_string(&out).expect("read hero svg");
        assert!(body.starts_with("<svg"), "svg root");
        assert!(body.contains("</svg>"), "closed svg");
        assert!(body.contains("class=\"panim\""), "animated root class");
        assert!(body.contains("@keyframes"), "css timeline present");
        assert!(
            body.contains("prefers-reduced-motion"),
            "reduced motion fallback present"
        );
        assert!(body.contains("pf-fb"), "static fallback scene marked");
        assert!(body.contains("Berkeley Mono"), "primary face");
        assert!(body.contains("@font-face"), "embedded fallback font");
        // Five tabs: hosts (with four typing deltas) + tunnels + containers
        // + snippets + keys = 9 frame groups.
        assert_eq!(
            body.matches("<g class=\"pf pf").count(),
            9,
            "expected 9 frame groups"
        );
        // The typing scene reaches search mode; the query itself accumulates
        // across delta frames (one new character per frame), so assert the
        // search UI plus the per-keystroke deltas rather than one literal run.
        assert!(body.contains(">search:</text>"), "search UI appears");
        for ch in ["p", "r", "o", "d"] {
            assert!(
                body.contains(&format!(">{ch}</text>")),
                "keystroke '{ch}' lands in a delta frame"
            );
        }

        // Deterministic: a second run produces byte-identical output.
        let out2 = tmp.path().join("hero2.svg");
        generate_hero(&out2, Some(&font_dir)).expect("generate_hero again");
        let body2 = fs::read_to_string(&out2).expect("read second hero svg");
        assert_eq!(body, body2, "hero generation is deterministic");

        crate::demo_flag::disable();
    }

    #[test]
    fn generate_writes_one_svg_per_scene() {
        let _lock = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        let font_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        let written = generate(tmp.path(), Some(&font_dir)).expect("generate");

        assert_eq!(written.len(), SCENES.len(), "one file per scene");
        let mut saw_brand_purple = false;
        for path in &written {
            let body = fs::read_to_string(path).expect("read svg");
            // set_color_mode(2) forces truecolor, so the brand purple resolves
            // to its #9333ea hex rather than the ANSI 16 magenta fallback.
            saw_brand_purple |= body.contains("#9333ea");
            assert!(body.starts_with("<svg"), "svg root in {}", path.display());
            assert!(body.contains("</svg>"), "closed svg in {}", path.display());
            assert!(
                body.contains("<rect"),
                "has cell rects in {}",
                path.display()
            );
            assert!(
                body.contains("@font-face"),
                "embeds fallback font in {}",
                path.display()
            );
            assert!(
                body.contains("Berkeley Mono"),
                "uses Berkeley Mono as the primary face in {}",
                path.display()
            );
        }
        assert!(
            saw_brand_purple,
            "truecolor mode emits the brand purple #9333ea in at least one scene"
        );

        crate::demo_flag::disable();
    }

    #[test]
    fn generate_with_no_font_dir_omits_embedded_font() {
        let _lock = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir");
        let written = generate(tmp.path(), None).expect("generate");
        assert!(!written.is_empty(), "scenes written without a font dir");
        let body = fs::read_to_string(&written[0]).expect("read svg");
        assert!(body.starts_with("<svg"), "svg root");
        assert!(body.contains("<rect"), "has cell rects");
        assert!(
            !body.contains("@font-face"),
            "no embedded font without a font dir"
        );

        crate::demo_flag::disable();
    }

    #[test]
    fn build_font_css_skips_absent_weights() {
        // Empty dir: neither weight present, so no @font-face is emitted.
        let empty = tempfile::tempdir().expect("tempdir");
        assert!(build_font_css(empty.path()).is_empty(), "no files, no css");

        // Only the regular weight on disk: the bold @font-face is skipped.
        let partial = tempfile::tempdir().expect("tempdir");
        let regular =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/ui-mono-regular.woff2");
        fs::copy(&regular, partial.path().join("ui-mono-regular.woff2")).expect("copy regular");
        let css = build_font_css(partial.path());
        assert!(css.contains("font-weight:400"), "regular weight embedded");
        assert!(!css.contains("font-weight:700"), "bold skipped when absent");
    }
}
