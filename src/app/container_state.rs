//! Containers overlay state.

use crate::app::{ContainerActionRequest, ContainerExecRequest, ContainerLogsRequest};

/// Per-host overlay session state; only valid while the containers overlay is open.
///
/// No `Default` impl: construction always requires an alias and runtime
/// metadata, so a default-constructed value would be meaningless.
pub struct ContainerSession {
    pub alias: String,
    pub askpass: Option<String>,
    pub runtime: Option<crate::containers::ContainerRuntime>,
    pub containers: Vec<crate::containers::ContainerInfo>,
    pub list_state: ratatui::widgets::ListState,
    pub loading: bool,
    pub error: Option<String>,
    pub action_in_progress: Option<String>,
    /// Pending confirmation for stop/restart actions: (action, container_name, container_id).
    pub confirm_action: Option<(crate::containers::ContainerAction, String, String)>,
}

/// Open container logs viewer. The `Screen::ContainerLogs` variant is
/// data-less; the alias/container identity and the streaming body live
/// here so screen transitions never clone the body Vec.
#[derive(Debug, Default)]
pub struct LogsView {
    pub alias: String,
    pub container_id: String,
    pub container_name: String,
    /// Rendered lines fetched via SSH `docker logs --tail`. Empty while
    /// the request is in flight, populated once the result lands.
    pub body: Vec<String>,
    pub fetched_at: u64,
    pub error: Option<String>,
    pub scroll: u16,
    /// Written by the renderer each frame so `G` and the result-arrival
    /// path can compute the tail-anchored scroll without guessing the
    /// visible-area size.
    pub last_render_height: u16,
    /// `/` search state. `None` when no search is active.
    pub search: Option<crate::app::ContainerLogsSearch>,
}

/// Always-present container-domain state: cache and cross-host pending operations.
/// Separate from `ContainerSession`, which is the per-host overlay session state.
#[derive(Debug, Default)]
pub struct ContainerState {
    pub(in crate::app) pending_exec: Option<ContainerExecRequest>,
    pub(in crate::app) pending_logs: Option<ContainerLogsRequest>,
    pub(in crate::app) pending_actions: std::collections::VecDeque<ContainerActionRequest>,
    pub(in crate::app) pending_fetch_aliases: Vec<String>,
    pub(in crate::app) cache:
        std::collections::HashMap<String, crate::containers::ContainerCacheEntry>,
    /// Open `Screen::ContainerLogs` overlay payload, `None` when no
    /// logs overlay is open.
    pub(in crate::app) logs_view: Option<LogsView>,
}

impl ContainerState {
    pub fn cache(
        &self,
    ) -> &std::collections::HashMap<String, crate::containers::ContainerCacheEntry> {
        &self.cache
    }

    pub fn set_cache(
        &mut self,
        cache: std::collections::HashMap<String, crate::containers::ContainerCacheEntry>,
    ) {
        self.cache = cache;
    }

    pub fn cache_entry(&self, alias: &str) -> Option<&crate::containers::ContainerCacheEntry> {
        self.cache.get(alias)
    }

    pub fn cache_entry_mut(
        &mut self,
        alias: &str,
    ) -> Option<&mut crate::containers::ContainerCacheEntry> {
        self.cache.get_mut(alias)
    }

    pub fn cache_contains(&self, alias: &str) -> bool {
        self.cache.contains_key(alias)
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    pub fn insert_cache_entry(
        &mut self,
        alias: String,
        entry: crate::containers::ContainerCacheEntry,
    ) {
        self.cache.insert(alias, entry);
    }

    pub fn remove_cache_entry(&mut self, alias: &str) {
        self.cache.remove(alias);
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn pending_exec_request(&self) -> Option<&ContainerExecRequest> {
        self.pending_exec.as_ref()
    }

    pub fn pending_logs_request(&self) -> Option<&ContainerLogsRequest> {
        self.pending_logs.as_ref()
    }

    pub fn has_pending_fetches(&self) -> bool {
        !self.pending_fetch_aliases.is_empty()
    }

    pub fn pending_actions_len(&self) -> usize {
        self.pending_actions.len()
    }

    pub fn take_pending_exec(&mut self) -> Option<ContainerExecRequest> {
        self.pending_exec.take()
    }

    pub fn take_pending_logs(&mut self) -> Option<ContainerLogsRequest> {
        self.pending_logs.take()
    }

    pub fn pop_next_action(&mut self) -> Option<ContainerActionRequest> {
        self.pending_actions.pop_front()
    }

    pub fn pending_actions_iter(&self) -> impl Iterator<Item = &ContainerActionRequest> {
        self.pending_actions.iter()
    }

    pub fn pending_actions_at(&self, idx: usize) -> Option<&ContainerActionRequest> {
        self.pending_actions.get(idx)
    }

    pub fn pending_fetch_aliases(&self) -> &[String] {
        &self.pending_fetch_aliases
    }

    pub fn extend_pending_fetches<I: IntoIterator<Item = String>>(&mut self, iter: I) {
        self.pending_fetch_aliases.extend(iter);
    }

    /// Queue a logs request for the main loop to drain. Replaces any
    /// previous pending logs request and logs the displaced alias so a
    /// dropped request is traceable.
    pub fn queue_logs(&mut self, req: ContainerLogsRequest) {
        if let Some(prev) = self.pending_logs.as_ref() {
            log::debug!(
                "[purple] queue_logs replaced pending request for alias={} id={}",
                prev.alias,
                prev.container_id,
            );
        }
        self.pending_logs = Some(req);
    }

    /// Queue an exec request for the main loop to drain. Same replace
    /// and log semantics as `queue_logs`.
    pub fn queue_exec(&mut self, req: ContainerExecRequest) {
        if let Some(prev) = self.pending_exec.as_ref() {
            log::debug!(
                "[purple] queue_exec replaced pending request for alias={} id={}",
                prev.alias,
                prev.container_id,
            );
        }
        self.pending_exec = Some(req);
    }

    /// Queue a non-interactive container action for the worker thread.
    /// Actions are FIFO via `VecDeque::push_back`; multiple actions
    /// against the same alias process in order.
    pub fn queue_action(&mut self, req: ContainerActionRequest) {
        self.pending_actions.push_back(req);
    }

    /// Enqueue an alias for the initial container-cache fetch. Drained by
    /// the main loop on the next tick via `drain_pending_fetches`.
    pub fn queue_fetch(&mut self, alias: String) {
        self.pending_fetch_aliases.push(alias);
    }

    /// Take the full fetch queue, leaving it empty.
    pub fn drain_pending_fetches(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_fetch_aliases)
    }

    /// Read the active container-logs overlay payload (`None` when no
    /// logs overlay is open).
    pub fn logs_view(&self) -> Option<&LogsView> {
        self.logs_view.as_ref()
    }

    /// Mutable read for the active container-logs overlay payload.
    pub fn logs_view_mut(&mut self) -> Option<&mut LogsView> {
        self.logs_view.as_mut()
    }

    /// Install a fresh logs-view payload. Caller is responsible for
    /// transitioning the screen.
    pub fn set_logs_view(&mut self, view: LogsView) {
        self.logs_view = Some(view);
    }

    /// Drop the logs-view payload. Called on overlay close.
    pub fn clear_logs_view(&mut self) {
        self.logs_view = None;
    }

    /// Drop cache entries whose host alias is no longer in
    /// `valid_aliases`. Returns `true` when anything was dropped so the
    /// caller can persist the trimmed cache via
    /// `containers::save_container_cache`. The cache is also the source
    /// of valid container IDs for downstream inspect/logs pruning.
    pub fn prune_orphans(&mut self, valid_aliases: &std::collections::HashSet<&str>) -> bool {
        let pre = self.cache.len();
        self.cache
            .retain(|alias, _| valid_aliases.contains(alias.as_str()));
        let dropped = pre.saturating_sub(self.cache.len());
        // Logs overlay targeting a deleted host is no longer valid;
        // dropping the view also frees the body Vec. The matching
        // handler restores the screen to HostList on the next key
        // (`container_logs_key` checks for a missing view).
        if let Some(view) = self.logs_view.as_ref()
            && !valid_aliases.contains(view.alias.as_str())
        {
            self.logs_view = None;
        }
        if dropped > 0 {
            log::debug!("[purple] reload_hosts: dropped {dropped} orphan container_cache host(s)");
            true
        } else {
            false
        }
    }

    /// Move a cache entry from `old` to `new` on host rename. Returns
    /// `true` when the cache changed so the caller can persist. No-op
    /// (returns `false`) when `old == new` or no entry exists under `old`.
    pub fn migrate_alias(&mut self, old: &str, new: &str) -> bool {
        if old == new {
            return false;
        }
        // Open logs overlay: rename its alias too so a refresh queued
        // after the rename does not run against the stale name.
        if let Some(view) = self.logs_view.as_mut()
            && view.alias == old
        {
            view.alias = new.to_string();
        }
        if let Some(v) = self.cache.remove(old) {
            debug_assert!(
                !self.cache.contains_key(new),
                "container_state.cache collision on rename {old} -> {new}"
            );
            self.cache.insert(new.to_string(), v);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::containers::{ContainerCacheEntry, ContainerRuntime};

    fn make_logs_request(alias: &str) -> ContainerLogsRequest {
        ContainerLogsRequest {
            alias: alias.to_string(),
            askpass: None,
            runtime: ContainerRuntime::Docker,
            container_id: "abc123".to_string(),
            container_name: "nginx".to_string(),
        }
    }

    fn make_cache_entry() -> ContainerCacheEntry {
        ContainerCacheEntry {
            timestamp: 1700000000,
            runtime: ContainerRuntime::Docker,
            engine_version: Some("28.0.0".to_string()),
            containers: vec![],
        }
    }

    #[test]
    fn queue_logs_sets_pending() {
        let mut state = ContainerState::default();
        assert!(state.pending_logs.is_none());
        state.queue_logs(make_logs_request("host-a"));
        assert!(state.pending_logs.is_some());
        assert_eq!(state.pending_logs.as_ref().unwrap().alias, "host-a");
    }

    #[test]
    fn queue_logs_replaces_previous() {
        let mut state = ContainerState::default();
        state.queue_logs(make_logs_request("host-a"));
        state.queue_logs(make_logs_request("host-b"));
        assert_eq!(state.pending_logs.as_ref().unwrap().alias, "host-b");
    }

    #[test]
    fn queue_exec_sets_pending() {
        let mut state = ContainerState::default();
        assert!(state.pending_exec.is_none());
        state.queue_exec(ContainerExecRequest {
            alias: "host-a".to_string(),
            askpass: None,
            runtime: ContainerRuntime::Docker,
            container_id: "abc".to_string(),
            container_name: "nginx".to_string(),
            command: Some("echo hi".to_string()),
        });
        assert!(state.pending_exec.is_some());
        assert_eq!(state.pending_exec.as_ref().unwrap().alias, "host-a");
    }

    #[test]
    fn queue_fetch_pushes_alias() {
        let mut state = ContainerState::default();
        state.queue_fetch("host-a".to_string());
        state.queue_fetch("host-b".to_string());
        assert_eq!(state.pending_fetch_aliases, vec!["host-a", "host-b"]);
    }

    #[test]
    fn drain_pending_fetches_returns_and_clears() {
        let mut state = ContainerState::default();
        state.queue_fetch("host-a".to_string());
        state.queue_fetch("host-b".to_string());
        let drained = state.drain_pending_fetches();
        assert_eq!(drained, vec!["host-a", "host-b"]);
        assert!(state.pending_fetch_aliases.is_empty());
    }

    #[test]
    fn drain_pending_fetches_empty_when_no_aliases() {
        let mut state = ContainerState::default();
        let drained = state.drain_pending_fetches();
        assert!(drained.is_empty());
        assert!(state.pending_fetch_aliases.is_empty());
    }

    #[test]
    fn migrate_alias_renames_cache_entry() {
        let mut state = ContainerState::default();
        state.cache.insert("old".to_string(), make_cache_entry());
        assert!(state.migrate_alias("old", "new"));
        assert!(state.cache.contains_key("new"));
        assert!(!state.cache.contains_key("old"));
    }

    #[test]
    fn migrate_alias_returns_false_when_no_entry() {
        let mut state = ContainerState::default();
        assert!(!state.migrate_alias("missing", "new"));
        assert!(state.cache.is_empty());
    }

    #[test]
    fn migrate_alias_self_rename_is_noop() {
        let mut state = ContainerState::default();
        state.cache.insert("same".to_string(), make_cache_entry());
        assert!(!state.migrate_alias("same", "same"));
        assert!(state.cache.contains_key("same"));
    }

    #[test]
    fn queue_action_pushes_back_in_order() {
        let mut state = ContainerState::default();
        for id in ["a", "b", "c"] {
            state.queue_action(ContainerActionRequest {
                alias: "host".to_string(),
                askpass: None,
                runtime: ContainerRuntime::Docker,
                container_id: id.to_string(),
                container_name: id.to_string(),
                action: crate::containers::ContainerAction::Restart,
            });
        }
        assert_eq!(state.pending_actions.len(), 3);
        let ids: Vec<String> = state
            .pending_actions
            .iter()
            .map(|r| r.container_id.clone())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn prune_orphans_drops_unknown_aliases_and_signals_persist() {
        let mut state = ContainerState::default();
        state.cache.insert("keep".to_string(), make_cache_entry());
        state.cache.insert("drop".to_string(), make_cache_entry());

        let valid: std::collections::HashSet<&str> = ["keep"].into_iter().collect();
        let changed = state.prune_orphans(&valid);

        assert!(changed, "returns true so caller persists the trimmed cache");
        assert!(state.cache.contains_key("keep"));
        assert!(!state.cache.contains_key("drop"));
    }

    #[test]
    fn prune_orphans_returns_false_when_nothing_dropped() {
        let mut state = ContainerState::default();
        state.cache.insert("keep".to_string(), make_cache_entry());

        let valid: std::collections::HashSet<&str> = ["keep"].into_iter().collect();
        assert!(!state.prune_orphans(&valid));
    }
}
