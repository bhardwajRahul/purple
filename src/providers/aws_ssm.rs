//! AWS Systems Manager Session Manager as an SSH transport.
//!
//! Session Manager tunnels SSH inside a WebSocket the node opens outbound, so
//! a host with every inbound port closed, or with no public address at all,
//! still answers. purple writes the `ProxyCommand` AWS documents into the host
//! block and plain `ssh`, `scp` and `sftp` then work unchanged.
//!
//! Which nodes get one is the `auto` question: EC2 cannot report whether an
//! instance is SSM-managed, so the only answer is to ask Systems Manager
//! itself. That API is AWS JSON 1.1 over POST rather than the EC2 Query API,
//! which is why the SigV4 signer takes a method, a payload and extra headers.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use super::aws::{AwsCredentials, SigV4Request, format_utc, sign_request};
use super::{ProviderError, map_ureq_error};

/// SigV4 service name for Systems Manager.
const SSM_SERVICE: &str = "ssm";

/// The JSON 1.1 target header for DescribeInstanceInformation. SSM uses the
/// `AmazonSSM.<Operation>` prefix with no version suffix.
const SSM_TARGET_DESCRIBE: &str = "AmazonSSM.DescribeInstanceInformation";

/// Content type for AWS JSON 1.1. Signed, so it has to match byte for byte.
const SSM_CONTENT_TYPE: &str = "application/x-amz-json-1.1";

/// Page size for DescribeInstanceInformation. AWS accepts 5 to 50 and defaults
/// to 10, so asking for the maximum is the fewest round trips.
const SSM_MAX_RESULTS: u32 = 50;

/// Page guard for the node listing, matching the one the EC2 walk uses. The
/// default fleet quota is 2400 nodes per account and region, which is 48 pages.
const SSM_MAX_PAGES: usize = 500;

/// The SSM document that runs an SSH session. Not to be confused with
/// `AWS-StartPortForwardingSession`, which tunnels a port to localhost.
const SSM_SSH_DOCUMENT: &str = "AWS-StartSSHSession";

/// Whether a config routes its hosts through Session Manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SsmMode {
    /// Hosts keep their IP address. The default.
    #[default]
    Off,
    /// Ask Systems Manager which nodes are reachable and route only those.
    Auto,
    /// Route every instance without asking, for a caller allowed to open a
    /// session but not to list nodes.
    Always,
}

impl SsmMode {
    /// The value as it is written to the provider config.
    pub fn as_str(self) -> &'static str {
        match self {
            SsmMode::Off => "off",
            SsmMode::Auto => "auto",
            SsmMode::Always => "always",
        }
    }

    /// The three modes in cycle order, for the form field and the CLI help.
    pub const ALL: &'static [SsmMode] = &[SsmMode::Off, SsmMode::Auto, SsmMode::Always];

    /// The next mode when the form field is activated.
    pub fn next(self) -> SsmMode {
        match self {
            SsmMode::Off => SsmMode::Auto,
            SsmMode::Auto => SsmMode::Always,
            SsmMode::Always => SsmMode::Off,
        }
    }

    /// Whether any host of this config gets a proxy command.
    pub fn is_enabled(self) -> bool {
        !matches!(self, SsmMode::Off)
    }
}

impl FromStr for SsmMode {
    type Err = ();

    /// Parses the three names. An unknown or empty value reads as `Off`, so a
    /// hand-edited config never fails to load over it.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(SsmMode::Auto),
            "always" | "true" | "yes" | "on" => Ok(SsmMode::Always),
            _ => Ok(SsmMode::Off),
        }
    }
}

impl std::fmt::Display for SsmMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a profile name is safe to place inside the generated command.
///
/// The proxy command is a shell line purple writes itself, and OpenSSH expands
/// its tokens without quoting. Restricting the name to the charset AWS
/// documents for a profile keeps a quote or a semicolon out of it.
pub fn is_safe_profile_name(profile: &str) -> bool {
    !profile.is_empty()
        && profile
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// The `ProxyCommand` value for a Session Manager host.
///
/// Follows the line AWS publishes, so a user who has read their docs
/// recognizes it. `%h` is the resolved `HostName`, which purple sets to the
/// instance ID, and `%p` carries the port the host block resolved to, so a
/// non-default `Port` reaches the right daemon.
pub fn proxy_command(profile: &str, region: &str) -> String {
    let profile_part = if profile.is_empty() {
        String::new()
    } else {
        format!(" --profile {}", profile)
    };
    format!("{}{}{}", command_head(), profile_part, command_tail(region))
}

/// Everything before the profile: the shell wrapper, the target, the document
/// and the parameters.
fn command_head() -> String {
    format!(
        "sh -c \"aws ssm start-session --target %h --document-name {} --parameters 'portNumber=%p'",
        SSM_SSH_DOCUMENT
    )
}

/// Everything after the profile.
fn command_tail(region: &str) -> String {
    if region.is_empty() {
        "\"".to_string()
    } else {
        format!(" --region {}\"", region)
    }
}

/// Whether `value` is a proxy command purple generated for `region`.
///
/// Every fixed part has to be there byte for byte: the shell wrapper, the
/// `%h` target, the document, the parameters plus the region. The one segment
/// free to differ is `--profile`, which the config decides and which purple
/// therefore cannot pin to a single value. It still has to be a name purple
/// would write. A line the user typed survives unless it is purple's own line
/// down to the byte apart from that one name, at which point the two are the
/// same line.
pub(super) fn is_generated_proxy_command(value: &str, region: &str) -> bool {
    let Some(rest) = value.strip_prefix(&command_head()) else {
        return false;
    };
    let Some(middle) = rest.strip_suffix(&command_tail(region)) else {
        return false;
    };
    middle.is_empty()
        || middle
            .strip_prefix(" --profile ")
            .is_some_and(is_safe_profile_name)
}

/// The SSM endpoint for a region. Every region purple syncs is in the
/// commercial partition, so there is one suffix. See `aws_sts::region_endpoint`
/// for what a second partition would take.
pub(super) fn region_endpoint(region: &str) -> String {
    format!("https://ssm.{}.amazonaws.com", region)
}

/// Instance IDs of the managed nodes Session Manager reports as online in one
/// region. The endpoint is injected so tests drive signing, paging and parsing
/// against a mock server.
pub(super) fn online_nodes_with_endpoint(
    agent: &ureq::Agent,
    creds: &AwsCredentials,
    region: &str,
    cancel: &AtomicBool,
    endpoint: &str,
) -> Result<HashSet<String>, ProviderError> {
    let mut ids = HashSet::new();
    let mut next_token: Option<String> = None;

    for _ in 0..SSM_MAX_PAGES {
        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Cancelled);
        }
        let body = request_body(next_token.as_deref());
        let text = post(agent, creds, region, endpoint, &body)?;
        let parsed: DescribeInstanceInformationResponse = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Parse(format!("{}: {}", region, e)))?;

        for node in parsed.instance_information_list {
            // Every field is optional in the API model, and a node that is not
            // online cannot take a session, so both are filtered here rather
            // than trusted from the server-side filter alone.
            let (Some(id), Some(status)) = (node.instance_id, node.ping_status) else {
                continue;
            };
            if status == "Online" {
                ids.insert(id);
            }
        }

        // SSM signals the end with an empty token rather than by omitting it.
        match parsed.next_token {
            Some(token) if !token.is_empty() => next_token = Some(token),
            _ => return Ok(ids),
        }
    }
    Ok(ids)
}

/// The JSON body for one page.
fn request_body(next_token: Option<&str>) -> String {
    let filters = format!(
        r#""Filters":[{{"Key":"PingStatus","Values":["Online"]}}],"MaxResults":{}"#,
        SSM_MAX_RESULTS
    );
    match next_token {
        // The token is service-issued and base64-shaped, but it is escaped
        // anyway: it is going into a JSON document purple builds by hand.
        Some(token) => format!(r#"{{{},"NextToken":{}}}"#, filters, json_string(token)),
        None => format!("{{{}}}", filters),
    }
}

/// A JSON string literal, with the characters JSON requires escaped.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One signed JSON POST to Systems Manager.
fn post(
    agent: &ureq::Agent,
    creds: &AwsCredentials,
    region: &str,
    endpoint: &str,
    body: &str,
) -> Result<String, ProviderError> {
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

    // JSON 1.1 signs the body, and content-type plus x-amz-target join the
    // canonical headers. Both must be sent exactly as signed.
    let auth = sign_request(
        creds,
        region,
        &SigV4Request {
            method: "POST",
            service: SSM_SERVICE,
            host: &host,
            query_string: "",
            payload: body.as_bytes(),
            extra_headers: &[
                ("content-type", SSM_CONTENT_TYPE),
                ("x-amz-target", SSM_TARGET_DESCRIBE),
            ],
        },
        &timestamp,
        &datestamp,
    );

    let url = format!("{}/", endpoint);
    let mut req = agent
        .post(&url)
        .header("Authorization", &auth)
        .header("x-amz-date", &timestamp)
        .header("content-type", SSM_CONTENT_TYPE)
        .header("x-amz-target", SSM_TARGET_DESCRIBE);
    if let Some(token) = &creds.session_token {
        req = req.header("x-amz-security-token", token);
    }

    // Systems Manager puts the reason in the body, and ureq turns a 4xx into
    // an error that drops it. Taking the status as data keeps
    // "AccessDeniedException: User ... is not authorized to perform:
    // ssm:DescribeInstanceInformation" instead of a bare status.
    let mut resp = req
        .config()
        .http_status_as_error(false)
        .build()
        .send(body)
        .map_err(map_ureq_error)?;
    let status = resp.status();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| ProviderError::Parse(e.to_string()))?;

    if !status.is_success() {
        let code = status.as_u16();
        // Rate limiting is the one status the caller reads rather than
        // reports. Everything else carries the service's own sentence, which
        // says more than any mapping purple could apply to the status alone.
        if code == super::HTTP_TOO_MANY_REQUESTS {
            log::warn!("[external] aws ssm: HTTP {} rate limited", code);
            return Err(ProviderError::RateLimited);
        }
        let detail = parse_error(&text).unwrap_or_else(|| format!("HTTP {}", code));
        log::warn!("[external] aws ssm: {}", detail);
        return Err(ProviderError::Execute(detail));
    }
    Ok(text)
}

/// Cut an error message to the shared cap. The value ends up in a toast, and
/// a body that is hostile or simply enormous must not fill it.
fn capped(text: String) -> String {
    if text.chars().count() > super::aws_sts::MAX_ERROR_MESSAGE {
        return text
            .chars()
            .take(super::aws_sts::MAX_ERROR_MESSAGE)
            .collect();
    }
    text
}

/// `Code: message` from an AWS JSON 1.1 error body, or None when the body is
/// not one.
///
/// The type is a shape id (`com.amazonaws.ssm#AccessDeniedException` in some
/// responses, the bare name in others), so only the part after the `#` is
/// kept. The message key is lowercase in most services and capitalized in a
/// few, and both spellings occur on SSM.
fn parse_error(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let code = parsed.get("__type")?.as_str()?;
    let code = code.rsplit('#').next().unwrap_or(code).trim();
    // Without a code there is no reason to report, and an empty detail reads
    // worse than the status the caller falls back to.
    if code.is_empty() {
        return None;
    }
    let message = parsed
        .get("message")
        .or_else(|| parsed.get("Message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim();
    if message.is_empty() {
        return Some(capped(code.to_string()));
    }
    Some(capped(format!("{}: {}", code, message)))
}

#[derive(serde::Deserialize, Debug, Default)]
struct DescribeInstanceInformationResponse {
    #[serde(rename = "InstanceInformationList", default)]
    instance_information_list: Vec<InstanceInformation>,
    #[serde(rename = "NextToken", default)]
    next_token: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct InstanceInformation {
    #[serde(rename = "InstanceId", default)]
    instance_id: Option<String>,
    #[serde(rename = "PingStatus", default)]
    ping_status: Option<String>,
}

#[cfg(test)]
#[path = "aws_ssm_tests.rs"]
mod tests;
