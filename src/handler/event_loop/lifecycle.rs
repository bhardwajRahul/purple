//! Tick, ping result and update-available events. Spinner animation,
//! ping TTL expiry, config-change detection, tunnel exit polling and
//! update notification all belong to the live-state lifecycle.

use std::time::Instant;

use crate::app::{self, App};

/// Handle `AppEvent::Tick` and `None` (timeout): spinner animation, ping TTL
/// expiry, config change detection and tunnel exit polling.
pub(crate) fn handle_tick(
    app: &mut App,
    anim: &mut crate::animation::AnimationState,
    vault_signing: bool,
    last_config_check: &mut Instant,
) {
    app.tick_status();
    app.tick_toast();
    let provider_syncing = !app.providers.syncing().is_empty();
    // Tick the spinner whenever something needs animation. Reachable hosts
    // drive the breathing online-dot pulse via `online_dot_pulsing(tick)`,
    // so they share the same monotonically-incrementing tick counter as
    // the spinner. saves a parallel tick driver. Active tunnels also
    // tick the spinner so the live chart wave has a continuous phase.
    let tunnels_animating =
        matches!(app.top_page, crate::app::TopPage::Tunnels) && !app.tunnels.active().is_empty();
    if anim.has_checking_hosts(app)
        || vault_signing
        || provider_syncing
        || anim.has_reachable_hosts(app)
        || tunnels_animating
    {
        anim.tick_spinner();
    }
    // Update the spinner character in the signing status text
    // so the spinner animates between VaultSignProgress events.
    if vault_signing
        && let Some(status) = app.status_center.status_mut()
        && status.sticky
        && !status.is_error()
    {
        let frame = crate::animation::SPINNER_FRAMES
            [anim.spinner_tick as usize % crate::animation::SPINNER_FRAMES.len()];
        if let Some(updated) = crate::replace_spinner_frame(&status.text, frame) {
            status.text = updated;
        }
    }
    // Animate the provider-sync footer: rotate the leading spinner frame on
    // each tick while a sync is in flight. The status is non-sticky (Info),
    // so we match by spinner-prefix instead of the sticky flag like
    // vault_signing does.
    if provider_syncing && let Some(status) = app.status_center.status_mut() {
        let frame = crate::animation::SPINNER_FRAMES
            [anim.spinner_tick as usize % crate::animation::SPINNER_FRAMES.len()];
        if let Some(updated) = crate::replace_spinner_frame(&status.text, frame) {
            status.text = updated;
            // Refresh created_at so the Info-class footer message does not
            // expire by length-proportional timeout in the gap between
            // sync_complete events. The message stays alive as long as at
            // least one provider is still syncing.
            status.created_at = std::time::Instant::now();
        }
    }
    // Throttle config file stat() to every 4 seconds
    if last_config_check.elapsed() >= std::time::Duration::from_secs(4) {
        app.check_config_changed();
        app.check_keys_changed();
        *last_config_check = Instant::now();
    }
    // Poll active tunnels for exit
    let exited = app.poll_tunnels();
    for (_alias, msg, is_error) in exited {
        if is_error {
            app.notify_background_error(msg);
        } else {
            app.notify_background(msg);
        }
    }
}

/// Handle `AppEvent::PingResult`.
pub(crate) fn handle_ping_result(
    app: &mut App,
    alias: String,
    rtt_ms: Option<u32>,
    generation: u64,
) {
    if generation == app.ping.generation() {
        let status = app::classify_ping(rtt_ms, app.ping.slow_threshold_ms());
        let now = Instant::now();
        log::debug!(
            "[external] ping-result: {} → {:?} (rtt={:?}ms, gen={})",
            alias,
            status,
            rtt_ms,
            generation
        );
        app.ping.insert_status(alias.clone(), status.clone());
        app.ping.record_check(alias.clone(), now);
        // Propagate bastion status to all ProxyJump dependents.
        app::propagate_ping_to_dependents(
            app.hosts_state.list(),
            app.ping.status_map_mut(),
            &alias,
            &status,
        );
        let mut propagated = 0usize;
        for h in app.hosts_state.list() {
            if h.proxy_jump == alias {
                app.ping.record_check(h.alias.clone(), now);
                propagated += 1;
            }
        }
        if propagated > 0 {
            log::debug!(
                "[purple] ping-result: propagated bastion {} status+timestamp to {} dependent(s)",
                alias,
                propagated
            );
        }
        // Update live filter/sort as results arrive
        if app.ping.filter_down_only() {
            app.apply_filter();
        }
        if app.hosts_state.sort_mode() == app::SortMode::Status {
            app.apply_sort();
        }
        // Update "last checked" timestamp when all pings are done
        if !app.ping.status_is_empty()
            && app
                .ping
                .status_map()
                .values()
                .all(|s| !matches!(s, app::PingStatus::Checking))
        {
            app.ping.set_checked_at(Some(Instant::now()));
        }
    }
}

/// Handle `AppEvent::UpdateAvailable`.
pub(crate) fn handle_update_available(app: &mut App, version: String, headline: Option<String>) {
    app.update.announce(version, headline);
}
