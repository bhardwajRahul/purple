use crossterm::event::{KeyCode, KeyEvent};

use super::ctx::Nav;
use crate::app::{App, KeysState, Screen, UiSelection};

/// The slice of App the help overlay and Keys-tab navigation touch: the help
/// scroll / tag-picker selection state (`ui`), the discovered-keys list
/// (`keys`) and the screen. None of these handlers notify or defer whole-App
/// work, so they only implement `Nav`.
struct HelpCtx<'a> {
    ui: &'a mut UiSelection,
    keys: &'a mut KeysState,
    screen: &'a mut Screen,
}

impl Nav for HelpCtx<'_> {
    fn screen_mut(&mut self) -> &mut Screen {
        self.screen
    }
}

fn ctx_from_app(app: &mut App) -> HelpCtx<'_> {
    HelpCtx {
        ui: &mut app.ui,
        keys: &mut app.keys,
        screen: &mut app.screen,
    }
}

pub(super) fn handle_key(app: &mut App, key: KeyEvent) {
    let mut ctx = ctx_from_app(app);
    help_key(&mut ctx, key);
}

fn help_key(ctx: &mut HelpCtx, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            ctx.ui.set_help_scroll(0);
            ctx.pop_help_overlay();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            ctx.ui
                .set_help_scroll(ctx.ui.help_scroll().saturating_add(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            ctx.ui
                .set_help_scroll(ctx.ui.help_scroll().saturating_sub(1));
        }
        KeyCode::PageDown => {
            ctx.ui
                .set_help_scroll(ctx.ui.help_scroll().saturating_add(10));
        }
        KeyCode::PageUp => {
            ctx.ui
                .set_help_scroll(ctx.ui.help_scroll().saturating_sub(10));
        }
        _ => {}
    }
}

pub(super) fn handle_key_list_key(app: &mut App, key: KeyEvent) {
    let mut ctx = ctx_from_app(app);
    key_list_key(&mut ctx, key);
}

fn key_list_key(ctx: &mut HelpCtx, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('K') => {
            ctx.set_screen(Screen::HostList);
        }
        KeyCode::Char('?') => {
            ctx.push_help_overlay();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let len = ctx.keys.list().len();
            crate::app::cycle_selection(ctx.keys.list_state_mut(), len, true);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let len = ctx.keys.list().len();
            crate::app::cycle_selection(ctx.keys.list_state_mut(), len, false);
        }
        KeyCode::PageDown => {
            let len = ctx.keys.list().len();
            crate::app::page_down(ctx.keys.list_state_mut(), len, 10);
        }
        KeyCode::PageUp => {
            let len = ctx.keys.list().len();
            crate::app::page_up(ctx.keys.list_state_mut(), len, 10);
        }
        KeyCode::Enter => {
            if let Some(index) = ctx.keys.list_state().selected()
                && index < ctx.keys.list().len()
            {
                ctx.set_screen(Screen::KeyDetail { index });
            }
        }
        _ => {}
    }
}

pub(super) fn handle_key_detail_key(app: &mut App, key: KeyEvent) {
    let mut ctx = ctx_from_app(app);
    key_detail_key(&mut ctx, key);
}

fn key_detail_key(ctx: &mut HelpCtx, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            ctx.set_screen(Screen::KeyList);
        }
        KeyCode::Char('?') => {
            ctx.push_help_overlay();
        }
        _ => {}
    }
}
