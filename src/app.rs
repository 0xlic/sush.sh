use anyhow::Result;

use crate::config::host::{Host, HostSource};
use crate::config::store;

pub enum AppMode {
    Main,
    Ssh,
    Sftp,
}

pub struct App {
    pub mode: AppMode,
    pub hosts: Vec<Host>,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub selected_index: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Result<Self> {
        let hosts = store::load_hosts()?;
        let filtered_indices = (0..hosts.len()).collect();
        Ok(Self {
            mode: AppMode::Main,
            hosts,
            search_query: String::new(),
            filtered_indices,
            selected_index: 0,
            should_quit: false,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // TODO: 启动 TUI 事件循环
        Ok(())
    }
}
