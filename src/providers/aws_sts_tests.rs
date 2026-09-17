use super::*;

use std::sync::atomic::AtomicBool;

/// Unwrap the error side without printing the Ok side. `AwsCredentials` has no
/// `Debug` on purpose, so `expect_err` would leak a secret into a panic if it
/// ever compiled.
fn err_of(result: Result<AwsCredentials, ProviderError>, what: &str) -> ProviderError {
    match result {
        Ok(_) => panic!("{}", what),
        Err(e) => e,
    }
}

fn base_creds() -> AwsCredentials {
    AwsCredentials {
        access_key: "AKIDEXAMPLE".to_string(),
        secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
        session_token: None,
    }
}

fn temp_creds() -> AwsCredentials {
    AwsCredentials {
        access_key: "ASIAEXAMPLE".to_string(),
        secret_key: "SECRET".to_string(),
        session_token: Some("SESSION".to_string()),
    }
}

fn step(role_arn: &str) -> RoleStep {
    RoleStep {
        profile: "prod".to_string(),
        role_arn: role_arn.to_string(),
        role_session_name: String::new(),
        external_id: String::new(),
        duration_seconds: String::new(),
    }
}

/// A success body in the shape AWS documents for AssumeRole.
fn success_body(access: &str, secret: &str, token: &str) -> String {
    format!(
        r#"<AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleResult>
    <AssumedRoleUser>
      <Arn>arn:aws:sts::123456789012:assumed-role/demo/purple</Arn>
      <AssumedRoleId>ARO123EXAMPLE123:purple</AssumedRoleId>
    </AssumedRoleUser>
    <Credentials>
      <AccessKeyId>{}</AccessKeyId>
      <SecretAccessKey>{}</SecretAccessKey>
      <SessionToken>{}</SessionToken>
      <Expiration>2026-11-09T13:34:41Z</Expiration>
    </Credentials>
    <PackedPolicySize>6</PackedPolicySize>
  </AssumeRoleResult>
  <ResponseMetadata>
    <RequestId>c6104cbe-af31-11e0-8154-cbc7ccf896c7</RequestId>
  </ResponseMetadata>
</AssumeRoleResponse>"#,
        access, secret, token
    )
}

// =========================================================================
// Endpoints
// =========================================================================

#[test]
fn the_endpoint_is_regional_so_the_scope_region_is_the_request_region() {
    assert_eq!(
        region_endpoint("eu-west-1"),
        "https://sts.eu-west-1.amazonaws.com"
    );
    assert_eq!(
        region_endpoint("us-east-1"),
        "https://sts.us-east-1.amazonaws.com"
    );
}

#[test]
fn govcloud_uses_the_commercial_suffix() {
    assert_eq!(
        region_endpoint("us-gov-west-1"),
        "https://sts.us-gov-west-1.amazonaws.com"
    );
}

// =========================================================================
// Session name
// =========================================================================

#[test]
fn an_absent_session_name_falls_back_to_the_default() {
    assert_eq!(session_name(&step("arn:r")), DEFAULT_ROLE_SESSION_NAME);
}

#[test]
fn a_valid_session_name_is_used_as_written() {
    let mut s = step("arn:r");
    s.role_session_name = "ci-build_7".to_string();
    assert_eq!(session_name(&s), "ci-build_7");
}

#[test]
fn an_illegal_session_name_is_replaced_rather_than_failing_the_sync() {
    let mut s = step("arn:r");
    s.role_session_name = "has spaces".to_string();
    assert_eq!(session_name(&s), DEFAULT_ROLE_SESSION_NAME);
}

#[test]
fn session_name_length_bounds_match_the_service() {
    assert!(!is_valid_session_name("a"));
    assert!(is_valid_session_name("ab"));
    assert!(is_valid_session_name(&"a".repeat(64)));
    assert!(!is_valid_session_name(&"a".repeat(65)));
    assert!(!is_valid_session_name(""));
}

#[test]
fn session_name_charset_matches_the_service() {
    assert!(is_valid_session_name("A_z0+=,.@-"));
    assert!(!is_valid_session_name("bad/slash"));
    assert!(!is_valid_session_name("bad:colon"));
}

// =========================================================================
// Duration
// =========================================================================

#[test]
fn an_absent_duration_lets_the_service_default_apply() {
    assert_eq!(duration_seconds(&step("arn:r"), false), None);
}

#[test]
fn a_duration_inside_the_range_is_sent_as_written() {
    let mut s = step("arn:r");
    s.duration_seconds = "7200".to_string();
    assert_eq!(duration_seconds(&s, false), Some(7200));
}

#[test]
fn a_duration_outside_the_range_is_dropped() {
    let mut s = step("arn:r");
    s.duration_seconds = "60".to_string();
    assert_eq!(duration_seconds(&s, false), None);
    s.duration_seconds = "99999".to_string();
    assert_eq!(duration_seconds(&s, false), None);
}

#[test]
fn a_non_numeric_duration_is_dropped() {
    let mut s = step("arn:r");
    s.duration_seconds = "an hour".to_string();
    assert_eq!(duration_seconds(&s, false), None);
}

#[test]
fn chaining_caps_the_duration_at_one_hour() {
    // The literal, not the constant: AWS refuses a chained AssumeRole above
    // 3600 whatever the role's own maximum allows, so a wrong constant has to
    // fail here rather than only against the live service.
    let mut s = step("arn:r");
    s.duration_seconds = "43200".to_string();
    assert_eq!(duration_seconds(&s, true), Some(3600));
}

#[test]
fn chaining_leaves_a_shorter_duration_alone() {
    let mut s = step("arn:r");
    s.duration_seconds = "1800".to_string();
    assert_eq!(duration_seconds(&s, true), Some(1800));
}

// =========================================================================
// Response parsing
// =========================================================================

#[test]
fn credentials_are_read_out_of_a_documented_response() {
    let creds = parse_credentials(&success_body("ASIANEW", "NEWSECRET", "NEWTOKEN"))
        .expect("credentials present");
    assert_eq!(creds.access_key, "ASIANEW");
    assert_eq!(creds.secret_key, "NEWSECRET");
    assert_eq!(creds.session_token.as_deref(), Some("NEWTOKEN"));
}

#[test]
fn credentials_are_matched_by_name_not_position() {
    // GetSessionToken documents a different child order, so the parser must
    // not depend on it.
    let body = r#"<AssumeRoleResponse><AssumeRoleResult><Credentials>
      <SessionToken>TOK</SessionToken>
      <SecretAccessKey>SEC</SecretAccessKey>
      <Expiration>2026-01-01T00:00:00Z</Expiration>
      <AccessKeyId>AKI</AccessKeyId>
    </Credentials></AssumeRoleResult></AssumeRoleResponse>"#;
    let creds = parse_credentials(body).expect("credentials present");
    assert_eq!(creds.access_key, "AKI");
    assert_eq!(creds.secret_key, "SEC");
    assert_eq!(creds.session_token.as_deref(), Some("TOK"));
}

#[test]
fn a_wrapped_session_token_is_put_back_together() {
    // A session token is base64 and never holds whitespace, so the pieces
    // join straight up. A space where the wrap was would be signed into the
    // request and AWS would refuse it.
    let body = "<Credentials><AccessKeyId>AKI</AccessKeyId>\
        <SecretAccessKey>SEC</SecretAccessKey>\
        <SessionToken>AAA\n      BBB</SessionToken></Credentials>";
    let creds = parse_credentials(body).expect("credentials present");
    assert_eq!(creds.session_token.as_deref(), Some("AAABBB"));
}

#[test]
fn a_response_missing_a_field_is_not_credentials() {
    let body = "<Credentials><AccessKeyId>AKI</AccessKeyId></Credentials>";
    assert!(parse_credentials(body).is_none());
}

#[test]
fn an_empty_field_is_not_credentials() {
    let body = "<Credentials><AccessKeyId></AccessKeyId>\
        <SecretAccessKey>SEC</SecretAccessKey>\
        <SessionToken>TOK</SessionToken></Credentials>";
    assert!(parse_credentials(body).is_none());
}

#[test]
fn unknown_fields_do_not_break_parsing() {
    // PackedPolicySize is deprecated and SessionTokenSize is newer, so the
    // parser must tolerate both presence and absence.
    let body = "<Credentials><AccessKeyId>AKI</AccessKeyId>\
        <SecretAccessKey>SEC</SecretAccessKey>\
        <SessionToken>TOK</SessionToken>\
        <Expiration>2026-01-01T00:00:00Z</Expiration>\
        <SessionTokenSize>42</SessionTokenSize></Credentials>";
    assert!(parse_credentials(body).is_some());
}

// =========================================================================
// Error parsing
// =========================================================================

#[test]
fn an_error_body_yields_code_and_message() {
    let body = r#"<ErrorResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <Error>
    <Type>Sender</Type>
    <Code>AccessDenied</Code>
    <Message>User is not authorized to perform sts:AssumeRole</Message>
  </Error>
  <RequestId>abc</RequestId>
</ErrorResponse>"#;
    assert_eq!(
        parse_error(body).as_deref(),
        Some("AccessDenied: User is not authorized to perform sts:AssumeRole")
    );
}

#[test]
fn an_error_without_a_message_still_yields_the_code() {
    let body = "<ErrorResponse><Error><Code>ExpiredTokenException</Code></Error></ErrorResponse>";
    assert_eq!(parse_error(body).as_deref(), Some("ExpiredTokenException"));
}

#[test]
fn a_success_body_is_not_an_error() {
    assert!(parse_error(&success_body("A", "B", "C")).is_none());
}

#[test]
fn a_very_long_error_message_is_cut() {
    let body = format!(
        "<Error><Code>X</Code><Message>{}</Message></Error>",
        "y".repeat(MAX_ERROR_MESSAGE * 2)
    );
    let parsed = parse_error(&body).expect("error parsed");
    assert_eq!(parsed.chars().count(), MAX_ERROR_MESSAGE);
}

#[test]
fn an_error_message_wrapped_across_lines_is_collapsed() {
    let body = "<Error><Code>X</Code><Message>one\n    two</Message></Error>";
    assert_eq!(parse_error(body).as_deref(), Some("X: one two"));
}

// =========================================================================
// HTTP roundtrip
// =========================================================================

#[test]
fn assume_chain_signs_and_returns_the_new_credentials() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("Action".into(), "AssumeRole".into()),
            mockito::Matcher::UrlEncoded("Version".into(), STS_API_VERSION.into()),
            mockito::Matcher::UrlEncoded("RoleArn".into(), "arn:aws:iam::1:role/Admin".into()),
            mockito::Matcher::UrlEncoded(
                "RoleSessionName".into(),
                DEFAULT_ROLE_SESSION_NAME.into(),
            ),
        ]))
        .match_header(
            "Authorization",
            mockito::Matcher::Regex("sts/aws4_request".into()),
        )
        .with_status(200)
        .with_body(success_body("ASIANEW", "NEWSECRET", "NEWTOKEN"))
        .create();

    let url = server.url();
    let creds = assume_chain_with_endpoint(
        &super::super::http_agent(),
        base_creds(),
        &[step("arn:aws:iam::1:role/Admin")],
        "eu-west-1",
        &AtomicBool::new(false),
        |_| url.clone(),
    )
    .expect("assume succeeds");

    mock.assert();
    assert_eq!(creds.access_key, "ASIANEW");
    assert_eq!(creds.session_token.as_deref(), Some("NEWTOKEN"));
}

#[test]
fn an_empty_chain_returns_the_base_untouched() {
    let creds = assume_chain_with_endpoint(
        &super::super::http_agent(),
        base_creds(),
        &[],
        "eu-west-1",
        &AtomicBool::new(false),
        |_| "http://127.0.0.1:1".to_string(),
    )
    .expect("no roles means no call");
    assert_eq!(creds.access_key, "AKIDEXAMPLE");
}

#[test]
fn a_temporary_base_sends_its_session_token() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .match_header("x-amz-security-token", "SESSION")
        .with_status(200)
        .with_body(success_body("ASIANEW", "NEWSECRET", "NEWTOKEN"))
        .create();

    let url = server.url();
    assume_chain_with_endpoint(
        &super::super::http_agent(),
        temp_creds(),
        &[step("arn:r")],
        "eu-west-1",
        &AtomicBool::new(false),
        |_| url.clone(),
    )
    .expect("assume succeeds");
    mock.assert();
}

#[test]
fn a_two_step_chain_makes_two_calls_and_keeps_the_last_credentials() {
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body(success_body("ASIAFINAL", "FINALSECRET", "FINALTOKEN"))
        .expect(2)
        .create();

    let url = server.url();
    let creds = assume_chain_with_endpoint(
        &super::super::http_agent(),
        base_creds(),
        &[step("arn:inner"), step("arn:outer")],
        "eu-west-1",
        &AtomicBool::new(false),
        |_| url.clone(),
    )
    .expect("assume succeeds");

    mock.assert();
    assert_eq!(creds.access_key, "ASIAFINAL");
}

#[test]
fn a_refused_role_names_the_role_and_the_service_code() {
    // 403 with the body STS actually sends. The status has to be taken as
    // data, or the HTTP client drops the body and the sentence naming the
    // principal, the role and the condition goes with it.
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(403)
        .with_body(
            "<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>User: arn:aws:iam::111122223333:user/eric is not authorized to perform: sts:AssumeRole on resource: arn:aws:iam::1:role/Admin</Message></Error></ErrorResponse>",
        )
        .create();

    let url = server.url();
    let err = err_of(
        assume_chain_with_endpoint(
            &super::super::http_agent(),
            base_creds(),
            &[step("arn:aws:iam::1:role/Admin")],
            "eu-west-1",
            &AtomicBool::new(false),
            |_| url.clone(),
        ),
        "a refused role must fail",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("arn:aws:iam::1:role/Admin"),
        "role not named: {msg}"
    );
    assert!(msg.contains("AccessDenied"), "code not carried: {msg}");
    assert!(
        msg.contains("is not authorized to perform: sts:AssumeRole"),
        "the service's own sentence was dropped with the body: {msg}"
    );
}

#[test]
fn an_http_error_status_still_names_the_role() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(403)
        .with_body("<ErrorResponse/>")
        .create();

    let url = server.url();
    let err = err_of(
        assume_chain_with_endpoint(
            &super::super::http_agent(),
            base_creds(),
            &[step("arn:aws:iam::1:role/Admin")],
            "eu-west-1",
            &AtomicBool::new(false),
            |_| url.clone(),
        ),
        "a 403 must fail",
    );
    assert!(err.to_string().contains("arn:aws:iam::1:role/Admin"));
}

#[test]
fn a_success_body_without_credentials_is_reported_as_unparsable() {
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body("<AssumeRoleResponse><AssumeRoleResult/></AssumeRoleResponse>")
        .create();

    let url = server.url();
    let err = err_of(
        assume_chain_with_endpoint(
            &super::super::http_agent(),
            base_creds(),
            &[step("arn:r")],
            "eu-west-1",
            &AtomicBool::new(false),
            |_| url.clone(),
        ),
        "no credentials must fail",
    );
    assert!(matches!(err, ProviderError::Parse(_)));
}

#[test]
fn a_cancelled_sync_stops_before_the_first_call() {
    let err = err_of(
        assume_chain_with_endpoint(
            &super::super::http_agent(),
            base_creds(),
            &[step("arn:r")],
            "eu-west-1",
            &AtomicBool::new(true),
            |_| "http://127.0.0.1:1".to_string(),
        ),
        "cancel must stop it",
    );
    assert!(matches!(err, ProviderError::Cancelled));
}

#[test]
fn a_throttled_assume_role_is_reported_as_rate_limiting() {
    // STS answers 429 with a normal error body, so the status has to be read
    // before the body or throttling would surface as a refused role.
    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/")
        .match_query(mockito::Matcher::Any)
        .with_status(429)
        .with_body(
            "<ErrorResponse><Error><Code>Throttling</Code><Message>Rate exceeded</Message></Error></ErrorResponse>",
        )
        .create();

    let url = server.url();
    let err = err_of(
        assume_chain_with_endpoint(
            &super::super::http_agent(),
            base_creds(),
            &[step("arn:aws:iam::1:role/Admin")],
            "eu-west-1",
            &AtomicBool::new(false),
            |_| url.clone(),
        ),
        "429 must fail",
    );
    assert!(
        matches!(err, ProviderError::RateLimited),
        "expected RateLimited, got {err:?}"
    );
}
