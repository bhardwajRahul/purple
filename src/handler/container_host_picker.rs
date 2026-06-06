//! Host picker reached from the containers overview when adding a
//! host to the container cache (`a` keypress).
//!
//! Lists every host in the SSH config that does NOT already have a
//! cache entry. On Enter, fires one `docker ps` listing for the
//! chosen host and returns to the overview. Mirrors the tunnel host
//! picker's "type to filter" rhythm so muscle memory carries over.

use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::ctx::{Effectful, Effects, Nav};
use crate::app::{App, ContainerState, HostState, Screen, UiSelection};
use crate::event::AppEvent;

/// Aliases of every host that has no cache entry yet, in display
/// order. The picker's `a`-flow is for *adding* — already-cached
/// hosts use `r`/`R` instead — so they are filtered out here.
pub(crate) fn uncached_aliases(app: &App) -> Vec<String> {
    app.hosts_state
        .list()
        .iter()
        .filter(|h| !app.container_state.cache_contains(&h.alias))
        .map(|h| h.alias.clone())
        .collect()
}

/// Hosts that match the live query, paired with the matching hostname
/// for display, best match first. Uses the shared fuzzy host ranking, the
/// same type-to-filter behaviour as the tunnel and snippet host pickers.
pub(crate) fn filtered_hosts(app: &App) -> Vec<(String, String)> {
    filter_hosts(
        app.ui.container_host_picker_query(),
        &app.hosts_state,
        &app.container_state,
    )
}

/// Shared filter used by both the public `filtered_hosts(&App)` (render side)
/// and the picker slice. Single source of truth for the uncached-host match:
/// the uncached hosts, fuzzy-ranked against the query.
fn filter_hosts(
    query: &str,
    hosts: &HostState,
    container_state: &ContainerState,
) -> Vec<(String, String)> {
    let candidates: Vec<usize> = hosts
        .list()
        .iter()
        .enumerate()
        .filter(|(_, h)| !container_state.cache_contains(&h.alias))
        .map(|(i, _)| i)
        .collect();
    crate::fuzzy::rank_host_indices(hosts.list(), &candidates, query)
        .into_iter()
        .filter_map(|i| hosts.list().get(i))
        .map(|h| (h.alias.clone(), h.hostname.clone()))
        .collect()
}

/// The slice of App the container host picker touches: the picker selection and
/// query (`ui`), the host list and container cache (read-only, for the
/// uncached-host filter) and the screen. Firing the initial `docker ps` reaches
/// across many domains (askpass, tunnels, the overview cache, bw session), so
/// it runs as a deferred effect after the slice borrow ends.
struct ContainerHostPickerCtx<'a> {
    ui: &'a mut UiSelection,
    hosts: &'a HostState,
    container_state: &'a ContainerState,
    screen: &'a mut Screen,
    effects: Effects,
}

impl Nav for ContainerHostPickerCtx<'_> {
    fn screen_mut(&mut self) -> &mut Screen {
        self.screen
    }
}

impl Effectful for ContainerHostPickerCtx<'_> {
    fn effects_mut(&mut self) -> &mut Effects {
        &mut self.effects
    }
}

impl ContainerHostPickerCtx<'_> {
    /// Hosts matching the live query, mirroring the public `filtered_hosts`.
    fn filtered_hosts(&self) -> Vec<(String, String)> {
        filter_hosts(
            self.ui.container_host_picker_query(),
            self.hosts,
            self.container_state,
        )
    }
}

pub(super) fn handle_key(app: &mut App, key: KeyEvent, events_tx: &mpsc::Sender<AppEvent>) {
    let effects = {
        let mut ctx = ContainerHostPickerCtx {
            ui: &mut app.ui,
            hosts: &app.hosts_state,
            container_state: &app.container_state,
            screen: &mut app.screen,
            effects: Effects::default(),
        };
        picker_key(&mut ctx, key, events_tx);
        ctx.effects
    };
    effects.apply(app);
}

fn picker_key(ctx: &mut ContainerHostPickerCtx, key: KeyEvent, events_tx: &mpsc::Sender<AppEvent>) {
    let total = ctx.filtered_hosts().len();
    match key.code {
        KeyCode::Esc => close(ctx),
        KeyCode::Down if total > 0 => {
            let cur = ctx.ui.container_host_picker_state().selected().unwrap_or(0);
            let next = (cur + 1).min(total - 1);
            ctx.ui.container_host_picker_state_mut().select(Some(next));
        }
        KeyCode::Up => {
            let cur = ctx.ui.container_host_picker_state().selected().unwrap_or(0);
            ctx.ui
                .container_host_picker_state_mut()
                .select(Some(cur.saturating_sub(1)));
        }
        KeyCode::Enter => {
            let Some(idx) = ctx.ui.container_host_picker_state().selected() else {
                return;
            };
            let Some((alias, _)) = ctx.filtered_hosts().into_iter().nth(idx) else {
                return;
            };
            close(ctx);
            let tx = events_tx.clone();
            ctx.defer(move |app| spawn_initial_listing(app, alias, &tx));
        }
        KeyCode::Backspace => {
            if ctx.ui.container_host_picker_query().is_empty() {
                close(ctx);
            } else {
                ctx.ui.container_host_picker_query_mut().pop();
                reset_cursor_after_query_change(ctx);
            }
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && ctx.ui.container_host_picker_query().len() < 64 =>
        {
            ctx.ui.container_host_picker_query_mut().push(c);
            reset_cursor_after_query_change(ctx);
        }
        _ => {}
    }
}

fn close(ctx: &mut ContainerHostPickerCtx) {
    ctx.ui.container_host_picker_state_mut().select(None);
    ctx.ui.container_host_picker_query_mut().clear();
    ctx.set_screen(Screen::HostList);
}

fn reset_cursor_after_query_change(ctx: &mut ContainerHostPickerCtx) {
    let total = ctx.filtered_hosts().len();
    if total == 0 {
        ctx.ui.container_host_picker_state_mut().select(None);
    } else {
        ctx.ui.container_host_picker_state_mut().select(Some(0));
    }
}

/// Fire the first `docker ps` for `alias`. No cached runtime → the
/// listing command runs the sentinel-detection variant. Mirrors the
/// `C`-on-host-list flow exactly except for the surrounding flow
/// control (we are coming from the picker, not the host list).
fn spawn_initial_listing(app: &mut App, alias: String, events_tx: &mpsc::Sender<AppEvent>) {
    let askpass = app
        .hosts_state
        .list()
        .iter()
        .find(|h| h.alias == alias)
        .and_then(|h| h.askpass.clone());
    let has_tunnel = app.tunnels.active_contains(&alias);
    log::debug!("[purple] container cache add: alias={}", alias);
    app.notify(crate::messages::container_refreshing(&alias));
    // Mark in-flight so the post-key auto-refresh does not spawn a
    // second `docker ps` for the same alias before this one returns.
    app.containers_overview
        .mark_auto_list_pending(alias.clone());
    let ctx = crate::ssh_context::OwnedSshContext {
        alias,
        config_path: app.reload.config_path().to_path_buf(),
        askpass,
        bw_session: app.bw_session.clone(),
        has_tunnel,
        env: std::sync::Arc::clone(&app.env),
    };
    let tx = events_tx.clone();
    crate::containers::spawn_container_listing(ctx, None, move |a, result| {
        let _ = tx.send(AppEvent::ContainerListing { alias: a, result });
    });
}
