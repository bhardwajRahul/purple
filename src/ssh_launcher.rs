//! Which program runs an interactive SSH login.
//!
//! Plain `ssh` by default. `PURPLE_SSH_COMMAND` in the environment or
//! `ssh_command` in `~/.purple/preferences` swaps it for a wrapper such as
//! kitty's `kitten ssh`. Only the interactive login paths use this;
//! tunnels, snippets, key push, scp and the MCP server keep calling `ssh`.

use std::process::Command;

use crate::runtime::env::{Env, Paths};

/// Environment variable that overrides the preference.
pub const ENV_VAR: &str = "PURPLE_SSH_COMMAND";
/// Preference key in `~/.purple/preferences`.
pub const PREF_KEY: &str = "ssh_command";
/// What runs when nothing is configured.
pub const DEFAULT_PROGRAM: &str = "ssh";

/// A resolved launcher: the program plus any leading arguments that come
/// before purple's own `-F <config> ... -- <alias>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshLauncher {
    words: Vec<String>,
}

impl Default for SshLauncher {
    fn default() -> Self {
        Self {
            words: vec![DEFAULT_PROGRAM.to_string()],
        }
    }
}

/// Why a configured value was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherError {
    /// The value was empty or whitespace only.
    Empty,
    /// A quote was opened and never closed.
    UnterminatedQuote,
}

impl std::fmt::Display for LauncherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LauncherError::Empty => write!(f, "empty command"),
            LauncherError::UnterminatedQuote => write!(f, "unterminated quote"),
        }
    }
}

impl SshLauncher {
    /// Parse a command line such as `kitten ssh` or `"/opt/kitty/bin/kitten" ssh`.
    /// Splits on whitespace, honors single and double quotes and a backslash
    /// escape outside single quotes.
    pub fn parse(spec: &str) -> Result<Self, LauncherError> {
        let words = split_words(spec)?;
        if words.is_empty() {
            return Err(LauncherError::Empty);
        }
        Ok(Self { words })
    }

    /// Resolve from the environment first, then the preference, then the
    /// default. A value that fails to parse is logged and ignored so a typo
    /// never locks the user out of connecting.
    pub fn resolve(env: &Env) -> Self {
        Self::resolve_from(env.var(ENV_VAR), env.paths())
    }

    fn resolve_from(env_value: Option<&str>, paths: Option<&Paths>) -> Self {
        if let Some(spec) = env_value.map(str::trim).filter(|s| !s.is_empty()) {
            match Self::parse(spec) {
                Ok(l) => {
                    log::debug!("[purple] ssh launcher from {ENV_VAR}: {}", l.display());
                    return l;
                }
                Err(e) => log::warn!("[config] ignoring {ENV_VAR}={spec:?}: {e}"),
            }
        }
        if let Some(spec) = crate::preferences::load_ssh_command(paths) {
            match Self::parse(&spec) {
                Ok(l) => {
                    log::debug!(
                        "[purple] ssh launcher from preference {PREF_KEY}: {}",
                        l.display()
                    );
                    return l;
                }
                Err(e) => log::warn!("[config] ignoring preference {PREF_KEY}={spec:?}: {e}"),
            }
        }
        Self::default()
    }

    /// The program to spawn.
    pub fn program(&self) -> &str {
        &self.words[0]
    }

    /// Arguments that precede purple's own ssh arguments.
    pub fn leading_args(&self) -> &[String] {
        &self.words[1..]
    }

    /// Every word, program first. Handy for argv lists such as `tmux new-window`.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// True when nothing but plain `ssh` is configured.
    pub fn is_default(&self) -> bool {
        self.words.len() == 1 && self.words[0] == DEFAULT_PROGRAM
    }

    /// A `Command` for the program with the leading arguments applied.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(self.program());
        cmd.args(self.leading_args());
        cmd
    }

    /// Space-joined form for logs.
    pub fn display(&self) -> String {
        self.words.join(" ")
    }

    /// Shell-safe form for pasting: words with spaces or shell metacharacters
    /// are single-quoted.
    pub fn shell_display(&self) -> String {
        self.words
            .iter()
            .map(|w| shell_quote(w))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Single-quote `word` unless it only holds characters a POSIX shell passes
/// through untouched.
fn shell_quote(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=+@%,".contains(c));
    if safe {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// Shell-style word split: whitespace separates, quotes group, a backslash
/// escapes the next character outside single quotes.
fn split_words(spec: &str) -> Result<Vec<String>, LauncherError> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            Some(_) => {
                if c == '"' {
                    quote = None;
                } else if c == '\\' {
                    match chars.next() {
                        Some(n) if n == '"' || n == '\\' => current.push(n),
                        Some(n) => {
                            current.push('\\');
                            current.push(n);
                        }
                        None => current.push('\\'),
                    }
                } else {
                    current.push(c);
                }
            }
            None => {
                if c.is_whitespace() {
                    if in_word {
                        words.push(std::mem::take(&mut current));
                        in_word = false;
                    }
                } else if c == '\'' || c == '"' {
                    quote = Some(c);
                    in_word = true;
                } else if c == '\\' {
                    in_word = true;
                    if let Some(n) = chars.next() {
                        current.push(n);
                    }
                } else {
                    in_word = true;
                    current.push(c);
                }
            }
        }
    }
    if quote.is_some() {
        return Err(LauncherError::UnterminatedQuote);
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(spec: &str) -> Vec<String> {
        SshLauncher::parse(spec).unwrap().words().to_vec()
    }

    #[test]
    fn default_is_plain_ssh() {
        let l = SshLauncher::default();
        assert_eq!(l.program(), "ssh");
        assert!(l.leading_args().is_empty());
        assert!(l.is_default());
        assert_eq!(l.display(), "ssh");
    }

    #[test]
    fn parse_splits_on_whitespace() {
        assert_eq!(words("kitten ssh"), vec!["kitten", "ssh"]);
        assert_eq!(words("  kitten   ssh  "), vec!["kitten", "ssh"]);
        assert_eq!(words("ssh"), vec!["ssh"]);
    }

    #[test]
    fn parse_honors_quotes_and_escapes() {
        assert_eq!(
            words("\"/Applications/kitty.app/Contents/MacOS/kitten\" ssh"),
            vec!["/Applications/kitty.app/Contents/MacOS/kitten", "ssh"]
        );
        assert_eq!(words("'/my dir/kitten' ssh"), vec!["/my dir/kitten", "ssh"]);
        assert_eq!(words("/my\\ dir/kitten ssh"), vec!["/my dir/kitten", "ssh"]);
        assert_eq!(words("\"a \\\"b\\\"\" c"), vec!["a \"b\"", "c"]);
        // Backslash inside single quotes is literal.
        assert_eq!(words("'a\\b'"), vec!["a\\b"]);
        // Backslash before an ordinary char inside double quotes stays.
        assert_eq!(words("\"a\\nb\""), vec!["a\\nb"]);
        // Empty quoted word survives as an empty argument.
        assert_eq!(words("kitten '' ssh"), vec!["kitten", "", "ssh"]);
    }

    #[test]
    fn parse_rejects_empty_and_unterminated() {
        assert_eq!(SshLauncher::parse(""), Err(LauncherError::Empty));
        assert_eq!(SshLauncher::parse("   "), Err(LauncherError::Empty));
        assert_eq!(
            SshLauncher::parse("kitten 'ssh"),
            Err(LauncherError::UnterminatedQuote)
        );
        assert_eq!(
            SshLauncher::parse("\"kitten ssh"),
            Err(LauncherError::UnterminatedQuote)
        );
    }

    #[test]
    fn program_and_leading_args_and_display() {
        let l = SshLauncher::parse("kitten ssh --kitten interpreter=python").unwrap();
        assert_eq!(l.program(), "kitten");
        assert_eq!(l.leading_args(), ["ssh", "--kitten", "interpreter=python"]);
        assert!(!l.is_default());
        assert_eq!(l.display(), "kitten ssh --kitten interpreter=python");
    }

    #[test]
    fn shell_display_quotes_only_when_needed() {
        assert_eq!(SshLauncher::default().shell_display(), "ssh");
        assert_eq!(
            SshLauncher::parse("kitten ssh").unwrap().shell_display(),
            "kitten ssh"
        );
        assert_eq!(
            SshLauncher::parse("'/my dir/kitten' ssh")
                .unwrap()
                .shell_display(),
            "'/my dir/kitten' ssh"
        );
        assert_eq!(
            SshLauncher::parse("\"it's\" ssh").unwrap().shell_display(),
            "'it'\\''s' ssh"
        );
        assert_eq!(
            SshLauncher::parse("kitten '' ssh").unwrap().shell_display(),
            "kitten '' ssh"
        );
    }

    #[test]
    fn command_carries_program_and_leading_args() {
        let l = SshLauncher::parse("kitten ssh").unwrap();
        let cmd = l.command();
        assert_eq!(cmd.get_program(), "kitten");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["ssh"]);
    }

    #[test]
    fn resolve_prefers_env_over_preference_over_default() {
        crate::preferences::tests_helpers::with_temp_prefs(|paths| {
            // Nothing configured: default.
            assert!(SshLauncher::resolve_from(None, Some(paths)).is_default());

            // Preference only.
            crate::preferences::save_ssh_command(Some(paths), "kitten ssh").unwrap();
            let l = SshLauncher::resolve_from(None, Some(paths));
            assert_eq!(l.words(), ["kitten", "ssh"]);

            // Env wins over preference.
            let l = SshLauncher::resolve_from(Some("mosh-wrapper ssh"), Some(paths));
            assert_eq!(l.words(), ["mosh-wrapper", "ssh"]);

            // Empty or blank env value is treated as unset.
            let l = SshLauncher::resolve_from(Some("   "), Some(paths));
            assert_eq!(l.words(), ["kitten", "ssh"]);
        });
    }

    #[test]
    fn resolve_ignores_unparseable_values() {
        crate::preferences::tests_helpers::with_temp_prefs(|paths| {
            // Bad env value falls through to the preference.
            crate::preferences::save_ssh_command(Some(paths), "kitten ssh").unwrap();
            let l = SshLauncher::resolve_from(Some("kitten 'ssh"), Some(paths));
            assert_eq!(l.words(), ["kitten", "ssh"]);

            // Bad preference falls through to the default.
            crate::preferences::save_ssh_command(Some(paths), "\"broken").unwrap();
            assert!(SshLauncher::resolve_from(None, Some(paths)).is_default());
        });
    }

    #[test]
    fn resolve_reads_env_var_through_env() {
        let env = Env::sandboxed().with_var(ENV_VAR, "kitten ssh");
        assert_eq!(SshLauncher::resolve(&env).words(), ["kitten", "ssh"]);
        let env = Env::sandboxed();
        assert!(SshLauncher::resolve(&env).is_default());
    }
}
