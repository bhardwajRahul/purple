use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use serde::Deserialize;

use super::{Provider, ProviderError, ProviderHost, map_ureq_error};

pub struct TransIp;

#[derive(Deserialize)]
struct VpsListResponse {
    vpss: Vec<TransIpVps>,
}

#[derive(Deserialize)]
struct TransIpVps {
    name: String,
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "productName")]
    product_name: String,
    #[serde(default, rename = "operatingSystem")]
    operating_system: String,
    #[serde(default)]
    status: String,
    #[serde(default, rename = "ipAddress")]
    ip_address: String,
    #[serde(default, rename = "availabilityZone")]
    availability_zone: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
}

/// Parsed token field: either login + key path, or a pre-generated Bearer token.
enum TransIpAuth<'a> {
    KeyFile { login: &'a str, key_path: &'a str },
    BearerToken(&'a str),
}

/// Parse the token field. Format `login:/path/to/key.pem` triggers RSA auth,
/// anything else is treated as a pre-generated Bearer token.
fn parse_auth(token: &str) -> TransIpAuth<'_> {
    if let Some(colon_pos) = token.find(':') {
        let after = &token[colon_pos + 1..];
        if after.starts_with('/') || after.starts_with('~') || after.starts_with('.') {
            return TransIpAuth::KeyFile {
                login: &token[..colon_pos],
                key_path: after,
            };
        }
    }
    TransIpAuth::BearerToken(token)
}

/// Request a Bearer token from the TransIP API using RSA-SHA512 signing.
fn request_token(
    agent: &ureq::Agent,
    base_url: &str,
    login: &str,
    key_path: &str,
    env: &crate::runtime::env::Env,
) -> Result<String, ProviderError> {
    let home = || {
        env.paths()
            .map(|p| p.home().to_path_buf())
            .unwrap_or_default()
    };
    let resolved_path = if let Some(stripped) = key_path.strip_prefix("~/") {
        home().join(stripped).to_string_lossy().into_owned()
    } else if key_path == "~" {
        home().to_string_lossy().into_owned()
    } else {
        key_path.to_string()
    };

    let key_pem = std::fs::read_to_string(&resolved_path).map_err(|e| {
        ProviderError::Execute(format!(
            "Failed to read TransIP private key {}: {}",
            resolved_path, e
        ))
    })?;

    let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(&key_pem)
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(&key_pem))
        .map_err(|e| ProviderError::Execute(format!("Failed to parse RSA private key: {}", e)))?;

    // Generate a unique nonce from timestamp + PID
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let nonce = format!("{:016x}{:08x}", now.as_nanos(), std::process::id());

    let body = serde_json::json!({
        "login": login,
        "nonce": nonce,
        "read_only": true,
        "expiration_time": "1 hour",
        "label": "purple-ssh",
        "global_key": false
    });
    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| ProviderError::Execute(format!("Failed to serialize auth body: {}", e)))?;

    // Sign body with RSA-SHA512 PKCS#1 v1.5
    let signing_key = rsa::pkcs1v15::SigningKey::<sha2::Sha512>::new(private_key);
    let signature = signing_key.sign(&body_bytes);
    let sig_b64 = STANDARD.encode(signature.to_bytes());

    let mut resp = agent
        .post(&format!("{}/v6/auth", base_url))
        .header("Signature", &sig_b64)
        .content_type("application/json")
        .send(&body_bytes)
        .map_err(map_ureq_error)?;
    let resp: TokenResponse = resp
        .body_mut()
        .read_json()
        .map_err(|e| ProviderError::Parse(e.to_string()))?;

    Ok(resp.token)
}

impl TransIp {
    /// Real API host. Overridable per call via `fetch_from` so tests can point
    /// the full fetch pipeline (auth token exchange and VPS listing) at a mock
    /// server.
    const API_BASE: &'static str = "https://api.transip.nl";

    /// Fetch hosts against an explicit API base. Production passes `API_BASE`;
    /// tests pass a mock server URL. Single seam covering the optional RSA token
    /// exchange, URL construction, auth header, error mapping, pagination and
    /// `ProviderHost` mapping.
    fn fetch_from(
        &self,
        base_url: &str,
        token: &str,
        cancel: &AtomicBool,
        env: &crate::runtime::env::Env,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Cancelled);
        }

        let agent = super::http_agent();

        let bearer = match parse_auth(token) {
            TransIpAuth::BearerToken(t) => t.to_string(),
            TransIpAuth::KeyFile { login, key_path } => {
                request_token(&agent, base_url, login, key_path, env)?
            }
        };

        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Cancelled);
        }

        let per_page = 200usize;

        super::paginate(cancel, |idx| {
            let page = idx + 1;
            let url = format!("{}/v6/vps?page={}&pageSize={}", base_url, page, per_page);
            let resp: VpsListResponse = agent
                .get(&url)
                .header("Authorization", &super::bearer_auth(&bearer))
                .call()
                .map_err(map_ureq_error)?
                .body_mut()
                .read_json()
                .map_err(|e| ProviderError::Parse(e.to_string()))?;

            let mut hosts = Vec::with_capacity(resp.vpss.len());
            for vps in &resp.vpss {
                if vps.ip_address.is_empty() {
                    continue;
                }
                let ip = super::strip_cidr(&vps.ip_address).to_string();

                let mut metadata = super::ProviderMetadata::new();
                if !vps.availability_zone.is_empty() {
                    metadata.push("zone", vps.availability_zone.clone());
                }
                if !vps.product_name.is_empty() {
                    metadata.push("plan", vps.product_name.clone());
                }
                if !vps.operating_system.is_empty() {
                    metadata.push("os", vps.operating_system.clone());
                }
                if !vps.status.is_empty() {
                    metadata.push("status", vps.status.clone());
                }

                let display_name = if !vps.description.is_empty() {
                    vps.description.clone()
                } else {
                    vps.name.clone()
                };

                hosts.push(ProviderHost {
                    server_id: if !vps.uuid.is_empty() {
                        vps.uuid.clone()
                    } else {
                        vps.name.clone()
                    },
                    name: display_name,
                    ip,
                    tags: vps.tags.clone(),
                    metadata: metadata.finish(),
                    ..Default::default()
                });
            }

            let more = !resp.vpss.is_empty() && resp.vpss.len() >= per_page;
            Ok(super::PageResult { hosts, more })
        })
    }
}

impl Provider for TransIp {
    fn name(&self) -> &str {
        "transip"
    }

    fn short_label(&self) -> &str {
        "tip"
    }

    fn fetch_hosts_cancellable(
        &self,
        token: &str,
        cancel: &AtomicBool,
        env: &crate::runtime::env::Env,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        self.fetch_from(Self::API_BASE, token, cancel, env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Auth parsing tests ──────────────────────────────────────────

    #[test]
    fn test_parse_auth_key_file_absolute() {
        match parse_auth("mylogin:/home/user/.transip/key.pem") {
            TransIpAuth::KeyFile { login, key_path } => {
                assert_eq!(login, "mylogin");
                assert_eq!(key_path, "/home/user/.transip/key.pem");
            }
            _ => panic!("expected KeyFile"),
        }
    }

    #[test]
    fn test_parse_auth_key_file_tilde() {
        match parse_auth("mylogin:~/.transip/key.pem") {
            TransIpAuth::KeyFile { login, key_path } => {
                assert_eq!(login, "mylogin");
                assert_eq!(key_path, "~/.transip/key.pem");
            }
            _ => panic!("expected KeyFile"),
        }
    }

    #[test]
    fn test_parse_auth_key_file_relative() {
        match parse_auth("mylogin:./key.pem") {
            TransIpAuth::KeyFile { login, key_path } => {
                assert_eq!(login, "mylogin");
                assert_eq!(key_path, "./key.pem");
            }
            _ => panic!("expected KeyFile"),
        }
    }

    #[test]
    fn test_parse_auth_bearer_token() {
        let jwt = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJsb2dpbiI6InRlc3QifQ.sig";
        match parse_auth(jwt) {
            TransIpAuth::BearerToken(t) => assert_eq!(t, jwt),
            _ => panic!("expected BearerToken"),
        }
    }

    #[test]
    fn test_parse_auth_plain_token_no_colon() {
        match parse_auth("some-api-token") {
            TransIpAuth::BearerToken(t) => assert_eq!(t, "some-api-token"),
            _ => panic!("expected BearerToken"),
        }
    }

    // ── Deserialization tests ───────────────────────────────────────

    #[test]
    fn test_parse_vps_response() {
        let json = r#"{
            "vpss": [{
                "name": "example-vps",
                "uuid": "bfa08ad9-6c12-4e03-95dd-a888b97ffe49",
                "description": "My web server",
                "productName": "vps-bladevps-x1",
                "operatingSystem": "ubuntu-22.04",
                "status": "running",
                "ipAddress": "37.97.254.6",
                "availabilityZone": "ams0",
                "tags": ["production", "web"],
                "cpus": 2,
                "memorySize": 4194304
            }]
        }"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.vpss.len(), 1);
        assert_eq!(resp.vpss[0].name, "example-vps");
        assert_eq!(resp.vpss[0].ip_address, "37.97.254.6");
        assert_eq!(resp.vpss[0].status, "running");
        assert_eq!(resp.vpss[0].availability_zone, "ams0");
        assert_eq!(resp.vpss[0].tags, vec!["production", "web"]);
        assert_eq!(resp.vpss[0].product_name, "vps-bladevps-x1");
        assert_eq!(resp.vpss[0].operating_system, "ubuntu-22.04");
    }

    #[test]
    fn test_parse_vps_minimal() {
        let json = r#"{"vpss": [{"name": "v1", "ipAddress": "1.2.3.4"}]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.vpss.len(), 1);
        assert_eq!(resp.vpss[0].name, "v1");
        assert!(resp.vpss[0].tags.is_empty());
        assert_eq!(resp.vpss[0].description, "");
    }

    #[test]
    fn test_parse_vps_empty_list() {
        let resp: VpsListResponse = serde_json::from_str(r#"{"vpss": []}"#).unwrap();
        assert!(resp.vpss.is_empty());
    }

    #[test]
    fn test_parse_vps_extra_fields_ignored() {
        let json = r#"{"vpss": [{
            "name": "v1",
            "ipAddress": "1.2.3.4",
            "diskSize": 157286400,
            "macAddress": "52:54:00:3b:52:65",
            "currentSnapshots": 1,
            "maxSnapshots": 10,
            "isLocked": false,
            "isBlocked": false,
            "isCustomerLocked": false
        }]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.vpss[0].name, "v1");
    }

    #[test]
    fn test_parse_token_response() {
        let json = r#"{"token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.body.sig"}"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert!(resp.token.starts_with("eyJ"));
    }

    // ── Provider trait tests ────────────────────────────────────────

    #[test]
    fn test_name_and_short_label() {
        let tip = TransIp;
        assert_eq!(tip.name(), "transip");
        assert_eq!(tip.short_label(), "tip");
    }

    #[test]
    fn test_vps_no_ip_skipped() {
        let json = r#"{"vpss": [
            {"name": "has-ip", "ipAddress": "1.2.3.4", "status": "running"},
            {"name": "no-ip", "ipAddress": "", "status": "installing"}
        ]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        let hosts: Vec<_> = resp
            .vpss
            .iter()
            .filter(|v| !v.ip_address.is_empty())
            .collect();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "has-ip");
    }

    #[test]
    fn test_metadata_all_fields() {
        let json = r#"{"vpss": [{
            "name": "v1",
            "description": "Web server",
            "productName": "vps-bladevps-x4",
            "operatingSystem": "ubuntu-22.04",
            "status": "running",
            "ipAddress": "1.2.3.4",
            "availabilityZone": "ams0",
            "tags": ["prod"]
        }]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        let vps = &resp.vpss[0];
        let mut metadata = super::super::ProviderMetadata::new();
        if !vps.availability_zone.is_empty() {
            metadata.push("zone", vps.availability_zone.clone());
        }
        if !vps.product_name.is_empty() {
            metadata.push("plan", vps.product_name.clone());
        }
        if !vps.operating_system.is_empty() {
            metadata.push("os", vps.operating_system.clone());
        }
        if !vps.status.is_empty() {
            metadata.push("status", vps.status.clone());
        }
        let metadata = metadata.finish();
        assert_eq!(metadata.len(), 4);
        assert_eq!(metadata[0], ("zone".to_string(), "ams0".to_string()));
        assert_eq!(
            metadata[1],
            ("plan".to_string(), "vps-bladevps-x4".to_string())
        );
    }

    #[test]
    fn test_description_used_as_display_name() {
        let json = r#"{"vpss": [{
            "name": "vps-abc123",
            "description": "My web server",
            "ipAddress": "1.2.3.4"
        }]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        let vps = &resp.vpss[0];
        let display_name = if !vps.description.is_empty() {
            vps.description.clone()
        } else {
            vps.name.clone()
        };
        assert_eq!(display_name, "My web server");
    }

    #[test]
    fn test_name_used_when_no_description() {
        let json = r#"{"vpss": [{"name": "vps-abc123", "ipAddress": "1.2.3.4"}]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        let vps = &resp.vpss[0];
        let display_name = if !vps.description.is_empty() {
            vps.description.clone()
        } else {
            vps.name.clone()
        };
        assert_eq!(display_name, "vps-abc123");
    }

    #[test]
    fn test_ipv6_cidr_stripped() {
        assert_eq!(super::super::strip_cidr("2a01:7c8::1/64"), "2a01:7c8::1");
    }

    #[test]
    fn test_ipv4_unchanged() {
        assert_eq!(super::super::strip_cidr("37.97.254.6"), "37.97.254.6");
    }

    // ── HTTP roundtrip tests (mockito) ──────────────────────────────

    #[test]
    fn test_http_vps_list_roundtrip() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v6/vps")
            .match_header("Authorization", "Bearer test-jwt-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "vpss": [{
                    "name": "example-vps",
                    "uuid": "uuid-1",
                    "description": "Web server",
                    "productName": "vps-bladevps-x1",
                    "operatingSystem": "ubuntu-22.04",
                    "status": "running",
                    "ipAddress": "37.97.254.6",
                    "availabilityZone": "ams0",
                    "tags": ["prod"]
                }]
            }"#,
            )
            .create();

        let agent = super::super::http_agent();
        let resp: VpsListResponse = agent
            .get(&format!("{}/v6/vps", server.url()))
            .header("Authorization", "Bearer test-jwt-token")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(resp.vpss.len(), 1);
        assert_eq!(resp.vpss[0].name, "example-vps");
        assert_eq!(resp.vpss[0].ip_address, "37.97.254.6");
        mock.assert();
    }

    #[test]
    fn test_http_vps_list_auth_failure() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v6/vps")
            .with_status(401)
            .with_body(r#"{"error": "Your access token is invalid"}"#)
            .create();

        let agent = super::super::http_agent();
        let result = agent
            .get(&format!("{}/v6/vps", server.url()))
            .header("Authorization", "Bearer bad-token")
            .call();

        assert!(result.is_err());
        let err = super::map_ureq_error(result.unwrap_err());
        assert!(matches!(err, ProviderError::AuthFailed));
        mock.assert();
    }

    #[test]
    fn test_http_vps_list_rate_limited() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v6/vps")
            .with_status(429)
            .with_body(r#"{"error": "Rate limit exceeded"}"#)
            .create();

        let agent = super::super::http_agent();
        let result = agent
            .get(&format!("{}/v6/vps", server.url()))
            .header("Authorization", "Bearer test-token")
            .call();

        assert!(result.is_err());
        let err = super::map_ureq_error(result.unwrap_err());
        assert!(matches!(err, ProviderError::RateLimited));
        mock.assert();
    }

    #[test]
    fn test_http_vps_list_empty() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v6/vps")
            .match_header("Authorization", "Bearer test-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"vpss": []}"#)
            .create();

        let agent = super::super::http_agent();
        let resp: VpsListResponse = agent
            .get(&format!("{}/v6/vps", server.url()))
            .header("Authorization", "Bearer test-token")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert!(resp.vpss.is_empty());
        mock.assert();
    }

    #[test]
    fn test_http_vps_list_multiple() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v6/vps")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "vpss": [
                    {"name": "web-1", "ipAddress": "1.1.1.1", "status": "running", "tags": ["web"]},
                    {"name": "db-1", "ipAddress": "2.2.2.2", "status": "running", "tags": ["db"]},
                    {"name": "cache-1", "ipAddress": "", "status": "installing", "tags": []}
                ]
            }"#,
            )
            .create();

        let agent = super::super::http_agent();
        let resp: VpsListResponse = agent
            .get(&format!("{}/v6/vps", server.url()))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(resp.vpss.len(), 3);
        // Only 2 have IPs (cache-1 has empty IP and would be filtered)
        let with_ip: Vec<_> = resp
            .vpss
            .iter()
            .filter(|v| !v.ip_address.is_empty())
            .collect();
        assert_eq!(with_ip.len(), 2);
        mock.assert();
    }

    // ── Cancellation test ───────────────────────────────────────────

    #[test]
    fn test_cancellation_returns_cancelled() {
        let tip = TransIp;
        let cancel = AtomicBool::new(true);
        let result =
            tip.fetch_hosts_cancellable("some-token", &cancel, &crate::runtime::env::Env::empty());
        assert!(matches!(result, Err(ProviderError::Cancelled)));
    }

    // ── Edge case tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_auth_bare_tilde() {
        match parse_auth("login:~") {
            TransIpAuth::KeyFile { login, key_path } => {
                assert_eq!(login, "login");
                assert_eq!(key_path, "~");
            }
            _ => panic!("expected KeyFile"),
        }
    }

    #[test]
    fn test_parse_auth_bare_dot() {
        match parse_auth("login:.") {
            TransIpAuth::KeyFile { login, key_path } => {
                assert_eq!(login, "login");
                assert_eq!(key_path, ".");
            }
            _ => panic!("expected KeyFile"),
        }
    }

    #[test]
    fn test_uuid_used_as_server_id() {
        let json = r#"{"vpss": [{
            "name": "vps-slug",
            "uuid": "bfa08ad9-6c12-4e03-95dd-a888b97ffe49",
            "ipAddress": "1.2.3.4"
        }]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        let vps = &resp.vpss[0];
        let server_id = if !vps.uuid.is_empty() {
            vps.uuid.clone()
        } else {
            vps.name.clone()
        };
        assert_eq!(server_id, "bfa08ad9-6c12-4e03-95dd-a888b97ffe49");
    }

    #[test]
    fn test_name_fallback_when_no_uuid() {
        let json = r#"{"vpss": [{"name": "vps-slug", "ipAddress": "1.2.3.4"}]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        let vps = &resp.vpss[0];
        let server_id = if !vps.uuid.is_empty() {
            vps.uuid.clone()
        } else {
            vps.name.clone()
        };
        assert_eq!(server_id, "vps-slug");
    }

    // ── Pagination tests ────────────────────────────────────────────

    #[test]
    fn test_pagination_stops_on_short_page() {
        // A response with fewer items than per_page means it's the last page
        let json = r#"{"vpss": [
            {"name": "v1", "ipAddress": "1.1.1.1"}
        ]}"#;
        let resp: VpsListResponse = serde_json::from_str(json).unwrap();
        // batch_len (1) < per_page (200) → should stop
        assert!(resp.vpss.len() < 200);
    }

    #[test]
    fn test_pagination_stops_on_empty() {
        let resp: VpsListResponse = serde_json::from_str(r#"{"vpss": []}"#).unwrap();
        assert!(resp.vpss.is_empty());
    }

    // ── fetch_from seam tests (mockito) ─────────────────────────────

    #[test]
    fn fetch_from_drives_full_pipeline_against_mock() {
        // Drives the production fetch path end to end through fetch_from with a
        // pre-generated Bearer token (no key-file auth step): URL construction,
        // auth header, JSON deserialize, ProviderHost mapping and pagination
        // termination.
        let jwt = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJsb2dpbiI6InQifQ.sig";
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v6/vps")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("pageSize".into(), "200".into()),
            ]))
            .match_header("Authorization", format!("Bearer {}", jwt).as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "vpss": [{
                        "name": "example-vps",
                        "uuid": "uuid-1",
                        "description": "Web server",
                        "productName": "vps-bladevps-x1",
                        "operatingSystem": "ubuntu-22.04",
                        "status": "running",
                        "ipAddress": "37.97.254.6",
                        "availabilityZone": "ams0",
                        "tags": ["prod"]
                    }]
                }"#,
            )
            .create();

        let hosts = TransIp
            .fetch_from(
                &server.url(),
                jwt,
                &AtomicBool::new(false),
                &crate::runtime::env::Env::empty(),
            )
            .expect("fetch_from must succeed against the mock");
        mock.assert();

        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].server_id, "uuid-1");
        assert_eq!(hosts[0].name, "Web server");
        assert_eq!(hosts[0].ip, "37.97.254.6");
        assert_eq!(hosts[0].tags, vec!["prod"]);
        assert!(
            hosts[0]
                .metadata
                .contains(&("zone".to_string(), "ams0".to_string()))
        );
    }

    #[test]
    fn fetch_from_maps_auth_failure_to_provider_error() {
        let jwt = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJsb2dpbiI6InQifQ.sig";
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v6/vps")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"error": "Your access token is invalid"}"#)
            .create();

        let result = TransIp.fetch_from(
            &server.url(),
            jwt,
            &AtomicBool::new(false),
            &crate::runtime::env::Env::empty(),
        );
        mock.assert();
        assert!(
            matches!(result, Err(ProviderError::AuthFailed)),
            "401 must map to ProviderError::AuthFailed, got {result:?}"
        );
    }
}
