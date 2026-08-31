//! Tag domain state: per-host tag tracking and bulk-tag-editor model.

use crate::app::host_state::GroupBy;
use crate::ssh_config::model::HostEntry;

/// A display tag with its source (user-defined or provider-synced).
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayTag {
    pub name: String,
    pub is_user: bool,
}

/// Select up to 3 tags for display based on view mode and grouping.
/// Returns a Vec of up to 3 DisplayTags (user tags first, then provider tags).
///
/// In grouped views the tag matching the group criterion is suppressed
/// (it lives in the group header). Non-matching provider tags and the
/// provider name itself stay visible.
pub fn select_display_tags(
    host: &HostEntry,
    group_by: &GroupBy,
    detail_mode: bool,
) -> Vec<DisplayTag> {
    let group_name = match group_by {
        GroupBy::Provider => host.provider.clone(),
        GroupBy::Tag(t) => Some(t.clone()),
        GroupBy::None => None,
    };

    let not_group = |t: &str| {
        group_name
            .as_ref()
            .is_none_or(|g| !t.eq_ignore_ascii_case(g))
    };

    let user_tags = host
        .tags
        .iter()
        .filter(|t| not_group(t))
        .map(|t| DisplayTag {
            name: t.to_string(),
            is_user: true,
        });

    let provider_tags = host
        .provider_tags
        .iter()
        .filter(|t| not_group(t))
        .chain(host.provider.iter().filter(|p| not_group(p)))
        .map(|t| DisplayTag {
            name: t.to_string(),
            is_user: false,
        });

    let limit = if detail_mode { 1 } else { 3 };
    user_tags.chain(provider_tags).take(limit).collect()
}

/// Tag editor state.
#[derive(Default)]
pub struct TagState {
    pub(in crate::app) input: Option<String>,
    pub(in crate::app) cursor: usize,
    pub(in crate::app) list: Vec<String>,
}

impl TagState {
    /// Open the inline tag-edit input on the host detail screen with the
    /// given seed text. Cursor lands at the end of the text so users can
    /// type extra tags without re-positioning.
    pub(crate) fn open_tag_input(&mut self, text: String) {
        self.cursor = text.chars().count();
        self.input = Some(text);
    }

    /// Close the inline tag-edit input. Called on both Enter (after the
    /// submit hits disk) and Esc (cancel) so the two fields cannot drift
    /// out of sync.
    pub(crate) fn close_tag_input(&mut self) {
        self.input = None;
        self.cursor = 0;
    }

    pub fn input(&self) -> Option<&str> {
        self.input.as_deref()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn list(&self) -> &[String] {
        &self.list
    }

    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn cursor_right(&mut self) {
        if let Some(ref input) = self.input
            && self.cursor < input.chars().count()
        {
            self.cursor += 1;
        }
    }

    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    pub fn cursor_end(&mut self) {
        if let Some(ref input) = self.input {
            self.cursor = input.chars().count();
        }
    }

    /// Insert a char at the cursor position and advance the cursor.
    /// No-op when the input is not active.
    pub fn insert_char(&mut self, c: char) {
        if let Some(ref mut input) = self.input {
            let byte_pos = super::char_to_byte_pos(input, self.cursor);
            input.insert(byte_pos, c);
            self.cursor += 1;
        }
    }

    /// Delete the char left of the cursor. No-op when cursor is 0 or input is inactive.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if let Some(ref mut input) = self.input {
            let byte_pos = super::char_to_byte_pos(input, self.cursor);
            let prev = super::char_to_byte_pos(input, self.cursor - 1);
            input.drain(prev..byte_pos);
            self.cursor -= 1;
        }
    }
}

/// User action per tag row in the bulk tag editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkTagAction {
    /// `[~]` Leave each host's state for this tag unchanged.
    Leave,
    /// `[x]` Ensure the tag is present on every selected host.
    AddToAll,
    /// `[ ]` Ensure the tag is absent from every selected host.
    RemoveFromAll,
}

impl BulkTagAction {
    /// 3-way cycle: `Leave` → `AddToAll` → `RemoveFromAll` → `Leave`.
    pub fn cycle(self) -> Self {
        match self {
            BulkTagAction::Leave => BulkTagAction::AddToAll,
            BulkTagAction::AddToAll => BulkTagAction::RemoveFromAll,
            BulkTagAction::RemoveFromAll => BulkTagAction::Leave,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            BulkTagAction::Leave => "[~]",
            BulkTagAction::AddToAll => "[x]",
            BulkTagAction::RemoveFromAll => "[ ]",
        }
    }
}

/// A single row in the bulk tag editor.
#[derive(Debug, Clone)]
pub struct BulkTagRow {
    pub tag: String,
    /// Number of selected hosts that had this tag at editor open time.
    pub initial_count: usize,
    pub action: BulkTagAction,
}

/// Snapshot state for the bulk tag editor overlay.
#[derive(Debug, Default)]
pub struct BulkTagEditorState {
    pub rows: Vec<BulkTagRow>,
    /// Aliases being edited, snapshot at open time so selection changes
    /// during the flow do not affect the in-progress edit.
    pub aliases: Vec<String>,
    /// Aliases that live in an Include file and cannot be edited in place.
    /// Surfaced in the header so the user sees the blast radius.
    pub skipped_included: Vec<String>,
    /// Draft name for a brand-new tag being typed by the user. `None` when
    /// the input bar is inactive. Newly entered tags are appended to `rows`
    /// with `action = AddToAll`.
    pub new_tag_input: Option<String>,
    pub new_tag_cursor: usize,
    /// Snapshot of `rows[i].action` at editor open time. Used by `is_dirty`
    /// to detect pending changes on Esc and prompt the user before
    /// discarding. Captured by the opener (e.g. `App::open_bulk_tag_editor`)
    /// after `rows` is populated.
    ///
    /// Length-mismatch semantics: any extra row beyond the baseline length
    /// (i.e. a newly added tag via `+`) counts as dirty if its action is
    /// non-Leave. This matches the user's intuition that "I typed a new tag,
    /// closing now should warn me".
    pub initial_actions: Vec<BulkTagAction>,
}

impl BulkTagEditorState {
    /// Returns true if any row's action differs from the open-time baseline,
    /// or if rows have been added since open.
    ///
    /// Single source of truth for the dirty check. The handler consults this
    /// on Esc to decide between immediate exit and discard confirmation.
    /// Every editable surface gets a dirty-check so Esc never drops unsaved
    /// work.
    ///
    /// **Invariant**: rows is append-only after `open_bulk_tag_editor`
    /// captures the baseline. The `+ new tag` flow only appends to `rows`;
    /// no code path removes rows during the editor session. If a future
    /// change introduces row removal, the length-mismatch branch below will
    /// silently treat the missing baseline rows as clean (because `zip`
    /// stops at the shorter slice). At that point this method needs an
    /// explicit shrink branch; the assertion below guards the assumption.
    pub fn is_dirty(&self) -> bool {
        debug_assert!(
            self.rows.len() >= self.initial_actions.len(),
            "rows must be append-only after baseline capture; \
             shorter rows breaks the dirty-check"
        );
        if self.rows.len() != self.initial_actions.len() {
            // Tags added since open. New rows count as dirty unless still Leave.
            return self
                .rows
                .iter()
                .skip(self.initial_actions.len())
                .any(|r| r.action != BulkTagAction::Leave)
                || self
                    .rows
                    .iter()
                    .zip(self.initial_actions.iter())
                    .any(|(r, baseline)| r.action != *baseline);
        }
        self.rows
            .iter()
            .zip(self.initial_actions.iter())
            .any(|(r, baseline)| r.action != *baseline)
    }
}

/// Outcome of applying a bulk tag edit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BulkTagApplyResult {
    /// Hosts whose tag list actually changed.
    pub changed_hosts: usize,
    /// Total (host, tag) additions.
    pub added: usize,
    /// Total (host, tag) removals.
    pub removed: usize,
    /// Hosts skipped because they live in an Include file.
    pub skipped_included: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_tag_input_seeds_text_and_parks_cursor_at_end() {
        let mut t = TagState::default();
        t.open_tag_input("prod, web".to_string());
        assert_eq!(t.input.as_deref(), Some("prod, web"));
        assert_eq!(t.cursor, "prod, web".chars().count());
    }

    #[test]
    fn open_tag_input_with_empty_text_lands_cursor_at_zero() {
        let mut t = TagState::default();
        t.open_tag_input(String::new());
        assert_eq!(t.input.as_deref(), Some(""));
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn open_tag_input_counts_chars_not_bytes() {
        // Cursor units are character positions; multi-byte text must not
        // produce a byte-offset cursor (host_detail handler indexes by
        // chars when converting to byte positions).
        let mut t = TagState::default();
        t.open_tag_input("café".to_string());
        assert_eq!(t.cursor, 4);
    }

    #[test]
    fn close_tag_input_clears_both_fields() {
        let mut t = TagState::default();
        t.open_tag_input("staging".to_string());
        t.close_tag_input();
        assert!(t.input.is_none());
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn close_tag_input_on_idle_state_is_noop() {
        let mut t = TagState::default();
        t.close_tag_input();
        assert!(t.input.is_none());
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn close_tag_input_does_not_touch_picker_list() {
        // The `list` field powers the tag picker overlay and lives
        // independently of the inline tag-edit input.
        let mut t = TagState {
            list: vec!["prod".to_string(), "web".to_string()],
            ..Default::default()
        };
        t.open_tag_input("staging".to_string());
        t.close_tag_input();
        assert_eq!(t.list, vec!["prod".to_string(), "web".to_string()]);
    }
}
