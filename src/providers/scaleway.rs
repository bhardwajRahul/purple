use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;

use super::{Provider, ProviderError, ProviderHost, map_ureq_error};

pub struct Scaleway {
    pub zones: Vec<String>,
}

/// All Scaleway availability zones with display names.
/// Single source of truth. SCW_ZONE_GROUPS references slices of this array.
pub const SCW_ZONES: &[(&str, &str)] = &[
    // Paris (0..3)
    ("fr-par-1", "Paris 1"),
    ("fr-par-2", "Paris 2"),
    ("fr-par-3", "Paris 3"),
    // Amsterdam (3..6)
    ("nl-ams-1", "Amsterdam 1"),
    ("nl-ams-2", "Amsterdam 2"),
    ("nl-ams-3", "Amsterdam 3"),
    // Warsaw (6..9)
    ("pl-waw-1", "Warsaw 1"),
    ("pl-waw-2", "Warsaw 2"),
    ("pl-waw-3", "Warsaw 3"),
    // Milan (9..10)
    ("it-mil-1", "Milan 1"),
];

/// Zone group labels with start..end indices into SCW_ZONES.
pub const SCW_ZONE_GROUPS: &[(&str, usize, usize)] = &[
    ("Paris", 0, 3),
    ("Amsterdam", 3, 6),
    ("Warsaw", 6, 9),
    ("Milan", 9, 10),
];

// --- Serde response models ---

#[derive(Deserialize)]
struct ListServersResponse {
    #[serde(default)]
    servers: Vec<ScalewayServer>,
    #[serde(default)]
    total_count: u64,
}

#[derive(Deserialize)]
struct ScalewayServer {
    id: String,
    name: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    commercial_type: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    public_ips: Vec<ServerIp>,
    #[serde(default)]
    public_ip: Option<LegacyPublicIp>,
    #[serde(default)]
    private_ip: Option<String>,
    #[serde(default)]
    image: Option<ScalewayImage>,
    #[serde(default)]
    #[allow(dead_code)]
    // Deserialized from API but we use the zone parameter from the request URL
    zone: String,
}

#[derive(Deserialize)]
struct ServerIp {
    #[serde(default)]
    address: String,
    #[serde(default)]
    family: String,
}

#[derive(Deserialize)]
struct LegacyPublicIp {
    #[serde(default)]
    address: String,
}

#[derive(Deserialize)]
struct ScalewayImage {
    #[serde(default)]
    name: Option<String>,
}

/// Build metadata key-value pairs for a server.
fn build_metadata(server: &ScalewayServer, zone: &str) -> Vec<(String, String)> {
    let mut metadata = super::ProviderMetadata::new();
    if !zone.is_empty() {
        metadata.push("zone", zone.to_string());
    }
    if !server.commercial_type.is_empty() {
        metadata.push("type", server.commercial_type.clone());
    }
    if let Some(ref image) = server.image {
        if let Some(ref name) = image.name {
            if !name.is_empty() {
                metadata.push("image", name.clone());
            }
        }
    }
    if !server.state.is_empty() {
        metadata.push("status", server.state.clone());
    }
    metadata.finish()
}

/// Select the best IP for a server.
/// Prefers public IPv4 > public IPv6 > legacy public_ip > private_ip.
fn select_ip(server: &ScalewayServer) -> Option<String> {
    // Prefer public IPv4 from public_ips
    if let Some(ip) = server
        .public_ips
        .iter()
        .find(|ip| ip.family == "inet" && !ip.address.is_empty())
    {
        return Some(super::strip_cidr(&ip.address).to_string());
    }
    // Fall back to public IPv6 from public_ips
    if let Some(ip) = server
        .public_ips
        .iter()
        .find(|ip| ip.family == "inet6" && !ip.address.is_empty())
    {
        return Some(super::strip_cidr(&ip.address).to_string());
    }
    // Fall back to legacy public_ip field
    if let Some(ref legacy) = server.public_ip {
        if !legacy.address.is_empty() {
            return Some(legacy.address.clone());
        }
    }
    // Fall back to private_ip
    if let Some(ref priv_ip) = server.private_ip {
        if !priv_ip.is_empty() {
            return Some(priv_ip.clone());
        }
    }
    None
}

impl Scaleway {
    /// Real API host. Overridable per call via `fetch_from` so tests can point
    /// the full fetch pipeline at a mock server.
    const API_BASE: &'static str = "https://api.scaleway.com";

    /// Fetch hosts against an explicit API base. Production passes `API_BASE`;
    /// tests pass a mock server URL. The single seam that makes zone fan-out,
    /// URL construction, the auth header, error mapping, pagination and
    /// `ProviderHost` mapping testable end to end. Preserves partial-failure
    /// semantics: only `base_url` is routed through.
    fn fetch_from(
        &self,
        base_url: &str,
        token: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        self.fetch_from_with_progress(base_url, token, cancel, &|_| {})
    }

    fn fetch_from_with_progress(
        &self,
        base_url: &str,
        token: &str,
        cancel: &AtomicBool,
        progress: &dyn Fn(&str),
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        if self.zones.is_empty() {
            return Err(ProviderError::Http(
                "No Scaleway zones configured. Add zones in the provider settings.".to_string(),
            ));
        }

        let valid_codes: HashSet<&str> = SCW_ZONES.iter().map(|(c, _)| *c).collect();
        for zone in &self.zones {
            if !valid_codes.contains(zone.as_str()) {
                return Err(ProviderError::Http(format!(
                    "Unknown Scaleway zone '{}'. Check your provider settings.",
                    zone
                )));
            }
        }

        let agent = super::http_agent();
        let total_zones = self.zones.len();
        let mut all_hosts = Vec::new();
        let mut failed_zones = 0usize;

        for (i, zone) in self.zones.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(ProviderError::Cancelled);
            }

            progress(&format!("Fetching {} ({}/{})...", zone, i + 1, total_zones));

            match fetch_zone(base_url, &agent, token, zone, cancel) {
                Ok(hosts) => all_hosts.extend(hosts),
                Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
                Err(ProviderError::AuthFailed) => return Err(ProviderError::AuthFailed),
                Err(ProviderError::RateLimited) => return Err(ProviderError::RateLimited),
                Err(_) => {
                    failed_zones += 1;
                    continue;
                }
            }
        }

        // Summary
        let mut parts = vec![format!("{} instances", all_hosts.len())];
        if failed_zones > 0 {
            parts.push(format!("{} of {} zones failed", failed_zones, total_zones));
        }
        progress(&parts.join(", "));

        if failed_zones > 0 {
            if all_hosts.is_empty() {
                return Err(ProviderError::Http(format!(
                    "All {} zones failed. Check your credentials and zone configuration.",
                    total_zones,
                )));
            }
            return Err(ProviderError::PartialResult {
                hosts: all_hosts,
                failures: failed_zones,
                total: total_zones,
            });
        }

        Ok(all_hosts)
    }
}

impl Provider for Scaleway {
    fn name(&self) -> &str {
        "scaleway"
    }

    fn short_label(&self) -> &str {
        "scw"
    }

    fn fetch_hosts_cancellable(
        &self,
        token: &str,
        cancel: &AtomicBool,
        _env: &crate::runtime::env::Env,
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        self.fetch_from(Self::API_BASE, token, cancel)
    }

    fn fetch_hosts_with_progress(
        &self,
        token: &str,
        cancel: &AtomicBool,
        _env: &crate::runtime::env::Env,
        progress: &dyn Fn(&str),
    ) -> Result<Vec<ProviderHost>, ProviderError> {
        self.fetch_from_with_progress(Self::API_BASE, token, cancel, progress)
    }
}

/// Fetch all servers in a single zone (handles pagination).
fn fetch_zone(
    base_url: &str,
    agent: &ureq::Agent,
    token: &str,
    zone: &str,
    cancel: &AtomicBool,
) -> Result<Vec<ProviderHost>, ProviderError> {
    let mut hosts = Vec::new();
    let mut page = 1u64;
    let per_page = 100;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ProviderError::Cancelled);
        }

        let url = format!(
            "{}/instance/v1/zones/{}/servers?page={}&per_page={}",
            base_url, zone, page, per_page
        );
        let resp: ListServersResponse = agent
            .get(&url)
            .header("X-Auth-Token", token)
            .call()
            .map_err(map_ureq_error)?
            .body_mut()
            .read_json()
            .map_err(|e| ProviderError::Parse(format!("{}: {}", zone, e)))?;

        if resp.servers.is_empty() {
            break;
        }

        let count = resp.servers.len();

        for server in &resp.servers {
            if let Some(ip) = select_ip(server) {
                hosts.push(ProviderHost {
                    server_id: server.id.clone(),
                    name: server.name.clone(),
                    ip,
                    tags: server.tags.clone(),
                    metadata: build_metadata(server, zone),
                    ..Default::default()
                });
            }
        }

        let total = resp.total_count;
        if (count as u64) < per_page || (total > 0 && page * per_page >= total) {
            break;
        }
        page += 1;
        if page > 500 {
            break;
        }
    }

    Ok(hosts)
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Response parsing
    // =========================================================================

    #[test]
    fn test_parse_list_servers_response() {
        let json = r#"{
            "servers": [
                {
                    "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
                    "name": "web-1",
                    "state": "running",
                    "commercial_type": "DEV1-S",
                    "tags": ["production"],
                    "public_ips": [
                        {"id": "ip-1", "address": "51.15.1.2", "family": "inet"}
                    ],
                    "zone": "fr-par-1"
                }
            ]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers.len(), 1);
        assert_eq!(resp.servers[0].name, "web-1");
        assert_eq!(resp.servers[0].state, "running");
        assert_eq!(resp.servers[0].commercial_type, "DEV1-S");
    }

    #[test]
    fn test_parse_server_with_public_ips() {
        let json = r#"{
            "servers": [{
                "id": "abc",
                "name": "dual",
                "public_ips": [
                    {"address": "51.15.1.2", "family": "inet"},
                    {"address": "2001:bc8::1", "family": "inet6"}
                ],
                "tags": []
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].public_ips.len(), 2);
        assert_eq!(resp.servers[0].public_ips[0].family, "inet");
        assert_eq!(resp.servers[0].public_ips[1].family, "inet6");
    }

    #[test]
    fn test_parse_server_with_legacy_public_ip() {
        let json = r#"{
            "servers": [{
                "id": "abc",
                "name": "legacy",
                "public_ips": [],
                "public_ip": {"address": "51.15.1.2", "dynamic": false},
                "tags": []
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            resp.servers[0].public_ip.as_ref().unwrap().address,
            "51.15.1.2"
        );
    }

    #[test]
    fn test_parse_server_extra_fields_ignored() {
        let json = r#"{
            "servers": [{
                "id": "abc",
                "name": "full",
                "state": "running",
                "commercial_type": "GP1-M",
                "tags": ["web"],
                "public_ips": [{"address": "1.2.3.4", "family": "inet"}],
                "created_at": "2024-01-01T00:00:00Z",
                "disk": 25,
                "memory": 2147483648,
                "arch": "x86_64",
                "hostname": "full",
                "zone": "fr-par-1"
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].name, "full");
    }

    // =========================================================================
    // IP selection
    // =========================================================================

    fn server_with_ips(
        public_ips: Vec<ServerIp>,
        public_ip: Option<LegacyPublicIp>,
        private_ip: Option<String>,
    ) -> ScalewayServer {
        ScalewayServer {
            id: "test".to_string(),
            name: "test".to_string(),
            state: String::new(),
            commercial_type: String::new(),
            tags: vec![],
            public_ips,
            public_ip,
            private_ip,
            image: None,
            zone: String::new(),
        }
    }

    #[test]
    fn test_select_ip_prefers_v4_over_v6() {
        let server = server_with_ips(
            vec![
                ServerIp {
                    address: "51.15.1.2".to_string(),
                    family: "inet".to_string(),
                },
                ServerIp {
                    address: "2001:bc8::1".to_string(),
                    family: "inet6".to_string(),
                },
            ],
            None,
            None,
        );
        assert_eq!(select_ip(&server), Some("51.15.1.2".to_string()));
    }

    #[test]
    fn test_select_ip_v6_only() {
        let server = server_with_ips(
            vec![ServerIp {
                address: "2001:bc8::1".to_string(),
                family: "inet6".to_string(),
            }],
            None,
            None,
        );
        assert_eq!(select_ip(&server), Some("2001:bc8::1".to_string()));
    }

    #[test]
    fn test_select_ip_empty_public_ips_uses_legacy() {
        let server = server_with_ips(
            vec![],
            Some(LegacyPublicIp {
                address: "51.15.1.2".to_string(),
            }),
            None,
        );
        assert_eq!(select_ip(&server), Some("51.15.1.2".to_string()));
    }

    #[test]
    fn test_select_ip_falls_back_to_private() {
        let server = server_with_ips(vec![], None, Some("10.0.0.5".to_string()));
        assert_eq!(select_ip(&server), Some("10.0.0.5".to_string()));
    }

    #[test]
    fn test_select_ip_no_ip_returns_none() {
        let server = server_with_ips(vec![], None, None);
        assert_eq!(select_ip(&server), None);
    }

    #[test]
    fn test_select_ip_empty_address_skipped() {
        let server = server_with_ips(
            vec![ServerIp {
                address: String::new(),
                family: "inet".to_string(),
            }],
            None,
            None,
        );
        assert_eq!(select_ip(&server), None);
    }

    #[test]
    fn test_select_ip_v6_cidr_stripped() {
        let server = server_with_ips(
            vec![ServerIp {
                address: "2001:bc8::1/128".to_string(),
                family: "inet6".to_string(),
            }],
            None,
            None,
        );
        assert_eq!(select_ip(&server), Some("2001:bc8::1".to_string()));
    }

    #[test]
    fn test_select_ip_multiple_v4_uses_first() {
        let server = server_with_ips(
            vec![
                ServerIp {
                    address: "51.15.1.2".to_string(),
                    family: "inet".to_string(),
                },
                ServerIp {
                    address: "51.15.1.3".to_string(),
                    family: "inet".to_string(),
                },
            ],
            None,
            None,
        );
        assert_eq!(select_ip(&server), Some("51.15.1.2".to_string()));
    }

    #[test]
    fn test_select_ip_empty_private_skipped() {
        let server = server_with_ips(vec![], None, Some(String::new()));
        assert_eq!(select_ip(&server), None);
    }

    // =========================================================================
    // Tags
    // =========================================================================

    #[test]
    fn test_tags_preserved() {
        let json = r#"{
            "servers": [{
                "id": "abc",
                "name": "tagged",
                "public_ips": [{"address": "1.2.3.4", "family": "inet"}],
                "tags": ["web", "production", "eu"]
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].tags, vec!["web", "production", "eu"]);
    }

    #[test]
    fn test_default_tags_empty() {
        let json = r#"{
            "servers": [{"id": "abc", "name": "no-tags", "public_ips": []}]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert!(resp.servers[0].tags.is_empty());
    }

    // =========================================================================
    // Metadata
    // =========================================================================

    #[test]
    fn test_metadata_from_server() {
        let server = ScalewayServer {
            id: "abc".to_string(),
            name: "web-1".to_string(),
            state: "running".to_string(),
            commercial_type: "DEV1-S".to_string(),
            tags: vec![],
            public_ips: vec![ServerIp {
                address: "1.2.3.4".to_string(),
                family: "inet".to_string(),
            }],
            public_ip: None,
            private_ip: None,
            image: Some(ScalewayImage {
                name: Some("Ubuntu 22.04 Jammy Jellyfish".to_string()),
            }),
            zone: "fr-par-1".to_string(),
        };
        let ip = select_ip(&server).unwrap();
        assert_eq!(ip, "1.2.3.4");

        let metadata = build_metadata(&server, "fr-par-1");
        assert_eq!(
            metadata,
            vec![
                ("zone".to_string(), "fr-par-1".to_string()),
                ("type".to_string(), "DEV1-S".to_string()),
                (
                    "image".to_string(),
                    "Ubuntu 22.04 Jammy Jellyfish".to_string()
                ),
                ("status".to_string(), "running".to_string()),
            ]
        );
    }

    #[test]
    fn test_metadata_uses_zone_param_not_server_field() {
        let server = ScalewayServer {
            id: "abc".to_string(),
            name: "web-1".to_string(),
            state: "running".to_string(),
            commercial_type: String::new(),
            tags: vec![],
            public_ips: vec![],
            public_ip: None,
            private_ip: None,
            image: None,
            zone: "nl-ams-2".to_string(),
        };
        let metadata = build_metadata(&server, "fr-par-1");
        assert_eq!(metadata[0], ("zone".to_string(), "fr-par-1".to_string()));
    }

    #[test]
    fn test_metadata_empty_fields_omitted() {
        let server = ScalewayServer {
            id: "abc".to_string(),
            name: "bare".to_string(),
            state: String::new(),
            commercial_type: String::new(),
            tags: vec![],
            public_ips: vec![ServerIp {
                address: "1.2.3.4".to_string(),
                family: "inet".to_string(),
            }],
            public_ip: None,
            private_ip: None,
            image: None,
            zone: String::new(),
        };
        let metadata = build_metadata(&server, "");
        assert!(metadata.is_empty());
    }

    // =========================================================================
    // Pagination
    // =========================================================================

    #[test]
    fn test_empty_server_list_stops_pagination() {
        let json = r#"{"servers": []}"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert!(resp.servers.is_empty());
    }

    // =========================================================================
    // Zone constants
    // =========================================================================

    #[test]
    fn test_scw_zones_count() {
        assert_eq!(SCW_ZONES.len(), 10);
    }

    #[test]
    fn test_scw_zone_groups_cover_all_zones() {
        let total: usize = SCW_ZONE_GROUPS.iter().map(|&(_, s, e)| e - s).sum();
        assert_eq!(total, SCW_ZONES.len());
        let mut expected_start = 0;
        for &(_, start, end) in SCW_ZONE_GROUPS {
            assert_eq!(start, expected_start, "Gap or overlap in zone groups");
            assert!(end > start, "Empty zone group");
            expected_start = end;
        }
        assert_eq!(expected_start, SCW_ZONES.len());
    }

    #[test]
    fn test_scw_zones_no_duplicates() {
        let mut seen = HashSet::new();
        for (code, _) in SCW_ZONES {
            assert!(seen.insert(code), "Duplicate zone: {}", code);
        }
    }

    #[test]
    fn test_scw_zones_contains_common() {
        let codes: Vec<&str> = SCW_ZONES.iter().map(|(c, _)| *c).collect();
        assert!(codes.contains(&"fr-par-1"));
        assert!(codes.contains(&"nl-ams-1"));
        assert!(codes.contains(&"pl-waw-1"));
        assert!(codes.contains(&"it-mil-1"));
    }

    // =========================================================================
    // Provider trait
    // =========================================================================

    #[test]
    fn test_scaleway_provider_name() {
        let scw = Scaleway { zones: vec![] };
        assert_eq!(scw.name(), "scaleway");
        assert_eq!(scw.short_label(), "scw");
    }

    #[test]
    fn test_scaleway_no_zones_error() {
        let scw = Scaleway { zones: vec![] };
        let result = scw.fetch_hosts("fake-token", &crate::runtime::env::Env::empty());
        match result {
            Err(ProviderError::Http(msg)) => assert!(msg.contains("No Scaleway zones")),
            other => panic!("Expected Http error, got: {:?}", other),
        }
    }

    #[test]
    fn test_scaleway_invalid_zone_error() {
        let scw = Scaleway {
            zones: vec!["xx-invalid-1".to_string()],
        };
        let result = scw.fetch_hosts("fake-token", &crate::runtime::env::Env::empty());
        match result {
            Err(ProviderError::Http(msg)) => assert!(msg.contains("Unknown Scaleway zone")),
            other => panic!("Expected Http error for invalid zone, got: {:?}", other),
        }
    }

    #[test]
    fn test_scaleway_mixed_valid_invalid_zone_error() {
        let scw = Scaleway {
            zones: vec!["fr-par-1".to_string(), "xx-fake-9".to_string()],
        };
        let result = scw.fetch_hosts("fake-token", &crate::runtime::env::Env::empty());
        match result {
            Err(ProviderError::Http(msg)) => assert!(msg.contains("xx-fake-9")),
            other => panic!("Expected Http error for invalid zone, got: {:?}", other),
        }
    }

    // =========================================================================
    // Server ID is UUID string
    // =========================================================================

    #[test]
    fn test_server_id_is_uuid_string() {
        let json = r#"{
            "servers": [{
                "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
                "name": "uuid-test",
                "public_ips": [],
                "tags": []
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].id, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    }

    // =========================================================================
    // Image parsing
    // =========================================================================

    #[test]
    fn test_image_name_parsed() {
        let json = r#"{
            "servers": [{
                "id": "abc",
                "name": "with-image",
                "image": {"id": "img-1", "name": "Ubuntu 22.04 Jammy Jellyfish"},
                "public_ips": [],
                "tags": []
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            resp.servers[0].image.as_ref().unwrap().name.as_deref(),
            Some("Ubuntu 22.04 Jammy Jellyfish")
        );
    }

    #[test]
    fn test_image_null_handled() {
        let json = r#"{
            "servers": [{
                "id": "abc",
                "name": "no-image",
                "image": null,
                "public_ips": [],
                "tags": []
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert!(resp.servers[0].image.is_none());
    }

    // =========================================================================
    // Private IP field
    // =========================================================================

    #[test]
    fn test_private_ip_parsed() {
        let json = r#"{
            "servers": [{
                "id": "abc",
                "name": "priv",
                "private_ip": "10.1.2.3",
                "public_ips": [],
                "tags": []
            }]
        }"#;
        let resp: ListServersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.servers[0].private_ip.as_deref(), Some("10.1.2.3"));
    }

    // =========================================================================
    // HTTP roundtrip tests (mockito)
    // =========================================================================

    #[test]
    fn test_http_list_servers_roundtrip() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/instance/v1/zones/fr-par-1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "100".into()),
            ]))
            .match_header("X-Auth-Token", "scw-secret-token-123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [
                        {
                            "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
                            "name": "web-prod-1",
                            "state": "running",
                            "commercial_type": "DEV1-S",
                            "tags": ["production", "web"],
                            "public_ips": [
                                {"address": "51.15.42.10", "family": "inet"},
                                {"address": "2001:bc8:1200::1", "family": "inet6"}
                            ],
                            "private_ip": "10.68.0.5",
                            "image": {"id": "img-1", "name": "Ubuntu 22.04 Jammy Jellyfish"},
                            "zone": "fr-par-1"
                        }
                    ],
                    "total_count": 1
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        let url = format!(
            "{}/instance/v1/zones/fr-par-1/servers?page=1&per_page=100",
            server.url()
        );
        let resp: ListServersResponse = agent
            .get(&url)
            .header("X-Auth-Token", "scw-secret-token-123")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();

        assert_eq!(resp.servers.len(), 1);
        let s = &resp.servers[0];
        assert_eq!(s.id, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
        assert_eq!(s.name, "web-prod-1");
        assert_eq!(s.state, "running");
        assert_eq!(s.commercial_type, "DEV1-S");
        assert_eq!(s.tags, vec!["production", "web"]);
        assert_eq!(s.public_ips.len(), 2);
        assert_eq!(s.public_ips[0].address, "51.15.42.10");
        assert_eq!(s.public_ips[0].family, "inet");
        assert_eq!(select_ip(s), Some("51.15.42.10".to_string()));
        assert_eq!(resp.total_count, 1);
        mock.assert();
    }

    #[test]
    fn test_http_list_servers_pagination() {
        let mut server = mockito::Server::new();
        let page1 = server
            .mock("GET", "/instance/v1/zones/nl-ams-1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "100".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [{"id": "s1", "name": "a", "public_ips": [{"address": "1.1.1.1", "family": "inet"}], "tags": []}],
                    "total_count": 2
                }"#,
            )
            .create();
        let page2 = server
            .mock("GET", "/instance/v1/zones/nl-ams-1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "2".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "100".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [{"id": "s2", "name": "b", "public_ips": [{"address": "2.2.2.2", "family": "inet"}], "tags": []}],
                    "total_count": 2
                }"#,
            )
            .create();

        let agent = super::super::http_agent();
        // Page 1
        let r1: ListServersResponse = agent
            .get(&format!(
                "{}/instance/v1/zones/nl-ams-1/servers?page=1&per_page=100",
                server.url()
            ))
            .header("X-Auth-Token", "tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r1.servers.len(), 1);
        assert_eq!(r1.total_count, 2);
        // Page 2
        let r2: ListServersResponse = agent
            .get(&format!(
                "{}/instance/v1/zones/nl-ams-1/servers?page=2&per_page=100",
                server.url()
            ))
            .header("X-Auth-Token", "tk")
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(r2.servers.len(), 1);
        page1.assert();
        page2.assert();
    }

    #[test]
    fn test_http_list_servers_auth_failure() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/instance/v1/zones/fr-par-1/servers")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"message": "Invalid authentication token"}"#)
            .create();

        let agent = super::super::http_agent();
        let result = agent
            .get(&format!(
                "{}/instance/v1/zones/fr-par-1/servers?page=1&per_page=100",
                server.url()
            ))
            .header("X-Auth-Token", "bad-token")
            .call();

        match result {
            Err(ureq::Error::StatusCode(401)) => {} // expected
            other => panic!("expected 401 error, got {:?}", other),
        }
        mock.assert();
    }

    #[test]
    fn fetch_from_drives_full_pipeline_against_mock() {
        // Exercises the production fetch path end to end: zone fan-out, URL
        // construction, auth header, JSON deserialize, ProviderHost mapping and
        // pagination termination. A single zone keeps one mock endpoint enough.
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/instance/v1/zones/fr-par-1/servers")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("page".into(), "1".into()),
                mockito::Matcher::UrlEncoded("per_page".into(), "100".into()),
            ]))
            .match_header("X-Auth-Token", "scw-tk-42")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "servers": [
                        {
                            "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
                            "name": "web-prod-1",
                            "state": "running",
                            "commercial_type": "DEV1-S",
                            "tags": ["production", "web"],
                            "public_ips": [
                                {"address": "51.15.42.10", "family": "inet"}
                            ],
                            "image": {"id": "img-1", "name": "Ubuntu 22.04 Jammy Jellyfish"},
                            "zone": "fr-par-1"
                        }
                    ],
                    "total_count": 1
                }"#,
            )
            .create();

        let hosts = Scaleway {
            zones: vec!["fr-par-1".into()],
        }
        .fetch_from(&server.url(), "scw-tk-42", &AtomicBool::new(false))
        .expect("fetch_from must succeed against the mock");
        mock.assert();

        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].server_id, "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
        assert_eq!(hosts[0].name, "web-prod-1");
        assert_eq!(hosts[0].ip, "51.15.42.10");
        assert_eq!(hosts[0].tags, vec!["production", "web"]);
        assert!(
            hosts[0]
                .metadata
                .contains(&("zone".to_string(), "fr-par-1".to_string()))
        );
    }

    #[test]
    fn fetch_from_maps_auth_failure_to_provider_error() {
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/instance/v1/zones/fr-par-1/servers")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"message": "Invalid authentication token"}"#)
            .create();

        let result = Scaleway {
            zones: vec!["fr-par-1".into()],
        }
        .fetch_from(&server.url(), "bad", &AtomicBool::new(false));
        mock.assert();
        assert!(
            matches!(result, Err(ProviderError::AuthFailed)),
            "401 must map to ProviderError::AuthFailed, got {result:?}"
        );
    }
}
