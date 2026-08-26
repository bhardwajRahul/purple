//! OpenAPI schema validation tests.
//!
//! Each test loads a vendored OpenAPI schema fragment from
//! `tests/api_contracts/openapi/` and verifies that the corresponding golden
//! fixture contains all fields the provider spec marks as required.
//!
//! See `tests/api_contracts/openapi/README.md` for the fragment format.

mod common;

use common::{assert_has_key, load_json};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn openapi_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("api_contracts")
        .join("openapi")
}

fn load_schema(name: &str) -> Value {
    let path = openapi_dir().join(name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read schema {}: {}", path.display(), e));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("failed to parse schema {}: {}", name, e))
}

/// Load a schema fragment, load its referenced fixture, assert all required paths exist.
///
/// If the schema has `"root": "array"`, paths are validated against the first
/// element of the fixture array (for APIs like Oracle/OVH that return bare arrays).
fn validate_schema(schema_name: &str) {
    let schema = load_schema(schema_name);
    let fixture_name = schema["fixture"]
        .as_str()
        .unwrap_or_else(|| panic!("schema {} missing 'fixture' field", schema_name));
    let fixture = load_json(fixture_name);

    let root_is_array = schema
        .get("root")
        .and_then(|v| v.as_str())
        .is_some_and(|v| v == "array");

    let target = if root_is_array {
        fixture
            .get(0)
            .unwrap_or_else(|| panic!("fixture {} is empty array", fixture_name))
    } else {
        &fixture
    };

    let required_paths = schema["required_paths"]
        .as_array()
        .unwrap_or_else(|| panic!("schema {} missing 'required_paths' array", schema_name));

    for path_val in required_paths {
        let path = path_val
            .as_str()
            .unwrap_or_else(|| panic!("non-string path in schema {}", schema_name));
        assert_has_key(target, path);
    }
}

// ── Azure ────────────────────────────────────────────────────────────

#[test]
fn schema_azure_token() {
    validate_schema("azure_token.json");
}

#[test]
fn schema_azure_vms() {
    validate_schema("azure_vms.json");
}

#[test]
fn schema_azure_nics() {
    validate_schema("azure_nics.json");
}

#[test]
fn schema_azure_public_ips() {
    validate_schema("azure_public_ips.json");
}

// ── DigitalOcean ─────────────────────────────────────────────────────

#[test]
fn schema_digitalocean_droplets() {
    validate_schema("digitalocean_droplets.json");
}

// ── GCP ──────────────────────────────────────────────────────────────

#[test]
fn schema_gcp_token() {
    validate_schema("gcp_token.json");
}

#[test]
fn schema_gcp_aggregated_instances() {
    validate_schema("gcp_aggregated_instances.json");
}

// ── Hetzner ──────────────────────────────────────────────────────────

#[test]
fn schema_hetzner_servers() {
    validate_schema("hetzner_servers.json");
}

// ── Leaseweb ─────────────────────────────────────────────────────────

#[test]
fn schema_leaseweb_baremetal() {
    validate_schema("leaseweb_baremetal.json");
}

#[test]
fn schema_leaseweb_cloud() {
    validate_schema("leaseweb_cloud.json");
}

// ── Linode ───────────────────────────────────────────────────────────

#[test]
fn schema_linode_instances() {
    validate_schema("linode_instances.json");
}

// ── NetBox ───────────────────────────────────────────────────────────

#[test]
fn schema_netbox_devices() {
    validate_schema("netbox_devices.json");
}

#[test]
fn schema_netbox_virtual_machines() {
    validate_schema("netbox_virtual_machines.json");
}

// ── Oracle Cloud ─────────────────────────────────────────────────────

#[test]
fn schema_oracle_compartments() {
    validate_schema("oracle_compartments.json");
}

#[test]
fn schema_oracle_instances() {
    validate_schema("oracle_instances.json");
}

#[test]
fn schema_oracle_vnic_attachments() {
    validate_schema("oracle_vnic_attachments.json");
}

#[test]
fn schema_oracle_vnic() {
    validate_schema("oracle_vnic.json");
}

#[test]
fn schema_oracle_image() {
    validate_schema("oracle_image.json");
}

// ── OVHcloud ─────────────────────────────────────────────────────────

#[test]
fn schema_ovh_instances() {
    validate_schema("ovh_instances.json");
}

// ── Scaleway ─────────────────────────────────────────────────────────

#[test]
fn schema_scaleway_servers() {
    validate_schema("scaleway_servers.json");
}

// ── Tailscale ────────────────────────────────────────────────────────

#[test]
fn schema_tailscale_api_devices() {
    validate_schema("tailscale_api_devices.json");
}

// ── UpCloud ──────────────────────────────────────────────────────────

#[test]
fn schema_upcloud_servers() {
    validate_schema("upcloud_servers.json");
}

#[test]
fn schema_upcloud_server_detail() {
    validate_schema("upcloud_server_detail.json");
}

// ── Vultr ────────────────────────────────────────────────────────────

#[test]
fn schema_vultr_instances() {
    validate_schema("vultr_instances.json");
}

// ── Schema completeness ─────────────────────────────────────────────

/// Verify all expected schema fragment files exist.
#[test]
fn schema_all_fragments_present() {
    let dir = openapi_dir();

    let expected = [
        // Azure
        "azure_token.json",
        "azure_vms.json",
        "azure_nics.json",
        "azure_public_ips.json",
        // DigitalOcean
        "digitalocean_droplets.json",
        // GCP
        "gcp_token.json",
        "gcp_aggregated_instances.json",
        // Hetzner
        "hetzner_servers.json",
        // Leaseweb
        "leaseweb_baremetal.json",
        "leaseweb_cloud.json",
        // Linode
        "linode_instances.json",
        // NetBox
        "netbox_devices.json",
        "netbox_virtual_machines.json",
        // Oracle
        "oracle_compartments.json",
        "oracle_instances.json",
        "oracle_vnic_attachments.json",
        "oracle_vnic.json",
        "oracle_image.json",
        // OVH
        "ovh_instances.json",
        // Scaleway
        "scaleway_servers.json",
        // Tailscale
        "tailscale_api_devices.json",
        // UpCloud
        "upcloud_servers.json",
        "upcloud_server_detail.json",
        // Vultr
        "vultr_instances.json",
    ];

    let mut missing = Vec::new();
    for name in &expected {
        if !dir.join(name).exists() {
            missing.push(*name);
        }
    }
    assert!(
        missing.is_empty(),
        "missing OpenAPI schema fragments: {:?}",
        missing
    );
}
