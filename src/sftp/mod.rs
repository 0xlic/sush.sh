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
    pub list_state: ratatui::widgets::ListState,
}

impl SftpPaneState {
    pub fn new(remote_home: String) -> Self {
        let mut ls = ratatui::widgets::ListState::default();
        ls.select(Some(0));
        Self {
            side: PaneSide::Remote,
            local_path: std::env::current_dir().unwrap_or_default(),
            remote_path: remote_home,
            local_entries: Vec::new(),
            remote_entries: Vec::new(),
            list_state: ls,
        }
    }
}
