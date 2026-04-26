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
    pub local_entries: Vec<FileEntry>,
    pub remote_entries: Vec<FileEntry>,
    pub local_list_state: ratatui::widgets::ListState,
    pub remote_list_state: ratatui::widgets::ListState,
    pub local_selection: BTreeSet<usize>,
    pub remote_selection: BTreeSet<usize>,
    pub local_selection_anchor: Option<usize>,
    pub remote_selection_anchor: Option<usize>,
    pub local_last_space_at: Option<Instant>,
    pub remote_last_space_at: Option<Instant>,
}

impl SftpPaneState {
    pub fn new(remote_home: String) -> Self {
        let mut local_list_state = ratatui::widgets::ListState::default();
        local_list_state.select(Some(0));
        let mut remote_list_state = ratatui::widgets::ListState::default();
        remote_list_state.select(Some(0));
        Self {
            side: PaneSide::Remote,
            local_path: std::env::current_dir().unwrap_or_default(),
            remote_path: remote_home,
            local_entries: Vec::new(),
            remote_entries: Vec::new(),
            local_list_state,
            remote_list_state,
            local_selection: BTreeSet::new(),
            remote_selection: BTreeSet::new(),
            local_selection_anchor: None,
            remote_selection_anchor: None,
            local_last_space_at: None,
            remote_last_space_at: None,
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

    pub fn toggle_active_selection(&mut self) {
        let index = self.selected_index();
        match self.side {
            PaneSide::Local => {
                if !self.local_selection.insert(index) {
                    self.local_selection.remove(&index);
                }
                self.local_selection_anchor = Some(index);
                self.local_last_space_at = Some(Instant::now());
            }
            PaneSide::Remote => {
                if !self.remote_selection.insert(index) {
                    self.remote_selection.remove(&index);
                }
                self.remote_selection_anchor = Some(index);
                self.remote_last_space_at = Some(Instant::now());
            }
        }
    }

    pub fn clear_active_selection(&mut self) {
        match self.side {
            PaneSide::Local => {
                self.local_selection.clear();
                self.local_selection_anchor = None;
                self.local_last_space_at = None;
            }
            PaneSide::Remote => {
                self.remote_selection.clear();
                self.remote_selection_anchor = None;
                self.remote_last_space_at = None;
            }
        }
    }
}
