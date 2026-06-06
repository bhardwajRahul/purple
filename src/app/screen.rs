//! Screen enum: tags the currently-displayed overlay or view.

/// Top-level page selected via the top navigation bar.
///
/// Orthogonal to [`Screen`]. `Screen` tracks overlays and modal forms,
/// `TopPage` tracks which base view (hosts, tunnels, containers, keys)
/// renders behind them. Tab/Shift+Tab cycles through the variants when
/// no overlay is active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TopPage {
    #[default]
    Hosts,
    Tunnels,
    Containers,
    Snippets,
    Keys,
}

impl TopPage {
    /// Cycle to the next page
    /// (Hosts -> Tunnels -> Containers -> Snippets -> Keys -> Hosts).
    pub fn next(self) -> Self {
        match self {
            TopPage::Hosts => TopPage::Tunnels,
            TopPage::Tunnels => TopPage::Containers,
            TopPage::Containers => TopPage::Snippets,
            TopPage::Snippets => TopPage::Keys,
            TopPage::Keys => TopPage::Hosts,
        }
    }

    /// Cycle to the previous page
    /// (Hosts -> Keys -> Snippets -> Containers -> Tunnels -> Hosts).
    pub fn prev(self) -> Self {
        match self {
            TopPage::Hosts => TopPage::Keys,
            TopPage::Tunnels => TopPage::Hosts,
            TopPage::Containers => TopPage::Tunnels,
            TopPage::Snippets => TopPage::Containers,
            TopPage::Keys => TopPage::Snippets,
        }
    }
}

/// State for the What's New overlay.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct WhatsNewState {
    pub scroll: u16,
}

/// Search state for the container logs viewer. `None` on
/// `Screen::ContainerLogs.search` means no search is active.
///
/// Modeless: while the struct is `Some`, every keystroke either edits
/// the query (chars / cursor / delete) or navigates matches
/// (Tab / Shift+Tab). There is no "confirm" step: matches are
/// recomputed live and `Esc` exits search outright.
///
/// `matches` are line indices into the rendered body; `current`
/// indexes into `matches`. Smart case is decided by the query at
/// match time: any uppercase rune flips to case-sensitive (vim's
/// `'smartcase'`).
///
/// `cursor_pos` is a char index into `query` (0..=chars().count()).
/// Mirrors the host_form pattern so Left/Right/Home/End/Delete edit
/// mid-string instead of forcing append-only behaviour.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ContainerLogsSearch {
    pub query: String,
    pub matches: Vec<usize>,
    pub current: usize,
    pub cursor_pos: usize,
}

impl ContainerLogsSearch {
    pub fn insert_char(&mut self, c: char) {
        let byte_pos = super::forms::char_to_byte_pos(&self.query, self.cursor_pos);
        self.query.insert(byte_pos, c);
        self.cursor_pos += 1;
    }

    pub fn delete_char_before_cursor(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let byte_pos = super::forms::char_to_byte_pos(&self.query, self.cursor_pos);
        let prev = super::forms::char_to_byte_pos(&self.query, self.cursor_pos - 1);
        self.query.drain(prev..byte_pos);
        self.cursor_pos -= 1;
    }

    pub fn delete_char_at_cursor(&mut self) {
        let len = self.query.chars().count();
        if self.cursor_pos >= len {
            return;
        }
        let byte_pos = super::forms::char_to_byte_pos(&self.query, self.cursor_pos);
        let next = super::forms::char_to_byte_pos(&self.query, self.cursor_pos + 1);
        self.query.drain(byte_pos..next);
    }

    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    pub fn move_right(&mut self) {
        let len = self.query.chars().count();
        if self.cursor_pos < len {
            self.cursor_pos += 1;
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_pos = self.query.chars().count();
    }
}

/// One running compose-stack member surfaced in the stack-restart
/// confirm dialog. Carried so the confirm body can list every
/// container that will be cycled, identity-and-state-clear.
#[derive(Debug, Clone, PartialEq)]
pub struct StackMember {
    pub container_id: String,
    pub container_name: String,
    pub uptime: Option<String>,
}

/// Which screen is currently displayed.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    HostList,
    AddHost,
    EditHost {
        alias: String,
    },
    ConfirmDelete {
        alias: String,
    },
    Help {
        return_screen: Box<Screen>,
    },
    KeyList,
    KeyDetail {
        index: usize,
    },
    /// Multi-host picker reached from the Keys tab by pressing `p`.
    /// `key_index` points into `app.keys.list` for the key to push. The picker
    /// shows hosts with checkbox selection; hosts whose `vault_ssh` role
    /// is configured are dimmed and not selectable (Vault SSH workflow
    /// uses signed certs, not authorized_keys appends).
    KeyPushPicker {
        key_index: usize,
    },
    /// Destructive confirm shown after the picker commits. Footer renders
    /// action verbs both sides via `design::confirm_footer_destructive`.
    /// On y the worker thread is spawned and the screen returns to
    /// HostList; on n/Esc returns to the picker with selection intact.
    ///
    /// The frozen alias list lives on `app.keys.push.committed` instead
    /// of inside the Screen variant: keeping the vec out of the enum
    /// prevents per-frame clones during overlay redraws and keeps the
    /// `Screen` payload uniformly small.
    ConfirmKeyPush {
        key_index: usize,
    },
    HostDetail {
        index: usize,
    },
    TagPicker,
    ThemePicker,
    Providers,
    ProviderForm {
        id: crate::providers::config::ProviderConfigId,
    },
    /// Step 1 of the lazy add-second-config flow: ask the user to pick a
    /// label for the existing (bare) config of `provider` before opening
    /// the new-config form. The chosen label lives on
    /// `app.providers.pending_label_migration` until step 2 saves both
    /// configs together.
    ProviderLabelMigration {
        provider: String,
    },
    TunnelList {
        alias: String,
    },
    TunnelForm {
        alias: String,
        editing: Option<usize>,
    },
    /// Host picker reached from the Tunnels overview when adding a new
    /// tunnel: the user must choose a host before the tunnel form opens.
    /// On confirm, transitions to `TunnelForm { alias, editing: None }`.
    TunnelHostPicker,
    /// Multi-host picker for snippet execution. The host aliases live
    /// on `snippets.flow_targets`; the variant stays data-less.
    SnippetPicker,
    /// Edit form for a snippet. `editing` (index) and target_aliases
    /// live on `snippets.form_editing` / `snippets.flow_targets`.
    SnippetForm,
    /// Output viewer after running a snippet against the flow targets.
    /// `snippet_name` lives on `snippets.output_snippet_name`;
    /// `target_aliases` on `snippets.flow_targets`.
    SnippetOutput,
    /// Param-substitution form shown before running a parametrised
    /// snippet. The `Snippet` lives on `snippets.param_snippet` and
    /// the target aliases on `snippets.flow_targets`.
    SnippetParamForm,
    /// Multi-host picker for running a snippet from the Snippets tab. The
    /// chosen snippet lives on `snippets.flow_snippet`; the picker selection
    /// on `snippets.host_pick().selected`. Data-less variant.
    SnippetHostPicker,
    /// Confirm dialog shown after the snippet host picker commits. Lists the
    /// snippet name and target host count. On y the run proceeds via
    /// `run_or_prompt_params`; on n/Esc returns to the picker with the
    /// selection intact. Data-less.
    ConfirmRunSnippet,
    ConfirmHostKeyReset {
        alias: String,
        hostname: String,
        known_hosts_path: String,
        askpass: Option<String>,
    },
    FileBrowser {
        alias: String,
    },
    Containers {
        alias: String,
    },
    /// Picker reached from the containers overview when adding a host
    /// to the cache (`a`). Lists hosts that have no cache entry yet;
    /// on Enter, spawns a `docker ps` listing for the chosen host and
    /// returns to the overview.
    ContainerHostPicker,
    /// One-shot logs viewer for a single container. The identity
    /// (alias / container_id / container_name), the streaming body, the
    /// scroll position and the search state all live on
    /// `container_state.logs_view`; this variant only tags the open
    /// overlay so the dispatch table reads cleanly.
    ContainerLogs,
    /// Confirm dialog for `K` (kick). restart a single running
    /// container. Reuses `route_confirm_key` so y/n/Esc are the only
    /// effective inputs; stake-test footer phrases the verb on both
    /// sides.
    ConfirmContainerRestart {
        alias: String,
        container_id: String,
        container_name: String,
        project: Option<String>,
        uptime: Option<String>,
    },
    /// Confirm dialog for `S` (stop). stop a single running container.
    /// Same key contract as ConfirmContainerRestart.
    ConfirmContainerStop {
        alias: String,
        container_id: String,
        container_name: String,
        project: Option<String>,
        uptime: Option<String>,
    },
    /// Single-line prompt for an arbitrary command to run inside the
    /// container via `docker exec -it`. Submit hits the existing
    /// `pending_container_exec` flow with the typed command in place
    /// of the default `bash || sh`.
    ContainerExecPrompt {
        alias: String,
        container_id: String,
        container_name: String,
        query: String,
    },
    /// Confirm dialog for `Ctrl-K` (stack kick). Restarts every running
    /// member of a compose stack on a single host, sequentially. The
    /// alias/project/members payload lives on
    /// `containers_overview.pending_bulk_confirm` so screen
    /// transitions stay allocation-free.
    ConfirmStackRestart,
    /// Confirm dialog for `K` pressed on a host-divider row in the
    /// containers overview. Restarts every running container on the
    /// host. Payload on `containers_overview.pending_bulk_confirm`.
    ConfirmHostRestartAll,
    /// Confirm dialog for `S` pressed on a host-divider row. Stops
    /// every running container on the host. Payload on
    /// `containers_overview.pending_bulk_confirm`.
    ConfirmHostStopAll,
    ConfirmImport {
        count: usize,
    },
    /// Confirm dialog for purging stale (provider-managed but deleted)
    /// hosts. The alias list and provider scope live on
    /// `providers.pending_purge` so the variant stays data-less.
    ConfirmPurgeStale,
    /// Confirm dialog for `V` (bulk vault sign). The precomputed
    /// signable list lives on `vault.pending_sign` so the screen
    /// variant stays data-less.
    ConfirmVaultSign,
    Welcome {
        has_backup: bool,
        host_count: usize,
        known_hosts_count: usize,
    },
    /// Bulk tag editor: tri-state checkbox picker that edits tags across
    /// all hosts in `multi_select` in one go. Opened via `t` when a
    /// multi-host selection is active.
    BulkTagEditor,
    /// What's New overlay: shows recent changelog sections to the user
    /// after an upgrade. Opened via the upgrade toast or `n` key.
    WhatsNew(WhatsNewState),
}

impl Screen {
    /// Stable short variant name used in state-transition logs.
    /// Omits inner fields so log lines never leak host aliases, paths or
    /// tokens.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Screen::HostList => "HostList",
            Screen::AddHost => "AddHost",
            Screen::EditHost { .. } => "EditHost",
            Screen::ConfirmDelete { .. } => "ConfirmDelete",
            Screen::Help { .. } => "Help",
            Screen::KeyList => "KeyList",
            Screen::KeyDetail { .. } => "KeyDetail",
            Screen::KeyPushPicker { .. } => "KeyPushPicker",
            Screen::ConfirmKeyPush { .. } => "ConfirmKeyPush",
            Screen::HostDetail { .. } => "HostDetail",
            Screen::TagPicker => "TagPicker",
            Screen::ThemePicker => "ThemePicker",
            Screen::Providers => "Providers",
            Screen::ProviderForm { .. } => "ProviderForm",
            Screen::ProviderLabelMigration { .. } => "ProviderLabelMigration",
            Screen::TunnelList { .. } => "TunnelList",
            Screen::TunnelForm { .. } => "TunnelForm",
            Screen::TunnelHostPicker => "TunnelHostPicker",
            Screen::SnippetPicker => "SnippetPicker",
            Screen::SnippetForm => "SnippetForm",
            Screen::SnippetOutput => "SnippetOutput",
            Screen::SnippetParamForm => "SnippetParamForm",
            Screen::SnippetHostPicker => "SnippetHostPicker",
            Screen::ConfirmRunSnippet => "ConfirmRunSnippet",
            Screen::ConfirmHostKeyReset { .. } => "ConfirmHostKeyReset",
            Screen::FileBrowser { .. } => "FileBrowser",
            Screen::Containers { .. } => "Containers",
            Screen::ContainerHostPicker => "ContainerHostPicker",
            Screen::ContainerLogs => "ContainerLogs",
            Screen::ConfirmContainerRestart { .. } => "ConfirmContainerRestart",
            Screen::ConfirmContainerStop { .. } => "ConfirmContainerStop",
            Screen::ContainerExecPrompt { .. } => "ContainerExecPrompt",
            Screen::ConfirmStackRestart => "ConfirmStackRestart",
            Screen::ConfirmHostRestartAll => "ConfirmHostRestartAll",
            Screen::ConfirmHostStopAll => "ConfirmHostStopAll",
            Screen::ConfirmImport { .. } => "ConfirmImport",
            Screen::ConfirmPurgeStale => "ConfirmPurgeStale",
            Screen::ConfirmVaultSign => "ConfirmVaultSign",
            Screen::Welcome { .. } => "Welcome",
            Screen::BulkTagEditor => "BulkTagEditor",
            Screen::WhatsNew(_) => "WhatsNew",
        }
    }
}

#[cfg(test)]
mod top_page_tests {
    use super::TopPage;

    #[test]
    fn cycle_includes_snippets_between_containers_and_keys() {
        assert_eq!(TopPage::Containers.next(), TopPage::Snippets);
        assert_eq!(TopPage::Snippets.next(), TopPage::Keys);
        assert_eq!(TopPage::Keys.prev(), TopPage::Snippets);
        assert_eq!(TopPage::Snippets.prev(), TopPage::Containers);
    }

    #[test]
    fn full_forward_cycle_wraps() {
        let mut p = TopPage::Hosts;
        for _ in 0..5 {
            p = p.next();
        }
        assert_eq!(p, TopPage::Hosts);
    }
}
