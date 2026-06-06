//! Unified jump bar types.
//!
//! Sources hosts, tunnels, containers, snippets and actions in one ranked
//! list. Sections render in a fixed order. Empty sections are omitted.

use std::path::PathBuf;

use crate::fs_util::atomic_write;
use crate::runtime::env::Paths;

/// What kind of thing a jump hit represents. Drives the type-marker glyph
/// rendered in the left column and the section grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Host,
    Tunnel,
    Container,
    Snippet,
    Action,
}

impl SourceKind {
    pub fn section_label(self) -> &'static str {
        match self {
            Self::Host => "HOSTS",
            Self::Tunnel => "TUNNELS",
            Self::Container => "CONTAINERS",
            Self::Snippet => "SNIPPETS",
            Self::Action => "ACTIONS",
        }
    }

    /// Fixed render order. Empty sections are skipped at render time but the
    /// order itself never changes. Keeps muscle memory stable.
    pub fn render_order() -> [Self; 5] {
        [
            Self::Host,
            Self::Tunnel,
            Self::Container,
            Self::Snippet,
            Self::Action,
        ]
    }
}

/// One row in the unified jump bar. Each variant carries enough state for the
/// dispatch step to navigate the user to the matched item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpHit {
    Action(JumpAction),
    Host(HostHit),
    Tunnel(TunnelHit),
    Container(ContainerHit),
    Snippet(SnippetHit),
}

impl JumpHit {
    pub fn kind(&self) -> SourceKind {
        match self {
            Self::Action(_) => SourceKind::Action,
            Self::Host(_) => SourceKind::Host,
            Self::Tunnel(_) => SourceKind::Tunnel,
            Self::Container(_) => SourceKind::Container,
            Self::Snippet(_) => SourceKind::Snippet,
        }
    }

    /// All searchable strings, including aliases. Score = max over haystacks.
    /// Returns borrowed slices so the scoring loop is allocation-free per
    /// hit. The single exception is the action hotkey which needs a tiny
    /// owned buffer; we render it via `key_str` which is a `String` field
    /// on `JumpAction`.
    pub fn haystacks(&self) -> Vec<&str> {
        match self {
            Self::Action(a) => {
                let mut v = Vec::with_capacity(2 + a.aliases.len());
                v.push(a.label);
                v.push(a.key_str);
                for alias in a.aliases {
                    v.push(*alias);
                }
                v
            }
            Self::Host(h) => {
                let mut v = Vec::with_capacity(7 + h.tags.len());
                v.push(h.alias.as_str());
                v.push(h.hostname.as_str());
                if let Some(p) = &h.provider {
                    v.push(p.as_str());
                }
                for t in &h.tags {
                    v.push(t.as_str());
                }
                if !h.user.is_empty() {
                    v.push(h.user.as_str());
                }
                if !h.identity_file.is_empty() {
                    v.push(h.identity_file.as_str());
                }
                if !h.proxy_jump.is_empty() {
                    v.push(h.proxy_jump.as_str());
                }
                if let Some(role) = &h.vault_ssh {
                    v.push(role.as_str());
                }
                v
            }
            Self::Tunnel(t) => vec![t.alias.as_str(), t.destination.as_str(), &t.bind_port_str],
            Self::Container(c) => vec![
                c.container_name.as_str(),
                c.alias.as_str(),
                c.container_id.as_str(),
            ],
            Self::Snippet(s) => vec![s.name.as_str(), s.command_preview.as_str()],
        }
    }

    /// Stable identity used for MRU dedup.
    pub fn identity(&self) -> RecentRef {
        match self {
            Self::Action(a) => RecentRef::new(SourceKind::Action, a.key.to_string()),
            Self::Host(h) => RecentRef::new(SourceKind::Host, h.alias.clone()),
            Self::Tunnel(t) => {
                RecentRef::new(SourceKind::Tunnel, format!("{}:{}", t.alias, t.bind_port))
            }
            Self::Container(c) => RecentRef::new(
                SourceKind::Container,
                format!("{}/{}", c.alias, c.container_name),
            ),
            Self::Snippet(s) => RecentRef::new(SourceKind::Snippet, s.name.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpAction {
    pub key: char,
    /// Same letter as `key` but as a `&'static str` so it can be used as a
    /// haystack without allocating per scoring call. Stored once in the
    /// static action table; verified by debug assertion in tests.
    pub key_str: &'static str,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
    /// Which top-page handler executes this action. The dispatch path
    /// switches `app.top_page` to this target before synthesising the
    /// hotkey keypress, so `Tunnels: Add tunnel` works from the Hosts
    /// tab and vice versa.
    pub target: JumpActionTarget,
    /// Modifier flags applied to the synthesised keypress. Lets the
    /// palette dispatch modifier-bound shortcuts like Ctrl-k (restart
    /// compose stack) that the underlying handler routes through a
    /// distinct match arm. `KeyModifiers::NONE` for the common case.
    pub modifiers: crossterm::event::KeyModifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpActionTarget {
    Hosts,
    Tunnels,
    Containers,
    Keys,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostHit {
    pub alias: String,
    pub hostname: String,
    pub tags: Vec<String>,
    pub provider: Option<String>,
    pub user: String,
    pub identity_file: String,
    pub proxy_jump: String,
    pub vault_ssh: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelHit {
    pub alias: String,
    pub bind_port: u16,
    /// Pre-rendered port number, kept around so `haystacks()` can return
    /// borrowed slices instead of allocating a fresh `format!` per
    /// keystroke.
    pub bind_port_str: String,
    pub destination: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerHit {
    pub alias: String,
    pub container_name: String,
    pub container_id: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetHit {
    pub name: String,
    pub command_preview: String,
}

/// Stable reference to a hit, used for the on-disk MRU log and for
/// dispatching jumps.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RecentRef {
    pub kind: SourceKind,
    pub key: String,
}

impl RecentRef {
    pub fn new(kind: SourceKind, key: String) -> Self {
        Self { kind, key }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentEntry {
    #[serde(flatten)]
    pub target: RecentRef,
    pub last_used_unix: i64,
}

/// On-disk schema for `~/.purple/recents.json`. Versioned so future shape
/// changes can rev without dropping user state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecentsFile {
    pub version: u32,
    pub entries: Vec<RecentEntry>,
}

impl Default for RecentsFile {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

const RECENTS_VERSION: u32 = 1;
const RECENTS_CAP: usize = 50;

/// Resolve the recents file path from the injected paths
/// (`~/.purple/recents.json`). `None` when the home directory is unknown.
pub fn recents_path(paths: Option<&Paths>) -> Option<PathBuf> {
    paths.map(Paths::recents)
}

pub fn load_recents(paths: Option<&Paths>) -> RecentsFile {
    let Some(path) = recents_path(paths) else {
        return RecentsFile::default();
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return RecentsFile::default(),
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_recents(file: &RecentsFile, paths: Option<&Paths>) -> std::io::Result<()> {
    let Some(path) = recents_path(paths) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(file).map_err(std::io::Error::other)?;
    atomic_write(&path, &bytes)
}

/// Rewrite host recents from `old_alias` to `new_alias`. Called from the
/// host-form rename path so the jump bar's RECENT section keeps the host
/// after a rename. When both aliases already have entries (defensive) the
/// newer `last_used_unix` wins and the duplicate is dropped.
///
/// Returns `true` when the file changed.
pub fn rename_host_recent(file: &mut RecentsFile, old_alias: &str, new_alias: &str) -> bool {
    if old_alias == new_alias {
        return false;
    }
    let old_idx = file
        .entries
        .iter()
        .position(|e| e.target.kind == SourceKind::Host && e.target.key == old_alias);
    let Some(old_idx) = old_idx else {
        return false;
    };
    let new_idx = file
        .entries
        .iter()
        .position(|e| e.target.kind == SourceKind::Host && e.target.key == new_alias);
    if let Some(new_idx) = new_idx {
        let drop_idx =
            if file.entries[old_idx].last_used_unix >= file.entries[new_idx].last_used_unix {
                new_idx
            } else {
                old_idx
            };
        let keep_idx = if drop_idx == new_idx {
            old_idx
        } else {
            new_idx
        };
        file.entries[keep_idx].target.key = new_alias.to_string();
        file.entries.remove(drop_idx);
    } else {
        file.entries[old_idx].target.key = new_alias.to_string();
    }
    file.version = RECENTS_VERSION;
    true
}

/// Insert or move-to-front a recent ref. Caps the list at `RECENTS_CAP`.
pub fn touch_recent(file: &mut RecentsFile, target: RecentRef) {
    file.version = RECENTS_VERSION;
    file.entries.retain(|e| e.target != target);
    let now = current_unix_ts();
    file.entries.insert(
        0,
        RecentEntry {
            target,
            last_used_unix: now,
        },
    );
    if file.entries.len() > RECENTS_CAP {
        file.entries.truncate(RECENTS_CAP);
    }
}

fn current_unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Which command set the jump bar displays. Determined by the screen that
/// opened the jump bar so the action list matches what the underlying
/// handler can dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JumpMode {
    #[default]
    Hosts,
    Tunnels,
    Containers,
    Snippets,
    Keys,
}

/// On the empty-query state we show only the top-N actions to keep the
/// first impression a short menu rather than a wall. The full list is
/// one keystroke away. Lives in the data layer so `visible_hits()`,
/// `empty_state_groups()` and the Down handler all agree on the bound.
pub const JUMP_EMPTY_STATE_ACTIONS_CAP: usize = 6;

/// On context-specific tabs (Tunnels, Containers) the empty-state bumps
/// up to this many actions of the active tab's category to the front,
/// before round-robining the remaining slots across other categories.
/// Sized so half the cap surfaces tab actions and the other half stays
/// reachable as a hub menu (cross-tab discovery).
const EMPTY_STATE_TAB_BIAS: usize = 3;

/// Display order for action categories on the empty state. The
/// round-robin walks these buckets in this order, NOT in static-table
/// order, so the first impression always shows the most-used categories
/// regardless of how the static action list happens to be sorted.
/// Categories not listed here fall through to a stable last-seen order.
const CATEGORY_PRIORITY: &[&str] = &[
    "Hosts",
    "Tunnels",
    "Containers",
    "Files",
    "Vault",
    "Keys",
    "Providers",
    "Snippets",
    "Clipboard",
    "Settings",
    "Help",
];

/// Minimum nucleo score for actions. Below this the action is dropped from
/// results. Stops broad character-scatter matches on action labels.
pub(crate) const PALETTE_ACTION_FLOOR: u32 = 30;

/// Reorder actions so the first N show one per category, the next N
/// show the second action of each category, etc. Preserves within-bucket
/// order so muscle memory survives. Buckets are visited in
/// `CATEGORY_PRIORITY` order. declarative, decoupled from static-table
/// row order. so the empty-state top-N always leads with the most
/// important categories. Categories not in the priority list fall to
/// the end in stable encounter order.
fn round_robin_actions_by_category(actions: impl Iterator<Item = JumpAction>) -> Vec<JumpHit> {
    let mut buckets: Vec<(String, Vec<JumpAction>)> = Vec::new();
    for action in actions {
        let category = action
            .label
            .split_once(':')
            .map(|(c, _)| c.trim().to_string())
            .unwrap_or_else(|| "Other".to_string());
        if let Some(slot) = buckets.iter_mut().find(|(c, _)| c == &category) {
            slot.1.push(action);
        } else {
            buckets.push((category, vec![action]));
        }
    }
    let priority_index = |cat: &str| -> usize {
        CATEGORY_PRIORITY
            .iter()
            .position(|p| *p == cat)
            .unwrap_or(usize::MAX)
    };
    buckets.sort_by_key(|(c, _)| priority_index(c));
    let mut out: Vec<JumpHit> = Vec::new();
    let mut depth = 0usize;
    let max_depth = buckets.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    while depth < max_depth {
        for (_, bucket) in &buckets {
            if let Some(action) = bucket.get(depth) {
                out.push(JumpHit::Action(*action));
            }
        }
        depth += 1;
    }
    out
}

/// Like `round_robin_actions_by_category` but pulls up to `bump` actions
/// whose dispatch `target` matches `preferred` to the front before
/// round-robining the rest. Used by the empty-state on context-specific
/// tabs (Tunnels, Containers) so the user sees actions that operate on
/// the active tab, not just actions whose label happens to start with
/// the same prefix. Filtering by `target` (dispatch destination) instead
/// of label category keeps `Containers: List containers` (target=Hosts,
/// opens the legacy per-host overlay) out of the bias on the Containers
/// tab, where it would otherwise crowd out the genuinely tab-relevant
/// `Refresh / Cycle sort / Toggle detail panel` actions.
fn round_robin_actions_with_bias(
    actions: impl Iterator<Item = JumpAction>,
    preferred: JumpActionTarget,
    bump: usize,
) -> Vec<JumpHit> {
    let collected: Vec<JumpAction> = actions.collect();
    let biased: Vec<JumpAction> = collected
        .iter()
        .filter(|a| a.target == preferred)
        .take(bump)
        .copied()
        .collect();
    let biased_keys: std::collections::HashSet<char> = biased.iter().map(|a| a.key).collect();
    let rest: Vec<JumpAction> = collected
        .into_iter()
        .filter(|a| !(biased_keys.contains(&a.key) && a.target == preferred))
        .collect();
    let mut out: Vec<JumpHit> = biased.into_iter().map(JumpHit::Action).collect();
    out.extend(round_robin_actions_by_category(rest.into_iter()));
    out
}

/// Like `round_robin_actions_with_bias` but pulls up to `bump` actions whose
/// LABEL category matches `category` to the front. Used by the Snippets tab,
/// whose run-actions dispatch to the host list (`target == Hosts`) yet belong to
/// the "Snippets" category, so the target-based bias would miss them.
fn round_robin_actions_with_category_bias(
    actions: impl Iterator<Item = JumpAction>,
    category: &str,
    bump: usize,
) -> Vec<JumpHit> {
    let collected: Vec<JumpAction> = actions.collect();
    let cat_of = |a: &JumpAction| a.label.split_once(':').map(|(c, _)| c.trim()).unwrap_or("");
    let biased: Vec<JumpAction> = collected
        .iter()
        .filter(|a| cat_of(a) == category)
        .take(bump)
        .copied()
        .collect();
    let biased_keys: std::collections::HashSet<char> = biased.iter().map(|a| a.key).collect();
    let rest: Vec<JumpAction> = collected
        .into_iter()
        .filter(|a| !(biased_keys.contains(&a.key) && cat_of(a) == category))
        .collect();
    let mut out: Vec<JumpHit> = biased.into_iter().map(JumpHit::Action).collect();
    out.extend(round_robin_actions_by_category(rest.into_iter()));
    out
}

#[derive(Debug, Default)]
pub struct JumpState {
    pub(in crate::app) query: String,
    pub(in crate::app) selected: usize,
    pub(in crate::app) mode: JumpMode,
    /// Computed result list, recomputed on every query change. Empty until
    /// `App::recompute_jump_hits` runs.
    pub(in crate::app) hits: Vec<JumpHit>,
    /// MRU snapshot loaded on jump bar open, used by the empty-query state.
    pub(in crate::app) recents: Vec<JumpHit>,
    /// True once the user has navigated (Down/Up/Tab) at least once. The
    /// renderer keeps the selection invisible on the empty state until
    /// this flips, so the eye stays on the input field on first open.
    /// Also makes the FIRST Down keystroke land on row 0 instead of
    /// skipping to row 1.
    pub(in crate::app) cursor_revealed: bool,
    /// Reused matcher with growable scratch buffers. Populated lazily on
    /// the first scoring pass and kept across keystrokes so nucleo's
    /// internal vectors do not reallocate every recompute.
    pub(in crate::app) matcher: Option<nucleo_matcher::Matcher>,
}

// Manual `Clone` because `nucleo_matcher::Matcher` is not `Clone`. State
// clones (e.g. in tests) drop the cached matcher and let the next
// recompute build a fresh one. correct behavior, just slightly slower
// for the next keystroke after a clone.
impl Clone for JumpState {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            selected: self.selected,
            mode: self.mode,
            hits: self.hits.clone(),
            recents: self.recents.clone(),
            cursor_revealed: self.cursor_revealed,
            matcher: None,
        }
    }
}

impl JumpState {
    pub fn for_mode(mode: JumpMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn mode(&self) -> JumpMode {
        self.mode
    }

    pub fn cursor_revealed(&self) -> bool {
        self.cursor_revealed
    }

    pub fn hits(&self) -> &[JumpHit] {
        &self.hits
    }

    pub fn recents(&self) -> &[JumpHit] {
        &self.recents
    }

    pub fn set_selected(&mut self, n: usize) {
        self.selected = n;
    }

    pub fn set_hits(&mut self, hits: Vec<JumpHit>) {
        self.hits = hits;
    }

    pub fn set_recents(&mut self, recents: Vec<JumpHit>) {
        self.recents = recents;
    }

    /// Down arrow: on first navigation reveal the cursor on row 0;
    /// thereafter advance by one, capped at the last visible row.
    pub fn move_down(&mut self) {
        let count = self.visible_hits().len();
        if count == 0 {
            return;
        }
        if !self.cursor_revealed {
            self.cursor_revealed = true;
            self.selected = 0;
        } else {
            self.selected = (self.selected + 1).min(count - 1);
        }
    }

    /// Up arrow: on first navigation reveal the cursor on row 0;
    /// thereafter step back saturating at row 0.
    pub fn move_up(&mut self) {
        if !self.cursor_revealed {
            self.cursor_revealed = true;
            self.selected = 0;
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    pub fn reveal_cursor(&mut self) {
        self.cursor_revealed = true;
    }

    /// Backspace cleared the query: re-hide the selection cue and re-park
    /// the cursor on row 0 so the eye lands back on the input field.
    pub fn reset_after_clear_query(&mut self) {
        self.cursor_revealed = false;
        self.selected = 0;
    }

    pub fn push_query(&mut self, c: char) {
        if self.query.len() < 64 {
            self.query.push(c);
        }
        // Selection is handled by `App::recompute_jump_hits` which
        // tries to keep the previously-selected hit's identity. We do
        // NOT reset to 0 here because that would defeat mid-typing
        // navigation: typing a char must not jump the cursor.
    }

    pub fn pop_query(&mut self) {
        self.query.pop();
    }

    /// Return the hit list to render. With an empty query this is the
    /// composed empty-state view (recents + the round-robin top-N
    /// actions); otherwise it is the live computed `hits`. The cap on
    /// the empty state is applied HERE (data layer) so the Down/Up
    /// handlers, `visible_hits().len()`, and the renderer all agree on
    /// the same bound. without this, scrolling past the rendered cap
    /// would silently advance `selected` into invisible rows and the
    /// highlight would appear to jump back to row 0.
    pub fn visible_hits(&self) -> Vec<JumpHit> {
        if self.query.is_empty() {
            let mut out: Vec<JumpHit> = self.recents.clone();
            out.extend(self.empty_state_actions());
            out
        } else {
            // Return hits in the same fixed-section render order the overlay
            // lays them out, preserving the score order within each section.
            // Navigation and dispatch index this list while the renderer
            // groups by `SourceKind`; the two must agree or the highlighted
            // row and the executed hit drift apart.
            let mut out: Vec<JumpHit> = Vec::with_capacity(self.hits.len());
            for kind in SourceKind::render_order() {
                out.extend(self.hits.iter().filter(|h| h.kind() == kind).cloned());
            }
            out
        }
    }

    /// Action set for the empty-state, after the `recent_keys` filter
    /// is applied. Shared by `empty_state_actions` (which adds bias and
    /// caps) and `empty_state_actions_total` (which just counts).
    /// Centralising the filter predicate guarantees the rendered
    /// "Actions  N of M" header stays in sync with the rendered list
    /// across future edits.
    fn filtered_actions_for_empty_state(&self) -> Vec<JumpAction> {
        let recent_keys: std::collections::HashSet<RecentRef> =
            self.recents.iter().map(|h| h.identity()).collect();
        JumpAction::for_mode(self.mode)
            .iter()
            .filter(|a| {
                let id = RecentRef::new(SourceKind::Action, a.key.to_string());
                !recent_keys.contains(&id)
            })
            .copied()
            .collect()
    }

    /// Top-N actions for the empty-state, after `recent_keys` filtering
    /// and the optional tab-bias. Single source of truth for both the
    /// renderer (`empty_state_groups`) and the navigation handler
    /// (`visible_hits`); without it the two would drift on the bias and
    /// the cursor would land on different rows than the user sees.
    fn empty_state_actions(&self) -> Vec<JumpHit> {
        let filtered = self.filtered_actions_for_empty_state();
        let preferred_target = match self.mode {
            JumpMode::Hosts => None,
            JumpMode::Tunnels => Some(JumpActionTarget::Tunnels),
            JumpMode::Containers => Some(JumpActionTarget::Containers),
            JumpMode::Keys => Some(JumpActionTarget::Keys),
            // Snippet run-actions dispatch to the host list (target=Hosts), so
            // they bias by label category instead of by dispatch target.
            JumpMode::Snippets => None,
        };
        let actions = match self.mode {
            JumpMode::Snippets => round_robin_actions_with_category_bias(
                filtered.into_iter(),
                "Snippets",
                EMPTY_STATE_TAB_BIAS,
            ),
            _ => match preferred_target {
                Some(t) => {
                    round_robin_actions_with_bias(filtered.into_iter(), t, EMPTY_STATE_TAB_BIAS)
                }
                None => round_robin_actions_by_category(filtered.into_iter()),
            },
        };
        actions
            .into_iter()
            .take(JUMP_EMPTY_STATE_ACTIONS_CAP)
            .collect()
    }

    /// Number of actions available for the empty-state ACTIONS section
    /// BEFORE the cap. Used by the renderer to render `Actions  6 of 29`
    /// when the cap is applied.
    pub fn empty_state_actions_total(&self) -> usize {
        self.filtered_actions_for_empty_state().len()
    }

    /// Group `visible_hits()` for the query view: by `SourceKind` in render
    /// order. Empty sections are omitted. Only meaningful when a query is
    /// active; the empty-state view uses `empty_state_groups` instead.
    pub fn grouped_hits(&self) -> Vec<(SourceKind, Vec<JumpHit>)> {
        let visible = self.visible_hits();
        let mut out = Vec::with_capacity(SourceKind::render_order().len());
        for kind in SourceKind::render_order() {
            let group: Vec<JumpHit> = visible
                .iter()
                .filter(|h| h.kind() == kind)
                .cloned()
                .collect();
            if !group.is_empty() {
                out.push((kind, group));
            }
        }
        out
    }

    /// Empty-state grouping: a single `RECENT` group (everything that came
    /// from the MRU log, of any kind) followed by an `ACTIONS` group.
    /// Returns `(label, hits)` rather than `(kind, hits)` so the renderer
    /// can distinguish "RECENT" from a per-kind label.
    pub fn empty_state_groups(&self) -> Vec<(&'static str, Vec<JumpHit>)> {
        let mut out: Vec<(&'static str, Vec<JumpHit>)> = Vec::new();
        if !self.recents.is_empty() {
            out.push(("RECENT", self.recents.clone()));
        }
        // Single source of truth shared with `visible_hits` so the
        // navigation cursor and the rendered list cannot drift.
        let actions = self.empty_state_actions();
        if !actions.is_empty() {
            out.push(("ACTIONS", actions));
        }
        out
    }

    /// Map `selected` index (into `visible_hits()`) to a `SourceKind` so the
    /// renderer knows which section header is currently active.
    pub fn selected_section(&self) -> Option<SourceKind> {
        self.visible_hits().get(self.selected).map(|h| h.kind())
    }

    /// Return actions whose label substring-matches the current query.
    /// Test-only shim for tests that predate the unified jump bar.
    /// Production code iterates `visible_hits()` instead.
    #[cfg(test)]
    pub fn filtered_commands(&self) -> Vec<JumpAction> {
        let all = JumpAction::for_mode(self.mode);
        if self.query.is_empty() {
            return all.to_vec();
        }
        let q = self.query.to_lowercase();
        all.iter()
            .filter(|cmd| {
                cmd.label.to_lowercase().contains(&q)
                    || cmd.aliases.iter().any(|a| a.to_lowercase().contains(&q))
            })
            .copied()
            .collect()
    }

    /// Move selection to the first hit in the next non-empty section. Wraps.
    pub fn jump_next_section(&mut self) {
        let visible = self.visible_hits();
        if visible.is_empty() {
            return;
        }
        if self.query.is_empty() {
            // Empty-state has up to two groups: RECENT (length =
            // recents.len()) and ACTIONS (the rest). Tab toggles between
            // their first rows. Skip the toggle if there is no second
            // group to jump to (e.g. no recents, or no actions after
            // recents). The two `if` branches inside this block both fire
            // in real cases: from RECENT row n we jump to actions; from
            // an action row we wrap back to the first recent.
            let n_recent = self.recents.len();
            if n_recent == 0 || n_recent >= visible.len() {
                return;
            }
            if self.selected < n_recent {
                self.selected = n_recent; // RECENT -> ACTIONS
            } else {
                self.selected = 0; // ACTIONS -> first RECENT
            }
            return;
        }
        let groups = self.grouped_hits();
        if groups.len() < 2 {
            return;
        }
        let cur_kind = match self.selected_section() {
            Some(k) => k,
            None => {
                self.selected = 0;
                return;
            }
        };
        let cur_idx = groups.iter().position(|(k, _)| *k == cur_kind).unwrap_or(0);
        let next_idx = (cur_idx + 1) % groups.len();
        let next_kind = groups[next_idx].0;
        if let Some(pos) = visible.iter().position(|h| h.kind() == next_kind) {
            self.selected = pos;
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    // `test_path` is thread-local, so each test thread gets an isolated
    // recents file with no shared state. No process-wide lock is needed.
    fn with_temp<F: FnOnce(&Paths)>(f: F) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        f(&paths);
    }

    #[test]
    fn snippets_mode_empty_state_leads_with_snippet_actions() {
        // The Snippets tab opens jump in Snippets mode; the empty-state must
        // bias the "Snippets:" run-actions to the front (they dispatch to the
        // host list, so the bias is by label category, not by target).
        let state = JumpState::for_mode(JumpMode::Snippets);
        let hits = state.visible_hits();
        match hits.first().expect("at least one action") {
            JumpHit::Action(a) => assert!(
                a.label.starts_with("Snippets:"),
                "expected a Snippets action first, got {:?}",
                a.label
            ),
            other => panic!("expected an action first, got {other:?}"),
        }
    }

    #[test]
    fn hosts_mode_empty_state_does_not_lead_with_snippet_actions() {
        // Control: in the default Hosts mode the round-robin leads with the
        // highest-priority category (Hosts), not Snippets.
        let state = JumpState::for_mode(JumpMode::Hosts);
        let hits = state.visible_hits();
        if let Some(JumpHit::Action(a)) = hits.first() {
            assert!(
                !a.label.starts_with("Snippets:"),
                "Hosts mode should not lead with a Snippets action, got {:?}",
                a.label
            );
        }
    }

    #[test]
    fn visible_hits_matches_grouped_render_order_with_active_query() {
        // Regression for the jump-bar mis-dispatch: navigation (`move_down`)
        // and dispatch index `visible_hits()`, while the renderer lays rows
        // out in `grouped_hits()` order. When a query matches across sections
        // the score-sorted flat list interleaves kinds differently from the
        // fixed render order, so the highlighted row and the executed hit
        // point at different items. Pin that the two orders agree.
        let action = JumpAction::all()[0];
        let host = HostHit {
            alias: "proxy-vm".into(),
            hostname: "proxy-vm.example.com".into(),
            tags: Vec::new(),
            provider: None,
            user: String::new(),
            identity_file: String::new(),
            proxy_jump: String::new(),
            vault_ssh: None,
        };
        // Score order puts the action first (a strong action match outranks a
        // fuzzy host match). The renderer regroups HOSTS before ACTIONS.
        let state = JumpState {
            query: "prov".into(),
            hits: vec![JumpHit::Action(action), JumpHit::Host(host)],
            ..Default::default()
        };

        let visible = state.visible_hits();
        let flattened: Vec<JumpHit> = state
            .grouped_hits()
            .into_iter()
            .flat_map(|(_, hits)| hits)
            .collect();
        assert_eq!(
            visible, flattened,
            "visible_hits() must equal the flattened grouped order so the \
             highlighted row and the dispatched hit reference the same item"
        );
        // Row 0 is what the selection cue lands on first; with HOSTS rendered
        // before ACTIONS it must be the host, matching what the user sees.
        assert!(
            matches!(visible[0], JumpHit::Host(_)),
            "first visible row must follow render order (HOSTS first)"
        );
    }

    #[test]
    fn section_labels_are_uppercase() {
        for k in SourceKind::render_order() {
            let label = k.section_label();
            assert_eq!(label, label.to_uppercase(), "{:?} not uppercase", k);
        }
    }

    #[test]
    fn render_order_starts_with_hosts() {
        assert_eq!(SourceKind::render_order()[0], SourceKind::Host);
        assert_eq!(SourceKind::render_order()[4], SourceKind::Action);
    }

    #[test]
    fn touch_moves_existing_to_front_and_caps() {
        let mut f = RecentsFile::default();
        for i in 0..(RECENTS_CAP + 5) {
            touch_recent(&mut f, RecentRef::new(SourceKind::Host, format!("h{i}")));
        }
        assert_eq!(f.entries.len(), RECENTS_CAP);
        // Re-touching an existing ref moves it to the front.
        let target = RecentRef::new(SourceKind::Host, format!("h{}", RECENTS_CAP + 2));
        touch_recent(&mut f, target.clone());
        assert_eq!(f.entries[0].target, target);
        assert_eq!(f.entries.len(), RECENTS_CAP);
    }

    #[test]
    fn save_then_load_roundtrip() {
        with_temp(|paths| {
            let mut f = RecentsFile::default();
            touch_recent(&mut f, RecentRef::new(SourceKind::Action, "F".into()));
            touch_recent(&mut f, RecentRef::new(SourceKind::Host, "web-01".into()));
            save_recents(&f, Some(paths)).expect("save");
            let loaded = load_recents(Some(paths));
            assert_eq!(loaded.version, RECENTS_VERSION);
            assert_eq!(loaded.entries.len(), 2);
            assert_eq!(loaded.entries[0].target.key, "web-01");
            assert_eq!(loaded.entries[1].target.key, "F");
        });
    }

    #[test]
    fn missing_file_loads_empty() {
        with_temp(|paths| {
            let loaded = load_recents(Some(paths));
            assert!(loaded.entries.is_empty());
        });
    }

    #[test]
    fn corrupt_file_loads_empty() {
        with_temp(|paths| {
            let path = paths.recents();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"not json").unwrap();
            let loaded = load_recents(Some(paths));
            assert!(loaded.entries.is_empty());
        });
    }

    fn host_entry(alias: &str, ts: i64) -> RecentEntry {
        RecentEntry {
            target: RecentRef::new(SourceKind::Host, alias.to_string()),
            last_used_unix: ts,
        }
    }

    #[test]
    fn rename_host_recent_rewrites_key() {
        let mut file = RecentsFile::default();
        file.entries.push(host_entry("web-old", 100));
        file.entries.push(RecentEntry {
            target: RecentRef::new(SourceKind::Tunnel, "web-old:5432".to_string()),
            last_used_unix: 90,
        });

        assert!(rename_host_recent(&mut file, "web-old", "web-new"));
        assert_eq!(file.entries[0].target.kind, SourceKind::Host);
        assert_eq!(file.entries[0].target.key, "web-new");
        // Non-host entries with a coincidental key prefix are untouched.
        assert_eq!(file.entries[1].target.kind, SourceKind::Tunnel);
        assert_eq!(file.entries[1].target.key, "web-old:5432");
    }

    #[test]
    fn rename_host_recent_dedups_on_collision_keeping_most_recent() {
        let mut file = RecentsFile::default();
        // Old entry is more recent. After rename the newer timestamp must
        // survive and the older duplicate must be dropped.
        file.entries.push(host_entry("a", 200));
        file.entries.push(host_entry("b", 100));

        assert!(rename_host_recent(&mut file, "a", "b"));
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].target.key, "b");
        assert_eq!(file.entries[0].last_used_unix, 200);
    }

    #[test]
    fn rename_host_recent_dedups_when_new_key_is_newer() {
        let mut file = RecentsFile::default();
        file.entries.push(host_entry("a", 100));
        file.entries.push(host_entry("b", 200));

        assert!(rename_host_recent(&mut file, "a", "b"));
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.entries[0].target.key, "b");
        assert_eq!(file.entries[0].last_used_unix, 200);
    }

    #[test]
    fn rename_host_recent_noop_when_same() {
        let mut file = RecentsFile::default();
        file.entries.push(host_entry("a", 10));
        assert!(!rename_host_recent(&mut file, "a", "a"));
        assert_eq!(file.entries.len(), 1);
    }

    #[test]
    fn rename_host_recent_noop_when_absent() {
        let mut file = RecentsFile::default();
        assert!(!rename_host_recent(&mut file, "ghost", "phantom"));
        assert!(file.entries.is_empty());
    }
}
