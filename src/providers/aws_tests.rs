use super::*;

/// Resolve credentials that need no assume-role call, which is what every
/// test below expects. Panics on a chain so a mis-set fixture is loud.
fn resolve_ready(
    token: &str,
    profile: &str,
    env: &crate::runtime::env::Env,
) -> Result<AwsCredentials, ProviderError> {
    match resolve_credentials(token, profile, env)? {
        CredentialSource::Ready(creds) => Ok(creds),
        CredentialSource::AssumeRole { .. } => {
            panic!("expected ready credentials, got a role chain")
        }
    }
}

/// Sign an EC2 Query API request: the shape every pre-existing signing test
/// was written against.
fn sign_ec2(
    creds: &AwsCredentials,
    region: &str,
    host: &str,
    query_string: &str,
    timestamp: &str,
    datestamp: &str,
) -> String {
    sign_request(
        creds,
        region,
        &SigV4Request {
            method: "GET",
            service: EC2_SERVICE,
            host,
            query_string,
            payload: b"",
            extra_headers: &[],
        },
        timestamp,
        datestamp,
    )
}

// =========================================================================
// format_utc
// =========================================================================

#[test]
fn test_format_utc_epoch_zero() {
    let (ts, ds) = format_utc(0);
    assert_eq!(ts, "19700101T000000Z");
    assert_eq!(ds, "19700101");
}

#[test]
fn test_format_utc_known_date() {
    // 2024-01-15 12:30:45 UTC = 1705321845
    let (ts, ds) = format_utc(1705321845);
    assert_eq!(ts, "20240115T123045Z");
    assert_eq!(ds, "20240115");
}

#[test]
fn test_format_utc_leap_year() {
    // 2024-02-29 00:00:00 UTC = 1709164800
    let (ts, ds) = format_utc(1709164800);
    assert_eq!(ts, "20240229T000000Z");
    assert_eq!(ds, "20240229");
}

#[test]
fn test_format_utc_end_of_year() {
    // 2023-12-31 23:59:59 UTC = 1704067199
    let (ts, ds) = format_utc(1704067199);
    assert_eq!(ts, "20231231T235959Z");
    assert_eq!(ds, "20231231");
}

#[test]
fn test_format_utc_year_2000() {
    // 2000-03-01 00:00:00 UTC = 951868800
    let (ts, ds) = format_utc(951868800);
    assert_eq!(ts, "20000301T000000Z");
    assert_eq!(ds, "20000301");
}

// =========================================================================
// uri_encode
// =========================================================================

#[test]
fn test_uri_encode_passthrough() {
    assert_eq!(uri_encode("abc123-_.~"), "abc123-_.~");
}

#[test]
fn test_uri_encode_special_chars() {
    assert_eq!(uri_encode("hello world"), "hello%20world");
    assert_eq!(uri_encode("a=b&c"), "a%3Db%26c");
    assert_eq!(uri_encode("/path"), "%2Fpath");
}

#[test]
fn test_uri_encode_empty() {
    assert_eq!(uri_encode(""), "");
}

// =========================================================================
// hex_encode
// =========================================================================

#[test]
fn test_hex_encode() {
    assert_eq!(hex_encode(&[0x00, 0xff, 0xab]), "00ffab");
    assert_eq!(hex_encode(&[]), "");
}

// =========================================================================
// sha256_hash
// =========================================================================

#[test]
fn test_sha256_empty() {
    let hash = hex_encode(&sha256_hash(b""));
    assert_eq!(
        hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn test_sha256_known() {
    let hash = hex_encode(&sha256_hash(b"hello"));
    assert_eq!(
        hash,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

// =========================================================================
// hmac_sha256
// =========================================================================

#[test]
fn test_hmac_sha256_known() {
    // HMAC-SHA256("key", "message") is a well-known test vector
    let result = hex_encode(&hmac_sha256(
        b"key",
        b"The quick brown fox jumps over the lazy dog",
    ));
    assert_eq!(
        result,
        "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
    );
}

// =========================================================================
// sign_request (SigV4)
// =========================================================================

#[test]
fn test_sign_request_format() {
    let creds = AwsCredentials {
        access_key: "AKIDEXAMPLE".to_string(),
        secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
        session_token: None,
    };
    let auth = sign_ec2(
        &creds,
        "us-east-1",
        "ec2.us-east-1.amazonaws.com",
        "Action=DescribeInstances&Version=2016-11-15",
        "20150830T123600Z",
        "20150830",
    );
    assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/ec2/aws4_request, SignedHeaders=host;x-amz-date, Signature="));
    // Signature should be a 64-char hex string
    let sig = auth.rsplit("Signature=").next().unwrap();
    assert_eq!(sig.len(), 64);
    assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_sign_request_deterministic() {
    let creds = AwsCredentials {
        access_key: "AK".to_string(),
        secret_key: "SK".to_string(),
        session_token: None,
    };
    let a = sign_ec2(
        &creds,
        "us-east-1",
        "ec2.us-east-1.amazonaws.com",
        "Action=DescribeInstances",
        "20240101T000000Z",
        "20240101",
    );
    let b = sign_ec2(
        &creds,
        "us-east-1",
        "ec2.us-east-1.amazonaws.com",
        "Action=DescribeInstances",
        "20240101T000000Z",
        "20240101",
    );
    assert_eq!(a, b);
}

#[test]
fn test_sign_request_different_regions() {
    let creds = AwsCredentials {
        access_key: "AK".to_string(),
        secret_key: "SK".to_string(),
        session_token: None,
    };
    let a = sign_ec2(
        &creds,
        "us-east-1",
        "ec2.us-east-1.amazonaws.com",
        "Action=DescribeInstances",
        "20240101T000000Z",
        "20240101",
    );
    let b = sign_ec2(
        &creds,
        "eu-west-1",
        "ec2.eu-west-1.amazonaws.com",
        "Action=DescribeInstances",
        "20240101T000000Z",
        "20240101",
    );
    assert_ne!(a, b);
}

#[test]
fn test_sign_request_includes_security_token_when_present() {
    let creds = AwsCredentials {
        access_key: "ASIAEXAMPLE".to_string(),
        secret_key: "SK".to_string(),
        session_token: Some("TOKEN".to_string()),
    };
    let auth = sign_ec2(
        &creds,
        "eu-central-1",
        "ec2.eu-central-1.amazonaws.com",
        "Action=DescribeInstances&Version=2016-11-15",
        "20240101T000000Z",
        "20240101",
    );
    assert!(auth.contains("SignedHeaders=host;x-amz-date;x-amz-security-token,"));
}

#[test]
fn test_sign_request_session_token_changes_signature() {
    let base = AwsCredentials {
        access_key: "ASIAEXAMPLE".to_string(),
        secret_key: "SK".to_string(),
        session_token: None,
    };
    let with_token = AwsCredentials {
        access_key: "ASIAEXAMPLE".to_string(),
        secret_key: "SK".to_string(),
        session_token: Some("TOKEN".to_string()),
    };
    let args = (
        "eu-central-1",
        "ec2.eu-central-1.amazonaws.com",
        "Action=DescribeInstances",
        "20240101T000000Z",
        "20240101",
    );
    let a = sign_ec2(&base, args.0, args.1, args.2, args.3, args.4);
    let b = sign_ec2(&with_token, args.0, args.1, args.2, args.3, args.4);
    assert_ne!(a, b);
}

#[test]
fn test_resolve_credentials_env_without_session_token() {
    let env = crate::runtime::env::Env::for_test("/tmp/x")
        .with_var("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE")
        .with_var("AWS_SECRET_ACCESS_KEY", "SECRET");
    let creds = resolve_ready("", "", &env).unwrap();
    assert_eq!(creds.access_key, "AKIDEXAMPLE");
    assert_eq!(creds.secret_key, "SECRET");
    assert_eq!(creds.session_token, None);
}

#[test]
fn test_resolve_credentials_env_with_session_token() {
    let env = crate::runtime::env::Env::for_test("/tmp/x")
        .with_var("AWS_ACCESS_KEY_ID", "ASIAEXAMPLE")
        .with_var("AWS_SECRET_ACCESS_KEY", "SECRET")
        .with_var("AWS_SESSION_TOKEN", "TOKEN");
    let creds = resolve_ready("", "", &env).unwrap();
    assert_eq!(creds.access_key, "ASIAEXAMPLE");
    assert_eq!(creds.secret_key, "SECRET");
    assert_eq!(creds.session_token.as_deref(), Some("TOKEN"));
}

#[test]
fn test_resolve_credentials_profile_shadows_env_session_token() {
    // A configured profile wins over the environment, so every field comes
    // from the credentials file even when both sources are populated.
    let home = tempfile::tempdir().expect("tempdir");
    let aws_dir = home.path().join(".aws");
    std::fs::create_dir_all(&aws_dir).expect("create .aws");
    std::fs::write(
        aws_dir.join("credentials"),
        "[default]\naws_access_key_id = ASIAFROMFILE\naws_secret_access_key = FILESECRET\naws_session_token = FILETOKEN\n",
    )
    .expect("write credentials file");

    let env = crate::runtime::env::Env::for_test(home.path())
        .with_var("AWS_ACCESS_KEY_ID", "ASIAFROMENV")
        .with_var("AWS_SECRET_ACCESS_KEY", "ENVSECRET")
        .with_var("AWS_SESSION_TOKEN", "ENVTOKEN");

    let creds = resolve_ready("", "default", &env).expect("profile resolves from file");
    assert_eq!(creds.access_key, "ASIAFROMFILE");
    assert_eq!(creds.secret_key, "FILESECRET");
    assert_eq!(creds.session_token.as_deref(), Some("FILETOKEN"));
}

#[test]
fn test_resolve_credentials_token_with_session_token() {
    let creds = resolve_ready(
        "ASIAEXAMPLE:SECRET:TOKEN",
        "",
        &crate::runtime::env::Env::empty(),
    )
    .unwrap();
    assert_eq!(creds.access_key, "ASIAEXAMPLE");
    assert_eq!(creds.secret_key, "SECRET");
    assert_eq!(creds.session_token.as_deref(), Some("TOKEN"));
}

#[test]
fn test_resolve_credentials_token_trailing_separator() {
    // An empty third component leaves the secret key intact and yields no
    // session token.
    let creds = resolve_ready("AKID:SECRET:", "", &crate::runtime::env::Env::empty())
        .expect("access key and secret are both present");
    assert_eq!(creds.access_key, "AKID");
    assert_eq!(creds.secret_key, "SECRET");
    assert_eq!(creds.session_token, None);
}

// =========================================================================
// resolve_credentials (token parsing)
// =========================================================================

#[test]
fn test_resolve_credentials_token_format() {
    let creds = resolve_ready("AKID:SECRET", "", &crate::runtime::env::Env::empty()).unwrap();
    assert_eq!(creds.access_key, "AKID");
    assert_eq!(creds.secret_key, "SECRET");
}

#[test]
fn test_resolve_credentials_empty_parts() {
    // Empty access key
    assert!(resolve_ready(":SECRET", "", &crate::runtime::env::Env::empty()).is_err());
    // Empty secret key
    assert!(resolve_ready("AKID:", "", &crate::runtime::env::Env::empty()).is_err());
}

#[test]
fn test_resolve_credentials_nothing_configured_names_the_sources() {
    // The config saves without a token and without a profile, so the sync-time
    // failure has to say where credentials can come from.
    // `AwsCredentials` has no Debug on purpose, so unwrap the error side.
    let err = resolve_ready("", "", &crate::runtime::env::Env::empty())
        .err()
        .expect("no token, no profile and no environment must fail");
    let msg = err.to_string();
    assert!(msg.contains("AWS_ACCESS_KEY_ID"), "unhelpful error: {msg}");
    assert!(
        !msg.contains("API token"),
        "must not blame a token that was never set: {msg}"
    );
}

#[test]
fn test_resolve_credentials_missing_profile_names_the_profile() {
    // A profile takes priority and is used on its own, so its failure must
    // not read as a token problem. The token here is deliberately valid.
    let home = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(home.path().join(".aws")).expect("mkdir");
    std::fs::write(
        home.path().join(".aws").join("credentials"),
        "[other]\naws_access_key_id = AKID\naws_secret_access_key = SECRET\n",
    )
    .expect("write");
    let env = crate::runtime::env::Env::for_test(home.path());
    let err = resolve_ready("AKID:SECRET", "missing", &env)
        .err()
        .expect("a profile that is not in the file must fail");
    let msg = err.to_string();
    assert!(msg.contains("missing"), "profile not named: {msg}");
    assert!(!msg.contains("API token"), "blames the token: {msg}");
}

#[test]
fn test_resolve_credentials_malformed_token_still_blames_the_token() {
    // A token that was set but cannot be parsed keeps the auth error.
    let err = resolve_ready("AKID:", "", &crate::runtime::env::Env::empty())
        .err()
        .expect("a malformed token must fail");
    assert!(
        err.to_string().contains("API token"),
        "malformed token should point at the token: {err}"
    );
}

#[test]
fn test_resolve_credentials_no_colon() {
    // No colon in token: split_once fails, falls through to env vars
    // Token-only (no colon) should not produce valid credentials from token path
    let result = resolve_ready("just-a-token", "", &crate::runtime::env::Env::empty());
    // Result depends on env vars. Verify token path was skipped by
    // confirming credentials (if any) don't contain the raw token string.
    if let Ok(ref creds) = result {
        assert_ne!(creds.access_key, "just-a-token");
        assert_ne!(creds.secret_key, "just-a-token");
    }
}

// =========================================================================
// XML parsing: DescribeInstances
// =========================================================================

#[test]
fn test_parse_describe_instances_basic() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<requestId>abc123</requestId>
<reservationSet>
    <item>
        <reservationId>r-12345</reservationId>
        <instancesSet>
            <item>
                <instanceId>i-abc123</instanceId>
                <imageId>ami-12345</imageId>
                <instanceState><name>running</name></instanceState>
                <instanceType>t3.micro</instanceType>
                <ipAddress>1.2.3.4</ipAddress>
                <placement><availabilityZone>us-east-1a</availabilityZone></placement>
                <tagSet>
                    <item><key>Name</key><value>web-01</value></item>
                    <item><key>Environment</key><value>prod</value></item>
                </tagSet>
            </item>
        </instancesSet>
    </item>
</reservationSet>
</DescribeInstancesResponse>"#;

    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    assert_eq!(resp.reservation_set.item.len(), 1);
    let instance = &resp.reservation_set.item[0].instances_set.item[0];
    assert_eq!(instance.instance_id, "i-abc123");
    assert_eq!(instance.image_id, "ami-12345");
    assert_eq!(instance.instance_state.name, "running");
    assert_eq!(instance.instance_type, "t3.micro");
    assert_eq!(instance.ip_address.as_deref(), Some("1.2.3.4"));
    assert_eq!(instance.tag_set.item.len(), 2);
}

#[test]
fn test_parse_describe_instances_no_public_ip() {
    let xml = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<reservationSet>
    <item>
        <instancesSet>
            <item>
                <instanceId>i-noip</instanceId>
                <instanceState><name>running</name></instanceState>
                <tagSet/>
            </item>
        </instancesSet>
    </item>
</reservationSet>
</DescribeInstancesResponse>"#;

    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    let instance = &resp.reservation_set.item[0].instances_set.item[0];
    assert!(instance.ip_address.is_none());
}

#[test]
fn test_parse_describe_instances_empty() {
    let xml = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<reservationSet/>
</DescribeInstancesResponse>"#;

    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    assert!(resp.reservation_set.item.is_empty());
}

#[test]
fn test_parse_describe_instances_with_next_token() {
    let xml = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<reservationSet/>
<nextToken>eyJ0b2tlbiI6ICJ0ZXN0In0=</nextToken>
</DescribeInstancesResponse>"#;

    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    assert_eq!(resp.next_token.as_deref(), Some("eyJ0b2tlbiI6ICJ0ZXN0In0="));
}

#[test]
fn test_parse_describe_instances_multiple_reservations() {
    let xml = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<reservationSet>
    <item>
        <instancesSet>
            <item>
                <instanceId>i-001</instanceId>
                <instanceState><name>running</name></instanceState>
                <ipAddress>1.1.1.1</ipAddress>
            </item>
        </instancesSet>
    </item>
    <item>
        <instancesSet>
            <item>
                <instanceId>i-002</instanceId>
                <instanceState><name>running</name></instanceState>
                <ipAddress>2.2.2.2</ipAddress>
            </item>
        </instancesSet>
    </item>
</reservationSet>
</DescribeInstancesResponse>"#;

    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    assert_eq!(resp.reservation_set.item.len(), 2);
    assert_eq!(
        resp.reservation_set.item[0].instances_set.item[0].instance_id,
        "i-001"
    );
    assert_eq!(
        resp.reservation_set.item[1].instances_set.item[0].instance_id,
        "i-002"
    );
}

// =========================================================================
// XML parsing: DescribeImages
// =========================================================================

#[test]
fn test_parse_describe_images() {
    let xml = r#"<DescribeImagesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<imagesSet>
    <item>
        <imageId>ami-12345</imageId>
        <name>ubuntu/images/hvm-ssd/ubuntu-jammy-22.04-amd64-server-20240101</name>
    </item>
    <item>
        <imageId>ami-67890</imageId>
        <name>amzn2-ami-hvm-2.0.20240101.0-x86_64-gp2</name>
    </item>
</imagesSet>
</DescribeImagesResponse>"#;

    let resp: DescribeImagesResponse = quick_xml::de::from_str(xml).unwrap();
    assert_eq!(resp.images_set.item.len(), 2);
    assert_eq!(resp.images_set.item[0].image_id, "ami-12345");
    assert!(resp.images_set.item[0].name.contains("ubuntu"));
    assert_eq!(resp.images_set.item[1].image_id, "ami-67890");
}

#[test]
fn test_parse_describe_images_empty() {
    let xml = r#"<DescribeImagesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<imagesSet/>
</DescribeImagesResponse>"#;

    let resp: DescribeImagesResponse = quick_xml::de::from_str(xml).unwrap();
    assert!(resp.images_set.item.is_empty());
}

// =========================================================================
// extract_tags
// =========================================================================

#[test]
fn test_extract_tags_name_and_values() {
    let tags = vec![
        Ec2Tag {
            key: "Name".to_string(),
            value: "web-01".to_string(),
        },
        Ec2Tag {
            key: "Environment".to_string(),
            value: "prod".to_string(),
        },
        Ec2Tag {
            key: "Team".to_string(),
            value: "backend".to_string(),
        },
    ];
    let (name, extracted) = extract_tags(&tags);
    assert_eq!(name, "web-01");
    assert_eq!(extracted, vec!["backend", "prod"]); // sorted
}

#[test]
fn test_extract_tags_filters_aws_prefix() {
    let tags = vec![
        Ec2Tag {
            key: "Name".to_string(),
            value: "srv".to_string(),
        },
        Ec2Tag {
            key: "aws:cloudformation:stack-name".to_string(),
            value: "my-stack".to_string(),
        },
        Ec2Tag {
            key: "aws:autoscaling:groupName".to_string(),
            value: "my-asg".to_string(),
        },
        Ec2Tag {
            key: "custom".to_string(),
            value: "val".to_string(),
        },
    ];
    let (name, extracted) = extract_tags(&tags);
    assert_eq!(name, "srv");
    assert_eq!(extracted, vec!["val"]);
}

#[test]
fn test_extract_tags_no_name() {
    let tags = vec![Ec2Tag {
        key: "Environment".to_string(),
        value: "dev".to_string(),
    }];
    let (name, extracted) = extract_tags(&tags);
    assert!(name.is_empty());
    assert_eq!(extracted, vec!["dev"]);
}

#[test]
fn test_extract_tags_empty_value_skipped() {
    let tags = vec![Ec2Tag {
        key: "flag".to_string(),
        value: "".to_string(),
    }];
    let (_, extracted) = extract_tags(&tags);
    assert!(extracted.is_empty());
}

#[test]
fn test_extract_tags_empty() {
    let (name, tags) = extract_tags(&[]);
    assert!(name.is_empty());
    assert!(tags.is_empty());
}

// =========================================================================
// AWS_REGIONS constant
// =========================================================================

#[test]
fn test_aws_regions_not_empty() {
    assert!(AWS_REGIONS.len() >= 20);
}

#[test]
fn test_aws_region_groups_cover_all_regions() {
    let total: usize = AWS_REGION_GROUPS.iter().map(|&(_, s, e)| e - s).sum();
    assert_eq!(total, AWS_REGIONS.len());
    // Verify groups are contiguous and non-overlapping
    let mut expected_start = 0;
    for &(_, start, end) in AWS_REGION_GROUPS {
        assert_eq!(start, expected_start, "Gap or overlap in region groups");
        assert!(end > start, "Empty region group");
        expected_start = end;
    }
    assert_eq!(expected_start, AWS_REGIONS.len());
}

#[test]
fn test_aws_regions_no_duplicates() {
    let mut seen = HashSet::new();
    for (code, _) in AWS_REGIONS {
        assert!(seen.insert(code), "Duplicate region: {}", code);
    }
}

#[test]
fn test_aws_regions_contains_common() {
    let codes: Vec<&str> = AWS_REGIONS.iter().map(|(c, _)| *c).collect();
    assert!(codes.contains(&"us-east-1"));
    assert!(codes.contains(&"eu-west-1"));
    assert!(codes.contains(&"ap-northeast-1"));
}

// =========================================================================
// Provider trait
// =========================================================================

#[test]
fn test_aws_provider_name() {
    let aws = Aws {
        regions: vec![],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    assert_eq!(aws.name(), "aws");
    assert_eq!(aws.short_label(), "aws");
}

#[test]
fn test_aws_no_regions_error() {
    let aws = Aws {
        regions: vec![],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    let result = aws.fetch_hosts("fake", &crate::runtime::env::Env::empty());
    match result {
        Err(ProviderError::Http(msg)) => assert!(msg.contains("No AWS regions")),
        other => panic!("Expected Http error, got: {:?}", other),
    }
}

// =========================================================================
// param helper
// =========================================================================

#[test]
fn test_param_helper() {
    let (k, v) = param("Action", "DescribeInstances");
    assert_eq!(k, "Action");
    assert_eq!(v, "DescribeInstances");
}

// =========================================================================
// Region validation
// =========================================================================

#[test]
fn test_aws_invalid_region_error() {
    let aws = Aws {
        regions: vec!["xx-invalid-1".to_string()],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    let result = aws.fetch_hosts("AKID:SECRET", &crate::runtime::env::Env::empty());
    match result {
        Err(ProviderError::Http(msg)) => assert!(msg.contains("Unknown AWS region")),
        other => panic!("Expected Http error for invalid region, got: {:?}", other),
    }
}

#[test]
fn test_aws_mixed_valid_invalid_region_error() {
    let aws = Aws {
        regions: vec!["us-east-1".to_string(), "xx-fake-9".to_string()],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    let result = aws.fetch_hosts("AKID:SECRET", &crate::runtime::env::Env::empty());
    match result {
        Err(ProviderError::Http(msg)) => assert!(msg.contains("xx-fake-9")),
        other => panic!("Expected Http error for invalid region, got: {:?}", other),
    }
}

// =========================================================================
// Profile credential errors return AuthFailed
// =========================================================================

// =========================================================================
// AMI batch constant
// =========================================================================

#[test]
fn test_ami_batch_size_is_reasonable() {
    assert_eq!(
        AMI_BATCH_SIZE, 100,
        "AMI batch size should be 100 (AWS limit per DescribeImages call)"
    );
}

// =========================================================================
// Private IP fallback
// =========================================================================

#[test]
fn test_parse_private_ip_address() {
    let xml = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<reservationSet><item><instancesSet><item>
    <instanceId>i-priv</instanceId>
    <instanceState><name>running</name></instanceState>
    <privateIpAddress>10.0.1.5</privateIpAddress>
    <tagSet/>
</item></instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#;
    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    let inst = &resp.reservation_set.item[0].instances_set.item[0];
    assert!(inst.ip_address.is_none());
    assert_eq!(inst.private_ip_address.as_deref(), Some("10.0.1.5"));
}

#[test]
fn test_public_ip_preferred_over_private() {
    let xml = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<reservationSet><item><instancesSet><item>
    <instanceId>i-both</instanceId>
    <instanceState><name>running</name></instanceState>
    <ipAddress>54.1.2.3</ipAddress>
    <privateIpAddress>10.0.1.5</privateIpAddress>
    <tagSet/>
</item></instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#;
    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    let inst = &resp.reservation_set.item[0].instances_set.item[0];
    assert_eq!(inst.ip_address.as_deref(), Some("54.1.2.3"));
    assert_eq!(inst.private_ip_address.as_deref(), Some("10.0.1.5"));
}

#[test]
fn test_no_ip_at_all_still_parseable() {
    let xml = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
<reservationSet><item><instancesSet><item>
    <instanceId>i-noip</instanceId>
    <instanceState><name>running</name></instanceState>
    <tagSet/>
</item></instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#;
    let resp: DescribeInstancesResponse = quick_xml::de::from_str(xml).unwrap();
    let inst = &resp.reservation_set.item[0].instances_set.item[0];
    assert!(inst.ip_address.is_none());
    assert!(inst.private_ip_address.is_none());
}

// =========================================================================
// HTTP roundtrip tests (mockito)
// =========================================================================

#[test]
fn test_http_describe_instances_roundtrip() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("Action".into(), "DescribeInstances".into()),
            mockito::Matcher::UrlEncoded("Version".into(), "2016-11-15".into()),
        ]))
        .match_header("Authorization", mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(
            r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet>
<item>
  <instancesSet>
    <item>
      <instanceId>i-1234567890</instanceId>
      <instanceState><name>running</name></instanceState>
      <privateIpAddress>10.0.0.1</privateIpAddress>
      <ipAddress>54.1.2.3</ipAddress>
      <imageId>ami-12345678</imageId>
      <instanceType>t3.micro</instanceType>
      <tagSet><item><key>Name</key><value>web-1</value></item></tagSet>
    </item>
  </instancesSet>
</item>
  </reservationSet>
</DescribeInstancesResponse>"#,
        )
        .create();

    let agent = super::super::http_agent();
    let url = format!(
        "{}/?Action=DescribeInstances&Version=2016-11-15",
        server.url()
    );
    let body = agent
        .get(&url)
        .header("Authorization", "AWS4-HMAC-SHA256 Credential=fake")
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    let resp: DescribeInstancesResponse = quick_xml::de::from_str(&body).unwrap();

    assert_eq!(resp.reservation_set.item.len(), 1);
    let inst = &resp.reservation_set.item[0].instances_set.item[0];
    assert_eq!(inst.instance_id, "i-1234567890");
    assert_eq!(inst.instance_state.name, "running");
    assert_eq!(inst.ip_address.as_deref(), Some("54.1.2.3"));
    assert_eq!(inst.private_ip_address.as_deref(), Some("10.0.0.1"));
    assert_eq!(inst.image_id, "ami-12345678");
    assert_eq!(inst.instance_type, "t3.micro");
    assert_eq!(inst.tag_set.item.len(), 1);
    assert_eq!(inst.tag_set.item[0].key, "Name");
    assert_eq!(inst.tag_set.item[0].value, "web-1");
    mock.assert();
}

#[test]
fn test_http_describe_instances_auth_failure() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(401)
        .with_header("content-type", "text/xml")
        .with_body("<Error><Code>AuthFailure</Code></Error>")
        .create();

    let agent = super::super::http_agent();
    let result = agent
        .get(&format!(
            "{}/?Action=DescribeInstances&Version=2016-11-15",
            server.url()
        ))
        .header("Authorization", "AWS4-HMAC-SHA256 Credential=bad")
        .call();

    match result {
        Err(ureq::Error::StatusCode(401)) => {} // expected
        other => panic!("expected 401 error, got {:?}", other),
    }
    mock.assert();
}

#[test]
fn test_http_describe_images_roundtrip() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("Action".into(), "DescribeImages".into()),
            mockito::Matcher::UrlEncoded("Version".into(), "2016-11-15".into()),
            mockito::Matcher::UrlEncoded("ImageId.1".into(), "ami-12345678".into()),
        ]))
        .match_header("Authorization", mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(
            r#"<DescribeImagesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <imagesSet>
<item>
  <imageId>ami-12345678</imageId>
  <name>amzn2-ami-hvm-2.0</name>
</item>
  </imagesSet>
</DescribeImagesResponse>"#,
        )
        .create();

    let agent = super::super::http_agent();
    let url = format!(
        "{}/?Action=DescribeImages&Version=2016-11-15&ImageId.1=ami-12345678",
        server.url()
    );
    let body = agent
        .get(&url)
        .header("Authorization", "AWS4-HMAC-SHA256 Credential=fake")
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    let resp: DescribeImagesResponse = quick_xml::de::from_str(&body).unwrap();

    assert_eq!(resp.images_set.item.len(), 1);
    assert_eq!(resp.images_set.item[0].image_id, "ami-12345678");
    assert_eq!(resp.images_set.item[0].name, "amzn2-ami-hvm-2.0");
    mock.assert();
}

#[test]
fn test_http_describe_images_auth_failure() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(401)
        .with_header("content-type", "text/xml")
        .with_body("<Error><Code>AuthFailure</Code></Error>")
        .create();

    let agent = super::super::http_agent();
    let result = agent
        .get(&format!(
            "{}/?Action=DescribeImages&Version=2016-11-15&ImageId.1=ami-abc",
            server.url()
        ))
        .header("Authorization", "AWS4-HMAC-SHA256 Credential=bad")
        .call();

    match result {
        Err(ureq::Error::StatusCode(401)) => {} // expected
        other => panic!("expected 401 error, got {:?}", other),
    }
    mock.assert();
}

#[test]
fn fetch_from_drives_full_pipeline_against_mock() {
    // Exercises the production per-region pipeline end to end through the
    // endpoint seam: SigV4 signing, DescribeInstances + DescribeImages,
    // XML deserialize and ProviderHost mapping. Before the seam these were
    // only covered by re-issuing equivalent requests inline.
    let mut server = mockito::Server::new();
    let instances = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::UrlEncoded(
            "Action".into(),
            "DescribeInstances".into(),
        ))
        .match_header("Authorization", mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(
            r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet><item><instancesSet><item>
    <instanceId>i-1234567890</instanceId>
    <instanceState><name>running</name></instanceState>
    <privateIpAddress>10.0.0.1</privateIpAddress>
    <ipAddress>54.1.2.3</ipAddress>
    <imageId>ami-12345678</imageId>
    <instanceType>t3.micro</instanceType>
    <tagSet><item><key>Name</key><value>web-1</value></item></tagSet>
  </item></instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#,
        )
        .create();
    let images = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::UrlEncoded(
            "Action".into(),
            "DescribeImages".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(
            r#"<DescribeImagesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <imagesSet><item><imageId>ami-12345678</imageId><name>amzn2-ami-hvm-2.0</name></item></imagesSet>
</DescribeImagesResponse>"#,
        )
        .create();

    let aws = Aws {
        regions: vec!["us-east-1".to_string()],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    let url = server.url();
    let hosts = aws
        .fetch_with_endpoint(
            &Endpoints {
                ec2: &|_region: &str| url.clone(),
                ssm: &|_region: &str| url.clone(),
                sts: &|_region: &str| url.clone(),
            },
            "AKID:SECRET",
            &AtomicBool::new(false),
            &crate::runtime::env::Env::empty(),
            &|_| {},
        )
        .expect("fetch_with_endpoint must succeed against the mock");
    instances.assert();
    images.assert();

    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].server_id, "i-1234567890");
    assert_eq!(hosts[0].name, "web-1");
    assert_eq!(hosts[0].ip, "54.1.2.3");
    assert!(
        hosts[0]
            .metadata
            .contains(&("region".to_string(), "us-east-1".to_string()))
    );
    assert!(
        hosts[0]
            .metadata
            .contains(&("os".to_string(), "amzn2-ami-hvm-2.0".to_string()))
    );
}

#[test]
fn fetch_from_maps_auth_failure_to_provider_error() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(401)
        .with_header("content-type", "text/xml")
        .with_body("<Error><Code>AuthFailure</Code></Error>")
        .create();

    let aws = Aws {
        regions: vec!["us-east-1".to_string()],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    let url = server.url();
    let result = aws.fetch_with_endpoint(
        &Endpoints {
            ec2: &|_region: &str| url.clone(),
            ssm: &|_region: &str| url.clone(),
            sts: &|_region: &str| url.clone(),
        },
        "AKID:SECRET",
        &AtomicBool::new(false),
        &crate::runtime::env::Env::empty(),
        &|_| {},
    );
    mock.assert();
    assert!(
        matches!(result, Err(ProviderError::AuthFailed)),
        "a 401 from the region must surface as AuthFailed, got {result:?}"
    );
}

#[test]
fn fetch_sends_security_token_header_for_temporary_credentials() {
    // Both EC2 calls must carry x-amz-security-token, because the signature
    // declares it in SignedHeaders.
    let mut server = mockito::Server::new();
    let instances = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::UrlEncoded(
            "Action".into(),
            "DescribeInstances".into(),
        ))
        .match_header("x-amz-security-token", "TOKEN")
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(
            r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet><item><instancesSet><item>
    <instanceId>i-1234567890</instanceId>
    <instanceState><name>running</name></instanceState>
    <ipAddress>54.1.2.3</ipAddress>
    <imageId>ami-12345678</imageId>
    <tagSet><item><key>Name</key><value>web-1</value></item></tagSet>
  </item></instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#,
        )
        .create();
    let images = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::UrlEncoded(
            "Action".into(),
            "DescribeImages".into(),
        ))
        .match_header("x-amz-security-token", "TOKEN")
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(
            r#"<DescribeImagesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <imagesSet><item><imageId>ami-12345678</imageId><name>amzn2-ami-hvm-2.0</name></item></imagesSet>
</DescribeImagesResponse>"#,
        )
        .create();

    let aws = Aws {
        regions: vec!["us-east-1".to_string()],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    let url = server.url();
    let hosts = aws
        .fetch_with_endpoint(
            &Endpoints {
                ec2: &|_region: &str| url.clone(),
                ssm: &|_region: &str| url.clone(),
                sts: &|_region: &str| url.clone(),
            },
            "ASIAEXAMPLE:SECRET:TOKEN",
            &AtomicBool::new(false),
            &crate::runtime::env::Env::empty(),
            &|_| {},
        )
        .expect("a signed request carrying the session token must reach the mock");
    instances.assert();
    images.assert();
    assert_eq!(hosts.len(), 1);
}

#[test]
fn fetch_omits_security_token_header_for_static_credentials() {
    // Long-lived keys sign without the token, so the header must be absent.
    let mut server = mockito::Server::new();
    let instances = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .match_header("x-amz-security-token", mockito::Matcher::Missing)
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(
            r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet></reservationSet>
</DescribeInstancesResponse>"#,
        )
        .create();

    let aws = Aws {
        regions: vec!["us-east-1".to_string()],
        profile: String::new(),
        ssm: crate::providers::aws_ssm::SsmMode::default(),
    };
    let url = server.url();
    let hosts = aws
        .fetch_with_endpoint(
            &Endpoints {
                ec2: &|_region: &str| url.clone(),
                ssm: &|_region: &str| url.clone(),
                sts: &|_region: &str| url.clone(),
            },
            "AKID:SECRET",
            &AtomicBool::new(false),
            &crate::runtime::env::Env::empty(),
            &|_| {},
        )
        .expect("static credentials must reach the mock without a token header");
    instances.assert();
    assert!(hosts.is_empty());
}

// =========================================================================
// Session Manager routing
// =========================================================================

/// Two instances: one with a public address, one with none at all. The second
/// is the case only Session Manager can reach.
const TWO_INSTANCES_XML: &str = r#"<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet><item><instancesSet>
    <item>
      <instanceId>i-public</instanceId>
      <instanceState><name>running</name></instanceState>
      <ipAddress>54.1.2.3</ipAddress>
      <instanceType>t3.micro</instanceType>
      <tagSet><item><key>Name</key><value>web</value></item></tagSet>
    </item>
    <item>
      <instanceId>i-private</instanceId>
      <instanceState><name>running</name></instanceState>
      <instanceType>t3.micro</instanceType>
      <tagSet><item><key>Name</key><value>worker</value></item></tagSet>
    </item>
  </instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#;

fn aws_with_ssm(mode: crate::providers::aws_ssm::SsmMode, profile: &str) -> Aws {
    Aws {
        regions: vec!["us-east-1".to_string()],
        profile: profile.to_string(),
        ssm: mode,
    }
}

fn host<'a>(hosts: &'a [ProviderHost], server_id: &str) -> &'a ProviderHost {
    hosts
        .iter()
        .find(|h| h.server_id == server_id)
        .unwrap_or_else(|| panic!("no host {server_id}"))
}

/// An `Env` whose `~/.aws/credentials` holds `profile` with static keys, so a
/// config that names a profile can resolve credentials from it.
fn env_with_profile(dir: &std::path::Path, profile: &str) -> crate::runtime::env::Env {
    let aws = dir.join(".aws");
    std::fs::create_dir_all(&aws).expect("create .aws");
    std::fs::write(
        aws.join("credentials"),
        format!("[\"{profile}\"]\naws_access_key_id = AKID\naws_secret_access_key = SECRET\n"),
    )
    .expect("write credentials");
    crate::runtime::env::Env::for_test(dir)
}

/// Serve EC2 on one mock and Session Manager on another, so the two endpoints
/// are told apart the way production tells them apart.
fn ssm_fetch(
    aws: &Aws,
    ec2_body: &str,
    ssm_body: Option<&str>,
) -> Result<Vec<ProviderHost>, ProviderError> {
    let home = tempfile::tempdir().expect("tempdir");
    let env = if aws.profile.is_empty() {
        crate::runtime::env::Env::empty()
    } else {
        env_with_profile(home.path(), &aws.profile)
    };
    let mut ec2_server = mockito::Server::new();
    let _instances = ec2_server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(ec2_body)
        .create();

    let mut ssm_server = mockito::Server::new();
    let _nodes = ssm_server
        .mock("POST", "/")
        .with_status(if ssm_body.is_some() { 200 } else { 403 })
        .with_body(ssm_body.unwrap_or(
            r#"{"__type":"AccessDeniedException","message":"User: arn:aws:iam::1:user/eric is not authorized to perform: ssm:DescribeInstanceInformation on resource: *"}"#,
        ))
        .create();

    let ec2_url = ec2_server.url();
    let ssm_url = ssm_server.url();
    aws.fetch_with_endpoint(
        &Endpoints {
            ec2: &|_region: &str| ec2_url.clone(),
            ssm: &|_region: &str| ssm_url.clone(),
            sts: &|_region: &str| ec2_url.clone(),
        },
        "AKID:SECRET",
        &AtomicBool::new(false),
        &env,
        &|_| {},
    )
}

#[test]
fn ssm_off_keeps_the_ip_address_and_reports_an_instance_without_one_as_addressless() {
    // An instance with no address is reported with an empty one rather than
    // dropped, so it stays in the remote set. Dropping it would let sync read
    // a running instance as gone and `--remove` delete its host block. Same
    // contract Proxmox uses for a stopped VM.
    let hosts = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Off, ""),
        TWO_INSTANCES_XML,
        None,
    )
    .expect("fetch succeeds");
    assert_eq!(hosts.len(), 2);
    assert_eq!(host(&hosts, "i-public").ip, "54.1.2.3");
    assert!(host(&hosts, "i-public").directives.is_empty());
    assert_eq!(host(&hosts, "i-private").ip, "");
}

#[test]
fn an_addressless_instance_keeps_its_proxy_command() {
    // Turning Session Manager off cannot put this host back on an address,
    // because it has none. Withdrawing the command would leave a host that
    // reaches nothing, so purple leaves it alone.
    let hosts = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Off, ""),
        TWO_INSTANCES_XML,
        None,
    )
    .expect("fetch succeeds");
    let addressless = host(&hosts, "i-private");
    assert!(
        addressless.retract_directives.is_empty(),
        "an unreachable host must not have its only route withdrawn"
    );
    // A host that does have an address is put back on it, so the withdrawal
    // still happens where purple has something to fall back to.
    assert_eq!(host(&hosts, "i-public").retract_directives.len(), 1);
}

#[test]
fn ssm_off_still_withdraws_a_proxy_command_purple_wrote_before() {
    let hosts = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Off, ""),
        TWO_INSTANCES_XML,
        None,
    )
    .expect("fetch succeeds");
    let retract = &host(&hosts, "i-public").retract_directives;
    // Scoped to the host that has an address to return to, and claiming the
    // whole command rather than its opening: a prefix would also match the
    // line a user wrote from AWS's own documentation.
    assert_eq!(retract.len(), 1);
    assert_eq!(retract[0].key, "ProxyCommand");
    assert!(retract[0].claims(&crate::providers::aws_ssm::proxy_command("", "us-east-1")));
    // The profile is the one segment the config can change between syncs, so
    // the line purple wrote under a different profile is still its own.
    assert!(retract[0].claims(&crate::providers::aws_ssm::proxy_command(
        "org-prod",
        "us-east-1"
    )));
    // Another region's line belongs to another config.
    assert!(!retract[0].claims(&crate::providers::aws_ssm::proxy_command("", "eu-west-1")));
    // The line AWS publishes, typed by hand against a fixed instance.
    assert!(!retract[0].claims(
        "sh -c \"aws ssm start-session --target i-0abc --document-name AWS-StartSSHSession --parameters 'portNumber=%p' --region us-east-1\""
    ));
}

#[test]
fn ssm_always_routes_every_instance_without_asking_the_service() {
    // No SSM mock is reachable here, which is the point: `always` must not
    // call DescribeInstanceInformation at all.
    let hosts = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Always, ""),
        TWO_INSTANCES_XML,
        None,
    )
    .expect("fetch succeeds without an SSM lookup");
    assert_eq!(hosts.len(), 2, "the address-less instance is reachable now");
    for id in ["i-public", "i-private"] {
        let h = host(&hosts, id);
        assert_eq!(h.ip, id, "HostName must be the instance ID");
        assert_eq!(h.directives.len(), 1);
        assert_eq!(h.directives[0].0, "ProxyCommand");
        assert!(h.directives[0].1.contains("--target %h"));
        assert!(h.retract_directives.is_empty());
    }
}

#[test]
fn ssm_always_carries_the_profile_and_region_into_the_command() {
    let hosts = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Always, "org-prod"),
        TWO_INSTANCES_XML,
        None,
    )
    .expect("fetch succeeds");
    let command = &host(&hosts, "i-public").directives[0].1;
    assert!(command.contains("--profile org-prod"), "{command}");
    assert!(command.contains("--region us-east-1"), "{command}");
}

#[test]
fn ssm_auto_routes_only_the_nodes_the_service_reports_online() {
    let hosts = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Auto, ""),
        TWO_INSTANCES_XML,
        Some(r#"{"InstanceInformationList":[{"InstanceId":"i-private","PingStatus":"Online"}],"NextToken":""}"#),
    )
    .expect("fetch succeeds");
    assert_eq!(hosts.len(), 2);

    // Managed: reached by instance ID through the proxy command.
    let managed = host(&hosts, "i-private");
    assert_eq!(managed.ip, "i-private");
    assert_eq!(managed.directives.len(), 1);
    assert!(
        managed
            .metadata
            .contains(&("via".to_string(), "Session Manager".to_string()))
    );

    // Not managed: untouched, its old command withdrawn.
    let plain = host(&hosts, "i-public");
    assert_eq!(plain.ip, "54.1.2.3");
    assert!(plain.directives.is_empty());
    assert_eq!(plain.retract_directives.len(), 1);
}

#[test]
fn ssm_auto_reports_a_denied_lookup_instead_of_silently_routing_nothing() {
    // Without this the sync would look successful while every host quietly
    // stayed on its IP address, which is the failure the user cannot see.
    let result = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Auto, ""),
        TWO_INSTANCES_XML,
        None,
    );
    let err = result.expect_err("a denied lookup must fail the region");
    let msg = err.to_string();
    assert!(msg.contains("No instances"), "unexpected error: {msg}");
    // The region's own reason travels into the summary, and the assertion is
    // on a phrase only the service's body can supply: a mapping built from
    // the status alone would pass a check on purple's own static hint.
    assert!(
        msg.contains("is not authorized to perform"),
        "the body was dropped on the way to the summary: {msg}"
    );
    assert!(
        !msg.contains("API token"),
        "AWS has no API token, so the credentials must not be blamed: {msg}"
    );
    // An EC2 read-only policy does not carry this action, so naming it is the
    // whole diagnosis.
    assert!(
        msg.contains("ssm:DescribeInstanceInformation"),
        "the missing action is not named: {msg}"
    );
    assert!(
        !msg.contains("Check your credentials"),
        "one missing IAM action is not a credential problem: {msg}"
    );
}

#[test]
fn a_region_that_fails_on_ec2_carries_its_own_reason_into_the_summary() {
    // Nothing to do with Session Manager: the same rule has to hold for the
    // listing call, or the summary invents a credential problem out of an
    // unreachable service.
    let mut ec2_server = mockito::Server::new();
    let _mock = ec2_server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(503)
        .with_body("service unavailable")
        .create();

    let url = ec2_server.url();
    let err = aws_with_ssm(crate::providers::aws_ssm::SsmMode::Off, "")
        .fetch_with_endpoint(
            &Endpoints {
                ec2: &|_region: &str| url.clone(),
                ssm: &|_region: &str| url.clone(),
                sts: &|_region: &str| url.clone(),
            },
            "AKID:SECRET",
            &AtomicBool::new(false),
            &crate::runtime::env::Env::empty(),
            &|_| {},
        )
        .expect_err("a failed listing must fail the region");
    let msg = err.to_string();
    assert!(msg.contains("us-east-1"), "region not named: {msg}");
    assert!(msg.contains("503"), "the status is the reason: {msg}");
    assert!(
        !msg.contains("Check your credentials"),
        "an unreachable service is not a credential problem: {msg}"
    );
}

#[test]
fn an_unsafe_profile_name_is_refused_before_any_request() {
    // The check runs ahead of credential resolution, so an empty environment
    // is enough: the name never reaches a request.
    let aws = aws_with_ssm(
        crate::providers::aws_ssm::SsmMode::Always,
        "bad name\"; rm -rf /",
    );
    let err = aws
        .fetch_with_endpoint(
            &Endpoints {
                ec2: &|_region: &str| "http://127.0.0.1:1".to_string(),
                ssm: &|_region: &str| "http://127.0.0.1:1".to_string(),
                sts: &|_region: &str| "http://127.0.0.1:1".to_string(),
            },
            "AKID:SECRET",
            &AtomicBool::new(false),
            &crate::runtime::env::Env::empty(),
            &|_| {},
        )
        .expect_err("an unsafe profile name must fail");
    assert!(err.to_string().contains("ProxyCommand"), "got: {err}");
}

#[test]
fn an_unsafe_profile_name_is_allowed_when_session_manager_is_off() {
    // The name only has to be shell-safe because it goes into a command; with
    // Session Manager off it never does, so the sync must not be blocked.
    let result = ssm_fetch(
        &aws_with_ssm(crate::providers::aws_ssm::SsmMode::Off, "odd name"),
        TWO_INSTANCES_XML,
        None,
    );
    match result {
        Ok(_) => {}
        Err(e) => panic!("off must not validate the profile name, got: {e}"),
    }
}

// =========================================================================
// Assume role, end to end
// =========================================================================

/// The AssumeRole response shape AWS documents, with the credentials filled in.
fn assume_role_body(access: &str, secret: &str, token: &str) -> String {
    format!(
        r#"<AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleResult>
    <AssumedRoleUser>
      <Arn>arn:aws:sts::1:assumed-role/Admin/purple</Arn>
      <AssumedRoleId>AROA:purple</AssumedRoleId>
    </AssumedRoleUser>
    <Credentials>
      <AccessKeyId>{}</AccessKeyId>
      <SecretAccessKey>{}</SecretAccessKey>
      <SessionToken>{}</SessionToken>
      <Expiration>2026-01-01T00:00:00Z</Expiration>
    </Credentials>
  </AssumeRoleResult>
</AssumeRoleResponse>"#,
        access, secret, token
    )
}

/// An `Env` whose `~/.aws` describes `org-prod` assuming a role from `base`.
fn env_with_role_chain(dir: &std::path::Path) -> crate::runtime::env::Env {
    let aws = dir.join(".aws");
    std::fs::create_dir_all(&aws).expect("create .aws");
    std::fs::write(
        aws.join("config"),
        "[profile org-prod]\nrole_arn = arn:aws:iam::1:role/Admin\nsource_profile = base\n",
    )
    .expect("write config");
    std::fs::write(
        aws.join("credentials"),
        "[base]\naws_access_key_id = AKIABASE\naws_secret_access_key = BASESECRET\n",
    )
    .expect("write credentials");
    crate::runtime::env::Env::for_test(dir)
}

#[test]
fn a_role_profile_reports_the_chain_rather_than_following_it() {
    let home = tempfile::tempdir().expect("tempdir");
    let env = env_with_role_chain(home.path());
    match resolve_credentials("", "org-prod", &env).expect("the chain resolves") {
        CredentialSource::AssumeRole { base, roles } => {
            assert_eq!(base.access_key, "AKIABASE");
            assert_eq!(roles.len(), 1);
            assert_eq!(roles[0].role_arn, "arn:aws:iam::1:role/Admin");
            assert_eq!(roles[0].profile, "org-prod");
        }
        CredentialSource::Ready(_) => panic!("a role_arn profile must report its chain"),
    }
}

#[test]
fn a_nested_chain_is_reported_innermost_first() {
    // The order the caller assumes them in: the base's own role first, then
    // the one that role may take.
    let home = tempfile::tempdir().expect("tempdir");
    let aws = home.path().join(".aws");
    std::fs::create_dir_all(&aws).expect("create .aws");
    std::fs::write(
        aws.join("config"),
        "[profile outer]\nrole_arn = arn:outer\nsource_profile = inner\n\
         [profile inner]\nrole_arn = arn:inner\nsource_profile = base\n",
    )
    .expect("write config");
    std::fs::write(
        aws.join("credentials"),
        "[base]\naws_access_key_id = AKIABASE\naws_secret_access_key = BASESECRET\n",
    )
    .expect("write credentials");
    let env = crate::runtime::env::Env::for_test(home.path());
    match resolve_credentials("", "outer", &env).expect("the chain resolves") {
        CredentialSource::AssumeRole { roles, .. } => {
            let arns: Vec<&str> = roles.iter().map(|r| r.role_arn.as_str()).collect();
            assert_eq!(arns, ["arn:inner", "arn:outer"]);
        }
        CredentialSource::Ready(_) => panic!("a role_arn profile must report its chain"),
    }
}

#[test]
fn an_assumed_role_signs_the_ec2_call_with_the_credentials_it_returned() {
    // The seam between the profile chain and the API calls. Signing the
    // listing with the base key pair instead would silently sync the wrong
    // account's instances, and nothing else in the suite reaches this path.
    let home = tempfile::tempdir().expect("tempdir");
    let env = env_with_role_chain(home.path());

    let mut sts_server = mockito::Server::new();
    let assume = sts_server
        .mock("GET", "/")
        .match_query(mockito::Matcher::UrlEncoded(
            "Action".into(),
            "AssumeRole".into(),
        ))
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(assume_role_body(
            "ASIAASSUMED",
            "ASSUMEDSECRET",
            "ASSUMEDTOKEN",
        ))
        .create();

    let mut ec2_server = mockito::Server::new();
    let instances = ec2_server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .match_header(
            "authorization",
            mockito::Matcher::Regex("Credential=ASIAASSUMED/".to_string()),
        )
        .match_header("x-amz-security-token", "ASSUMEDTOKEN")
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(TWO_INSTANCES_XML)
        .create();

    let sts_url = sts_server.url();
    let ec2_url = ec2_server.url();
    let hosts = aws_with_ssm(crate::providers::aws_ssm::SsmMode::Off, "org-prod")
        .fetch_with_endpoint(
            &Endpoints {
                ec2: &|_region: &str| ec2_url.clone(),
                ssm: &|_region: &str| ec2_url.clone(),
                sts: &|_region: &str| sts_url.clone(),
            },
            "",
            &AtomicBool::new(false),
            &env,
            &|_| {},
        )
        .expect("the assumed credentials must reach EC2");

    assume.assert();
    instances.assert();
    assert_eq!(hosts.len(), 2);
}

#[test]
fn the_sts_call_is_scoped_to_the_first_configured_region() {
    // The credential scope has to match the endpoint the request went to, so
    // one regional endpoint is picked and pinned rather than resolved per
    // region alongside EC2.
    let home = tempfile::tempdir().expect("tempdir");
    let env = env_with_role_chain(home.path());

    let mut sts_server = mockito::Server::new();
    let assume = sts_server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .match_header(
            "authorization",
            mockito::Matcher::Regex("/eu-west-1/sts/aws4_request".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(assume_role_body(
            "ASIAASSUMED",
            "ASSUMEDSECRET",
            "ASSUMEDTOKEN",
        ))
        .create();

    let mut ec2_server = mockito::Server::new();
    let _instances = ec2_server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/xml")
        .with_body(TWO_INSTANCES_XML)
        .create();

    let sts_url = sts_server.url();
    let ec2_url = ec2_server.url();
    let aws = Aws {
        regions: vec!["eu-west-1".to_string(), "us-east-1".to_string()],
        profile: "org-prod".to_string(),
        ssm: crate::providers::aws_ssm::SsmMode::Off,
    };
    aws.fetch_with_endpoint(
        &Endpoints {
            ec2: &|_region: &str| ec2_url.clone(),
            ssm: &|_region: &str| ec2_url.clone(),
            sts: &|_region: &str| sts_url.clone(),
        },
        "",
        &AtomicBool::new(false),
        &env,
        &|_| {},
    )
    .expect("fetch succeeds");
    assume.assert();
}

#[test]
fn a_refused_assume_role_stops_the_sync_before_any_region_is_listed() {
    let home = tempfile::tempdir().expect("tempdir");
    let env = env_with_role_chain(home.path());

    let mut sts_server = mockito::Server::new();
    let _assume = sts_server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(403)
        .with_body("<ErrorResponse><Error><Code>AccessDenied</Code><Message>User: arn:aws:iam::1:user/eric is not authorized to perform: sts:AssumeRole</Message></Error></ErrorResponse>")
        .create();

    let sts_url = sts_server.url();
    let err = aws_with_ssm(crate::providers::aws_ssm::SsmMode::Off, "org-prod")
        .fetch_with_endpoint(
            &Endpoints {
                // Unreachable on purpose: a refused role must not get this far.
                ec2: &|_region: &str| "http://127.0.0.1:1".to_string(),
                ssm: &|_region: &str| "http://127.0.0.1:1".to_string(),
                sts: &|_region: &str| sts_url.clone(),
            },
            "",
            &AtomicBool::new(false),
            &env,
            &|_| {},
        )
        .expect_err("a refused role must fail the sync");
    let msg = err.to_string();
    assert!(msg.contains("arn:aws:iam::1:role/Admin"), "{msg}");
    assert!(msg.contains("sts:AssumeRole"), "{msg}");
}

// =========================================================================
// Profile failure wording
// =========================================================================

#[test]
fn every_profile_failure_gets_its_own_wording() {
    // One fixture per ChainError, asserted on the phrase that tells it apart.
    // Troubleshooting.md quotes some of these verbatim, and two arms swapped
    // would otherwise ship unnoticed.
    let cases: &[(&str, &str, &str, &str)] = &[
        // (profile asked for, ~/.aws/config, ~/.aws/credentials, phrase)
        ("ghost", "", "", "is in neither"),
        (
            "p",
            "[profile p]\nregion = eu-west-1\n",
            "",
            "has no credentials",
        ),
        (
            "p",
            "[profile p]\nrole_arn = arn:r\n",
            "",
            "no source_profile",
        ),
        (
            "p",
            "[profile p]\nrole_arn = arn:r\nsource_profile = b\ncredential_source = Ec2InstanceMetadata\n",
            "",
            "credential_source",
        ),
        (
            "p",
            "[profile p]\ncredential_process = /usr/bin/creds\n",
            "",
            "credential_process",
        ),
        (
            "p",
            "[profile p]\nsso_start_url = https://x.awsapps.com/start\n",
            "",
            "IAM Identity Center",
        ),
        (
            "p",
            "[profile p]\nrole_arn = arn:r\nweb_identity_token_file = /var/run/token\n",
            "",
            "web_identity_token_file",
        ),
        (
            "p",
            "[profile p]\nrole_arn = arn:r\nsource_profile = b\nmfa_serial = arn:mfa\n",
            "",
            "mfa_serial",
        ),
        (
            "a",
            "[profile a]\nrole_arn = arn:a\nsource_profile = b\n[profile b]\nrole_arn = arn:b\nsource_profile = a\n",
            "",
            "reaches itself",
        ),
        (
            "p",
            "",
            "[p]\naws_access_key_id = AKIA\n",
            "aws_secret_access_key",
        ),
    ];

    for (profile, config, credentials, phrase) in cases {
        let home = tempfile::tempdir().expect("tempdir");
        let aws = home.path().join(".aws");
        std::fs::create_dir_all(&aws).expect("create .aws");
        std::fs::write(aws.join("config"), config).expect("write config");
        std::fs::write(aws.join("credentials"), credentials).expect("write credentials");
        let env = crate::runtime::env::Env::for_test(home.path());
        let err = resolve_credentials("AKID:SECRET", profile, &env)
            .err()
            .unwrap_or_else(|| panic!("profile '{profile}' must fail for: {config}{credentials}"));
        let msg = err.to_string();
        assert!(
            msg.contains(phrase),
            "expected '{phrase}' in the message for '{profile}', got: {msg}"
        );
        assert!(
            !msg.contains("API token"),
            "a profile failure must not blame the token: {msg}"
        );
    }
}

#[test]
fn a_chain_deeper_than_the_cap_says_so() {
    let home = tempfile::tempdir().expect("tempdir");
    let aws = home.path().join(".aws");
    std::fs::create_dir_all(&aws).expect("create .aws");
    let mut config = String::new();
    for i in 0..12 {
        config.push_str(&format!(
            "[profile p{}]\nrole_arn = arn:{}\nsource_profile = p{}\n",
            i,
            i,
            i + 1
        ));
    }
    std::fs::write(aws.join("config"), config).expect("write config");
    std::fs::write(aws.join("credentials"), "").expect("write credentials");
    let env = crate::runtime::env::Env::for_test(home.path());
    let err = resolve_credentials("", "p0", &env)
        .err()
        .expect("a chain past the cap must fail");
    assert!(err.to_string().contains("too long to follow"), "{err}");
}

#[test]
fn a_credentials_file_purple_cannot_open_is_named_instead_of_the_profile() {
    // The profile is sitting in a file purple could not read, so telling the
    // user to add it sends them after something that is already there.
    let home = tempfile::tempdir().expect("tempdir");
    let aws = home.path().join(".aws");
    std::fs::create_dir_all(&aws).expect("create .aws");
    std::fs::create_dir(aws.join("credentials")).expect("a directory where a file is expected");
    let env = crate::runtime::env::Env::for_test(home.path());
    let err = resolve_credentials("AKID:SECRET", "prod", &env)
        .err()
        .expect("an unreadable file must fail");
    let msg = err.to_string();
    assert!(msg.contains("Can't read"), "{msg}");
    assert!(msg.contains("credentials"), "the path is not named: {msg}");
}

#[test]
fn every_profile_failure_gets_its_own_picker_note() {
    // The row a user reads before picking. The full sentence has its own
    // table above; this pins the few words that stand in for it, so two arms
    // swapped cannot ship as "source_profile chain does not end" on a profile
    // that simply wants an MFA code.
    use crate::providers::aws_profile::ChainError;
    let name = || "p".to_string();
    let cases: &[(ChainError, &str)] = &[
        (
            ChainError::SsoNotSupported(name()),
            crate::messages::PROFILE_NOTE_SSO,
        ),
        (
            ChainError::WebIdentity(name()),
            crate::messages::PROFILE_NOTE_WEB_IDENTITY,
        ),
        (
            ChainError::CredentialProcess(name()),
            crate::messages::PROFILE_NOTE_CREDENTIAL_PROCESS,
        ),
        (
            ChainError::CredentialSource(name()),
            crate::messages::PROFILE_NOTE_CREDENTIAL_SOURCE,
        ),
        (
            ChainError::MfaRequired(name()),
            crate::messages::PROFILE_NOTE_MFA,
        ),
        (
            ChainError::Loop(name()),
            crate::messages::PROFILE_NOTE_CHAIN,
        ),
        (
            ChainError::TooDeep(name()),
            crate::messages::PROFILE_NOTE_CHAIN,
        ),
        (
            ChainError::RoleWithoutSource(name()),
            crate::messages::PROFILE_NOTE_NO_SOURCE,
        ),
        (
            ChainError::Missing(name()),
            crate::messages::PROFILE_NOTE_MISSING_SOURCE,
        ),
        (
            ChainError::FileUnreadable("/tmp/creds".to_string()),
            crate::messages::PROFILE_NOTE_UNREADABLE,
        ),
        (
            ChainError::NoKeys(name()),
            crate::messages::PROFILE_NOTE_NO_KEYS,
        ),
        (
            ChainError::PartialCredentials(name(), "aws_secret_access_key"),
            crate::messages::PROFILE_NOTE_NO_KEYS,
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(chain_error_note(error), *expected, "for {error:?}");
    }
    // Every note is short enough to sit behind a name on one picker row.
    let mut seen: Vec<&str> = cases.iter().map(|(_, note)| *note).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 10, "two variants share a note by accident");
}

#[test]
fn a_relocated_profile_file_is_named_in_the_message_that_sends_you_to_it() {
    // AWS_CONFIG_FILE and AWS_SHARED_CREDENTIALS_FILE move the files, so a
    // message naming ~/.aws would send the user to one purple never read.
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("conf");
    let credentials = dir.path().join("creds");
    std::fs::write(&config, "[profile other]\nregion = eu-west-1\n").expect("write config");
    std::fs::write(&credentials, "").expect("write credentials");
    let env = crate::runtime::env::Env::for_test(dir.path())
        .with_var("AWS_CONFIG_FILE", config.display().to_string())
        .with_var(
            "AWS_SHARED_CREDENTIALS_FILE",
            credentials.display().to_string(),
        );

    let err = resolve_credentials("AKID:SECRET", "ghost", &env)
        .err()
        .expect("a missing profile must fail");
    let msg = err.to_string();
    assert!(
        msg.contains(&config.display().to_string()),
        "the relocated config file is not named: {msg}"
    );
    assert!(
        !msg.contains("~/.aws"),
        "a file purple never read is named: {msg}"
    );
}

#[test]
fn the_default_files_are_named_when_nothing_relocates_them() {
    let home = tempfile::tempdir().expect("tempdir");
    let env = crate::runtime::env::Env::for_test(home.path());
    let err = resolve_credentials("AKID:SECRET", "ghost", &env)
        .err()
        .expect("a missing profile must fail");
    let msg = err.to_string();
    assert!(msg.contains(".aws"), "{msg}");
    assert!(msg.contains("config"), "{msg}");
    assert!(msg.contains("credentials"), "{msg}");
}
