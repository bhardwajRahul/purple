//! Vault SSH signing events and certificate expiry check events. Bulk
//! signing accumulates per-host outcomes and may write the SSH config
//! atomically on completion; the cert-check cache is keyed by alias and
//! survives single-host failures by storing them as `Invalid(message)`.

use crate::app::App;
use crate::ssh_config;
use crate::vault_ssh;

/// Handle `AppEvent::VaultSignResult`.
pub(crate) fn handle_vault_sign_result(
    app: &mut App,
    alias: String,
    existing_cert_file: String,
    success: bool,
    message: String,
) {
    if success {
        // The CertificateFile snapshot is carried in the event so
        // we never re-look up the host (which would be O(n) and
        // racy under concurrent renames).
        let mut host_missing = false;
        if crate::should_write_certificate_file(&existing_cert_file)
            && let Ok(cert_path) = vault_ssh::cert_path_for(app.env().paths(), &alias)
        {
            let updated = app
                .hosts_state
                .ssh_config_mut()
                .set_host_certificate_file(&alias, &cert_path.to_string_lossy());
            if !updated {
                host_missing = true;
            }
        }
        app.refresh_cert_cache(&alias);
        if host_missing {
            app.notify_error(crate::messages::vault_cert_saved_host_gone(&alias));
        } else {
            app.notify(crate::messages::vault_signed(&alias));
        }
    } else {
        app.notify_error(crate::messages::vault_sign_failed(&alias, &message));
    }
}

/// Handle `AppEvent::VaultSignProgress`.
pub(crate) fn handle_vault_sign_progress(
    app: &mut App,
    alias: String,
    done: usize,
    total: usize,
    spinner_tick: u64,
) {
    // Truncate long aliases so the status line fits even on
    // narrow terminals; the full alias is recoverable from the
    // host list.
    const ALIAS_BUDGET: usize = 40;
    let display_alias: String = if alias.chars().count() > ALIAS_BUDGET {
        let cut: String = alias.chars().take(ALIAS_BUDGET - 1).collect();
        format!("{}\u{2026}", cut)
    } else {
        alias.clone()
    };
    let spinner = crate::animation::SPINNER_FRAMES
        [spinner_tick as usize % crate::animation::SPINNER_FRAMES.len()];
    app.notify_progress(crate::messages::vault_signing_progress(
        spinner,
        done,
        total,
        &display_alias,
    ));
}

/// Handle `AppEvent::VaultSignAllDone`. Returns `ControlFlow::Break(())` when
/// the caller should `continue` the event loop (skip the rest of the iteration),
/// or `ControlFlow::Continue(())` for normal processing.
pub(crate) fn handle_vault_sign_all_done(
    app: &mut App,
    signed: u32,
    failed: u32,
    skipped: u32,
    cancelled: bool,
    aborted_message: Option<String>,
    first_error: Option<String>,
) -> std::ops::ControlFlow<()> {
    // Join the background thread now that it has finished. Use the
    // finalize variant: the worker already emitted VaultSignAllDone,
    // so re-signalling cancel here could hit a newer run's Arc if the
    // user pressed V→V→V during the dispatch window.
    if let Some(handle) = app.vault.finalize_signing_run() {
        log::debug!("[purple] vault sign thread: joining");
        let _ = handle.join();
        log::info!(
            "[purple] vault sign thread: joined (signed={} failed={} skipped={} cancelled={})",
            signed,
            failed,
            skipped,
            cancelled
        );
    }
    if let Some(msg) = aborted_message {
        app.notify_sticky_error(msg);
        return std::ops::ControlFlow::Break(()); // caller should `continue`
    }
    if cancelled {
        let msg = crate::messages::vault_signing_cancelled_summary(
            signed,
            failed,
            first_error.as_deref(),
        );
        if failed > 0 {
            app.notify_sticky_error(msg);
        } else {
            app.notify_info(msg);
        }
        return std::ops::ControlFlow::Break(()); // caller should `continue`
    }
    let summary_msg =
        crate::format_vault_sign_summary(signed, failed, skipped, first_error.as_deref());
    if signed > 0 {
        if app.is_form_open() {
            // Defer config write to avoid mtime conflict with open forms
            app.vault.set_pending_config_write(true);
            if failed > 0 {
                app.notify_sticky_error(summary_msg);
            } else {
                app.notify_info(summary_msg);
            }
        } else if app.external_config_changed() {
            // The on-disk ssh config (or an include) was modified
            // by an external editor while the bulk-sign worker was
            // running. Writing now would overwrite those edits.
            let reapply: Vec<(String, String)> = app
                .hosts_state
                .ssh_config()
                .host_entries()
                .into_iter()
                .filter_map(|h| {
                    if h.vault_ssh.is_some()
                        && crate::should_write_certificate_file(&h.certificate_file)
                    {
                        vault_ssh::cert_path_for(app.env().paths(), &h.alias)
                            .ok()
                            .map(|p| (h.alias.clone(), p.to_string_lossy().into_owned()))
                    } else {
                        None
                    }
                })
                .collect();
            match ssh_config::model::SshConfigFile::parse_with_env(
                app.reload.config_path(),
                app.env(),
            ) {
                Ok(fresh) => {
                    app.hosts_state.set_ssh_config(fresh);
                    let mut reapplied = 0usize;
                    for (alias, cert_path) in &reapply {
                        let entry = app
                            .hosts_state
                            .ssh_config()
                            .host_entries()
                            .into_iter()
                            .find(|h| &h.alias == alias);
                        if let Some(entry) = entry
                            && crate::should_write_certificate_file(&entry.certificate_file)
                            && app
                                .hosts_state
                                .ssh_config_mut()
                                .set_host_certificate_file(alias, cert_path)
                        {
                            reapplied += 1;
                        }
                    }
                    if reapplied > 0 {
                        if let Err(e) = app.hosts_state.ssh_config().write() {
                            app.notify_sticky_error(crate::messages::vault_config_reapply_failed(
                                signed as usize,
                                &e,
                            ));
                        } else {
                            app.update_last_modified();
                            app.reload_hosts();
                            if failed > 0 {
                                app.notify_sticky_error(
                                    crate::messages::vault_external_edits_merged(
                                        &summary_msg,
                                        reapplied,
                                    ),
                                );
                            } else {
                                app.notify_info(crate::messages::vault_external_edits_merged(
                                    &summary_msg,
                                    reapplied,
                                ));
                            }
                        }
                    } else {
                        app.reload_hosts();
                        app.notify_sticky_error(crate::messages::vault_external_edits_no_write(
                            &summary_msg,
                        ));
                    }
                }
                Err(e) => {
                    app.notify_sticky_error(crate::messages::vault_reparse_failed(
                        signed as usize,
                        &e,
                    ));
                }
            }
        } else if let Err(e) = app.hosts_state.ssh_config().write() {
            app.notify_sticky_error(crate::messages::vault_config_update_failed(
                signed as usize,
                &e,
            ));
        } else {
            app.update_last_modified();
            app.reload_hosts();
            if failed > 0 {
                app.notify_sticky_error(summary_msg);
            } else {
                app.notify_info(summary_msg);
            }
        }
    } else if failed > 0 {
        app.notify_sticky_error(summary_msg);
    } else {
        app.notify_info(summary_msg);
    }
    std::ops::ControlFlow::Continue(()) // normal flow
}

/// Handle `AppEvent::CertCheckResult`.
pub(crate) fn handle_cert_check_result(
    app: &mut App,
    alias: String,
    status: vault_ssh::CertStatus,
) {
    let mtime = crate::tui_loop::current_cert_mtime(&alias, app);
    app.vault.record_cert_check(alias, status, mtime);
}

/// Handle `AppEvent::CertCheckError`.
pub(crate) fn handle_cert_check_error(app: &mut App, alias: String, message: String) {
    // Cache the error as Invalid so the lazy-check loop doesn't
    // re-spawn a background thread on every poll tick.
    app.vault.record_cert_check(
        alias.clone(),
        vault_ssh::CertStatus::Invalid(message.clone()),
        None,
    );
    app.notify_background_error(crate::messages::vault_cert_check_failed(&alias, &message));
}
