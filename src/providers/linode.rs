use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use super::{Provider, ProviderError, ProviderHost, map_ureq_error};

pub struct Linode;

#[derive(Deserialize)]
struct LinodeResponse {
    data: Vec<LinodeInstance>,
    page: u64,
    pages: u64,
}

#[derive(Deserialize)]
struct LinodeInstance {
    id: u64,
    label: String,
    #[serde(default)]
    ipv4: Vec<String>,
    #[serde(default)]
    ipv6: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    region: String,
    #[serde(default, rename = "type")]
    instance_type: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    image: Option<String>,
}

/// Check if an IP address is in a private/reserved range.
fn is_private_ip(ip: &str) -> bool {
    ip.starts_with("10.")
        || ip.starts_with("192.168.")
        || ip.starts_with("127.")
        || (ip.starts_with("172.")
            && ip
                .split('.')
                .nth(1)
                .and_then(|s| s.parse::<u8>().ok())
                .is_some_and(|n| (16..=31).contains(&n)))
        || (ip.starts_with("100.")
            && ip
                .split('.')
                .nth(1)
                .and_then(|s| s.parse::<u8>().ok())
                .is_some_and(|n| (64..=127).contains(&n)))
}

impl Linode {
    /// Real API host. Overridable per call via `fetch_from` so tests can point
    /// the full fetch pipeline at a mock server.
    const API_BASE: &'static str = "https://api.linode.com";

    /// Fetch hosts against an explicit API base. Production passes `API_BASE`;
    /// tests pass a mock server URL. Single seam making URL construction, the
    /// auth header, error mapping, pagination and `ProviderHost` mapping
    /// testable end to end.
    fn fetch_from(
        &self,
        base_url: &str,
        token: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        let agent = super::http_agent();

        super::paginate(cancel, |idx| {
            let page = idx + 1;
            let url = format!(
                "{}/v4/linode/instances?page={}&page_size=500",
                base_url, page
            );
            let resp: LinodeResponse = agent
                .get(&url)
                .header("Authorization", &super::bearer_auth(token))
                .call()
                .map_err(map_ureq_error)?
                .body_mut()
                .read_json()
                .map_err(|e| ProviderError::Parse(e.to_string()))?;

            let mut hosts = Vec::new();
            for instance in &resp.data {
                // Prefer public IPv4; fall back to private IPv4, then IPv6
                let ip = instance
                    .ipv4
                    .iter()
                    .find(|ip| !is_private_ip(ip))
                    .or_else(|| instance.ipv4.first())
                    .cloned()
                    .or_else(|| {
                        instance
                            .ipv6
                            .as_ref()
                            .filter(|v| !v.is_empty())
                            .map(|v| super::strip_cidr(v).to_string())
                    });
                if let Some(ip) = ip
                    && !ip.is_empty()
                {
                    let mut metadata = super::ProviderMetadata::new();
                    if !instance.region.is_empty() {
                        metadata.push("region", instance.region.clone());
                    }
                    if !instance.instance_type.is_empty() {
                        metadata.push("plan", instance.instance_type.clone());
                    }
                    if let Some(ref image) = instance.image
                        && !image.is_empty()
                    {
                        metadata.push("image", image.clone());
                    }
                    if !instance.status.is_empty() {
                        metadata.push("status", instance.status.clone());
                    }
                    hosts.push(ProviderHost {
                        server_id: instance.id.to_string(),
                        name: instance.label.clone(),
                        ip,
                        tags: instance.tags.clone(),
                        metadata: metadata.finish(),
                        ..Default::default()
                    });
                }
            }

            let more = !resp.data.is_empty() && resp.page < resp.pages;
            Ok(super::PageResult { hosts, more })
        })
    }
}

impl Provider for Linode {
    fn name(&self) -> &str {
        "linode"
    }

    fn short_label(&self) -> &str {
        "linode"
    }

    fn fetch_hosts_cancellable(
        &self,
        token: &str,
        cancel: &AtomicBool,
        _env: &crate::runtime::env::Env,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        self.fetch_from(Self::API_BASE, token, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_private_ip() {
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("192.168.1.1"));
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
        assert!(is_private_ip("100.64.0.1"));
        assert!(is_private_ip("127.0.0.1"));
        assert!(!is_private_ip("1.2.3.4"));
        assert!(!is_private_ip("172.15.0.1"));
        assert!(!is_private_ip("172.32.0.1"));
        assert!(!is_private_ip("100.63.0.1"));
    }

    #[test]
    fn test_parse_linode_prefers_public_ip() {
        let json = r#"{
            "data": [
                {
                    "id": 111,
                    "label": "mixed-ips",
                    "ipv4": ["192.168.1.1", "5.6.7.8"],
                    "tags": []
                }
            ],
            "page": 1,
            "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        let instance = &resp.data[0];
        let ip = instance
            .ipv4
            .iter()
            .find(|ip| !is_private_ip(ip))
            .or_else(|| instance.ipv4.first());
        assert_eq!(ip.unwrap(), "5.6.7.8");
    }

    #[test]
    fn test_parse_linode_response() {
        let json = r#"{
            "data": [
                {
                    "id": 111,
                    "label": "app-server",
                    "ipv4": ["9.8.7.6", "192.168.1.1"],
                    "tags": ["production"]
                },
                {
                    "id": 222,
                    "label": "no-ip-server",
                    "ipv4": [],
                    "tags": []
                }
            ],
            "page": 1,
            "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 2);
        assert_eq!(resp.data[0].label, "app-server");
        assert_eq!(resp.data[0].ipv4[0], "9.8.7.6");
        assert!(resp.data[1].ipv4.is_empty());
    }

    // Helper: same IP selection logic as fetch_hosts_cancellable
    fn select_linode_ip(instance: &LinodeInstance) -> Option<String> {
        instance
            .ipv4
            .iter()
            .find(|ip| !is_private_ip(ip))
            .or_else(|| instance.ipv4.first())
            .cloned()
            .or_else(|| {
                instance
                    .ipv6
                    .as_ref()
                    .filter(|v| !v.is_empty())
                    .map(|v| crate::providers::strip_cidr(v).to_string())
            })
    }

    #[test]
    fn test_is_private_ip_100_range_boundary() {
        // 100.64-127 is CGNAT (private), 100.63 and 100.128 are public
        assert!(is_private_ip("100.64.0.1"));
        assert!(is_private_ip("100.127.255.255"));
        assert!(!is_private_ip("100.63.255.255"));
        assert!(!is_private_ip("100.128.0.1"));
    }

    #[test]
    fn test_is_private_ip_172_range_boundary() {
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.0.1"));
        assert!(!is_private_ip("172.15.0.1"));
        assert!(!is_private_ip("172.32.0.1"));
    }

    #[test]
    fn test_linode_private_only_falls_back_to_private() {
        let json = r#"{
            "data": [{"id": 1, "label": "private-only", "ipv4": ["192.168.1.1"], "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        // When no public IP, falls back to first private IP
        assert_eq!(
            select_linode_ip(&resp.data[0]),
            Some("192.168.1.1".to_string())
        );
    }

    #[test]
    fn test_linode_no_ips_at_all() {
        let json = r#"{
            "data": [{"id": 1, "label": "empty", "ipv4": [], "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_linode_ip(&resp.data[0]), None);
    }

    #[test]
    fn test_linode_ipv6_null() {
        let json = r#"{
            "data": [{"id": 1, "label": "null-v6", "ipv4": [], "ipv6": null, "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_linode_ip(&resp.data[0]), None);
    }

    #[test]
    fn test_linode_ipv6_empty_string() {
        let json = r#"{
            "data": [{"id": 1, "label": "empty-v6", "ipv4": [], "ipv6": "", "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_linode_ip(&resp.data[0]), None);
    }

    #[test]
    fn test_linode_pagination_continues() {
        let json = r#"{
            "data": [{"id": 1, "label": "a", "ipv4": ["1.1.1.1"], "tags": []}],
            "page": 1, "pages": 5
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.page < resp.pages);
    }

    #[test]
    fn test_linode_pagination_stops() {
        let json = r#"{
            "data": [{"id": 1, "label": "a", "ipv4": ["1.1.1.1"], "tags": []}],
            "page": 5, "pages": 5
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.page >= resp.pages);
    }

    #[test]
    fn test_linode_tags_preserved() {
        let json = r#"{
            "data": [{"id": 1, "label": "tagged", "ipv4": ["1.1.1.1"], "tags": ["web", "prod"]}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data[0].tags, vec!["web", "prod"]);
    }

    #[test]
    fn test_linode_multiple_public_ips_uses_first() {
        let json = r#"{
            "data": [{"id": 1, "label": "multi", "ipv4": ["1.2.3.4", "5.6.7.8"], "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_linode_ip(&resp.data[0]), Some("1.2.3.4".to_string()));
    }

    #[test]
    fn test_linode_ipv6_cidr_stripped() {
        let json = r#"{
            "data": [{"id": 1, "label": "v6-cidr", "ipv4": [], "ipv6": "2600:3c00::1/128", "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_linode_ip(&resp.data[0]),
            Some("2600:3c00::1".to_string())
        );
    }

    #[test]
    fn test_linode_ipv6_no_cidr() {
        let json = r#"{
            "data": [{"id": 1, "label": "v6-bare", "ipv4": [], "ipv6": "2600:3c00::1", "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_linode_ip(&resp.data[0]),
            Some("2600:3c00::1".to_string())
        );
    }

    #[test]
    fn test_linode_public_ipv4_preferred_over_ipv6() {
        let json = r#"{
            "data": [{
                "id": 1, "label": "dual",
                "ipv4": ["1.2.3.4"],
                "ipv6": "2600:3c00::1/128",
                "tags": []
            }],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        // Public IPv4 is not private, so it wins over IPv6
        assert_eq!(select_linode_ip(&resp.data[0]), Some("1.2.3.4".to_string()));
    }

    #[test]
    fn test_linode_missing_ipv6_field() {
        // When ipv6 is not in the JSON at all
        let json = r#"{
            "data": [{"id": 1, "label": "no-v6", "ipv4": ["5.6.7.8"], "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data[0].ipv6, None);
        assert_eq!(select_linode_ip(&resp.data[0]), Some("5.6.7.8".to_string()));
    }

    #[test]
    fn test_linode_empty_label() {
        let json = r#"{
            "data": [{"id": 1, "label": "", "ipv4": ["1.2.3.4"], "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data[0].label, "");
    }

    #[test]
    fn test_linode_default_tags_empty() {
        let json = r#"{
            "data": [{"id": 1, "label": "a", "ipv4": ["1.1.1.1"]}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data[0].tags.is_empty());
    }

    #[test]
    fn test_linode_cgnat_100_64_is_private() {
        // CGNAT range: 100.64.0.0 - 100.127.255.255
        assert!(is_private_ip("100.64.0.0"));
        assert!(is_private_ip("100.100.50.25"));
        assert!(is_private_ip("100.127.255.255"));
    }

    #[test]
    fn test_linode_large_id() {
        let json = r#"{
            "data": [{"id": 99999999999, "label": "big", "ipv4": ["1.2.3.4"], "tags": []}],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data[0].id, 99999999999);
    }

    #[test]
    fn test_linode_empty_data_stops_pagination() {
        let json = r#"{
            "data": [],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.is_empty());
    }

    #[test]
    fn test_linode_private_ip_first_then_public() {
        // API may return private IP before public in ipv4 array
        let json = r#"{
            "data": [{
                "id": 1, "label": "mixed",
                "ipv4": ["192.168.1.1", "10.0.0.1", "8.8.8.8"],
                "tags": []
            }],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        // Should find 8.8.8.8 as first non-private IP
        assert_eq!(select_linode_ip(&resp.data[0]), Some("8.8.8.8".to_string()));
    }

    #[test]
    fn test_is_private_ip_loopback() {
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("127.255.255.255"));
    }

    #[test]
    fn test_is_private_ip_public_ranges() {
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("1.1.1.1"));
        assert!(!is_private_ip("203.0.113.1"));
        assert!(!is_private_ip("198.51.100.1"));
    }

    #[test]
    fn test_is_private_ip_172_all_boundary_octets() {
        // 172.16-31 are private
        for n in 16..=31 {
            assert!(
                is_private_ip(&format!("172.{}.0.1", n)),
                "172.{}.0.1 should be private",
                n
            );
        }
        // 172.0-15 and 172.32+ are public
        assert!(!is_private_ip("172.0.0.1"));
        assert!(!is_private_ip("172.15.255.255"));
        assert!(!is_private_ip("172.32.0.1"));
        assert!(!is_private_ip("172.255.0.1"));
    }

    #[test]
    fn test_linode_all_private_falls_back_to_first() {
        let json = r#"{
            "data": [{
                "id": 1, "label": "all-private",
                "ipv4": ["10.0.0.1", "192.168.1.1", "172.16.0.1"],
                "tags": []
            }],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        // No public IP found, falls back to first in list
        assert_eq!(
            select_linode_ip(&resp.data[0]),
            Some("10.0.0.1".to_string())
        );
    }

    #[test]
    fn test_linode_private_v4_and_v6_prefers_private_v4() {
        // When only private IPv4 and public IPv6 available, private IPv4 wins
        // because the code prefers any v4 (public or private) over v6
        let json = r#"{
            "data": [{
                "id": 1, "label": "priv-v4-pub-v6",
                "ipv4": ["192.168.1.1"],
                "ipv6": "2600:3c00::1/128",
                "tags": []
            }],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        // Private v4 wins over public v6 (fallback to first v4)
        assert_eq!(
            select_linode_ip(&resp.data[0]),
            Some("192.168.1.1".to_string())
        );
    }

    #[test]
    fn test_linode_multiple_instances_parsed() {
        let json = r#"{
            "data": [
                {"id": 1, "label": "web-1", "ipv4": ["1.1.1.1"], "tags": ["web"]},
                {"id": 2, "label": "web-2", "ipv4": ["2.2.2.2"], "tags": ["web"]},
                {"id": 3, "label": "db", "ipv4": [], "ipv6": "2600::1/128", "tags": ["db"]}
            ],
            "page": 1, "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 3);
        assert_eq!(select_linode_ip(&resp.data[0]), Some("1.1.1.1".to_string()));
        assert_eq!(select_linode_ip(&resp.data[2]), Some("2600::1".to_string()));
    }

    #[test]
    fn test_ipv6_only_instance_uses_v6() {
        let json = r#"{
            "data": [
                {
                    "id": 333,
                    "label": "v6-only",
                    "ipv4": [],
                    "ipv6": "2600:3c00::1/128",
                    "tags": []
                }
            ],
            "page": 1,
            "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        let instance = &resp.data[0];
        let ip = instance
            .ipv4
            .iter()
            .find(|ip| !is_private_ip(ip))
            .or_else(|| instance.ipv4.first())
            .cloned()
            .or_else(|| {
                instance
                    .ipv6
                    .as_ref()
                    .filter(|v| !v.is_empty())
                    .map(|v| crate::providers::strip_cidr(v).to_string())
            });
        // CIDR suffix must be stripped for SSH compatibility
        assert_eq!(ip, Some("2600:3c00::1".to_string()));
    }

    // --- empty ipv4 and empty ipv6 → None ---

    #[test]
    fn test_linode_empty_ipv4_empty_ipv6_returns_none() {
        let json = r#"{
            "data": [{
                "id": 1,
                "label": "no-ip",
                "ipv4": [],
                "ipv6": "",
                "tags": []
            }],
            "page": 1,
            "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        let instance = &resp.data[0];
        let ip = instance
            .ipv4
            .iter()
            .find(|ip| !is_private_ip(ip))
            .or_else(|| instance.ipv4.first())
            .cloned()
            .or_else(|| {
                instance
                    .ipv6
                    .as_ref()
                    .filter(|v| !v.is_empty())
                    .map(|v| crate::providers::strip_cidr(v).to_string())
            });
        assert_eq!(ip, None);
    }

    // --- ipv6 field omitted (defaults to None) → falls through ---

    #[test]
    fn test_linode_ipv6_field_omitted_falls_to_private() {
        let json = r#"{
            "data": [{
                "id": 2,
                "label": "null-v6",
                "ipv4": ["10.0.0.1"],
                "tags": []
            }],
            "page": 1,
            "pages": 1
        }"#;
        let resp: LinodeResponse = serde_json::from_str(json).unwrap();
        let instance = &resp.data[0];
        assert!(instance.ipv6.is_none());
        // Should fall back to private ipv4
        let ip = instance
            .ipv4
            .iter()
            .find(|ip| !is_private_ip(ip))
            .or_else(|| instance.ipv4.first())
            .cloned();
        assert_eq!(ip, Some("10.0.0.1".to_string()));
    }

    // --- 100.63 is NOT CGNAT (boundary) ---

    #[test]
    fn test_is_private_ip_100_63_not_cgnat() {
        assert!(!is_private_ip("100.63.0.1"));
    }

    // --- 100.128 is NOT CGNAT (boundary) ---

    #[test]
    fn test_is_private_ip_100_128_not_cgnat() {
        assert!(!is_private_ip("100.128.0.1"));
    }

    // --- 172.15 is NOT private (below range) ---

    #[test]
    fn test_is_private_ip_172_15_not_private() {
        assert!(!is_private_ip("172.15.0.1"));
    }

    // --- 172.32 is NOT private (above range) ---

    #[test]
    fn test_is_private_ip_172_32_not_private() {
        assert!(!is_private_ip("172.32.0.1"));
    }

    // --- non-numeric second octet for 172.x ---

    #[test]
    fn test_is_private_ip_172_nonnumeric() {
        assert!(!is_private_ip("172.abc.0.1"));
    }

    // ── HTTP roundtrip tests (mockito) ──────────────────────────────

    #[test]
    fn test_http_instances_roundtrip() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v4/linode/instances")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("page_size".into(), "500".into()),
            ]))
            .match_header("Authorization", "Bearer test-linode-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": [
                        {
                            "id": 12345678,
                            "label": "web-prod-1",
                            "ipv4": ["45.33.100.10", "192.168.200.1"],
                            "ipv6": "2600:3c00::f03c:92ff:fe12:3456/128",
                            "tags": ["prod", "web"],
                            "region": "us-east",
                            "type": "g6-standard-2",
                            "status": "running",
                            "image": "linode/ubuntu22.04"
                        }
                    ],
                    "page": 1,
                    "pages": 1
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        let url = format!("{}/v4/linode/instances?page=1&page_size=500", server.url());
        let resp: LinodeResponse = agent
            .get(&url)
            .header("Authorization", "Bearer test-linode-token")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(resp.data.len(), 1);
        let i = &resp.data[0];
        assert_eq!(i.id, 12345678);
        assert_eq!(i.label, "web-prod-1");
        assert_eq!(i.ipv4, vec!["45.33.100.10", "192.168.200.1"]);
        assert_eq!(
            i.ipv6.as_deref(),
            Some("2600:3c00::f03c:92ff:fe12:3456/128")
        );
        assert_eq!(i.tags, vec!["prod", "web"]);
        assert_eq!(i.region, "us-east");
        assert_eq!(i.instance_type, "g6-standard-2");
        assert_eq!(i.status, "running");
        assert_eq!(i.image.as_deref(), Some("linode/ubuntu22.04"));
        assert_eq!(resp.page, 1);
        assert_eq!(resp.pages, 1);
        mock.assert();
    }

    #[test]
    fn test_http_instances_pagination() {
        let mut server = mockito::Server::new();
        let page1 = server
            .mock("GET", "/v4/linode/instances")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("page_size".into(), "500".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": [{"id": 1, "label": "a", "ipv4": ["1.1.1.1"], "tags": []}],
                    "page": 1,
                    "pages": 2
                }"#,
            )
            .create();
        let page2 = server
            .mock("GET", "/v4/linode/instances")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "2".into()),
                mockito::Matcher::UrlEncoded("page_size".into(), "500".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": [{"id": 2, "label": "b", "ipv4": ["2.2.2.2"], "tags": []}],
                    "page": 2,
                    "pages": 2
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        // Page 1
        let r1: LinodeResponse = agent
            .get(&format!(
                "{}/v4/linode/instances?page=1&page_size=500",
                server.url()
            ))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r1.data.len(), 1);
        assert_eq!(r1.page, 1);
        assert_eq!(r1.pages, 2);
        // Page 2
        let r2: LinodeResponse = agent
            .get(&format!(
                "{}/v4/linode/instances?page=2&page_size=500",
                server.url()
            ))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r2.data.len(), 1);
        assert_eq!(r2.page, 2);
        assert_eq!(r2.pages, 2);
        page1.assert();
        page2.assert();
    }

    #[test]
    fn test_http_instances_auth_failure() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v4/linode/instances")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"errors": [{"reason": "Invalid Token"}]}"#)
            .create();

        let agent = super::super::http_agent();
        let result = agent
            .get(&format!(
                "{}/v4/linode/instances?page=1&page_size=500",
                server.url()
            ))
            .header("Authorization", "Bearer bad-token")
            .call();

        match result {
            Err(ureq::Error::StatusCode(401)) => {} // expected
            other => panic!("expected 401 error, got {:?}", other),
        }
        mock.assert();
    }

    #[test]
    fn fetch_from_drives_full_pipeline_against_mock() {
        // Exercises the production fetch path end to end: URL construction,
        // auth header, JSON deserialize, ProviderHost mapping and pagination
        // termination through fetch_hosts_cancellable's seam.
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v4/linode/instances")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("page_size".into(), "500".into()),
            ]))
            .match_header("Authorization", "Bearer tk-42")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": [
                        {
                            "id": 12345678,
                            "label": "web-prod-1",
                            "ipv4": ["192.168.200.1", "45.33.100.10"],
                            "ipv6": "2600:3c00::f03c:92ff:fe12:3456/128",
                            "tags": ["prod", "web"],
                            "region": "us-east",
                            "type": "g6-standard-2",
                            "status": "running",
                            "image": "linode/ubuntu22.04"
                        }
                    ],
                    "page": 1,
                    "pages": 1
                }"#,
            )
            .create();

        let hosts = Linode
            .fetch_from(&server.url(), "tk-42", &AtomicBool::new(false))
            .expect("fetch_from must succeed against the mock");
        mock.assert();

        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].server_id, "12345678");
        assert_eq!(hosts[0].name, "web-prod-1");
        // Public IPv4 preferred over the private entry and the IPv6.
        assert_eq!(hosts[0].ip, "45.33.100.10");
        assert_eq!(hosts[0].tags, vec!["prod", "web"]);
        assert!(
            hosts[0]
                .metadata
                .contains(&("region".to_string(), "us-east".to_string()))
        );
    }

    #[test]
    fn fetch_from_maps_auth_failure_to_provider_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v4/linode/instances")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"errors": [{"reason": "Invalid Token"}]}"#)
            .create();

        let result = Linode.fetch_from(&server.url(), "bad", &AtomicBool::new(false));
        mock.assert();
        assert!(
            matches!(result, Err(ProviderError::AuthFailed)),
            "401 must map to ProviderError::AuthFailed, got {result:?}"
        );
    }
}
