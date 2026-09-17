use super::*;

fn creds() -> AwsCredentials {
    AwsCredentials {
        access_key: "AKIDEXAMPLE".to_string(),
        secret_key: "SECRET".to_string(),
        session_token: None,
    }
}

fn node_page(ids: &[&str], next: Option<&str>) -> String {
    let list: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{"InstanceId":"{}","PingStatus":"Online","PlatformType":"Linux","AgentVersion":"3.2.1.0","ResourceType":"EC2Instance"}}"#,
                id
            )
        })
        .collect();
    match next {
        Some(token) => format!(
            r#"{{"InstanceInformationList":[{}],"NextToken":"{}"}}"#,
            list.join(","),
            token
        ),
        None => format!(
            r#"{{"InstanceInformationList":[{}],"NextToken":""}}"#,
            list.join(",")
        ),
    }
}

// =========================================================================
// SsmMode
// =========================================================================

#[test]
fn the_default_mode_is_off() {
    assert_eq!(SsmMode::default(), SsmMode::Off);
    assert!(!SsmMode::default().is_enabled());
}

#[test]
fn mode_names_round_trip() {
    for mode in SsmMode::ALL {
        assert_eq!(mode.as_str().parse::<SsmMode>(), Ok(*mode));
    }
}

#[test]
fn an_unknown_mode_reads_as_off_rather_than_failing_the_load() {
    assert_eq!("".parse::<SsmMode>(), Ok(SsmMode::Off));
    assert_eq!("nonsense".parse::<SsmMode>(), Ok(SsmMode::Off));
}

#[test]
fn boolean_spellings_mean_always() {
    for spelling in ["true", "yes", "on", "ALWAYS", " Always "] {
        assert_eq!(
            spelling.parse::<SsmMode>(),
            Ok(SsmMode::Always),
            "{spelling}"
        );
    }
}

#[test]
fn the_cycle_visits_every_mode_and_returns() {
    let mut mode = SsmMode::Off;
    let mut seen = vec![mode];
    for _ in 0..SsmMode::ALL.len() - 1 {
        mode = mode.next();
        seen.push(mode);
    }
    assert_eq!(seen, SsmMode::ALL.to_vec());
    assert_eq!(mode.next(), SsmMode::Off);
}

#[test]
fn only_off_is_disabled() {
    assert!(!SsmMode::Off.is_enabled());
    assert!(SsmMode::Auto.is_enabled());
    assert!(SsmMode::Always.is_enabled());
}

// =========================================================================
// Proxy command
// =========================================================================

#[test]
fn the_proxy_command_matches_the_documented_line() {
    // AWS publishes this line verbatim, minus the profile and region purple
    // appends. Keeping it recognizable matters more than shortening it.
    assert_eq!(
        proxy_command("", ""),
        "sh -c \"aws ssm start-session --target %h --document-name AWS-StartSSHSession --parameters 'portNumber=%p'\""
    );
}

#[test]
fn the_region_is_appended_so_the_shell_default_cannot_win() {
    assert!(proxy_command("", "eu-west-1").ends_with("--region eu-west-1\""));
}

#[test]
fn the_profile_is_appended_when_the_config_names_one() {
    let command = proxy_command("org-prod", "eu-west-1");
    assert!(command.contains("--profile org-prod"), "{command}");
    assert!(command.contains("--region eu-west-1"), "{command}");
}

#[test]
fn the_proxy_command_keeps_the_ssh_tokens_unexpanded() {
    // %h is the resolved HostName and %p the resolved Port. Substituting
    // either at write time would pin the wrong value.
    let command = proxy_command("p", "eu-west-1");
    assert!(command.contains("--target %h"), "{command}");
    assert!(command.contains("portNumber=%p"), "{command}");
}

#[test]
fn the_ssh_document_is_the_session_one_not_the_port_forwarding_one() {
    assert!(proxy_command("", "").contains("AWS-StartSSHSession"));
    assert!(!proxy_command("", "").contains("PortForwarding"));
}

// =========================================================================
// Profile-name safety
// =========================================================================

#[test]
fn an_ordinary_profile_name_is_safe() {
    for name in ["prod", "org-prod", "team_1", "a.b"] {
        assert!(is_safe_profile_name(name), "{name}");
    }
}

#[test]
fn a_name_that_could_break_out_of_the_command_is_refused() {
    for name in [
        "a\"b",
        "a b",
        "a;rm -rf /",
        "a'b",
        "a$b",
        "a`b`",
        "a\nb",
        "a|b",
        "",
    ] {
        assert!(!is_safe_profile_name(name), "{name:?} must be refused");
    }
}

// =========================================================================
// Endpoints
// =========================================================================

#[test]
fn the_endpoint_is_regional() {
    assert_eq!(
        region_endpoint("eu-west-1"),
        "https://ssm.eu-west-1.amazonaws.com"
    );
}

// =========================================================================
// Request body
// =========================================================================

#[test]
fn the_first_page_asks_for_online_nodes_at_the_maximum_page_size() {
    // The literal, not the constant: DescribeInstanceInformation accepts 5 to
    // 50, so a wrong constant has to fail here rather than against AWS.
    let body = request_body(None);
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(parsed["MaxResults"], 50);
    assert_eq!(parsed["Filters"][0]["Key"], "PingStatus");
    assert_eq!(parsed["Filters"][0]["Values"][0], "Online");
    assert!(parsed.get("NextToken").is_none());
}

#[test]
fn a_later_page_carries_the_token() {
    let body = request_body(Some("abc123"));
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(parsed["NextToken"], "abc123");
}

#[test]
fn a_token_with_json_metacharacters_is_escaped() {
    let body = request_body(Some("a\"b\\c"));
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(parsed["NextToken"], "a\"b\\c");
}

#[test]
fn a_token_with_a_control_character_is_escaped() {
    let body = request_body(Some("a\nb\tc"));
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(parsed["NextToken"], "a\nb\tc");
}

// =========================================================================
// HTTP roundtrip
// =========================================================================

#[test]
fn online_nodes_signs_a_json_post_and_collects_the_ids() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/")
        .match_header("x-amz-target", SSM_TARGET_DESCRIBE)
        .match_header("content-type", SSM_CONTENT_TYPE)
        .match_header(
            "Authorization",
            mockito::Matcher::Regex("content-type;host;x-amz-date;x-amz-target".into()),
        )
        .with_status(200)
        .with_body(node_page(&["i-aaa", "i-bbb"], None))
        .create();

    let ids = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("lookup succeeds");

    mock.assert();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains("i-aaa"));
    assert!(ids.contains("i-bbb"));
}

#[test]
fn the_signature_scopes_to_the_ssm_service() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/")
        .match_header(
            "Authorization",
            mockito::Matcher::Regex("/eu-west-1/ssm/aws4_request".into()),
        )
        .with_status(200)
        .with_body(node_page(&[], None))
        .create();

    online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("lookup succeeds");
    mock.assert();
}

#[test]
fn paging_follows_a_non_empty_token_and_stops_on_an_empty_one() {
    let mut server = mockito::Server::new();
    let first = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({"MaxResults": SSM_MAX_RESULTS}),
        ))
        .with_status(200)
        .with_body(node_page(&["i-aaa"], Some("page2")))
        .create();
    let second = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJson(
            serde_json::json!({"NextToken": "page2"}),
        ))
        .with_status(200)
        .with_body(node_page(&["i-bbb"], None))
        .create();

    let ids = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("lookup succeeds");

    first.assert();
    second.assert();
    assert_eq!(ids.len(), 2);
}

#[test]
fn a_node_that_is_not_online_is_left_out() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            r#"{"InstanceInformationList":[
                {"InstanceId":"i-up","PingStatus":"Online"},
                {"InstanceId":"i-lost","PingStatus":"ConnectionLost"},
                {"InstanceId":"i-old","PingStatus":"Inactive"}
            ],"NextToken":""}"#,
        )
        .create();

    let ids = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("lookup succeeds");
    assert_eq!(ids.len(), 1);
    assert!(ids.contains("i-up"));
}

#[test]
fn a_node_missing_its_id_or_status_is_skipped() {
    // Every field of InstanceInformation is optional in the API model.
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(
            r#"{"InstanceInformationList":[
                {"PingStatus":"Online"},
                {"InstanceId":"i-nostatus"},
                {"InstanceId":"i-fine","PingStatus":"Online"}
            ],"NextToken":""}"#,
        )
        .create();

    let ids = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("lookup succeeds");
    assert_eq!(ids.len(), 1);
    assert!(ids.contains("i-fine"));
}

#[test]
fn an_empty_fleet_is_an_empty_set_not_an_error() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(r#"{"InstanceInformationList":[]}"#)
        .create();

    let ids = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("an empty fleet is not a failure");
    assert!(ids.is_empty());
}

#[test]
fn a_hybrid_managed_node_is_kept() {
    // mi- nodes take a session the same way i- nodes do.
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_body(node_page(&["mi-0123456789abcdef0"], None))
        .create();

    let ids = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("lookup succeeds");
    assert!(ids.contains("mi-0123456789abcdef0"));
}

#[test]
fn a_denied_lookup_carries_the_action_the_caller_may_not_perform() {
    // An EC2 read-only policy does not include ssm:DescribeInstanceInformation,
    // so this is the common denial. AWS names the action in the body, which
    // only survives because the status is taken as data.
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(400)
        .with_body(
            r#"{"__type":"com.amazon.ssm#AccessDeniedException","message":"User: arn:aws:iam::1:user/eric is not authorized to perform: ssm:DescribeInstanceInformation on resource: *"}"#,
        )
        .create();

    let err = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect_err("a denial must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("AccessDeniedException"),
        "code not carried: {msg}"
    );
    assert!(
        msg.contains("ssm:DescribeInstanceInformation"),
        "the action the caller lacks was dropped: {msg}"
    );
    assert!(
        !msg.contains("API token"),
        "AWS has no API token, so the credentials must not be blamed: {msg}"
    );
}

#[test]
fn a_denial_with_no_body_still_reports_its_status() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(403)
        .with_body("")
        .create();

    let err = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect_err("403 must fail");
    assert!(err.to_string().contains("403"), "status lost: {err}");
}

#[test]
fn a_throttled_lookup_is_reported_as_rate_limiting() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(429)
        .with_body(r#"{"__type":"ThrottlingException"}"#)
        .create();

    let err = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect_err("429 must fail");
    assert!(matches!(err, ProviderError::RateLimited));
}

#[test]
fn a_body_that_is_not_json_is_a_parse_error_naming_the_region() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_body("<html>nope</html>")
        .create();

    let err = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect_err("a non-JSON body must fail");
    assert!(err.to_string().contains("eu-west-1"), "{err}");
}

#[test]
fn a_cancelled_sync_stops_before_the_first_call() {
    let err = online_nodes_with_endpoint(
        &super::super::http_agent(),
        &creds(),
        "eu-west-1",
        &AtomicBool::new(true),
        "http://127.0.0.1:1",
    )
    .expect_err("cancel must stop it");
    assert!(matches!(err, ProviderError::Cancelled));
}

#[test]
fn temporary_credentials_send_their_session_token() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/")
        .match_header("x-amz-security-token", "SESSION")
        .with_status(200)
        .with_body(node_page(&[], None))
        .create();

    let temp = AwsCredentials {
        access_key: "ASIA".to_string(),
        secret_key: "SECRET".to_string(),
        session_token: Some("SESSION".to_string()),
    };
    online_nodes_with_endpoint(
        &super::super::http_agent(),
        &temp,
        "eu-west-1",
        &AtomicBool::new(false),
        &server.url(),
    )
    .expect("lookup succeeds");
    mock.assert();
}

#[test]
fn an_enormous_error_message_is_cut_to_the_shared_cap() {
    // The detail travels into a toast by way of the region summary, so a body
    // that is hostile or simply huge must not fill it.
    let long = "y".repeat(super::super::aws_sts::MAX_ERROR_MESSAGE * 3);
    let body = format!(
        r#"{{"__type":"AccessDeniedException","message":"{}"}}"#,
        long
    );
    let parsed = parse_error(&body).expect("error parsed");
    assert_eq!(
        parsed.chars().count(),
        super::super::aws_sts::MAX_ERROR_MESSAGE
    );
}

#[test]
fn a_body_without_a_type_is_not_an_error_detail() {
    assert_eq!(parse_error(r#"{"message":"something"}"#), None);
}

#[test]
fn an_empty_type_is_not_an_error_detail() {
    // Splitting on the shape-id separator can leave an empty code, which
    // would render as an empty sentence in the middle of the region message.
    assert_eq!(parse_error(r##"{"__type":"","message":"x"}"##), None);
    assert_eq!(parse_error(r##"{"__type":"#","message":"x"}"##), None);
    assert_eq!(
        parse_error(r##"{"__type":"com.amazon.ssm#","message":"x"}"##),
        None
    );
}

// =========================================================================
// Recognizing purple's own proxy command
// =========================================================================

#[test]
fn a_command_purple_generated_is_recognized_whatever_profile_it_named() {
    // The profile is the one part the config decides, so a host still
    // carrying the line written under an earlier profile is still purple's.
    for profile in ["", "org-prod", "team_1", "a.b-c"] {
        let written = proxy_command(profile, "eu-west-1");
        assert!(
            is_generated_proxy_command(&written, "eu-west-1"),
            "not recognized for profile {profile:?}: {written}"
        );
    }
}

#[test]
fn another_regions_command_belongs_to_another_config() {
    let written = proxy_command("p", "us-east-1");
    assert!(!is_generated_proxy_command(&written, "eu-west-1"));
}

#[test]
fn the_line_aws_publishes_typed_by_hand_is_not_purples() {
    // The regression the whole rule exists for. AWS documents this command,
    // so a user who set Session Manager up themselves has the same opening.
    for by_hand in [
        // A fixed instance id rather than the %h token.
        "sh -c \"aws ssm start-session --target i-0abc --document-name AWS-StartSSHSession --parameters 'portNumber=%p' --region eu-west-1\"",
        // The port-forwarding document instead of the SSH one.
        "sh -c \"aws ssm start-session --target %h --document-name AWS-StartPortForwardingSession --parameters 'portNumber=%p' --region eu-west-1\"",
        // An extra flag purple does not write.
        "sh -c \"aws ssm start-session --target %h --document-name AWS-StartSSHSession --parameters 'portNumber=%p' --endpoint-url https://x --region eu-west-1\"",
        // No shell wrapper.
        "aws ssm start-session --target %h --document-name AWS-StartSSHSession --parameters 'portNumber=%p' --region eu-west-1",
        // Something else entirely.
        "ssh -W %h:%p bastion",
    ] {
        assert!(
            !is_generated_proxy_command(by_hand, "eu-west-1"),
            "a hand-written command was claimed: {by_hand}"
        );
    }
}

#[test]
fn a_profile_segment_purple_would_not_write_is_not_claimed() {
    // The name has to be one purple would have put there, so a line carrying
    // something it refuses to write is somebody else's.
    let head = "sh -c \"aws ssm start-session --target %h --document-name AWS-StartSSHSession --parameters 'portNumber=%p'";
    for middle in [
        " --profile a b",
        " --profile a\"b",
        " --profile ",
        " --region x",
    ] {
        let value = format!("{head}{middle} --region eu-west-1\"");
        assert!(
            !is_generated_proxy_command(&value, "eu-west-1"),
            "claimed: {value}"
        );
    }
}
