//! Host picker reached from the Tunnels overview when adding a new tunnel.
//!
//! Lists all editable hosts (hosts that live in the user's own SSH config,
//! not in an included file). Always-on filter input: every printable keystroke
//! appends to the query and the candidate set shrinks live, using the shared
//! fuzzy host ranking (`fuzzy::rank_host_indices`) so type-to-filter behaves
//! identically across the tunnel, container and snippet host pickers.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::ctx::{Effectful, Effects, Nav};
use crate::app::{App, HostState, Screen, UiSelection};

/// Editable hosts visible in the picker, in display order.
///
/// Mirrors the filter applied in `handler::tunnel::handle_tunnel_list` for
/// add/edit/delete: hosts whose `source_file` is `Some(_)` originate from an
/// `Include` directive and cannot be mutated through purple.
pub(crate) fn editable_aliases(app: &App) -> Vec<String> {
    app.hosts_state
        .list()
        .iter()
        .filter(|h| h.source_file.is_none())
        .map(|h| h.alias.clone())
        .collect()
}

/// Hosts that match the live query, paired with the matching hostname for
/// display, best match first. When the query is empty every editable host is
/// returned in config order.
///
/// Match rule is the shared fuzzy host ranking (`fuzzy::rank_host_indices`),
/// the same type-to-filter behaviour as the container and snippet host pickers.
pub(crate) fn filtered_hosts(app: &App) -> Vec<(String, String)> {
    filter_hosts(app.ui.tunnel_host_picker_query(), &app.hosts_state)
}

/// Shared filter used by both the public `filtered_hosts(&App)` (render side)
/// and the picker slice. Single source of truth for the editable-host match:
/// the editable hosts, fuzzy-ranked against the query.
fn filter_hosts(query: &str, hosts: &HostState) -> Vec<(String, String)> {
    let candidates: Vec<usize> = hosts
        .list()
        .iter()
        .enumerate()
        .filter(|(_, h)| h.source_file.is_none())
        .map(|(i, _)| i)
        .collect();
    crate::fuzzy::rank_host_indices(hosts.list(), &candidates, query)
        .into_iter()
        .filter_map(|i| hosts.list().get(i))
        .map(|h| (h.alias.clone(), h.hostname.clone()))
        .collect()
}

/// The slice of App the tunnel host picker touches: the picker selection and
/// query (`ui`), the host list (read-only, for the editable-host filter) and
/// the screen. Opening the tunnel add form is a whole-App helper shared with
/// four other handlers, so it runs as a deferred effect after the slice borrow
/// ends. The picker never reaches into tunnels, providers or any other domain.
struct TunnelHostPickerCtx<'a> {
    ui: &'a mut UiSelection,
    hosts: &'a HostState,
    screen: &'a mut Screen,
    effects: Effects,
}

impl Nav for TunnelHostPickerCtx<'_> {
    fn screen_mut(&mut self) -> &mut Screen {
        self.screen
    }
}

impl Effectful for TunnelHostPickerCtx<'_> {
    fn effects_mut(&mut self) -> &mut Effects {
        &mut self.effects
    }
}

impl TunnelHostPickerCtx<'_> {
    /// Hosts matching the live query, mirroring the public `filtered_hosts`.
    fn filtered_hosts(&self) -> Vec<(String, String)> {
        filter_hosts(self.ui.tunnel_host_picker_query(), self.hosts)
    }
}

pub(super) fn handle_key(app: &mut App, key: KeyEvent) {
    let effects = {
        let mut ctx = TunnelHostPickerCtx {
            ui: &mut app.ui,
            hosts: &app.hosts_state,
            screen: &mut app.screen,
            effects: Effects::default(),
        };
        picker_key(&mut ctx, key);
        ctx.effects
    };
    effects.apply(app);
}

fn picker_key(ctx: &mut TunnelHostPickerCtx, key: KeyEvent) {
    let total = ctx.filtered_hosts().len();
    match key.code {
        KeyCode::Esc => close(ctx),
        KeyCode::Down if total > 0 => {
            let cur = ctx.ui.tunnel_host_picker_state().selected().unwrap_or(0);
            let next = (cur + 1).min(total - 1);
            ctx.ui.tunnel_host_picker_state_mut().select(Some(next));
        }
        KeyCode::Up => {
            let cur = ctx.ui.tunnel_host_picker_state().selected().unwrap_or(0);
            ctx.ui
                .tunnel_host_picker_state_mut()
                .select(Some(cur.saturating_sub(1)));
        }
        KeyCode::Enter => {
            let Some(idx) = ctx.ui.tunnel_host_picker_state().selected() else {
                return;
            };
            let Some((alias, _)) = ctx.filtered_hosts().into_iter().nth(idx) else {
                return;
            };
            ctx.ui.tunnel_host_picker_state_mut().select(None);
            ctx.ui.tunnel_host_picker_query_mut().clear();
            ctx.defer(move |app| app.open_tunnel_add_form(alias));
        }
        KeyCode::Backspace => {
            // Mirror the jump: Backspace on an empty query
            // closes the overlay; otherwise it shortens the query.
            if ctx.ui.tunnel_host_picker_query().is_empty() {
                close(ctx);
            } else {
                ctx.ui.tunnel_host_picker_query_mut().pop();
                reset_cursor_after_query_change(ctx);
            }
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && ctx.ui.tunnel_host_picker_query().len() < 64 =>
        {
            // Cap the query length so a stuck key cannot grow the buffer
            // unbounded. Same 64-char cap the jump uses.
            ctx.ui.tunnel_host_picker_query_mut().push(c);
            reset_cursor_after_query_change(ctx);
        }
        _ => {}
    }
}

fn close(ctx: &mut TunnelHostPickerCtx) {
    ctx.ui.tunnel_host_picker_state_mut().select(None);
    ctx.ui.tunnel_host_picker_query_mut().clear();
    ctx.set_screen(Screen::HostList);
}

/// After the candidate set shrinks or grows, snap the cursor to row 0 (or
/// `None` when the set is empty) so the highlight always sits on a real row.
fn reset_cursor_after_query_change(ctx: &mut TunnelHostPickerCtx) {
    let total = ctx.filtered_hosts().len();
    if total == 0 {
        ctx.ui.tunnel_host_picker_state_mut().select(None);
    } else {
        ctx.ui.tunnel_host_picker_state_mut().select(Some(0));
    }
}
