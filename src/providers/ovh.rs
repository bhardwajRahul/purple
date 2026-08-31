use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use sha1::{Digest, Sha1};

use super::{Provider, ProviderError, ProviderHost, map_ureq_error};

/// OVH API endpoints. Users pick from these in the region picker.
pub const OVH_ENDPOINTS: &[(&str, &str)] = &[
    ("eu", "Europe (eu.api.ovh.com)"),
    ("ca", "Canada (ca.api.ovh.com)"),
    ("us", "US (api.us.ovhcloud.com)"),
];

pub const OVH_ENDPOINT_GROUPS: &[(&str, usize, usize)] = &[("API Endpoint", 0, 3)];

pub struct Ovh {
    pub project: String,
    pub endpoint: String,
}

fn endpoint_url(endpoint: &str) -> &'static str {
    match endpoint {
        "ca" => "https://ca.api.ovh.com/1.0",
        "us" => "https://api.us.ovhcloud.com/1.0",
        _ => "https://eu.api.ovh.com/1.0",
    }
}

#[derive(Deserialize)]
struct OvhInstance {
    id: String,
    name: String,
    status: String,
    #[serde(default)]
    region: String,
    #[serde(rename = "ipAddresses", default)]
    ip_addresses: Vec<OvhIpAddress>,
    #[serde(default)]
    flavor: Option<OvhFlavor>,
    #[serde(default)]
    image: Option<OvhImage>,
}

#[derive(Deserialize)]
struct OvhIpAddress {
    ip: String,
    #[serde(rename = "type")]
    ip_type: String,
    version: u8,
}

#[derive(Deserialize)]
struct OvhFlavor {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct OvhImage {
    #[serde(default)]
    name: Option<String>,
}

/// Parse "app_key:app_secret:consumer_key" token format.
fn parse_token(token: &str) -> Result<(&str, &str, &str), ProviderError> {
    let parts: Vec<&str> = token.splitn(3, ':').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(ProviderError::AuthFailed);
    }
    Ok((parts[0], parts[1], parts[2]))
}

/// Compute OVH API signature.
/// Format: "$1$" + SHA1(app_secret + "+" + consumer_key + "+" + METHOD + "+" + url + "+" + body + "+" + timestamp)
fn sign_request(
    app_secret: &str,
    consumer_key: &str,
    method: &str,
    url: &str,
    body: &str,
    timestamp: u64,
) -> String {
    let pre_hash = format!(
        "{}+{}+{}+{}+{}+{}",
        app_secret, consumer_key, method, url, body, timestamp
    );
    let mut hasher = Sha1::new();
    hasher.update(pre_hash.as_bytes());
    let hash = hasher.finalize();
    format!("$1${}", hex_encode(&hash))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Select best IP: public IPv4 > public IPv6 > private IPv4.
fn select_ip(addresses: &[OvhIpAddress]) -> Option<String> {
    addresses
        .iter()
        .find(|a| a.ip_type == "public" && a.version == 4)
        .or_else(|| {
            addresses
                .iter()
                .find(|a| a.ip_type == "public" && a.version == 6)
        })
        .or_else(|| {
            addresses
                .iter()
                .find(|a| a.ip_type == "private" && a.version == 4)
        })
        .map(|a| super::strip_cidr(&a.ip).to_string())
}

impl Ovh {
    /// Fetch instances against an explicit API base. Production resolves the
    /// regional OVH host via `endpoint_url`; tests pass a mock URL so the
    /// server-time fetch, request signing, instance list and `ProviderHost`
    /// mapping all run end to end.
    fn fetch_from(
        &self,
        base: &str,
        token: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        let (app_key, app_secret, consumer_key) = parse_token(token)?;
        let agent = super::http_agent();

        if self.project.is_empty() {
            return Err(ProviderError::Execute(
                "OVH project ID is required. Set it in the provider config.".to_string(),
            ));
        }

        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Cancelled);
        }

        // Step 1: Get server time
        let time_url = format!("{}/auth/time", base);
        let server_time: u64 = agent
            .get(&time_url)
            .call()
            .map_err(map_ureq_error)?
            .body_mut()
            .read_json()
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Cancelled);
        }

        let instances_url = format!(
            "{}/cloud/project/{}/instance",
            base,
            super::percent_encode(&self.project)
        );

        let signature = sign_request(
            app_secret,
            consumer_key,
            "GET",
            &instances_url,
            "",
            server_time,
        );

        let instances: Vec<OvhInstance> = agent
            .get(&instances_url)
            .header("X-Ovh-Application", app_key)
            .header("X-Ovh-Timestamp", &server_time.to_string())
            .header("X-Ovh-Consumer", consumer_key)
            .header("X-Ovh-Signature", &signature)
            .header("Content-Type", "application/json;charset=utf-8")
            .call()
            .map_err(map_ureq_error)?
            .body_mut()
            .read_json()
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let mut hosts = Vec::with_capacity(instances.len());
        for instance in &instances {
            if let Some(ip) = select_ip(&instance.ip_addresses) {
                let mut metadata = super::ProviderMetadata::new();
                if !instance.region.is_empty() {
                    metadata.push("region", instance.region.clone());
                }
                if let Some(ref flavor) = instance.flavor
                    && !flavor.name.is_empty()
                {
                    metadata.push("type", flavor.name.clone());
                }
                if let Some(ref image) = instance.image
                    && let Some(ref name) = image.name
                    && !name.is_empty()
                {
                    metadata.push("image", name.clone());
                }
                if !instance.status.is_empty() {
                    metadata.push("status", instance.status.clone());
                }
                hosts.push(ProviderHost {
                    server_id: instance.id.clone(),
                    name: instance.name.clone(),
                    ip,
                    tags: Vec::new(),
                    metadata: metadata.finish(),
                    ..Default::default()
                });
            }
        }

        Ok(hosts)
    }
}

impl Provider for Ovh {
    fn name(&self) -> &str {
        "ovh"
    }

    fn short_label(&self) -> &str {
        "ovh"
    }

    fn fetch_hosts_cancellable(
        &self,
        token: &str,
        cancel: &AtomicBool,
        _env: &crate::runtime::env::Env,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        self.fetch_from(endpoint_url(&self.endpoint), token, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_token_valid() {
        let (ak, as_, ck) = parse_token("app-key:app-secret:consumer-key").unwrap();
        assert_eq!(ak, "app-key");
        assert_eq!(as_, "app-secret");
        assert_eq!(ck, "consumer-key");
    }

    #[test]
    fn test_parse_token_missing_part() {
        assert!(parse_token("key:secret").is_err());
    }

    #[test]
    fn test_parse_token_empty_part() {
        assert!(parse_token("key::consumer").is_err());
        assert!(parse_token(":secret:consumer").is_err());
    }

    #[test]
    fn test_parse_token_colon_in_consumer_key() {
        let (ak, as_, ck) = parse_token("key:secret:consumer:with:colons").unwrap();
        assert_eq!(ak, "key");
        assert_eq!(as_, "secret");
        assert_eq!(ck, "consumer:with:colons");
    }

    #[test]
    fn test_sign_request_format() {
        let sig = sign_request(
            "EgWIz07P0HYwtQDs",
            "MtSwSrPpNjqfVSmJhLbPyr2i45lSwPU1",
            "GET",
            "https://eu.api.ovh.com/1.0/cloud/project/abc/instance",
            "",
            1366560945,
        );
        assert!(sig.starts_with("$1$"), "signature must start with $1$");
        assert_eq!(sig.len(), 3 + 40, "should be $1$ + 40 hex chars");
        assert!(sig[3..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_request_deterministic() {
        let sig1 = sign_request("s", "c", "GET", "https://example.com", "", 12345);
        let sig2 = sign_request("s", "c", "GET", "https://example.com", "", 12345);
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_sign_request_different_timestamps() {
        let sig1 = sign_request("s", "c", "GET", "https://example.com", "", 1);
        let sig2 = sign_request("s", "c", "GET", "https://example.com", "", 2);
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode(&[0x00, 0xff]), "00ff");
    }

    #[test]
    fn test_parse_instance_response() {
        let json = r#"[
            {
                "id": "uuid-123",
                "name": "web-1",
                "status": "ACTIVE",
                "region": "GRA11",
                "ipAddresses": [
                    {"ip": "1.2.3.4", "type": "public", "version": 4},
                    {"ip": "10.0.0.1", "type": "private", "version": 4}
                ],
                "flavor": {"name": "b2-7"},
                "image": {"name": "Ubuntu 22.04"}
            }
        ]"#;
        let instances: Vec<OvhInstance> = serde_json::from_str(json).unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].id, "uuid-123");
        assert_eq!(instances[0].name, "web-1");
        assert_eq!(instances[0].status, "ACTIVE");
        assert_eq!(instances[0].region, "GRA11");
        assert_eq!(instances[0].ip_addresses.len(), 2);
        assert_eq!(instances[0].flavor.as_ref().unwrap().name, "b2-7");
        assert_eq!(
            instances[0].image.as_ref().unwrap().name.as_deref(),
            Some("Ubuntu 22.04")
        );
    }

    #[test]
    fn test_parse_instance_minimal_fields() {
        let json = r#"[{"id": "x", "name": "y", "status": "BUILD"}]"#;
        let instances: Vec<OvhInstance> = serde_json::from_str(json).unwrap();
        assert_eq!(instances.len(), 1);
        assert!(instances[0].ip_addresses.is_empty());
        assert!(instances[0].flavor.is_none());
        assert!(instances[0].image.is_none());
    }

    #[test]
    fn test_select_ip_prefers_public_ipv4() {
        let addrs = vec![
            OvhIpAddress {
                ip: "10.0.0.1".into(),
                ip_type: "private".into(),
                version: 4,
            },
            OvhIpAddress {
                ip: "1.2.3.4".into(),
                ip_type: "public".into(),
                version: 4,
            },
            OvhIpAddress {
                ip: "2001:db8::1".into(),
                ip_type: "public".into(),
                version: 6,
            },
        ];
        assert_eq!(select_ip(&addrs).unwrap(), "1.2.3.4");
    }

    #[test]
    fn test_select_ip_falls_back_to_public_ipv6() {
        let addrs = vec![
            OvhIpAddress {
                ip: "10.0.0.1".into(),
                ip_type: "private".into(),
                version: 4,
            },
            OvhIpAddress {
                ip: "2001:db8::1/64".into(),
                ip_type: "public".into(),
                version: 6,
            },
        ];
        assert_eq!(select_ip(&addrs).unwrap(), "2001:db8::1");
    }

    #[test]
    fn test_select_ip_falls_back_to_private_ipv4() {
        let addrs = vec![OvhIpAddress {
            ip: "10.0.0.1".into(),
            ip_type: "private".into(),
            version: 4,
        }];
        assert_eq!(select_ip(&addrs).unwrap(), "10.0.0.1");
    }

    #[test]
    fn test_select_ip_empty() {
        assert!(select_ip(&[]).is_none());
    }

    #[test]
    fn test_http_instances_roundtrip() {
        let mut server = mockito::Server::new();
        let time_mock = server
            .mock("GET", "/1.0/auth/time")
            .with_status(200)
            .with_body("1700000000")
            .create();

        let instances_mock = server
            .mock("GET", "/1.0/cloud/project/proj-123/instance")
            .match_header("X-Ovh-Application", "app-key")
            .match_header("X-Ovh-Consumer", "consumer-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[{
                    "id": "i-1",
                    "name": "web-1",
                    "status": "ACTIVE",
                    "region": "GRA11",
                    "ipAddresses": [{"ip": "1.2.3.4", "type": "public", "version": 4}],
                    "flavor": {"name": "b2-7"},
                    "image": {"name": "Ubuntu 22.04"}
                }]"#,
            )
            .create();

        let base_url = server.url();
        let token = "app-key:app-secret:consumer-key";
        let (app_key, app_secret, consumer_key) = parse_token(token).unwrap();
        let agent = super::super::http_agent();

        // Fetch time
        let time_url = format!("{}/1.0/auth/time", base_url);
        let server_time: u64 = agent
            .get(&time_url)
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        // Fetch instances
        let instances_url = format!("{}/1.0/cloud/project/proj-123/instance", base_url);
        let sig = sign_request(
            app_secret,
            consumer_key,
            "GET",
            &instances_url,
            "",
            server_time,
        );
        let instances: Vec<OvhInstance> = agent
            .get(&instances_url)
            .header("X-Ovh-Application", app_key)
            .header("X-Ovh-Timestamp", &server_time.to_string())
            .header("X-Ovh-Consumer", consumer_key)
            .header("X-Ovh-Signature", &sig)
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].name, "web-1");
        assert_eq!(select_ip(&instances[0].ip_addresses).unwrap(), "1.2.3.4");

        time_mock.assert();
        instances_mock.assert();
    }

    #[test]
    fn test_http_instances_auth_failure() {
        let mut server = mockito::Server::new();
        let time_mock = server
            .mock("GET", "/1.0/auth/time")
            .with_status(200)
            .with_body("1700000000")
            .create();

        let instances_mock = server
            .mock("GET", "/1.0/cloud/project/proj-123/instance")
            .with_status(401)
            .with_body(r#"{"message": "Invalid credentials"}"#)
            .create();

        let agent = super::super::http_agent();
        let base_url = server.url();

        let _: u64 = agent
            .get(&format!("{}/1.0/auth/time", base_url))
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        let result = agent
            .get(&format!("{}/1.0/cloud/project/proj-123/instance", base_url))
            .call();

        assert!(result.is_err());
        let err = super::map_ureq_error(result.unwrap_err());
        assert!(matches!(err, ProviderError::AuthFailed));

        time_mock.assert();
        instances_mock.assert();
    }

    #[test]
    fn test_rejects_empty_project() {
        let ovh = Ovh {
            project: String::new(),
            endpoint: String::new(),
        };
        let cancel = AtomicBool::new(false);
        let result =
            ovh.fetch_hosts_cancellable("ak:as:ck", &cancel, &crate::runtime::env::Env::empty());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("project ID is required"));
    }

    #[test]
    fn test_rejects_invalid_token_before_network() {
        let ovh = Ovh {
            project: "proj".to_string(),
            endpoint: String::new(),
        };
        let cancel = AtomicBool::new(false);
        let result =
            ovh.fetch_hosts_cancellable("bad-token", &cancel, &crate::runtime::env::Env::empty());
        assert!(matches!(result.unwrap_err(), ProviderError::AuthFailed));
    }

    #[test]
    fn test_endpoint_url_eu() {
        assert_eq!(endpoint_url("eu"), "https://eu.api.ovh.com/1.0");
        assert_eq!(endpoint_url(""), "https://eu.api.ovh.com/1.0");
        assert_eq!(endpoint_url("unknown"), "https://eu.api.ovh.com/1.0");
    }

    #[test]
    fn test_endpoint_url_ca() {
        assert_eq!(endpoint_url("ca"), "https://ca.api.ovh.com/1.0");
    }

    #[test]
    fn test_endpoint_url_us() {
        assert_eq!(endpoint_url("us"), "https://api.us.ovhcloud.com/1.0");
    }

    #[test]
    fn test_sign_request_known_vector() {
        // OVH documentation reference vector
        let sig = sign_request(
            "EgWIz07P0HYwtQDs",
            "MtSwSrPpNjqfVSmJhLbPyr2i45lSwPU1",
            "GET",
            "https://eu.api.ovh.com/1.0/auth/time",
            "",
            1366560945,
        );
        assert_eq!(sig, "$1$069f8fd9c1fbec55d67f24f80e65cb1a14f09dce");
    }

    #[test]
    fn test_sign_request_with_body() {
        let sig_empty = sign_request("s", "c", "GET", "https://x.com", "", 1);
        let sig_body = sign_request("s", "c", "POST", "https://x.com", r#"{"key":"val"}"#, 1);
        assert_ne!(sig_empty, sig_body);
    }

    #[test]
    fn test_sign_request_different_methods() {
        let get = sign_request("s", "c", "GET", "https://x.com", "", 1);
        let post = sign_request("s", "c", "POST", "https://x.com", "", 1);
        assert_ne!(get, post);
    }

    #[test]
    fn test_parse_token_empty_string() {
        assert!(parse_token("").is_err());
    }

    #[test]
    fn test_parse_token_only_colons() {
        assert!(parse_token("::").is_err());
    }

    #[test]
    fn test_parse_token_trailing_colon() {
        assert!(parse_token("key:secret:").is_err());
    }

    #[test]
    fn test_parse_instance_extra_fields_ignored() {
        let json = r#"[{
            "id": "uuid-123",
            "name": "web-1",
            "status": "ACTIVE",
            "created": "2024-01-15T10:30:00Z",
            "planCode": "d2-2.runabove",
            "monthlyBilling": null,
            "sshKey": {"id": "key-1"},
            "currentMonthOutgoingTraffic": 12345,
            "operationIds": [],
            "ipAddresses": [{"ip": "1.2.3.4", "type": "public", "version": 4, "gatewayIp": "1.2.3.1", "networkId": "net-1"}],
            "flavor": {"name": "b2-7", "available": true, "disk": 50, "ram": 7168, "vcpus": 2},
            "image": {"name": "Ubuntu 22.04", "type": "linux", "user": "ubuntu", "visibility": "public"}
        }]"#;
        let instances: Vec<OvhInstance> = serde_json::from_str(json).unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].name, "web-1");
    }

    #[test]
    fn test_parse_instance_null_flavor_and_image() {
        let json =
            r#"[{"id": "x", "name": "y", "status": "BUILD", "flavor": null, "image": null}]"#;
        let instances: Vec<OvhInstance> = serde_json::from_str(json).unwrap();
        assert!(instances[0].flavor.is_none());
        assert!(instances[0].image.is_none());
    }

    #[test]
    fn test_parse_empty_instance_list() {
        let instances: Vec<OvhInstance> = serde_json::from_str("[]").unwrap();
        assert!(instances.is_empty());
    }

    #[test]
    fn test_select_ip_private_ipv6_only_returns_none() {
        let addrs = vec![OvhIpAddress {
            ip: "fd00::1".into(),
            ip_type: "private".into(),
            version: 6,
        }];
        assert!(select_ip(&addrs).is_none());
    }

    #[test]
    fn test_select_ip_unknown_type_returns_none() {
        let addrs = vec![OvhIpAddress {
            ip: "1.2.3.4".into(),
            ip_type: "floating".into(),
            version: 4,
        }];
        assert!(select_ip(&addrs).is_none());
    }

    #[test]
    fn test_select_ip_public_ipv4_with_cidr() {
        let addrs = vec![OvhIpAddress {
            ip: "1.2.3.4/32".into(),
            ip_type: "public".into(),
            version: 4,
        }];
        assert_eq!(select_ip(&addrs).unwrap(), "1.2.3.4");
    }

    #[test]
    fn test_select_ip_multiple_public_ipv4_uses_first() {
        let addrs = vec![
            OvhIpAddress {
                ip: "1.1.1.1".into(),
                ip_type: "public".into(),
                version: 4,
            },
            OvhIpAddress {
                ip: "2.2.2.2".into(),
                ip_type: "public".into(),
                version: 4,
            },
        ];
        assert_eq!(select_ip(&addrs).unwrap(), "1.1.1.1");
    }

    #[test]
    fn test_metadata_all_fields_present() {
        let json = r#"[{
            "id": "i-1", "name": "web", "status": "ACTIVE", "region": "GRA11",
            "ipAddresses": [{"ip": "1.2.3.4", "type": "public", "version": 4}],
            "flavor": {"name": "b2-7"},
            "image": {"name": "Ubuntu 22.04"}
        }]"#;
        let instances: Vec<OvhInstance> = serde_json::from_str(json).unwrap();
        let inst = &instances[0];
        // Simulate the metadata assembly from fetch_hosts_cancellable
        let mut metadata = super::super::ProviderMetadata::new();
        if !inst.region.is_empty() {
            metadata.push("region", inst.region.clone());
        }
        if let Some(ref flavor) = inst.flavor
            && !flavor.name.is_empty()
        {
            metadata.push("type", flavor.name.clone());
        }
        if let Some(ref image) = inst.image
            && let Some(ref name) = image.name
            && !name.is_empty()
        {
            metadata.push("image", name.clone());
        }
        if !inst.status.is_empty() {
            metadata.push("status", inst.status.clone());
        }
        let metadata = metadata.finish();
        assert_eq!(metadata.len(), 4);
        assert_eq!(metadata[0], ("region".to_string(), "GRA11".to_string()));
        assert_eq!(metadata[1], ("type".to_string(), "b2-7".to_string()));
        assert_eq!(
            metadata[2],
            ("image".to_string(), "Ubuntu 22.04".to_string())
        );
        assert_eq!(metadata[3], ("status".to_string(), "ACTIVE".to_string()));
    }

    #[test]
    fn test_metadata_no_optional_fields() {
        let json = r#"[{"id": "i-1", "name": "web", "status": "", "region": ""}]"#;
        let instances: Vec<OvhInstance> = serde_json::from_str(json).unwrap();
        let inst = &instances[0];
        let mut metadata = super::super::ProviderMetadata::new();
        if !inst.region.is_empty() {
            metadata.push("region", inst.region.clone());
        }
        if let Some(ref flavor) = inst.flavor
            && !flavor.name.is_empty()
        {
            metadata.push("type", flavor.name.clone());
        }
        if !inst.status.is_empty() {
            metadata.push("status", inst.status.clone());
        }
        assert!(metadata.finish().is_empty());
    }

    #[test]
    fn test_instance_no_ip_skipped() {
        let json = r#"[
            {"id": "i-1", "name": "has-ip", "status": "ACTIVE", "ipAddresses": [{"ip": "1.2.3.4", "type": "public", "version": 4}]},
            {"id": "i-2", "name": "no-ip", "status": "ACTIVE", "ipAddresses": []},
            {"id": "i-3", "name": "private-v6-only", "status": "ACTIVE", "ipAddresses": [{"ip": "fd00::1", "type": "private", "version": 6}]}
        ]"#;
        let instances: Vec<OvhInstance> = serde_json::from_str(json).unwrap();
        let hosts: Vec<_> = instances
            .iter()
            .filter_map(|inst| select_ip(&inst.ip_addresses).map(|ip| (inst.name.clone(), ip)))
            .collect();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].0, "has-ip");
    }

    #[test]
    fn test_name_and_short_label() {
        let ovh = Ovh {
            project: String::new(),
            endpoint: String::new(),
        };
        assert_eq!(ovh.name(), "ovh");
        assert_eq!(ovh.short_label(), "ovh");
    }

    #[test]
    fn test_cancellation_returns_cancelled() {
        let cancel = AtomicBool::new(true);
        let ovh = Ovh {
            project: "test-project".to_string(),
            endpoint: String::new(),
        };
        let result =
            ovh.fetch_hosts_cancellable("AK:AS:CK", &cancel, &crate::runtime::env::Env::empty());
        assert!(matches!(result, Err(ProviderError::Cancelled)));
    }

    #[test]
    fn fetch_from_drives_full_pipeline_against_mock() {
        // Drives the production pipeline through the API base seam: the
        // server-time fetch, OVH request signing, the instance list and
        // ProviderHost mapping all run against a mock server.
        let mut server = mockito::Server::new();
        let time = server
            .mock("GET", "/auth/time")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("1700000000")
            .create();
        let instances = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/cloud/project/.*/instance".into()),
            )
            .match_header("X-Ovh-Application", "app-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[{
                    "id": "inst-1",
                    "name": "web-1",
                    "status": "ACTIVE",
                    "region": "GRA7",
                    "ipAddresses": [
                        {"ip": "10.0.0.2", "type": "private", "version": 4},
                        {"ip": "51.91.1.2", "type": "public", "version": 4}
                    ],
                    "flavor": {"name": "b2-7"}
                }]"#,
            )
            .create();

        let ovh = Ovh {
            project: "my-project".to_string(),
            endpoint: String::new(),
        };
        let hosts = ovh
            .fetch_from(
                &server.url(),
                "app-key:app-secret:consumer-key",
                &AtomicBool::new(false),
            )
            .expect("fetch_from must succeed against the mock");
        time.assert();
        instances.assert();

        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].server_id, "inst-1");
        assert_eq!(hosts[0].name, "web-1");
        assert_eq!(hosts[0].ip, "51.91.1.2");
        assert!(
            hosts[0]
                .metadata
                .contains(&("region".to_string(), "GRA7".to_string()))
        );
    }

    #[test]
    fn fetch_from_maps_auth_failure_to_provider_error() {
        let mut server = mockito::Server::new();
        let _time = server
            .mock("GET", "/auth/time")
            .with_status(200)
            .with_body("1700000000")
            .create();
        let mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"/cloud/project/.*/instance".into()),
            )
            .with_status(401)
            .with_body(r#"{"message": "Invalid signature"}"#)
            .create();

        let ovh = Ovh {
            project: "my-project".to_string(),
            endpoint: String::new(),
        };
        let result = ovh.fetch_from(
            &server.url(),
            "app-key:app-secret:consumer-key",
            &AtomicBool::new(false),
        );
        mock.assert();
        assert!(
            matches!(result, Err(ProviderError::AuthFailed)),
            "401 must map to AuthFailed, got {result:?}"
        );
    }
}
