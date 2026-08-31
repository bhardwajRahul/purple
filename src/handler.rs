use std::sync::atomic::Ordering;
use std::sync::mpsc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Screen};
use crate::event::AppEvent;

mod bulk_tag_editor;
mod confirm;
mod container_exec_prompt;
pub(crate) mod container_host_picker;
pub(crate) mod container_logs;
mod containers;
pub(crate) mod containers_overview;
mod ctx;
pub(crate) mod event_loop;
mod file_browser;
mod help;
mod host_detail;
mod host_form;
mod host_list;
mod jump;
pub(crate) mod key_push_picker;
mod keys_overview;
mod picker;
mod ping;
mod provider;
mod snippet;
pub(crate) mod snippet_host_picker;
mod snippets_overview;
mod sync;
mod tag_picker;
mod theme_picker;
mod tunnel;
pub(crate) mod tunnel_host_picker;
mod tunnels_overview;
mod welcome;
mod whats_new;

pub use confirm::{ConfirmAction, route_confirm_key};
pub(crate) use provider::zone_data_for;
pub use sync::spawn_provider_sync;

/// Handle a key event based on the current screen.
pub fn handle_key_event(
    app: &mut App,
    key: KeyEvent,
    events_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    // Global Ctrl+C handler — screen-conditional for SnippetOutput
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if matches!(app.screen, Screen::SnippetOutput) {
            if let Some(state) = app.snippets.output()
                && !state.all_done
            {
                if state.cancel.load(Ordering::Relaxed) {
                    // Second Ctrl+C: cancel already pending, force close
                } else {
                    // First Ctrl+C: request cancellation
                    state.cancel.store(true, Ordering::Relaxed);
                    return Ok(());
                }
            }
            app.snippets.set_output(None);
            app.set_screen(Screen::HostList);
            return Ok(());
        }
        if let Some(cancel) = app.vault.signing_cancel() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        app.running = false;
        return Ok(());
    }

    // Jump intercept
    if app.jump.is_some() {
        jump::handle_key(app, key, events_tx);
        return Ok(());
    }

    match &app.screen {
        Screen::HostList => host_list::handle_key(app, key, events_tx),
        Screen::AddHost | Screen::EditHost { .. } => host_form::handle_key(app, key),
        Screen::ConfirmDelete { .. } => confirm::handle_delete_key(app, key),
        Screen::Help { .. } => help::handle_key(app, key),
        Screen::KeyList => help::handle_key_list_key(app, key),
        Screen::KeyDetail { .. } => help::handle_key_detail_key(app, key),
        Screen::KeyPushPicker { .. } => key_push_picker::handle_key(app, key),
        Screen::ConfirmKeyPush { .. } => confirm::handle_key_push_key(app, key, events_tx),
        Screen::HostDetail { .. } => host_detail::handle_key(app, key),
        Screen::TagPicker => tag_picker::handle_key(app, key),
        Screen::BulkTagEditor => bulk_tag_editor::handle_key(app, key),
        Screen::ThemePicker => theme_picker::handle_key(app, key),
        Screen::Providers => provider::handle_provider_list_key(app, key, events_tx),
        Screen::ProviderForm { .. } => provider::handle_provider_form_key(app, key, events_tx),
        Screen::ProviderLabelMigration { .. } => {
            provider::handle_label_migration_key(app, key, events_tx)
        }
        Screen::TunnelList { .. } => tunnel::handle_tunnel_list_key(app, key),
        Screen::TunnelForm { .. } => tunnel::handle_tunnel_form_key(app, key),
        Screen::TunnelHostPicker => tunnel_host_picker::handle_key(app, key),
        Screen::ContainerHostPicker => container_host_picker::handle_key(app, key, events_tx),
        Screen::SnippetPicker => snippet::handle_picker_key(app, key, events_tx),
        Screen::SnippetForm => snippet::handle_form_key(app, key),
        Screen::SnippetOutput => snippet::handle_output_key(app, key),
        Screen::SnippetParamForm => snippet::handle_param_form_key(app, key, events_tx),
        Screen::SnippetHostPicker => snippet_host_picker::handle_key(app, key),
        Screen::ConfirmRunSnippet => confirm::handle_run_snippet_confirm_key(app, key, events_tx),
        Screen::ConfirmHostKeyReset { .. } => confirm::handle_host_key_reset_key(app, key),
        Screen::ConfirmVaultSign => confirm::handle_vault_sign_key(app, key, events_tx),
        Screen::ConfirmImport { .. } => confirm::handle_import_key(app, key),
        Screen::ConfirmPurgeStale => confirm::handle_purge_stale_key(app, key),
        Screen::FileBrowser { .. } => file_browser::handle_key(app, key, events_tx),
        Screen::Containers { .. } => containers::handle_key(app, key, events_tx)?,
        Screen::ContainerLogs => container_logs::handle_key(app, key, events_tx),
        Screen::ConfirmContainerRestart { .. } => confirm::handle_container_restart_key(app, key),
        Screen::ConfirmContainerStop { .. } => confirm::handle_container_stop_key(app, key),
        Screen::ContainerExecPrompt { .. } => container_exec_prompt::handle_key(app, key),
        Screen::ConfirmStackRestart => confirm::handle_stack_restart_key(app, key),
        Screen::ConfirmHostRestartAll => confirm::handle_host_restart_all_key(app, key),
        Screen::ConfirmHostStopAll => confirm::handle_host_stop_all_key(app, key),
        Screen::Welcome { .. } => welcome::handle_key(app, key),
        Screen::WhatsNew(_) => whats_new::handle_key(app, key),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
