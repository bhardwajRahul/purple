//! File browser listing and SCP transfer completion events. Updates the
//! `app.file_browser_session` overlay, forces a terminal redraw (ssh may
//! have written to /dev/tty) and respawns the remote listing thread once
//! a successful transfer lands.

use std::sync::mpsc;

use crate::app::App;
use crate::event::AppEvent;
use crate::file_browser;
use crate::tui;

/// Handle `AppEvent::FileBrowserListing`.
pub(crate) fn handle_file_browser_listing(
    app: &mut App,
    alias: String,
    path: String,
    entries: Result<Vec<crate::file_browser::FileEntry>, String>,
    terminal: &mut tui::Tui,
) {
    let mut record_connection = false;
    if let Some(ref mut fb) = app.file_browser_session
        && fb.alias == alias
    {
        fb.remote_loading = false;
        match entries {
            Ok(listing) => {
                if !fb.connection_recorded {
                    fb.connection_recorded = true;
                    record_connection = true;
                }
                if fb.remote_path.is_empty() || fb.remote_path != path {
                    fb.remote_path = path;
                }
                fb.remote_entries = listing;
                fb.remote_error = None;
                fb.remote_list_state = ratatui::widgets::ListState::default();
                fb.remote_list_state.select(Some(0));
            }
            Err(msg) => {
                if fb.remote_path.is_empty() {
                    fb.remote_path = path;
                }
                fb.remote_error = Some(msg);
                fb.remote_entries.clear();
            }
        }
    }
    if record_connection {
        app.history.record(&alias);
        app.record_key_use(&alias, crate::key_activity::now_secs());
        app.apply_sort();
    }
    // Force full redraw: ssh may have written to /dev/tty
    terminal.force_redraw();
}

/// Handle `AppEvent::ScpComplete`.
pub(crate) fn handle_scp_complete(
    app: &mut App,
    alias: String,
    success: bool,
    message: String,
    events_tx: &mpsc::Sender<AppEvent>,
    terminal: &mut tui::Tui,
) {
    // Track whether we need to spawn a remote refresh (can't do it inside the fb borrow
    // because spawn_remote_listing needs values from app too)
    let mut refresh_remote: Option<(
        String,
        Option<String>,
        String,
        bool,
        file_browser::BrowserSort,
    )> = None;
    let matched = if let Some(ref mut fb) = app.file_browser_session {
        if fb.alias == alias {
            fb.transferring = None;
            if success {
                app.history.record(&alias);
                // Field-disjoint helper: fb already holds &mut app.file_browser_session,
                // so the `App::record_key_use` method would not borrow-check.
                crate::key_activity::record_and_flush(
                    app.keys.activity_mut(),
                    &alias,
                    crate::key_activity::now_secs(),
                    app.env.paths().cloned().as_ref(),
                );
                // history_width depends on formatted timestamps; rebuild next render
                app.hosts_state.invalidate_render_cache();
                fb.local_selected.clear();
                fb.remote_selected.clear();
                match file_browser::list_local(&fb.local_path, fb.show_hidden, fb.sort) {
                    Ok(entries) => {
                        fb.local_entries = entries;
                        fb.local_error = None;
                    }
                    Err(e) => {
                        fb.local_entries = Vec::new();
                        fb.local_error = Some(e.to_string());
                    }
                }
                fb.local_list_state.select(Some(0));
                if !fb.remote_path.is_empty() {
                    fb.remote_loading = true;
                    fb.remote_entries.clear();
                    fb.remote_error = None;
                    fb.remote_list_state = ratatui::widgets::ListState::default();
                    refresh_remote = Some((
                        fb.alias.clone(),
                        fb.askpass.clone(),
                        fb.remote_path.clone(),
                        fb.show_hidden,
                        fb.sort,
                    ));
                }
            } else {
                fb.transfer_error = Some(message.clone());
            }
            true
        } else {
            false
        }
    } else {
        false
    };
    if matched && success {
        app.notify_background(crate::messages::TRANSFER_COMPLETE);
        // Rebuild display list so frecency sort and LAST column reflect the transfer
        app.apply_sort();
    }
    if let Some((fb_alias, askpass_fb, path, show_hidden, sort)) = refresh_remote {
        let has_tunnel = app.tunnels.active_contains(&fb_alias);
        let ctx = crate::ssh_context::OwnedSshContext {
            alias: fb_alias,
            config_path: app.reload.config_path().to_path_buf(),
            askpass: askpass_fb,
            bw_session: app.bw_session.clone(),
            has_tunnel,
            env: std::sync::Arc::clone(&app.env),
        };
        let tx = events_tx.clone();
        file_browser::spawn_remote_listing(ctx, path, show_hidden, sort, move |a, p, e| {
            let _ = tx.send(AppEvent::FileBrowserListing {
                alias: a,
                path: p,
                entries: e,
            });
        });
    }
    crate::askpass::cleanup_marker(app.env.paths(), &alias);
    // Force full redraw: ssh may have written to /dev/tty
    terminal.force_redraw();
}
