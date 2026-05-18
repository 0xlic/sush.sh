pub mod client;
pub mod transfer;

use std::collections::BTreeSet;
use std::time::Instant;

use crate::sftp::client::FileEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneSide {
    Local,
    Remote,
}

pub struct SftpPaneState {
    pub side: PaneSide,
    pub local_path: std::path::PathBuf,
    pub remote_path: String,
    pub local_all_entries: Vec<FileEntry>,
    pub remote_all_entries: Vec<FileEntry>,
    pub local_entries: Vec<FileEntry>,
    pub remote_entries: Vec<FileEntry>,
    pub local_list_state: ratatui::widgets::ListState,
    pub remote_list_state: ratatui::widgets::ListState,
    pub local_selection: BTreeSet<usize>,
    pub remote_selection: BTreeSet<usize>,
    pub local_selection_anchor: Option<usize>,
    pub remote_selection_anchor: Option<usize>,
    pub local_pending_anchor: Option<usize>,
    pub remote_pending_anchor: Option<usize>,
    pub local_last_space_at: Option<Instant>,
    pub remote_last_space_at: Option<Instant>,
    pub local_last_space_index: Option<usize>,
    pub remote_last_space_index: Option<usize>,
}

impl SftpPaneState {
    const DOUBLE_SPACE_WINDOW_MS: u128 = 500;

    pub fn new(remote_home: String) -> Self {
        let mut local_list_state = ratatui::widgets::ListState::default();
        local_list_state.select(Some(0));
        let mut remote_list_state = ratatui::widgets::ListState::default();
        remote_list_state.select(Some(0));
        Self {
            side: PaneSide::Remote,
            local_path: std::env::current_dir().unwrap_or_default(),
            remote_path: remote_home,
            local_all_entries: Vec::new(),
            remote_all_entries: Vec::new(),
            local_entries: Vec::new(),
            remote_entries: Vec::new(),
            local_list_state,
            remote_list_state,
            local_selection: BTreeSet::new(),
            remote_selection: BTreeSet::new(),
            local_selection_anchor: None,
            remote_selection_anchor: None,
            local_pending_anchor: None,
            remote_pending_anchor: None,
            local_last_space_at: None,
            remote_last_space_at: None,
            local_last_space_index: None,
            remote_last_space_index: None,
        }
    }

    pub fn selected_index(&self) -> usize {
        match self.side {
            PaneSide::Local => self.local_list_state.selected().unwrap_or(0),
            PaneSide::Remote => self.remote_list_state.selected().unwrap_or(0),
        }
    }

    pub fn active_list_state_mut(&mut self) -> &mut ratatui::widgets::ListState {
        match self.side {
            PaneSide::Local => &mut self.local_list_state,
            PaneSide::Remote => &mut self.remote_list_state,
        }
    }

    pub fn set_local_entries(&mut self, entries: Vec<FileEntry>) {
        self.local_all_entries = entries.clone();
        self.local_entries = entries;
        self.local_selection.clear();
        self.local_list_state
            .select((!self.local_entries.is_empty()).then_some(0));
    }

    pub fn set_remote_entries(&mut self, entries: Vec<FileEntry>) {
        self.remote_all_entries = entries.clone();
        self.remote_entries = entries;
        self.remote_selection.clear();
        self.remote_list_state
            .select((!self.remote_entries.is_empty()).then_some(0));
    }

    pub fn filter_active_entries(&mut self, query: &str) {
        match self.side {
            PaneSide::Local => {
                self.local_entries = filter_entries(&self.local_all_entries, query);
                self.local_selection.clear();
                self.local_list_state
                    .select((!self.local_entries.is_empty()).then_some(0));
            }
            PaneSide::Remote => {
                self.remote_entries = filter_entries(&self.remote_all_entries, query);
                self.remote_selection.clear();
                self.remote_list_state
                    .select((!self.remote_entries.is_empty()).then_some(0));
            }
        }
    }

    pub fn toggle_active_selection(&mut self) {
        let index = self.selected_index();
        match self.side {
            PaneSide::Local => {
                if self
                    .local_pending_anchor
                    .is_some_and(|pending| pending != index)
                {
                    self.local_selection_anchor = self.local_pending_anchor;
                    self.local_pending_anchor = None;
                }

                let previous_anchor = self.local_selection_anchor;
                let is_double_space = self
                    .local_last_space_at
                    .is_some_and(|last| last.elapsed().as_millis() < Self::DOUBLE_SPACE_WINDOW_MS)
                    && previous_anchor.is_some()
                    && previous_anchor != Some(index)
                    && self.local_last_space_index == Some(index);
                if is_double_space {
                    apply_range_selection(
                        &mut self.local_selection,
                        previous_anchor.unwrap(),
                        index,
                    );
                    self.local_pending_anchor = None;
                    self.local_last_space_at = Some(Instant::now());
                    self.local_last_space_index = Some(index);
                    return;
                }
                if !self.local_selection.insert(index) {
                    self.local_selection.remove(&index);
                }
                if previous_anchor.is_none() {
                    self.local_selection_anchor = Some(index);
                    self.local_pending_anchor = None;
                } else {
                    self.local_pending_anchor = Some(index);
                }
                self.local_last_space_at = Some(Instant::now());
                self.local_last_space_index = Some(index);
            }
            PaneSide::Remote => {
                if self
                    .remote_pending_anchor
                    .is_some_and(|pending| pending != index)
                {
                    self.remote_selection_anchor = self.remote_pending_anchor;
                    self.remote_pending_anchor = None;
                }

                let previous_anchor = self.remote_selection_anchor;
                let is_double_space = self
                    .remote_last_space_at
                    .is_some_and(|last| last.elapsed().as_millis() < Self::DOUBLE_SPACE_WINDOW_MS)
                    && previous_anchor.is_some()
                    && previous_anchor != Some(index)
                    && self.remote_last_space_index == Some(index);
                if is_double_space {
                    apply_range_selection(
                        &mut self.remote_selection,
                        previous_anchor.unwrap(),
                        index,
                    );
                    self.remote_pending_anchor = None;
                    self.remote_last_space_at = Some(Instant::now());
                    self.remote_last_space_index = Some(index);
                    return;
                }
                if !self.remote_selection.insert(index) {
                    self.remote_selection.remove(&index);
                }
                if previous_anchor.is_none() {
                    self.remote_selection_anchor = Some(index);
                    self.remote_pending_anchor = None;
                } else {
                    self.remote_pending_anchor = Some(index);
                }
                self.remote_last_space_at = Some(Instant::now());
                self.remote_last_space_index = Some(index);
            }
        }
    }

    pub fn clear_active_selection(&mut self) {
        match self.side {
            PaneSide::Local => {
                self.local_selection.clear();
                self.local_selection_anchor = None;
                self.local_pending_anchor = None;
                self.local_last_space_at = None;
                self.local_last_space_index = None;
            }
            PaneSide::Remote => {
                self.remote_selection.clear();
                self.remote_selection_anchor = None;
                self.remote_pending_anchor = None;
                self.remote_last_space_at = None;
                self.remote_last_space_index = None;
            }
        }
    }
}

fn filter_entries(entries: &[FileEntry], query: &str) -> Vec<FileEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return entries.to_vec();
    }

    use nucleo::{Config, Matcher, Utf32Str};
    use std::cmp::Reverse;

    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut needle_buf = Vec::new();
    let needle = Utf32Str::new(&query, &mut needle_buf);
    let mut scored: Vec<(usize, u32)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let haystack = entry.name.to_lowercase();
            let mut haystack_buf = Vec::new();
            let haystack = Utf32Str::new(&haystack, &mut haystack_buf);
            matcher
                .fuzzy_match(haystack, needle)
                .map(|score| (index, score as u32))
        })
        .collect();
    scored.sort_by_key(|&(_, score)| Reverse(score));
    scored
        .into_iter()
        .map(|(index, _)| entries[index].clone())
        .collect()
}

fn apply_range_selection(selection: &mut BTreeSet<usize>, anchor: usize, current: usize) {
    let start = anchor.min(current);
    let end = anchor.max(current);
    selection.extend(start..=end);
}
