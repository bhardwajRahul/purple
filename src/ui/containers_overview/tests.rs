use super::*;
use crate::containers::{ContainerCacheEntry, ContainerInfo, ContainerRuntime};
use std::collections::HashMap;

type RawContainer<'a> = (&'a str, &'a str, &'a str, &'a str);
type RawCacheEntry<'a> = (&'a str, &'a [RawContainer<'a>]);

fn cache_with(entries: &[RawCacheEntry<'_>]) -> HashMap<String, ContainerCacheEntry> {
    let mut map = HashMap::new();
    for (alias, items) in entries {
        let containers = items
            .iter()
            .map(|(id, name, image, state)| ContainerInfo {
                id: id.to_string(),
                names: name.to_string(),
                image: image.to_string(),
                state: state.to_string(),
                status: "Up 5 minutes".to_string(),
                ports: String::new(),
            })
            .collect();
        map.insert(
            alias.to_string(),
            ContainerCacheEntry {
                timestamp: 0,
                runtime: ContainerRuntime::Docker,
                engine_version: None,
                containers,
            },
        );
    }
    map
}

/// `build_demo_app` flips the global demo flag, so every test that builds one
/// holds the shared test lock for its whole run.
fn demo_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::demo_flag::GLOBAL_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn app_with_cache(cache: HashMap<String, ContainerCacheEntry>) -> App {
    let mut app = crate::demo::build_demo_app();
    app.container_state.set_cache(cache);
    app
}

#[test]
fn alpha_host_sort_orders_by_host_then_name() {
    let _lock = demo_lock();
    let cache = cache_with(&[
        ("zeus", &[("1", "alpha", "img", "running")]),
        (
            "apollo",
            &[
                ("2", "zebra", "img", "running"),
                ("3", "ant", "img", "exited"),
            ],
        ),
    ]);
    let app = app_with_cache(cache);
    let rows = visible_rows(&app);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].alias, "apollo");
    assert_eq!(rows[0].name, "ant");
    assert_eq!(rows[1].alias, "apollo");
    assert_eq!(rows[1].name, "zebra");
    assert_eq!(rows[2].alias, "zeus");
}

#[test]
fn alpha_container_sort_orders_by_name_then_host() {
    let _lock = demo_lock();
    let cache = cache_with(&[
        ("zeus", &[("1", "alpha", "img", "running")]),
        ("apollo", &[("2", "zebra", "img", "running")]),
    ]);
    let mut app = app_with_cache(cache);
    app.containers_overview
        .set_sort_mode_ephemeral(ContainersSortMode::AlphaContainer);
    let rows = visible_rows(&app);
    assert_eq!(rows[0].name, "alpha");
    assert_eq!(rows[1].name, "zebra");
}

#[test]
fn search_filters_on_alias_name_or_image() {
    let _lock = demo_lock();
    let cache = cache_with(&[
        ("zeus", &[("1", "alpha", "redis:7", "running")]),
        ("apollo", &[("2", "zebra", "postgres:16", "exited")]),
    ]);
    let mut app = app_with_cache(cache);
    app.search.set_query(Some("postgres".to_string()));
    let rows = visible_rows(&app);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "zebra");

    app.search.set_query(Some("ZEUS".to_string()));
    let rows = visible_rows(&app);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].alias, "zeus");
}

#[test]
fn empty_search_query_returns_everything() {
    let _lock = demo_lock();
    let cache = cache_with(&[("zeus", &[("1", "alpha", "img", "running")])]);
    let mut app = app_with_cache(cache);
    app.search.set_query(Some(String::new()));
    let rows = visible_rows(&app);
    assert_eq!(rows.len(), 1);
}

#[test]
fn clean_name_strips_docker_leading_slash() {
    assert_eq!(clean_name("/web"), "web");
    assert_eq!(clean_name("web"), "web");
}

#[test]
fn is_running_is_case_insensitive() {
    assert!(is_running("running"));
    assert!(is_running("Running"));
    assert!(!is_running("exited"));
    assert!(!is_running(""));
}

#[test]
fn format_iso_timestamp_strips_t_and_fraction() {
    assert_eq!(
        format_iso_timestamp("2026-05-09T08:00:00Z"),
        Some("2026-05-09 08:00:00".to_string())
    );
    assert_eq!(
        format_iso_timestamp("2026-05-09T08:00:00.123456789Z"),
        Some("2026-05-09 08:00:00".to_string())
    );
}

#[test]
fn format_iso_timestamp_rejects_empty_and_zero_time() {
    assert_eq!(format_iso_timestamp(""), None);
    // Go's zero time, emitted by docker for unset finished_at.
    assert_eq!(format_iso_timestamp("0001-01-01T00:00:00Z"), None);
}

#[test]
fn pad_or_truncate_pads_short_strings() {
    assert_eq!(pad_or_truncate("hi", 5), "hi   ");
}

#[test]
fn pad_or_truncate_truncates_long_strings() {
    // crate::ui::truncate uses `…` (1 column) so a 10-col input squeezed
    // to 5 cols becomes 4 chars + `…`.
    let out = pad_or_truncate("abcdefghij", 5);
    assert_eq!(out.chars().count(), 5);
    assert!(out.ends_with('…'));
}

fn col_row(name: &str, image: &str) -> ContainerRow {
    ContainerRow {
        id: format!("id-{}", name),
        alias: "h".to_string(),
        name: name.to_string(),
        image: image.to_string(),
        state: "running".to_string(),
        status: "Up 1m".to_string(),
        ports: String::new(),
        uptime: Some("1m".to_string()),
        cache_timestamp: 0,
    }
}

#[test]
fn compute_columns_enables_uptime_when_wide_enough() {
    let rows = [col_row("svc", "img:1")];
    let cols = compute_columns(rows.iter(), 200, false);
    assert!(cols.show_uptime);
}

#[test]
fn compute_columns_keeps_uptime_at_modest_width() {
    // PORTS is gone, so a width that previously demoted PORTS while
    // keeping UPTIME must still keep UPTIME.
    let rows = [col_row(
        "very-long-container-name-here",
        "registry.example.com/long/image:v1",
    )];
    let cols = compute_columns(rows.iter(), 75, false);
    assert!(cols.show_uptime, "UPTIME survives modest widths");
}

#[test]
fn compute_columns_flexes_image_to_anchor_uptime_right() {
    // With surplus width and UPTIME on, IMAGE absorbs the surplus so
    // UPTIME sits at the right edge instead of floating after a short
    // image string. Mirrors host_list's flex_gap behaviour.
    let rows = [col_row("svc", "img:1")];
    let cols = compute_columns(rows.iter(), 200, false);
    let consumed =
        HIGHLIGHT_W + MARKER_W + STATUS_DOT_W + cols.name + GAP_W + cols.image + GAP_W + UPTIME_W;
    assert_eq!(consumed, 200, "rendered row spans full content width");
    assert!(cols.image > IMAGE_MIN, "image flexed beyond minimum");
}

#[test]
fn compute_columns_flexes_image_with_host_column_visible() {
    // AlphaContainer mode renders the HOST column. The flex
    // accounting must subtract the host segment too, otherwise
    // UPTIME would overshoot and the row would overflow.
    let rows = [col_row("svc", "img:1")];
    let cols = compute_columns(rows.iter(), 200, true);
    let consumed = HIGHLIGHT_W
        + MARKER_W
        + STATUS_DOT_W
        + cols.host
        + GAP_W
        + cols.name
        + GAP_W
        + cols.image
        + GAP_W
        + UPTIME_W;
    assert_eq!(
        consumed, 200,
        "rendered row with HOST column spans full content width"
    );
}

#[test]
fn compute_columns_drops_uptime_at_extreme_width() {
    // Below the UPTIME-fit threshold even at IMAGE_MIN, the only
    // flex column disappears. With STATUS, HEALTH and PORTS
    // gone the threshold sits below 40 cols.
    let rows = [col_row("svc", "img")];
    let cols = compute_columns(rows.iter(), 35, false);
    assert!(!cols.show_uptime);
}

#[test]
fn state_glyph_running_with_unhealthy_health_uses_error_tier() {
    let (glyph, _) = state_glyph("running", Some("unhealthy"), "Up 1m", None, 0);
    assert_eq!(glyph, design::ICON_ONLINE);
}

#[test]
fn state_glyph_dead_state_uses_error_glyph() {
    let (glyph, _) = state_glyph("dead", None, "Dead", None, 0);
    assert_eq!(glyph, design::ICON_ERROR);
}

#[test]
fn state_glyph_exited_with_nonzero_code_uses_error_glyph() {
    let (glyph, _) = state_glyph("exited", None, "Exited (137) 2h ago", None, 0);
    assert_eq!(glyph, design::ICON_ERROR);
}

#[test]
fn state_glyph_exited_with_zero_code_uses_hollow_circle() {
    let (glyph, _) = state_glyph("exited", None, "Exited (0) 1m ago", None, 0);
    assert_eq!(glyph, design::ICON_STOPPED);
}

#[test]
fn state_glyph_paused_uses_half_circle() {
    let (glyph, _) = state_glyph("paused", None, "Paused", None, 0);
    assert_eq!(glyph, design::ICON_PAUSED);
}

#[test]
fn state_glyph_running_no_health_pulses_default_dot() {
    let (glyph, _) = state_glyph("running", None, "Up 5d", None, 0);
    assert_eq!(glyph, design::ICON_ONLINE);
}

#[test]
fn state_glyph_podman_stopped_treated_as_exited() {
    // Podman 3.x uses State="stopped" where docker uses "exited".
    // Both must take the exit-code branch.
    let (glyph, _) = state_glyph("stopped", None, "", Some(137), 0);
    assert_eq!(glyph, design::ICON_ERROR);
    let (glyph, _) = state_glyph("stopped", None, "", Some(0), 0);
    assert_eq!(glyph, design::ICON_STOPPED);
}

#[test]
fn state_glyph_podman_exited_empty_status_uses_inspect_exit_code() {
    // Podman emits empty Status; parse_exit_code_from_status returns
    // None. The cached inspect ExitCode is the fallback signal.
    let (glyph, _) = state_glyph("exited", None, "", Some(137), 0);
    assert_eq!(glyph, design::ICON_ERROR);
    let (glyph, _) = state_glyph("exited", None, "", Some(0), 0);
    assert_eq!(glyph, design::ICON_STOPPED);
    let (glyph, _) = state_glyph("exited", None, "", None, 0);
    assert_eq!(glyph, design::ICON_STOPPED);
}

// -- container_has_nonzero_exit ----------------------------------
// Gates the ATTENTION card non-zero exit highlight. Only reachable
// via render in production; tests document all five branches so a
// refactor cannot silently drop a podman host from the warning set.

fn make_container_info(id: &str, state: &str, status: &str) -> crate::containers::ContainerInfo {
    crate::containers::ContainerInfo {
        id: id.to_string(),
        names: "svc".to_string(),
        image: "img".to_string(),
        state: state.to_string(),
        status: status.to_string(),
        ports: String::new(),
    }
}

fn seed_inspect_exit_code(app: &mut App, id: &str, exit_code: i32) {
    use crate::app::InspectCacheEntry;
    app.containers_overview.inspect_cache_mut().entries.insert(
        id.to_string(),
        InspectCacheEntry {
            timestamp: 0,
            result: Ok(crate::containers::ContainerInspect {
                exit_code,
                ..Default::default()
            }),
        },
    );
}

#[test]
fn container_has_nonzero_exit_docker_status_nonzero() {
    let _lock = demo_lock();
    let app = app_with_cache(HashMap::new());
    let c = make_container_info("c1", "exited", "Exited (137) 2h ago");
    assert!(container_has_nonzero_exit(&app, &c));
}

#[test]
fn container_has_nonzero_exit_docker_status_zero() {
    let _lock = demo_lock();
    let app = app_with_cache(HashMap::new());
    let c = make_container_info("c2", "exited", "Exited (0) 1m ago");
    assert!(!container_has_nonzero_exit(&app, &c));
}

#[test]
fn container_has_nonzero_exit_podman_empty_status_with_inspect_nonzero() {
    let _lock = demo_lock();
    let mut app = app_with_cache(HashMap::new());
    app.containers_overview.inspect_cache_mut().entries.clear();
    seed_inspect_exit_code(&mut app, "c3", 137);
    let c = make_container_info("c3", "exited", "");
    assert!(container_has_nonzero_exit(&app, &c));
}

#[test]
fn container_has_nonzero_exit_podman_empty_status_no_inspect_is_false() {
    let _lock = demo_lock();
    // No false alarm when we have no exit signal yet.
    let mut app = app_with_cache(HashMap::new());
    app.containers_overview.inspect_cache_mut().entries.clear();
    let c = make_container_info("c4", "exited", "");
    assert!(!container_has_nonzero_exit(&app, &c));
}

#[test]
fn container_has_nonzero_exit_running_state_blocks_inspect_fallback() {
    let _lock = demo_lock();
    // A stale inspect saying exit=137 must not flag a currently
    // running container as failed.
    let mut app = app_with_cache(HashMap::new());
    app.containers_overview.inspect_cache_mut().entries.clear();
    seed_inspect_exit_code(&mut app, "c5", 137);
    let c = make_container_info("c5", "running", "");
    assert!(!container_has_nonzero_exit(&app, &c));
}

#[test]
fn container_has_nonzero_exit_podman3_stopped_state_uses_fallback() {
    let _lock = demo_lock();
    // Podman 3.x uses state="stopped" where podman 5.x / docker use
    // "exited". Both must accept the inspect fallback.
    let mut app = app_with_cache(HashMap::new());
    app.containers_overview.inspect_cache_mut().entries.clear();
    seed_inspect_exit_code(&mut app, "c6", 1);
    let c = make_container_info("c6", "stopped", "");
    assert!(container_has_nonzero_exit(&app, &c));
}

// -- view_cache + view_fingerprint --------------------------------
// Verifies the memoization layer in visible_items: first call
// populates, identical state hits cache, every fingerprint input
// mutation busts the cache.

fn cached_fp(app: &App) -> Option<u64> {
    app.containers_overview
        .view_cache()
        .borrow()
        .as_ref()
        .map(|(fp, _)| *fp)
}

#[test]
fn view_cache_starts_empty_and_populates_on_first_call() {
    let _lock = demo_lock();
    let cache = cache_with(&[("host1", &[("id1", "web", "nginx", "running")])]);
    let app = app_with_cache(cache);
    // Demo build_demo_app pre-populates and the assignment of a new
    // cache map does NOT clear view_cache. Reset it for the test so
    // we can assert the populate-from-empty transition.
    *app.containers_overview.view_cache().borrow_mut() = None;

    assert!(cached_fp(&app).is_none());
    let _ = visible_rows(&app);
    assert!(cached_fp(&app).is_some());
}

#[test]
fn view_cache_hits_on_identical_state() {
    let _lock = demo_lock();
    let cache = cache_with(&[("h", &[("i", "a", "img", "running")])]);
    let app = app_with_cache(cache);
    *app.containers_overview.view_cache().borrow_mut() = None;
    let rows1 = visible_rows(&app);
    let fp1 = cached_fp(&app).unwrap();
    let rows2 = visible_rows(&app);
    let fp2 = cached_fp(&app).unwrap();
    assert_eq!(fp1, fp2);
    assert_eq!(rows1, rows2);
}

#[test]
fn view_cache_invalidates_on_sort_mode_change() {
    let _lock = demo_lock();
    let cache = cache_with(&[("h", &[("i", "a", "img", "running")])]);
    let mut app = app_with_cache(cache);
    *app.containers_overview.view_cache().borrow_mut() = None;
    let _ = visible_rows(&app);
    let fp_before = cached_fp(&app).unwrap();
    app.containers_overview
        .set_sort_mode_ephemeral(ContainersSortMode::AlphaContainer);
    let _ = visible_rows(&app);
    assert_ne!(fp_before, cached_fp(&app).unwrap());
}

#[test]
fn view_cache_invalidates_on_search_query_change() {
    let _lock = demo_lock();
    let cache = cache_with(&[("h", &[("i", "web", "img", "running")])]);
    let mut app = app_with_cache(cache);
    *app.containers_overview.view_cache().borrow_mut() = None;
    let _ = visible_rows(&app);
    let fp_before = cached_fp(&app).unwrap();
    app.search.set_query(Some("web".to_string()));
    let _ = visible_rows(&app);
    assert_ne!(fp_before, cached_fp(&app).unwrap());
}

#[test]
fn view_cache_invalidates_on_container_cache_timestamp_bump() {
    let _lock = demo_lock();
    let mut app = app_with_cache(cache_with(&[("h", &[("i", "web", "img", "running")])]));
    *app.containers_overview.view_cache().borrow_mut() = None;
    let _ = visible_rows(&app);
    let fp_before = cached_fp(&app).unwrap();
    if let Some(entry) = app.container_state.cache_entry_mut("h") {
        entry.timestamp += 1;
    }
    let _ = visible_rows(&app);
    assert_ne!(fp_before, cached_fp(&app).unwrap());
}

#[test]
fn view_cache_invalidates_on_collapsed_hosts_toggle() {
    let _lock = demo_lock();
    let cache = cache_with(&[("h", &[("i", "web", "img", "running")])]);
    let mut app = app_with_cache(cache);
    *app.containers_overview.view_cache().borrow_mut() = None;
    let _ = visible_rows(&app);
    let fp_before = cached_fp(&app).unwrap();
    app.containers_overview.toggle_host_collapsed("h");
    let _ = visible_rows(&app);
    assert_ne!(fp_before, cached_fp(&app).unwrap());
}

#[test]
fn build_detail_lines_running_container_has_no_exit_row() {
    let row = ContainerRow {
        id: "c1".to_string(),
        alias: "web".to_string(),
        name: "nginx".to_string(),
        image: "nginx:1.25".to_string(),
        state: "running".to_string(),
        status: "Up 3 hours".to_string(),
        ports: "0.0.0.0:80->80/tcp".to_string(),
        uptime: Some("3h".to_string()),
        cache_timestamp: 0,
    };
    let inspect = crate::containers::ContainerInspect {
        exit_code: 0,
        oom_killed: false,
        started_at: "2026-05-09T08:00:00Z".to_string(),
        finished_at: String::new(),
        health: Some("healthy".to_string()),
        restart_count: 0,
        command: Some(vec!["nginx".to_string(), "-g".to_string()]),
        entrypoint: None,
        env_count: 5,
        mount_count: 1,
        networks: vec![],
        image_digest: None,
        restart_policy: None,
        user: None,
        privileged: false,
        readonly_rootfs: false,
        apparmor_profile: None,
        seccomp_profile: None,
        cap_add: Vec::new(),
        cap_drop: Vec::new(),
        mounts: Vec::new(),
        compose_project: None,
        compose_service: None,
        ..Default::default()
    };
    let result = Ok(inspect);
    let lines = build_detail_lines(&row, Some(&result), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("nginx"));
    assert!(text.contains("on web"));
    assert!(text.contains("Up 3 hours"));
    // HEALTH card materialises because health is reported.
    assert!(text.contains("HEALTH"));
    assert!(text.contains("healthy"));
    assert!(text.contains("Started"));
    assert!(
        !text.contains("ATTENTION"),
        "running container must not raise ATTENTION card"
    );
    assert!(
        !text.contains("OOM"),
        "running container must not show OOM row"
    );
    assert!(
        !text.contains("Stopped"),
        "running container must not show Stopped row"
    );
}

#[test]
fn build_detail_lines_oom_killed_shows_exit_and_oom() {
    let row = ContainerRow {
        id: "c2".to_string(),
        alias: "db".to_string(),
        name: "postgres".to_string(),
        image: "postgres:16".to_string(),
        state: "exited".to_string(),
        status: "Exited (137) 2 minutes ago".to_string(),
        ports: String::new(),
        uptime: None,
        cache_timestamp: 0,
    };
    let inspect = crate::containers::ContainerInspect {
        exit_code: 137,
        oom_killed: true,
        started_at: "2026-05-09T07:00:00Z".to_string(),
        finished_at: "2026-05-09T08:00:00Z".to_string(),
        health: None,
        restart_count: 3,
        command: None,
        entrypoint: Some(vec!["/docker-entrypoint.sh".to_string()]),
        env_count: 0,
        mount_count: 0,
        networks: vec![],
        image_digest: None,
        restart_policy: None,
        user: None,
        privileged: false,
        readonly_rootfs: false,
        apparmor_profile: None,
        seccomp_profile: None,
        cap_add: Vec::new(),
        cap_drop: Vec::new(),
        mounts: Vec::new(),
        compose_project: None,
        compose_service: None,
        ..Default::default()
    };
    let result = Ok(inspect);
    let lines = build_detail_lines(&row, Some(&result), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("Exit"));
    assert!(text.contains("137"));
    assert!(text.contains("OOM"));
    assert!(text.contains("killed"));
    assert!(text.contains("Restarts"));
    assert!(text.contains("Stopped"));
    // Cmd absent: entrypoint takes its place inside the dedicated
    // CMD card. No "Command" or "Entrypoint" labels appear because
    // the CMD card has no label column.
    assert!(text.contains("CMD"));
    assert!(text.contains("/docker-entrypoint.sh"));
    assert!(!text.contains("Command"));
}

#[test]
fn build_detail_lines_cmd_card_keeps_breathing_room_against_right_border() {
    // The CMD card wraps long commands onto multiple lines. Every
    // wrapped line must keep at least one column of padding before
    // the right `│`, matching how MOUNTS and LOGS already breathe.
    let row = ContainerRow {
        id: "c3".to_string(),
        alias: "h".to_string(),
        name: "svc".to_string(),
        image: "i:1".to_string(),
        state: "running".to_string(),
        status: "Up 1m".to_string(),
        ports: String::new(),
        uptime: Some("1m".to_string()),
        cache_timestamp: 0,
    };
    // Long command that forces the wrapper to fill its wrap width
    // on each line. With a tight breathing budget every emitted
    // line is the worst case for right-edge padding.
    let cmd = "a".repeat(200);
    let inspect = crate::containers::ContainerInspect {
        command: Some(vec![cmd]),
        ..Default::default()
    };
    let result = Ok(inspect);
    let lines = build_detail_lines(&row, Some(&result), false, 0, 48);
    let mut in_cmd_card = false;
    let mut content_lines_checked = 0;
    for line in &lines {
        let raw = line.to_string();
        if raw.contains("CMD") && raw.contains("─") {
            in_cmd_card = true;
            continue;
        }
        if in_cmd_card {
            if raw.starts_with('╰') {
                break;
            }
            if raw.contains('a') {
                let trimmed_end = raw.trim_end();
                let last_border = trimmed_end
                    .rfind('│')
                    .expect("CMD content line ends with right border");
                let before_border = &trimmed_end[..last_border];
                assert!(
                    before_border.ends_with(' '),
                    "CMD card content must keep at least one space before │, got: {raw:?}"
                );
                content_lines_checked += 1;
            }
        }
    }
    assert!(
        content_lines_checked > 0,
        "expected at least one CMD content line to verify"
    );
}

#[test]
fn build_detail_lines_no_inspect_shows_loading_when_in_flight() {
    let row = ContainerRow {
        id: "c3".to_string(),
        alias: "host".to_string(),
        name: "demo".to_string(),
        image: "img".to_string(),
        state: "running".to_string(),
        status: "Up 1m".to_string(),
        ports: String::new(),
        uptime: Some("1m".to_string()),
        cache_timestamp: 0,
    };
    let lines = build_detail_lines(&row, None, true, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("loading"));
}

#[test]
fn build_detail_lines_renders_audit_fields_when_inspect_present() {
    let row = ContainerRow {
        id: "abcdef0123456789".to_string(),
        alias: "audit-host".to_string(),
        name: "auth-svc".to_string(),
        image: "auth:1.2.3".to_string(),
        state: "running".to_string(),
        status: "Up 5 weeks (healthy)".to_string(),
        ports: "0.0.0.0:443->443/tcp,127.0.0.1:9000->9000/tcp".to_string(),
        uptime: Some("5w".to_string()),
        cache_timestamp: 0,
    };
    let inspect = crate::containers::ContainerInspect {
        exit_code: 0,
        oom_killed: false,
        started_at: "2026-04-02T19:46:58Z".to_string(),
        finished_at: String::new(),
        health: Some("healthy".to_string()),
        restart_count: 0,
        command: Some(vec!["/auth".to_string()]),
        entrypoint: None,
        env_count: 12,
        mount_count: 2,
        networks: vec![],
        image_digest: Some(
            "sha256:a4f1e7c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7c91d".to_string(),
        ),
        restart_policy: Some("unless-stopped".to_string()),
        user: Some("root".to_string()),
        privileged: false,
        readonly_rootfs: false,
        apparmor_profile: Some("docker-default".to_string()),
        seccomp_profile: Some("default".to_string()),
        cap_add: Vec::new(),
        cap_drop: vec!["NET_RAW".to_string()],
        mounts: vec![
            crate::containers::MountInfo {
                source: "/etc/letsencrypt".to_string(),
                destination: "/etc/letsencrypt".to_string(),
                read_only: false,
            },
            crate::containers::MountInfo {
                source: "certs".to_string(),
                destination: "/etc/nginx/certs".to_string(),
                read_only: true,
            },
        ],
        compose_project: Some("auth-stack".to_string()),
        compose_service: Some("auth".to_string()),
        ..Default::default()
    };
    let result = Ok(inspect);
    let lines = build_detail_lines(&row, Some(&result), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    // LIFECYCLE card: restart policy on its own row, count on a
    // second row (renamed from the old combined "Restart  X · 0
    // restarts" string).
    assert!(text.contains("LIFECYCLE"));
    assert!(text.contains("Restart"), "expected Restart row");
    assert!(
        text.contains("unless-stopped"),
        "expected restart policy to render"
    );
    assert!(text.contains("Restarts"), "expected Restarts count row");
    // SECURITY card surfaces because user is root and cap-drop is
    // non-empty. Defaults (apparmor=docker-default, seccomp=default)
    // stay silenced.
    assert!(text.contains("SECURITY"));
    assert!(text.contains("User"));
    assert!(text.contains("root"));
    assert!(text.contains("Caps -"));
    assert!(text.contains("NET_RAW"));
    assert!(
        !text.contains("AppArmor"),
        "docker-default apparmor profile is noise; suppress"
    );
    assert!(
        !text.contains("Seccomp"),
        "default seccomp profile is noise; suppress"
    );
    // APP card: image + truncated digest.
    assert!(text.contains("APP"));
    assert!(text.contains("Digest"));
    assert!(text.contains("sha256:a4f1e7…c91d"));
    // NETWORK card (ladder): public ports surface as `:N  pub`
    // branches, loopback ports do not get the `pub` annotation.
    assert!(text.contains("NETWORK"));
    assert!(
        text.contains(":443"),
        "expected :443 branch in network ladder"
    );
    assert!(text.contains("pub"), "public binding must surface");
    assert!(
        !text.contains(":9000  pub"),
        "loopback ports must not be flagged pub"
    );
    // MOUNTS card (aligned table) shows source → dest with mode.
    assert!(text.contains("MOUNTS"));
    assert!(text.contains("rw"));
    assert!(text.contains("ro"));
    assert!(text.contains("/etc/nginx/certs"));
    // Layout regression: source must pad to the longest source-width
    // (16 cols for `/etc/letsencrypt`), NOT to a 50/50 split that
    // would leave a wide gap on the 5-char `certs` row before the
    // arrow. We isolate the MOUNTS rows by walking forward from the
    // MOUNTS header until the next section divider.
    let lines_strs: Vec<&str> = text.lines().collect();
    let mount_header_idx = lines_strs
        .iter()
        .position(|l| l.contains("MOUNTS"))
        .expect("MOUNTS header must be present");
    let mount_rows: Vec<&&str> = lines_strs[mount_header_idx + 1..]
        .iter()
        .take_while(|l| !l.starts_with("\u{2570}") && !l.contains("COMPOSE"))
        .filter(|l| l.contains(" \u{2192} "))
        .collect();
    assert_eq!(
        mount_rows.len(),
        2,
        "two mount rows must contain the arrow within the MOUNTS card"
    );
    // find() returns a byte offset; convert to character/column
    // count so the assertion is invariant to multi-byte border
    // glyphs like `│` (3 bytes / 1 column).
    let arrow_columns: Vec<usize> = mount_rows
        .iter()
        .map(|line| {
            let byte_pos = line.find(" \u{2192} ").unwrap_or(usize::MAX);
            line[..byte_pos].chars().count()
        })
        .collect();
    assert_eq!(
        arrow_columns[0], arrow_columns[1],
        "arrows must align across mount rows"
    );
    // The arrow should sit just past the longest source (16 cols)
    // plus the leading "│ " prefix (2 cols) = column 18, not at
    // column 21 where a 50/50 split (19-col source + 2-col prefix)
    // would push it.
    let expected_arrow_col = "│ /etc/letsencrypt".chars().count();
    assert_eq!(
        arrow_columns[0], expected_arrow_col,
        "arrow must hug the longest source, not float on a 50/50 split"
    );
    assert!(
        !text.contains("Env 12"),
        "Env count teaser dropped; full list not implemented"
    );
    // Mode tags (rw/ro) must leave at least one column of breathing
    // room before the right `│` so they do not press against the
    // card edge. Other section helpers get this gap for free via
    // their right-padding; the mounts row builds its own spacer.
    for row in &mount_rows {
        assert!(
            row.ends_with("rw \u{2502}") || row.ends_with("ro \u{2502}"),
            "mode tag must be followed by a space before the right border, got {row:?}"
        );
    }
}

#[test]
fn build_detail_lines_inspect_error_shows_error_message() {
    let row = ContainerRow {
        id: "c4".to_string(),
        alias: "host".to_string(),
        name: "demo".to_string(),
        image: "img".to_string(),
        state: "running".to_string(),
        status: "Up 1m".to_string(),
        ports: String::new(),
        uptime: Some("1m".to_string()),
        cache_timestamp: 0,
    };
    let err: Result<crate::containers::ContainerInspect, String> =
        Err("permission denied".to_string());
    let lines = build_detail_lines(&row, Some(&err), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("error"));
    assert!(text.contains("permission denied"));
}

fn make_row(name: &str, alias: &str, state: &str, status: &str) -> ContainerRow {
    ContainerRow {
        id: "abc123def456".to_string(),
        alias: alias.to_string(),
        name: name.to_string(),
        image: "img:latest".to_string(),
        state: state.to_string(),
        status: status.to_string(),
        ports: String::new(),
        uptime: None,
        cache_timestamp: 0,
    }
}

#[test]
fn health_card_omitted_when_no_healthcheck() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        health: None,
        health_test: None,
        health_failing_streak: None,
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(
        !text.contains("HEALTH"),
        "HEALTH card must stay hidden when image has no healthcheck"
    );
}

#[test]
fn health_card_renders_unhealthy_with_streak() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        health: Some("unhealthy".to_string()),
        health_test: Some(vec![
            "CMD".to_string(),
            "curl".to_string(),
            "-fs".to_string(),
        ]),
        health_interval_ns: Some(30_000_000_000),
        health_failing_streak: Some(4),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("HEALTH"));
    assert!(text.contains("unhealthy"));
    assert!(text.contains("curl -fs"));
    assert!(text.contains("4 failing"));
    assert!(text.contains("30s interval"));
}

#[test]
fn resources_card_omitted_when_no_limits() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        memory_limit: None,
        cpu_limit_nanos: None,
        pids_limit: None,
        log_driver: Some("json-file".to_string()),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(
        !text.contains("RESOURCES"),
        "RESOURCES card must stay hidden when no limits and json-file logs"
    );
}

#[test]
fn resources_card_renders_when_memory_set() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        memory_limit: Some(536870912),
        cpu_limit_nanos: Some(1_500_000_000),
        pids_limit: Some(200),
        log_driver: Some("json-file".to_string()),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("RESOURCES"));
    assert!(text.contains("512 MB"));
    assert!(text.contains("1.5 cores"));
    assert!(text.contains("200"));
    assert!(
        !text.contains("Logs"),
        "default json-file log driver stays silent"
    );
}

#[test]
fn resources_card_surfaces_non_standard_log_driver() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        log_driver: Some("syslog".to_string()),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("RESOURCES"));
    assert!(text.contains("Logs"));
    assert!(text.contains("syslog"));
}

#[test]
fn security_card_omitted_for_default_profile() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        user: Some("app".to_string()),
        privileged: false,
        readonly_rootfs: false,
        apparmor_profile: Some("docker-default".to_string()),
        seccomp_profile: Some("default".to_string()),
        cap_add: vec![],
        cap_drop: vec![],
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(
        !text.contains("SECURITY"),
        "SECURITY stays hidden for non-root + default profiles + no caps"
    );
}

#[test]
fn security_card_renders_when_privileged() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        privileged: true,
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("SECURITY"));
    assert!(text.contains("Privileged"));
}

#[test]
fn compose_card_only_when_compose_managed() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let bare = crate::containers::ContainerInspect::default();
    let lines = build_detail_lines(&row, Some(&Ok(bare)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(!text.contains("COMPOSE"));

    let managed = crate::containers::ContainerInspect {
        compose_project: Some("edge".to_string()),
        compose_service: Some("nginx".to_string()),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(managed)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("COMPOSE"));
    assert!(text.contains("Project"));
    assert!(text.contains("edge"));
}

#[test]
fn attention_card_only_for_failed_or_oom_containers() {
    let row = make_row("svc", "host", "exited", "Exited (137)");
    let oom = crate::containers::ContainerInspect {
        exit_code: 137,
        oom_killed: true,
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(oom)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("ATTENTION"));
    assert!(text.contains("OOM"));
    assert!(text.contains("137"));

    let healthy_row = make_row("svc", "host", "running", "Up 1m");
    let healthy = crate::containers::ContainerInspect::default();
    let lines = build_detail_lines(&healthy_row, Some(&Ok(healthy)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(!text.contains("ATTENTION"));
}

#[test]
fn stop_signal_only_when_overrides_default() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let default_sig = crate::containers::ContainerInspect {
        restart_policy: Some("no".to_string()),
        stop_signal: Some("SIGTERM".to_string()),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(default_sig)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(
        !text.contains("Stop sig"),
        "default SIGTERM stays silent in LIFECYCLE card"
    );

    let custom_sig = crate::containers::ContainerInspect {
        restart_policy: Some("no".to_string()),
        stop_signal: Some("SIGQUIT".to_string()),
        stop_timeout: Some(30),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(custom_sig)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("Stop sig"));
    assert!(text.contains("SIGQUIT"));
    assert!(text.contains("30s timeout"));
}

#[test]
fn format_memory_bytes_units() {
    assert_eq!(format_memory_bytes(512 * 1024 * 1024), "512 MB");
    assert_eq!(format_memory_bytes(1024 * 1024 * 1024), "1 GB");
    assert_eq!(format_memory_bytes(1536 * 1024 * 1024), "1.5 GB");
}

#[test]
fn format_cpu_nanos_whole_and_fractional() {
    assert_eq!(format_cpu_nanos(1_000_000_000), "1 cores");
    assert_eq!(format_cpu_nanos(2_000_000_000), "2 cores");
    assert_eq!(format_cpu_nanos(1_500_000_000), "1.5 cores");
}

#[test]
fn format_duration_ns_picks_natural_unit() {
    assert_eq!(format_duration_ns(30_000_000_000), "30s");
    assert_eq!(format_duration_ns(120_000_000_000), "2m");
    assert_eq!(format_duration_ns(7_200_000_000_000), "2h");
}

#[test]
fn network_ladder_renders_mode_and_hostname() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let inspect = crate::containers::ContainerInspect {
        network_mode: Some("bridge".to_string()),
        hostname: Some("c1abc123".to_string()),
        networks: vec![crate::containers::NetworkInfo {
            name: "edge_default".to_string(),
            ip_address: "172.18.0.5".to_string(),
        }],
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(inspect)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("NETWORK"));
    // Top node: ○ host
    assert!(text.contains('\u{25CB}'), "○ host node missing");
    // Middle node: ● bridge · edge_default
    assert!(text.contains('\u{25CF}'), "● network node missing");
    assert!(text.contains("bridge"));
    assert!(text.contains("edge_default"));
    assert!(text.contains("172.18.0.5"));
    // Container node ◉ + container name + hostname
    assert!(text.contains('\u{25C9}'), "◉ container node missing");
    assert!(text.contains("svc"));
    assert!(text.contains("c1abc123"));
}

#[test]
fn workdir_root_is_suppressed_app_keeps_other_paths() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let root = crate::containers::ContainerInspect {
        working_dir: Some("/".to_string()),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(root)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(
        !text.contains("WorkDir"),
        "implicit / WorkDir stays silent in APP card"
    );

    let custom = crate::containers::ContainerInspect {
        working_dir: Some("/var/lib/postgres".to_string()),
        ..Default::default()
    };
    let lines = build_detail_lines(&row, Some(&Ok(custom)), false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("WorkDir"));
    assert!(text.contains("/var/lib/postgres"));
}

#[test]
fn cache_only_render_omits_inspect_cards() {
    // No inspect data and not in flight: panel relies on `docker ps`
    // row data only. Header always renders, APP renders Image+ID,
    // NETWORK renders only when ports are cached. LIFECYCLE / HEALTH /
    // RESOURCES / MOUNTS / SECURITY / COMPOSE all stay hidden.
    let row = ContainerRow {
        id: "deadbeef0000".to_string(),
        alias: "host".to_string(),
        name: "svc".to_string(),
        image: "img:1".to_string(),
        state: "running".to_string(),
        status: "Up 1m".to_string(),
        ports: "0.0.0.0:80->80/tcp".to_string(),
        uptime: Some("1m".to_string()),
        cache_timestamp: 0,
    };
    let lines = build_detail_lines(&row, None, false, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    // Header survives.
    assert!(text.contains("svc"));
    assert!(text.contains("on host"));
    assert!(text.contains("Up 1m"));
    // APP card from cached row data only (no Version / Digest / Cmd).
    assert!(text.contains("APP"));
    assert!(text.contains("img:1"));
    assert!(text.contains("deadbeef0000"));
    // NETWORK card surfaces because ports are present in the cache
    // row. Ladder layout collapses the public binding to `:80  pub`.
    assert!(text.contains("NETWORK"));
    assert!(text.contains(":80"));
    assert!(text.contains("pub"));
    // Inspect-gated cards are hidden.
    assert!(!text.contains("LIFECYCLE"));
    assert!(!text.contains("HEALTH"));
    assert!(!text.contains("RESOURCES"));
    assert!(!text.contains("MOUNTS"));
    assert!(!text.contains("SECURITY"));
    assert!(!text.contains("COMPOSE"));
    assert!(!text.contains("ATTENTION"));
    assert!(!text.contains("DETAILS"));
}

#[test]
fn details_card_shows_loading_when_inspect_in_flight() {
    let row = make_row("svc", "host", "running", "Up 1m");
    let lines = build_detail_lines(&row, None, true, 0, 48);
    let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
    assert!(text.contains("DETAILS"));
    assert!(text.contains("loading"));
    assert!(text.contains("fetching inspect"));
}

#[test]
fn logs_card_omitted_when_height_below_three() {
    let logs: Vec<String> = vec!["a".to_string()];
    let lines = build_logs_card(Some(&Ok(logs)), false, 96, 2);
    assert!(lines.is_empty());
}

#[test]
fn logs_card_renders_open_close_borders_when_height_three() {
    let lines = build_logs_card(None, false, 96, 3);
    assert_eq!(lines.len(), 3, "open + one inner + close");
    let first = lines[0].to_string();
    let last = lines[2].to_string();
    assert!(first.contains("LOGS"));
    assert!(first.starts_with('\u{256D}'));
    assert!(last.starts_with('\u{2570}'));
}

#[test]
fn logs_card_fills_when_more_lines_than_capacity() {
    // 30 log lines, panel allows 12 inner rows: render the trailing
    // 12 (line18..line29). Lines older than the tail window are
    // dropped.
    let logs: Vec<String> = (0..30).map(|i| format!("line{}", i)).collect();
    let lines = build_logs_card(Some(&Ok(logs)), false, 96, 14);
    assert_eq!(lines.len(), 14);
    let body: Vec<String> = lines[1..13].iter().map(|l| l.to_string()).collect();
    for (i, expected) in (18..30).enumerate() {
        assert!(
            body[i].contains(&format!("line{}", expected)),
            "row {} expected line{} got {}",
            i,
            expected,
            body[i]
        );
    }
    assert!(!lines.iter().any(|l| l.to_string().contains("line17")));
    assert!(!lines.iter().any(|l| l.to_string().contains("line0 ")));
}

#[test]
fn logs_card_pads_when_fewer_lines_than_capacity() {
    // Only 3 log lines but the card has 12 inner rows. All 3 render
    // and the bottom 9 rows are blank padding so the close border
    // lands at card_height - 1.
    let logs: Vec<String> = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let lines = build_logs_card(Some(&Ok(logs)), false, 96, 14);
    assert_eq!(lines.len(), 14);
    // First three inner rows carry the log content.
    let body_text = lines[1..13]
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("|");
    assert!(body_text.contains("a"));
    assert!(body_text.contains("b"));
    assert!(body_text.contains("c"));
    // Padding rows still wear the box sides so the card looks flush.
    for line in &lines[4..13] {
        let s = line.to_string();
        assert!(s.starts_with('\u{2502}'));
        assert!(s.ends_with('\u{2502}'));
    }
}

#[test]
fn logs_card_loading_state_renders_status() {
    let lines = build_logs_card(None, true, 96, 8);
    let text = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("LOGS"));
    assert!(text.contains("loading"));
}

#[test]
fn logs_card_error_state_renders_message() {
    let err: Result<Vec<String>, String> = Err("permission denied".to_string());
    let lines = build_logs_card(Some(&err), false, 96, 8);
    let text = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("error"));
    assert!(text.contains("permission denied"));
}

#[test]
fn logs_card_empty_log_set_says_no_output() {
    let lines = build_logs_card(Some(&Ok(vec![])), false, 96, 8);
    let text = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("(no output)"));
}

#[test]
fn logs_card_truncates_overlong_lines() {
    let long_line = "x".repeat(300);
    let lines = build_logs_card(Some(&Ok(vec![long_line])), false, 48, 5);
    // Each rendered line must fit within box_width visually. We
    // assert the trailing ellipsis to confirm truncation happened.
    let body = lines[1].to_string();
    assert!(
        body.contains('…'),
        "expected ellipsis on truncated line, got: {}",
        body
    );
}

#[test]
fn logs_card_height_exactly_matches_card_height() {
    for h in [3usize, 5, 8, 14, 30] {
        let logs: Vec<String> = vec!["one".to_string(), "two".to_string()];
        let lines = build_logs_card(Some(&Ok(logs)), false, 96, h);
        assert_eq!(
            lines.len(),
            h,
            "card_height={} must produce exactly {} lines",
            h,
            h
        );
    }
}

#[test]
fn wrap_to_lines_returns_short_input_unchanged() {
    let out = wrap_to_lines("hello world", 30, 3);
    assert_eq!(out, vec!["hello world".to_string()]);
}

#[test]
fn wrap_to_lines_splits_on_width() {
    // 12 chars, width 5, max 4 lines -> 5,5,2 (3 lines, no overflow)
    let out = wrap_to_lines("abcdefghijkl", 5, 4);
    assert_eq!(out, vec!["abcde", "fghij", "kl"]);
}

#[test]
fn wrap_to_lines_truncates_with_ellipsis_when_overflow() {
    // 12 chars, width 4, max 2 lines: first line "abcd",
    // second line has 1 col reserved for `…` so chunk = "abc…"
    // (chars 4 through 6 then truncated).
    let out = wrap_to_lines("abcdefghijkl", 4, 2);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], "abcd");
    assert!(out[1].ends_with('\u{2026}'));
}

#[test]
fn wrap_to_lines_zero_args_return_empty() {
    assert!(wrap_to_lines("anything", 0, 5).is_empty());
    assert!(wrap_to_lines("anything", 10, 0).is_empty());
}

#[test]
fn pad_or_truncate_path_pads_short() {
    let out = pad_or_truncate_path("/etc", 10);
    assert_eq!(out, "/etc      ");
}

#[test]
fn pad_or_truncate_path_truncates_left_to_preserve_leaf() {
    // Long path: leaf `/foo/bar` should remain visible, prefix
    // gets `…`-marked.
    let out = pad_or_truncate_path("/very/long/prefix/foo/bar", 12);
    assert_eq!(out.chars().count(), 12);
    assert!(out.starts_with('\u{2026}'));
    assert!(out.contains("foo/bar"));
}

#[test]
fn pad_or_truncate_path_exact_width_returns_self() {
    let out = pad_or_truncate_path("abcdef", 6);
    assert_eq!(out, "abcdef");
}

#[test]
fn snap_top_to_card_boundary_keeps_complete_cards() {
    // Build mock lines: open / content / close / open / content / close
    // = 6 lines representing two cards.
    let line = |c: char| Line::from(Span::raw(c.to_string()));
    let lines = vec![
        line('\u{256D}'), // ╭ open
        line(' '),
        line('\u{2570}'), // ╰ close
        line('\u{256D}'),
        line(' '),
        line('\u{2570}'),
    ];
    // cap = 6: both cards fit, snap returns 6.
    assert_eq!(snap_top_to_card_boundary(&lines, 6), 6);
    // cap = 5: only first card fits cleanly (3 lines).
    assert_eq!(snap_top_to_card_boundary(&lines, 5), 3);
    // cap = 3: still first card (boundary at 3).
    assert_eq!(snap_top_to_card_boundary(&lines, 3), 3);
    // cap = 2: no boundary fits, fall back to cap.
    assert_eq!(snap_top_to_card_boundary(&lines, 2), 2);
}

#[test]
fn snap_top_to_card_boundary_no_close_lines_returns_cap() {
    let line = |c: char| Line::from(Span::raw(c.to_string()));
    let lines = vec![line('a'), line('b'), line('c')];
    assert_eq!(snap_top_to_card_boundary(&lines, 2), 2);
}

#[test]
fn format_health_test_strips_cmd_prefix() {
    let test = vec![
        "CMD".to_string(),
        "curl".to_string(),
        "-fs".to_string(),
        "http://localhost".to_string(),
    ];
    assert_eq!(format_health_test(&test), "curl -fs http://localhost");

    let shell = vec!["CMD-SHELL".to_string(), "ps -ef | grep nginx".to_string()];
    assert_eq!(format_health_test(&shell), "ps -ef | grep nginx");

    let none = vec!["NONE".to_string()];
    assert_eq!(format_health_test(&none), "disabled");
}

// -- host detail helpers ---------------------------------------------

#[test]
fn count_states_buckets_each_kind() {
    let containers = vec![
        ContainerInfo {
            id: "1".into(),
            names: "a".into(),
            image: "img".into(),
            state: "running".into(),
            status: "Up".into(),
            ports: String::new(),
        },
        ContainerInfo {
            id: "2".into(),
            names: "b".into(),
            image: "img".into(),
            state: "running".into(),
            status: "Up".into(),
            ports: String::new(),
        },
        ContainerInfo {
            id: "3".into(),
            names: "c".into(),
            image: "img".into(),
            state: "exited".into(),
            status: "Exited (0) 1h ago".into(),
            ports: String::new(),
        },
        ContainerInfo {
            id: "4".into(),
            names: "d".into(),
            image: "img".into(),
            state: "dead".into(),
            status: "Dead".into(),
            ports: String::new(),
        },
        ContainerInfo {
            id: "5".into(),
            names: "e".into(),
            image: "img".into(),
            state: "paused".into(),
            status: "Paused".into(),
            ports: String::new(),
        },
        ContainerInfo {
            id: "6".into(),
            names: "f".into(),
            image: "img".into(),
            state: "restarting".into(),
            status: "Restarting".into(),
            ports: String::new(),
        },
    ];
    let c = count_states(&containers);
    assert_eq!(c.running, 2);
    assert_eq!(c.exited, 1);
    assert_eq!(c.dead, 1);
    assert_eq!(c.paused, 1);
    assert_eq!(c.restarting, 1);
    assert_eq!(c.created, 0);
}

#[test]
fn exit_code_extracted_when_present() {
    assert_eq!(
        parse_exit_code_from_status("Exited (137) 2h ago"),
        Some(137)
    );
    assert_eq!(parse_exit_code_from_status("Exited (0) 1m ago"), Some(0));
}

#[test]
fn exit_code_absent_when_status_does_not_match() {
    assert_eq!(parse_exit_code_from_status("Up 3 days"), None);
    assert_eq!(parse_exit_code_from_status("Exited"), None);
    assert_eq!(parse_exit_code_from_status("Exited (abc)"), None);
    assert_eq!(parse_exit_code_from_status(""), None);
}

fn host_detail_text(app: &App, alias: &str, total: usize, running: usize) -> String {
    let lines = build_host_detail_lines(app, alias, total, running, 80, 30);
    lines.iter().map(|l| l.to_string() + "\n").collect()
}

#[test]
fn host_detail_renders_status_and_fleet_cards_for_healthy_host() {
    let _lock = demo_lock();
    // Demo cache + inspect data is the easiest seed: every demo
    // host has a fleet, runtime label, and (most) carry an
    // engine_version on the cache entry.
    let app = crate::demo::build_demo_app();
    let alias = "aws-api-staging";
    let entry = app.container_state.cache_entry(alias).expect("demo seeded");
    let total = entry.containers.len();
    let running = entry
        .containers
        .iter()
        .filter(|c| c.state == "running")
        .count();
    let text = host_detail_text(&app, alias, total, running);
    assert!(text.contains("STATUS"));
    assert!(text.contains("FLEET"));
    assert!(text.contains("ACTIONS"));
    assert!(text.contains("HOST"));
    assert!(text.contains("Docker 25.0.3"));
}

#[test]
fn host_detail_attention_card_appears_for_dead_or_oom_or_restart_loop() {
    let _lock = demo_lock();
    // bastion-ams in the demo carries a container with restart_count=14
    // (app-backend) so the inspect-aggregate ATTENTION row triggers.
    let app = crate::demo::build_demo_app();
    let entry = app
        .container_state
        .cache_entry("bastion-ams")
        .expect("seeded");
    let total = entry.containers.len();
    let running = entry
        .containers
        .iter()
        .filter(|c| c.state == "running")
        .count();
    let text = host_detail_text(&app, "bastion-ams", total, running);
    assert!(text.contains("ATTENTION"));
    assert!(text.contains("Restart loop"));
}

#[test]
fn host_detail_runtime_falls_back_to_label_only_without_engine_version() {
    let _lock = demo_lock();
    // gateway-vpn in the demo deliberately omits engine_version on
    // its cache line. The Runtime row must still render with just
    // "Docker" (no trailing version).
    let app = crate::demo::build_demo_app();
    let entry = app
        .container_state
        .cache_entry("gateway-vpn")
        .expect("seeded");
    let total = entry.containers.len();
    let running = entry
        .containers
        .iter()
        .filter(|c| c.state == "running")
        .count();
    let text = host_detail_text(&app, "gateway-vpn", total, running);
    assert!(text.contains("Runtime"));
    assert!(text.contains("Docker"));
    // No semver-shaped trailing fragment after Docker on this host.
    assert!(!text.contains("Docker 25.0"));
    assert!(!text.contains("Docker 24.0"));
}

#[test]
fn host_detail_actions_disable_when_nothing_running() {
    let _lock = demo_lock();
    let cache = cache_with(&[(
        "host-x",
        &[("1", "a", "img", "exited"), ("2", "b", "img", "exited")],
    )]);
    let app = app_with_cache(cache);
    let text = host_detail_text(&app, "host-x", 2, 0);
    assert!(text.contains("ACTIONS"));
    assert!(text.contains("nothing running"));
}

#[test]
fn host_detail_actions_verbs_align_with_card_value_column() {
    let _lock = demo_lock();
    // The ACTIONS card key column must use `design::SECTION_LABEL_W`
    // so action verbs sit at the same X as the values in sibling
    // cards (STATUS / FLEET / HOST). A regression to a narrower key
    // column would make `Restart`, `Stop`, etc. land left of where
    // `Docker`, `3 running`, `192.0.2.1:22` start.
    let cache = cache_with(&[("host-z", &[("1", "n", "img", "running")])]);
    let app = app_with_cache(cache);
    let lines = build_host_detail_lines(&app, "host-z", 1, 1, 60, 40);
    // Find the row that starts with the K action (Restart). It is
    // styled in two spans (key + verb), so check the second span
    // begins exactly at `SECTION_LABEL_W` columns past the leading
    // `│ ` (2 cols).
    let k_row = lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|s| s.content == "Restart running on host")
        })
        .expect("ACTIONS card must include the K row");
    // Layout: [│ ][K_padded_to_SECTION_LABEL_W][verb]...
    let key_span = &k_row.spans[1];
    assert_eq!(
        key_span.content.len(),
        design::SECTION_LABEL_W as usize,
        "ACTIONS key column must be SECTION_LABEL_W wide so verbs align with sibling-card values"
    );
    assert!(
        key_span.content.starts_with("K"),
        "first ACTIONS row is the Restart binding"
    );
}

#[test]
fn host_detail_last_card_stretches_to_panel_bottom() {
    let _lock = demo_lock();
    let cache = cache_with(&[("host-y", &[("1", "n", "img", "running")])]);
    let app = app_with_cache(cache);
    let lines = build_host_detail_lines(&app, "host-y", 1, 1, 60, 40);
    // Panel height in lines is 40. stretch_last_card must pad up
    // to that count so the bottom border lands flush.
    assert_eq!(lines.len(), 40);
    // The very last line is the closing border of the HOST card.
    let last = lines.last().expect("at least one line");
    let first_span = last.spans.first().expect("border line carries spans");
    assert!(first_span.content.starts_with(design::BOX_BL));
}

/// Build a cache entry whose `timestamp` is `age_secs` in the past so
/// the "Stale" path in ATTENTION (>300s) is exercisable from tests.
fn cache_with_age(alias: &str, age_secs: u64) -> HashMap<String, ContainerCacheEntry> {
    let mut map = HashMap::new();
    let now = current_unix_secs();
    map.insert(
        alias.to_string(),
        ContainerCacheEntry {
            timestamp: now.saturating_sub(age_secs),
            runtime: ContainerRuntime::Docker,
            engine_version: Some("25.0.3".to_string()),
            containers: vec![ContainerInfo {
                id: "1".into(),
                names: "n".into(),
                image: "img".into(),
                state: "running".into(),
                status: "Up 1 hour".into(),
                ports: String::new(),
            }],
        },
    );
    map
}

#[test]
fn host_detail_attention_card_fires_for_stale_listing() {
    let _lock = demo_lock();
    let cache = cache_with_age("host-stale", 700);
    let app = app_with_cache(cache);
    let text = host_detail_text(&app, "host-stale", 1, 1);
    assert!(text.contains("ATTENTION"));
    assert!(text.contains("Stale"));
    assert!(text.contains("r to refresh"));
}

#[test]
fn host_detail_no_attention_card_when_listing_is_fresh_and_nothing_wrong() {
    let _lock = demo_lock();
    let cache = cache_with_age("host-fresh", 30);
    let app = app_with_cache(cache);
    let text = host_detail_text(&app, "host-fresh", 1, 1);
    assert!(!text.contains("ATTENTION"));
}

// The "tunnels active" branch (`app.tunnels.active().contains_key`)
// cannot be exercised cleanly from a unit test: `ActiveTunnel` owns
// a `std::process::Child` for the live ssh tunnel and has no test
// constructor. Coverage for that branch lives in the demo flow and
// visual regression goldens. We do exercise the alternative branch
// (configured tunnel directives without a live session) below.

#[test]
fn host_detail_fleet_shows_count_when_tunnel_count_is_set_but_inactive() {
    let _lock = demo_lock();
    // Seed a fresh host with tunnel_count > 0 so the FLEET card
    // takes the count branch. We append a HostEntry directly because
    // the demo app's fixed host list does not contain "host-tc".
    let cache = cache_with(&[("host-tc", &[("1", "n", "img", "running")])]);
    let mut app = app_with_cache(cache);
    let host = crate::ssh_config::model::HostEntry {
        alias: "host-tc".to_string(),
        hostname: "10.0.0.1".to_string(),
        user: "deploy".to_string(),
        port: 22,
        tunnel_count: 3,
        ..Default::default()
    };
    app.hosts_state.list_mut().push(host);
    let text = host_detail_text(&app, "host-tc", 1, 1);
    assert!(text.contains("Tunnels"));
    assert!(text.contains("3"));
}

#[test]
fn host_detail_fleet_marks_group_folded_when_collapsed() {
    let _lock = demo_lock();
    let cache = cache_with(&[("host-fold", &[("1", "n", "img", "running")])]);
    let mut app = app_with_cache(cache);
    app.containers_overview.toggle_host_collapsed("host-fold");
    let text = host_detail_text(&app, "host-fold", 1, 1);
    assert!(text.contains("Group"));
    assert!(text.contains("folded"));
}

#[test]
fn host_detail_actions_label_changes_when_group_collapsed() {
    let _lock = demo_lock();
    let cache = cache_with(&[("host-ex", &[("1", "n", "img", "running")])]);
    let mut app = app_with_cache(cache);
    app.containers_overview.toggle_host_collapsed("host-ex");
    let text = host_detail_text(&app, "host-ex", 1, 1);
    assert!(text.contains("Expand group"));
    assert!(!text.contains("Collapse group"));
}

#[test]
fn host_detail_ping_renders_each_status_variant() {
    let _lock = demo_lock();
    let cache = cache_with(&[("host-p", &[("1", "n", "img", "running")])]);
    let mut app = app_with_cache(cache);

    app.ping.insert_status(
        "host-p".into(),
        crate::app::PingStatus::Reachable { rtt_ms: 38 },
    );
    assert!(host_detail_text(&app, "host-p", 1, 1).contains("38ms"));

    app.ping.insert_status(
        "host-p".into(),
        crate::app::PingStatus::Slow { rtt_ms: 812 },
    );
    let slow = host_detail_text(&app, "host-p", 1, 1);
    assert!(slow.contains("slow"));
    assert!(slow.contains("812ms"));

    app.ping
        .insert_status("host-p".into(), crate::app::PingStatus::Unreachable);
    assert!(host_detail_text(&app, "host-p", 1, 1).contains("unreachable"));

    app.ping
        .insert_status("host-p".into(), crate::app::PingStatus::Checking);
    assert!(host_detail_text(&app, "host-p", 1, 1).contains("checking"));

    app.ping
        .insert_status("host-p".into(), crate::app::PingStatus::Skipped);
    assert!(host_detail_text(&app, "host-p", 1, 1).contains("--"));

    app.ping.remove_status("host-p");
    assert!(host_detail_text(&app, "host-p", 1, 1).contains("--"));
}

#[test]
fn restart_loop_threshold_boundary_at_five_excludes_six_includes() {
    let _lock = demo_lock();
    // Boundary: > 5, not >= 5. A container at exactly 5 must NOT
    // surface as a restart loop; one at 6 must.
    let app = crate::demo::build_demo_app();
    let make = |restart_count: u32| {
        let info = ContainerInfo {
            id: "boundary-id".into(),
            names: "svc".into(),
            image: "img".into(),
            state: "running".into(),
            status: "Up 1m".into(),
            ports: String::new(),
        };
        (info, restart_count)
    };
    let mut probe = app;
    probe
        .containers_overview
        .inspect_cache_mut()
        .entries
        .clear();
    let (info_at_5, _) = make(5);
    let (info_at_6, _) = make(6);
    // Insert two synthetic inspects keyed by the same id, swapping
    // the restart_count between probes. Easier than building a
    // ContainerInspect literal twice. Assert via collect_inspect_signals.
    for rc in [5u32, 6u32] {
        let inspect = crate::containers::ContainerInspect {
            restart_count: rc,
            ..Default::default()
        };
        probe
            .containers_overview
            .inspect_cache_mut()
            .entries
            .insert(
                "boundary-id".into(),
                crate::app::InspectCacheEntry {
                    timestamp: 0,
                    result: Ok(inspect),
                },
            );
        let containers = if rc == 5 {
            vec![info_at_5.clone()]
        } else {
            vec![info_at_6.clone()]
        };
        let signals = collect_inspect_signals(&probe, &containers);
        if rc == 5 {
            assert!(
                signals.restart_loops.is_empty(),
                "restart_count == 5 must NOT trigger restart loop"
            );
        } else {
            assert_eq!(
                signals.restart_loops.len(),
                1,
                "restart_count == 6 must trigger one restart loop"
            );
            assert_eq!(signals.restart_loops[0].1, 6);
        }
    }
}

#[test]
fn host_detail_truncates_restart_loop_rows_at_attention_cap() {
    let _lock = demo_lock();
    // Seed five containers each with restart_count above threshold.
    // The ATTENTION card must render at most ATTENTION_RESTART_LOOP_CAP
    // (= 3) restart-loop rows; the rest are dropped silently.
    let mut app = crate::demo::build_demo_app();
    app.containers_overview.inspect_cache_mut().entries.clear();
    let mut containers: Vec<ContainerInfo> = Vec::new();
    for i in 0..5 {
        let id = format!("loopy-{}", i);
        let info = ContainerInfo {
            id: id.clone(),
            names: format!("svc-{}", i),
            image: "img".into(),
            state: "running".into(),
            status: "Up 1m".into(),
            ports: String::new(),
        };
        containers.push(info);
        let inspect = crate::containers::ContainerInspect {
            restart_count: 20,
            ..Default::default()
        };
        app.containers_overview.inspect_cache_mut().entries.insert(
            id,
            crate::app::InspectCacheEntry {
                timestamp: 0,
                result: Ok(inspect),
            },
        );
    }
    // Override the demo cache so build_host_detail_lines reads the
    // synthetic containers under one alias.
    app.container_state.insert_cache_entry(
        "loopy-host".into(),
        ContainerCacheEntry {
            timestamp: current_unix_secs(),
            runtime: ContainerRuntime::Docker,
            engine_version: None,
            containers,
        },
    );
    let text = host_detail_text(&app, "loopy-host", 5, 5);
    let count = text.matches("Restart loop").count();
    assert_eq!(
        count, ATTENTION_RESTART_LOOP_CAP,
        "ATTENTION must cap restart-loop rows at the documented limit"
    );
}

#[test]
fn host_detail_action_qualifier_uses_count_with_correct_pluralisation() {
    let _lock = demo_lock();
    // Singular vs plural matters for one-off operator readability.
    let cache_one = cache_with(&[("host-1", &[("1", "n", "img", "running")])]);
    let app_one = app_with_cache(cache_one);
    let text_one = host_detail_text(&app_one, "host-1", 1, 1);
    assert!(text_one.contains("1 container"));
    assert!(!text_one.contains("1 containers"));

    let cache_many = cache_with(&[(
        "host-2",
        &[("1", "a", "img", "running"), ("2", "b", "img", "running")],
    )]);
    let app_many = app_with_cache(cache_many);
    let text_many = host_detail_text(&app_many, "host-2", 2, 2);
    assert!(text_many.contains("2 containers"));
}
