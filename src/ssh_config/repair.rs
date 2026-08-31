//! Repair utilities for `# purple:group` comments in an SSH config file.
//!
//! Two problems this module fixes:
//!
//! 1. Provider group headers (`# purple:group <DisplayName>`) that end up
//!    absorbed as non-directive lines inside a preceding Host block.
//!    `repair_absorbed_group_comments` promotes them back to top-level
//!    `GlobalLine` elements so they render correctly.
//! 2. Orphaned group headers for providers that no longer have any hosts
//!    configured. `remove_all_orphaned_group_headers` and
//!    `remove_orphaned_group_header` drop those stale lines.

use super::model::{ConfigElement, SshConfigFile};

/// Display name for a provider used in `# purple:group` headers. Delegates to
/// the provider registry so the cleanup matcher and the sync writer can never
/// disagree. A previous hand-maintained copy here drifted: ovh, i3d, leaseweb
/// and transip were missing, so their headers were deleted on startup despite
/// active hosts.
pub(super) fn provider_group_display_name(name: &str) -> &str {
    crate::providers::provider_display_name(name)
}

impl SshConfigFile {
    /// Remove all `# purple:group <DisplayName>` GlobalLines that point at a
    /// provider with no remaining Host blocks. Returns the count removed.
    pub fn remove_all_orphaned_group_headers(&mut self) -> usize {
        let active_providers: std::collections::HashSet<String> = self
            .elements
            .iter()
            .filter_map(|e| {
                if let ConfigElement::HostBlock(block) = e {
                    block
                        .provider()
                        .map(|(name, _)| provider_group_display_name(&name).to_string())
                } else {
                    None
                }
            })
            .collect();

        let mut removed = 0;
        self.elements.retain(|e| {
            if let ConfigElement::GlobalLine(line) = e
                && let Some(rest) = line.trim().strip_prefix("# purple:group ")
                && !active_providers.contains(rest.trim())
            {
                removed += 1;
                return false;
            }
            true
        });
        removed
    }

    /// Repair configs where `# purple:group` comments were absorbed into the
    /// preceding host block's directives instead of being stored as
    /// GlobalLines. Returns the number of blocks that were repaired.
    ///
    /// Only relocates lines whose suffix matches a known provider's display
    /// name. A user-authored comment like `# purple:group canary notes` is
    /// left in place: the suffix `canary notes` is not a provider name, so
    /// the comment is treated as free-form prose rather than a misparsed
    /// group header. This protects hand-written comments from being silently
    /// scrubbed by the next-startup `remove_all_orphaned_group_headers` pass.
    pub fn repair_absorbed_group_comments(&mut self) -> usize {
        // Derive known provider display names from the registry so this matcher
        // can never drift from the sync writer. A hand-maintained copy here
        // previously omitted ovh, leaseweb, i3d and transip, so their absorbed
        // group comments were never relocated to top-level GlobalLines.
        let known_providers: std::collections::HashSet<&str> = crate::providers::PROVIDER_NAMES
            .iter()
            .map(|n| crate::providers::provider_display_name(n))
            .collect();

        let is_known_group = |raw: &str| -> bool {
            raw.trim()
                .strip_prefix("# purple:group ")
                .map(|suffix| known_providers.contains(suffix.trim()))
                .unwrap_or(false)
        };

        let mut repaired = 0;
        let mut idx = 0;
        while idx < self.elements.len() {
            let needs_repair = if let ConfigElement::HostBlock(block) = &self.elements[idx] {
                block
                    .directives
                    .iter()
                    .any(|d| d.is_non_directive && is_known_group(&d.raw_line))
            } else {
                false
            };

            if !needs_repair {
                idx += 1;
                continue;
            }

            let block = if let ConfigElement::HostBlock(block) = &mut self.elements[idx] {
                block
            } else {
                unreachable!()
            };

            let group_idx = block
                .directives
                .iter()
                .position(|d| d.is_non_directive && is_known_group(&d.raw_line))
                .unwrap();

            let mut keep_end = group_idx;
            while keep_end > 0
                && block.directives[keep_end - 1].is_non_directive
                && block.directives[keep_end - 1].raw_line.trim().is_empty()
            {
                keep_end -= 1;
            }

            let extracted: Vec<ConfigElement> = block
                .directives
                .drain(keep_end..)
                .map(|d| ConfigElement::GlobalLine(d.raw_line))
                .collect();

            let insert_at = idx + 1;
            for (i, elem) in extracted.into_iter().enumerate() {
                self.elements.insert(insert_at + i, elem);
            }

            repaired += 1;
            idx = insert_at;
            while idx < self.elements.len() {
                if let ConfigElement::HostBlock(_) = &self.elements[idx] {
                    break;
                }
                idx += 1;
            }
        }
        repaired
    }

    /// Remove the `# purple:group <DisplayName>` GlobalLine for a single
    /// provider if no remaining HostBlock has that provider.
    pub(super) fn remove_orphaned_group_header(&mut self, provider_name: &str) {
        if self.find_hosts_by_provider(provider_name).is_empty() {
            let display = provider_group_display_name(provider_name);
            let header = format!("# purple:group {}", display);
            self.elements
                .retain(|e| !matches!(e, ConfigElement::GlobalLine(line) if *line == header));
        }
    }
}
