use crossterm::event::{KeyCode, KeyEvent};

use super::ctx::{Effectful, Effects, Nav};
use crate::app::{App, Screen, TagState, UiSelection};

/// The slice of App the tag picker touches: the picker selection state, the tag
/// library and the screen. Selecting a tag kicks off a whole-App search
/// (start_search, set_query and apply_filter touch hosts, ping, providers and
/// search together), so that sequence runs as a deferred effect after the slice
/// borrow ends.
struct TagCtx<'a> {
    ui: &'a mut UiSelection,
    tags: &'a mut TagState,
    screen: &'a mut Screen,
    effects: Effects,
}

impl Nav for TagCtx<'_> {
    fn screen_mut(&mut self) -> &mut Screen {
        self.screen
    }
}

impl Effectful for TagCtx<'_> {
    fn effects_mut(&mut self) -> &mut Effects {
        &mut self.effects
    }
}

pub(super) fn handle_key(app: &mut App, key: KeyEvent) {
    let effects = {
        let mut ctx = TagCtx {
            ui: &mut app.ui,
            tags: &mut app.tags,
            screen: &mut app.screen,
            effects: Effects::default(),
        };
        tag_key(&mut ctx, key);
        ctx.effects
    };
    effects.apply(app);
}

fn tag_key(ctx: &mut TagCtx, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('#') => {
            ctx.set_screen(Screen::HostList);
        }
        KeyCode::Char('?') => {
            ctx.push_help_overlay();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            crate::app::cycle_selection(ctx.ui.tag_picker_state_mut(), ctx.tags.list().len(), true);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            crate::app::cycle_selection(
                ctx.ui.tag_picker_state_mut(),
                ctx.tags.list().len(),
                false,
            );
        }
        KeyCode::PageDown => {
            crate::app::page_down(ctx.ui.tag_picker_state_mut(), ctx.tags.list().len(), 10);
        }
        KeyCode::PageUp => {
            crate::app::page_up(ctx.ui.tag_picker_state_mut(), ctx.tags.list().len(), 10);
        }
        KeyCode::Enter => {
            if let Some(index) = ctx.ui.tag_picker_state().selected()
                && let Some(tag) = ctx.tags.list().get(index)
            {
                let tag: String = tag.clone();
                ctx.set_screen(Screen::HostList);
                // start_search and apply_filter recompute the host display
                // list (hosts, ping, providers), so they run on the full App
                // after the slice borrow ends. set_query sits between them
                // in the original, so defer the whole sequence to keep order.
                ctx.defer(move |app| {
                    app.start_search();
                    app.search.set_query(Some(format!("tag={}", tag)));
                    app.apply_filter();
                });
            }
        }
        _ => {}
    }
}
