//! AWS Security Token Service: assuming a role so one set of long-lived keys
//! reaches several accounts.
//!
//! Uses the Query API over GET, which AWS supports for every STS action, so
//! the EC2 signer carries straight over with `sts` as the service name. Always
//! a regional endpoint: AWS recommends it, tokens it issues are valid in every
//! Region, and it keeps the credential-scope Region equal to the request
//! Region instead of the global endpoint's implicit `us-east-1`.

use std::sync::atomic::AtomicBool;

use super::aws::{AwsCredentials, SigV4Request, format_utc, sign_request, uri_encode};
use super::aws_profile::RoleStep;
use super::{ProviderError, map_ureq_error};

/// SigV4 service name for STS.
const STS_SERVICE: &str = "sts";

/// The STS Query API version. Every request carries it.
const STS_API_VERSION: &str = "2011-06-15";

/// Session name used when the profile names none. AWS requires 2 to 64
/// characters from `[\w+=,.@-]`, and the name shows up in CloudTrail, so it
/// says which tool opened the session.
const DEFAULT_ROLE_SESSION_NAME: &str = "purple";

/// Bounds AWS accepts for `RoleSessionName`.
const MIN_SESSION_NAME_LEN: usize = 2;
const MAX_SESSION_NAME_LEN: usize = 64;

/// Lower bound AWS accepts for `DurationSeconds`.
const MIN_DURATION_SECONDS: u32 = 900;

/// Upper bound AWS accepts for `DurationSeconds`.
const MAX_DURATION_SECONDS: u32 = 43200;

/// What AWS uses when `DurationSeconds` is omitted, and the hard ceiling while
/// role chaining: assuming a role from an already-assumed session fails outright
/// above one hour, whatever the role's own maximum says.
const CHAINED_DURATION_SECONDS: u32 = 3600;

/// Longest AWS error message purple repeats back. Service messages are short;
/// the cap stops a hostile or malformed body from filling a toast. Shared with
/// Systems Manager, whose message lands in the same places.
pub(super) const MAX_ERROR_MESSAGE: usize = 300;

/// The STS endpoint for a region. Every region purple syncs is in the
/// commercial partition, GovCloud included, so there is one suffix. A partition
/// with its own suffix, China being the one AWS has, needs a branch here, one
/// in `aws_ssm` and one on the EC2 endpoint together with its region codes.
pub(super) fn region_endpoint(region: &str) -> String {
    format!("https://sts.{}.amazonaws.com", region)
}

/// Assume every role in `steps`, in order, starting from `base`. Returns the
/// credentials of the last role. An empty `steps` returns `base` untouched.
/// The endpoint is injected, so tests drive the whole sign-and-parse path
/// against a mock server.
pub(super) fn assume_chain_with_endpoint(
    agent: &ureq::Agent,
    base: AwsCredentials,
    steps: &[RoleStep],
    region: &str,
    cancel: &AtomicBool,
    resolve_endpoint: impl Fn(&str) -> String,
) -> Result<AwsCredentials, ProviderError> {
    let mut creds = base;
    let endpoint = resolve_endpoint(region);
    for step in steps {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ProviderError::Cancelled);
        }
        creds = assume_role(agent, &creds, step, region, &endpoint)?;
        log::debug!(
            "[external] aws sts: assumed {} for profile '{}'",
            step.role_arn,
            step.profile
        );
    }
    Ok(creds)
}

/// One AssumeRole call.
fn assume_role(
    agent: &ureq::Agent,
    creds: &AwsCredentials,
    step: &RoleStep,
    region: &str,
    endpoint: &str,
) -> Result<AwsCredentials, ProviderError> {
    let host = endpoint
        .split_once("://")
        .map(|(_, authority)| authority)
        .unwrap_or(endpoint)
        .to_string();
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (timestamp, datestamp) = format_utc(epoch);

    let mut params: Vec<(String, String)> = vec![
        ("Action".to_string(), "AssumeRole".to_string()),
        ("Version".to_string(), STS_API_VERSION.to_string()),
        ("RoleArn".to_string(), step.role_arn.clone()),
        ("RoleSessionName".to_string(), session_name(step)),
    ];
    if let Some(duration) = duration_seconds(step, creds.session_token.is_some()) {
        params.push(("DurationSeconds".to_string(), duration.to_string()));
    }
    if !step.external_id.is_empty() {
        params.push(("ExternalId".to_string(), step.external_id.clone()));
    }

    // SigV4 needs the query string sorted and percent-encoded.
    let mut sorted: Vec<(String, String)> = params
        .into_iter()
        .map(|(k, v)| (uri_encode(&k), uri_encode(&v)))
        .collect();
    sorted.sort();
    let query_string: String = sorted
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    let auth = sign_request(
        creds,
        region,
        &SigV4Request {
            method: "GET",
            service: STS_SERVICE,
            host: &host,
            query_string: &query_string,
            payload: b"",
            extra_headers: &[],
        },
        &timestamp,
        &datestamp,
    );

    let url = format!("{}/?{}", endpoint, query_string);
    let mut req = agent
        .get(&url)
        .header("Authorization", &auth)
        .header("x-amz-date", &timestamp);
    if let Some(token) = &creds.session_token {
        req = req.header("x-amz-security-token", token);
    }

    // STS reports a refused role in the body, and ureq turns a 4xx into an
    // error that drops it. Taking the status as data keeps "not authorized to
    // perform sts:AssumeRole on resource ..." instead of "HTTP 403".
    let mut resp = req
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(map_ureq_error)?;
    let status = resp.status();
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| ProviderError::Parse(e.to_string()))?;

    // Same reading as the rest of the provider layer: the caller backs off on
    // this one rather than showing it.
    if status.as_u16() == super::HTTP_TOO_MANY_REQUESTS {
        return Err(ProviderError::RateLimited);
    }
    if let Some(error) = parse_error(&body) {
        return Err(ProviderError::Execute(
            crate::messages::aws_assume_role_failed(&step.role_arn, &error),
        ));
    }
    if !status.is_success() {
        return Err(ProviderError::Execute(
            crate::messages::aws_assume_role_failed(
                &step.role_arn,
                &format!("HTTP {}", status.as_u16()),
            ),
        ));
    }
    parse_credentials(&body).ok_or_else(|| {
        ProviderError::Parse(crate::messages::aws_assume_role_unparsable(&step.role_arn))
    })
}

/// The session name to send: the profile's own, or a default. An invalid name
/// is replaced rather than rejected, because it only labels the session in
/// CloudTrail and failing a sync over it would be the worse outcome.
fn session_name(step: &RoleStep) -> String {
    let candidate = step.role_session_name.trim();
    if is_valid_session_name(candidate) {
        candidate.to_string()
    } else {
        DEFAULT_ROLE_SESSION_NAME.to_string()
    }
}

/// AWS: 2 to 64 characters, each of `[\w+=,.@-]`.
fn is_valid_session_name(name: &str) -> bool {
    let len = name.chars().count();
    (MIN_SESSION_NAME_LEN..=MAX_SESSION_NAME_LEN).contains(&len)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_+=,.@-".contains(c))
}

/// The `DurationSeconds` to send, or None to let AWS apply its own default.
///
/// Chaining from an already-assumed session fails above one hour, so a request
/// signed with a session token is capped there rather than passed through.
fn duration_seconds(step: &RoleStep, chained: bool) -> Option<u32> {
    let requested = step.duration_seconds.trim().parse::<u32>().ok()?;
    if !(MIN_DURATION_SECONDS..=MAX_DURATION_SECONDS).contains(&requested) {
        return None;
    }
    Some(if chained {
        requested.min(CHAINED_DURATION_SECONDS)
    } else {
        requested
    })
}

/// Pull the four credential fields out of an AssumeRole response.
///
/// Matches on element names rather than position: STS documents a different
/// child order for `Credentials` across its operations.
fn parse_credentials(body: &str) -> Option<AwsCredentials> {
    let access_key = element_value(body, "AccessKeyId")?;
    let secret_key = element_value(body, "SecretAccessKey")?;
    let session_token = element_value(body, "SessionToken")?;
    if access_key.is_empty() || secret_key.is_empty() || session_token.is_empty() {
        return None;
    }
    Some(AwsCredentials {
        access_key,
        secret_key,
        session_token: Some(session_token),
    })
}

/// `Code: Message` from an STS error body, or None when the body is not one.
///
/// STS uses the Query-protocol shape `ErrorResponse > Error > {Type, Code,
/// Message}`, which is not the EC2 shape, so this does not share EC2's parser.
fn parse_error(body: &str) -> Option<String> {
    let code = element_prose(body, "Code")?;
    let message = element_prose(body, "Message").unwrap_or_default();
    let mut text = if message.is_empty() {
        code
    } else {
        format!("{}: {}", code, message)
    };
    if text.chars().count() > MAX_ERROR_MESSAGE {
        text = text.chars().take(MAX_ERROR_MESSAGE).collect();
    }
    Some(text)
}

/// Text of the first `<name>` element with every run of whitespace removed.
///
/// STS sends a credential on one line, but a proxy or a body pasted from the
/// documentation can wrap it. A key id, a secret and a session token are all
/// base64 and never contain whitespace, so the pieces join straight up: a
/// space put back where the wrap was would be signed as part of the value.
fn element_value(body: &str, name: &str) -> Option<String> {
    element_slice(body, name).map(|raw| raw.split_whitespace().collect())
}

/// Text of the first `<name>` element as prose: trimmed, with internal runs of
/// whitespace collapsed to one space. For the error body, which is a sentence.
fn element_prose(body: &str, name: &str) -> Option<String> {
    element_slice(body, name).map(|raw| raw.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The raw text between `<name>` and `</name>`.
///
/// Deliberately name-based rather than a typed deserialize: the response
/// carries fields that come and go (`PackedPolicySize` is deprecated,
/// `SessionTokenSize` is new) and a strict struct would break on them.
fn element_slice<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{}>", name);
    let close = format!("</{}>", name);
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    Some(&body[start..end])
}

#[cfg(test)]
#[path = "aws_sts_tests.rs"]
mod tests;
