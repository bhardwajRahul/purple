use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use super::{Provider, ProviderError, ProviderHost, map_ureq_error};

pub struct DigitalOcean;

impl DigitalOcean {
    /// Real API host. Overridable per call via `fetch_from` so tests can point
    /// the full fetch pipeline at a mock server.
    const API_BASE: &'static str = "https://api.digitalocean.com";

    /// Fetch hosts against an explicit API base. Production passes
    /// `API_BASE`; tests pass a mock server URL. This is the single seam that
    /// makes URL construction, the auth header, error mapping, pagination and
    /// `ProviderHost` mapping testable end to end.
    fn fetch_from(
        &self,
        base_url: &str,
        token: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        let agent = super::http_agent();
        let per_page = 200u64;

        super::paginate(cancel, |idx| {
            let page = idx + 1;
            let url = format!(
                "{}/v2/droplets?page={}&per_page={}",
                base_url, page, per_page
            );
            let resp: DropletResponse = agent
                .get(&url)
                .header("Authorization", &super::bearer_auth(token))
                .call()
                .map_err(map_ureq_error)?
                .body_mut()
                .read_json()
                .map_err(|e| ProviderError::Parse(e.to_string()))?;

            let mut hosts = Vec::new();
            for droplet in &resp.droplets {
                // Prefer public IPv4, fall back to public IPv6
                let ip = droplet
                    .networks
                    .v4
                    .iter()
                    .find(|n| n.net_type == "public")
                    .or_else(|| droplet.networks.v6.iter().find(|n| n.net_type == "public"))
                    .map(|n| n.ip_address.clone());
                if let Some(ip) = ip {
                    let mut metadata = super::ProviderMetadata::new();
                    if let Some(ref region) = droplet.region {
                        if !region.slug.is_empty() {
                            metadata.push("region", &region.slug);
                        }
                    }
                    if !droplet.size_slug.is_empty() {
                        metadata.push("size", &droplet.size_slug);
                    }
                    if let Some(ref image) = droplet.image {
                        let label = match (&image.distribution, &image.name) {
                            (Some(dist), Some(name)) if !dist.is_empty() && !name.is_empty() => {
                                format!("{} {}", dist, name)
                            }
                            (Some(dist), _) if !dist.is_empty() => dist.clone(),
                            (_, Some(name)) if !name.is_empty() => name.clone(),
                            _ => String::new(),
                        };
                        if !label.is_empty() {
                            metadata.push("image", label);
                        }
                    }
                    if !droplet.status.is_empty() {
                        metadata.push("status", &droplet.status);
                    }
                    hosts.push(ProviderHost {
                        server_id: droplet.id.to_string(),
                        name: droplet.name.clone(),
                        ip,
                        tags: droplet.tags.clone(),
                        metadata: metadata.finish(),
                        ..Default::default()
                    });
                }
            }

            let fetched = page * per_page;
            let more = !resp.droplets.is_empty() && fetched < resp.meta.total;
            Ok(super::PageResult { hosts, more })
        })
    }
}

#[derive(Deserialize)]
struct DropletResponse {
    droplets: Vec<Droplet>,
    meta: Meta,
}

#[derive(Deserialize)]
struct Droplet {
    id: u64,
    name: String,
    networks: Networks,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    size_slug: String,
    #[serde(default)]
    region: Option<Region>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    image: Option<DropletImage>,
}

#[derive(Deserialize)]
struct DropletImage {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    distribution: Option<String>,
}

#[derive(Deserialize)]
struct Region {
    slug: String,
}

#[derive(Deserialize)]
struct Networks {
    v4: Vec<NetworkIp>,
    #[serde(default)]
    v6: Vec<NetworkIp>,
}

#[derive(Deserialize)]
struct NetworkIp {
    ip_address: String,
    #[serde(rename = "type")]
    net_type: String,
}

#[derive(Deserialize)]
struct Meta {
    total: u64,
}

impl Provider for DigitalOcean {
    fn name(&self) -> &str {
        "digitalocean"
    }

    fn short_label(&self) -> &str {
        "do"
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
    fn test_parse_droplet_response() {
        let json = r#"{
            "droplets": [
                {
                    "id": 12345,
                    "name": "web-1",
                    "networks": {
                        "v4": [
                            {"ip_address": "10.0.0.1", "type": "private"},
                            {"ip_address": "1.2.3.4", "type": "public"}
                        ]
                    },
                    "tags": ["production"]
                },
                {
                    "id": 67890,
                    "name": "db-1",
                    "networks": {
                        "v4": [
                            {"ip_address": "10.0.0.2", "type": "private"}
                        ]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 2}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.droplets.len(), 2);
        assert_eq!(resp.droplets[0].name, "web-1");
        // web-1 has public IP
        let public_ip = resp.droplets[0]
            .networks
            .v4
            .iter()
            .find(|n| n.net_type == "public");
        assert!(public_ip.is_some());
        assert_eq!(public_ip.unwrap().ip_address, "1.2.3.4");
        // db-1 has no public IP (private only)
        let public_ip = resp.droplets[1]
            .networks
            .v4
            .iter()
            .find(|n| n.net_type == "public");
        assert!(public_ip.is_none());
    }

    // Helper: apply the same IP selection logic as fetch_hosts_cancellable
    fn select_droplet_ip(droplet: &Droplet) -> Option<String> {
        droplet
            .networks
            .v4
            .iter()
            .find(|n| n.net_type == "public")
            .or_else(|| droplet.networks.v6.iter().find(|n| n.net_type == "public"))
            .map(|n| n.ip_address.clone())
    }

    #[test]
    fn test_droplet_private_only_skipped() {
        let json = r#"{
            "droplets": [
                {
                    "id": 99,
                    "name": "private-only",
                    "networks": {
                        "v4": [{"ip_address": "10.132.0.2", "type": "private"}]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_droplet_ip(&resp.droplets[0]), None);
    }

    #[test]
    fn test_droplet_empty_networks_skipped() {
        let json = r#"{
            "droplets": [
                {
                    "id": 100,
                    "name": "no-networks",
                    "networks": {"v4": []},
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_droplet_ip(&resp.droplets[0]), None);
    }

    #[test]
    fn test_droplet_prefers_v4_over_v6() {
        let json = r#"{
            "droplets": [
                {
                    "id": 101,
                    "name": "dual-stack",
                    "networks": {
                        "v4": [{"ip_address": "1.2.3.4", "type": "public"}],
                        "v6": [{"ip_address": "2604:a880::1", "type": "public"}]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_droplet_ip(&resp.droplets[0]),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn test_droplet_tags_preserved() {
        let json = r#"{
            "droplets": [
                {
                    "id": 102,
                    "name": "tagged",
                    "networks": {"v4": [{"ip_address": "1.2.3.4", "type": "public"}]},
                    "tags": ["web", "production", "us-east"]
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.droplets[0].tags, vec!["web", "production", "us-east"]);
    }

    #[test]
    fn test_droplet_id_is_u64() {
        let json = r#"{
            "droplets": [{"id": 999999999, "name": "big-id", "networks": {"v4": []}, "tags": []}],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.droplets[0].id, 999999999);
    }

    #[test]
    fn test_droplet_v6_default_empty() {
        let json = r#"{
            "droplets": [
                {
                    "id": 103,
                    "name": "no-v6",
                    "networks": {"v4": [{"ip_address": "5.6.7.8", "type": "public"}]},
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert!(resp.droplets[0].networks.v6.is_empty());
    }

    #[test]
    fn test_pagination_continues_when_total_exceeds_fetched() {
        let json = r#"{
            "droplets": [{"id": 1, "name": "a", "networks": {"v4": []}, "tags": []}],
            "meta": {"total": 500}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        let page = 1u64;
        let per_page = 200u64;
        let fetched = page * per_page;
        // Should continue: fetched (200) < total (500)
        assert!(fetched < resp.meta.total);
    }

    #[test]
    fn test_pagination_stops_when_fetched_reaches_total() {
        let json = r#"{
            "droplets": [{"id": 1, "name": "a", "networks": {"v4": []}, "tags": []}],
            "meta": {"total": 200}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        let page = 1u64;
        let per_page = 200u64;
        let fetched = page * per_page;
        // Should stop: fetched (200) >= total (200)
        assert!(fetched >= resp.meta.total);
    }

    #[test]
    fn test_empty_droplet_list_stops_pagination() {
        let json = r#"{
            "droplets": [],
            "meta": {"total": 0}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert!(resp.droplets.is_empty());
    }

    #[test]
    fn test_droplet_multiple_public_v4_uses_first() {
        let json = r#"{
            "droplets": [
                {
                    "id": 104,
                    "name": "multi-public",
                    "networks": {
                        "v4": [
                            {"ip_address": "1.2.3.4", "type": "public"},
                            {"ip_address": "5.6.7.8", "type": "public"}
                        ]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_droplet_ip(&resp.droplets[0]),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn test_droplet_private_v4_public_v6_uses_v6() {
        // No public IPv4, but there is a public IPv6
        let json = r#"{
            "droplets": [
                {
                    "id": 105,
                    "name": "private-v4-public-v6",
                    "networks": {
                        "v4": [{"ip_address": "10.132.0.5", "type": "private"}],
                        "v6": [{"ip_address": "2604:a880::1", "type": "public"}]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_droplet_ip(&resp.droplets[0]),
            Some("2604:a880::1".to_string())
        );
    }

    #[test]
    fn test_droplet_default_tags_empty() {
        // When tags key is missing entirely, should default to empty
        let json = r#"{
            "droplets": [
                {"id": 106, "name": "no-tags-key", "networks": {"v4": [{"ip_address": "1.2.3.4", "type": "public"}]}}
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert!(resp.droplets[0].tags.is_empty());
    }

    #[test]
    fn test_droplet_private_v6_not_used() {
        // Private v6 should not be picked as public
        let json = r#"{
            "droplets": [
                {
                    "id": 107,
                    "name": "private-v6",
                    "networks": {
                        "v4": [],
                        "v6": [{"ip_address": "fd00::1", "type": "private"}]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_droplet_ip(&resp.droplets[0]), None);
    }

    #[test]
    fn test_droplet_large_id() {
        let json = r#"{
            "droplets": [{"id": 999999999999, "name": "big", "networks": {"v4": [{"ip_address": "1.2.3.4", "type": "public"}]}, "tags": []}],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.droplets[0].id, 999999999999);
    }

    #[test]
    fn test_droplet_multiple_private_v4_no_public() {
        // Multiple private IPs but no public - should return None
        let json = r#"{
            "droplets": [
                {
                    "id": 108,
                    "name": "multi-private",
                    "networks": {
                        "v4": [
                            {"ip_address": "10.132.0.1", "type": "private"},
                            {"ip_address": "10.132.0.2", "type": "private"}
                        ]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_droplet_ip(&resp.droplets[0]), None);
    }

    // --- Resilience: extra/unknown fields are ignored by serde ---

    #[test]
    fn test_droplet_extra_fields_ignored() {
        // Real DO API returns many more fields. Verify unknown fields don't break parsing.
        let json = r#"{
            "droplets": [
                {
                    "id": 200,
                    "name": "full-response",
                    "status": "active",
                    "size_slug": "s-1vcpu-1gb",
                    "region": {"slug": "nyc3", "name": "New York 3"},
                    "image": {"id": 12345, "name": "Ubuntu 22.04"},
                    "created_at": "2024-01-01T00:00:00Z",
                    "disk": 25,
                    "memory": 1024,
                    "vcpus": 1,
                    "networks": {
                        "v4": [{"ip_address": "1.2.3.4", "type": "public", "netmask": "255.255.240.0", "gateway": "1.2.0.1"}],
                        "v6": [{"ip_address": "2604::1", "type": "public", "netmask": 64, "gateway": "2604::"}]
                    },
                    "tags": ["web"],
                    "volume_ids": ["abc"],
                    "features": ["backups"]
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.droplets[0].name, "full-response");
        assert_eq!(
            select_droplet_ip(&resp.droplets[0]),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn test_network_ip_extra_fields_ignored() {
        // NetworkIp may have extra fields like netmask, gateway
        let json = r#"{
            "droplets": [{
                "id": 201,
                "name": "extra-net",
                "networks": {
                    "v4": [{"ip_address": "5.6.7.8", "type": "public", "netmask": "255.255.240.0", "gateway": "5.6.0.1"}]
                },
                "tags": []
            }],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.droplets[0].networks.v4[0].ip_address, "5.6.7.8");
    }

    #[test]
    fn test_meta_extra_fields_ignored() {
        let json = r#"{
            "droplets": [],
            "meta": {"total": 0},
            "links": {"pages": {}}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.meta.total, 0);
    }

    #[test]
    fn test_ipv6_only_droplet_uses_v6() {
        let json = r#"{
            "droplets": [
                {
                    "id": 11111,
                    "name": "v6-only",
                    "networks": {
                        "v4": [],
                        "v6": [
                            {"ip_address": "2604:a880::1", "type": "public"}
                        ]
                    },
                    "tags": []
                }
            ],
            "meta": {"total": 1}
        }"#;
        let resp: DropletResponse = serde_json::from_str(json).unwrap();
        let droplet = &resp.droplets[0];
        let ip = droplet
            .networks
            .v4
            .iter()
            .find(|n| n.net_type == "public")
            .or_else(|| droplet.networks.v6.iter().find(|n| n.net_type == "public"))
            .map(|n| n.ip_address.clone());
        assert_eq!(ip, Some("2604:a880::1".to_string()));
    }

    // ── HTTP roundtrip tests (mockito) ──────────────────────────────

    #[test]
    fn test_http_list_droplets_roundtrip() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/droplets")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "200".into()),
            ]))
            .match_header("Authorization", "Bearer test-token-123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "droplets": [
                        {
                            "id": 99001,
                            "name": "web-prod-1",
                            "networks": {
                                "v4": [
                                    {"ip_address": "10.0.0.5", "type": "private"},
                                    {"ip_address": "203.0.113.10", "type": "public"}
                                ]
                            },
                            "tags": ["prod", "web"],
                            "size_slug": "s-2vcpu-4gb",
                            "region": {"slug": "nyc3"},
                            "status": "active",
                            "image": {"name": "22.04 (LTS) x64", "distribution": "Ubuntu"}
                        }
                    ],
                    "meta": {"total": 1}
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        let url = format!("{}/v2/droplets?page=1&per_page=200", server.url());
        let resp: DropletResponse = agent
            .get(&url)
            .header("Authorization", "Bearer test-token-123")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(resp.droplets.len(), 1);
        let d = &resp.droplets[0];
        assert_eq!(d.id, 99001);
        assert_eq!(d.name, "web-prod-1");
        assert_eq!(d.size_slug, "s-2vcpu-4gb");
        assert_eq!(d.region.as_ref().unwrap().slug, "nyc3");
        assert_eq!(d.status, "active");
        assert_eq!(d.tags, vec!["prod", "web"]);
        assert_eq!(resp.meta.total, 1);
        mock.assert();
    }

    #[test]
    fn test_http_list_droplets_pagination() {
        let mut server = mockito::Server::new();
        let page1 = server
            .mock("GET", "/v2/droplets")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "200".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "droplets": [{"id": 1, "name": "a", "networks": {"v4": [{"ip_address": "1.1.1.1", "type": "public"}]}}],
                    "meta": {"total": 2}
                }"#,
            )
            .create();
        let page2 = server
            .mock("GET", "/v2/droplets")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "2".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "200".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "droplets": [{"id": 2, "name": "b", "networks": {"v4": [{"ip_address": "2.2.2.2", "type": "public"}]}}],
                    "meta": {"total": 2}
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        // Page 1
        let r1: DropletResponse = agent
            .get(&format!("{}/v2/droplets?page=1&per_page=200", server.url()))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r1.droplets.len(), 1);
        assert_eq!(r1.meta.total, 2);
        // Page 2
        let r2: DropletResponse = agent
            .get(&format!("{}/v2/droplets?page=2&per_page=200", server.url()))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r2.droplets.len(), 1);
        page1.assert();
        page2.assert();
    }

    #[test]
    fn fetch_from_drives_full_pipeline_against_mock() {
        // Exercises the production fetch path end to end: URL construction,
        // auth header, JSON deserialize, ProviderHost mapping and pagination
        // termination. Before the seam, these were only tested by re-issuing
        // an equivalent request inline, never through fetch_hosts_cancellable.
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/droplets")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "200".into()),
            ]))
            .match_header("Authorization", "Bearer tk-42")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "droplets": [
                        {
                            "id": 99001,
                            "name": "web-prod-1",
                            "networks": {
                                "v4": [
                                    {"ip_address": "10.0.0.5", "type": "private"},
                                    {"ip_address": "203.0.113.10", "type": "public"}
                                ]
                            },
                            "tags": ["prod", "web"],
                            "size_slug": "s-2vcpu-4gb",
                            "region": {"slug": "nyc3"},
                            "status": "active",
                            "image": {"name": "22.04 (LTS) x64", "distribution": "Ubuntu"}
                        }
                    ],
                    "meta": {"total": 1}
                }"#,
            )
            .create();

        let hosts = DigitalOcean
            .fetch_from(&server.url(), "tk-42", &AtomicBool::new(false))
            .expect("fetch_from must succeed against the mock");
        mock.assert();

        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].server_id, "99001");
        assert_eq!(hosts[0].name, "web-prod-1");
        assert_eq!(hosts[0].ip, "203.0.113.10");
        assert_eq!(hosts[0].tags, vec!["prod", "web"]);
        assert!(
            hosts[0]
                .metadata
                .contains(&("region".to_string(), "nyc3".to_string()))
        );
    }

    #[test]
    fn fetch_from_maps_auth_failure_to_provider_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/droplets")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"id": "Unauthorized", "message": "bad token"}"#)
            .create();

        let result = DigitalOcean.fetch_from(&server.url(), "bad", &AtomicBool::new(false));
        mock.assert();
        assert!(
            matches!(result, Err(ProviderError::AuthFailed)),
            "401 must map to ProviderError::AuthFailed, got {result:?}"
        );
    }

    #[test]
    fn test_http_list_droplets_auth_failure() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/droplets")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"id": "Unauthorized", "message": "Unable to authenticate you"}"#)
            .create();

        let agent = super::super::http_agent();
        let result = agent
            .get(&format!("{}/v2/droplets?page=1&per_page=200", server.url()))
            .header("Authorization", "Bearer bad-token")
            .call();

        match result {
            Err(ureq::Error::StatusCode(401)) => {} // expected
            other => panic!("expected 401 error, got {:?}", other),
        }
        mock.assert();
    }
}
