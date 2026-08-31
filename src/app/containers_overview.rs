//! State for the global Containers tab (top_page = Containers).

use std::collections::{HashMap, HashSet, VecDeque};

use super::host_state::ViewMode;
use crate::containers::{ContainerInspect, ContainerRuntime};

/// One queued host in a `R` batch refresh: everything the listing
/// thread needs to spawn an SSH `docker ps` for that alias.
#[derive(Debug, Clone)]
pub struct RefreshQueueItem {
    pub alias: String,
    pub askpass: Option<String>,
    pub cached_runtime: Option<ContainerRuntime>,
    pub has_tunnel: bool,
}

/// State of a `R` batch refresh. None when no batch is active.
/// Drives a windowed concurrency: at most `MAX_PARALLEL` listings are
/// in flight at any time. Each `ContainerListing` event decrements
/// `in_flight` and pops the next queued item; the batch ends when
/// `queue` is empty and `in_flight` drops to zero.
#[derive(Debug, Default)]
pub struct RefreshBatch {
    pub queue: VecDeque<RefreshQueueItem>,
    pub in_flight: usize,
    /// Total host count when the batch started. used for the
    /// `Refreshing N/M` progress readout in the status footer.
    pub total: usize,
    /// Hosts whose listing has already returned (success or error).
    pub completed: usize,
    /// Aliases that the batch has spawned but not yet seen complete.
    /// Listings whose alias is NOT in this set are non-batch traffic
    /// (host-list `C`, action-complete refresh, `a`-add) and must not
    /// touch the counters. Without this guard the in-flight counter
    /// gets corrupted whenever a parallel non-batch fetch completes
    /// during a `R` run.
    pub in_flight_aliases: HashSet<String>,
}

/// Cap on parallel SSH connections during a `R` batch refresh. Picked
/// to keep load on the local SSH agent and remote sshd reasonable
/// while still amortising connection setup.
pub const REFRESH_MAX_PARALLEL: usize = 4;

/// Pending request to drop the user into a remote container shell. Set
/// by the handler when Enter is pressed on a running container; drained
/// by `handle_pending_container_exec` in the main loop, which suspends
/// the TUI, runs `ssh -t <alias> <runtime> exec -it <id> sh -c
/// 'bash || sh'`, then restores the TUI on exit.
///
/// `command` is the full remote command. `None` runs the default
/// shell (`sh -c 'bash || sh'`); `Some` runs the user-typed exec
/// prompt verbatim (validated upstream. must not contain newlines).
#[derive(Debug, Clone)]
pub struct ContainerExecRequest {
    pub alias: String,
    pub askpass: Option<String>,
    pub runtime: ContainerRuntime,
    pub container_id: String,
    /// Human-readable container name (for the log line and the toast
    /// the user sees when the session ends). Not used to build the
    /// command. the validated `container_id` is what addresses the
    /// container.
    pub container_name: String,
    /// User-supplied command. `None` falls back to the interactive
    /// shell (`sh -c 'bash || sh'`) used by the default Enter binding.
    /// `Some` is the verbatim payload from the `e` exec prompt.
    pub command: Option<String>,
}

/// Pending request to fetch container logs from a remote host. Drained
/// by the main loop into a background SSH thread; the result returns
/// via `AppEvent::ContainerLogsComplete` and lands on the
/// `Screen::ContainerLogs` overlay the user opened with `l`.
#[derive(Debug, Clone)]
pub struct ContainerLogsRequest {
    pub alias: String,
    pub askpass: Option<String>,
    pub runtime: ContainerRuntime,
    pub container_id: String,
    pub container_name: String,
}

/// Pending non-interactive container action (restart, stop). Same shape
/// as `ContainerExecRequest` but plus an action tag and minus the askpass
/// handling for an interactive shell. these run in a worker thread, not
/// the foreground TUI. Reuses `containers::ContainerAction` so the
/// command formatter (`container_action_command`) is shared.
#[derive(Debug, Clone)]
pub struct ContainerActionRequest {
    pub alias: String,
    pub askpass: Option<String>,
    pub runtime: ContainerRuntime,
    pub container_id: String,
    pub container_name: String,
    pub action: crate::containers::ContainerAction,
}

/// Sort order for the containers overview screen. Cycled with `s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContainersSortMode {
    /// Alphabetical by host alias, then container name.
    #[default]
    AlphaHost,
    /// Alphabetical by container name, then host alias.
    AlphaContainer,
}

impl ContainersSortMode {
    pub fn next(self) -> Self {
        match self {
            ContainersSortMode::AlphaHost => ContainersSortMode::AlphaContainer,
            ContainersSortMode::AlphaContainer => ContainersSortMode::AlphaHost,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContainersSortMode::AlphaHost => "A-Z host",
            ContainersSortMode::AlphaContainer => "A-Z container",
        }
    }

    pub fn to_key(self) -> &'static str {
        match self {
            ContainersSortMode::AlphaHost => "alpha_host",
            ContainersSortMode::AlphaContainer => "alpha_container",
        }
    }

    pub fn from_key(s: &str) -> Self {
        match s {
            "alpha_container" => ContainersSortMode::AlphaContainer,
            _ => ContainersSortMode::AlphaHost,
        }
    }
}

/// One cached `docker inspect` result, paired with the wall-clock seconds
/// at which it was fetched. The detail panel treats entries older than
/// `INSPECT_CACHE_TTL_SECS` as stale and re-fires the SSH call.
#[derive(Debug, Clone)]
pub struct InspectCacheEntry {
    pub timestamp: u64,
    pub result: Result<ContainerInspect, String>,
}

/// Detail-panel inspect cache and in-flight tracking. Keyed on the full
/// container ID rather than `(alias, container_id)` because `docker`
/// container IDs are globally unique 64-char hex strings. collisions
/// across hosts are practically impossible. Keeps the key shape simple.
#[derive(Debug, Default)]
pub struct InspectCache {
    pub entries: HashMap<String, InspectCacheEntry>,
    /// Container IDs with a pending background `inspect` thread. Prevents
    /// re-firing while a previous fetch is still in flight.
    pub in_flight: HashSet<String>,
}

/// One cached `docker logs --tail N` result. Same TTL semantics as
/// `InspectCacheEntry`: the LOGS card on the detail panel re-fires the
/// SSH call once the entry is older than `LOGS_CACHE_TTL_SECS`.
#[derive(Debug, Clone)]
pub struct LogsCacheEntry {
    pub timestamp: u64,
    pub result: Result<Vec<String>, String>,
}

/// Detail-panel logs cache and in-flight tracking. Mirrors `InspectCache`
/// so the LOGS card and the inspect cards refresh on the same rhythm.
#[derive(Debug, Default)]
pub struct LogsCache {
    pub entries: HashMap<String, LogsCacheEntry>,
    pub in_flight: HashSet<String>,
}

/// Cache TTL in seconds. Kept short so resource-y fields like
/// `RestartCount` and `Health.Status` do not lag too far behind reality
/// while still avoiding an SSH storm when the user scrolls a long list.
pub const INSPECT_CACHE_TTL_SECS: u64 = 30;

/// TTL for the per-container `docker logs --tail` cache feeding the
/// LOGS card. Same value as the inspect cache so a single user-driven
/// refresh re-fires both streams in lockstep.
pub const LOGS_CACHE_TTL_SECS: u64 = 30;

/// How many log lines the LOGS card requests via `--tail`. Sized for
/// the worst-case panel height we expect (a tall terminal can fit
/// dozens of lines inside the card); the renderer slices the trailing
/// `inner_capacity` rows so a short panel only paints what fits.
pub const LOGS_TAIL: usize = 50;

/// TTL for the per-host `docker ps` cache used by the auto-list-refresh
/// helper. When the user scrolls to a row whose host has a stale (or
/// missing) entry in `container_cache`, we re-fire the listing so the
/// running/exited counts and uptime in the visible row reflect reality.
/// Same value as the inspect TTL so the two refresh streams stay
/// loosely in lockstep.
pub const LIST_CACHE_TTL_SECS: u64 = 30;

/// Which bulk-action confirm dialog is open. Mirrored by the
/// corresponding `Screen::Confirm*` variant (now data-less) so the
/// renderer knows which header to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkConfirmKind {
    /// `Ctrl-K` on a container row: restart every running member of
    /// the selected container's compose stack.
    StackRestart,
    /// `K` on a host-divider row: restart every running container on
    /// that host, ignoring compose-project boundaries.
    HostRestartAll,
    /// `S` on a host-divider row: stop every running container on
    /// that host.
    HostStopAll,
}

/// Payload of the three bulk container confirm dialogs. The Screen
/// variant carries only the kind tag; the alias/project/members live
/// here so screen transitions never clone the member vec.
#[derive(Debug, Clone)]
pub struct BulkConfirmContext {
    pub kind: BulkConfirmKind,
    pub alias: String,
    /// Compose project name. Always `Some` for `StackRestart`, `None`
    /// for the host-wide variants.
    pub project: Option<String>,
    pub members: Vec<crate::app::StackMember>,
}

#[derive(Debug)]
pub struct ContainersOverviewState {
    pub(in crate::app) sort_mode: ContainersSortMode,
    pub(in crate::app) inspect_cache: InspectCache,
    pub(in crate::app) logs_cache: LogsCache,
    /// Currently-running `R` batch, if any. `None` when idle.
    pub(in crate::app) refresh_batch: Option<RefreshBatch>,
    /// Aliases whose `docker ps` listing was kicked off by the
    /// scroll-driven auto-refresh helper and has not yet returned.
    /// Lets the helper skip a re-spawn while one is already pending.
    /// Cleared by `handle_container_listing` on arrival.
    pub(in crate::app) auto_list_in_flight: HashSet<String>,
    /// Toggle for the per-row detail panel on the right. Mirrors the
    /// host-list `v` toggle. Default `Detailed` so the panel is visible
    /// whenever the terminal is wide enough.
    pub(in crate::app) view_mode: ViewMode,
    /// Aliases whose container group is currently collapsed in the
    /// AlphaHost rendering. Persisted across sessions via preferences
    /// so a folded group stays folded after restart.
    pub(in crate::app) collapsed_hosts: HashSet<String>,
    /// Memoized render list. The render and handler paths call
    /// `visible_items` repeatedly (24 call sites, several per key
    /// event) and each call cloned 6 String fields per container. The
    /// cache stores the built `Vec<ContainerListItem>` keyed on a
    /// content fingerprint over the inputs (sort_mode, search query,
    /// collapsed_hosts, per-host (timestamp, container_count)). On a
    /// hit we skip the collect/sort/intersperse step entirely and
    /// return a clone of the cached vec. The fingerprint walks all
    /// hosts but only reads a few fields each, so it is dramatically
    /// cheaper than rebuilding the row set.
    pub(in crate::app) view_cache:
        std::cell::RefCell<Option<(u64, Vec<crate::ui::containers_overview::ContainerListItem>)>>,
    /// Payload of the currently-open bulk container confirm dialog
    /// (`ConfirmStackRestart` / `ConfirmHostRestartAll` /
    /// `ConfirmHostStopAll`). `None` when no such dialog is open.
    pub(in crate::app) pending_bulk_confirm: Option<BulkConfirmContext>,
}

impl Default for ContainersOverviewState {
    fn default() -> Self {
        Self {
            sort_mode: ContainersSortMode::default(),
            inspect_cache: InspectCache::default(),
            logs_cache: LogsCache::default(),
            refresh_batch: None,
            auto_list_in_flight: HashSet::new(),
            view_mode: ViewMode::Detailed,
            collapsed_hosts: HashSet::new(),
            view_cache: std::cell::RefCell::new(None),
            pending_bulk_confirm: None,
        }
    }
}

impl ContainersOverviewState {
    /// Install a fresh refresh batch. Caller is responsible for spawning
    /// the initial parallel listings after this call; this method only
    /// owns the state slot.
    pub fn start_refresh(&mut self, batch: RefreshBatch) {
        self.refresh_batch = Some(batch);
    }

    /// Drop the active refresh batch. Called when the queue drains and
    /// in-flight count returns to zero.
    pub fn clear_refresh(&mut self) {
        self.refresh_batch = None;
    }

    // Sealed-field accessors. Fields are `pub(in crate::app)`; callers
    // outside the app module reach state through these.

    pub fn sort_mode(&self) -> ContainersSortMode {
        self.sort_mode
    }

    /// Set the sort mode without persisting to preferences. Demo mode and
    /// tests only; the persisting path is `set_sort_mode`.
    pub fn set_sort_mode_ephemeral(&mut self, mode: ContainersSortMode) {
        self.sort_mode = mode;
    }

    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    /// Set the view mode without persisting to preferences. Demo mode and
    /// tests only; the persisting path is `set_view_mode`.
    pub fn set_view_mode_ephemeral(&mut self, mode: ViewMode) {
        self.view_mode = mode;
    }

    pub fn collapsed_hosts(&self) -> &HashSet<String> {
        &self.collapsed_hosts
    }

    /// Read the active bulk confirm payload (`None` when no bulk
    /// container confirm dialog is open).
    pub fn pending_bulk_confirm(&self) -> Option<&BulkConfirmContext> {
        self.pending_bulk_confirm.as_ref()
    }

    /// Install a fresh bulk confirm payload. Caller is responsible for
    /// transitioning the screen to the matching `Screen::Confirm*`
    /// variant.
    pub fn set_pending_bulk_confirm(&mut self, ctx: BulkConfirmContext) {
        self.pending_bulk_confirm = Some(ctx);
    }

    /// Drop the active bulk confirm payload, returning it for use by
    /// the confirm-yes handler.
    pub fn take_pending_bulk_confirm(&mut self) -> Option<BulkConfirmContext> {
        self.pending_bulk_confirm.take()
    }

    /// Fold or unfold a host group; returns the new collapsed state.
    pub fn toggle_host_collapsed(&mut self, alias: &str) -> bool {
        if self.collapsed_hosts.remove(alias) {
            false
        } else {
            self.collapsed_hosts.insert(alias.to_string());
            true
        }
    }

    pub fn refresh_batch(&self) -> Option<&RefreshBatch> {
        self.refresh_batch.as_ref()
    }

    pub fn refresh_batch_mut(&mut self) -> Option<&mut RefreshBatch> {
        self.refresh_batch.as_mut()
    }

    pub fn auto_list_in_flight(&self) -> &HashSet<String> {
        &self.auto_list_in_flight
    }

    /// True when a scroll-driven auto-listing is already in flight for
    /// `alias`, so the helper skips a re-spawn.
    pub fn auto_list_pending(&self, alias: &str) -> bool {
        self.auto_list_in_flight.contains(alias)
    }

    /// Mark a scroll-driven auto-listing as in flight for `alias`.
    pub fn mark_auto_list_pending(&mut self, alias: String) {
        self.auto_list_in_flight.insert(alias);
    }

    /// Clear the in-flight marker once an auto-listing arrives.
    pub fn clear_auto_list_pending(&mut self, alias: &str) {
        self.auto_list_in_flight.remove(alias);
    }

    pub fn inspect_cache(&self) -> &InspectCache {
        &self.inspect_cache
    }

    pub fn inspect_cache_mut(&mut self) -> &mut InspectCache {
        &mut self.inspect_cache
    }

    pub fn logs_cache(&self) -> &LogsCache {
        &self.logs_cache
    }

    pub fn logs_cache_mut(&mut self) -> &mut LogsCache {
        &mut self.logs_cache
    }

    pub fn view_cache(
        &self,
    ) -> &std::cell::RefCell<Option<(u64, Vec<crate::ui::containers_overview::ContainerListItem>)>>
    {
        &self.view_cache
    }

    /// Load the persisted overview fields from `preferences`. Startup only:
    /// clobbers any in-memory state with whatever is on disk. `pub(crate)`
    /// so a stray mid-session caller cannot quietly revert user edits.
    pub(crate) fn hydrate_from_prefs(&mut self, paths: Option<&crate::runtime::env::Paths>) {
        self.view_mode = crate::preferences::load_containers_view_mode(paths);
        self.sort_mode = crate::preferences::load_containers_sort_mode(paths);
        self.collapsed_hosts = crate::preferences::load_containers_collapsed_hosts(paths);
    }

    /// Update `view_mode` and persist. Returns the persist error so the
    /// caller can surface it (current call site discards it intentionally
    /// to match the pre-encapsulation behavior where view-mode persist
    /// failures only logged).
    pub fn set_view_mode(
        &mut self,
        paths: Option<&crate::runtime::env::Paths>,
        mode: ViewMode,
    ) -> std::io::Result<()> {
        self.view_mode = mode;
        crate::preferences::save_containers_view_mode(paths, mode).inspect_err(|e| {
            log::warn!("[config] Failed to persist containers view mode: {e}");
        })
    }

    /// Update `sort_mode` and persist. Same contract as `set_view_mode`;
    /// the call site does surface the error via a toast.
    pub fn set_sort_mode(
        &mut self,
        paths: Option<&crate::runtime::env::Paths>,
        mode: ContainersSortMode,
    ) -> std::io::Result<()> {
        self.sort_mode = mode;
        crate::preferences::save_containers_sort_mode(paths, mode).inspect_err(|e| {
            log::warn!("[config] Failed to persist containers sort mode: {e}");
        })
    }

    /// Rename an alias across every alias-keyed set in this state.
    /// Returns `true` when `collapsed_hosts` changed so the caller can
    /// persist; `auto_list_in_flight` and `refresh_batch.in_flight_aliases`
    /// are also migrated but are not persistent so they do not affect
    /// the return value. No-op (returns `false`) when `old == new`.
    pub fn migrate_alias(&mut self, old: &str, new: &str) -> bool {
        if old == new {
            return false;
        }
        if self.auto_list_in_flight.remove(old) {
            debug_assert!(
                !self.auto_list_in_flight.contains(new),
                "auto_list_in_flight collision on rename {old} -> {new}"
            );
            self.auto_list_in_flight.insert(new.to_string());
        }
        if let Some(batch) = self.refresh_batch.as_mut()
            && batch.in_flight_aliases.remove(old)
        {
            debug_assert!(
                !batch.in_flight_aliases.contains(new),
                "refresh_batch.in_flight_aliases collision on rename {old} -> {new}"
            );
            batch.in_flight_aliases.insert(new.to_string());
        }
        if let Some(ctx) = self.pending_bulk_confirm.as_mut()
            && ctx.alias == old
        {
            ctx.alias = new.to_string();
        }
        if self.collapsed_hosts.remove(old) {
            debug_assert!(
                !self.collapsed_hosts.contains(new),
                "collapsed_hosts collision on rename {old} -> {new}"
            );
            self.collapsed_hosts.insert(new.to_string());
            true
        } else {
            false
        }
    }

    /// Drop inspect-cache and logs-cache entries whose container ID is
    /// not in `valid_container_ids`. Called from `App::reload_hosts`
    /// after the per-host container cache has been pruned: container
    /// IDs come from the just-pruned cache, so this keeps inspect/logs
    /// aligned with the surviving hosts.
    pub fn prune_by_container_ids(&mut self, valid_container_ids: &HashSet<String>) {
        let pre_inspect = self.inspect_cache.entries.len();
        self.inspect_cache
            .entries
            .retain(|id, _| valid_container_ids.contains(id));
        self.inspect_cache
            .in_flight
            .retain(|id| valid_container_ids.contains(id));
        self.logs_cache
            .entries
            .retain(|id, _| valid_container_ids.contains(id));
        self.logs_cache
            .in_flight
            .retain(|id| valid_container_ids.contains(id));
        let dropped = pre_inspect.saturating_sub(self.inspect_cache.entries.len());
        if dropped > 0 {
            log::debug!("[purple] reload_hosts: dropped {dropped} orphan inspect_cache entrie(s)");
        }
    }

    /// Drop per-alias overview state (auto-list in-flight, refresh batch
    /// in-flight aliases, collapsed-hosts) whose alias is no longer in
    /// `valid_aliases`. Returns `true` when `collapsed_hosts` shrank, so
    /// the caller can persist the trimmed set via
    /// `preferences::save_containers_collapsed_hosts`.
    pub fn prune_orphans(&mut self, valid_aliases: &HashSet<&str>) -> bool {
        // Auto-list in-flight markers for deleted hosts. The listing
        // thread still posts a result and the race guard in
        // `handle_container_listing` removes it; pruning here keeps
        // debug state clean and avoids a false-positive dedup hit if
        // the same alias is re-added before the stray listing returns.
        self.auto_list_in_flight
            .retain(|alias| valid_aliases.contains(alias.as_str()));

        // Container-overview refresh batch (R). Tracks in-flight aliases
        // to gate counter updates against non-batch listings; prune so a
        // host removed mid-batch cannot linger.
        if let Some(batch) = self.refresh_batch.as_mut() {
            let pre = batch.in_flight_aliases.len();
            batch
                .in_flight_aliases
                .retain(|alias| valid_aliases.contains(alias.as_str()));
            let dropped = pre.saturating_sub(batch.in_flight_aliases.len());
            if dropped > 0 {
                log::debug!(
                    "[purple] reload_hosts: dropped {dropped} orphan refresh_batch in_flight alias(es)"
                );
            }
        }

        // Bulk-confirm payload aimed at a deleted host: clear it; the
        // matching Screen::Confirm* variant would render an empty body
        // and a y press would no-op.
        if let Some(ctx) = self.pending_bulk_confirm.as_ref()
            && !valid_aliases.contains(ctx.alias.as_str())
        {
            self.pending_bulk_confirm = None;
        }

        // Containers-overview collapsed groups are persisted to disk via
        // preferences, so leftover aliases survive restart. Rename is
        // already handled by `apply_alias_renames`; this covers delete.
        let pre_collapsed = self.collapsed_hosts.len();
        self.collapsed_hosts
            .retain(|alias| valid_aliases.contains(alias.as_str()));
        let dropped_collapsed = pre_collapsed.saturating_sub(self.collapsed_hosts.len());
        if dropped_collapsed > 0 {
            log::debug!(
                "[purple] reload_hosts: dropped {dropped_collapsed} orphan collapsed_hosts entrie(s)"
            );
            true
        } else {
            false
        }
    }
}

impl InspectCache {
    /// Returns `Some(entry)` only when the cache holds a *fresh* entry
    /// for `container_id` (`now - timestamp < TTL`). Stale entries
    /// behave like absent ones so the trigger code re-fetches.
    pub fn fresh(&self, container_id: &str, now: u64) -> Option<&InspectCacheEntry> {
        self.entries
            .get(container_id)
            .filter(|e| now.saturating_sub(e.timestamp) < INSPECT_CACHE_TTL_SECS)
    }
}

impl LogsCache {
    /// Same fresh-window contract as `InspectCache::fresh`, against
    /// `LOGS_CACHE_TTL_SECS`.
    pub fn fresh(&self, container_id: &str, now: u64) -> Option<&LogsCacheEntry> {
        self.entries
            .get(container_id)
            .filter(|e| now.saturating_sub(e.timestamp) < LOGS_CACHE_TTL_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::env::Paths;

    fn batch_with_aliases(aliases: &[&str]) -> RefreshBatch {
        RefreshBatch {
            queue: VecDeque::new(),
            in_flight: aliases.len(),
            total: aliases.len(),
            completed: 0,
            in_flight_aliases: aliases.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn start_refresh_installs_batch() {
        let mut state = ContainersOverviewState::default();
        assert!(state.refresh_batch.is_none());
        state.start_refresh(batch_with_aliases(&["host-a", "host-b"]));
        let batch = state.refresh_batch.as_ref().unwrap();
        assert_eq!(batch.total, 2);
        assert_eq!(batch.in_flight, 2);
        assert!(batch.in_flight_aliases.contains("host-a"));
    }

    #[test]
    fn clear_refresh_drops_batch() {
        let mut state = ContainersOverviewState::default();
        state.start_refresh(batch_with_aliases(&["host-a"]));
        state.clear_refresh();
        assert!(state.refresh_batch.is_none());
    }

    #[test]
    fn hydrate_from_prefs_reads_persisted_values() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        crate::preferences::save_containers_view_mode(Some(&paths), ViewMode::Compact).unwrap();
        crate::preferences::save_containers_sort_mode(
            Some(&paths),
            ContainersSortMode::AlphaContainer,
        )
        .unwrap();
        let mut collapsed = std::collections::HashSet::new();
        collapsed.insert("folded-host".to_string());
        crate::preferences::save_containers_collapsed_hosts(Some(&paths), &collapsed).unwrap();

        let mut state = ContainersOverviewState::default();
        state.hydrate_from_prefs(Some(&paths));
        assert_eq!(state.view_mode, ViewMode::Compact);
        assert_eq!(state.sort_mode, ContainersSortMode::AlphaContainer);
        assert!(state.collapsed_hosts.contains("folded-host"));
    }

    #[test]
    fn set_view_mode_updates_field_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let mut state = ContainersOverviewState::default();
        state
            .set_view_mode(Some(&paths), ViewMode::Compact)
            .unwrap();
        assert_eq!(state.view_mode, ViewMode::Compact);
        assert_eq!(
            crate::preferences::load_containers_view_mode(Some(&paths)),
            ViewMode::Compact
        );
    }

    #[test]
    fn set_sort_mode_updates_field_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let mut state = ContainersOverviewState::default();
        state
            .set_sort_mode(Some(&paths), ContainersSortMode::AlphaContainer)
            .unwrap();
        assert_eq!(state.sort_mode, ContainersSortMode::AlphaContainer);
        assert_eq!(
            crate::preferences::load_containers_sort_mode(Some(&paths)),
            ContainersSortMode::AlphaContainer
        );
    }

    #[test]
    fn migrate_alias_renames_auto_list_in_flight() {
        let mut state = ContainersOverviewState::default();
        state.auto_list_in_flight.insert("old".to_string());
        state.migrate_alias("old", "new");
        assert!(state.auto_list_in_flight.contains("new"));
        assert!(!state.auto_list_in_flight.contains("old"));
    }

    #[test]
    fn migrate_alias_renames_refresh_batch_in_flight() {
        let mut state = ContainersOverviewState::default();
        state.start_refresh(batch_with_aliases(&["old"]));
        // Non-persistent change: collapsed_hosts is untouched so the
        // return value must be false even though the in-flight set
        // was migrated. Pins the contract that drives the persist
        // call in app::hosts::migrate_alias_keyed_caches.
        assert!(!state.migrate_alias("old", "new"));
        let batch = state.refresh_batch.as_ref().unwrap();
        assert!(batch.in_flight_aliases.contains("new"));
        assert!(!batch.in_flight_aliases.contains("old"));
    }

    #[test]
    fn migrate_alias_self_rename_is_noop() {
        let mut state = ContainersOverviewState::default();
        state.collapsed_hosts.insert("same".to_string());
        state.auto_list_in_flight.insert("same".to_string());
        assert!(!state.migrate_alias("same", "same"));
        assert!(state.collapsed_hosts.contains("same"));
        assert!(state.auto_list_in_flight.contains("same"));
    }

    #[test]
    fn migrate_alias_renames_collapsed_hosts_and_returns_true() {
        let mut state = ContainersOverviewState::default();
        state.collapsed_hosts.insert("old".to_string());
        assert!(state.migrate_alias("old", "new"));
        assert!(state.collapsed_hosts.contains("new"));
        assert!(!state.collapsed_hosts.contains("old"));
    }

    #[test]
    fn migrate_alias_returns_false_when_collapsed_unchanged() {
        let mut state = ContainersOverviewState::default();
        state.auto_list_in_flight.insert("old".to_string());
        assert!(!state.migrate_alias("old", "new"));
        assert!(state.auto_list_in_flight.contains("new"));
    }

    #[test]
    fn migrate_alias_is_noop_when_nothing_matches() {
        let mut state = ContainersOverviewState::default();
        assert!(!state.migrate_alias("missing", "new"));
    }

    #[test]
    fn prune_by_container_ids_drops_unknown_id_from_in_flight_sets() {
        let mut state = ContainersOverviewState::default();
        state.inspect_cache.in_flight.insert("id-keep".to_string());
        state.inspect_cache.in_flight.insert("id-drop".to_string());
        state.logs_cache.in_flight.insert("id-keep".to_string());
        state.logs_cache.in_flight.insert("id-drop".to_string());

        let valid: HashSet<String> = ["id-keep".to_string()].into_iter().collect();
        state.prune_by_container_ids(&valid);

        assert!(state.inspect_cache.in_flight.contains("id-keep"));
        assert!(!state.inspect_cache.in_flight.contains("id-drop"));
        assert!(state.logs_cache.in_flight.contains("id-keep"));
        assert!(!state.logs_cache.in_flight.contains("id-drop"));
    }

    #[test]
    fn prune_by_container_ids_drops_unknown_id_from_entries_maps() {
        // Distinct from the in_flight test: this one populates the
        // `.entries` HashMaps so the retain() calls covering them
        // cannot be removed without a test failure.
        let mut state = ContainersOverviewState::default();
        state.inspect_cache.entries.insert(
            "id-keep".to_string(),
            InspectCacheEntry {
                timestamp: 0,
                result: Err("placeholder".to_string()),
            },
        );
        state.inspect_cache.entries.insert(
            "id-drop".to_string(),
            InspectCacheEntry {
                timestamp: 0,
                result: Err("placeholder".to_string()),
            },
        );
        state.logs_cache.entries.insert(
            "id-keep".to_string(),
            LogsCacheEntry {
                timestamp: 0,
                result: Ok(vec!["line".to_string()]),
            },
        );
        state.logs_cache.entries.insert(
            "id-drop".to_string(),
            LogsCacheEntry {
                timestamp: 0,
                result: Ok(vec!["line".to_string()]),
            },
        );

        let valid: HashSet<String> = ["id-keep".to_string()].into_iter().collect();
        state.prune_by_container_ids(&valid);

        assert!(state.inspect_cache.entries.contains_key("id-keep"));
        assert!(!state.inspect_cache.entries.contains_key("id-drop"));
        assert!(state.logs_cache.entries.contains_key("id-keep"));
        assert!(!state.logs_cache.entries.contains_key("id-drop"));
    }

    #[test]
    fn prune_orphans_drops_unknown_and_signals_collapsed_change() {
        let mut state = ContainersOverviewState::default();
        state.auto_list_in_flight.insert("keep".to_string());
        state.auto_list_in_flight.insert("drop".to_string());
        state.collapsed_hosts.insert("keep".to_string());
        state.collapsed_hosts.insert("drop".to_string());

        let valid: HashSet<&str> = ["keep"].into_iter().collect();
        let collapsed_changed = state.prune_orphans(&valid);

        assert!(
            collapsed_changed,
            "returns true so caller persists collapsed_hosts"
        );
        assert!(state.auto_list_in_flight.contains("keep"));
        assert!(!state.auto_list_in_flight.contains("drop"));
        assert!(state.collapsed_hosts.contains("keep"));
        assert!(!state.collapsed_hosts.contains("drop"));
    }

    #[test]
    fn prune_orphans_returns_false_when_collapsed_unchanged() {
        let mut state = ContainersOverviewState::default();
        state.auto_list_in_flight.insert("only".to_string());
        let valid: HashSet<&str> = ["only"].into_iter().collect();
        assert!(!state.prune_orphans(&valid));
    }
}
