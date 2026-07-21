//! Accessors, mutators and converters for a single `Host <pattern>` block.
//!
//! Everything that reads or writes `# purple:*` comment metadata, SSH
//! directives, or round-trip formatting for one block lives here. The
//! type definition itself and the rest of the file-level model stay in
//! [`super::model`].

use super::model::{Directive, HostBlock, HostEntry, PatternEntry};

impl HostBlock {
    /// Index of the first trailing blank line (for inserting content before separators).
    pub(super) fn content_end(&self) -> usize {
        let mut pos = self.directives.len();
        while pos > 0 {
            if self.directives[pos - 1].is_non_directive
                && self.directives[pos - 1].raw_line.trim().is_empty()
            {
                pos -= 1;
            } else {
                break;
            }
        }
        pos
    }

    /// Remove and return trailing blank lines.
    #[allow(dead_code)]
    pub(super) fn pop_trailing_blanks(&mut self) -> Vec<Directive> {
        let end = self.content_end();
        self.directives.drain(end..).collect()
    }

    /// Ensure exactly one trailing blank line.
    #[allow(dead_code)]
    pub(super) fn ensure_trailing_blank(&mut self) {
        self.pop_trailing_blanks();
        self.directives.push(Directive {
            key: String::new(),
            value: String::new(),
            raw_line: String::new(),
            is_non_directive: true,
        });
    }

    /// Detect indentation used by existing directives (falls back to "  ").
    pub(super) fn detect_indent(&self) -> String {
        for d in &self.directives {
            if !d.is_non_directive && !d.raw_line.is_empty() {
                let trimmed = d.raw_line.trim_start();
                let indent_len = d.raw_line.len() - trimmed.len();
                if indent_len > 0 {
                    return d.raw_line[..indent_len].to_string();
                }
            }
        }
        "  ".to_string()
    }

    /// Extract tags from purple:tags comment in directives.
    pub fn tags(&self) -> Vec<String> {
        for d in &self.directives {
            if d.is_non_directive {
                let trimmed = d.raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("# purple:tags ") {
                    return rest
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// Extract provider-synced tags from purple:provider_tags comment.
    pub fn provider_tags(&self) -> Vec<String> {
        for d in &self.directives {
            if d.is_non_directive {
                let trimmed = d.raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("# purple:provider_tags ") {
                    return rest
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// Check if a purple:provider_tags comment exists (even if empty).
    /// Used to distinguish "never migrated" from "migrated with no tags".
    pub fn has_provider_tags_comment(&self) -> bool {
        self.directives.iter().any(|d| {
            d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:provider_tags" || t.starts_with("# purple:provider_tags ")
            }
        })
    }

    /// Extract the last-synced provider username from a purple:provider_user
    /// comment. Returns `None` when absent or empty. The `_user`/`_key` suffix
    /// (no space) keeps these clear of the `# purple:provider ` marker parser.
    pub fn provider_user(&self) -> Option<String> {
        self.single_value_marker("# purple:provider_user")
    }

    /// Extract the last-synced provider identity file from a
    /// purple:provider_key comment. Returns `None` when absent or empty.
    pub fn provider_key(&self) -> Option<String> {
        self.single_value_marker("# purple:provider_key")
    }

    /// Read a single-value `# purple:<name> <value>` comment, trimmed.
    /// Returns `None` when the comment is absent or carries no value.
    fn single_value_marker(&self, prefix: &str) -> Option<String> {
        let with_space = format!("{} ", prefix);
        for d in &self.directives {
            if !d.is_non_directive {
                continue;
            }
            let trimmed = d.raw_line.trim();
            if let Some(rest) = trimmed.strip_prefix(with_space.as_str()) {
                let value = rest.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
        None
    }

    /// Extract provider info from purple:provider comment in directives.
    /// Returns (provider_name, server_id), e.g. ("digitalocean", "412345678").
    /// Label is dropped here; use `provider_id()` for the full identifier.
    pub fn provider(&self) -> Option<(String, String)> {
        self.provider_id()
            .map(|(id, server_id)| (id.provider, server_id))
    }

    /// Raw 2-segment interpretation of the purple:provider marker.
    /// Splits on the FIRST colon only and returns (provider_name, full_tail).
    /// For `proxmox:qemu:300` this yields `("proxmox", "qemu:300")`, NOT
    /// `("proxmox:qemu", "300")` like the label-aware `provider_id()` would.
    ///
    /// Use this when you need to claim or compare every marker for a provider
    /// regardless of whether the middle segment happens to look like a label.
    /// Server_ids containing colons (Proxmox `qemu:N`, OCI compartment paths)
    /// otherwise produce false labeled-marker interpretations.
    pub fn provider_raw(&self) -> Option<(String, String)> {
        for d in &self.directives {
            if !d.is_non_directive {
                continue;
            }
            let trimmed = d.raw_line.trim();
            let rest = match trimmed.strip_prefix("# purple:provider ") {
                Some(r) => r.trim(),
                None => continue,
            };
            let (provider, server_id) = rest.split_once(':')?;
            let provider = provider.trim();
            let server_id = server_id.trim();
            if provider.is_empty() || server_id.is_empty() {
                return None;
            }
            return Some((provider.to_string(), server_id.to_string()));
        }
        None
    }

    /// Extract provider info as `(ProviderConfigId, server_id)`.
    /// Supports both 2-segment legacy markers (`provider:server_id`) and
    /// 3-segment labeled markers (`provider:label:server_id`).
    pub fn provider_id(&self) -> Option<(crate::providers::config::ProviderConfigId, String)> {
        for d in &self.directives {
            if !d.is_non_directive {
                continue;
            }
            let trimmed = d.raw_line.trim();
            let rest = match trimmed.strip_prefix("# purple:provider ") {
                Some(r) => r.trim(),
                None => continue,
            };
            // splitn(3) so a server_id containing ':' (rare, but possible)
            // ends up wholly in the last segment.
            let parts: Vec<&str> = rest.splitn(3, ':').collect();
            return match parts.as_slice() {
                [provider, server_id] => {
                    let provider = provider.trim();
                    let server_id = server_id.trim();
                    if provider.is_empty() || server_id.is_empty() {
                        return None;
                    }
                    Some((
                        crate::providers::config::ProviderConfigId::bare(provider),
                        server_id.to_string(),
                    ))
                }
                [provider, label, server_id] => {
                    let label = label.trim();
                    let provider = provider.trim();
                    let server_id = server_id.trim();
                    // Empty provider or empty server_id: malformed marker.
                    // Returning None makes the host appear unowned, which is
                    // safer than guessing an interpretation that could let
                    // sync claim or delete the wrong host.
                    if provider.is_empty() || server_id.is_empty() {
                        return None;
                    }
                    if crate::providers::config::validate_label(label).is_ok() {
                        // Well-formed 3-segment labeled marker.
                        Some((
                            crate::providers::config::ProviderConfigId::labeled(provider, label),
                            server_id.to_string(),
                        ))
                    } else if label.is_empty() {
                        // Empty middle (e.g. `aws::123`) cannot be either a
                        // valid labeled marker or a legacy 2-segment one.
                        // Treat as malformed.
                        None
                    } else {
                        // Middle has content but isn't a valid label
                        // (e.g. `azure:RES:i-12345`): legacy interpretation
                        // with the embedded colon kept in server_id.
                        Some((
                            crate::providers::config::ProviderConfigId::bare(provider),
                            format!("{}:{}", label, server_id),
                        ))
                    }
                }
                _ => None,
            };
        }
        None
    }

    /// Set provider on a host block using a full ProviderConfigId.
    /// Emits 2-segment marker if `id.label` is None, 3-segment if Some.
    pub fn set_provider_id(
        &mut self,
        id: &crate::providers::config::ProviderConfigId,
        server_id: &str,
    ) {
        // Sanitise the server_id before interpolation — a provider API
        // returning `123\n  ProxyJump attacker` would otherwise inject a
        // ProxyJump directive into the user's config.
        let server_id = Self::sanitize_raw_line_value(server_id);
        let indent = self.detect_indent();
        self.directives.retain(|d| {
            !(d.is_non_directive && d.raw_line.trim().starts_with("# purple:provider "))
        });
        let pos = self.content_end();
        self.directives.insert(
            pos,
            Directive {
                key: String::new(),
                value: String::new(),
                raw_line: format!("{}# purple:provider {}:{}", indent, id, server_id),
                is_non_directive: true,
            },
        );
    }

    /// Extract askpass source from purple:askpass comment in directives.
    pub fn askpass(&self) -> Option<String> {
        for d in &self.directives {
            if d.is_non_directive {
                let trimmed = d.raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("# purple:askpass ") {
                    let val = rest.trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
        None
    }

    /// Extract vault-ssh role from purple:vault-ssh comment.
    pub fn vault_ssh(&self) -> Option<String> {
        for d in &self.directives {
            if d.is_non_directive {
                let trimmed = d.raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("# purple:vault-ssh ") {
                    let val = rest.trim();
                    if !val.is_empty() && crate::vault_ssh::is_valid_role(val) {
                        return Some(val.to_string());
                    }
                }
            }
        }
        None
    }

    /// Set vault-ssh role. Replaces existing comment or adds one. Empty string removes.
    pub fn set_vault_ssh(&mut self, role: &str) {
        let role = Self::sanitize_raw_line_value(role);
        let indent = self.detect_indent();
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:vault-ssh" || t.starts_with("# purple:vault-ssh ")
            })
        });
        if !role.is_empty() {
            let pos = self.content_end();
            self.directives.insert(
                pos,
                Directive {
                    key: String::new(),
                    value: String::new(),
                    raw_line: format!("{}# purple:vault-ssh {}", indent, role),
                    is_non_directive: true,
                },
            );
        }
    }

    /// Extract the Vault SSH endpoint from a `# purple:vault-addr` comment.
    /// Returns None when the comment is absent, blank or contains an invalid
    /// URL value. Validation is intentionally minimal: we reject empty,
    /// whitespace-containing and control-character values but otherwise let
    /// the Vault CLI surface its own error on typos.
    pub fn vault_addr(&self) -> Option<String> {
        for d in &self.directives {
            if d.is_non_directive {
                let trimmed = d.raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("# purple:vault-addr ") {
                    let val = rest.trim();
                    if !val.is_empty() && crate::vault_ssh::is_valid_vault_addr(val) {
                        return Some(val.to_string());
                    }
                }
            }
        }
        None
    }

    /// Set vault-addr endpoint. Replaces existing comment or adds one. Empty
    /// string removes. Caller is expected to have validated the URL upstream
    /// (e.g. via `is_valid_vault_addr`) — this function does not re-validate.
    pub fn set_vault_addr(&mut self, url: &str) {
        let url = Self::sanitize_raw_line_value(url);
        let indent = self.detect_indent();
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:vault-addr" || t.starts_with("# purple:vault-addr ")
            })
        });
        if !url.is_empty() {
            let pos = self.content_end();
            self.directives.insert(
                pos,
                Directive {
                    key: String::new(),
                    value: String::new(),
                    raw_line: format!("{}# purple:vault-addr {}", indent, url),
                    is_non_directive: true,
                },
            );
        }
    }

    /// Set askpass source on a host block. Replaces existing purple:askpass comment or adds one.
    /// Pass an empty string to remove the comment.
    pub fn set_askpass(&mut self, source: &str) {
        let source = Self::sanitize_raw_line_value(source);
        let indent = self.detect_indent();
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:askpass" || t.starts_with("# purple:askpass ")
            })
        });
        if !source.is_empty() {
            let pos = self.content_end();
            self.directives.insert(
                pos,
                Directive {
                    key: String::new(),
                    value: String::new(),
                    raw_line: format!("{}# purple:askpass {}", indent, source),
                    is_non_directive: true,
                },
            );
        }
    }

    /// Extract provider metadata from purple:meta comment in directives.
    /// Format: `# purple:meta key=value,key=value`
    pub fn meta(&self) -> Vec<(String, String)> {
        for d in &self.directives {
            if d.is_non_directive {
                let trimmed = d.raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("# purple:meta ") {
                    return rest
                        .split(',')
                        .filter_map(|pair| {
                            let (k, v) = pair.split_once('=')?;
                            let k = k.trim();
                            let v = v.trim();
                            if k.is_empty() {
                                None
                            } else {
                                Some((k.to_string(), v.to_string()))
                            }
                        })
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// Set provider metadata on a host block. Replaces existing purple:meta comment or adds one.
    /// Pass an empty slice to remove the comment.
    pub fn set_meta(&mut self, meta: &[(String, String)]) {
        let indent = self.detect_indent();
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:meta" || t.starts_with("# purple:meta ")
            })
        });
        if !meta.is_empty() {
            let encoded: Vec<String> = meta
                .iter()
                .map(|(k, v)| {
                    let clean_k = Self::sanitize_tag(&k.replace([',', '='], ""));
                    let clean_v = Self::sanitize_tag(&v.replace(',', ""));
                    format!("{}={}", clean_k, clean_v)
                })
                .collect();
            let pos = self.content_end();
            self.directives.insert(
                pos,
                Directive {
                    key: String::new(),
                    value: String::new(),
                    raw_line: format!("{}# purple:meta {}", indent, encoded.join(",")),
                    is_non_directive: true,
                },
            );
        }
    }

    /// Extract stale timestamp from purple:stale comment in directives.
    /// Returns `None` if absent or malformed.
    pub fn stale(&self) -> Option<u64> {
        for d in &self.directives {
            if d.is_non_directive {
                let trimmed = d.raw_line.trim();
                if let Some(rest) = trimmed.strip_prefix("# purple:stale ") {
                    return rest.trim().parse::<u64>().ok();
                }
            }
        }
        None
    }

    /// Mark a host block as stale with a unix timestamp.
    /// Replaces existing purple:stale comment or adds one.
    pub fn set_stale(&mut self, timestamp: u64) {
        let indent = self.detect_indent();
        self.clear_stale();
        let pos = self.content_end();
        self.directives.insert(
            pos,
            Directive {
                key: String::new(),
                value: String::new(),
                raw_line: format!("{}# purple:stale {}", indent, timestamp),
                is_non_directive: true,
            },
        );
    }

    /// Remove stale marking from a host block.
    pub fn clear_stale(&mut self) {
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:stale" || t.starts_with("# purple:stale ")
            })
        });
    }

    /// Sanitize a tag value: strip control characters, commas (delimiter),
    /// and Unicode format/bidi override characters. Truncate to 128 chars.
    pub(super) fn sanitize_tag(tag: &str) -> String {
        tag.chars()
            .filter(|c| {
                !c.is_control()
                    && *c != ','
                    && !('\u{200B}'..='\u{200F}').contains(c) // zero-width, bidi marks
                    && !('\u{202A}'..='\u{202E}').contains(c) // bidi embedding/override
                    && !('\u{2066}'..='\u{2069}').contains(c) // bidi isolate
                    && *c != '\u{FEFF}' // BOM/zero-width no-break space
            })
            .take(128)
            .collect()
    }

    /// Strip line-breaking characters from any value that gets interpolated
    /// into a `raw_line`. A `\n`, `\r` or `\0` in a provider-supplied
    /// `server_id`, a user-typed askpass URI, or a Vault role would otherwise
    /// split one line into multiple SSH config directives (directive
    /// injection). All setters that format user-controlled bytes into
    /// `raw_line` must route the value through this helper first.
    ///
    /// Returns the input unchanged when no offending byte is present so the
    /// common case incurs no allocation. Logs a warning when a substitution
    /// happens. The substitution is silent for the user-facing flow but
    /// surfaces in the log file for forensics.
    pub(super) fn sanitize_raw_line_value(s: &str) -> std::borrow::Cow<'_, str> {
        if !s.contains(['\n', '\r', '\0']) {
            return std::borrow::Cow::Borrowed(s);
        }
        log::warn!(
            "[purple] sanitized line-breaking characters from value before writing to ssh_config"
        );
        std::borrow::Cow::Owned(s.replace(['\n', '\r', '\0'], " "))
    }

    /// Render a single-argument directive value for interpolation into a
    /// `raw_line`. OpenSSH carries an argument containing spaces by wrapping
    /// it in double quotes (ssh_config(5): "Arguments may optionally be
    /// enclosed in double quotes"). Without quoting, `~/my key/id` writes as
    /// three tokens and `~/id #note` loses its tail to inline-comment
    /// stripping on the next parse. A value that itself contains a `"` is also
    /// quoted, with embedded `\` and `"` backslash-escaped, so a single-token
    /// value containing both whitespace and a quote round-trips faithfully
    /// instead of being emitted unquoted and split by OpenSSH.
    /// `parser::strip_surrounding_quotes` is the inverse. Only for single-token
    /// directives (HostName, User, IdentityFile, ProxyJump, CertificateFile);
    /// multi-arg directives like LocalForward must not be routed through here.
    pub(super) fn render_value(value: &str) -> std::borrow::Cow<'_, str> {
        if value.chars().any(char::is_whitespace) || value.contains('"') {
            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            std::borrow::Cow::Owned(format!("\"{escaped}\""))
        } else {
            std::borrow::Cow::Borrowed(value)
        }
    }

    /// Set user tags on a host block. Replaces existing purple:tags comment or adds one.
    pub fn set_tags(&mut self, tags: &[String]) {
        let indent = self.detect_indent();
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:tags" || t.starts_with("# purple:tags ")
            })
        });
        let sanitized: Vec<String> = tags
            .iter()
            .map(|t| Self::sanitize_tag(t))
            .filter(|t| !t.is_empty())
            .collect();
        if !sanitized.is_empty() {
            let pos = self.content_end();
            self.directives.insert(
                pos,
                Directive {
                    key: String::new(),
                    value: String::new(),
                    raw_line: format!("{}# purple:tags {}", indent, sanitized.join(",")),
                    is_non_directive: true,
                },
            );
        }
    }

    /// Set provider-synced tags. Replaces existing purple:provider_tags comment.
    /// Always writes the comment (even when empty) as a migration sentinel.
    pub fn set_provider_tags(&mut self, tags: &[String]) {
        let indent = self.detect_indent();
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == "# purple:provider_tags" || t.starts_with("# purple:provider_tags ")
            })
        });
        let sanitized: Vec<String> = tags
            .iter()
            .map(|t| Self::sanitize_tag(t))
            .filter(|t| !t.is_empty())
            .collect();
        let raw = if sanitized.is_empty() {
            format!("{}# purple:provider_tags", indent)
        } else {
            format!("{}# purple:provider_tags {}", indent, sanitized.join(","))
        };
        let pos = self.content_end();
        self.directives.insert(
            pos,
            Directive {
                key: String::new(),
                value: String::new(),
                raw_line: raw,
                is_non_directive: true,
            },
        );
    }

    /// Set the last-synced provider username marker. Empty value removes it.
    pub fn set_provider_user(&mut self, user: &str) {
        self.set_single_value_marker("# purple:provider_user", user);
    }

    /// Set the last-synced provider identity file marker. Empty value removes it.
    pub fn set_provider_key(&mut self, key: &str) {
        self.set_single_value_marker("# purple:provider_key", key);
    }

    /// Replace (or, when `value` is empty, remove) a single-value
    /// `# purple:<name> <value>` comment. Routes the value through
    /// `sanitize_raw_line_value` so a stray newline cannot inject directives.
    fn set_single_value_marker(&mut self, prefix: &str, value: &str) {
        let indent = self.detect_indent();
        let with_space = format!("{} ", prefix);
        self.directives.retain(|d| {
            !(d.is_non_directive && {
                let t = d.raw_line.trim();
                t == prefix || t.starts_with(with_space.as_str())
            })
        });
        let value = Self::sanitize_raw_line_value(value.trim());
        if !value.is_empty() {
            let pos = self.content_end();
            self.directives.insert(
                pos,
                Directive {
                    key: String::new(),
                    value: String::new(),
                    raw_line: format!("{}{} {}", indent, prefix, value),
                    is_non_directive: true,
                },
            );
        }
    }

    /// Extract a convenience HostEntry view from this block.
    ///
    /// Matches OpenSSH `ssh_config(5)`: "Unless noted otherwise, for each
    /// parameter, the first obtained value will be used." Duplicate
    /// HostName/User/Port/ProxyJump entries keep the FIRST value seen.
    pub fn to_host_entry(&self) -> HostEntry {
        let mut entry = HostEntry {
            alias: self.host_pattern.clone(),
            port: 22,
            ..Default::default()
        };
        let mut port_seen = false;
        for d in &self.directives {
            if d.is_non_directive {
                continue;
            }
            if d.key.eq_ignore_ascii_case("hostname") {
                if entry.hostname.is_empty() {
                    entry.hostname = d.value.clone();
                }
            } else if d.key.eq_ignore_ascii_case("user") {
                if entry.user.is_empty() {
                    entry.user = d.value.clone();
                }
            } else if d.key.eq_ignore_ascii_case("port") {
                if !port_seen {
                    entry.port = d.value.parse().unwrap_or(22);
                    port_seen = true;
                }
            } else if d.key.eq_ignore_ascii_case("identityfile") {
                if entry.identity_file.is_empty() {
                    entry.identity_file = d.value.clone();
                }
            } else if d.key.eq_ignore_ascii_case("proxyjump") {
                if entry.proxy_jump.is_empty() {
                    entry.proxy_jump = d.value.clone();
                }
            } else if d.key.eq_ignore_ascii_case("certificatefile")
                && entry.certificate_file.is_empty()
            {
                entry.certificate_file = d.value.clone();
            }
        }
        entry.tags = self.tags();
        entry.provider_tags = self.provider_tags();
        entry.has_provider_tags = self.has_provider_tags_comment();
        if let Some((id, _)) = self.provider_id() {
            entry.provider = Some(id.provider);
            entry.provider_label = id.label;
        }
        entry.tunnel_count = self.tunnel_count();
        entry.askpass = self.askpass();
        entry.vault_ssh = self.vault_ssh();
        entry.vault_addr = self.vault_addr();
        entry.provider_meta = self.meta();
        entry.stale = self.stale();
        entry.provider_user = self.provider_user();
        entry.provider_key = self.provider_key();
        entry
    }

    /// Extract a convenience PatternEntry view from this block.
    pub fn to_pattern_entry(&self) -> PatternEntry {
        let mut entry = PatternEntry {
            pattern: self.host_pattern.clone(),
            hostname: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            proxy_jump: String::new(),
            tags: self.tags(),
            askpass: self.askpass(),
            source_file: None,
            directives: Vec::new(),
        };
        let mut port_seen = false;
        for d in &self.directives {
            if d.is_non_directive {
                continue;
            }
            match d.key.to_ascii_lowercase().as_str() {
                "hostname" if entry.hostname.is_empty() => entry.hostname = d.value.clone(),
                "user" if entry.user.is_empty() => entry.user = d.value.clone(),
                "port" if !port_seen => {
                    entry.port = d.value.parse().unwrap_or(22);
                    port_seen = true;
                }
                "identityfile" if entry.identity_file.is_empty() => {
                    entry.identity_file = d.value.clone();
                }
                "proxyjump" if entry.proxy_jump.is_empty() => entry.proxy_jump = d.value.clone(),
                _ => {}
            }
            entry.directives.push((d.key.clone(), d.value.clone()));
        }
        entry
    }

    /// Count forwarding directives (LocalForward, RemoteForward, DynamicForward).
    pub fn tunnel_count(&self) -> u16 {
        let count = self
            .directives
            .iter()
            .filter(|d| {
                !d.is_non_directive
                    && (d.key.eq_ignore_ascii_case("localforward")
                        || d.key.eq_ignore_ascii_case("remoteforward")
                        || d.key.eq_ignore_ascii_case("dynamicforward"))
            })
            .count();
        count.min(u16::MAX as usize) as u16
    }

    /// Check if this block has any tunnel forwarding directives.
    #[allow(dead_code)]
    pub fn has_tunnels(&self) -> bool {
        self.directives.iter().any(|d| {
            !d.is_non_directive
                && (d.key.eq_ignore_ascii_case("localforward")
                    || d.key.eq_ignore_ascii_case("remoteforward")
                    || d.key.eq_ignore_ascii_case("dynamicforward"))
        })
    }

    /// Extract tunnel rules from forwarding directives.
    pub fn tunnel_directives(&self) -> Vec<crate::tunnel::TunnelRule> {
        self.directives
            .iter()
            .filter(|d| !d.is_non_directive)
            .filter_map(|d| crate::tunnel::TunnelRule::parse_value(&d.key, &d.value))
            .collect()
    }
}
