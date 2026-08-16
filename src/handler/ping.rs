use std::sync::mpsc;

use super::ctx::Notify;
use crate::app::{App, HostState, PingState, StatusCenter};
use crate::event::AppEvent;
use crate::ping;

/// The slice of App the ping handlers touch: the per-host ping state, the host
/// list (read-only, for the ProxyJump bastion lookup) and the status center
/// (for the "pinging…" / bastion-not-found toasts). The currently selected
/// host is resolved in the thin wrapper while it still holds `&App`, because
/// `App::selected_host` reads search, ui and hosts together; the slice never
/// reaches into search, ui or any other domain.
struct PingCtx<'a> {
    ping: &'a mut PingState,
    hosts: &'a HostState,
    status: &'a mut StatusCenter,
}

impl Notify for PingCtx<'_> {
    fn status_mut(&mut self) -> &mut StatusCenter {
        self.status
    }
}

/// Re-ping the selected host in the background when its last result is older
/// than `STALE_REFRESH_AFTER` (or never recorded). No toast, just sets the
/// status to `Checking` and schedules a single-host TCP probe. Skips when
/// `auto_ping` is off so the user's "no network noise" intent is respected.
pub(crate) fn refresh_selected_if_stale(app: &mut App, events_tx: &mpsc::Sender<AppEvent>) {
    if !app.ping.auto_ping() {
        return;
    }
    let Some(host) = app.selected_host().cloned() else {
        return;
    };
    let mut ctx = PingCtx {
        ping: &mut app.ping,
        hosts: &app.hosts_state,
        status: &mut app.status_center,
    };
    refresh_stale(&mut ctx, &host, events_tx);
}

fn refresh_stale(
    ctx: &mut PingCtx,
    host: &crate::ssh_config::model::HostEntry,
    events_tx: &mpsc::Sender<AppEvent>,
) {
    let alias = host.alias.clone();
    if !ctx.ping.is_stale(&alias) {
        return;
    }
    if host.has_proxy_command {
        // Reached through a helper process: nothing to probe directly.
        log::debug!("[purple] stale-refresh: {alias} has a ProxyCommand, marked proxied");
        ctx.ping
            .insert_status(alias, crate::app::PingStatus::Skipped);
        return;
    }
    let (ping_alias, hostname, port) = if !host.proxy_jump.is_empty() {
        let bastion_alias = host.proxy_jump.clone();
        match ctx.hosts.list().iter().find(|h| h.alias == bastion_alias) {
            Some(b) if !b.hostname.is_empty() => (b.alias.clone(), b.hostname.clone(), b.port),
            _ => return,
        }
    } else if host.hostname.is_empty() {
        return;
    } else {
        (alias, host.hostname.clone(), host.port)
    };
    if matches!(
        ctx.ping.status_of(&ping_alias),
        Some(crate::app::PingStatus::Checking)
    ) {
        return;
    }
    log::debug!(
        "[external] stale-refresh: marking {} Checking and probing {}:{}",
        ping_alias,
        hostname,
        port
    );
    ctx.ping
        .insert_status(ping_alias.clone(), crate::app::PingStatus::Checking);
    ping::ping_host(
        ping_alias,
        hostname,
        port,
        events_tx.clone(),
        ctx.ping.generation(),
    );
}

/// Ping the currently selected host (shared by 'p' key and Ctrl+P in search mode).
pub(super) fn ping_selected_host(
    app: &mut App,
    events_tx: &mpsc::Sender<AppEvent>,
    show_hint: bool,
) {
    let Some(host) = app.selected_host().cloned() else {
        return;
    };
    let mut ctx = PingCtx {
        ping: &mut app.ping,
        hosts: &app.hosts_state,
        status: &mut app.status_center,
    };
    ping_host_now(&mut ctx, &host, events_tx, show_hint);
}

fn ping_host_now(
    ctx: &mut PingCtx,
    host: &crate::ssh_config::model::HostEntry,
    events_tx: &mpsc::Sender<AppEvent>,
    show_hint: bool,
) {
    let alias = host.alias.clone();
    if host.has_proxy_command {
        log::debug!("[purple] ping skipped for {alias}: ProxyCommand host");
        ctx.ping
            .insert_status(alias.clone(), crate::app::PingStatus::Skipped);
        ctx.notify(crate::messages::ping_skipped_proxied(&alias));
        return;
    }
    // For ProxyJump hosts, ping the bastion instead and propagate the
    // result to all dependents (handled in main.rs PingResult handler).
    let (ping_alias, hostname, port) = if !host.proxy_jump.is_empty() {
        let bastion_alias = host.proxy_jump.clone();
        if let Some(bastion) = ctx.hosts.list().iter().find(|h| h.alias == bastion_alias) {
            ctx.ping
                .insert_status(alias.clone(), crate::app::PingStatus::Checking);
            (
                bastion.alias.clone(),
                bastion.hostname.clone(),
                bastion.port,
            )
        } else {
            ctx.notify_warning(crate::messages::bastion_not_found(&bastion_alias));
            return;
        }
    } else {
        (alias.clone(), host.hostname.clone(), host.port)
    };
    ctx.ping
        .insert_status(ping_alias.clone(), crate::app::PingStatus::Checking);
    if show_hint && !ctx.ping.has_pinged() {
        ctx.notify(crate::messages::pinging_host(&ping_alias, true));
        ctx.ping.set_has_pinged(true);
    } else {
        ctx.notify(crate::messages::pinging_host(&ping_alias, false));
    }
    ping::ping_host(
        ping_alias,
        hostname,
        port,
        events_tx.clone(),
        ctx.ping.generation(),
    );
}
