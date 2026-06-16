use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{OnceLock, RwLock};

use ratatui::style::{Color, Modifier, Style};

/// Color mode: 0 = NO_COLOR, 1 = ANSI 16, 2 = truecolor.
static COLOR_MODE: AtomicU8 = AtomicU8::new(1);

/// Whether the host terminal renders SGR 58 (styled underline color, "kitty
/// underline" extension). Defaults to 1 (supported); `init()` flips it to 0
/// for known holdouts such as macOS Terminal so styles can pick a fallback.
/// Acts as the central point for terminal-capability quirks, mirroring how
/// `messages` centralises user-facing strings.
static COLORED_UNDERLINE: AtomicU8 = AtomicU8::new(1);

/// Active theme.
static THEME: OnceLock<RwLock<ThemeDef>> = OnceLock::new();

/// A single color slot with per-tier values.
#[derive(Debug, Clone)]
pub struct ColorSlot {
    pub truecolor: Option<Color>,
    pub ansi16: Option<Color>,
    pub add_modifier: Option<Modifier>,
    pub remove_modifier: Option<Modifier>,
}

impl ColorSlot {
    pub(crate) const fn new() -> Self {
        Self {
            truecolor: None,
            ansi16: None,
            add_modifier: None,
            remove_modifier: None,
        }
    }

    pub const fn new_with_modifier(m: Modifier) -> Self {
        Self {
            truecolor: None,
            ansi16: None,
            add_modifier: Some(m),
            remove_modifier: None,
        }
    }

    /// Resolve this slot to a foreground Style based on color mode.
    pub fn to_style(&self, mode: u8) -> Style {
        let mut style = Style::default();
        match mode {
            0 => {} // NO_COLOR: no fg/bg colors
            2 => {
                if let Some(c) = self.truecolor {
                    style = style.fg(c);
                }
            }
            _ => {
                if let Some(c) = self.ansi16 {
                    style = style.fg(c);
                }
            }
        }
        if let Some(m) = self.add_modifier {
            style = style.add_modifier(m);
        }
        if let Some(m) = self.remove_modifier {
            style = style.remove_modifier(m);
        }
        style
    }

    #[allow(dead_code)]
    pub fn to_style_bg(&self, mode: u8) -> Style {
        let mut style = Style::default();
        match mode {
            0 => {}
            2 => {
                if let Some(c) = self.truecolor {
                    style = style.bg(c);
                }
            }
            _ => {
                if let Some(c) = self.ansi16 {
                    style = style.bg(c);
                }
            }
        }
        if let Some(m) = self.add_modifier {
            style = style.add_modifier(m);
        }
        if let Some(m) = self.remove_modifier {
            style = style.remove_modifier(m);
        }
        style
    }
}

/// Complete theme definition with all color slots.
#[derive(Debug, Clone)]
pub struct ThemeDef {
    pub name: String,
    pub accent: ColorSlot,
    pub accent_bg: ColorSlot,
    pub success: ColorSlot,
    pub success_dim: ColorSlot,
    pub warning: ColorSlot,
    pub error: ColorSlot,
    pub highlight: ColorSlot,
    pub border: ColorSlot,
    pub border_active: ColorSlot,
    pub fg_muted: ColorSlot,
    pub fg_bold: ColorSlot,
    pub footer_key: ColorSlot,
    pub badge: ColorSlot,
    pub selected_fg: ColorSlot,
    pub footer_key_fg: ColorSlot,
    /// Accent for the trailing `.` of the `purple.` logotype in overlay
    /// headers (Welcome, Help, What's-New). Intentionally different from
    /// `accent` so the dot reads as a separate accent glyph — mirrors
    /// the landing-page hero where the dot is cyan over purple text.
    pub logo_dot: ColorSlot,
}

impl ThemeDef {
    pub fn purple() -> Self {
        Self {
            name: "Purple".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(34, 197, 94)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(34, 197, 94)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(234, 179, 8)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(239, 68, 68)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(88, 88, 88)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn purple_purple() -> Self {
        Self {
            name: "Purple Purple".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(34, 197, 94)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(34, 197, 94)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(234, 179, 8)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(239, 68, 68)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(88, 88, 88)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(147, 51, 234)),
                ansi16: Some(Color::Magenta),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "Catppuccin Mocha".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(137, 180, 250)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(137, 180, 250)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(166, 227, 161)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(166, 227, 161)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(249, 226, 175)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(243, 139, 168)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(88, 91, 112)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(137, 180, 250)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(108, 112, 134)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(69, 71, 90)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(137, 180, 250)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::Rgb(30, 30, 46)), // Mocha Base
                ansi16: Some(Color::Black),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn dracula() -> Self {
        Self {
            name: "Dracula".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(189, 147, 249)),
                ansi16: Some(Color::Magenta),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(189, 147, 249)),
                ansi16: Some(Color::Magenta),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(80, 250, 123)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(80, 250, 123)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(241, 250, 140)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(255, 85, 85)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(68, 71, 90)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(189, 147, 249)),
                ansi16: Some(Color::Magenta),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(98, 114, 164)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(68, 71, 90)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(189, 147, 249)),
                ansi16: Some(Color::Magenta),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::Rgb(40, 42, 54)), // Dracula Background
                ansi16: Some(Color::Black),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn gruvbox_dark() -> Self {
        Self {
            name: "Gruvbox Dark".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(215, 153, 33)),
                ansi16: Some(Color::LightYellow),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(215, 153, 33)),
                ansi16: Some(Color::LightYellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(152, 151, 26)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(152, 151, 26)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(250, 189, 47)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(251, 73, 52)), // Gruvbox bright_red
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(80, 73, 69)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(215, 153, 33)),
                ansi16: Some(Color::LightYellow),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(146, 131, 116)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(80, 73, 69)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(215, 153, 33)),
                ansi16: Some(Color::LightYellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::Rgb(40, 40, 40)), // Gruvbox bg0
                ansi16: Some(Color::Black),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn nord() -> Self {
        Self {
            name: "Nord".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(136, 192, 208)),
                ansi16: Some(Color::Cyan),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(136, 192, 208)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(163, 190, 140)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(163, 190, 140)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(235, 203, 139)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(191, 97, 106)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(76, 86, 106)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(136, 192, 208)),
                ansi16: Some(Color::Cyan),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(76, 86, 106)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(76, 86, 106)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(136, 192, 208)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::Rgb(46, 52, 64)), // Nord0 polar night
                ansi16: Some(Color::Black),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn tokyo_night() -> Self {
        Self {
            name: "Tokyo Night".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(122, 162, 247)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(122, 162, 247)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(158, 206, 106)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(158, 206, 106)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(224, 175, 104)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(247, 118, 142)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(61, 89, 161)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(122, 162, 247)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(86, 95, 137)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(61, 89, 161)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(122, 162, 247)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::Rgb(26, 27, 38)), // Tokyo Night bg
                ansi16: Some(Color::Black),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn one_dark() -> Self {
        Self {
            name: "One Dark".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(97, 175, 239)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(97, 175, 239)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(152, 195, 121)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(152, 195, 121)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(229, 192, 123)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(224, 108, 117)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(62, 68, 81)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(97, 175, 239)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(92, 99, 112)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(62, 68, 81)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(97, 175, 239)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::Rgb(40, 44, 52)), // One Dark bg
                ansi16: Some(Color::Black),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn catppuccin_latte() -> Self {
        Self {
            name: "Catppuccin Latte".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(30, 102, 245)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(30, 102, 245)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(64, 160, 43)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(64, 160, 43)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(223, 142, 29)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(210, 15, 57)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(172, 176, 190)),
                ansi16: None,
                add_modifier: None, // No DIM for light themes
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(30, 102, 245)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(140, 143, 161)),
                ansi16: None,
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(172, 176, 190)),
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(30, 102, 245)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::Rgb(76, 79, 105)), // Latte Text
                ansi16: Some(Color::Black),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn solarized_light() -> Self {
        Self {
            name: "Solarized Light".to_string(),
            accent: ColorSlot {
                truecolor: Some(Color::Rgb(38, 139, 210)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            accent_bg: ColorSlot {
                truecolor: Some(Color::Rgb(38, 139, 210)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot {
                truecolor: Some(Color::Rgb(133, 153, 0)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            success_dim: ColorSlot {
                truecolor: Some(Color::Rgb(133, 153, 0)),
                ansi16: Some(Color::Green),
                add_modifier: Some(Modifier::DIM),
                remove_modifier: None,
            },
            warning: ColorSlot {
                truecolor: Some(Color::Rgb(181, 137, 0)),
                ansi16: Some(Color::Yellow),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            error: ColorSlot {
                truecolor: Some(Color::Rgb(220, 50, 47)),
                ansi16: Some(Color::Red),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot {
                truecolor: Some(Color::Rgb(147, 161, 161)),
                ansi16: None,
                add_modifier: None, // No DIM for light themes
                remove_modifier: None,
            },
            border_active: ColorSlot {
                truecolor: Some(Color::Rgb(38, 139, 210)),
                ansi16: Some(Color::Blue),
                add_modifier: None,
                remove_modifier: None,
            },
            fg_muted: ColorSlot {
                truecolor: Some(Color::Rgb(147, 161, 161)),
                ansi16: None,
                add_modifier: None, // No DIM for light themes
                remove_modifier: None,
            },
            fg_bold: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key: ColorSlot {
                truecolor: Some(Color::Rgb(7, 54, 66)), // Solarized base02
                ansi16: Some(Color::DarkGray),
                add_modifier: None,
                remove_modifier: None,
            },
            badge: ColorSlot {
                truecolor: Some(Color::Rgb(38, 139, 210)),
                ansi16: Some(Color::Blue),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot {
                truecolor: Some(Color::White),
                ansi16: Some(Color::White),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
            footer_key_fg: ColorSlot {
                truecolor: Some(Color::Rgb(238, 232, 213)), // Solarized base2
                ansi16: Some(Color::White),
                add_modifier: None,
                remove_modifier: None,
            },
            logo_dot: ColorSlot {
                truecolor: Some(Color::Rgb(0, 240, 255)),
                ansi16: Some(Color::Cyan),
                add_modifier: Some(Modifier::BOLD),
                remove_modifier: None,
            },
        }
    }

    pub fn no_color() -> Self {
        Self {
            name: "No Color".to_string(),
            accent: ColorSlot::new_with_modifier(Modifier::BOLD),
            accent_bg: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: Some(Modifier::DIM),
            },
            success: ColorSlot::new_with_modifier(Modifier::BOLD),
            success_dim: ColorSlot::new(),
            warning: ColorSlot::new_with_modifier(Modifier::BOLD),
            error: ColorSlot::new_with_modifier(Modifier::BOLD),
            highlight: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: None,
            },
            border: ColorSlot::new_with_modifier(Modifier::DIM),
            border_active: ColorSlot::new_with_modifier(Modifier::BOLD),
            fg_muted: ColorSlot::new_with_modifier(Modifier::DIM),
            fg_bold: ColorSlot::new_with_modifier(Modifier::BOLD),
            footer_key: ColorSlot::new_with_modifier(Modifier::REVERSED),
            badge: ColorSlot {
                truecolor: None,
                ansi16: None,
                add_modifier: Some(Modifier::BOLD | Modifier::REVERSED),
                remove_modifier: Some(Modifier::DIM),
            },
            selected_fg: ColorSlot::new_with_modifier(Modifier::BOLD),
            footer_key_fg: ColorSlot::new_with_modifier(Modifier::REVERSED),
            logo_dot: ColorSlot::new_with_modifier(Modifier::BOLD),
        }
    }

    pub fn builtins() -> Vec<ThemeDef> {
        vec![
            Self::purple(),
            Self::purple_purple(),
            Self::catppuccin_mocha(),
            Self::dracula(),
            Self::gruvbox_dark(),
            Self::nord(),
            Self::tokyo_night(),
            Self::one_dark(),
            Self::catppuccin_latte(),
            Self::solarized_light(),
            Self::no_color(),
        ]
    }

    pub fn find_builtin(name: &str) -> Option<ThemeDef> {
        Self::builtins()
            .into_iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn parse_toml(content: &str) -> Option<Self> {
        let mut values: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim().to_string();
                let val = val.trim();
                let val = if let Some(idx) = val.find(" #") {
                    &val[..idx]
                } else {
                    val
                };
                let val = val.trim().trim_matches('"').to_string();
                values.insert(key, val);
            }
        }
        let name = values.get("name")?.to_string();
        let theme_name = name.clone();
        let fallback = Self::purple();
        let resolve_slot = |key: &str, fb: &ColorSlot| -> ColorSlot {
            let truecolor = match values.get(key) {
                Some(v) => match parse_hex(v) {
                    Some(c) => Some(c),
                    None => {
                        log::warn!(
                            "[config] theme '{}' field '{}' has invalid hex value '{}'; falling back",
                            theme_name,
                            key,
                            v
                        );
                        fb.truecolor
                    }
                },
                None => fb.truecolor,
            };
            let ansi_key = format!("{key}_ansi");
            let ansi16 = match values.get(&ansi_key) {
                Some(v) => match parse_ansi_name(v) {
                    Some(c) => Some(c),
                    None => {
                        log::warn!(
                            "[config] theme '{}' field '{}' has invalid colour name '{}'; falling back",
                            theme_name,
                            ansi_key,
                            v
                        );
                        truecolor.and_then(auto_ansi16).or(fb.ansi16)
                    }
                },
                None => truecolor.and_then(auto_ansi16).or(fb.ansi16),
            };
            ColorSlot {
                truecolor,
                ansi16,
                add_modifier: fb.add_modifier,
                remove_modifier: fb.remove_modifier,
            }
        };
        Some(Self {
            name,
            accent: resolve_slot("accent", &fallback.accent),
            accent_bg: resolve_slot("accent_bg", &fallback.accent_bg),
            success: resolve_slot("success", &fallback.success),
            success_dim: resolve_slot("success_dim", &fallback.success_dim),
            warning: resolve_slot("warning", &fallback.warning),
            error: resolve_slot("error", &fallback.error),
            highlight: fallback.highlight,
            border: resolve_slot("border", &fallback.border),
            border_active: resolve_slot("border_active", &fallback.border_active),
            fg_muted: resolve_slot("fg_muted", &fallback.fg_muted),
            fg_bold: fallback.fg_bold,
            footer_key: resolve_slot("footer_key_bg", &fallback.footer_key),
            badge: resolve_slot("badge_bg", &fallback.badge),
            selected_fg: resolve_slot("selected_fg", &fallback.selected_fg),
            footer_key_fg: resolve_slot("footer_key_fg", &fallback.footer_key_fg),
            logo_dot: resolve_slot("logo_dot", &fallback.logo_dot),
        })
    }

    pub fn load_custom(paths: Option<&crate::runtime::env::Paths>) -> Vec<ThemeDef> {
        let Some(dir) = paths.map(crate::runtime::env::Paths::themes_dir) else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut themes = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(theme) = Self::parse_toml(&content) {
                        themes.push(theme);
                    } else {
                        log::warn!("[config] Invalid theme file: {}", path.display());
                    }
                }
            }
        }
        themes.sort_by(|a, b| a.name.cmp(&b.name));
        themes
    }
}

// ---------------------------------------------------------------------------
// TOML parser helpers
// ---------------------------------------------------------------------------

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn parse_ansi_name(s: &str) -> Option<Color> {
    match s.to_ascii_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "darkgray" | "dark_gray" => Some(Color::DarkGray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        "gray" => Some(Color::Gray),
        _ => None,
    }
}

fn auto_ansi16(color: Color) -> Option<Color> {
    let Color::Rgb(r, g, b) = color else {
        return Some(color);
    };
    let max = r.max(g).max(b);
    if max < 50 {
        return Some(Color::Black);
    }
    let is_bright = max > 170;
    if r > g && r > b {
        return Some(if is_bright {
            Color::LightRed
        } else {
            Color::Red
        });
    }
    if g > r && g > b {
        return Some(if is_bright {
            Color::LightGreen
        } else {
            Color::Green
        });
    }
    if b > r && b > g {
        return Some(if is_bright {
            Color::LightBlue
        } else {
            Color::Blue
        });
    }
    if r > 150 && g > 150 && b < 100 {
        return Some(Color::Yellow);
    }
    if r > 150 && b > 150 && g < 100 {
        return Some(Color::Magenta);
    }
    if g > 150 && b > 150 && r < 100 {
        return Some(Color::Cyan);
    }
    if r > 200 && g > 200 && b > 200 {
        return Some(Color::White);
    }
    if r > 100 && g > 100 && b > 100 {
        return Some(Color::Gray);
    }
    Some(Color::DarkGray)
}

// ---------------------------------------------------------------------------
// Global theme state
// ---------------------------------------------------------------------------

fn active_theme() -> std::sync::RwLockReadGuard<'static, ThemeDef> {
    THEME
        .get_or_init(|| RwLock::new(ThemeDef::purple()))
        .read()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn set_theme(theme: ThemeDef) {
    let lock = THEME.get_or_init(|| RwLock::new(ThemeDef::purple()));
    *lock.write().unwrap_or_else(|e| e.into_inner()) = theme;
}

pub fn current_theme() -> ThemeDef {
    active_theme().clone()
}

pub fn color_mode() -> u8 {
    COLOR_MODE.load(Ordering::Acquire)
}

/// Internal alias for color_mode().
fn mode() -> u8 {
    COLOR_MODE.load(Ordering::Acquire)
}

/// True when the terminal is expected to honour SGR 58 (colored underline).
/// Falls back to false on known legacy terminals so styles can pick a
/// non-styled-underline alternative. Detected once in `init()`.
pub fn supports_colored_underline() -> bool {
    COLORED_UNDERLINE.load(Ordering::Acquire) != 0
}

/// Pure decision: given a `TERM_PROGRAM` value, does this terminal honour
/// styled underlines (SGR 58/59)? Conservative default is `true`; only known
/// holdouts opt out. Kept separate from env reads so it stays unit-testable.
fn detects_colored_underline(term_program: Option<&str>) -> bool {
    // macOS Terminal (Terminal.app) does not implement styled underlines.
    !matches!(term_program, Some("Apple_Terminal"))
}

/// Write the detected cap flags into the atomics from the resolved env.
fn detect_terminal_caps(env: &crate::runtime::env::Env) {
    let supports = detects_colored_underline(env.term_program());
    COLORED_UNDERLINE.store(u8::from(supports), Ordering::Release);
}

/// Initialize theme settings. Call once at startup.
pub fn init(env: &crate::runtime::env::Env) {
    detect_terminal_caps(env);
    if env.no_color() {
        COLOR_MODE.store(0, Ordering::Release);
        set_theme(ThemeDef::no_color());
        return;
    }
    if env
        .colorterm()
        .map(|v| v == "truecolor" || v == "24bit")
        .unwrap_or(false)
    {
        COLOR_MODE.store(2, Ordering::Release);
    }
    if let Some(name) = crate::preferences::load_theme(env.paths()) {
        if let Some(theme) = ThemeDef::find_builtin(&name) {
            set_theme(theme);
        } else {
            let custom = ThemeDef::load_custom(env.paths());
            if let Some(theme) = custom
                .into_iter()
                .find(|t| t.name.eq_ignore_ascii_case(&name))
            {
                set_theme(theme);
            }
        }
    }
}

/// Force a specific color mode (0 = NO_COLOR, 1 = ANSI 16, 2 = truecolor).
/// Used by asset generation (`gen-assets`) to render the brand truecolor
/// palette regardless of the host terminal's reported capabilities.
pub(crate) fn set_color_mode(mode: u8) {
    COLOR_MODE.store(mode, Ordering::Release);
}

#[cfg(test)]
pub(crate) fn init_with_mode(m: u8) {
    set_color_mode(m);
    let _ = THEME.get_or_init(|| RwLock::new(ThemeDef::purple()));
}

#[cfg(test)]
pub(crate) fn set_colored_underline_support(v: bool) {
    COLORED_UNDERLINE.store(u8::from(v), Ordering::Release);
}

// ---------------------------------------------------------------------------
// Data-driven public style functions (preserving exact existing signatures)
// ---------------------------------------------------------------------------

/// Brand badge: purple background with white text. The single splash of color.
/// Truecolor: #9333EA purple bg. ANSI 16: Magenta bg. NO_COLOR: REVERSED.
/// Removes DIM so border_style doesn't leak through ratatui's Style::patch().
pub fn brand_badge() -> Style {
    let m = mode();
    let t = active_theme();
    if m == 0 {
        Style::default()
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            .remove_modifier(Modifier::DIM)
    } else {
        let mut style = t.selected_fg.to_style(m);
        style = match m {
            2 => {
                if let Some(c) = t.badge.truecolor {
                    style.bg(c)
                } else {
                    style
                }
            }
            _ => {
                if let Some(c) = t.badge.ansi16 {
                    style.bg(c)
                } else {
                    style
                }
            }
        };
        if let Some(add) = t.badge.add_modifier {
            style = style.add_modifier(add);
        }
        if let Some(rm) = t.badge.remove_modifier {
            style = style.remove_modifier(rm);
        }
        style
    }
}

/// Brand accent for dialog/popup titles.
/// Removes DIM so border_style doesn't leak through ratatui's Style::patch().
pub fn brand() -> Style {
    Style::default()
        .add_modifier(Modifier::BOLD)
        .remove_modifier(Modifier::DIM)
}

/// Active tab marker for the top navigation bar.
///
/// Modern terminals (Ghostty, iTerm2 3.4+, WezTerm, Kitty, recent VTE):
/// bold white label with a brand-coloured underline via SGR 58. Letters stay
/// white so the active tab reads as a label, not as a second brand splash.
///
/// Legacy terminals without SGR 58 (macOS Terminal): bold white label with
/// the basic SGR 4 underline only. The underline inherits the foreground
/// (white) because Terminal.app cannot decouple underline colour from fg.
/// Brand accent on the underline is sacrificed; the label stays white so
/// it does not visually compete with the solid `purple` brand badge.
pub fn nav_active() -> Style {
    let mut style = bold().add_modifier(Modifier::UNDERLINED);
    if !supports_colored_underline() {
        return style;
    }
    let m = mode();
    let accent_color = match m {
        2 => active_theme().accent.truecolor,
        1 => active_theme().accent.ansi16,
        _ => None,
    };
    if let Some(c) = accent_color {
        style = style.underline_color(c);
    }
    style
}

/// Structural border style for overlay frames and dividers. Reads the
/// `border` colour slot, which carries the DIM modifier on most themes so
/// overlay frames recede behind their content. Use for borders, dividers
/// and any other "dim chrome" that should not steal attention.
///
/// Distinct from `accent()` and `accent_bold()`: those paint with the brand
/// accent colour (purple by default), while this helper paints the dim
/// border tone. Earlier releases conflated the two under the `accent` name
/// and rendered focus cursors invisibly on the default theme; the rename to
/// `border_dim()` plus a new accent-slot-backed `accent()` fixes that.
pub fn border_dim() -> Style {
    active_theme().border.to_style(mode())
}

/// Brand-coloured emphasis without BOLD. Reads the `accent` colour slot so
/// it renders in the active theme's brand colour (purple on the default
/// theme). Use for in-content cursor indicators, focused tag spans and
/// any non-bold accent emphasis. Pair with `accent_bold()` when the same
/// content should also bold (overlay titles, focused selection, reference
/// keys in help text).
pub fn accent() -> Style {
    active_theme().accent.to_style(mode())
}

/// Keybinding keys in footer/help. Brand accent slot with BOLD.
pub fn accent_bold() -> Style {
    let mut style = active_theme().accent.to_style(mode());
    style = style.add_modifier(Modifier::BOLD);
    style
}

/// Search match highlight.
pub fn highlight_bold() -> Style {
    active_theme().highlight.to_style(mode())
}

/// Footer keycap style: background matches the dim border tone.
/// Truecolor: explicit gray bg matching typical DIM rendering.
/// ANSI 16: DarkGray bg approximates DIM borders.
/// NO_COLOR: REVERSED fallback.
pub fn footer_key() -> Style {
    let m = mode();
    if m == 0 {
        return Style::default().add_modifier(Modifier::REVERSED);
    }
    let t = active_theme();
    let mut style = t.footer_key_fg.to_style(m);
    style = match m {
        2 => {
            if let Some(c) = t.footer_key.truecolor {
                style.bg(c)
            } else {
                style
            }
        }
        _ => {
            if let Some(c) = t.footer_key.ansi16 {
                style.bg(c)
            } else {
                style
            }
        }
    };
    style
}

/// Muted/secondary text.
pub fn muted() -> Style {
    active_theme().fg_muted.to_style(mode())
}

/// Section headers (help overlay, host detail).
pub fn section_header() -> Style {
    active_theme().fg_bold.to_style(mode())
}

/// Section card titles (`╭─ TITLE ─╮`). Border tone plus BOLD so the title
/// reads as part of the card frame instead of competing with the content.
pub fn section_card_title() -> Style {
    active_theme()
        .border
        .to_style(mode())
        .add_modifier(Modifier::BOLD)
}

/// Error message. Red when color is available.
pub fn error() -> Style {
    active_theme().error.to_style(mode())
}

/// Success message. Green when color is available.
pub fn success() -> Style {
    active_theme().success.to_style(mode())
}

/// Style for online status dot. Three urgency tiers:
/// NO_COLOR = normal (no modifier), ANSI 16 = Green + DIM, truecolor = muted green + DIM.
pub fn online_dot() -> Style {
    active_theme().success_dim.to_style(mode())
}

/// Style for stable healthy attributes that are neither "live now"
/// (`online_dot`) nor a positive action outcome (`success`). Use for
/// "passphrase encrypted", "agent loaded", "TTL > threshold" and other
/// settled-good states that the user does not need to act on.
///
/// Currently shares the `success_dim` colour slot with `online_dot()` so
/// the visual tone is identical; the distinction lives in intent, not in
/// pixels. Centralising the use through this helper lets a future theme
/// or NO_COLOR fallback adjust the two tiers independently without a
/// codebase-wide sweep.
pub fn healthy() -> Style {
    active_theme().success_dim.to_style(mode())
}

/// Style for the active-tunnel indicator on host-list rows. Brand accent
/// colour without BOLD. Named so the call sites read as intent ("this row
/// has an active tunnel"), not as pixel choice.
pub fn tunnel_active() -> Style {
    active_theme().accent.to_style(mode())
}

/// Style for user-defined tag spans rendered in the host list and detail
/// panel. Brand accent colour without BOLD. Named so the call sites read
/// as intent ("this token is a user tag") rather than as styling choice.
pub fn tag_user() -> Style {
    active_theme().accent.to_style(mode())
}

/// Breathing variant of `online_dot()` for per-host indicators on the host
/// list. Cycles through three states over ~2.4s (30 ticks at 80ms) to give
/// reachable hosts a subtle "alive" pulse without the constant attention
/// pull of a blinking glyph or moving shape:
///
/// - trough: `success_dim` (current static look)
/// - mid:    `success` (regular green)
/// - peak:   `success` + BOLD
///
/// Sine-driven so transitions are gradual rather than discrete blinks. At
/// `tick = 0` the cycle starts at mid (Regular green), so a freshly-rendered
/// frame in tests/visual regressions is reproducible. Down/error/checking
/// hosts deliberately stay static — the contrast is the signal.
pub fn online_dot_pulsing(tick: u64) -> Style {
    use ratatui::style::Modifier;
    const PERIOD: u64 = 30;
    let phase = (tick % PERIOD) as f32 * std::f32::consts::TAU / PERIOD as f32;
    // Map sin(-1..1) to alpha 0..1, then to brightness 0.40..1.00 so even
    // the trough remains clearly visible (60-100% alpha range from the
    // design brief). Using a smooth sine — never thresholded — eliminates
    // the discrete "blink" you get when stepping between BOLD/Regular/DIM
    // modifiers in ANSI 16.
    let alpha = 0.40 + 0.60 * (phase.sin() * 0.5 + 0.5);
    let m = mode();
    if m == 2 {
        // Truecolor: lerp the success_dim RGB toward white by `alpha`.
        // success and success_dim share the same base hue per theme so we
        // can read either; we read `success` because it is the canonical
        // "fully alive" colour at alpha=1.0.
        let base = active_theme().success.truecolor;
        if let Some(ratatui::style::Color::Rgb(r, g, b)) = base {
            let lerp = |c: u8| -> u8 {
                let f = c as f32 / 255.0;
                // Lerp between a dimmed version (0.55 of base) and the
                // base itself, controlled by alpha. Keeps the hue, only
                // varies brightness.
                let dim_f = f * 0.55;
                ((dim_f + (f - dim_f) * alpha).clamp(0.0, 1.0) * 255.0).round() as u8
            };
            return ratatui::style::Style::default().fg(ratatui::style::Color::Rgb(
                lerp(r),
                lerp(g),
                lerp(b),
            ));
        }
        // Fallthrough if theme has no truecolor value (NoColor theme).
    }
    // ANSI 16 / NO_COLOR fallback: discrete 3-state cycle on modifiers.
    // Less smooth than truecolor lerp but the only option when we cannot
    // address sub-palette brightness. Most users on modern terminals run
    // in truecolor mode and get the smooth path above.
    if alpha > 0.85 {
        active_theme().success.to_style(m)
    } else if alpha < 0.55 {
        active_theme().success_dim.to_style(m)
    } else {
        active_theme()
            .success
            .to_style(m)
            .remove_modifier(Modifier::BOLD)
    }
}

/// Warning message. Yellow/amber when color is available.
pub fn warning() -> Style {
    active_theme().warning.to_style(mode())
}

/// Toast border for success/confirmation messages.
pub fn toast_border_success() -> Style {
    let m = mode();
    let t = active_theme();
    let mut style = t.success.to_style(m);
    if m == 0 {
        style = Style::default().add_modifier(Modifier::BOLD);
    }
    style
}

/// Toast border for error messages (red, bold-on-NO_COLOR).
pub fn toast_border_error() -> Style {
    let m = mode();
    let t = active_theme();
    let mut style = t.error.to_style(m);
    if m == 0 {
        style = Style::default().add_modifier(Modifier::BOLD);
    }
    style
}

/// Toast border for warning messages (yellow, bold-on-NO_COLOR).
/// Visually distinct from `toast_border_error` so warnings (recoverable)
/// and errors (require acknowledgement) can be told apart at a glance.
pub fn toast_border_warning() -> Style {
    let m = mode();
    let t = active_theme();
    let mut style = t.warning.to_style(m);
    if m == 0 {
        style = Style::default().add_modifier(Modifier::BOLD);
    }
    style
}

/// Danger action key (delete confirmation). Red when color is available.
pub fn danger() -> Style {
    active_theme().error.to_style(mode())
}

/// Default border (unfocused).
pub fn border() -> Style {
    active_theme().border.to_style(mode())
}

/// Version number in help overlay. Purple foreground.
pub fn version() -> Style {
    active_theme().accent.to_style(mode())
}

/// Search-mode border. Purple to signal active filter state.
pub fn border_search() -> Style {
    active_theme().border_active.to_style(mode())
}

/// Accent colour for the trailing `.` of the `purple.` logotype in overlay
/// headers. Intentionally distinct from `accent_bold` so the dot echoes
/// the cyan accent from the landing-page hero.
pub fn logo_dot() -> Style {
    active_theme().logo_dot.to_style(mode())
}

/// Selected item in a list. Purple highlight for brand consistency.
pub fn selected_row() -> Style {
    let m = mode();
    let t = active_theme();
    if m == 0 {
        return Style::default()
            .add_modifier(Modifier::REVERSED)
            .remove_modifier(Modifier::DIM);
    }
    let mut style = t.selected_fg.to_style(m);
    style = match m {
        2 => {
            if let Some(c) = t.accent_bg.truecolor {
                style.bg(c)
            } else {
                style
            }
        }
        _ => {
            if let Some(c) = t.accent_bg.ansi16 {
                style.bg(c)
            } else {
                style
            }
        }
    };
    if let Some(add) = t.accent_bg.add_modifier {
        style = style.add_modifier(add);
    }
    if let Some(rm) = t.accent_bg.remove_modifier {
        style = style.remove_modifier(rm);
    }
    style
}

/// Danger border (delete dialog). Red when color is available.
pub fn border_danger() -> Style {
    active_theme().error.to_style(mode())
}

/// Bold text (labels, emphasis).
pub fn bold() -> Style {
    active_theme().fg_bold.to_style(mode())
}

/// Update available badge. Purple background to stand out in the title bar.
pub fn update_badge() -> Style {
    brand_badge()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
