use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::animation::AnimationState;
use crate::app::{App, Screen, WhatsNewState};
use crate::changelog;

/// These tests set the color mode and the changelog override, both process
/// globals, so they serialize on the shared test lock like every other
/// theme mutator.
fn test_override_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn build_app() -> App {
    let path = tempfile::tempdir()
        .expect("tempdir")
        .keep()
        .join("test_config");
    let config = crate::ssh_config::model::SshConfigFile {
        elements: crate::ssh_config::model::SshConfigFile::parse_content(""),
        path,
        crlf: false,
        bom: false,
    };
    let mut app = App::new(config);
    *app.providers.config_mut() = crate::providers::config::ProviderConfig::default();
    app
}

fn render_with_fixture(width: u16, height: u16, scroll: u16, fixture_path: &str) -> String {
    let _guard = test_override_lock();
    let mut captured = String::new();
    crate::ui::theme::init_with_mode(1);
    let mut app = build_app();
    let paths = app.env().paths().cloned();
    crate::preferences::save_last_seen_version(paths.as_ref(), "0.0.1").unwrap();
    app.screen = Screen::WhatsNew(WhatsNewState { scroll });
    let fixture = std::fs::read_to_string(fixture_path).unwrap();
    let _override = changelog::set_test_override(fixture);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut anim = AnimationState::default();
    terminal
        .draw(|f| crate::ui::render(f, &mut app, &mut anim))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    for y in 0..height {
        for x in 0..width {
            captured.push_str(buffer[(x, y)].symbol());
        }
        captured.push('\n');
    }
    captured
}

#[test]
fn renders_title() {
    let out = render_with_fixture(120, 40, 0, "tests/fixtures/changelog/simple.md");
    assert!(out.contains("What's new"), "title missing, got:\n{out}");
}

#[test]
fn renders_at_minimum_terminal_size_without_truncation() {
    let out = render_with_fixture(80, 24, 0, "tests/fixtures/changelog/simple.md");
    assert!(out.contains("What's new"), "title missing, got:\n{out}");
    assert!(out.contains("close"), "close action missing, got:\n{out}");
}

#[test]
fn renders_strict_glyph_prefixes() {
    let out = render_with_fixture(120, 40, 0, "tests/fixtures/changelog/simple.md");
    assert!(out.contains("+ feat"), "feat prefix missing, got:\n{out}");
    assert!(out.contains("! fix"), "fix prefix missing, got:\n{out}");
}

#[test]
fn whats_new_shows_up_to_ten_recent_releases() {
    // The overlay caps its history view at the ten newest releases.
    // With twelve synthetic releases available, the eight newest plus
    // two more must render; the oldest two must fall outside the cap.
    let _guard = test_override_lock();
    crate::ui::theme::init_with_mode(1);
    let mut app = build_app();
    let paths = app.env().paths().cloned();
    crate::preferences::save_last_seen_version(paths.as_ref(), "0.0.1").unwrap();
    app.screen = Screen::WhatsNew(WhatsNewState { scroll: 0 });

    let mut fixture = String::new();
    for i in (1..=12).rev() {
        fixture.push_str(&format!("## 1.{i}.0 - 2026-01-01\n- feat: bullet\n\n"));
    }
    let _override = changelog::set_test_override(fixture);

    let backend = TestBackend::new(120, 200);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut anim = AnimationState::default();
    terminal
        .draw(|f| crate::ui::render(f, &mut app, &mut anim))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..200 {
        for x in 0..120 {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }

    assert!(out.contains("1.12.0"), "newest version must render");
    assert!(
        out.contains("1.3.0"),
        "tenth-newest version must render within cap"
    );
    assert!(
        !out.contains("1.2.0"),
        "1.2.0 sits beyond RECENT_CAP and must not render"
    );
    assert!(
        !out.contains("1.1.0"),
        "1.1.0 sits beyond RECENT_CAP and must not render"
    );
}

#[test]
fn renders_scroll_indicator_when_content_overflows() {
    let _guard = test_override_lock();
    crate::ui::theme::init_with_mode(1);
    let mut app = build_app();
    app.screen = Screen::WhatsNew(WhatsNewState { scroll: 5 });

    let _override =
        changelog::set_test_override("## 1.0.0\n- feat: a\n- feat: b\n- feat: c\n".into());
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut anim = AnimationState::default();
    terminal
        .draw(|f| crate::ui::render(f, &mut app, &mut anim))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..20 {
        for x in 0..120 {
            out.push_str(buf[(x, y)].symbol());
        }
    }
    assert!(
        out.contains('/'),
        "scroll indicator '/' missing, got:\n{out}"
    );
}
