use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use super::{Provider, ProviderError, ProviderHost, map_ureq_error};

pub struct Vultr;

#[derive(Deserialize)]
struct InstanceResponse {
    instances: Vec<Instance>,
    meta: VultrMeta,
}

#[derive(Deserialize)]
struct Instance {
    id: String,
    label: String,
    main_ip: String,
    #[serde(default)]
    v6_main_ip: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    region: String,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    power_status: String,
}

#[derive(Deserialize)]
struct VultrMeta {
    links: VultrLinks,
}

#[derive(Deserialize)]
struct VultrLinks {
    #[serde(default)]
    next: String,
}

impl Vultr {
    const API_BASE: &'static str = "https://api.vultr.com";

    fn fetch_from(
        &self,
        base_url: &str,
        token: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        let agent = super::http_agent();
        let mut cursor: Option<String> = None;

        super::paginate(cancel, |_idx| {
            let url = match &cursor {
                None => format!("{}/v2/instances?per_page=500", base_url),
                Some(c) => format!(
                    "{}/v2/instances?per_page=500&cursor={}",
                    base_url,
                    super::percent_encode(c)
                ),
            };
            let resp: InstanceResponse = agent
                .get(&url)
                .header("Authorization", &super::bearer_auth(token))
                .call()
                .map_err(map_ureq_error)?
                .body_mut()
                .read_json()
                .map_err(|e| ProviderError::Parse(e.to_string()))?;

            let mut hosts = Vec::new();
            for instance in &resp.instances {
                // Prefer public IPv4, fall back to public IPv6
                let ip = if !instance.main_ip.is_empty() && instance.main_ip != "0.0.0.0" {
                    instance.main_ip.clone()
                } else if !instance.v6_main_ip.is_empty() && instance.v6_main_ip != "::" {
                    instance.v6_main_ip.clone()
                } else {
                    continue;
                };
                let mut metadata = super::ProviderMetadata::new();
                if !instance.region.is_empty() {
                    metadata.push("region", instance.region.clone());
                }
                if !instance.plan.is_empty() {
                    metadata.push("plan", instance.plan.clone());
                }
                if !instance.os.is_empty() {
                    metadata.push("os", instance.os.clone());
                }
                if !instance.power_status.is_empty() {
                    metadata.push("status", instance.power_status.clone());
                }
                hosts.push(ProviderHost {
                    server_id: instance.id.clone(),
                    name: instance.label.clone(),
                    ip,
                    tags: instance.tags.clone(),
                    metadata: metadata.finish(),
                    ..Default::default()
                });
            }

            let more = !resp.instances.is_empty() && !resp.meta.links.next.is_empty();
            cursor = if more {
                Some(resp.meta.links.next.clone())
            } else {
                None
            };
            Ok(super::PageResult { hosts, more })
        })
    }
}

impl Provider for Vultr {
    fn name(&self) -> &str {
        "vultr"
    }

    fn short_label(&self) -> &str {
        "vultr"
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
    fn test_parse_instance_response() {
        let json = r#"{
            "instances": [
                {
                    "id": "abc-123",
                    "label": "my-server",
                    "main_ip": "5.6.7.8",
                    "tags": ["web"]
                },
                {
                    "id": "def-456",
                    "label": "pending-server",
                    "main_ip": "0.0.0.0",
                    "tags": []
                }
            ],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances.len(), 2);
        assert_eq!(resp.instances[0].label, "my-server");
        assert_eq!(resp.instances[0].main_ip, "5.6.7.8");
        // Second instance has 0.0.0.0 (should be skipped)
        assert_eq!(resp.instances[1].main_ip, "0.0.0.0");
    }

    #[test]
    fn test_vultr_empty_ip_skipped() {
        let json = r#"{
            "instances": [
                {
                    "id": "abc-123",
                    "label": "empty-ip",
                    "main_ip": "",
                    "tags": []
                }
            ],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances.len(), 1);
        assert!(resp.instances[0].main_ip.is_empty());
        // This instance should be skipped during fetch_hosts because main_ip is empty
    }

    #[test]
    fn test_vultr_v6_fallback() {
        let json = r#"{
            "instances": [
                {
                    "id": "v6-only",
                    "label": "v6-server",
                    "main_ip": "0.0.0.0",
                    "v6_main_ip": "2001:db8::1",
                    "tags": []
                }
            ],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        let instance = &resp.instances[0];
        assert_eq!(instance.main_ip, "0.0.0.0");
        assert_eq!(instance.v6_main_ip, "2001:db8::1");
    }

    // Helper: same IP selection logic as fetch_hosts_cancellable
    fn select_vultr_ip(instance: &Instance) -> Option<String> {
        if !instance.main_ip.is_empty() && instance.main_ip != "0.0.0.0" {
            Some(instance.main_ip.clone())
        } else if !instance.v6_main_ip.is_empty() && instance.v6_main_ip != "::" {
            Some(instance.v6_main_ip.clone())
        } else {
            None
        }
    }

    #[test]
    fn test_vultr_both_placeholder_skipped() {
        let json = r#"{
            "instances": [
                {"id": "xyz", "label": "both-zero", "main_ip": "0.0.0.0", "v6_main_ip": "::", "tags": []}
            ],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_vultr_ip(&resp.instances[0]), None);
    }

    #[test]
    fn test_vultr_prefers_v4_over_v6() {
        let json = r#"{
            "instances": [
                {"id": "a", "label": "dual", "main_ip": "5.6.7.8", "v6_main_ip": "2001:db8::1", "tags": []}
            ],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_vultr_ip(&resp.instances[0]),
            Some("5.6.7.8".to_string())
        );
    }

    #[test]
    fn test_vultr_tags_preserved() {
        let json = r#"{
            "instances": [
                {"id": "t", "label": "tagged", "main_ip": "1.2.3.4", "tags": ["web", "prod"]}
            ],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances[0].tags, vec!["web", "prod"]);
    }

    #[test]
    fn test_vultr_cursor_pagination_has_next() {
        let json = r#"{
            "instances": [{"id": "a", "label": "a", "main_ip": "1.2.3.4", "tags": []}],
            "meta": {"links": {"next": "bmV4dA=="}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.meta.links.next.is_empty());
    }

    #[test]
    fn test_vultr_default_v6_empty_string() {
        // v6_main_ip defaults to "" when not in JSON
        let json = r#"{
            "instances": [{"id": "a", "label": "a", "main_ip": "1.2.3.4", "tags": []}],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances[0].v6_main_ip, "");
    }

    #[test]
    fn test_vultr_default_tags_empty() {
        let json = r#"{
            "instances": [{"id": "a", "label": "a", "main_ip": "1.2.3.4"}],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert!(resp.instances[0].tags.is_empty());
    }

    #[test]
    fn test_vultr_empty_instance_list_stops() {
        let json = r#"{
            "instances": [],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert!(resp.instances.is_empty());
    }

    #[test]
    fn test_vultr_string_id_preserved() {
        // Vultr uses string UUIDs for instance IDs, unlike other providers
        let json = r#"{
            "instances": [{
                "id": "cb676a46-66fd-4dfb-b839-443f2e6c0b60",
                "label": "uuid-test",
                "main_ip": "1.2.3.4",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances[0].id, "cb676a46-66fd-4dfb-b839-443f2e6c0b60");
    }

    #[test]
    fn test_vultr_valid_v4_ignores_placeholder_v6() {
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "v4-with-placeholder-v6",
                "main_ip": "5.6.7.8",
                "v6_main_ip": "::",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_vultr_ip(&resp.instances[0]),
            Some("5.6.7.8".to_string())
        );
    }

    #[test]
    fn test_vultr_empty_v4_and_valid_v6() {
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "empty-v4-valid-v6",
                "main_ip": "",
                "v6_main_ip": "2001:db8::1",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            select_vultr_ip(&resp.instances[0]),
            Some("2001:db8::1".to_string())
        );
    }

    #[test]
    fn test_vultr_empty_v4_and_empty_v6() {
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "both-empty",
                "main_ip": "",
                "v6_main_ip": "",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_vultr_ip(&resp.instances[0]), None);
    }

    #[test]
    fn test_vultr_multiple_tags() {
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "multi-tag",
                "main_ip": "1.2.3.4",
                "tags": ["web", "production", "us-east", "team-a"]
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances[0].tags.len(), 4);
    }

    #[test]
    fn test_vultr_missing_next_link() {
        // VultrLinks.next should default to empty string when missing
        let json = r#"{
            "instances": [],
            "meta": {"links": {}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert!(resp.meta.links.next.is_empty());
    }

    #[test]
    fn test_vultr_v6_placeholder_only() {
        // main_ip is 0.0.0.0 and v6 is :: → skipped
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "no-ip",
                "main_ip": "0.0.0.0",
                "v6_main_ip": "::",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(select_vultr_ip(&resp.instances[0]), None);
    }

    #[test]
    fn test_vultr_label_with_special_chars() {
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "web-server (prod) #1",
                "main_ip": "1.2.3.4",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances[0].label, "web-server (prod) #1");
    }

    #[test]
    fn test_vultr_v4_zero_not_empty() {
        // "0.0.0.0" is the placeholder, not empty string
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "pending",
                "main_ip": "0.0.0.0",
                "v6_main_ip": "2001:db8::1",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        // 0.0.0.0 treated as placeholder, falls back to v6
        assert_eq!(
            select_vultr_ip(&resp.instances[0]),
            Some("2001:db8::1".to_string())
        );
    }

    #[test]
    fn test_vultr_cursor_pagination_empty_next_stops() {
        let json = r#"{
            "instances": [{"id": "a", "label": "a", "main_ip": "1.2.3.4", "tags": []}],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert!(resp.meta.links.next.is_empty());
    }

    #[test]
    fn test_vultr_multiple_instances_parsed() {
        let json = r#"{
            "instances": [
                {"id": "a", "label": "web-1", "main_ip": "1.1.1.1", "tags": ["web"]},
                {"id": "b", "label": "web-2", "main_ip": "2.2.2.2", "tags": ["web"]},
                {"id": "c", "label": "db-1", "main_ip": "3.3.3.3", "tags": ["db"]}
            ],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances.len(), 3);
        for inst in &resp.instances {
            assert!(select_vultr_ip(inst).is_some());
        }
    }

    // --- v4 is 0.0.0.0 with no v6 field at all → None ---

    #[test]
    fn test_vultr_placeholder_v4_no_v6_field() {
        let json = r#"{
            "instances": [{
                "id": "a",
                "label": "no-v6",
                "main_ip": "0.0.0.0",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        // v6_main_ip defaults to "" which is also not usable
        assert_eq!(select_vultr_ip(&resp.instances[0]), None);
    }

    // --- valid v4 with empty label ---

    #[test]
    fn test_vultr_empty_label() {
        let json = r#"{
            "instances": [{
                "id": "x",
                "label": "",
                "main_ip": "1.2.3.4",
                "tags": []
            }],
            "meta": {"links": {"next": ""}}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances[0].label, "");
        assert_eq!(
            select_vultr_ip(&resp.instances[0]),
            Some("1.2.3.4".to_string())
        );
    }

    // --- Resilience: extra/unknown fields are ignored by serde ---

    #[test]
    fn test_vultr_extra_fields_ignored() {
        // Real Vultr API returns many more fields per instance
        let json = r#"{
            "instances": [{
                "id": "cb676a46-66fd-4dfb-b839-443f2e6c0b60",
                "os": "Ubuntu 22.04 LTS x64",
                "ram": 1024,
                "disk": 25,
                "main_ip": "45.76.1.1",
                "vcpu_count": 1,
                "region": "ewr",
                "plan": "vc2-1c-1gb",
                "date_created": "2024-01-01T00:00:00+00:00",
                "status": "active",
                "allowed_bandwidth": 1000,
                "netmask_v4": "255.255.254.0",
                "gateway_v4": "45.76.0.1",
                "power_status": "running",
                "server_status": "ok",
                "v6_main_ip": "2001:19f0::1",
                "v6_network": "2001:19f0::",
                "v6_network_size": 64,
                "label": "full-response",
                "internal_ip": "",
                "kvm": "https://my.vultr.com/subs/vps/novnc/...",
                "hostname": "full-response",
                "os_id": 1743,
                "app_id": 0,
                "image_id": "",
                "firewall_group_id": "",
                "features": ["auto_backups"],
                "tags": ["web", "prod"],
                "user_scheme": "root"
            }],
            "meta": {"links": {"next": "", "prev": ""}, "total": 1}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.instances[0].label, "full-response");
        assert_eq!(resp.instances[0].main_ip, "45.76.1.1");
        assert_eq!(resp.instances[0].v6_main_ip, "2001:19f0::1");
        assert_eq!(resp.instances[0].tags, vec!["web", "prod"]);
    }

    #[test]
    fn test_vultr_meta_extra_fields_ignored() {
        // Meta may contain additional fields like total and prev
        let json = r#"{
            "instances": [],
            "meta": {"links": {"next": "", "prev": ""}, "total": 0}
        }"#;
        let resp: InstanceResponse = serde_json::from_str(json).unwrap();
        assert!(resp.instances.is_empty());
        assert!(resp.meta.links.next.is_empty());
    }

    // ── HTTP roundtrip tests (mockito) ──────────────────────────────

    #[test]
    fn test_http_instances_roundtrip() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/instances")
            .match_query(mockito::Matcher::AllOf(vec![mockito::Matcher::UrlEncoded(
                "per_page".into(),
                "500".into(),
            )]))
            .match_header("Authorization", "Bearer test-vultr-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "instances": [
                        {
                            "id": "cb676a46-66fd-4dfb-b839-443f2e6c0b60",
                            "label": "web-prod-1",
                            "main_ip": "149.28.100.10",
                            "v6_main_ip": "2001:19f0:5001::1",
                            "tags": ["prod", "web"],
                            "region": "ewr",
                            "plan": "vc2-2c-4gb",
                            "os": "Ubuntu 22.04 LTS x64",
                            "power_status": "running"
                        }
                    ],
                    "meta": {"links": {"next": ""}}
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        let url = format!("{}/v2/instances?per_page=500", server.url());
        let resp: InstanceResponse = agent
            .get(&url)
            .header("Authorization", "Bearer test-vultr-token")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(resp.instances.len(), 1);
        let i = &resp.instances[0];
        assert_eq!(i.id, "cb676a46-66fd-4dfb-b839-443f2e6c0b60");
        assert_eq!(i.label, "web-prod-1");
        assert_eq!(i.main_ip, "149.28.100.10");
        assert_eq!(i.v6_main_ip, "2001:19f0:5001::1");
        assert_eq!(i.region, "ewr");
        assert_eq!(i.plan, "vc2-2c-4gb");
        assert_eq!(i.os, "Ubuntu 22.04 LTS x64");
        assert_eq!(i.power_status, "running");
        assert_eq!(i.tags, vec!["prod", "web"]);
        assert!(resp.meta.links.next.is_empty());
        mock.assert();
    }

    #[test]
    fn test_http_instances_pagination() {
        let mut server = mockito::Server::new();
        let page1 = server
            .mock("GET", "/v2/instances")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("per_page".into(), "500".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "instances": [{"id": "aaa", "label": "web-1", "main_ip": "1.1.1.1", "tags": []}],
                    "meta": {"links": {"next": "bmV4dEN1cnNvcg=="}}
                }"#,
            )
            .create();
        let page2 = server
            .mock("GET", "/v2/instances")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("per_page".into(), "500".into()),
                mockito::Matcher::UrlEncoded("cursor".into(), "bmV4dEN1cnNvcg==".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "instances": [{"id": "bbb", "label": "web-2", "main_ip": "2.2.2.2", "tags": []}],
                    "meta": {"links": {"next": ""}}
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        // Page 1
        let r1: InstanceResponse = agent
            .get(&format!("{}/v2/instances?per_page=500", server.url()))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r1.instances.len(), 1);
        assert_eq!(r1.instances[0].id, "aaa");
        assert_eq!(r1.meta.links.next, "bmV4dEN1cnNvcg==");
        // Page 2
        let r2: InstanceResponse = agent
            .get(&format!(
                "{}/v2/instances?per_page=500&cursor=bmV4dEN1cnNvcg==",
                server.url()
            ))
            .header("Authorization", "Bearer tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r2.instances.len(), 1);
        assert_eq!(r2.instances[0].id, "bbb");
        assert!(r2.meta.links.next.is_empty());
        page1.assert();
        page2.assert();
    }

    #[test]
    fn fetch_from_drives_full_pipeline_against_mock() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/instances")
            .match_query(mockito::Matcher::AllOf(vec![mockito::Matcher::UrlEncoded(
                "per_page".into(),
                "500".into(),
            )]))
            .match_header("Authorization", "Bearer test-vultr-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "instances": [
                        {
                            "id": "cb676a46-66fd-4dfb-b839-443f2e6c0b60",
                            "label": "web-prod-1",
                            "main_ip": "149.28.100.10",
                            "v6_main_ip": "2001:19f0:5001::1",
                            "tags": ["prod", "web"],
                            "region": "ewr",
                            "plan": "vc2-2c-4gb",
                            "os": "Ubuntu 22.04 LTS x64",
                            "power_status": "running"
                        },
                        {
                            "id": "pending-1",
                            "label": "pending",
                            "main_ip": "0.0.0.0",
                            "v6_main_ip": "::",
                            "tags": []
                        }
                    ],
                    "meta": {"links": {"next": ""}}
                }"#,
            )
            .create();

        let hosts = Vultr
            .fetch_from(&server.url(), "test-vultr-token", &AtomicBool::new(false))
            .unwrap();

        // The placeholder-only instance is skipped; only the real one maps.
        assert_eq!(hosts.len(), 1);
        let h = &hosts[0];
        assert_eq!(h.server_id, "cb676a46-66fd-4dfb-b839-443f2e6c0b60");
        assert_eq!(h.name, "web-prod-1");
        assert_eq!(h.ip, "149.28.100.10");
        assert_eq!(h.tags, vec!["prod", "web"]);
        assert_eq!(
            h.metadata,
            vec![
                ("region".to_string(), "ewr".to_string()),
                ("plan".to_string(), "vc2-2c-4gb".to_string()),
                ("os".to_string(), "Ubuntu 22.04 LTS x64".to_string()),
                ("status".to_string(), "running".to_string()),
            ]
        );
        mock.assert();
    }

    #[test]
    fn fetch_from_maps_auth_failure_to_provider_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/instances")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"error": "Invalid API token", "status": 401}"#)
            .create();

        let err = Vultr
            .fetch_from(&server.url(), "bad-token", &AtomicBool::new(false))
            .unwrap_err();
        assert!(matches!(err, ProviderError::AuthFailed));
        mock.assert();
    }

    #[test]
    fn test_http_instances_auth_failure() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/v2/instances")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"error": "Invalid API token", "status": 401}"#)
            .create();

        let agent = super::super::http_agent();
        let result = agent
            .get(&format!("{}/v2/instances?per_page=500", server.url()))
            .header("Authorization", "Bearer bad-token")
            .call();

        match result {
            Err(ureq::Error::StatusCode(401)) => {} // expected
            other => panic!("expected 401 error, got {:?}", other),
        }
        mock.assert();
    }
}
