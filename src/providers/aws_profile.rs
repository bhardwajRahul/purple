//! AWS shared-config profiles: the `~/.aws/config` and `~/.aws/credentials`
//! files, merged per profile name, plus the assume-role chain they describe.
//!
//! The AWS CLI reads both files and merges them per profile, with the
//! credentials file winning key by key. Named profiles are `[profile <name>]`
//! in the config file and `[<name>]` in the credentials file; `default` is
//! bare in both.

use std::collections::HashMap;
use std::path::Path;

/// How far a `source_profile` chain may nest before we call it a loop. AWS
/// documents no limit; real chains are one or two hops, and STS itself refuses
/// to chain a session beyond an hour, so anything deeper is a config mistake.
const MAX_SOURCE_PROFILE_DEPTH: usize = 8;

/// One profile's settings, already merged across the two files.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AwsProfile {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub role_arn: String,
    pub source_profile: String,
    pub credential_source: String,
    pub role_session_name: String,
    pub external_id: String,
    pub duration_seconds: String,
    pub mfa_serial: String,
    pub region: String,
    pub credential_process: String,
    /// `web_identity_token_file`: the OIDC token an EKS pod role or a CI job
    /// signs in with. The AWS CLI hands a profile carrying it to its web
    /// identity provider and keeps the assume-role provider away from it, so
    /// it is not a `source_profile` chain even with a `role_arn` beside it.
    pub web_identity_token_file: String,
    /// Set when the profile configures IAM Identity Center, through either the
    /// `sso_session` form or the legacy `sso_start_url` form.
    pub sso: bool,
    /// True when the key pair came from `~/.aws/credentials` rather than from
    /// `~/.aws/config`. The AWS CLI reads that file ahead of
    /// `credential_process` and the config file after it, so a profile
    /// carrying both resolves differently depending on which file the keys
    /// are in.
    pub keys_from_credentials_file: bool,
}

/// Manual `Debug` so a key pair read out of `~/.aws/credentials` never reaches
/// a log line, a panic message or test output through `{:?}`. Matches what
/// `ProviderSection` does with the provider token, and what the deliberate
/// absence of `Debug` on `AwsCredentials` protects further down the same
/// pipeline. Everything holding an `AwsProfile` inherits this.
impl std::fmt::Debug for AwsProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsProfile")
            .field(
                "access_key_id",
                &super::config::redacted(&self.access_key_id),
            )
            .field(
                "secret_access_key",
                &super::config::redacted(&self.secret_access_key),
            )
            .field(
                "session_token",
                &super::config::redacted(&self.session_token),
            )
            .field("role_arn", &self.role_arn)
            .field("source_profile", &self.source_profile)
            .field("credential_source", &self.credential_source)
            .field("role_session_name", &self.role_session_name)
            .field("external_id", &self.external_id)
            .field("duration_seconds", &self.duration_seconds)
            .field("mfa_serial", &self.mfa_serial)
            .field("region", &self.region)
            .field("credential_process", &self.credential_process)
            .field("web_identity_token_file", &self.web_identity_token_file)
            .field("sso", &self.sso)
            .field(
                "keys_from_credentials_file",
                &self.keys_from_credentials_file,
            )
            .finish()
    }
}

impl AwsProfile {
    /// Whether the profile carries a usable static key pair.
    pub fn has_static_keys(&self) -> bool {
        !self.access_key_id.is_empty() && !self.secret_access_key.is_empty()
    }

    /// Whether the profile offers a key pair at all, complete or not. This is
    /// the test the AWS CLI applies when it decides that a profile is a
    /// credential source rather than another link in a chain, so half a pair
    /// stops the walk and is reported instead of being walked past.
    fn offers_static_keys(&self) -> bool {
        !self.access_key_id.is_empty() || !self.secret_access_key.is_empty()
    }

    /// Apply one `key = value` pair. Unknown keys are ignored, which is what
    /// lets the same parser read files carrying settings purple has no use for.
    fn set(&mut self, key: &str, value: &str) {
        let value = value.to_string();
        match key {
            "aws_access_key_id" => self.access_key_id = value,
            "aws_secret_access_key" => self.secret_access_key = value,
            "aws_session_token" => self.session_token = value,
            "role_arn" => self.role_arn = value,
            "source_profile" => self.source_profile = value,
            "credential_source" => self.credential_source = value,
            "role_session_name" => self.role_session_name = value,
            "external_id" => self.external_id = value,
            "duration_seconds" => self.duration_seconds = value,
            "mfa_serial" => self.mfa_serial = value,
            "region" => self.region = value,
            "credential_process" => self.credential_process = value,
            "web_identity_token_file" => self.web_identity_token_file = value,
            "sso_session" | "sso_start_url" => self.sso = true,
            _ => {}
        }
    }
}

/// Every profile found in the two files, keyed by profile name.
#[derive(Debug, Clone, Default)]
pub struct AwsProfiles {
    profiles: HashMap<String, AwsProfile>,
    /// Files that are there but could not be opened, in load order. A file
    /// purple cannot read looks exactly like an empty one, so this is what
    /// keeps an absent profile from being reported as missing.
    unreadable: Vec<String>,
    /// The two files as they were asked for, in load order. `AWS_CONFIG_FILE`
    /// and `AWS_SHARED_CREDENTIALS_FILE` move them, so a message that sends
    /// the user to a file names the one purple actually read.
    sources: Vec<String>,
}

impl AwsProfiles {
    /// Read both files. A missing file is not an error: a user with only
    /// `~/.aws/credentials` is the common case, and so is only `~/.aws/config`.
    pub fn load(config_file: Option<&Path>, credentials_file: Option<&Path>) -> Self {
        let mut profiles: HashMap<String, AwsProfile> = HashMap::new();
        let mut unreadable: Vec<String> = Vec::new();
        let mut sources: Vec<String> = Vec::new();
        // The credentials file is read second, so its keys overwrite the
        // config file's per the AWS precedence rule.
        for (path, style) in [
            (config_file, SectionStyle::Config),
            (credentials_file, SectionStyle::Credentials),
        ] {
            let Some(path) = path else { continue };
            sources.push(path.display().to_string());
            match std::fs::read_to_string(path) {
                Ok(text) => merge_into(&mut profiles, parse(&text, style), style),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    log::warn!(
                        "[config] aws profiles: cannot read {}: {}",
                        path.display(),
                        e
                    );
                    unreadable.push(path.display().to_string());
                }
            }
        }
        Self {
            profiles,
            unreadable,
            sources,
        }
    }

    pub fn get(&self, name: &str) -> Option<&AwsProfile> {
        self.profiles.get(name)
    }

    /// The files this set was read from, for a message that sends the user to
    /// one of them. Falls back to the AWS defaults when no path was given.
    pub fn sources(&self) -> String {
        if self.sources.is_empty() {
            return "~/.aws/config and ~/.aws/credentials".to_string();
        }
        self.sources.join(" and ")
    }

    /// Profile names, sorted, for error messages that list what is available.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.profiles.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Walk a `source_profile` chain from `name` down to the profile holding
    /// real keys. Returns the base profile first and each role to assume after
    /// it, outermost last, so a caller assumes them in order.
    ///
    /// Errors name the profile at fault rather than the one the user asked for,
    /// because a chain hides which link is broken.
    pub fn resolve_chain(&self, name: &str) -> Result<ResolvedChain, ChainError> {
        let mut visited: Vec<String> = Vec::new();
        let mut roles: Vec<RoleStep> = Vec::new();
        let mut current = name.to_string();
        // Only the profile the user named follows its own `role_arn` come what
        // may. The AWS CLI resolves a profile it reached through
        // `source_profile` with its provider chain, which stops at that
        // profile's keys and never looks at its role.
        let mut requested = true;

        loop {
            if visited.iter().any(|seen| seen == &current) {
                return Err(ChainError::Loop(current));
            }
            if visited.len() >= MAX_SOURCE_PROFILE_DEPTH {
                return Err(ChainError::TooDeep(current));
            }
            visited.push(current.clone());

            let Some(profile) = self.get(&current) else {
                return Err(self.name_the_file_if_unread(ChainError::Missing(current)));
            };

            // The AWS CLI keeps its assume-role provider away from a profile
            // that signs in with an OIDC token, whatever else it carries, so
            // this is not a chain to walk.
            if !profile.web_identity_token_file.is_empty() {
                return Err(ChainError::WebIdentity(current));
            }

            if !profile.role_arn.is_empty() && (requested || !profile.offers_static_keys()) {
                if !profile.credential_source.is_empty() {
                    return Err(ChainError::CredentialSource(current));
                }
                if !profile.mfa_serial.is_empty() {
                    return Err(ChainError::MfaRequired(current));
                }
                if profile.source_profile.is_empty() {
                    return Err(ChainError::RoleWithoutSource(current));
                }
                // Push in walk order, reversed once the base is found, so the
                // caller assumes the innermost role first.
                roles.push(RoleStep {
                    profile: current.clone(),
                    role_arn: profile.role_arn.clone(),
                    role_session_name: profile.role_session_name.clone(),
                    external_id: profile.external_id.clone(),
                    duration_seconds: profile.duration_seconds.clone(),
                });
                if profile.source_profile != current {
                    current = profile.source_profile.clone();
                    requested = false;
                    continue;
                }
                // A profile may name itself as its own source, which is how
                // AWS documents keeping static keys and a role in one block.
                // With no key of its own it is a plain cycle, which is what
                // the AWS CLI calls it too. Otherwise this same profile is the
                // base, so fall through to the checks below.
                if profile.access_key_id.is_empty() && profile.secret_access_key.is_empty() {
                    return Err(ChainError::Loop(current));
                }
            }

            base_outcome(profile, &current).map_err(|e| self.name_the_file_if_unread(e))?;
            let base = profile.clone();
            roles.reverse();
            return Ok(ResolvedChain {
                base,
                base_profile: current,
                roles,
            });
        }
    }

    /// A file purple could not open parses as empty, so an outcome that turns
    /// on a key not being there names that file instead of the profile. The
    /// keys may well be sitting in it.
    fn name_the_file_if_unread(&self, error: ChainError) -> ChainError {
        match (&error, self.unreadable.first()) {
            (
                ChainError::Missing(_) | ChainError::NoKeys(_) | ChainError::PartialCredentials(..),
                Some(path),
            ) => ChainError::FileUnreadable(path.clone()),
            _ => error,
        }
    }
}

/// Whether a profile can serve as the base of a chain, in the order the AWS
/// CLI's own provider chain applies: Identity Center first, then the static
/// key pair, then the providers purple cannot run.
fn base_outcome(profile: &AwsProfile, name: &str) -> Result<(), ChainError> {
    if profile.sso {
        return Err(ChainError::SsoNotSupported(name.to_string()));
    }
    let has_id = !profile.access_key_id.is_empty();
    let has_secret = !profile.secret_access_key.is_empty();
    if has_id != has_secret {
        let missing = if has_id {
            "aws_secret_access_key"
        } else {
            "aws_access_key_id"
        };
        return Err(ChainError::PartialCredentials(name.to_string(), missing));
    }
    // The AWS CLI reads `~/.aws/credentials` before it runs
    // `credential_process` and `~/.aws/config` after it, so which file the
    // pair came from decides which one wins.
    if !profile.credential_process.is_empty() && !profile.keys_from_credentials_file {
        return Err(ChainError::CredentialProcess(name.to_string()));
    }
    if profile.has_static_keys() {
        return Ok(());
    }
    if !profile.credential_process.is_empty() {
        return Err(ChainError::CredentialProcess(name.to_string()));
    }
    if !profile.credential_source.is_empty() {
        return Err(ChainError::CredentialSource(name.to_string()));
    }
    Err(ChainError::NoKeys(name.to_string()))
}

/// One `role_arn` to assume on the way to the requested profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleStep {
    /// The profile this step came from, so an error can name it.
    pub profile: String,
    pub role_arn: String,
    pub role_session_name: String,
    pub external_id: String,
    pub duration_seconds: String,
}

/// A profile chain resolved down to its credential-bearing base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChain {
    pub base: AwsProfile,
    pub base_profile: String,
    /// Roles to assume in order, innermost first. Empty for a plain profile.
    pub roles: Vec<RoleStep>,
}

/// Why a profile chain could not be resolved. Each variant carries the profile
/// name at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    Missing(String),
    NoKeys(String),
    /// Half a key pair: the profile name, then the key it is missing.
    PartialCredentials(String, &'static str),
    RoleWithoutSource(String),
    CredentialSource(String),
    CredentialProcess(String),
    /// `web_identity_token_file`: an OIDC sign-in purple does not perform.
    WebIdentity(String),
    SsoNotSupported(String),
    MfaRequired(String),
    Loop(String),
    TooDeep(String),
    /// A file that is there but could not be opened. Carries the path rather
    /// than a profile name, because no profile could be read out of it.
    FileUnreadable(String),
}

/// Which section-header spelling a file uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SectionStyle {
    /// `~/.aws/config`: `[default]` and `[profile <name>]`.
    Config,
    /// `~/.aws/credentials`: `[default]` and `[<name>]`.
    Credentials,
}

/// The two keys that make a section own the credential set. A section
/// carrying either one replaces all three, so a secret from `~/.aws/config`
/// never pairs with an access key id from `~/.aws/credentials`. The AWS CLI
/// reads the pair from one file too, and signing with a mixed pair fails with
/// nothing but `SignatureDoesNotMatch` to go on.
///
/// `aws_session_token` is cleared with them but does not trigger the clear: a
/// section holding only a token has no pair of its own to offer, so it must
/// not blank the one the other file supplied.
const CREDENTIAL_PAIR_KEYS: [&str; 2] = ["aws_access_key_id", "aws_secret_access_key"];

/// Merge parsed sections into the running map, key by key, so a later file
/// overrides only the settings it actually carries.
fn merge_into(
    target: &mut HashMap<String, AwsProfile>,
    parsed: Vec<(String, Vec<(String, String)>)>,
    style: SectionStyle,
) {
    for (name, pairs) in parsed {
        let entry = target.entry(name).or_default();
        if pairs
            .iter()
            .any(|(key, _)| CREDENTIAL_PAIR_KEYS.contains(&key.as_str()))
        {
            entry.access_key_id.clear();
            entry.secret_access_key.clear();
            entry.session_token.clear();
            entry.keys_from_credentials_file = style == SectionStyle::Credentials;
        }
        for (key, value) in pairs {
            entry.set(&key, &value);
        }
    }
}

/// Parse an AWS INI file into `(profile name, key-value pairs)`, one entry per
/// section header.
///
/// Returns raw pairs rather than `AwsProfile` so the caller can merge two files
/// key by key; building a struct per file would let an absent key in the second
/// file blank one the first file set. Two headers naming the same profile stay
/// two entries, so [`merge_into`] applies each one on its own and the
/// credential set never splits across them.
fn parse(text: &str, style: SectionStyle) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut current: Option<usize> = None;
    // AWS nests a sub-setting under its parent by indenting it, and the parent
    // is always a key with an empty value (`s3 =`). Tracking that is what
    // separates a real sub-setting from a plainly indented setting, which
    // hand-written files do use.
    let mut inside_nested = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            inside_nested = false;
            current = match section_name(line, style) {
                Some(name) => {
                    out.push((name, Vec::new()));
                    Some(out.len() - 1)
                }
                None => None,
            };
            continue;
        }
        let indented = raw.starts_with(' ') || raw.starts_with('\t');
        if indented && inside_nested {
            continue;
        }
        let Some(idx) = current else { continue };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        let value = clean_value(value);
        if !indented {
            inside_nested = value.is_empty();
        }
        out[idx].1.push((key, value));
    }
    out
}

/// The profile name a section header names, or None when the header is not a
/// profile (an `[sso-session x]` or `[services x]` block).
fn section_name(line: &str, style: SectionStyle) -> Option<String> {
    let close = line.find(']')?;
    let inner = line[1..close].trim();
    if inner.is_empty() {
        return None;
    }
    // A fully quoted header is one name, spaces and all, so it is recognized
    // before any whitespace split: `["my work"]` is the profile `my work`,
    // not a keyword followed by a name.
    if inner.starts_with('"') && inner.ends_with('"') && inner.len() >= 2 {
        let name = unquote(inner);
        return (!name.is_empty()).then(|| name.to_string());
    }
    // Otherwise the `profile` keyword is split off first, then the name is
    // unquoted, so `[profile "my work"]` also yields `my work`. Quoted names
    // are accepted because boto3 documents that form, even though the
    // file-format reference says names hold no spaces.
    match inner.split_once(char::is_whitespace) {
        Some((kind, rest)) => {
            if style == SectionStyle::Config && kind == "profile" {
                let name = unquote(rest.trim());
                (!name.is_empty()).then(|| name.to_string())
            } else {
                // `[sso-session x]`, `[services x]` or a `[profile x]` header
                // in the credentials file, which AWS does not define.
                None
            }
        }
        None => {
            let name = unquote(inner);
            // The AWS CLI's own single-word sections in `~/.aws/config`. They
            // carry settings rather than credentials, and a profile of the
            // same name would be written `[profile plugins]`.
            if style == SectionStyle::Config && CONFIG_ONLY_SECTIONS.contains(&name) {
                return None;
            }
            Some(name.to_string())
        }
    }
}

/// Single-word `~/.aws/config` sections that are not profiles.
const CONFIG_ONLY_SECTIONS: [&str; 2] = ["plugins", "preview"];

/// Drop one surrounding pair of single or double quotes.
fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(value)
}

/// Trim a value and drop a trailing comment.
///
/// A `#` or `;` only starts a comment when whitespace precedes it, so a value
/// like `foo#1` stays intact. Surrounding quotes are dropped, matching what the
/// Go and Java SDKs do.
fn clean_value(value: &str) -> String {
    let mut cut = value.len();
    let bytes = value.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if (*b == b'#' || *b == b';') && i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            cut = i;
            break;
        }
    }
    unquote(value[..cut].trim()).to_string()
}

#[cfg(test)]
#[path = "aws_profile_tests.rs"]
mod tests;
