use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use super::{Provider, ProviderError, ProviderHost, map_ureq_error};

pub struct Hetzner;

#[derive(Deserialize)]
struct HetznerResponse {
    servers: Vec<HetznerServer>,
    meta: HetznerMeta,
}

#[derive(Deserialize)]
struct HetznerServer {
    id: u64,
    name: String,
    public_net: PublicNet,
    #[serde(default)]
    labels: std::collections::HashMap<String, String>,
    #[serde(default)]
    server_type: Option<HetznerServerType>,
    #[serde(default)]
    datacenter: Option<HetznerDatacenter>,
    #[serde(default)]
    location: Option<HetznerLocation>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    image: Option<HetznerImage>,
}

#[derive(Deserialize)]
struct HetznerImage {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct HetznerServerType {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct HetznerDatacenter {
    #[serde(default)]
    location: Option<HetznerLocation>,
}

#[derive(Deserialize)]
struct HetznerLocation {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct PublicNet {
    ipv4: Option<IpInfo>,
    #[serde(default)]
    ipv6: Option<IpInfo>,
}

#[derive(Deserialize)]
struct IpInfo {
    ip: String,
}

#[derive(Deserialize)]
struct HetznerMeta {
    pagination: Pagination,
}

#[derive(Deserialize)]
struct Pagination {
    page: u64,
    last_page: u64,
}

impl Hetzner {
    /// Real API host. Overridable per call via `fetch_from` so tests can point
    /// the full fetch pipeline at a mock server.
    const API_BASE: &'static str = "https://api.hetzner.cloud";

    /// Fetch hosts against an explicit API base. Production passes `API_BASE`;
    /// tests pass a mock server URL. The single seam that makes URL
    /// construction, the auth header, error mapping, pagination and
    /// `ProviderHost` mapping testable end to end.
    fn fetch_from(
        &self,
        base_url: &str,
        token: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        let agent = super::http_agent();

        super::paginate(cancel, |idx| {
            let page = idx + 1;
            let url = format!("{}/v1/servers?page={}&per_page=50", base_url, page);
            let resp: HetznerResponse = agent
                .get(&url)
                .header("Authorization", &super::bearer_auth(token))
                .call()
                .map_err(map_ureq_error)?
                .body_mut()
                .read_json()
                .map_err(|e| ProviderError::Parse(e.to_string()))?;

            let mut hosts = Vec::new();
            for server in &resp.servers {
                // Prefer public IPv4, fall back to public IPv6
                // IPv6 addresses may include CIDR suffix (e.g. "2a01:4f8::1/64")
                // which must be stripped for SSH compatibility.
                let ip_str = server
                    .public_net
                    .ipv4
                    .as_ref()
                    .filter(|v| !v.ip.is_empty())
                    .map(|v| v.ip.clone())
                    .or_else(|| {
                        server
                            .public_net
                            .ipv6
                            .as_ref()
                            .filter(|v| !v.ip.is_empty())
                            .map(|v| super::strip_cidr(&v.ip).to_string())
                    });
                if let Some(ip) = ip_str {
                    let mut tags: Vec<String> = server
                        .labels
                        .iter()
                        .map(|(k, v)| {
                            if v.is_empty() {
                                k.clone()
                            } else {
                                format!("{}={}", k, v)
                            }
                        })
                        .collect();
                    tags.sort();
                    let mut metadata = super::ProviderMetadata::new();
                    let region = server
                        .location
                        .as_ref()
                        .map(|l| &l.name)
                        .filter(|n| !n.is_empty())
                        .or_else(|| {
                            server
                                .datacenter
                                .as_ref()
                                .and_then(|d| d.location.as_ref())
                                .map(|l| &l.name)
                                .filter(|n| !n.is_empty())
                        });
                    metadata.push_opt("location", region.cloned());
                    if let Some(ref st) = server.server_type
                        && !st.name.is_empty()
                    {
                        metadata.push("type", st.name.clone());
                    }
                    if let Some(ref image) = server.image
                        && let Some(ref name) = image.name
                        && !name.is_empty()
                    {
                        metadata.push("image", name.clone());
                    }
                    if !server.status.is_empty() {
                        metadata.push("status", server.status.clone());
                    }
                    hosts.push(ProviderHost {
                        server_id: server.id.to_string(),
                        name: server.name.clone(),
                        ip,
                        tags,
                        metadata: metadata.finish(),
                        ..Default::default()
                    });
                }
            }

            let more = !resp.servers.is_empty()
                && resp.meta.pagination.page < resp.meta.pagination.last_page;
            Ok(super::PageResult { hosts, more })
        })
    }
}

impl Provider for Hetzner {
    fn name(&self) -> &str {
        "hetzner"
    }

    fn short_label(&self) -> &str {
        "hetzner"
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
    fn test_parse_hetzner_response() {
        let json = r#"{
            "servers": [
                {
                    "id": 42,
                    "name": "my-server",
                    "public_net": {
                        "ipv4": {"ip": "1.2.3.4"}
                    },
                    "labels": {"env": "prod", "team": ""}
                },
                {
                    "id": 43,
                    "name": "no-ip",
                    "public_net": {
                        "ipv4": null
                    },
                    "labels": {}
                }
            ],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers.len(), 2);
        assert_eq!(resp.servers[0].name, "my-server");
        assert_eq!(
            resp.servers[0].public_net.ipv4.as_ref().unwrap().ip,
            "1.2.3.4"
        );
        assert!(resp.servers[1].public_net.ipv4.is_none());
    }

    // Helper: same IP selection logic as fetch_hosts_cancellable
    fn select_hetzner_ip(server: &HetznerServer) -> Option<String> {
        server
            .public_net
            .ipv4
            .as_ref()
            .filter(|v| !v.ip.is_empty())
            .map(|v| v.ip.clone())
            .or_else(|| {
                server
                    .public_net
                    .ipv6
                    .as_ref()
                    .filter(|v| !v.ip.is_empty())
                    .map(|v| crate::providers::strip_cidr(&v.ip).to_string())
            })
    }

    fn make_hetzner_tags(labels: &std::collections::HashMap<String, String>) -> Vec<String> {
        let mut tags: Vec<String> = labels
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    k.clone()
                } else {
                    format!("{}={}", k, v)
                }
            })
            .collect();
        tags.sort();
        tags
    }

    #[test]
    fn test_hetzner_no_ip_skipped() {
        let json = r#"{
            "servers": [{"id": 1, "name": "no-ip", "public_net": {"ipv4": null}, "labels": {}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_hetzner_ip(&resp.servers[0]), None);
    }

    #[test]
    fn test_hetzner_empty_ipv4_skipped() {
        let json = r#"{
            "servers": [{"id": 1, "name": "empty", "public_net": {"ipv4": {"ip": ""}}, "labels": {}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_hetzner_ip(&resp.servers[0]), None);
    }

    #[test]
    fn test_hetzner_prefers_v4_over_v6() {
        let json = r#"{
            "servers": [{
                "id": 1, "name": "dual",
                "public_net": {"ipv4": {"ip": "1.2.3.4"}, "ipv6": {"ip": "2a01::1/64"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn test_hetzner_labels_to_tags_key_value() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        labels.insert("team".to_string(), "backend".to_string());
        let tags = make_hetzner_tags(&labels);
        assert!(tags.contains(&"env=prod".to_string()));
        assert!(tags.contains(&"team=backend".to_string()));
    }

    #[test]
    fn test_hetzner_labels_to_tags_empty_value() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("managed".to_string(), String::new());
        let tags = make_hetzner_tags(&labels);
        assert_eq!(tags, vec!["managed"]);
    }

    #[test]
    fn test_hetzner_labels_sorted() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("z-key".to_string(), "val".to_string());
        labels.insert("a-key".to_string(), "val".to_string());
        let tags = make_hetzner_tags(&labels);
        assert_eq!(tags[0], "a-key=val");
        assert_eq!(tags[1], "z-key=val");
    }

    #[test]
    fn test_hetzner_pagination_continues() {
        let json = r#"{
            "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.1.1.1"}}, "labels": {}}],
            "meta": {"pagination": {"page": 1, "last_page": 3}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert!(resp.meta.pagination.page < resp.meta.pagination.last_page);
    }

    #[test]
    fn test_hetzner_pagination_stops() {
        let json = r#"{
            "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.1.1.1"}}, "labels": {}}],
            "meta": {"pagination": {"page": 3, "last_page": 3}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert!(resp.meta.pagination.page >= resp.meta.pagination.last_page);
    }

    #[test]
    fn test_hetzner_empty_server_list() {
        let json = r#"{
            "servers": [],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert!(resp.servers.is_empty());
    }

    #[test]
    fn test_hetzner_default_labels_empty() {
        let json = r#"{
            "servers": [{"id": 1, "name": "no-labels", "public_net": {"ipv4": {"ip": "1.1.1.1"}}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert!(resp.servers[0].labels.is_empty());
    }

    #[test]
    fn test_hetzner_v6_only_fallback() {
        let json = r#"{
            "servers": [{
                "id": 1, "name": "v6-only",
                "public_net": {"ipv4": null, "ipv6": {"ip": "2a01::1/64"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("2a01::1".to_string())
        );
    }

    #[test]
    fn test_hetzner_v6_without_cidr() {
        let json = r#"{
            "servers": [{
                "id": 1, "name": "v6-bare",
                "public_net": {"ipv4": null, "ipv6": {"ip": "2a01::1"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("2a01::1".to_string())
        );
    }

    #[test]
    fn test_hetzner_empty_v6_skipped() {
        let json = r#"{
            "servers": [{
                "id": 1, "name": "empty-v6",
                "public_net": {"ipv4": null, "ipv6": {"ip": ""}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_hetzner_ip(&resp.servers[0]), None);
    }

    #[test]
    fn test_hetzner_multiple_labels() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        labels.insert("app".to_string(), "web".to_string());
        labels.insert("managed".to_string(), String::new());
        let tags = make_hetzner_tags(&labels);
        assert_eq!(tags.len(), 3);
        // Sorted: app=web, env=prod, managed
        assert_eq!(tags[0], "app=web");
        assert_eq!(tags[1], "env=prod");
        assert_eq!(tags[2], "managed");
    }

    #[test]
    fn test_hetzner_null_v6_field() {
        // When ipv6 is completely absent from JSON
        let json = r#"{
            "servers": [{
                "id": 1, "name": "no-v6",
                "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert!(resp.servers[0].public_net.ipv6.is_none());
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn test_hetzner_labels_with_special_chars() {
        // Labels can contain equals signs in values (handled by split format)
        let mut labels = std::collections::HashMap::new();
        labels.insert("config".to_string(), "key=val".to_string());
        let tags = make_hetzner_tags(&labels);
        assert_eq!(tags, vec!["config=key=val"]);
    }

    #[test]
    fn test_hetzner_large_id() {
        // Hetzner IDs are u64 and can be large
        let json = r#"{
            "servers": [{
                "id": 99999999999,
                "name": "big-id",
                "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].id, 99999999999);
    }

    #[test]
    fn test_hetzner_empty_ipv4_with_v6_uses_v6() {
        // ipv4 exists but is empty, v6 available
        let json = r#"{
            "servers": [{
                "id": 1, "name": "empty-v4-has-v6",
                "public_net": {"ipv4": {"ip": ""}, "ipv6": {"ip": "2a01::1/64"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("2a01::1".to_string())
        );
    }

    #[test]
    fn test_hetzner_both_null_skipped() {
        let json = r#"{
            "servers": [{
                "id": 1, "name": "no-ips",
                "public_net": {"ipv4": null, "ipv6": null},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_hetzner_ip(&resp.servers[0]), None);
    }

    #[test]
    fn test_hetzner_both_empty_skipped() {
        let json = r#"{
            "servers": [{
                "id": 1, "name": "empty-both",
                "public_net": {"ipv4": {"ip": ""}, "ipv6": {"ip": ""}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_hetzner_ip(&resp.servers[0]), None);
    }

    #[test]
    fn test_hetzner_many_labels_sorted() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("zzz".to_string(), "last".to_string());
        labels.insert("aaa".to_string(), "first".to_string());
        labels.insert("mmm".to_string(), String::new());
        let tags = make_hetzner_tags(&labels);
        assert_eq!(tags, vec!["aaa=first", "mmm", "zzz=last"]);
    }

    #[test]
    fn test_hetzner_v6_cidr_128() {
        let json = r#"{
            "servers": [{
                "id": 1, "name": "v6-128",
                "public_net": {"ipv4": null, "ipv6": {"ip": "2a01:4f8::1/128"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("2a01:4f8::1".to_string())
        );
    }

    #[test]
    fn test_hetzner_multiple_servers_parsed() {
        let json = r#"{
            "servers": [
                {"id": 1, "name": "web-1", "public_net": {"ipv4": {"ip": "1.1.1.1"}}, "labels": {"env": "prod"}},
                {"id": 2, "name": "web-2", "public_net": {"ipv4": {"ip": "2.2.2.2"}}, "labels": {"env": "staging"}},
                {"id": 3, "name": "db", "public_net": {"ipv4": null}, "labels": {}}
            ],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers.len(), 3);
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("1.1.1.1".to_string())
        );
        assert_eq!(
            select_hetzner_ip(&resp.servers[1]),
            Some("2.2.2.2".to_string())
        );
        assert_eq!(select_hetzner_ip(&resp.servers[2]), None);
    }

    #[test]
    fn test_ipv6_only_server_uses_v6() {
        let json = r#"{
            "servers": [
                {
                    "id": 44,
                    "name": "v6-only",
                    "public_net": {
                        "ipv4": null,
                        "ipv6": {"ip": "2a01:4f8::1/64"}
                    },
                    "labels": {}
                }
            ],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        let server = &resp.servers[0];
        let ip = server
            .public_net
            .ipv4
            .as_ref()
            .filter(|v| !v.ip.is_empty())
            .map(|v| v.ip.clone())
            .or_else(|| {
                server
                    .public_net
                    .ipv6
                    .as_ref()
                    .filter(|v| !v.ip.is_empty())
                    .map(|v| crate::providers::strip_cidr(&v.ip).to_string())
            });
        // CIDR suffix must be stripped for SSH compatibility
        assert_eq!(ip, Some("2a01:4f8::1".to_string()));
    }

    // --- label with empty value uses key only ---

    #[test]
    fn test_hetzner_label_empty_value_uses_key() {
        let json = r#"{
            "servers": [{
                "id": 50,
                "name": "label-test",
                "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {"env": "", "role": "web"}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        let server = &resp.servers[0];
        let mut tags: Vec<String> = server
            .labels
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    k.clone()
                } else {
                    format!("{}={}", k, v)
                }
            })
            .collect();
        tags.sort();
        assert_eq!(tags, vec!["env", "role=web"]);
    }

    // --- server with empty name ---

    #[test]
    fn test_hetzner_empty_name() {
        let json = r#"{
            "servers": [{
                "id": 51,
                "name": "",
                "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].name, "");
    }

    // --- both ipv4 and ipv6 present: prefers ipv4 ---

    #[test]
    fn test_hetzner_dual_stack_prefers_v4() {
        let json = r#"{
            "servers": [{
                "id": 52,
                "name": "dual",
                "public_net": {
                    "ipv4": {"ip": "1.2.3.4"},
                    "ipv6": {"ip": "2a01::1/64"}
                },
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        let server = &resp.servers[0];
        let ip = server
            .public_net
            .ipv4
            .as_ref()
            .filter(|v| !v.ip.is_empty())
            .map(|v| v.ip.clone());
        assert_eq!(ip, Some("1.2.3.4".to_string()));
    }

    // --- Resilience: extra/unknown fields are ignored by serde ---

    #[test]
    fn test_hetzner_extra_fields_ignored() {
        // Real Hetzner API returns many more fields per server
        let json = r#"{
            "servers": [{
                "id": 60,
                "name": "full-response",
                "status": "running",
                "public_net": {
                    "ipv4": {"ip": "1.2.3.4", "blocked": false, "dns_ptr": "1.2.3.4.host.example.com"},
                    "ipv6": {"ip": "2a01:4f8::1/64", "blocked": false, "dns_ptr": []},
                    "floating_ips": [],
                    "firewalls": []
                },
                "server_type": {"id": 1, "name": "cx11", "description": "CX11"},
                "datacenter": {"id": 1, "name": "fsn1-dc14"},
                "image": {"id": 12345, "name": "ubuntu-22.04"},
                "iso": null,
                "rescue_enabled": false,
                "locked": false,
                "backup_window": "22-02",
                "outgoing_traffic": 123456,
                "ingoing_traffic": 654321,
                "included_traffic": 654321000000,
                "protection": {"delete": false, "rebuild": false},
                "labels": {"env": "prod"},
                "volumes": [],
                "load_balancers": [],
                "primary_disk_size": 20,
                "created": "2024-01-01T00:00:00+00:00"
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1, "per_page": 25, "total_entries": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].name, "full-response");
        assert_eq!(
            select_hetzner_ip(&resp.servers[0]),
            Some("1.2.3.4".to_string())
        );
        assert_eq!(resp.servers[0].labels.get("env").unwrap(), "prod");
    }

    #[test]
    fn test_hetzner_ipinfo_extra_fields_ignored() {
        // IpInfo may contain blocked, dns_ptr
        let json = r#"{
            "servers": [{
                "id": 61,
                "name": "ip-extra",
                "public_net": {
                    "ipv4": {"ip": "5.6.7.8", "blocked": false, "dns_ptr": "host.example.com"}
                },
                "labels": {}
            }],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            resp.servers[0].public_net.ipv4.as_ref().unwrap().ip,
            "5.6.7.8"
        );
    }

    #[test]
    fn test_hetzner_pagination_extra_fields_ignored() {
        // Pagination may contain per_page and total_entries
        let json = r#"{
            "servers": [],
            "meta": {"pagination": {"page": 1, "last_page": 1, "per_page": 25, "total_entries": 0, "next_page": null, "previous_page": null}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.meta.pagination.page, 1);
        assert_eq!(resp.meta.pagination.last_page, 1);
    }

    // =========================================================================
    // location preferred over datacenter
    // =========================================================================

    fn select_region(server: &HetznerServer) -> Option<String> {
        server
            .location
            .as_ref()
            .map(|l| &l.name)
            .filter(|n| !n.is_empty())
            .or_else(|| {
                server
                    .datacenter
                    .as_ref()
                    .and_then(|d| d.location.as_ref())
                    .map(|l| &l.name)
                    .filter(|n| !n.is_empty())
            })
            .cloned()
    }

    #[test]
    fn test_location_preferred_over_datacenter() {
        let json = r#"{
            "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}, "datacenter": {"name": "fsn1-dc14"}, "location": {"name": "fsn1"}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_region(&resp.servers[0]), Some("fsn1".to_string()));
    }

    #[test]
    fn test_datacenter_fallback_when_no_location() {
        let json = r#"{
            "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}, "datacenter": {"name": "fsn1-dc14", "location": {"name": "fsn1"}}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_region(&resp.servers[0]), Some("fsn1".to_string()));
    }

    #[test]
    fn test_empty_location_falls_back_to_datacenter_location() {
        let json = r#"{
            "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}, "datacenter": {"name": "fsn1-dc14", "location": {"name": "fsn1"}}, "location": {"name": ""}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_region(&resp.servers[0]), Some("fsn1".to_string()));
    }

    #[test]
    fn test_datacenter_without_nested_location_returns_none() {
        let json = r#"{
            "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}, "datacenter": {"name": "fsn1-dc14"}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_region(&resp.servers[0]), None);
    }

    #[test]
    fn test_no_location_no_datacenter() {
        let json = r#"{
            "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.2.3.4"}},
                "labels": {}}],
            "meta": {"pagination": {"page": 1, "last_page": 1}}
        }"#;
        let resp: HetznerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_region(&resp.servers[0]), None);
    }

    // ── HTTP roundtrip tests (mockito) ──────────────────────────────

    #[test]
    fn test_http_servers_roundtrip() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "50".into()),
            ]))
            .match_header("Authorization", "Bearer test-hetzner-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [
                        {
                            "id": 42,
                            "name": "web-prod-1",
                            "public_net": {
                                "ipv4": {"ip": "116.203.0.10"},
                                "ipv6": {"ip": "2a01:4f8:c010::1/64"}
                            },
                            "labels": {"env": "prod", "team": "backend"},
                            "server_type": {"name": "cx21"},
                            "datacenter": {"name": "fsn1-dc14", "location": {"name": "fsn1"}},
                            "location": {"name": "fsn1"},
                            "status": "running",
                            "image": {"name": "ubuntu-22.04"}
                        }
                    ],
                    "meta": {"pagination": {"page": 1, "last_page": 1}}
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        let url = format!("{}/v1/servers?page=1&per_page=50", server.url());
        let resp: HetznerResponse = agent
            .get(&url)
            .header("Authorization", "Bearer test-hetzner-token")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(resp.servers.len(), 1);
        let s = &resp.servers[0];
        assert_eq!(s.id, 42);
        assert_eq!(s.name, "web-prod-1");
        assert_eq!(s.public_net.ipv4.as_ref().unwrap().ip, "116.203.0.10");
        assert_eq!(
            s.public_net.ipv6.as_ref().unwrap().ip,
            "2a01:4f8:c010::1/64"
        );
        assert_eq!(s.labels.get("env").unwrap(), "prod");
        assert_eq!(s.labels.get("team").unwrap(), "backend");
        assert_eq!(s.server_type.as_ref().unwrap().name, "cx21");
        assert_eq!(s.location.as_ref().unwrap().name, "fsn1");
        assert_eq!(s.status, "running");
        assert_eq!(
            s.image.as_ref().unwrap().name.as_deref(),
            Some("ubuntu-22.04")
        );
        assert_eq!(resp.meta.pagination.page, 1);
        assert_eq!(resp.meta.pagination.last_page, 1);
        mock.assert();
    }

    #[test]
    fn test_http_servers_pagination() {
        let mut server = mockito::Server::new();
        let page1 = server
            .mock("GET", "/v1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "50".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [{"id": 1, "name": "a", "public_net": {"ipv4": {"ip": "1.1.1.1"}}, "labels": {}}],
                    "meta": {"pagination": {"page": 1, "last_page": 2}}
                }"#,
            )
            .create();
        let page2 = server
            .mock("GET", "/v1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "2".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "50".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [{"id": 2, "name": "b", "public_net": {"ipv4": {"ip": "2.2.2.2"}}, "labels": {}}],
                    "meta": {"pagination": {"page": 2, "last_page": 2}}
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        // Page 1
        let r1: HetznerResponse = agent
            .get(&format!("{}/v1/servers?page=1&per_page=50", server.url()))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r1.servers.len(), 1);
        assert_eq!(r1.meta.pagination.page, 1);
        assert_eq!(r1.meta.pagination.last_page, 2);
        // Page 2
        let r2: HetznerResponse = agent
            .get(&format!("{}/v1/servers?page=2&per_page=50", server.url()))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r2.servers.len(), 1);
        assert_eq!(r2.meta.pagination.page, 2);
        assert_eq!(r2.meta.pagination.last_page, 2);
        page1.assert();
        page2.assert();
    }

    #[test]
    fn test_http_servers_auth_failure() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/servers")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"error": {"message": "unauthorized", "code": "unauthorized"}}"#)
            .create();

        let agent = super::super::http_agent();
        let result = agent
            .get(&format!("{}/v1/servers?page=1&per_page=50", server.url()))
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
        // Drives the production seam end to end ACROSS A PAGE BOUNDARY: URL
        // construction, Bearer header, the page<last_page pagination loop,
        // label->tag formatting, the IPv6 CIDR-strip fallback and
        // ProviderHost mapping. Two pages confirm the loop actually advances.
        let mut server = mockito::Server::new();
        let page1 = server
            .mock("GET", "/v1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "50".into()),
            ]))
            .match_header("Authorization", "Bearer tk-9")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [{
                        "id": 101,
                        "name": "web-1",
                        "status": "running",
                        "public_net": {"ipv4": {"ip": "203.0.113.7"}},
                        "labels": {"env": "prod", "role": ""},
                        "location": {"name": "fsn1"},
                        "server_type": {"name": "cx22"}
                    }],
                    "meta": {"pagination": {"page": 1, "last_page": 2}}
                }"#,
            )
            .create();
        let page2 = server
            .mock("GET", "/v1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "2".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "50".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [{
                        "id": 102,
                        "name": "v6-only",
                        "status": "running",
                        "public_net": {"ipv6": {"ip": "2a01:4f8::1/64"}},
                        "labels": {}
                    }],
                    "meta": {"pagination": {"page": 2, "last_page": 2}}
                }"#,
            )
            .create();

        let hosts = Hetzner
            .fetch_from(&server.url(), "tk-9", &AtomicBool::new(false))
            .expect("fetch_from must succeed against the mock");
        page1.assert();
        page2.assert();

        assert_eq!(hosts.len(), 2, "both pages must be collected");
        assert_eq!(hosts[0].server_id, "101");
        assert_eq!(hosts[0].name, "web-1");
        assert_eq!(hosts[0].ip, "203.0.113.7");
        assert_eq!(
            hosts[0].tags,
            vec!["env=prod".to_string(), "role".to_string()]
        );
        assert!(
            hosts[0]
                .metadata
                .contains(&("location".to_string(), "fsn1".to_string()))
        );
        // IPv6-only server: CIDR suffix must be stripped.
        assert_eq!(hosts[1].server_id, "102");
        assert_eq!(hosts[1].ip, "2a01:4f8::1");
    }

    #[test]
    fn fetch_from_maps_auth_failure_to_provider_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v1/servers")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"error": {"message": "unauthorized", "code": "unauthorized"}}"#)
            .create();

        let result = Hetzner.fetch_from(&server.url(), "bad", &AtomicBool::new(false));
        mock.assert();
        assert!(
            matches!(result, Err(ProviderError::AuthFailed)),
            "401 must map to AuthFailed, got {result:?}"
        );
    }
}
