use super::*;

#[test]
fn color_slot_resolves_truecolor() {
    let slot = ColorSlot {
        truecolor: Some(Color::Rgb(147, 51, 234)),
        ansi16: Some(Color::Magenta),
        add_modifier: None,
        remove_modifier: None,
    };
    let style = slot.to_style(2);
    assert_eq!(style.fg, Some(Color::Rgb(147, 51, 234)));
}

#[test]
fn color_slot_resolves_ansi16() {
    let slot = ColorSlot {
        truecolor: Some(Color::Rgb(147, 51, 234)),
        ansi16: Some(Color::Magenta),
        add_modifier: None,
        remove_modifier: None,
    };
    let style = slot.to_style(1);
    assert_eq!(style.fg, Some(Color::Magenta));
}

#[test]
fn color_slot_resolves_no_color() {
    let slot = ColorSlot {
        truecolor: Some(Color::Rgb(147, 51, 234)),
        ansi16: Some(Color::Magenta),
        add_modifier: Some(Modifier::BOLD),
        remove_modifier: None,
    };
    let style = slot.to_style(0);
    assert_eq!(style.fg, None);
    assert!(style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn color_slot_remove_modifier() {
    let slot = ColorSlot {
        truecolor: None,
        ansi16: None,
        add_modifier: Some(Modifier::BOLD),
        remove_modifier: Some(Modifier::DIM),
    };
    let style = slot.to_style(0);
    assert!(style.add_modifier.contains(Modifier::BOLD));
    assert!(style.sub_modifier.contains(Modifier::DIM));
}

#[test]
fn color_slot_bg_resolves_truecolor() {
    let slot = ColorSlot {
        truecolor: Some(Color::Rgb(147, 51, 234)),
        ansi16: Some(Color::Magenta),
        add_modifier: None,
        remove_modifier: None,
    };
    let style = slot.to_style_bg(2);
    assert_eq!(style.bg, Some(Color::Rgb(147, 51, 234)));
    assert_eq!(style.fg, None);
}

#[test]
fn theme_def_purple_default_accent() {
    let t = ThemeDef::purple();
    assert_eq!(t.accent.truecolor, Some(Color::Rgb(147, 51, 234)));
}

#[test]
fn theme_error_returns_bold_red_truecolor() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    let style = error();
    assert_eq!(style.fg, Some(Color::Rgb(239, 68, 68)));
    assert!(style.add_modifier.contains(Modifier::BOLD));
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn theme_selected_row_removes_dim() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::purple());
    let style = selected_row();
    assert!(style.sub_modifier.contains(Modifier::DIM));
    assert_eq!(style.bg, Some(Color::Rgb(147, 51, 234)));
    assert_eq!(style.fg, Some(Color::White));
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());
}

#[test]
fn init_no_color_env_forces_mode_zero() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // NO_COLOR wins over COLORTERM. The injected Env (no home) means load_theme
    // finds no prefs file, so this exercises only the env-driven color decision.
    let env = crate::runtime::env::Env::for_test("/nonexistent-purple-home")
        .with_var("NO_COLOR", "1")
        .with_var("COLORTERM", "truecolor");
    init(&env);
    assert_eq!(color_mode(), 0);
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());
}

#[test]
fn init_colorterm_truecolor_sets_mode_two() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    COLOR_MODE.store(1, Ordering::Release);
    let env = crate::runtime::env::Env::for_test("/nonexistent-purple-home")
        .with_var("COLORTERM", "truecolor");
    init(&env);
    assert_eq!(color_mode(), 2);
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());
}

#[test]
fn theme_no_color_mode_ignores_colors() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(0);
    let style = error();
    assert_eq!(style.fg, None);
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn all_builtin_themes_have_unique_names() {
    let themes = ThemeDef::builtins();
    let mut names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), themes.len());
}

#[test]
fn all_builtin_themes_have_required_slots() {
    for t in ThemeDef::builtins() {
        if t.name != "No Color" {
            assert!(
                t.accent.truecolor.is_some(),
                "{} accent missing truecolor",
                t.name
            );
            assert!(
                t.error.truecolor.is_some(),
                "{} error missing truecolor",
                t.name
            );
        }
    }
}

#[test]
fn builtin_count_is_11() {
    assert_eq!(ThemeDef::builtins().len(), 11);
}

#[test]
fn no_color_theme_has_no_truecolor() {
    let t = ThemeDef::no_color();
    assert!(t.accent.truecolor.is_none());
    assert!(t.error.truecolor.is_none());
    assert!(t.success.truecolor.is_none());
}

#[test]
fn find_builtin_case_insensitive() {
    assert!(ThemeDef::find_builtin("catppuccin mocha").is_some());
    assert!(ThemeDef::find_builtin("CATPPUCCIN MOCHA").is_some());
    assert!(ThemeDef::find_builtin("nonexistent").is_none());
}

#[test]
fn parse_custom_theme_valid() {
    let toml = "name = \"My Theme\"\n\
                 accent = \"#ff0000\"\n\
                 success = \"#00ff00\"\n\
                 warning = \"#ffff00\"\n\
                 error = \"#ff0000\"\n";
    let t = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(t.name, "My Theme");
    assert_eq!(t.accent.truecolor, Some(Color::Rgb(255, 0, 0)));
    assert_eq!(t.success.truecolor, Some(Color::Rgb(0, 255, 0)));
    assert_eq!(t.warning.truecolor, Some(Color::Rgb(255, 255, 0)));
    assert_eq!(t.error.truecolor, Some(Color::Rgb(255, 0, 0)));
}

#[test]
fn parse_custom_theme_partial_fills_from_purple() {
    let toml = "name = \"Partial\"\n\
                 accent = \"#ff0000\"\n";
    let t = ThemeDef::parse_toml(toml).unwrap();
    let purple = ThemeDef::purple();
    assert_eq!(t.accent.truecolor, Some(Color::Rgb(255, 0, 0)));
    // success should fall back to Purple default
    assert_eq!(t.success.truecolor, purple.success.truecolor);
}

#[test]
fn parse_custom_theme_invalid_hex_skipped() {
    let toml = "name = \"BadHex\"\n\
                 accent = \"not-hex\"\n";
    let t = ThemeDef::parse_toml(toml).unwrap();
    let purple = ThemeDef::purple();
    // Falls back to Purple default since parse_hex fails
    assert_eq!(t.accent.truecolor, purple.accent.truecolor);
}

#[test]
fn parse_custom_theme_missing_name() {
    let toml = "accent = \"#ff0000\"\n";
    assert!(ThemeDef::parse_toml(toml).is_none());
}

#[test]
fn parse_hex_color_valid() {
    assert_eq!(parse_hex("#ff0000"), Some(Color::Rgb(255, 0, 0)));
}

#[test]
fn parse_hex_color_invalid() {
    assert!(parse_hex("not-hex").is_none());
    assert!(parse_hex("#gg0000").is_none());
    assert!(parse_hex("#fff").is_none());
}

#[test]
fn parse_ansi_color_name() {
    assert_eq!(parse_ansi_name("Red"), Some(Color::Red));
    assert_eq!(parse_ansi_name("blue"), Some(Color::Blue));
    assert_eq!(parse_ansi_name("DarkGray"), Some(Color::DarkGray));
    assert_eq!(parse_ansi_name("dark_gray"), Some(Color::DarkGray));
    assert_eq!(parse_ansi_name("unknown"), None);
}

// --- auto_ansi16 tests ---

#[test]
fn auto_ansi16_black_for_low_luminance() {
    assert_eq!(auto_ansi16(Color::Rgb(10, 10, 10)), Some(Color::Black));
}

#[test]
fn auto_ansi16_bright_red() {
    assert_eq!(auto_ansi16(Color::Rgb(200, 50, 30)), Some(Color::LightRed));
}

#[test]
fn auto_ansi16_dark_red() {
    assert_eq!(auto_ansi16(Color::Rgb(150, 50, 30)), Some(Color::Red));
}

#[test]
fn auto_ansi16_bright_green() {
    assert_eq!(
        auto_ansi16(Color::Rgb(50, 200, 30)),
        Some(Color::LightGreen)
    );
}

#[test]
fn auto_ansi16_bright_blue() {
    assert_eq!(auto_ansi16(Color::Rgb(30, 50, 200)), Some(Color::LightBlue));
}

#[test]
fn auto_ansi16_yellow() {
    assert_eq!(auto_ansi16(Color::Rgb(200, 200, 50)), Some(Color::Yellow));
}

#[test]
fn auto_ansi16_magenta() {
    assert_eq!(auto_ansi16(Color::Rgb(200, 50, 200)), Some(Color::Magenta));
}

#[test]
fn auto_ansi16_cyan() {
    assert_eq!(auto_ansi16(Color::Rgb(50, 200, 200)), Some(Color::Cyan));
}

#[test]
fn auto_ansi16_white() {
    assert_eq!(auto_ansi16(Color::Rgb(230, 230, 230)), Some(Color::White));
}

#[test]
fn auto_ansi16_gray() {
    assert_eq!(auto_ansi16(Color::Rgb(120, 120, 120)), Some(Color::Gray));
}

#[test]
fn auto_ansi16_passthrough_non_rgb() {
    assert_eq!(auto_ansi16(Color::Red), Some(Color::Red));
}

// --- find_builtin canonical name test ---

#[test]
fn find_builtin_returns_canonical_name() {
    let theme = ThemeDef::find_builtin("catppuccin mocha").unwrap();
    assert_eq!(theme.name, "Catppuccin Mocha");
}

// --- parse_toml edge case tests ---

#[test]
fn parse_toml_inline_comment() {
    let toml = "name = \"Commented\"\naccent = \"#ff0000\" # brand color\n";
    let theme = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(theme.accent.truecolor, Some(Color::Rgb(255, 0, 0)));
}

#[test]
fn parse_toml_ansi_override() {
    let toml = "name = \"Ansi\"\naccent = \"#ff0000\"\naccent_ansi = \"Blue\"\n";
    let theme = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(theme.accent.ansi16, Some(Color::Blue));
}

#[test]
fn parse_toml_duplicate_keys_last_wins() {
    let toml = "name = \"First\"\nname = \"Second\"\naccent = \"#ff0000\"\n";
    let theme = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(theme.name, "Second");
}

#[test]
fn parse_toml_footer_key_bg_maps_to_footer_key() {
    let toml = "name = \"Footer\"\nfooter_key_bg = \"#112233\"\n";
    let theme = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(theme.footer_key.truecolor, Some(Color::Rgb(17, 34, 51)));
}

#[test]
fn parse_toml_badge_bg_maps_to_badge() {
    let toml = "name = \"Badge\"\nbadge_bg = \"#aabbcc\"\n";
    let theme = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(theme.badge.truecolor, Some(Color::Rgb(170, 187, 204)));
}

#[test]
fn parse_toml_ignores_unknown_keys() {
    let toml = "name = \"Unknown\"\nunknown_key = \"value\"\naccent = \"#ff0000\"\n";
    let theme = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(theme.accent.truecolor, Some(Color::Rgb(255, 0, 0)));
}

#[test]
fn parse_toml_empty_lines_and_comments() {
    let toml =
        "\n# This is a comment\n\nname = \"Spaced\"\n\n# accent color\naccent = \"#00ff00\"\n";
    let theme = ThemeDef::parse_toml(toml).unwrap();
    assert_eq!(theme.name, "Spaced");
    assert_eq!(theme.accent.truecolor, Some(Color::Rgb(0, 255, 0)));
}

#[test]
fn catppuccin_mocha_selected_row_has_dark_fg() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::catppuccin_mocha());
    let style = selected_row();
    // Should use Mocha Base (#1E1E2E) not White
    assert_eq!(style.fg, Some(Color::Rgb(30, 30, 46)));
    assert_eq!(style.bg, Some(Color::Rgb(137, 180, 250)));
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());
}

#[test]
fn catppuccin_latte_footer_key_has_dark_fg() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::catppuccin_latte());
    let style = footer_key();
    // Should use Latte Text (#4C4F69) not White
    assert_eq!(style.fg, Some(Color::Rgb(76, 79, 105)));
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());
}

#[test]
fn gruvbox_accent_warning_ansi16_differ() {
    let t = ThemeDef::gruvbox_dark();
    // Accent should no longer collide with warning in ANSI 16
    assert_ne!(t.accent.ansi16, t.warning.ansi16);
}

// --- NO_COLOR override test ---

#[test]
fn no_color_mode_forces_no_color_theme() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    COLOR_MODE.store(0, Ordering::Release);
    set_theme(ThemeDef::no_color());

    let style = error();
    assert_eq!(style.fg, None);
    assert!(style.add_modifier.contains(Modifier::BOLD));

    let style = success();
    assert_eq!(style.fg, None);

    let style = warning();
    assert_eq!(style.fg, None);

    let style = border();
    assert_eq!(style.fg, None);

    // Cleanup
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());
}

// --- online_dot_pulsing breathing cycle ---

#[test]
fn online_dot_pulsing_starts_at_regular_mid_brightness() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Visual regression goldens render at tick=0; verify tick=0 produces
    // the Regular (mid) state — neither BOLD nor DIM — so the pulse cycle
    // begins at the visually neutral point and goldens stay reproducible.
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());

    let mid = online_dot_pulsing(0);
    assert!(!mid.add_modifier.contains(Modifier::BOLD));
    assert!(!mid.add_modifier.contains(Modifier::DIM));
}

#[test]
fn online_dot_pulsing_cycles_through_peak_mid_and_trough() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // 30-tick period. tick=0 is verified by the dedicated mid-baseline
    // test above; here we cover the three OTHER cardinal phases so
    // together the suite pins all four quadrants of the sine curve
    // without a duplicated tick=0 assertion.
    //   tick=7  sin≈+0.99 → peak  (BOLD via success slot, which is BOLD by default)
    //   tick=15 sin(pi)=0 → mid   (Regular: BOLD explicitly removed, no DIM)
    //   tick=22 sin≈-0.99 → trough (DIM via success_dim slot)
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());

    assert!(online_dot_pulsing(7).add_modifier.contains(Modifier::BOLD));

    let mid = online_dot_pulsing(15);
    assert!(!mid.add_modifier.contains(Modifier::BOLD));
    assert!(!mid.add_modifier.contains(Modifier::DIM));

    assert!(online_dot_pulsing(22).add_modifier.contains(Modifier::DIM));
}

#[test]
fn online_dot_pulsing_repeats_every_period() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    COLOR_MODE.store(1, Ordering::Release);
    set_theme(ThemeDef::purple());

    assert_eq!(online_dot_pulsing(0), online_dot_pulsing(30));
    assert_eq!(online_dot_pulsing(7), online_dot_pulsing(37));
}

#[test]
fn online_dot_pulsing_truecolor_lerps_brightness() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // In truecolor mode the pulse path returns a smooth per-channel RGB
    // lerp instead of BOLD/DIM modifier flips. Verify:
    //   1. Output is a truecolor `Rgb(r,g,b)` foreground, not a palette colour.
    //   2. No BOLD / DIM modifier is set — the signal is hue-brightness only.
    //   3. The peak frame is brighter (higher channel sum) than the trough,
    //      confirming the sin-driven alpha actually modulates the colour.
    COLOR_MODE.store(2, Ordering::Release);
    set_theme(ThemeDef::purple());

    let peak = online_dot_pulsing(7); // sin≈+0.99 → peak brightness
    let trough = online_dot_pulsing(22); // sin≈-0.99 → trough brightness

    match peak.fg {
        Some(Color::Rgb(..)) => {}
        other => panic!("truecolor pulse must emit Rgb fg, got {:?}", other),
    }
    assert!(!peak.add_modifier.contains(Modifier::BOLD));
    assert!(!peak.add_modifier.contains(Modifier::DIM));
    assert!(!trough.add_modifier.contains(Modifier::BOLD));
    assert!(!trough.add_modifier.contains(Modifier::DIM));

    let sum = |fg: Option<Color>| match fg {
        Some(Color::Rgb(r, g, b)) => r as u32 + g as u32 + b as u32,
        _ => 0,
    };
    assert!(
        sum(peak.fg) > sum(trough.fg),
        "peak channel sum must exceed trough (peak={:?} trough={:?})",
        peak.fg,
        trough.fg
    );

    COLOR_MODE.store(1, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Terminal-capability detection
// ---------------------------------------------------------------------------

#[test]
fn detects_colored_underline_opts_out_for_apple_terminal() {
    assert!(!detects_colored_underline(Some("Apple_Terminal")));
}

#[test]
fn detects_colored_underline_defaults_true_for_unknown_or_modern() {
    // Unknown / unset terminal -> assume modern. Add new opt-outs by name only.
    assert!(detects_colored_underline(None));
    assert!(detects_colored_underline(Some("ghostty")));
    assert!(detects_colored_underline(Some("iTerm.app")));
    assert!(detects_colored_underline(Some("WezTerm")));
    assert!(detects_colored_underline(Some("vscode")));
}

// ---------------------------------------------------------------------------
// accent / border_dim split
// ---------------------------------------------------------------------------

#[test]
fn accent_reads_from_accent_slot_not_border() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::purple());
    let purple = accent();
    let dim = border_dim();
    assert_eq!(
        purple.fg,
        Some(Color::Rgb(147, 51, 234)),
        "accent() should read the accent slot (brand purple)"
    );
    assert_eq!(
        dim.fg, None,
        "border_dim() should read the border slot (no fg on default Purple theme)"
    );
    assert!(
        dim.add_modifier.contains(Modifier::DIM),
        "border_dim() should carry the DIM modifier"
    );
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn accent_bold_matches_accent_with_bold_modifier() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::purple());
    let plain = accent();
    let bold = accent_bold();
    assert_eq!(plain.fg, bold.fg);
    assert!(bold.add_modifier.contains(Modifier::BOLD));
    assert!(!plain.add_modifier.contains(Modifier::BOLD));
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn tunnel_active_and_tag_user_match_accent() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::purple());
    let a = accent();
    assert_eq!(tunnel_active().fg, a.fg);
    assert_eq!(tag_user().fg, a.fg);
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn section_card_title_takes_the_border_tone_with_bold() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::purple());
    let title = section_card_title();
    let frame = border();
    assert_eq!(title.fg, frame.fg, "title color matches the card frame");
    assert_eq!(
        title.add_modifier.contains(Modifier::DIM),
        frame.add_modifier.contains(Modifier::DIM),
        "title carries the frame's dim tone"
    );
    assert!(
        title.add_modifier.contains(Modifier::BOLD),
        "title stays bold for scannability"
    );
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn set_color_mode_overrides_the_global_color_mode() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    set_color_mode(2);
    assert_eq!(color_mode(), 2, "truecolor forced regardless of terminal");
    set_color_mode(0);
    assert_eq!(color_mode(), 0, "no-color forced");
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn healthy_matches_online_dot() {
    let _global_test_lock = crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    init_with_mode(2);
    set_theme(ThemeDef::purple());
    // healthy() and online_dot() share the success_dim slot today.
    assert_eq!(healthy().fg, online_dot().fg);
    COLOR_MODE.store(1, Ordering::Release);
}

#[test]
fn parse_toml_falls_back_on_malformed_hex() {
    // Caller doesn't need to capture the log; just assert the parser
    // accepts the file and falls back to the Purple default.
    let content = "name = \"BadHex\"\naccent = \"not-a-color\"\n";
    let theme = ThemeDef::parse_toml(content).expect("parse_toml should succeed with fallback");
    assert_eq!(theme.accent.truecolor, Some(Color::Rgb(147, 51, 234)));
}
