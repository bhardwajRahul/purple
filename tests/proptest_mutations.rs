//! Property-based tests for combined mutation sequences.
//!
//! Simulates realistic purple usage: add hosts, set tags/provider/meta/askpass,
//! add tunnels, update, delete, undo, sync, import -- all interleaved -- and
//! verifies that the config never corrupts, panics, or loses structural integrity.

use std::path::PathBuf;

use proptest::prelude::*;
use purple_ssh::providers::config::ProviderConfigId;
use purple_ssh::ssh_config::model::{HostEntry, SshConfigFile};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_str(content: &str) -> SshConfigFile {
    SshConfigFile {
        elements: SshConfigFile::parse_content(content),
        path: PathBuf::from("/tmp/proptest_mutations"),
        crlf: purple_ssh::ssh_config::parser::detect_crlf_majority(content),
        bom: content.starts_with('\u{FEFF}'),
    }
}

/// Assert round-trip idempotency: serialize -> reparse -> serialize must match.
fn assert_idempotent(config: &SshConfigFile) {
    let s1 = config.serialize();
    let reparsed = parse_str(&s1);
    let s2 = reparsed.serialize();
    assert_eq!(
        s1,
        s2,
        "Round-trip idempotency broken (len {} vs {})",
        s1.len(),
        s2.len()
    );
}

/// Assert no host block leaks directives into another block.
fn assert_no_directive_leak(config: &SshConfigFile) {
    let serialized = config.serialize();
    let reparsed = parse_str(&serialized);
    let entries_before = config.host_entries();
    let entries_after = reparsed.host_entries();
    assert_eq!(
        entries_before.len(),
        entries_after.len(),
        "Host count changed after reparse ({} -> {})",
        entries_before.len(),
        entries_after.len(),
    );
    for (a, b) in entries_before.iter().zip(entries_after.iter()) {
        assert_eq!(a.alias, b.alias, "Alias mismatch after reparse");
        assert_eq!(
            a.hostname, b.hostname,
            "Hostname mismatch for '{}'",
            a.alias
        );
        assert_eq!(a.user, b.user, "User mismatch for '{}'", a.alias);
        assert_eq!(a.port, b.port, "Port mismatch for '{}'", a.alias);
        assert_eq!(a.tags, b.tags, "Tags mismatch for '{}'", a.alias);
        assert_eq!(
            a.provider, b.provider,
            "Provider mismatch for '{}'",
            a.alias
        );
        assert_eq!(a.askpass, b.askpass, "Askpass mismatch for '{}'", a.alias);
        assert_eq!(
            a.tunnel_count, b.tunnel_count,
            "Tunnel count mismatch for '{}'",
            a.alias
        );
        assert_eq!(a.stale, b.stale, "Stale mismatch for '{}'", a.alias);
    }
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn alias_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{1,15}"
}

fn hostname_strategy() -> impl Strategy<Value = String> {
    (1u8..=254, 0u8..=255, 0u8..=255, 1u8..=254).prop_map(|(a, b, c, d)| format!("{a}.{b}.{c}.{d}"))
}

fn user_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("root".to_string()),
        Just("admin".to_string()),
        Just("deploy".to_string()),
        "[a-z]{3,8}",
    ]
}

fn tags_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z]{2,8}", 0..=5)
}

fn meta_strategy() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec(("[a-z]{3,8}", "[a-z0-9-]{2,12}"), 0..=4)
        .prop_map(|v| v.into_iter().collect())
}

fn askpass_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("keychain".to_string()),
        Just("op://Servers/prod/password".to_string()),
        Just("bw:my-server-password".to_string()),
        Just("pass:servers/prod".to_string()),
        Just("vault:secret/ssh/prod".to_string()),
        Just(String::new()), // clear askpass
    ]
}

fn forward_strategy() -> impl Strategy<Value = (String, String)> {
    prop_oneof![
        (1024u16..=65535, 1u16..=65535).prop_map(|(local, remote)| {
            (
                "LocalForward".to_string(),
                format!("{local} localhost:{remote}"),
            )
        }),
        (1024u16..=65535, 1u16..=65535).prop_map(|(local, remote)| {
            (
                "RemoteForward".to_string(),
                format!("{local} localhost:{remote}"),
            )
        }),
        (1024u16..=65535,)
            .prop_map(|(port,)| { ("DynamicForward".to_string(), format!("{port}")) }),
    ]
}

fn host_entry(alias: &str, hostname: &str, user: &str) -> HostEntry {
    HostEntry {
        alias: alias.to_string(),
        hostname: hostname.to_string(),
        user: user.to_string(),
        port: 22,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// An action that can be applied to a config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Action {
    AddHost {
        alias: String,
        hostname: String,
        user: String,
    },
    UpdateHost {
        hostname: String,
        user: String,
    },
    DeleteFirst,
    DeleteUndoFirst,
    SwapFirstTwo,
    SetTags {
        tags: Vec<String>,
    },
    SetAskpass {
        source: String,
    },
    SetProvider {
        name: String,
        id: String,
    },
    SetProviderLabeled {
        name: String,
        label: String,
        id: String,
    },
    RewriteLegacyToLabel {
        name: String,
        label: String,
    },
    SetMeta {
        meta: Vec<(String, String)>,
    },
    AddForward {
        key: String,
        value: String,
    },
    RemoveForward {
        key: String,
        value: String,
    },
    SetStale {
        timestamp: u64,
    },
    ClearStale,
    RepairGroupComments,
    RemoveOrphanedHeaders,
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        // AddHost
        (alias_strategy(), hostname_strategy(), user_strategy()).prop_map(|(a, h, u)| {
            Action::AddHost {
                alias: a,
                hostname: h,
                user: u,
            }
        }),
        // UpdateHost
        (hostname_strategy(), user_strategy()).prop_map(|(h, u)| {
            Action::UpdateHost {
                hostname: h,
                user: u,
            }
        }),
        // DeleteFirst
        Just(Action::DeleteFirst),
        // DeleteUndoFirst (delete + immediate undo)
        Just(Action::DeleteUndoFirst),
        // SwapFirstTwo
        Just(Action::SwapFirstTwo),
        // SetTags
        tags_strategy().prop_map(|tags| Action::SetTags { tags }),
        // SetAskpass
        askpass_strategy().prop_map(|source| Action::SetAskpass { source }),
        // SetProvider (legacy 2-segment marker)
        (
            prop_oneof![
                Just("aws"),
                Just("digitalocean"),
                Just("hetzner"),
                Just("gcp")
            ],
            "[a-z0-9]{8,16}",
        )
            .prop_map(|(name, id)| Action::SetProvider {
                name: name.to_string(),
                id,
            }),
        // SetProviderLabeled (3-segment marker for multi-config)
        (
            prop_oneof![Just("aws"), Just("digitalocean"), Just("hetzner")],
            prop_oneof![
                Just("work"),
                Just("personal"),
                Just("default"),
                Just("prod-1"),
            ],
            "[a-z0-9]{6,12}",
        )
            .prop_map(|(name, label, id)| Action::SetProviderLabeled {
                name: name.to_string(),
                label: label.to_string(),
                id,
            }),
        // RewriteLegacyToLabel exercises the lazy-migration helper.
        (
            prop_oneof![Just("aws"), Just("digitalocean"), Just("hetzner")],
            prop_oneof![Just("default"), Just("work"), Just("primary")],
        )
            .prop_map(|(name, label)| Action::RewriteLegacyToLabel {
                name: name.to_string(),
                label: label.to_string(),
            }),
        // SetMeta
        meta_strategy().prop_map(|meta| Action::SetMeta { meta }),
        // AddForward
        forward_strategy().prop_map(|(key, value)| Action::AddForward { key, value }),
        // RemoveForward
        forward_strategy().prop_map(|(key, value)| Action::RemoveForward { key, value }),
        // SetStale
        (1_600_000_000u64..=1_900_000_000).prop_map(|ts| Action::SetStale { timestamp: ts }),
        // ClearStale
        Just(Action::ClearStale),
        // Repair
        Just(Action::RepairGroupComments),
        Just(Action::RemoveOrphanedHeaders),
    ]
}

/// Apply an action to a config. Returns an optional undo element for later undo.
fn apply_action(config: &mut SshConfigFile, action: &Action) {
    let entries = config.host_entries();
    let first_alias = entries.first().map(|e| e.alias.clone());

    match action {
        Action::AddHost {
            alias,
            hostname,
            user,
        } => {
            if !config.has_host(alias) {
                config.add_host(&host_entry(alias, hostname, user));
            }
        }
        Action::UpdateHost { hostname, user } => {
            if let Some(alias) = &first_alias {
                config.update_host(
                    alias,
                    &HostEntry {
                        alias: alias.clone(),
                        hostname: hostname.clone(),
                        user: user.clone(),
                        port: 22,
                        ..Default::default()
                    },
                );
            }
        }
        Action::DeleteFirst => {
            if let Some(alias) = &first_alias {
                config.delete_host(alias);
            }
        }
        Action::DeleteUndoFirst => {
            if let Some(alias) = &first_alias
                && let Some((element, position)) = config.delete_host_undoable(alias)
            {
                config.insert_host_at(element, position);
            }
        }
        Action::SwapFirstTwo => {
            if entries.len() >= 2 {
                let a = entries[0].alias.clone();
                let b = entries[1].alias.clone();
                config.swap_hosts(&a, &b);
            }
        }
        Action::SetTags { tags } => {
            if let Some(alias) = &first_alias {
                let _ = config.set_host_tags(alias, tags);
            }
        }
        Action::SetAskpass { source } => {
            if let Some(alias) = &first_alias {
                let _ = config.set_host_askpass(alias, source);
            }
        }
        Action::SetProvider { name, id } => {
            if let Some(alias) = &first_alias {
                let _ = config.set_host_provider_id(alias, &ProviderConfigId::bare(name), id);
            }
        }
        Action::SetProviderLabeled { name, label, id } => {
            if let Some(alias) = &first_alias {
                let _ =
                    config.set_host_provider_id(alias, &ProviderConfigId::labeled(name, label), id);
            }
        }
        Action::RewriteLegacyToLabel { name, label } => {
            // Lazy-migration helper. Must round-trip cleanly.
            let _ = config.rewrite_legacy_markers_to_label(name, label);
        }
        Action::SetMeta { meta } => {
            if let Some(alias) = &first_alias {
                let _ = config.set_host_meta(alias, meta);
            }
        }
        Action::AddForward { key, value } => {
            if let Some(alias) = &first_alias {
                config.add_forward(alias, key, value);
            }
        }
        Action::RemoveForward { key, value } => {
            if let Some(alias) = &first_alias {
                config.remove_forward(alias, key, value);
            }
        }
        Action::SetStale { timestamp } => {
            if let Some(alias) = &first_alias {
                let _ = config.set_host_stale(alias, *timestamp);
            }
        }
        Action::ClearStale => {
            if let Some(alias) = &first_alias {
                let _ = config.clear_host_stale(alias);
            }
        }
        Action::RepairGroupComments => {
            config.repair_absorbed_group_comments();
        }
        Action::RemoveOrphanedHeaders => {
            config.remove_all_orphaned_group_headers();
        }
    }
}

// ---------------------------------------------------------------------------
// Property: random action sequences never corrupt the config
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Apply 5-30 random actions to a fresh config. After each action,
    /// verify round-trip idempotency and no directive leaks.
    #[test]
    fn random_action_sequence(
        actions in prop::collection::vec(action_strategy(), 5..=30),
    ) {
        let mut config = parse_str("");

        for action in &actions {
            apply_action(&mut config, action);
            assert_idempotent(&config);
        }

        // Final deep check
        assert_no_directive_leak(&config);
    }

    /// Start with a populated config and apply random actions.
    #[test]
    fn random_actions_on_populated_config(
        actions in prop::collection::vec(action_strategy(), 5..=20),
    ) {
        let initial = "\
Host alpha
  HostName 10.0.0.1
  User admin
  IdentityFile ~/.ssh/id_ed25519
  # purple:tags prod,us-east
  # purple:provider aws:i-abc123
  # purple:meta region=us-east-1,status=running
  # purple:askpass keychain
  LocalForward 8080 localhost:80

Host beta
  HostName 10.0.0.2
  User deploy
  DynamicForward 1080
  # purple:tags staging

Host gamma
  HostName 10.0.0.3
  User root
  RemoteForward 9090 localhost:9090
  # purple:provider hetzner:srv-456
  # purple:stale 1700000000
  # purple:askpass op://Servers/gamma/password

Host *
  ServerAliveInterval 60
  ServerAliveCountMax 3
";
        let mut config = parse_str(initial);

        for action in &actions {
            apply_action(&mut config, action);
            assert_idempotent(&config);
        }

        assert_no_directive_leak(&config);
    }
}

// ---------------------------------------------------------------------------
// Property: full lifecycle (add -> annotate -> update -> tunnel -> delete -> undo)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn full_host_lifecycle(
        alias in alias_strategy(),
        hostname in hostname_strategy(),
        user in user_strategy(),
        tags in tags_strategy(),
        askpass in askpass_strategy(),
        meta in meta_strategy(),
        forward in forward_strategy(),
        new_hostname in hostname_strategy(),
        new_user in user_strategy(),
    ) {
        let mut config = parse_str("Host existing\n  HostName 1.2.3.4\n  User old\n");

        if config.has_host(&alias) {
            return Ok(());
        }

        // 1. Add host
        config.add_host(&host_entry(&alias, &hostname, &user));
        assert_idempotent(&config);
        prop_assert!(config.has_host(&alias));

        // 2. Set tags
        let _ = config.set_host_tags(&alias, &tags);
        assert_idempotent(&config);
        let entry = config.host_entries().into_iter().find(|e| e.alias == alias).unwrap();
        prop_assert_eq!(&entry.tags, &tags);

        // 3. Set provider
        let _ = config.set_host_provider_id(&alias, &ProviderConfigId::bare( "aws"),  "i-test123");
        assert_idempotent(&config);
        let entry = config.host_entries().into_iter().find(|e| e.alias == alias).unwrap();
        prop_assert_eq!(entry.provider.as_deref(), Some("aws"));

        // 4. Set meta
        let _ = config.set_host_meta(&alias, &meta);
        assert_idempotent(&config);

        // 4b. Set stale, verify, clear, verify
        let _ = config.set_host_stale(&alias, 1700000000);
        assert_idempotent(&config);
        let entry = config.host_entries().into_iter().find(|e| e.alias == alias).unwrap();
        prop_assert!(entry.stale.is_some(), "stale should be Some after set");

        let _ = config.clear_host_stale(&alias);
        assert_idempotent(&config);
        let entry = config.host_entries().into_iter().find(|e| e.alias == alias).unwrap();
        prop_assert!(entry.stale.is_none(), "stale should be None after clear");

        // 5. Set askpass
        let _ = config.set_host_askpass(&alias, &askpass);
        assert_idempotent(&config);

        // 6. Add forward
        config.add_forward(&alias, &forward.0, &forward.1);
        assert_idempotent(&config);

        // 7. Update host (should preserve all annotations)
        config.update_host(
            &alias,
            &HostEntry {
                alias: alias.clone(),
                hostname: new_hostname.clone(),
                user: new_user.clone(),
                port: 22,
                ..Default::default()
            },
        );
        assert_idempotent(&config);

        // Verify annotations survived update
        let entry = config.host_entries().into_iter().find(|e| e.alias == alias).unwrap();
        prop_assert_eq!(&entry.hostname, &new_hostname);
        prop_assert_eq!(&entry.user, &new_user);
        prop_assert_eq!(&entry.tags, &tags);
        prop_assert_eq!(entry.provider.as_deref(), Some("aws"));
        prop_assert!(entry.tunnel_count >= 1);

        // 8. Remove forward
        config.remove_forward(&alias, &forward.0, &forward.1);
        assert_idempotent(&config);

        // 9. Delete undoable
        let count_before = config.host_entries().len();
        let undo = config.delete_host_undoable(&alias);
        prop_assert!(undo.is_some());
        prop_assert_eq!(config.host_entries().len(), count_before - 1);

        // 10. Undo
        let (element, position) = undo.unwrap();
        config.insert_host_at(element, position);
        assert_idempotent(&config);
        prop_assert!(config.has_host(&alias));

        // 11. Final delete
        config.delete_host(&alias);
        assert_idempotent(&config);
        prop_assert!(!config.has_host(&alias));

        // Original host should still be there
        prop_assert!(config.has_host("existing"));

        // Full structural check
        assert_no_directive_leak(&config);
    }
}

// ---------------------------------------------------------------------------
// Property: multiple provider syncs don't corrupt each other
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Simulate two providers syncing hosts into the same config.
    #[test]
    fn dual_provider_sync_no_corruption(
        aws_hosts in prop::collection::vec(
            (alias_strategy(), hostname_strategy()),
            1..=8,
        ),
        do_hosts in prop::collection::vec(
            (alias_strategy(), hostname_strategy()),
            1..=8,
        ),
    ) {
        let mut config = parse_str("");

        // Simulate AWS sync: add hosts with provider markers
        for (name, ip) in &aws_hosts {
            let alias = format!("aws-{name}");
            if config.has_host(&alias) {
                continue;
            }
            config.add_host(&host_entry(&alias, ip, "ec2-user"));
            let _ = config.set_host_provider_id(&alias, &ProviderConfigId::bare( "aws"),  &format!("i-{name}"));
            let _ = config.set_host_tags(&alias, &["aws".to_string(), "cloud".to_string()]);
            let _ = config.set_host_meta(
                &alias,
                &[
                    ("region".to_string(), "us-east-1".to_string()),
                    ("status".to_string(), "running".to_string()),
                ],
            );
        }

        assert_idempotent(&config);

        // Simulate DO sync: add hosts with provider markers
        for (name, ip) in &do_hosts {
            let alias = format!("do-{name}");
            if config.has_host(&alias) {
                continue;
            }
            config.add_host(&host_entry(&alias, ip, "root"));
            let _ = config.set_host_provider_id(&alias, &ProviderConfigId::bare( "digitalocean"),  &format!("droplet-{name}"));
            let _ = config.set_host_tags(&alias, &["do".to_string()]);
        }

        assert_idempotent(&config);

        // Verify both providers coexist
        let entries = config.host_entries();
        let _aws_count = entries.iter().filter(|e| e.provider.as_deref() == Some("aws")).count();
        let _do_count = entries
            .iter()
            .filter(|e| e.provider.as_deref() == Some("digitalocean"))
            .count();

        // Each provider's hosts should be independently correct
        for entry in &entries {
            if entry.provider.as_deref() == Some("aws") {
                prop_assert_eq!(&entry.user, "ec2-user");
                prop_assert!(entry.tags.contains(&"aws".to_string()));
            }
            if entry.provider.as_deref() == Some("digitalocean") {
                prop_assert_eq!(&entry.user, "root");
                prop_assert!(entry.tags.contains(&"do".to_string()));
            }
        }

        // Now update all AWS hosts (simulate IP change on re-sync)
        for entry in entries.iter().filter(|e| e.provider.as_deref() == Some("aws")) {
            config.update_host(
                &entry.alias,
                &HostEntry {
                    alias: entry.alias.clone(),
                    hostname: "10.99.99.99".to_string(),
                    user: "ec2-user".to_string(),
                    port: 22,
                    ..Default::default()
                },
            );
        }

        assert_idempotent(&config);

        // DO hosts should be untouched
        let entries = config.host_entries();
        for entry in entries.iter().filter(|e| e.provider.as_deref() == Some("digitalocean")) {
            prop_assert_ne!(&entry.hostname, "10.99.99.99");
            prop_assert!(entry.tags.contains(&"do".to_string()));
        }

        // Deep check
        assert_no_directive_leak(&config);
    }
}

// ---------------------------------------------------------------------------
// Property: interleaved add/delete cycles don't accumulate blank lines
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn no_blank_line_accumulation(
        cycles in 5u32..=50,
    ) {
        let mut config = parse_str("Host permanent\n  HostName 1.1.1.1\n  User root\n");

        for i in 0..cycles {
            let alias = format!("temp-{i}");
            config.add_host(&host_entry(&alias, "2.2.2.2", "test"));
            let _ = config.set_host_tags(&alias, &["temp".to_string()]);
            config.delete_host(&alias);
        }

        let serialized = config.serialize();

        // No triple blank lines should ever appear
        prop_assert!(
            !serialized.contains("\n\n\n"),
            "Triple blank line found after {} add/delete cycles:\n{}",
            cycles,
            &serialized[..serialized.len().min(500)],
        );

        assert_idempotent(&config);
        prop_assert!(config.has_host("permanent"));
    }
}

// ---------------------------------------------------------------------------
// Property: rapid tag/meta/askpass changes don't corrupt other annotations
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn rapid_annotation_changes(
        tag_sets in prop::collection::vec(tags_strategy(), 3..=10),
        askpass_values in prop::collection::vec(askpass_strategy(), 3..=10),
        meta_sets in prop::collection::vec(meta_strategy(), 3..=10),
    ) {
        let mut config = parse_str("\
Host target
  HostName 10.0.0.1
  User admin
  IdentityFile ~/.ssh/key
  LocalForward 8080 localhost:80
  # purple:tags initial
  # purple:provider aws:i-initial
  # purple:meta region=us-east-1
  # purple:askpass keychain
");

        let max_rounds = tag_sets.len().min(askpass_values.len()).min(meta_sets.len());

        for i in 0..max_rounds {
            let _ = config.set_host_tags("target", &tag_sets[i]);
            let _ = config.set_host_askpass("target", &askpass_values[i]);
            let _ = config.set_host_meta("target", &meta_sets[i]);
            let _ = config.set_host_stale("target", i as u64);
            assert_idempotent(&config);
        }

        // Clear stale after the loop
        let _ = config.clear_host_stale("target");
        assert_idempotent(&config);

        // Verify final state
        let entries = config.host_entries();
        let entry = entries.iter().find(|e| e.alias == "target").unwrap();
        prop_assert_eq!(&entry.hostname, "10.0.0.1");
        prop_assert_eq!(&entry.user, "admin");
        prop_assert_eq!(entry.tunnel_count, 1);
        prop_assert_eq!(entry.provider.as_deref(), Some("aws"));

        // Tags should match last set
        let last_tags = &tag_sets[max_rounds - 1];
        prop_assert_eq!(&entry.tags, last_tags);

        assert_no_directive_leak(&config);
    }
}

// ---------------------------------------------------------------------------
// Property: concurrent-style undo stack stress
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn undo_stack_stress(
        host_count in 5u32..=20,
        delete_indices in prop::collection::vec(0usize..20, 3..=10),
    ) {
        let mut config = parse_str("");
        let mut undo_stack: Vec<(purple_ssh::ssh_config::model::ConfigElement, usize)> = Vec::new();

        // Add hosts
        for i in 0..host_count {
            let alias = format!("host-{i}");
            config.add_host(&host_entry(&alias, &format!("10.0.0.{}", i + 1), "user"));
            let _ = config.set_host_tags(&alias, &[format!("group-{}", i % 3)]);
        }

        assert_idempotent(&config);

        // Delete some hosts (undoable)
        for &idx in &delete_indices {
            let entries = config.host_entries();
            if entries.is_empty() {
                break;
            }
            let target_idx = idx % entries.len();
            let alias = entries[target_idx].alias.clone();
            if let Some(undo) = config.delete_host_undoable(&alias) {
                undo_stack.push(undo);
            }
            assert_idempotent(&config);
        }

        // Undo all deletes in reverse order
        while let Some((element, position)) = undo_stack.pop() {
            config.insert_host_at(element, position);
            assert_idempotent(&config);
        }

        // All original hosts should be back
        let entries = config.host_entries();
        for i in 0..host_count {
            let alias = format!("host-{i}");
            prop_assert!(
                entries.iter().any(|e| e.alias == alias),
                "Host '{}' lost after undo cycle",
                alias,
            );
        }

        assert_no_directive_leak(&config);
    }
}

// ---------------------------------------------------------------------------
// Property: mixed CRLF operations
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn crlf_survives_all_mutations(
        actions in prop::collection::vec(action_strategy(), 5..=15),
    ) {
        let initial = "Host alpha\r\n  HostName 10.0.0.1\r\n  User admin\r\n\r\nHost beta\r\n  HostName 10.0.0.2\r\n  User root\r\n";
        let mut config = parse_str(initial);
        prop_assert!(config.crlf);

        for action in &actions {
            apply_action(&mut config, action);
            // CRLF flag should never change
            prop_assert!(config.crlf, "CRLF flag lost after action: {:?}", action);
            assert_idempotent(&config);
        }

        // All line endings should still be CRLF
        let serialized = config.serialize();
        for line in serialized.split('\n') {
            if !line.is_empty() && line != "\r" {
                // Non-empty lines should end with \r (before the \n we split on)
                // Empty lines are just \r\n which splits into ["", ""]
                if !line.ends_with('\r') && serialized.contains("\r\n") {
                    // This is fine - the last segment after final \n may be empty
                    if !line.trim().is_empty() {
                        prop_assert!(
                            line.ends_with('\r'),
                            "Line without CRLF found: {:?}",
                            line,
                        );
                    }
                }
            }
        }
    }
}
