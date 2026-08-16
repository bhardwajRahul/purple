use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, ProviderState, Screen, StatusCenter, UiSelection};
use crate::handler::ctx::Notify;
use crate::providers::ProviderKind;

type ZoneList = &'static [(&'static str, &'static str)];
type ZoneGroups = &'static [(&'static str, usize, usize)];

/// The slice of App the region picker touches: the provider form (`providers`,
/// for the `regions` string it edits), the picker cursor (`ui`) and the status
/// center (`status`, for the selection-count toast). Everything the handler
/// does is slice-local, so no whole-App effects are deferred; the provider name
/// is resolved from `screen` in the thin wrapper and passed in. The slice never
/// reaches into hosts, vault or any other domain.
struct RegionPickerCtx<'a> {
    providers: &'a mut ProviderState,
    ui: &'a mut UiSelection,
    status: &'a mut StatusCenter,
}

impl Notify for RegionPickerCtx<'_> {
    fn status_mut(&mut self) -> &mut StatusCenter {
        self.status
    }
}

impl RegionPickerCtx<'_> {
    /// Close the region picker overlay. Mirrors `App::close_region_picker` on
    /// the slice (only touches `ui`).
    fn close_region_picker(&mut self) {
        log::debug!("[purple] close_region_picker");
        self.ui.region_picker_mut().open = false;
    }
}

/// Build the same row list used by the region picker renderer.
pub(crate) fn region_picker_rows(provider: &str) -> Vec<Option<&'static str>> {
    let (zones, groups) = zone_data_for(provider);
    let mut rows = Vec::new();
    for &(_, start, end) in groups {
        rows.push(None); // group header
        for &(code, _) in &zones[start..end] {
            rows.push(Some(code));
        }
    }
    rows
}

/// Rebuild the regions string from the selected set, preserving display order.
pub(crate) fn rebuild_regions_string(
    selected: &std::collections::HashSet<String>,
    provider: &str,
) -> String {
    let (zones, _) = zone_data_for(provider);
    let ordered: Vec<&str> = zones
        .iter()
        .filter(|(code, _)| selected.contains(*code))
        .map(|(code, _)| *code)
        .collect();
    ordered.join(",")
}

/// Return the zone/region data for a provider. Empty pair when the
/// provider has no region picker configured. Adding a new `ProviderKind`
/// variant forces a compile error here so the dispatch stays exhaustive.
pub(crate) fn zone_data_for(provider: &str) -> (ZoneList, ZoneGroups) {
    let Ok(kind) = provider.parse::<ProviderKind>() else {
        return (&[], &[]);
    };
    match kind {
        ProviderKind::Scaleway => (
            crate::providers::scaleway::SCW_ZONES,
            crate::providers::scaleway::SCW_ZONE_GROUPS,
        ),
        ProviderKind::Aws => (
            crate::providers::aws::AWS_REGIONS,
            crate::providers::aws::AWS_REGION_GROUPS,
        ),
        ProviderKind::Gcp => (
            crate::providers::gcp::GCP_ZONES,
            crate::providers::gcp::GCP_ZONE_GROUPS,
        ),
        ProviderKind::Oracle => (
            crate::providers::oracle::OCI_REGIONS,
            crate::providers::oracle::OCI_REGION_GROUPS,
        ),
        ProviderKind::Ovh => (
            crate::providers::ovh::OVH_ENDPOINTS,
            crate::providers::ovh::OVH_ENDPOINT_GROUPS,
        ),
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
        | ProviderKind::Vultr => (&[], &[]),
    }
}

pub(crate) fn handle_region_picker(app: &mut App, key: KeyEvent) {
    // Resolve the provider name from the screen in the wrapper while it still
    // holds `&App`, then narrow to the region-picker slice.
    let provider_name = match &app.screen {
        Screen::ProviderForm { id } => id.provider.clone(),
        _ => return,
    };
    let mut ctx = RegionPickerCtx {
        providers: &mut app.providers,
        ui: &mut app.ui,
        status: &mut app.status_center,
    };
    region_picker_key(&mut ctx, key, &provider_name);
}

fn region_picker_key(ctx: &mut RegionPickerCtx, key: KeyEvent, provider_name: &str) {
    let kind = provider_name.parse::<ProviderKind>().ok();
    let rows = region_picker_rows(provider_name);
    let total = rows.len();

    // Parse current regions into a set for toggling
    let mut selected: std::collections::HashSet<String> = ctx
        .providers
        .form()
        .regions
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let zone_label = if matches!(kind, Some(ProviderKind::Scaleway) | Some(ProviderKind::Gcp)) {
        "zone"
    } else if kind == Some(ProviderKind::Ovh) {
        "endpoint"
    } else {
        "region"
    };

    match key.code {
        KeyCode::Esc => {
            ctx.providers.form_mut().regions = rebuild_regions_string(&selected, provider_name);
            ctx.providers.form_mut().sync_cursor_to_end();
            ctx.close_region_picker();
            let count = selected.len();
            if count > 0 {
                ctx.notify(crate::messages::regions_selected_count(count, zone_label));
            }
        }
        KeyCode::Enter => {
            // For single-select providers (OVH): Enter on an item selects it
            // exclusively and closes. For multi-select: Enter confirms current
            // selection (same as Esc).
            if kind == Some(ProviderKind::Ovh) {
                let cursor = ctx.ui.region_picker().cursor;
                if let Some(Some(code)) = rows.get(cursor) {
                    selected.clear();
                    selected.insert(code.to_string());
                }
            }
            ctx.providers.form_mut().regions = rebuild_regions_string(&selected, provider_name);
            ctx.providers.form_mut().sync_cursor_to_end();
            ctx.close_region_picker();
            let count = selected.len();
            if count > 0 {
                ctx.notify(crate::messages::regions_selected_count(count, zone_label));
            }
        }
        KeyCode::Down | KeyCode::Char('j') if ctx.ui.region_picker().cursor + 1 < total => {
            ctx.ui.region_picker_mut().cursor += 1;
        }
        KeyCode::Up | KeyCode::Char('k') if ctx.ui.region_picker().cursor > 0 => {
            ctx.ui.region_picker_mut().cursor -= 1;
        }
        KeyCode::Char(' ') => {
            let cursor = ctx.ui.region_picker().cursor;
            if let Some(Some(code)) = rows.get(cursor) {
                // Toggle single region
                if selected.contains(*code) {
                    selected.remove(*code);
                } else {
                    selected.insert(code.to_string());
                }
            } else {
                // Group header: toggle all regions in this group
                let group_codes: Vec<&str> = rows[cursor + 1..]
                    .iter()
                    .take_while(|r| r.is_some())
                    .filter_map(|r| *r)
                    .collect();
                let all_selected = group_codes.iter().all(|c| selected.contains(*c));
                for code in group_codes {
                    if all_selected {
                        selected.remove(code);
                    } else {
                        selected.insert(code.to_string());
                    }
                }
            }
            ctx.providers.form_mut().regions = rebuild_regions_string(&selected, provider_name);
        }
        _ => {}
    }
}
