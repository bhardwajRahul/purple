use crossterm::event::{KeyCode, KeyEvent};

use super::ctx::{Effectful, Effects, Nav};
use crate::app::{App, Screen};
use crate::runtime::env::Env;

/// The slice of App the welcome overlay touches: the screen and the resolved
/// environment (for the preferences path). The known-hosts import touches most
/// of App (hosts, status, ui, reload), so it runs as a deferred effect after
/// the slice borrow ends.
struct WelcomeCtx<'a> {
    screen: &'a mut Screen,
    env: &'a Env,
    effects: Effects,
}

impl Nav for WelcomeCtx<'_> {
    fn screen_mut(&mut self) -> &mut Screen {
        self.screen
    }
}

impl Effectful for WelcomeCtx<'_> {
    fn effects_mut(&mut self) -> &mut Effects {
        &mut self.effects
    }
}

pub(super) fn handle_key(app: &mut App, key: KeyEvent) {
    let effects = {
        let mut ctx = WelcomeCtx {
            screen: &mut app.screen,
            env: app.env.as_ref(),
            effects: Effects::default(),
        };
        welcome_key(&mut ctx, key);
        ctx.effects
    };
    effects.apply(app);
}

fn welcome_key(ctx: &mut WelcomeCtx, key: KeyEvent) {
    let Screen::Welcome {
        known_hosts_count, ..
    } = &*ctx.screen
    else {
        return;
    };
    let known_hosts_count = *known_hosts_count;

    // Closing Welcome seeds last_seen_version so future launches do not re-trigger the upgraded flow.
    let version = env!("CARGO_PKG_VERSION");
    if let Err(e) = crate::preferences::save_last_seen_version(ctx.env.paths(), version) {
        log::warn!("[purple] failed to seed last_seen_version on welcome close: {e}");
    }
    if key.code == KeyCode::Char('?') {
        ctx.set_screen(Screen::Help {
            return_screen: Box::new(Screen::HostList),
        });
    } else if key.code == KeyCode::Char('I') && known_hosts_count > 0 {
        ctx.set_screen(Screen::HostList);
        ctx.defer(super::confirm::execute_known_hosts_import);
    } else {
        ctx.set_screen(Screen::HostList);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh_config::model::SshConfigFile;
    use crossterm::event::KeyModifiers;

    fn make_app_on_welcome(known_hosts_count: usize) -> App {
        let scratch = tempfile::tempdir().expect("tempdir").keep();
        let config = SshConfigFile {
            elements: SshConfigFile::parse_content(""),
            path: scratch.join("test_config"),
            crlf: false,
            bom: false,
        };
        let mut app = App::new(config);
        app.screen = Screen::Welcome {
            has_backup: false,
            host_count: 0,
            known_hosts_count,
        };
        app
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn question_key_opens_help() {
        let _lock = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = make_app_on_welcome(0);
        handle_key(&mut app, k(KeyCode::Char('?')));
        assert!(matches!(app.screen, Screen::Help { .. }));
    }

    #[test]
    fn any_other_key_returns_to_host_list() {
        let _lock = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = make_app_on_welcome(0);
        handle_key(&mut app, k(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::HostList));
    }

    #[test]
    fn capital_i_with_zero_known_hosts_just_dismisses() {
        let _lock = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = make_app_on_welcome(0);
        handle_key(&mut app, k(KeyCode::Char('I')));
        assert!(matches!(app.screen, Screen::HostList));
    }

    #[test]
    fn capital_i_with_known_hosts_triggers_import_path() {
        let _lock = crate::demo_flag::GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = make_app_on_welcome(1);
        // The import reads the App's sandboxed `~/.ssh/known_hosts`. Seed it
        // with a couple of parseable host entries so `execute_known_hosts_import`
        // has something to act on.
        let ssh_dir = app.env().paths().expect("sandbox paths").ssh_dir();
        std::fs::create_dir_all(&ssh_dir).expect("create .ssh dir");
        std::fs::write(
            ssh_dir.join("known_hosts"),
            "host-one ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITESTONE\n\
             host-two ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITESTTWO\n",
        )
        .expect("write known_hosts");

        handle_key(&mut app, k(KeyCode::Char('I')));
        assert!(matches!(app.screen, Screen::HostList));
        assert!(
            app.status_center.toast().is_some() || app.status_center.status().is_some(),
            "I-key with known_hosts_count > 0 must attempt import and emit a status"
        );
    }
}
