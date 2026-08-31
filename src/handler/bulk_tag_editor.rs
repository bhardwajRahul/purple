use crossterm::event::{KeyCode, KeyEvent};

use super::ctx::{Effectful, Effects, Nav, Notify};
use crate::app::{
    App, BulkTagEditorState, FormState, HostState, Screen, StatusCenter, UiSelection,
};

/// The slice of App the bulk tag editor touches: host state (apply writes the
/// config), the form scratch (rows, new-tag input), the picker selection, the
/// screen and the status center. Applying the edit reloads hosts, which
/// touches most of App, so that tail runs as a deferred effect after the slice
/// borrow ends.
struct BulkTagCtx<'a> {
    hosts: &'a mut HostState,
    forms: &'a mut FormState,
    ui: &'a mut UiSelection,
    screen: &'a mut Screen,
    status: &'a mut StatusCenter,
    effects: Effects,
}

impl Nav for BulkTagCtx<'_> {
    fn screen_mut(&mut self) -> &mut Screen {
        self.screen
    }
}

impl Notify for BulkTagCtx<'_> {
    fn status_mut(&mut self) -> &mut StatusCenter {
        self.status
    }
}

impl Effectful for BulkTagCtx<'_> {
    fn effects_mut(&mut self) -> &mut Effects {
        &mut self.effects
    }
}

pub(super) fn handle_key(app: &mut App, key: KeyEvent) {
    let effects = {
        let mut ctx = BulkTagCtx {
            hosts: &mut app.hosts_state,
            forms: &mut app.forms,
            ui: &mut app.ui,
            screen: &mut app.screen,
            status: &mut app.status_center,
            effects: Effects::default(),
        };
        bulk_tag_key(&mut ctx, key);
        ctx.effects
    };
    effects.apply(app);
}

fn bulk_tag_key(ctx: &mut BulkTagCtx, key: KeyEvent) {
    // When the "new tag" input bar is active, route character input there
    // first so users can type tag names without triggering the row-level
    // keybindings (j/k/Space/Enter). Esc cancels the input without closing
    // the editor. The new-tag-input early-return runs BEFORE the discard
    // confirm so typing-mode Esc does not trigger the dirty check.
    if ctx.forms.bulk_tag_editor().new_tag_input.is_some() {
        handle_new_tag_input(ctx, key);
        return;
    }

    // Discard confirmation: when the user pressed Esc on a dirty editor, the
    // main handler armed the discard-confirm dialog and re-rendered with the
    // discard footer. Route the next keypress through the central confirm
    // router (uniform with form discard prompts elsewhere).
    if ctx.forms.is_discard_pending() {
        match super::route_confirm_key(key) {
            super::ConfirmAction::Yes => {
                ctx.forms.dismiss_discard_confirm();
                ctx.set_screen(Screen::HostList);
                *ctx.forms.bulk_tag_editor_mut() = BulkTagEditorState::default();
            }
            super::ConfirmAction::No => {
                ctx.forms.dismiss_discard_confirm();
            }
            super::ConfirmAction::Ignored => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            // Stakes test: tag edits are non-trivial work (typing new tags,
            // deciding add/remove per row across N hosts). Warn before
            // discarding.
            if ctx.forms.bulk_tag_editor_mut().is_dirty() {
                ctx.forms.request_discard_confirm();
            } else {
                ctx.set_screen(Screen::HostList);
                *ctx.forms.bulk_tag_editor_mut() = BulkTagEditorState::default();
            }
        }
        KeyCode::Char('?') => {
            ctx.push_help_overlay();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            crate::app::cycle_selection(
                ctx.ui.bulk_tag_editor_state_mut(),
                ctx.forms.bulk_tag_editor().rows.len(),
                true,
            );
        }
        KeyCode::Char('k') | KeyCode::Up => {
            crate::app::cycle_selection(
                ctx.ui.bulk_tag_editor_state_mut(),
                ctx.forms.bulk_tag_editor().rows.len(),
                false,
            );
        }
        // SPACE GUARD MUST PRECEDE any generic Char(c) arm in this handler
        // so Space cycles the focused tag's tri-state rather than typing a
        // literal space into the new-tag input.
        KeyCode::Char(' ') => {
            crate::app::bulk_tag_cycle_current(ctx.ui, ctx.forms);
        }
        KeyCode::Char('+') => {
            ctx.forms.bulk_tag_editor_mut().new_tag_input = Some(String::new());
            ctx.forms.bulk_tag_editor_mut().new_tag_cursor = 0;
        }
        KeyCode::Enter => match crate::app::apply_bulk_tags(ctx.hosts, ctx.forms) {
            Ok(result) => {
                ctx.set_screen(Screen::HostList);
                *ctx.forms.bulk_tag_editor_mut() = BulkTagEditorState::default();
                // mtime refresh + reload touch most of App, so they run after
                // the slice borrow ends. The success toast is deferred behind
                // them so a reload-time conflict warning (notify_error inside
                // reload_hosts) still shows last, exactly as the pre-slice
                // inline order did.
                if result.changed_hosts > 0 {
                    ctx.update_last_modified();
                    ctx.reload_hosts();
                }
                let msg = format_apply_status(&result);
                if !msg.is_empty() {
                    ctx.defer(move |app| app.notify(msg));
                }
            }
            Err(err) => {
                ctx.notify_error(err);
            }
        },
        _ => {}
    }
}

/// Status string shown after a successful bulk apply. Empty when nothing
/// was pending (no-op) and no included-host warning applies, so the caller
/// can skip setting a status. Thin wrapper that funnels the formatting
/// through `crate::messages` to keep all user-facing copy in one place.
pub(crate) fn format_apply_status(result: &crate::app::BulkTagApplyResult) -> String {
    crate::messages::bulk_tag_apply_status(
        result.changed_hosts,
        result.added,
        result.removed,
        result.skipped_included,
    )
}

fn handle_new_tag_input(ctx: &mut BulkTagCtx, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            crate::app::bulk_tag_commit_new_tag(ctx.ui, ctx.forms);
        }
        KeyCode::Esc => {
            ctx.forms.bulk_tag_editor_mut().new_tag_input = None;
            ctx.forms.bulk_tag_editor_mut().new_tag_cursor = 0;
        }
        KeyCode::Left if ctx.forms.bulk_tag_editor().new_tag_cursor > 0 => {
            ctx.forms.bulk_tag_editor_mut().new_tag_cursor -= 1;
        }
        KeyCode::Right => {
            let len = ctx
                .forms
                .bulk_tag_editor()
                .new_tag_input
                .as_ref()
                .map(|s| s.chars().count());
            if let Some(len) = len
                && ctx.forms.bulk_tag_editor().new_tag_cursor < len
            {
                ctx.forms.bulk_tag_editor_mut().new_tag_cursor += 1;
            }
        }
        KeyCode::Home => {
            ctx.forms.bulk_tag_editor_mut().new_tag_cursor = 0;
        }
        KeyCode::End => {
            let len = ctx
                .forms
                .bulk_tag_editor()
                .new_tag_input
                .as_ref()
                .map(|s| s.chars().count());
            if let Some(len) = len {
                ctx.forms.bulk_tag_editor_mut().new_tag_cursor = len;
            }
        }
        KeyCode::Backspace if ctx.forms.bulk_tag_editor().new_tag_cursor > 0 => {
            let cursor = ctx.forms.bulk_tag_editor().new_tag_cursor;
            let mut drained = false;
            if let Some(ref mut input) = ctx.forms.bulk_tag_editor_mut().new_tag_input {
                let byte_pos = crate::app::char_to_byte_pos(input, cursor);
                let prev = crate::app::char_to_byte_pos(input, cursor - 1);
                input.drain(prev..byte_pos);
                drained = true;
            }
            if drained {
                ctx.forms.bulk_tag_editor_mut().new_tag_cursor -= 1;
            }
        }
        KeyCode::Char(c) => {
            let cursor = ctx.forms.bulk_tag_editor().new_tag_cursor;
            if let Some(ref mut input) = ctx.forms.bulk_tag_editor_mut().new_tag_input {
                let byte_pos = crate::app::char_to_byte_pos(input, cursor);
                input.insert(byte_pos, c);
                ctx.forms.bulk_tag_editor_mut().new_tag_cursor += 1;
            }
        }
        _ => {}
    }
}
