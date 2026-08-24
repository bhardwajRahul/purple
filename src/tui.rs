use std::io::{self, Stdout, stdout};
use std::sync::Once;

use anyhow::Result;
use crossterm::{
    ExecutableCommand, QueueableCommand,
    cursor::MoveTo,
    style::ResetColor,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::{Terminal, prelude::CrosstermBackend};

use log::debug;

use crate::app::App;
use crate::ui;

static PANIC_HOOK: Once = Once::new();

pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    /// Enter TUI mode: panic hook (installed once), raw mode, alternate screen.
    pub fn enter(&mut self) -> Result<()> {
        // Install panic hook BEFORE enabling raw mode to ensure cleanup on panic
        PANIC_HOOK.call_once(|| {
            let original_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic_info| {
                let _ = Self::reset();
                original_hook(panic_info);
            }));
        });

        enable_raw_mode()?;
        if let Err(e) = io::stdout().execute(EnterAlternateScreen) {
            disable_raw_mode()?;
            return Err(e.into());
        }

        if let Err(e) = self.terminal.hide_cursor() {
            let _ = Self::reset();
            return Err(e.into());
        }
        if let Err(e) = self.terminal.clear() {
            let _ = Self::reset();
            return Err(e.into());
        }
        Ok(())
    }

    /// Exit TUI mode: restore terminal.
    pub fn exit(&mut self) -> Result<()> {
        Self::reset()?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    /// Exit TUI mode and hand the next process a blank screen.
    /// Terminals without alternate-screen support keep the TUI painted
    /// on the main buffer, so without this clear a spawned ssh would
    /// draw on top of the old frame.
    pub fn suspend(&mut self) -> Result<()> {
        self.exit()?;
        queue_clear_home(&mut io::stdout())?;
        Ok(())
    }

    /// Reset terminal to normal mode.
    fn reset() -> Result<()> {
        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    /// Draw the UI.
    pub fn draw(
        &mut self,
        app: &mut App,
        anim: &mut crate::animation::AnimationState,
    ) -> Result<()> {
        self.terminal.draw(|frame| ui::render(frame, app, anim))?;
        Ok(())
    }

    /// Force a full redraw on the next draw() call.
    /// Use after external processes may have written to the terminal.
    pub fn force_redraw(&mut self) {
        if let Err(e) = self.terminal.clear() {
            debug!("[purple] Failed to clear terminal: {e}");
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = Self::reset();
        let _ = self.terminal.show_cursor();
    }
}

/// Queue a color reset, a full-screen clear and a cursor home into `w`.
/// The reset comes first because the clear fills the screen with the
/// active background color. Split out from `suspend` so tests can
/// assert the byte sequence.
fn queue_clear_home<W: io::Write>(w: &mut W) -> io::Result<()> {
    w.queue(ResetColor)?;
    w.queue(Clear(ClearType::All))?;
    w.queue(MoveTo(0, 0))?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_clear_home_emits_reset_clear_then_home() {
        let mut buf: Vec<u8> = Vec::new();
        queue_clear_home(&mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let reset_pos = s.find("\x1b[0m").expect("color-reset sequence present");
        let clear_pos = s.find("\x1b[2J").expect("clear-all sequence present");
        let home_pos = s.find("\x1b[1;1H").expect("cursor-home sequence present");
        assert!(reset_pos < clear_pos, "color reset must run before clear");
        assert!(clear_pos < home_pos, "clear must run before cursor home");
    }
}
