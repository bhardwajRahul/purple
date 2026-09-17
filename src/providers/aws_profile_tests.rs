use super::*;

fn profiles(config: &str, credentials: &str) -> AwsProfiles {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("config");
    let credentials_path = dir.path().join("credentials");
    std::fs::write(&config_path, config).expect("write config");
    std::fs::write(&credentials_path, credentials).expect("write credentials");
    AwsProfiles::load(Some(&config_path), Some(&credentials_path))
}

// =========================================================================
// Section headers
// =========================================================================

#[test]
fn named_profile_in_config_needs_the_profile_prefix() {
    let p = profiles("[profile work]\nregion = eu-west-1\n", "");
    assert_eq!(p.get("work").map(|x| x.region.as_str()), Some("eu-west-1"));
    // The bare form in the config file is not a named profile.
    assert!(p.get("profile work").is_none());
}

#[test]
fn a_bare_named_section_in_config_is_still_read_as_that_profile() {
    let p = profiles("[work]\nregion = eu-west-1\n", "");
    // AWS reads `[work]` in the config file as a profile literally named
    // "work" only in the credentials file; in config it needs the prefix.
    // We accept it as `work` because the header carries no other keyword,
    // which keeps a hand-written file usable.
    assert_eq!(p.get("work").map(|x| x.region.as_str()), Some("eu-west-1"));
}

#[test]
fn default_is_bare_in_both_files() {
    let p = profiles(
        "[default]\nregion = us-east-1\n",
        "[default]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    let d = p.get("default").expect("default profile");
    assert_eq!(d.region, "us-east-1");
    assert_eq!(d.access_key_id, "AKIA");
}

#[test]
fn named_profile_in_credentials_has_no_prefix() {
    let p = profiles(
        "",
        "[work]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    assert!(p.get("work").expect("work").has_static_keys());
}

#[test]
fn sso_session_and_services_sections_are_not_profiles() {
    let p = profiles(
        "[sso-session corp]\nsso_region = eu-west-1\n[services local]\nssm =\n[profile a]\nregion = eu-west-2\n",
        "",
    );
    assert!(p.get("corp").is_none());
    assert!(p.get("local").is_none());
    assert_eq!(p.get("a").map(|x| x.region.as_str()), Some("eu-west-2"));
}

#[test]
fn a_fully_quoted_header_is_one_name_including_its_spaces() {
    // A whitespace split would otherwise read this as a keyword plus a name.
    let p = profiles(
        "",
        "[\"odd name\"]\naws_access_key_id = AKID\naws_secret_access_key = S\n",
    );
    assert!(p.get("odd name").expect("odd name").has_static_keys());
}

#[test]
fn a_quoted_profile_name_loses_its_quotes() {
    let p = profiles("[profile \"my work\"]\nregion = eu-west-1\n", "");
    assert_eq!(
        p.get("my work").map(|x| x.region.as_str()),
        Some("eu-west-1")
    );
}

// =========================================================================
// Merging and precedence
// =========================================================================

#[test]
fn credentials_file_wins_key_by_key() {
    let p = profiles(
        "[profile work]\naws_access_key_id = FROM_CONFIG\nregion = eu-west-1\n",
        "[work]\naws_access_key_id = FROM_CREDENTIALS\naws_secret_access_key = s\n",
    );
    let w = p.get("work").expect("work");
    assert_eq!(w.access_key_id, "FROM_CREDENTIALS");
    // A key only the config file carries survives the merge.
    assert_eq!(w.region, "eu-west-1");
}

#[test]
fn a_repeated_section_in_one_file_merges_rather_than_replaces() {
    let p = profiles(
        "[profile work]\nregion = eu-west-1\n[profile work]\nrole_arn = arn:aws:iam::1:role/r\n",
        "",
    );
    let w = p.get("work").expect("work");
    assert_eq!(w.region, "eu-west-1");
    assert_eq!(w.role_arn, "arn:aws:iam::1:role/r");
}

#[test]
fn a_missing_file_is_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = AwsProfiles::load(
        Some(&dir.path().join("no-config")),
        Some(&dir.path().join("no-credentials")),
    );
    assert!(p.names().is_empty());
}

#[test]
fn no_paths_at_all_yields_no_profiles() {
    let p = AwsProfiles::load(None, None);
    assert!(p.names().is_empty());
}

#[test]
fn names_are_sorted() {
    let p = profiles(
        "[profile zeta]\nregion = a\n[profile alpha]\nregion = b\n",
        "",
    );
    assert_eq!(p.names(), vec!["alpha", "zeta"]);
}

// =========================================================================
// Value cleaning
// =========================================================================

#[test]
fn a_comment_after_whitespace_is_dropped() {
    let p = profiles("[profile a]\nregion = eu-west-1 # production\n", "");
    assert_eq!(p.get("a").map(|x| x.region.as_str()), Some("eu-west-1"));
}

#[test]
fn a_semicolon_comment_after_whitespace_is_dropped() {
    let p = profiles("[profile a]\nregion = eu-west-1\t; prod\n", "");
    assert_eq!(p.get("a").map(|x| x.region.as_str()), Some("eu-west-1"));
}

#[test]
fn a_hash_glued_to_the_value_stays_in_the_value() {
    let p = profiles("[profile a]\nrole_session_name = build#7\n", "");
    assert_eq!(
        p.get("a").map(|x| x.role_session_name.as_str()),
        Some("build#7")
    );
}

#[test]
fn full_line_comments_are_skipped_with_either_marker() {
    let p = profiles(
        "# hash\n; semi\n[profile a]\n# inside\nregion = eu-west-1\n",
        "",
    );
    assert_eq!(p.get("a").map(|x| x.region.as_str()), Some("eu-west-1"));
}

#[test]
fn surrounding_quotes_are_stripped() {
    let p = profiles(
        "[profile a]\nregion = \"eu-west-1\"\nexternal_id = 'abc'\n",
        "",
    );
    let a = p.get("a").expect("a");
    assert_eq!(a.region, "eu-west-1");
    assert_eq!(a.external_id, "abc");
}

#[test]
fn keys_are_case_insensitive_and_whitespace_around_equals_is_ignored() {
    let p = profiles(
        "[profile a]\nRegion=eu-west-1\nROLE_ARN   =   arn:aws:iam::1:role/r\n",
        "",
    );
    let a = p.get("a").expect("a");
    assert_eq!(a.region, "eu-west-1");
    assert_eq!(a.role_arn, "arn:aws:iam::1:role/r");
}

#[test]
fn indented_keys_are_still_settings() {
    // A hand-written credentials file often indents under the header, and
    // dropping those keys would lose the whole profile.
    let p = profiles(
        "",
        "[default]\n  aws_access_key_id  =  AKID  \n  aws_secret_access_key  =  SECRET  \n",
    );
    let d = p.get("default").expect("default");
    assert_eq!(d.access_key_id, "AKID");
    assert_eq!(d.secret_access_key, "SECRET");
}

#[test]
fn a_nested_sub_setting_is_not_read_as_a_profile_setting() {
    // `s3 =` opens a nested block, so the indented keys under it belong to s3.
    let p = profiles(
        "[profile a]\nregion = eu-west-1\ns3 =\n  region = should-be-ignored\n  max_concurrent_requests = 10\nrole_arn = arn:r\n",
        "",
    );
    let a = p.get("a").expect("a");
    assert_eq!(a.region, "eu-west-1");
    assert_eq!(a.role_arn, "arn:r");
}

#[test]
fn unknown_keys_are_ignored() {
    let p = profiles(
        "[profile a]\noutput = json\ncli_pager =\nregion = eu-west-1\n",
        "",
    );
    assert_eq!(p.get("a").map(|x| x.region.as_str()), Some("eu-west-1"));
}

#[test]
fn an_empty_value_reads_as_empty() {
    let p = profiles("[profile a]\nregion =\n", "");
    assert_eq!(p.get("a").map(|x| x.region.as_str()), Some(""));
}

// =========================================================================
// Chain resolution
// =========================================================================

#[test]
fn a_static_profile_resolves_with_no_roles() {
    let p = profiles(
        "",
        "[work]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    let chain = p.resolve_chain("work").expect("chain");
    assert_eq!(chain.base_profile, "work");
    assert!(chain.roles.is_empty());
    assert_eq!(chain.base.access_key_id, "AKIA");
}

#[test]
fn a_role_profile_resolves_to_its_source_plus_one_step() {
    let p = profiles(
        "[profile prod]\nrole_arn = arn:aws:iam::111122223333:role/Admin\nsource_profile = base\nregion = eu-west-1\n",
        "[base]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    let chain = p.resolve_chain("prod").expect("chain");
    assert_eq!(chain.base_profile, "base");
    assert_eq!(chain.roles.len(), 1);
    assert_eq!(
        chain.roles[0].role_arn,
        "arn:aws:iam::111122223333:role/Admin"
    );
    assert_eq!(chain.roles[0].profile, "prod");
}

#[test]
fn a_nested_chain_returns_roles_innermost_first() {
    let p = profiles(
        "[profile outer]\nrole_arn = arn:outer\nsource_profile = middle\n\
         [profile middle]\nrole_arn = arn:middle\nsource_profile = base\n",
        "[base]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    let chain = p.resolve_chain("outer").expect("chain");
    assert_eq!(chain.base_profile, "base");
    let arns: Vec<&str> = chain.roles.iter().map(|r| r.role_arn.as_str()).collect();
    assert_eq!(arns, vec!["arn:middle", "arn:outer"]);
}

#[test]
fn the_role_step_carries_its_optional_settings() {
    let p = profiles(
        "[profile prod]\nrole_arn = arn:r\nsource_profile = base\nrole_session_name = ci\nexternal_id = x1\nduration_seconds = 1800\n",
        "[base]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    let step = &p.resolve_chain("prod").expect("chain").roles[0];
    assert_eq!(step.role_session_name, "ci");
    assert_eq!(step.external_id, "x1");
    assert_eq!(step.duration_seconds, "1800");
}

#[test]
fn a_missing_profile_names_itself() {
    let p = profiles("", "");
    assert_eq!(
        p.resolve_chain("ghost"),
        Err(ChainError::Missing("ghost".to_string()))
    );
}

#[test]
fn a_missing_source_profile_names_the_source_not_the_entry() {
    let p = profiles(
        "[profile prod]\nrole_arn = arn:r\nsource_profile = gone\n",
        "",
    );
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::Missing("gone".to_string()))
    );
}

#[test]
fn a_profile_without_keys_reports_no_keys() {
    let p = profiles("[profile empty]\nregion = eu-west-1\n", "");
    assert_eq!(
        p.resolve_chain("empty"),
        Err(ChainError::NoKeys("empty".to_string()))
    );
}

#[test]
fn a_role_without_a_source_is_named() {
    let p = profiles("[profile prod]\nrole_arn = arn:r\n", "");
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::RoleWithoutSource("prod".to_string()))
    );
}

#[test]
fn credential_source_is_reported_rather_than_guessed() {
    let p = profiles(
        "[profile prod]\nrole_arn = arn:r\ncredential_source = Ec2InstanceMetadata\n",
        "",
    );
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::CredentialSource("prod".to_string()))
    );
}

#[test]
fn credential_source_without_a_role_is_also_reported() {
    let p = profiles("[profile base]\ncredential_source = Environment\n", "");
    assert_eq!(
        p.resolve_chain("base"),
        Err(ChainError::CredentialSource("base".to_string()))
    );
}

#[test]
fn mfa_serial_is_reported_because_sync_cannot_prompt() {
    let p = profiles(
        "[profile prod]\nrole_arn = arn:r\nsource_profile = base\nmfa_serial = arn:aws:iam::1:mfa/u\n",
        "[base]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::MfaRequired("prod".to_string()))
    );
}

#[test]
fn credential_process_is_reported() {
    let p = profiles("[profile p]\ncredential_process = /usr/bin/creds\n", "");
    assert_eq!(
        p.resolve_chain("p"),
        Err(ChainError::CredentialProcess("p".to_string()))
    );
}

#[test]
fn an_sso_session_profile_is_reported() {
    let p = profiles("[profile p]\nsso_session = corp\nsso_account_id = 1\n", "");
    assert_eq!(
        p.resolve_chain("p"),
        Err(ChainError::SsoNotSupported("p".to_string()))
    );
}

#[test]
fn a_legacy_sso_profile_is_reported() {
    let p = profiles(
        "[profile p]\nsso_start_url = https://x.awsapps.com/start\n",
        "",
    );
    assert_eq!(
        p.resolve_chain("p"),
        Err(ChainError::SsoNotSupported("p".to_string()))
    );
}

#[test]
fn an_sso_marker_wins_over_leftover_static_keys() {
    // `aws configure sso` leaves the old credentials block in place, so this
    // is what a migrated profile normally looks like. The AWS CLI runs its
    // SSO provider first and reaches one account; the keys reach another.
    // Saying so beats syncing the wrong account's instances.
    let p = profiles(
        "[profile p]\nsso_start_url = https://x.awsapps.com/start\n",
        "[p]\naws_access_key_id = AKIA\naws_secret_access_key = s\n",
    );
    assert_eq!(
        p.resolve_chain("p"),
        Err(ChainError::SsoNotSupported("p".to_string()))
    );
}

#[test]
fn a_self_referencing_source_profile_without_keys_is_a_loop() {
    let p = profiles("[profile a]\nrole_arn = arn:r\nsource_profile = a\n", "");
    assert_eq!(p.resolve_chain("a"), Err(ChainError::Loop("a".to_string())));
}

#[test]
fn a_profile_may_source_itself_when_it_holds_the_keys() {
    // Documented by AWS and allowed by botocore: keys and role config in one
    // block. `aws --profile a` resolves it, so purple has to as well.
    let p = profiles(
        "[profile a]\nrole_arn = arn:aws:iam::1:role/R\nsource_profile = a\n",
        "[a]\naws_access_key_id = AKIASELF\naws_secret_access_key = S\n",
    );
    let chain = p.resolve_chain("a").expect("chain");
    assert_eq!(chain.base_profile, "a");
    assert_eq!(chain.base.access_key_id, "AKIASELF");
    assert_eq!(chain.roles.len(), 1);
    assert_eq!(chain.roles[0].role_arn, "arn:aws:iam::1:role/R");
}

#[test]
fn a_two_profile_cycle_is_a_loop() {
    let p = profiles(
        "[profile a]\nrole_arn = arn:a\nsource_profile = b\n[profile b]\nrole_arn = arn:b\nsource_profile = a\n",
        "",
    );
    assert_eq!(p.resolve_chain("a"), Err(ChainError::Loop("a".to_string())));
}

#[test]
fn a_chain_deeper_than_the_cap_is_refused() {
    let mut config = String::new();
    for i in 0..MAX_SOURCE_PROFILE_DEPTH + 2 {
        config.push_str(&format!(
            "[profile p{}]\nrole_arn = arn:{}\nsource_profile = p{}\n",
            i,
            i,
            i + 1
        ));
    }
    let p = profiles(&config, "");
    assert!(matches!(p.resolve_chain("p0"), Err(ChainError::TooDeep(_))));
}

#[test]
fn the_session_token_rides_along_with_static_keys() {
    let p = profiles(
        "",
        "[t]\naws_access_key_id = ASIA\naws_secret_access_key = s\naws_session_token = tok\n",
    );
    let chain = p.resolve_chain("t").expect("chain");
    assert_eq!(chain.base.session_token, "tok");
}

#[test]
fn half_a_key_pair_names_the_key_it_is_missing() {
    let p = profiles("", "[t]\naws_access_key_id = AKIA\n");
    assert!(!p.get("t").expect("t").has_static_keys());
    assert_eq!(
        p.resolve_chain("t"),
        Err(ChainError::PartialCredentials(
            "t".to_string(),
            "aws_secret_access_key"
        ))
    );
}

#[test]
fn a_secret_without_its_access_key_id_names_the_missing_id() {
    let p = profiles("", "[t]\naws_secret_access_key = S\n");
    assert_eq!(
        p.resolve_chain("t"),
        Err(ChainError::PartialCredentials(
            "t".to_string(),
            "aws_access_key_id"
        ))
    );
}

#[test]
fn a_key_pair_never_splits_across_the_two_files() {
    // Mid-rotation: the credentials file supplies a new id while its secret
    // line is commented out, and the config file still holds the old pair.
    // Signing an id from one file with a secret from the other only ever
    // yields SignatureDoesNotMatch, so the credentials file's block wins
    // whole and the missing half is reported.
    let p = profiles(
        "[profile prod]\naws_access_key_id = FROM_CONFIG\naws_secret_access_key = CFGSECRET\nregion = us-east-1\n",
        "[prod]\naws_access_key_id = FROM_CREDS\n",
    );
    let merged = p.get("prod").expect("prod");
    assert_eq!(merged.access_key_id, "FROM_CREDS");
    assert_eq!(merged.secret_access_key, "");
    // Settings that are not credentials still merge key by key.
    assert_eq!(merged.region, "us-east-1");
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::PartialCredentials(
            "prod".to_string(),
            "aws_secret_access_key"
        ))
    );
}

#[test]
fn a_session_token_is_dropped_with_the_pair_it_belonged_to() {
    // The token signs the pair it was issued with, so it leaves when the
    // pair does.
    let p = profiles(
        "[profile t]\naws_access_key_id = ASIAOLD\naws_secret_access_key = OLD\naws_session_token = OLDTOKEN\n",
        "[t]\naws_access_key_id = AKIANEW\naws_secret_access_key = NEW\n",
    );
    let merged = p.get("t").expect("t");
    assert_eq!(merged.access_key_id, "AKIANEW");
    assert_eq!(merged.session_token, "");
}

#[test]
fn two_headers_naming_one_profile_do_not_split_its_key_pair() {
    // `[profile "work"]` and `[profile work]` unquote to the same name. The
    // second block replaces the first block's credential set rather than
    // completing it.
    let p = profiles(
        "[profile \"work\"]\naws_access_key_id = FIRST\n[profile work]\naws_secret_access_key = SECOND\n",
        "",
    );
    let merged = p.get("work").expect("work");
    assert_eq!(merged.access_key_id, "");
    assert_eq!(merged.secret_access_key, "SECOND");
}

#[test]
fn a_file_that_cannot_be_read_is_named_instead_of_the_profile() {
    // chmod 000, or a path that points at a directory. Both parse as empty,
    // so naming the profile would send the user after one that is right there.
    let dir = tempfile::tempdir().expect("tempdir");
    let credentials = dir.path().join("credentials");
    std::fs::create_dir(&credentials).expect("create a directory where a file is expected");
    let p = AwsProfiles::load(None, Some(&credentials));
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::FileUnreadable(
            credentials.display().to_string()
        ))
    );
}

#[test]
fn an_absent_file_still_reports_the_profile_as_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = AwsProfiles::load(None, Some(&dir.path().join("credentials")));
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::Missing("prod".to_string()))
    );
}

// =========================================================================
// Secret handling
// =========================================================================

#[test]
fn debug_output_never_carries_a_key_pair() {
    // A profile flows into the same pipeline as AwsCredentials, which has no
    // Debug at all on purpose. This one keeps Debug so a chain can be
    // compared in a test, so it has to redact instead.
    let p = profiles(
        "",
        "[work]\naws_access_key_id = AKIAREALKEYID\naws_secret_access_key = REALSECRET\naws_session_token = REALTOKEN\n",
    );
    let rendered = format!("{:?}", p.get("work").expect("work"));
    for secret in ["AKIAREALKEYID", "REALSECRET", "REALTOKEN"] {
        assert!(
            !rendered.contains(secret),
            "{secret} leaked into Debug output: {rendered}"
        );
    }
    assert!(rendered.contains("<redacted>"), "got: {rendered}");
}

#[test]
fn debug_output_still_shows_the_non_secret_fields() {
    // Redaction must not make the type useless for troubleshooting a chain.
    let p = profiles(
        "[profile prod]\nrole_arn = arn:aws:iam::1:role/Admin\nsource_profile = base\nregion = eu-west-1\n",
        "",
    );
    let rendered = format!("{:?}", p.get("prod").expect("prod"));
    assert!(
        rendered.contains("arn:aws:iam::1:role/Admin"),
        "got: {rendered}"
    );
    assert!(rendered.contains("eu-west-1"), "got: {rendered}");
    assert!(rendered.contains("<empty>"), "got: {rendered}");
}

#[test]
fn a_resolved_chain_does_not_leak_its_base_credentials() {
    // ResolvedChain embeds the base profile, so it inherits the redaction.
    let p = profiles(
        "[profile prod]\nrole_arn = arn:r\nsource_profile = base\n",
        "[base]\naws_access_key_id = AKIALEAK\naws_secret_access_key = SECRETLEAK\n",
    );
    let rendered = format!("{:?}", p.resolve_chain("prod").expect("chain"));
    assert!(!rendered.contains("AKIALEAK"), "got: {rendered}");
    assert!(!rendered.contains("SECRETLEAK"), "got: {rendered}");
}

#[test]
fn a_source_profile_that_holds_keys_does_not_add_its_own_role() {
    // The AWS CLI resolves a source profile through its provider chain, which
    // returns that profile's keys and never looks at its role_arn. Assuming it
    // anyway would ask for a hop the trust policy has no reason to allow.
    let p = profiles(
        "[profile child]\nrole_arn = arn:R2\nsource_profile = a\n\
         [profile a]\nrole_arn = arn:R1\nsource_profile = a\n",
        "[a]\naws_access_key_id = AKIA\naws_secret_access_key = S\n",
    );
    let chain = p.resolve_chain("child").expect("chain");
    assert_eq!(chain.base_profile, "a");
    let arns: Vec<&str> = chain.roles.iter().map(|r| r.role_arn.as_str()).collect();
    assert_eq!(arns, ["arn:R2"]);
}

#[test]
fn the_requested_profile_follows_its_role_even_when_it_holds_keys() {
    // The other half of the same rule: the profile the user named is resolved
    // by the assume-role provider, which runs ahead of the chain that would
    // have returned these keys.
    let p = profiles(
        "[profile a]\nrole_arn = arn:R\nsource_profile = base\n\
         [profile base]\nregion = eu-west-1\n",
        "[a]\naws_access_key_id = AKIAOWN\naws_secret_access_key = S\n\
         [base]\naws_access_key_id = AKIABASE\naws_secret_access_key = BS\n",
    );
    let chain = p.resolve_chain("a").expect("chain");
    assert_eq!(chain.base_profile, "base");
    assert_eq!(chain.base.access_key_id, "AKIABASE");
    assert_eq!(chain.roles.len(), 1);
}

#[test]
fn a_self_referencing_profile_with_an_sso_marker_is_still_refused() {
    // The self-reference resolves its base out of the same profile, so it has
    // to apply the provider order there too. Identity Center wins over the
    // leftover keys, exactly as it does for a plain profile.
    let p = profiles(
        "[profile a]\nrole_arn = arn:R\nsource_profile = a\nsso_start_url = https://x.awsapps.com/start\n",
        "[a]\naws_access_key_id = AKIAOLD\naws_secret_access_key = OLD\n",
    );
    assert_eq!(
        p.resolve_chain("a"),
        Err(ChainError::SsoNotSupported("a".to_string()))
    );
}

#[test]
fn a_credentials_section_holding_only_a_token_keeps_the_config_pair() {
    // The token alone is not a credential set, so it must not blank the pair
    // the config file supplied. The AWS CLI falls through to that pair too.
    let p = profiles(
        "[profile a]\naws_access_key_id = AK\naws_secret_access_key = SK\n",
        "[a]\naws_session_token = TOK\n",
    );
    let merged = p.get("a").expect("a");
    assert_eq!(merged.access_key_id, "AK");
    assert_eq!(merged.secret_access_key, "SK");
    assert_eq!(merged.session_token, "TOK");
    assert_eq!(p.resolve_chain("a").expect("chain").base_profile, "a");
}

#[test]
fn an_unreadable_file_is_named_even_when_another_file_declares_the_profile() {
    // What `aws configure` actually writes: the profile block lives in
    // ~/.aws/config while the keys live in ~/.aws/credentials. With the
    // credentials file unreadable the profile resolves, so the absence of keys
    // is the only symptom, and blaming the profile for it sends the user to
    // add keys that are already there.
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("config");
    std::fs::write(
        &config,
        "[profile prod]\nregion = eu-west-1\noutput = json\n",
    )
    .expect("write config");
    let credentials = dir.path().join("credentials");
    std::fs::create_dir(&credentials).expect("a directory where a file is expected");

    let p = AwsProfiles::load(Some(&config), Some(&credentials));
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::FileUnreadable(
            credentials.display().to_string()
        ))
    );
}

#[test]
fn a_readable_file_still_reports_a_profile_that_simply_has_no_keys() {
    let p = profiles("[profile prod]\nregion = eu-west-1\n", "");
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::NoKeys("prod".to_string()))
    );
}

#[test]
fn a_web_identity_profile_is_reported_rather_than_read_as_a_chain() {
    // IRSA on EKS and OIDC from a CI job. The AWS CLI keeps its assume-role
    // provider away from a profile carrying the token file, so the role_arn
    // beside it is not a source_profile chain and asking for one would send
    // the user after keys that do not exist.
    let p = profiles(
        "[profile eks]\nrole_arn = arn:aws:iam::1:role/Pod\nweb_identity_token_file = /var/run/secrets/token\n",
        "",
    );
    assert_eq!(
        p.resolve_chain("eks"),
        Err(ChainError::WebIdentity("eks".to_string()))
    );
}

#[test]
fn credential_process_wins_over_a_key_pair_in_the_config_file() {
    // The AWS CLI runs the process before it reads the config file's keys, so
    // using those keys would reach whatever account the stale pair belongs to.
    let p = profiles(
        "[profile a]\ncredential_process = /usr/bin/creds\naws_access_key_id = AK\naws_secret_access_key = SK\n",
        "",
    );
    assert_eq!(
        p.resolve_chain("a"),
        Err(ChainError::CredentialProcess("a".to_string()))
    );
}

#[test]
fn a_key_pair_in_the_credentials_file_wins_over_credential_process() {
    // The other side of the same order: that file is read first.
    let p = profiles(
        "[profile a]\ncredential_process = /usr/bin/creds\n",
        "[a]\naws_access_key_id = AK\naws_secret_access_key = SK\n",
    );
    let chain = p.resolve_chain("a").expect("chain");
    assert_eq!(chain.base.access_key_id, "AK");
}

#[test]
fn half_a_key_pair_stops_the_walk_instead_of_being_passed_over() {
    // The AWS CLI treats either key as "this profile is the source", then
    // fails on the missing half. Walking past it would sign against whatever
    // account the next link reaches.
    let p = profiles(
        "[profile top]\nrole_arn = arn:R\nsource_profile = mid\n\
         [profile mid]\nrole_arn = arn:R2\nsource_profile = base\naws_access_key_id = ONLYID\n\
         [profile base]\naws_access_key_id = AK\naws_secret_access_key = SK\n",
        "",
    );
    assert_eq!(
        p.resolve_chain("top"),
        Err(ChainError::PartialCredentials(
            "mid".to_string(),
            "aws_secret_access_key"
        ))
    );
}

#[test]
fn half_a_key_pair_under_an_unreadable_file_names_the_file() {
    // The secret may well be in the file purple could not open.
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("config");
    std::fs::write(&config, "[profile prod]\naws_access_key_id = AK\n").expect("write config");
    let credentials = dir.path().join("credentials");
    std::fs::create_dir(&credentials).expect("a directory where a file is expected");

    let p = AwsProfiles::load(Some(&config), Some(&credentials));
    assert_eq!(
        p.resolve_chain("prod"),
        Err(ChainError::FileUnreadable(
            credentials.display().to_string()
        ))
    );
}

#[test]
fn a_chain_exactly_one_link_past_the_cap_is_refused() {
    // Pinned at the boundary, so moving the comparison one link either way
    // fails here rather than only on a chain nobody writes.
    let mut config = String::new();
    for i in 0..=MAX_SOURCE_PROFILE_DEPTH {
        config.push_str(&format!(
            "[profile p{}]\nrole_arn = arn:{}\nsource_profile = p{}\n",
            i,
            i,
            i + 1
        ));
    }
    let p = profiles(&config, "");
    assert_eq!(
        p.resolve_chain("p0"),
        Err(ChainError::TooDeep(format!(
            "p{}",
            MAX_SOURCE_PROFILE_DEPTH
        )))
    );
}

#[test]
fn a_chain_exactly_at_the_cap_still_resolves() {
    let mut config = String::new();
    for i in 0..MAX_SOURCE_PROFILE_DEPTH - 1 {
        config.push_str(&format!(
            "[profile p{}]\nrole_arn = arn:{}\nsource_profile = p{}\n",
            i,
            i,
            i + 1
        ));
    }
    let base = format!("p{}", MAX_SOURCE_PROFILE_DEPTH - 1);
    let p = profiles(
        &config,
        &format!("[{base}]\naws_access_key_id = AK\naws_secret_access_key = SK\n"),
    );
    let chain = p.resolve_chain("p0").expect("a chain at the cap resolves");
    assert_eq!(chain.base_profile, base);
    assert_eq!(chain.roles.len(), MAX_SOURCE_PROFILE_DEPTH - 1);
}

#[test]
fn the_aws_cli_own_config_sections_are_not_profiles() {
    // `[plugins]` and `[preview]` carry CLI settings. A profile of the same
    // name would be written `[profile plugins]`, so reading them as profiles
    // would put two rows in the picker that can never sync.
    let p = profiles(
        "[plugins]\ncwd = /tmp\n[preview]\ncloudfront = true\n[profile real]\nregion = eu-west-1\n",
        "",
    );
    assert_eq!(p.names(), vec!["real"]);
}

#[test]
fn a_bare_plugins_section_in_the_credentials_file_is_still_a_profile() {
    // The exclusion is a config-file rule. The credentials file has no such
    // sections, so a profile really named `plugins` keeps working there.
    let p = profiles(
        "",
        "[plugins]\naws_access_key_id = AK\naws_secret_access_key = SK\n",
    );
    assert_eq!(p.names(), vec!["plugins"]);
}
