//! Typed provider kind. One variant per supported provider.
//!
//! Used everywhere outside the three string boundaries (TOML on disk,
//! SSH config write path, provider API JSON). `as_str` and `from_str`
//! bridge those boundaries; everything in-process compares variants
//! directly so dispatch is compiler-checked exhaustive.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Aws,
    Azure,
    DigitalOcean,
    Gcp,
    Hetzner,
    I3d,
    Leaseweb,
    Linode,
    Oracle,
    Ovh,
    Proxmox,
    Scaleway,
    Tailscale,
    Teleport,
    Transip,
    UpCloud,
    Vultr,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Aws => "aws",
            ProviderKind::Azure => "azure",
            ProviderKind::DigitalOcean => "digitalocean",
            ProviderKind::Gcp => "gcp",
            ProviderKind::Hetzner => "hetzner",
            ProviderKind::I3d => "i3d",
            ProviderKind::Leaseweb => "leaseweb",
            ProviderKind::Linode => "linode",
            ProviderKind::Oracle => "oracle",
            ProviderKind::Ovh => "ovh",
            ProviderKind::Proxmox => "proxmox",
            ProviderKind::Scaleway => "scaleway",
            ProviderKind::Tailscale => "tailscale",
            ProviderKind::Teleport => "teleport",
            ProviderKind::Transip => "transip",
            ProviderKind::UpCloud => "upcloud",
            ProviderKind::Vultr => "vultr",
        }
    }

    /// Default `auto_sync` value for a new section of this provider.
    /// Proxmox opts out by default because its API is N+1 per VM.
    pub fn default_auto_sync(self) -> bool {
        match self {
            ProviderKind::Proxmox => false,
            ProviderKind::Aws
            | ProviderKind::Azure
            | ProviderKind::DigitalOcean
            | ProviderKind::Gcp
            | ProviderKind::Hetzner
            | ProviderKind::I3d
            | ProviderKind::Leaseweb
            | ProviderKind::Linode
            | ProviderKind::Oracle
            | ProviderKind::Ovh
            | ProviderKind::Scaleway
            | ProviderKind::Tailscale
            | ProviderKind::Teleport
            | ProviderKind::Transip
            | ProviderKind::UpCloud
            | ProviderKind::Vultr => true,
        }
    }

    /// Canonical short alias-prefix suggestion shown in the provider form.
    /// Returned value is a project identifier, not localisable copy.
    pub fn alias_prefix(self) -> &'static str {
        match self {
            ProviderKind::Aws => "aws",
            ProviderKind::Azure => "az",
            ProviderKind::DigitalOcean => "do",
            ProviderKind::Gcp => "gcp",
            ProviderKind::Hetzner => "hetzner",
            ProviderKind::I3d => "i3d",
            ProviderKind::Leaseweb => "leaseweb",
            ProviderKind::Linode => "linode",
            ProviderKind::Oracle => "oci",
            ProviderKind::Ovh => "ovh",
            ProviderKind::Proxmox => "pve",
            ProviderKind::Scaleway => "scw",
            ProviderKind::Tailscale => "ts",
            ProviderKind::Teleport => "tp",
            ProviderKind::Transip => "transip",
            ProviderKind::UpCloud => "uc",
            ProviderKind::Vultr => "vultr",
        }
    }

    /// Whether this provider requires a `url` (Proxmox endpoint).
    pub fn requires_url(self) -> bool {
        match self {
            ProviderKind::Proxmox => true,
            ProviderKind::Aws
            | ProviderKind::Azure
            | ProviderKind::DigitalOcean
            | ProviderKind::Gcp
            | ProviderKind::Hetzner
            | ProviderKind::I3d
            | ProviderKind::Leaseweb
            | ProviderKind::Linode
            | ProviderKind::Oracle
            | ProviderKind::Ovh
            | ProviderKind::Scaleway
            | ProviderKind::Tailscale
            | ProviderKind::Teleport
            | ProviderKind::Transip
            | ProviderKind::UpCloud
            | ProviderKind::Vultr => false,
        }
    }

    /// Whether the CLI's `--regions` flag applies. Subset of `has_regions_field`
    /// because not every provider with a regions form field also exposes the CLI flag.
    pub fn accepts_cli_regions(self) -> bool {
        match self {
            ProviderKind::Aws
            | ProviderKind::Azure
            | ProviderKind::Gcp
            | ProviderKind::Oracle
            | ProviderKind::Scaleway => true,
            ProviderKind::DigitalOcean
            | ProviderKind::Hetzner
            | ProviderKind::I3d
            | ProviderKind::Leaseweb
            | ProviderKind::Linode
            | ProviderKind::Ovh
            | ProviderKind::Proxmox
            | ProviderKind::Tailscale
            | ProviderKind::Teleport
            | ProviderKind::Transip
            | ProviderKind::UpCloud
            | ProviderKind::Vultr => false,
        }
    }

    /// Whether the provider form exposes a `regions` field at all.
    pub fn has_regions_field(self) -> bool {
        match self {
            ProviderKind::Aws
            | ProviderKind::Azure
            | ProviderKind::Gcp
            | ProviderKind::Oracle
            | ProviderKind::Ovh
            | ProviderKind::Scaleway => true,
            ProviderKind::DigitalOcean
            | ProviderKind::Hetzner
            | ProviderKind::I3d
            | ProviderKind::Leaseweb
            | ProviderKind::Linode
            | ProviderKind::Proxmox
            | ProviderKind::Tailscale
            | ProviderKind::Teleport
            | ProviderKind::Transip
            | ProviderKind::UpCloud
            | ProviderKind::Vultr => false,
        }
    }

    /// Whether the `regions` field is mandatory for form submission.
    /// GCP and Oracle have meaningful defaults so they are merely optional;
    /// the others either need an explicit list or, for Azure, subscription IDs.
    pub fn regions_field_is_mandatory(self) -> bool {
        match self {
            ProviderKind::Aws
            | ProviderKind::Azure
            | ProviderKind::Ovh
            | ProviderKind::Scaleway => true,
            ProviderKind::DigitalOcean
            | ProviderKind::Gcp
            | ProviderKind::Hetzner
            | ProviderKind::I3d
            | ProviderKind::Leaseweb
            | ProviderKind::Linode
            | ProviderKind::Oracle
            | ProviderKind::Proxmox
            | ProviderKind::Tailscale
            | ProviderKind::Teleport
            | ProviderKind::Transip
            | ProviderKind::UpCloud
            | ProviderKind::Vultr => false,
        }
    }

    /// Whether activating the `regions` field opens a structured picker
    /// rather than accepting free-form text. Azure takes subscription IDs
    /// as free-form CSV input.
    pub fn regions_field_is_picker(self) -> bool {
        match self {
            ProviderKind::Aws
            | ProviderKind::Gcp
            | ProviderKind::Oracle
            | ProviderKind::Ovh
            | ProviderKind::Scaleway => true,
            ProviderKind::Azure
            | ProviderKind::DigitalOcean
            | ProviderKind::Hetzner
            | ProviderKind::I3d
            | ProviderKind::Leaseweb
            | ProviderKind::Linode
            | ProviderKind::Proxmox
            | ProviderKind::Tailscale
            | ProviderKind::Teleport
            | ProviderKind::Transip
            | ProviderKind::UpCloud
            | ProviderKind::Vultr => false,
        }
    }

    /// Whether the provider form exposes a `project` field.
    pub fn has_project_field(self) -> bool {
        match self {
            ProviderKind::Gcp | ProviderKind::Ovh => true,
            ProviderKind::Aws
            | ProviderKind::Azure
            | ProviderKind::DigitalOcean
            | ProviderKind::Hetzner
            | ProviderKind::I3d
            | ProviderKind::Leaseweb
            | ProviderKind::Linode
            | ProviderKind::Oracle
            | ProviderKind::Proxmox
            | ProviderKind::Scaleway
            | ProviderKind::Tailscale
            | ProviderKind::Teleport
            | ProviderKind::Transip
            | ProviderKind::UpCloud
            | ProviderKind::Vultr => false,
        }
    }
}

/// Error returned when a string does not match any known `ProviderKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownProviderKind;

impl fmt::Display for UnknownProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown provider kind")
    }
}

impl std::error::Error for UnknownProviderKind {}

impl FromStr for ProviderKind {
    type Err = UnknownProviderKind;

    fn from_str(s: &str) -> Result<Self, UnknownProviderKind> {
        match s {
            "aws" => Ok(ProviderKind::Aws),
            "azure" => Ok(ProviderKind::Azure),
            "digitalocean" => Ok(ProviderKind::DigitalOcean),
            "gcp" => Ok(ProviderKind::Gcp),
            "hetzner" => Ok(ProviderKind::Hetzner),
            "i3d" => Ok(ProviderKind::I3d),
            "leaseweb" => Ok(ProviderKind::Leaseweb),
            "linode" => Ok(ProviderKind::Linode),
            "oracle" => Ok(ProviderKind::Oracle),
            "ovh" => Ok(ProviderKind::Ovh),
            "proxmox" => Ok(ProviderKind::Proxmox),
            "scaleway" => Ok(ProviderKind::Scaleway),
            "tailscale" => Ok(ProviderKind::Tailscale),
            "teleport" => Ok(ProviderKind::Teleport),
            "transip" => Ok(ProviderKind::Transip),
            "upcloud" => Ok(ProviderKind::UpCloud),
            "vultr" => Ok(ProviderKind::Vultr),
            _ => Err(UnknownProviderKind),
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[(&str, ProviderKind)] = &[
        ("aws", ProviderKind::Aws),
        ("azure", ProviderKind::Azure),
        ("digitalocean", ProviderKind::DigitalOcean),
        ("gcp", ProviderKind::Gcp),
        ("hetzner", ProviderKind::Hetzner),
        ("i3d", ProviderKind::I3d),
        ("leaseweb", ProviderKind::Leaseweb),
        ("linode", ProviderKind::Linode),
        ("oracle", ProviderKind::Oracle),
        ("ovh", ProviderKind::Ovh),
        ("proxmox", ProviderKind::Proxmox),
        ("scaleway", ProviderKind::Scaleway),
        ("tailscale", ProviderKind::Tailscale),
        ("teleport", ProviderKind::Teleport),
        ("transip", ProviderKind::Transip),
        ("upcloud", ProviderKind::UpCloud),
        ("vultr", ProviderKind::Vultr),
    ];

    /// Marketing copy hardcodes the provider count in prose ("17 cloud
    /// providers"). This guard fails when a provider is added to `ALL`
    /// without updating the headline count in `llms.txt` and
    /// `site/page.html`, so the marketing sweep cannot be silently skipped.
    #[test]
    fn marketing_provider_count_matches_all() {
        let phrase = format!("{} cloud provider", ALL.len());
        let llms = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/llms.txt"));
        let page = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/site/page.html"));
        assert!(
            llms.contains(&phrase),
            "llms.txt must mention \"{phrase}\" (ProviderKind has {} variants); update marketing when adding a provider",
            ALL.len()
        );
        assert!(
            page.contains(&phrase),
            "site/page.html must mention \"{phrase}\" (ProviderKind has {} variants); update marketing when adding a provider",
            ALL.len()
        );
    }

    #[test]
    fn round_trip_string_to_kind_to_string() {
        for (name, kind) in ALL {
            assert_eq!(
                name.parse::<ProviderKind>().ok(),
                Some(*kind),
                "parse({name})"
            );
            assert_eq!(kind.as_str(), *name, "as_str for {name}");
        }
    }

    #[test]
    fn unknown_returns_err() {
        assert!("not-a-provider".parse::<ProviderKind>().is_err());
        assert!("".parse::<ProviderKind>().is_err());
        assert!("AWS".parse::<ProviderKind>().is_err(), "case sensitive");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", ProviderKind::Hetzner), "hetzner");
        assert_eq!(format!("{}", ProviderKind::DigitalOcean), "digitalocean");
        assert_eq!(format!("{}", ProviderKind::UpCloud), "upcloud");
    }

    #[test]
    fn requires_url_only_proxmox() {
        for (_, kind) in ALL {
            assert_eq!(
                kind.requires_url(),
                *kind == ProviderKind::Proxmox,
                "requires_url for {kind:?}"
            );
        }
    }

    #[test]
    fn accepts_cli_regions_matches_documented_set() {
        let expected: &[ProviderKind] = &[
            ProviderKind::Aws,
            ProviderKind::Azure,
            ProviderKind::Gcp,
            ProviderKind::Oracle,
            ProviderKind::Scaleway,
        ];
        for (_, kind) in ALL {
            let want = expected.contains(kind);
            assert_eq!(kind.accepts_cli_regions(), want, "regions cli for {kind:?}");
        }
    }

    #[test]
    fn has_regions_field_is_cli_set_plus_ovh() {
        for (_, kind) in ALL {
            let want = kind.accepts_cli_regions() || *kind == ProviderKind::Ovh;
            assert_eq!(kind.has_regions_field(), want, "regions field for {kind:?}");
        }
    }

    #[test]
    fn regions_mandatory_implies_has_field() {
        for (_, kind) in ALL {
            if kind.regions_field_is_mandatory() {
                assert!(
                    kind.has_regions_field(),
                    "{kind:?} mandates regions but has no field"
                );
            }
        }
    }

    #[test]
    fn regions_picker_implies_has_field_excluding_azure() {
        for (_, kind) in ALL {
            if kind.regions_field_is_picker() {
                assert!(kind.has_regions_field(), "{kind:?} picker without field");
                assert_ne!(*kind, ProviderKind::Azure, "azure regions are free-form");
            }
        }
    }

    #[test]
    fn project_field_set() {
        for (_, kind) in ALL {
            let want = matches!(kind, ProviderKind::Gcp | ProviderKind::Ovh);
            assert_eq!(kind.has_project_field(), want, "project field for {kind:?}");
        }
    }
}
