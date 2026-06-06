use std::collections::HashSet;

use ratatui::widgets::ListState;

use crate::app::SnippetFormBaseline;
use crate::app::forms::{SnippetForm, SnippetOutputState, SnippetParamFormState};
use crate::snippet::{Snippet, SnippetStore};

/// Why the snippet host picker is open: to run the snippet now, or to pick the
/// default target hosts for the open edit form. Decides where Enter/Esc return
/// and whether a run is launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnippetHostPickPurpose {
    /// Select hosts then run (-> `Screen::ConfirmRunSnippet`).
    #[default]
    Run,
    /// Select hosts then write them to the edit form's `default_hosts`
    /// (-> `Screen::SnippetForm`). An empty selection clears the defaults.
    EditDefault,
}

/// Picker half for the snippet -> hosts run flow. Mirrors the picker fields of
/// `KeyPushState`; snippets carry no worker state of their own because
/// execution reuses the existing snippet output machinery.
#[derive(Default)]
pub struct SnippetHostPick {
    /// Why the picker is open (run now vs set the edit form's default hosts).
    pub purpose: SnippetHostPickPurpose,
    /// Aliases toggled in `Screen::SnippetHostPicker`. Frozen into
    /// `flow_targets` on Enter.
    pub selected: HashSet<String>,
    /// Cursor in the picker's host list.
    pub list_state: ListState,
    /// Applied type-to-filter query. Empty means no filter (grouped browse).
    pub query: String,
    /// Whether keystrokes currently edit `query` (filter mode entered via `/`)
    /// rather than acting as selection commands.
    pub filtering: bool,
}

impl SnippetHostPick {
    /// Reset selection, filter and cursor. Called when the picker opens for a
    /// fresh snippet; pre-selection of saved targets happens after this.
    pub fn reset(&mut self) {
        self.selected.clear();
        self.query.clear();
        self.filtering = false;
        self.list_state.select(Some(0));
        self.purpose = SnippetHostPickPurpose::Run;
    }
}

/// Snippet-owned state grouped off the `App` god-struct. Holds the on-disk
/// snippet store, the edit form, the pending execution payload, the output
/// screen state, the param form, the terminal-submit flag, the dirty-check
/// baseline and the pending-delete index. Pure state container.
pub struct SnippetState {
    pub(in crate::app) store: SnippetStore,
    pub(in crate::app) form: SnippetForm,
    pub(in crate::app) pending: Option<(Snippet, Vec<String>)>,
    pub(in crate::app) output: Option<SnippetOutputState>,
    pub(in crate::app) param_form: Option<SnippetParamFormState>,
    pub(in crate::app) pending_terminal: bool,
    pub(in crate::app) form_baseline: Option<SnippetFormBaseline>,
    pub(in crate::app) pending_delete: Option<usize>,
    /// Target host aliases for the currently-open snippet flow
    /// (`Screen::SnippetPicker`, `SnippetForm`, `SnippetOutput`,
    /// `SnippetParamForm`). The Screen variants are data-less; the
    /// alias list lives here so screen transitions never clone it.
    pub(in crate::app) flow_targets: Vec<String>,
    /// `editing` payload for an open `Screen::SnippetForm`. `None`
    /// when adding a new snippet, `Some(index)` when editing an
    /// existing one. Set alongside the screen transition.
    pub(in crate::app) form_editing: Option<usize>,
    /// Snippet currently displayed by `Screen::SnippetParamForm`.
    pub(in crate::app) param_snippet: Option<Snippet>,
    /// Snippet name shown by `Screen::SnippetOutput` in its title.
    pub(in crate::app) output_snippet_name: Option<String>,
    /// Snippet-list cursor for the Snippets tab. Indexes the filtered list
    /// when `app.search` is active; the handler translates back to a store
    /// index before any data access.
    pub(in crate::app) list_state: ListState,
    /// Detail-panel visibility for the Snippets tab, toggled by `v`. Drives
    /// the shared detail animation exactly as the Containers tab does.
    pub(in crate::app) view_mode: crate::app::ViewMode,
    /// Host picker selection for the snippet -> hosts run flow.
    pub(in crate::app) host_pick: SnippetHostPick,
    /// Snippet chosen on the tab, cloned when the host picker opens and read by
    /// the confirm body and the run step. Cleared when the flow ends.
    pub(in crate::app) flow_snippet: Option<Snippet>,
    /// Whether the pending run uses terminal (foreground) mode. Set from the
    /// host picker (`!` vs Enter), read by the run step.
    pub(in crate::app) flow_terminal: bool,
    /// Whether the current snippet form / param form was opened from the
    /// Snippets tab (vs the host-list snippet picker). Decides where closing
    /// the form returns and which base page renders behind it.
    pub(in crate::app) form_return_to_tab: bool,
    /// Per-snippet run ledger, read by the detail TRACK RECORD verdict and
    /// trend chart, appended on every run completion.
    pub(in crate::app) runs: crate::snippet_runs::SnippetRunLog,
}

impl Default for SnippetState {
    fn default() -> Self {
        Self {
            store: SnippetStore::default(),
            form: SnippetForm::new(),
            pending: None,
            output: None,
            param_form: None,
            pending_terminal: false,
            form_baseline: None,
            pending_delete: None,
            flow_targets: Vec::new(),
            form_editing: None,
            param_snippet: None,
            output_snippet_name: None,
            list_state: ListState::default(),
            view_mode: crate::app::ViewMode::Detailed,
            host_pick: SnippetHostPick::default(),
            flow_snippet: None,
            flow_terminal: false,
            form_return_to_tab: false,
            runs: crate::snippet_runs::SnippetRunLog::default(),
        }
    }
}

impl SnippetState {
    pub fn store(&self) -> &SnippetStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut SnippetStore {
        &mut self.store
    }

    pub fn form(&self) -> &SnippetForm {
        &self.form
    }

    pub fn form_mut(&mut self) -> &mut SnippetForm {
        &mut self.form
    }

    pub fn output(&self) -> Option<&SnippetOutputState> {
        self.output.as_ref()
    }

    pub fn output_mut(&mut self) -> Option<&mut SnippetOutputState> {
        self.output.as_mut()
    }

    pub fn set_output(&mut self, output: Option<SnippetOutputState>) {
        self.output = output;
    }

    pub fn take_output(&mut self) -> Option<SnippetOutputState> {
        self.output.take()
    }

    pub fn param_form(&self) -> Option<&SnippetParamFormState> {
        self.param_form.as_ref()
    }

    pub fn param_form_mut(&mut self) -> Option<&mut SnippetParamFormState> {
        self.param_form.as_mut()
    }

    pub fn set_param_form(&mut self, param_form: Option<SnippetParamFormState>) {
        self.param_form = param_form;
    }

    pub fn pending_delete(&self) -> Option<usize> {
        self.pending_delete
    }

    pub fn take_pending_delete(&mut self) -> Option<usize> {
        self.pending_delete.take()
    }

    pub fn pending(&self) -> Option<&(Snippet, Vec<String>)> {
        self.pending.as_ref()
    }

    pub fn take_pending(&mut self) -> Option<(Snippet, Vec<String>)> {
        self.pending.take()
    }

    pub fn set_pending(&mut self, value: Option<(Snippet, Vec<String>)>) {
        self.pending = value;
    }

    pub fn flow_targets(&self) -> &[String] {
        &self.flow_targets
    }

    pub fn set_flow_targets(&mut self, targets: Vec<String>) {
        self.flow_targets = targets;
    }

    pub fn clear_flow_targets(&mut self) {
        self.flow_targets.clear();
    }

    pub fn form_editing(&self) -> Option<usize> {
        self.form_editing
    }

    pub fn set_form_editing(&mut self, editing: Option<usize>) {
        self.form_editing = editing;
    }

    pub fn param_snippet(&self) -> Option<&Snippet> {
        self.param_snippet.as_ref()
    }

    pub fn set_param_snippet(&mut self, snippet: Option<Snippet>) {
        self.param_snippet = snippet;
    }

    pub fn output_snippet_name(&self) -> Option<&str> {
        self.output_snippet_name.as_deref()
    }

    pub fn set_output_snippet_name(&mut self, name: Option<String>) {
        self.output_snippet_name = name;
    }

    pub fn pending_terminal(&self) -> bool {
        self.pending_terminal
    }

    pub fn set_pending_terminal(&mut self, value: bool) {
        self.pending_terminal = value;
    }

    pub fn form_baseline(&self) -> Option<&SnippetFormBaseline> {
        self.form_baseline.as_ref()
    }

    /// True if the snippet form differs from its captured baseline.
    pub fn form_is_dirty(&self) -> bool {
        match &self.form_baseline {
            Some(b) => {
                self.form.name != b.name
                    || self.form.command != b.command
                    || self.form.description != b.description
                    || self.form.default_hosts != b.default_hosts
            }
            None => false,
        }
    }

    pub fn set_form_baseline(&mut self, baseline: Option<SnippetFormBaseline>) {
        self.form_baseline = baseline;
    }

    /// Construct with snippet store loaded from disk.
    pub fn with_store_loaded(paths: Option<&crate::runtime::env::Paths>) -> Self {
        Self {
            store: crate::snippet::SnippetStore::load(paths),
            runs: crate::snippet_runs::SnippetRunLog::load(paths),
            ..Self::default()
        }
    }

    pub fn runs(&self) -> &crate::snippet_runs::SnippetRunLog {
        &self.runs
    }

    pub fn runs_mut(&mut self) -> &mut crate::snippet_runs::SnippetRunLog {
        &mut self.runs
    }

    /// Open a delete confirmation for the snippet at `idx`. The renderer
    /// reads `pending_delete` to draw the confirm overlay.
    pub fn request_delete(&mut self, idx: usize) {
        self.pending_delete = Some(idx);
    }

    /// Dismiss a pending delete confirmation. Idempotent.
    pub fn cancel_delete(&mut self) {
        self.pending_delete = None;
    }

    /// Close the parameter substitution form. Clears the form state and
    /// the terminal-submit flag that decide whether the next Enter sends
    /// the resolved command to the foreground terminal or to background
    /// output capture. Idempotent.
    pub fn close_param_form(&mut self) {
        self.param_form = None;
        self.pending_terminal = false;
    }

    pub fn list_state(&self) -> &ListState {
        &self.list_state
    }

    pub fn list_state_mut(&mut self) -> &mut ListState {
        &mut self.list_state
    }

    pub fn view_mode(&self) -> crate::app::ViewMode {
        self.view_mode
    }

    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            crate::app::ViewMode::Detailed => crate::app::ViewMode::Compact,
            crate::app::ViewMode::Compact => crate::app::ViewMode::Detailed,
        };
    }

    pub fn host_pick(&self) -> &SnippetHostPick {
        &self.host_pick
    }

    pub fn host_pick_mut(&mut self) -> &mut SnippetHostPick {
        &mut self.host_pick
    }

    pub fn reset_host_pick(&mut self) {
        self.host_pick.reset();
    }

    pub fn flow_snippet(&self) -> Option<&Snippet> {
        self.flow_snippet.as_ref()
    }

    pub fn set_flow_snippet(&mut self, snippet: Option<Snippet>) {
        self.flow_snippet = snippet;
    }

    pub fn take_flow_snippet(&mut self) -> Option<Snippet> {
        self.flow_snippet.take()
    }

    pub fn flow_terminal(&self) -> bool {
        self.flow_terminal
    }

    pub fn set_flow_terminal(&mut self, value: bool) {
        self.flow_terminal = value;
    }

    pub fn form_return_to_tab(&self) -> bool {
        self.form_return_to_tab
    }

    pub fn set_form_return_to_tab(&mut self, value: bool) {
        self.form_return_to_tab = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_empty() {
        let s = SnippetState::default();
        assert!(s.pending.is_none());
        assert!(s.output.is_none());
        assert!(s.param_form.is_none());
        assert!(!s.pending_terminal);
        assert!(s.form_baseline.is_none());
        assert!(s.pending_delete.is_none());
    }

    #[test]
    fn request_delete_sets_pending_delete_to_some_idx() {
        let mut s = SnippetState::default();
        s.request_delete(3);
        assert_eq!(s.pending_delete, Some(3));
    }

    #[test]
    fn cancel_delete_clears_pending_delete() {
        let mut s = SnippetState {
            pending_delete: Some(2),
            ..Default::default()
        };
        s.cancel_delete();
        assert!(s.pending_delete.is_none());
    }

    #[test]
    fn request_delete_overwrites_existing_pending() {
        let mut s = SnippetState {
            pending_delete: Some(1),
            ..Default::default()
        };
        s.request_delete(7);
        assert_eq!(s.pending_delete, Some(7));
    }

    #[test]
    fn close_param_form_clears_param_form_and_pending_terminal() {
        let mut s = SnippetState {
            param_form: Some(SnippetParamFormState::new(&[])),
            pending_terminal: true,
            ..Default::default()
        };
        s.close_param_form();
        assert!(s.param_form.is_none());
        assert!(!s.pending_terminal);
    }

    #[test]
    fn close_param_form_preserves_pending_output_and_store() {
        use crate::snippet::Snippet;
        let mut s = SnippetState {
            param_form: Some(SnippetParamFormState::new(&[])),
            pending_terminal: true,
            pending: Some((
                Snippet {
                    name: "ls".into(),
                    command: "ls -la".into(),
                    description: String::new(),
                },
                vec!["host-a".into()],
            )),
            ..Default::default()
        };

        s.close_param_form();

        assert!(
            s.pending.is_some(),
            "pending stays for the consumer to read"
        );
        assert!(s.pending_delete.is_none());
    }

    #[test]
    fn close_param_form_is_idempotent_when_already_none() {
        let mut s = SnippetState::default();
        s.close_param_form();
        s.close_param_form();
        assert!(s.param_form.is_none());
        assert!(!s.pending_terminal);
    }

    fn state_matching_baseline() -> SnippetState {
        let mut s = SnippetState::default();
        s.form.name = "deploy".into();
        s.form.command = "make deploy".into();
        s.form.description = "ship it".into();
        s.set_form_baseline(Some(SnippetFormBaseline {
            name: "deploy".into(),
            command: "make deploy".into(),
            description: "ship it".into(),
            default_hosts: Vec::new(),
        }));
        s
    }

    #[test]
    fn form_is_dirty_is_false_without_a_baseline() {
        let mut s = SnippetState::default();
        s.form.name = "edited".into();
        assert!(!s.form_is_dirty());
    }

    #[test]
    fn form_is_dirty_is_false_when_form_equals_baseline() {
        assert!(!state_matching_baseline().form_is_dirty());
    }

    fn assert_field_change_is_dirty(field: &str, mutate: impl FnOnce(&mut SnippetForm)) {
        let mut s = state_matching_baseline();
        mutate(&mut s.form);
        assert!(s.form_is_dirty(), "a change in {field} must read dirty");
    }

    #[test]
    fn form_is_dirty_detects_a_change_in_each_field() {
        assert_field_change_is_dirty("name", |f| f.name.push('x'));
        assert_field_change_is_dirty("command", |f| f.command.push('x'));
        assert_field_change_is_dirty("description", |f| f.description.push('x'));
    }

    #[test]
    fn default_view_mode_is_detailed() {
        let s = SnippetState::default();
        assert_eq!(s.view_mode(), crate::app::ViewMode::Detailed);
    }

    #[test]
    fn toggle_view_mode_flips_between_detailed_and_compact() {
        let mut s = SnippetState::default();
        s.toggle_view_mode();
        assert_eq!(s.view_mode(), crate::app::ViewMode::Compact);
        s.toggle_view_mode();
        assert_eq!(s.view_mode(), crate::app::ViewMode::Detailed);
    }

    #[test]
    fn reset_host_pick_clears_selection_and_resets_cursor() {
        let mut s = SnippetState::default();
        s.host_pick_mut().selected.insert("h1".into());
        s.host_pick_mut().list_state.select(Some(4));
        s.reset_host_pick();
        assert!(s.host_pick().selected.is_empty());
        assert_eq!(s.host_pick().list_state.selected(), Some(0));
    }

    #[test]
    fn flow_snippet_set_and_take_round_trips() {
        let mut s = SnippetState::default();
        assert!(s.flow_snippet().is_none());
        s.set_flow_snippet(Some(Snippet {
            name: "deploy".into(),
            command: "make deploy".into(),
            description: String::new(),
        }));
        assert_eq!(s.flow_snippet().map(|s| s.name.as_str()), Some("deploy"));
        let taken = s.take_flow_snippet();
        assert_eq!(taken.map(|s| s.name), Some("deploy".to_string()));
        assert!(s.flow_snippet().is_none());
    }

    #[test]
    fn flow_terminal_defaults_false_and_sets() {
        let mut s = SnippetState::default();
        assert!(!s.flow_terminal());
        s.set_flow_terminal(true);
        assert!(s.flow_terminal());
    }
}
