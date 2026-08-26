//! Provider configuration, label migration, validation messages and
//! region picker copy. Vault-related provider validation (role format)
//! lives here because it is surfaced by the provider form, not the
//! vault signing flow.

pub fn provider_removed(display_name: &str) -> String {
    format!(
        "Removed {} configuration. Synced hosts remain in your SSH config.",
        display_name
    )
}

pub fn label_invalid(reason: &str) -> String {
    format!("Invalid name: {}", reason)
}

pub const LABEL_MUST_DIFFER: &str = "The two names must be different.";

pub fn label_already_in_use(label: &str) -> String {
    format!(
        "A config named '{}' already exists for this provider.",
        label
    )
}

pub const LABEL_MIGRATION_FIELD_CURRENT: &str = " Name for your current config ";
pub const LABEL_MIGRATION_FIELD_NEW: &str = " Name for the new config ";

pub const EXPAND_TO_REMOVE_CONFIG: &str =
    "Expand the provider and pick a specific config to remove.";

pub fn provider_not_configured(display_name: &str) -> String {
    format!("{} is not configured. Nothing to remove.", display_name)
}

pub fn provider_configure_first(display_name: &str) -> String {
    format!("Configure {} first. Press Enter to set up.", display_name)
}

pub fn provider_saved_syncing(display_name: &str) -> String {
    format!("Saved {} configuration. Syncing...", display_name)
}

pub fn provider_saved(display_name: &str) -> String {
    format!("Saved {} configuration.", display_name)
}

pub fn no_stale_hosts_for(display_name: &str) -> String {
    format!("No stale hosts for {}.", display_name)
}

pub fn contains_control_chars(name: &str) -> String {
    format!("{} contains control characters.", name)
}

pub const TOKEN_FORMAT_AWS: &str = "Format: AccessKeyId:Secret[:SessionToken]";
/// Sync-time failure for an AWS config that carries no token and no profile
/// while the environment holds no credentials either.
pub const AWS_NO_CREDENTIALS: &str = "No AWS credentials. Set a token or a profile on the provider. Otherwise export AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.";

/// A profile is set, so it takes priority over the token and the environment.
/// These two say why it did not work, instead of blaming the API token.
pub fn aws_credentials_file_unreadable(path: &str) -> String {
    format!(
        "Can't read {}. The provider is set to use a profile, so nothing else is tried.",
        path
    )
}

pub fn aws_profile_not_found(profile: &str, path: &str) -> String {
    format!(
        "Profile '{}' is not in {}. Only that file is read, never ~/.aws/config, so a profile declared there stays invisible. A profile also takes priority, so the token and the environment are not tried.",
        profile, path
    )
}

/// The profile block exists but holds no key pair.
pub fn aws_profile_without_keys(profile: &str, path: &str) -> String {
    format!(
        "Profile '{}' in {} has no key pair. Write aws_access_key_id and aws_secret_access_key into it or clear the profile and let purple read your environment.",
        profile, path
    )
}
pub const URL_REQUIRED_PROXMOX: &str = "URL is required for Proxmox VE.";
pub const URL_REQUIRED_NETBOX: &str = "URL is required for NetBox.";
pub const PROJECT_REQUIRED_GCP: &str = "Project ID can't be empty. Set your GCP project ID.";
pub const COMPARTMENT_REQUIRED_OCI: &str =
    "Compartment can't be empty. Set your OCI compartment OCID.";
pub const REGIONS_REQUIRED_AWS: &str = "Select at least one AWS region.";
pub const ZONES_REQUIRED_SCALEWAY: &str = "Select at least one Scaleway zone.";
pub const SUBSCRIPTIONS_REQUIRED_AZURE: &str = "Enter at least one Azure subscription ID.";
pub const ALIAS_PREFIX_INVALID: &str =
    "Alias prefix can't contain spaces or pattern characters (*, ?, [, !).";
pub const USER_NO_WHITESPACE: &str = "User can't contain whitespace.";
pub const VAULT_ROLE_FORMAT: &str = "Vault SSH role must be in the form <mount>/sign/<role>.";

pub const PROVIDER_CONFIG_CHANGED_EXTERNALLY: &str =
    "Provider config changed externally. Press Esc and re-open to pick up changes.";
pub const PROVIDER_URL_REQUIRES_HTTPS: &str =
    "URL must start with https://. Toggle Verify TLS off for self-signed certificates.";
pub const PROVIDER_TOKEN_REQUIRED_GCP: &str =
    "Token can't be empty. Provide a service account JSON key file path or access token.";
pub const PROVIDER_TOKEN_REQUIRED_ORACLE: &str =
    "Token can't be empty. Provide the path to your OCI config file (e.g. ~/.oci/config).";

pub fn provider_token_required(display_name: &str) -> String {
    format!(
        "Token can't be empty. Grab one from your {} dashboard.",
        display_name
    )
}

pub fn azure_subscription_id_invalid(sub: &str) -> String {
    format!(
        "Invalid subscription ID '{}'. Expected UUID format \
         (e.g. 12345678-1234-1234-1234-123456789012).",
        sub
    )
}

// ── Region picker ───────────────────────────────────────────────────

pub fn regions_selected_count(count: usize, label: &str) -> String {
    let s = if count == 1 { "" } else { "s" };
    format!("{} {}{} selected.", count, label, s)
}
