pub mod client;
pub mod transfer;

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
}
