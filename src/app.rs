use std::io::{Stdout, stdout};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;
use russh::ChannelMsg;

use crate::config::history::ConnectionHistory;
use crate::config::host::Host;
use crate::config::secrets::{SecretError, SecretKey, SecretKind, SecretStore, SystemSecretBackend};
use crate::config::store;
use crate::sftp::client::{SftpClient, list_local};
use crate::sftp::transfer::{TransferProgress, TransferState, download, upload};
use crate::sftp::{PaneSide, SftpPaneState};
use crate::ssh::auth;
use crate::ssh::session::{ActiveSession, try_key_auth};
use crate::ssh::terminal::TerminalEmulator;
use crate::tui::event::{AppEvent, EventBus};
use crate::tui::views::edit_view::{self, EditDraft, EditField};
use crate::tui::views::folder_view::{
    self, FolderFocus, FolderViewState, JumpState, SearchState, hosts_in_path, jump_candidates,
    level_1_paths, parent_path,
};
use crate::tui::views::import_view::{self, ImportViewState};
use crate::tui::views::{main_view, password_dialog::PasswordDialog, sftp_view, ssh_view};
use crate::tui::widgets::confirm_dialog::ConfirmDialog;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainFocus {
    HostList,
    Directory,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Main,
    Ssh,
    Sftp,
    Edit,
    ImportSshConfig,
    #[allow(dead_code)]
    FolderView,
}

struct PwdDialog {
    dialog: PasswordDialog,
    result: Option<Option<String>>,
}

enum LoopEvent {
    App(AppEvent),
    Ssh(Option<ChannelMsg>),
}

/// Transfer direction determines which pane is refreshed after completion.
#[derive(Clone, Copy)]
pub enum TransferDir {
    Download, // remote -> local, refresh local when done
    Upload,   // local -> remote, refresh remote when done
}

pub struct ActiveTransfer {
    pub verb: &'static str,
    pub dir: TransferDir,
    pub progress: TransferProgress,
    pub rx: tokio::sync::mpsc::Receiver<TransferProgress>,
    pub cancel: tokio_util::sync::CancellationToken,
    pub done_at: Option<Instant>,
    pub needs_refresh: bool, // true until the completion-triggered refresh runs
}

pub struct App {
    pub mode: AppMode,
    pub hosts: Vec<Host>,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub should_quit: bool,
    pub main_focus: MainFocus,
    main_focus_before_search: MainFocus,
    pub show_folder_sidebar: bool,
    pub list_state: ListState,
    pub trigger_connect: bool,
    pub trigger_sftp: bool,
    pub trigger_pane_enter: bool,
    pub trigger_download: bool,
    pub trigger_upload: bool,
    pub trigger_refresh_local: bool,
    pub trigger_refresh_remote: bool,
    pub trigger_ssh_resume: bool,
    pub trigger_ssh_to_sftp: bool,
    pub active_session: Option<ActiveSession>,
    pub sftp_client: Option<SftpClient>,
    pub sftp_pane: Option<SftpPaneState>,
    pub current_host_alias: Option<String>,
    pub active_transfer: Option<ActiveTransfer>,
    pub last_ctrl_c: Option<Instant>,
    pub ssh_last_size: Option<(u16, u16)>,
    pub terminal_emulator: Option<TerminalEmulator>,
    pub edit_draft: Option<EditDraft>,
    pub import_state: Option<ImportViewState>,
    pub confirm_delete: bool,
    pub import_prompted: bool,
    pub metadata: store::Metadata,
    pub connection_history: ConnectionHistory,
    pub hover_host_id: Option<String>,
    pub hover_since: Option<Instant>,
    pub probe_rx: Option<tokio::sync::oneshot::Receiver<bool>>,
    pub probe_result: Option<bool>,
    pub status_msg: Option<(String, Instant)>,
    pub show_import_prompt: bool,
    pub folder_view_state: Option<FolderViewState>,
    pub folder_host_indices: Vec<usize>,
    secret_store: SecretStore,
    pwd_dialog: Option<PwdDialog>,
}

impl App {
    pub fn new() -> Result<Self> {
        let store = store::load_store(&store::config_path())?;
        let hosts = store.hosts;
        let metadata = store.metadata;
        let import_prompted = metadata.import_prompted;
        let history_path = store::config_dir().join("history.toml");
        let connection_history = ConnectionHistory::load(history_path);

        let filtered_indices: Vec<usize> = (0..hosts.len()).collect();
        let mut list_state = ListState::default();
        if !filtered_indices.is_empty() {
            list_state.select(Some(0));
        }

        Ok(Self {
            mode: AppMode::Main,
            hosts,
            search_query: String::new(),
            filtered_indices,
            should_quit: false,
            main_focus: MainFocus::HostList,
            main_focus_before_search: MainFocus::HostList,
            show_folder_sidebar: false,
            list_state,
            trigger_connect: false,
            trigger_sftp: false,
            trigger_pane_enter: false,
            trigger_download: false,
            trigger_upload: false,
            trigger_refresh_local: false,
            trigger_refresh_remote: false,
            trigger_ssh_resume: false,
            trigger_ssh_to_sftp: false,
            active_session: None,
            sftp_client: None,
            sftp_pane: None,
            current_host_alias: None,
            active_transfer: None,
            last_ctrl_c: None,
            ssh_last_size: None,
            terminal_emulator: None,
            edit_draft: None,
            import_state: None,
            confirm_delete: false,
            import_prompted,
            metadata,
            connection_history,
            hover_host_id: None,
            hover_since: None,
            probe_rx: None,
            probe_result: None,
            status_msg: None,
            show_import_prompt: false,
            folder_view_state: None,
            folder_host_indices: vec![],
            secret_store: SecretStore::new(Box::new(SystemSecretBackend::new())),
            pwd_dialog: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;
        self.check_first_launch_import();
        let result = self.event_loop(&mut terminal).await;
        restore_terminal(&mut terminal)?;
        result
    }

    fn check_first_launch_import(&mut self) {
        if self.import_prompted || !self.hosts.is_empty() {
            return;
        }
        let has_ssh_hosts = crate::config::ssh_config::import_ssh_config()
            .map(|(h, _)| !h.is_empty())
            .unwrap_or(false);
        if has_ssh_hosts {
            self.show_import_prompt = true;
        }
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let mut bus = EventBus::new();

        while !self.should_quit {
            // Trigger SSH connection.
            if self.trigger_connect {
                self.trigger_connect = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let host = self.hosts[idx].clone();
                    if let Err(e) = self
                        .ssh_connect_and_takeover(terminal, &mut bus, &host)
                        .await
                    {
                        self.connection_history.record(&host.alias);
                        self.set_status(format!("Connection failed: {e}"));
                    }
                }
            }

            // Trigger SFTP connection.
            if self.trigger_sftp {
                self.trigger_sftp = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let host = self.hosts[idx].clone();
                    if let Err(e) = self.sftp_connect(terminal, &mut bus, &host).await {
                        self.connection_history.record(&host.alias);
                        self.set_status(format!("SFTP failed: {e}"));
                    }
                }
            }

            if self.trigger_ssh_to_sftp {
                self.trigger_ssh_to_sftp = false;
                if let Err(e) = self.switch_ssh_to_sftp(terminal).await {
                    eprintln!("[SFTP error: {e}]");
                    self.leave_ssh_mode(terminal).await?;
                }
            }

            // Trigger SFTP directory navigation.
            if self.trigger_pane_enter {
                self.trigger_pane_enter = false;
                if let Err(e) = self.pane_enter().await {
                    eprintln!("[Navigation error: {e}]");
                }
            }

            // Trigger download.
            if self.trigger_download {
                self.trigger_download = false;
                if let Err(e) = self.start_download().await {
                    eprintln!("[Download error: {e}]");
                }
            }

            // Trigger upload.
            if self.trigger_upload {
                self.trigger_upload = false;
                if let Err(e) = self.start_upload().await {
                    eprintln!("[Upload error: {e}]");
                }
            }

            // Refresh the local directory after a transfer completes.
            if self.trigger_refresh_local {
                self.trigger_refresh_local = false;
                if let Some(pane) = &mut self.sftp_pane
                    && let Ok(entries) = list_local(&pane.local_path)
                {
                    pane.local_entries = entries;
                }
            }

            // Refresh the remote directory after a transfer completes.
            if self.trigger_refresh_remote {
                self.trigger_refresh_remote = false;
                if let Some(pane) = &self.sftp_pane {
                    let path = pane.remote_path.clone();
                    if let Some(client) = &self.sftp_client
                        && let Ok(entries) = client.list_dir(&path).await
                        && let Some(pane) = &mut self.sftp_pane
                    {
                        pane.remote_entries = entries;
                    }
                }
            }

            // Trigger SSH resume when switching back from SFTP.
            if self.trigger_ssh_resume {
                self.trigger_ssh_resume = false;
                if let Err(e) = self.resume_ssh_from_sftp().await {
                    eprintln!("[SSH error: {e}]");
                    self.leave_ssh_mode(terminal).await?;
                }
            }

            // Poll transfer progress.
            if let Some(tr) = &mut self.active_transfer {
                while let Ok(prog) = tr.rx.try_recv() {
                    tr.progress = prog;
                }
                let done = matches!(
                    tr.progress.state,
                    TransferState::Completed | TransferState::Failed(_) | TransferState::Cancelled
                );
                if done {
                    // Trigger the pane refresh only once when completion is first observed.
                    if tr.needs_refresh {
                        tr.needs_refresh = false;
                        match tr.dir {
                            TransferDir::Download => self.trigger_refresh_local = true,
                            TransferDir::Upload => self.trigger_refresh_remote = true,
                        }
                    }
                    if tr.done_at.is_none() {
                        tr.done_at = Some(Instant::now());
                    }
                    if tr
                        .done_at
                        .map(|t| t.elapsed().as_secs() >= 3)
                        .unwrap_or(false)
                    {
                        self.active_transfer = None;
                    }
                }
            }

            self.render(terminal)?;

            let next = if self.mode == AppMode::Ssh {
                if let Some(session) = self.active_session.as_mut().filter(|s| s.has_pty()) {
                    tokio::select! {
                        ev = bus.next() => ev.map(LoopEvent::App),
                        msg = session.wait_channel_msg() => Some(LoopEvent::Ssh(msg?)),
                    }
                } else {
                    bus.next().await.map(LoopEvent::App)
                }
            } else {
                bus.next().await.map(LoopEvent::App)
            };

            match next {
                Some(LoopEvent::App(ev)) => self.handle_event(ev, terminal).await?,
                Some(LoopEvent::Ssh(msg)) => self.handle_ssh_channel_msg(terminal, msg).await?,
                None => break,
            }
        }
        bus.shutdown();
        Ok(())
    }

    async fn handle_event(
        &mut self,
        ev: AppEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        match ev {
            AppEvent::Input(data) => self.handle_input(data).await,
            AppEvent::Tick => self.handle_tick(terminal).await,
        }
    }

    async fn handle_tick(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        if self.mode == AppMode::Ssh {
            self.sync_ssh_size().await?;
            if self
                .active_session
                .as_ref()
                .is_some_and(|session| !session.has_pty())
            {
                self.leave_ssh_mode(terminal).await?;
            }
        }
        // Clear status message after 4 seconds.
        if let Some((_, since)) = &self.status_msg
            && since.elapsed() >= std::time::Duration::from_secs(4)
        {
            self.status_msg = None;
        }
        // Connectivity probe — only in Main mode.
        if self.mode == AppMode::Main {
            // Collect completed probe result.
            if let Some(rx) = &mut self.probe_rx
                && let Ok(result) = rx.try_recv()
            {
                self.probe_result = Some(result);
                self.probe_rx = None;
            }
            // Start probe after 1s hover, if none is running or completed.
            if self.probe_rx.is_none()
                && self.probe_result.is_none()
                && self
                    .hover_since
                    .is_some_and(|s| s.elapsed() >= std::time::Duration::from_secs(1))
                && let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
            {
                let hostname = self.hosts[idx].hostname.clone();
                let port = self.hosts[idx].port;
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    let ok = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        tokio::net::TcpStream::connect((hostname.as_str(), port)),
                    )
                    .await
                    .map(|r| r.is_ok())
                    .unwrap_or(false);
                    let _ = tx.send(ok);
                });
                self.probe_rx = Some(rx);
            }
        }
        Ok(())
    }

    async fn handle_input(&mut self, data: Vec<u8>) -> Result<()> {
        if self.pwd_dialog.is_some() {
            for key in decode_tui_keys(&data) {
                self.handle_pwd_key(key);
            }
            return Ok(());
        }

        match self.mode {
            AppMode::Main | AppMode::Sftp => {
                for key in decode_tui_keys(&data) {
                    self.handle_key(key);
                }
                Ok(())
            }
            AppMode::Ssh => self.handle_ssh_input(data).await,
            AppMode::Edit => {
                for key in decode_tui_keys(&data) {
                    self.handle_edit_input(key);
                }
                Ok(())
            }
            AppMode::ImportSshConfig => {
                for key in decode_tui_keys(&data) {
                    self.handle_import_input(key);
                }
                Ok(())
            }
            AppMode::FolderView => {
                for key in decode_tui_keys(&data) {
                    self.handle_folder_input(key);
                }
                Ok(())
            }
        }
    }

    fn handle_key(&mut self, k: KeyEvent) {
        if self.pwd_dialog.is_some() {
            self.handle_pwd_key(k);
            return;
        }
        if self.confirm_delete {
            self.handle_confirm_delete(k);
            return;
        }
        if self.show_import_prompt {
            self.handle_import_prompt_key(k);
            return;
        }
        match self.mode {
            AppMode::Main => self.handle_main_key(k),
            AppMode::Sftp => self.handle_sftp_key(k),
            AppMode::Ssh | AppMode::Edit | AppMode::ImportSshConfig | AppMode::FolderView => {}
        }
    }

    fn handle_pwd_key(&mut self, k: KeyEvent) {
        let Some(pwd) = self.pwd_dialog.as_mut() else {
            return;
        };
        match k.code {
            KeyCode::Enter => {
                let input = pwd.dialog.input.clone();
                pwd.result = Some(Some(input));
            }
            KeyCode::Esc => {
                pwd.result = Some(None);
            }
            KeyCode::Backspace => {
                pwd.dialog.input.pop();
            }
            KeyCode::Char(c) => {
                pwd.dialog.input.push(c);
            }
            _ => {}
        }
    }

    pub fn folder_search_prefix(&self) -> Option<&str> {
        self.show_folder_sidebar
            .then(|| {
                self.folder_view_state
                    .as_ref()
                    .map(FolderViewState::selected_path)
            })
            .flatten()
    }

    fn handle_main_key(&mut self, k: KeyEvent) {
        if self.main_focus == MainFocus::Directory
            && self
                .folder_view_state
                .as_ref()
                .and_then(|fv| fv.jump.as_ref())
                .is_some()
        {
            self.handle_folder_jump_key(k);
            return;
        }

        match self.main_focus {
            MainFocus::HostList => self.handle_main_key_hostlist(k),
            MainFocus::Directory => self.handle_main_key_directory(k),
            MainFocus::Search => self.handle_main_key_search(k),
        }
    }

    fn handle_main_key_hostlist(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) if self.show_folder_sidebar => {
                self.main_focus = MainFocus::Directory;
            }
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.main_focus_before_search = MainFocus::HostList;
                self.main_focus = MainFocus::Search;
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                self.trigger_connect = true;
            }
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                self.trigger_sftp = true;
            }
            (KeyCode::Up, _) => self.select_previous(),
            (KeyCode::Down, _) => self.select_next(),
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.should_quit = true;
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                self.edit_draft = Some(EditDraft::new_host());
                self.mode = AppMode::Edit;
            }
            (KeyCode::Char('e'), KeyModifiers::NONE) => {
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    self.edit_draft = Some(EditDraft::from_host(&self.hosts[idx]));
                    self.mode = AppMode::Edit;
                }
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) if !self.filtered_indices.is_empty() => {
                self.confirm_delete = true;
            }
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                self.open_import_view();
            }
            (KeyCode::Char('f'), KeyModifiers::NONE) => {
                self.toggle_folder_sidebar();
            }
            _ => {}
        }
    }

    fn handle_main_key_directory(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) => {
                self.main_focus = MainFocus::HostList;
            }
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.main_focus_before_search = MainFocus::Directory;
                self.main_focus = MainFocus::Search;
            }
            (KeyCode::Char('f'), KeyModifiers::NONE) => {
                self.toggle_folder_sidebar();
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                if let Some(fv) = self.folder_view_state.as_mut() {
                    let candidates = jump_candidates("", &fv.tree);
                    fv.jump = Some(JumpState {
                        input: String::new(),
                        candidates,
                        sel: 0,
                    });
                }
            }
            (KeyCode::Enter, _) | (KeyCode::Right, _) => {
                self.directory_enter();
            }
            (KeyCode::Backspace, _) | (KeyCode::Left, _) => {
                self.directory_back();
            }
            (KeyCode::Up, _) => {
                self.directory_select_previous();
            }
            (KeyCode::Down, _) => {
                self.directory_select_next();
            }
            _ => {}
        }
    }

    fn handle_main_key_search(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) | (KeyCode::Esc, _) | (KeyCode::Enter, _) => {
                self.main_focus = self.main_focus_before_search;
            }
            (KeyCode::Up, _) => self.select_previous(),
            (KeyCode::Down, _) => self.select_next(),
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.search_query.clear();
                self.apply_search();
            }
            (KeyCode::Backspace, _) => {
                self.search_query.pop();
                self.apply_search();
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                self.search_query.push(c);
                self.apply_search();
            }
            _ => {}
        }
    }

    fn handle_sftp_key(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                let now = Instant::now();
                if let Some(last) = self.last_ctrl_c
                    && last.elapsed().as_millis() < 500
                {
                    self.exit_sftp();
                    return;
                }
                self.last_ctrl_c = Some(now);
                if let Some(tr) = &self.active_transfer {
                    tr.cancel.cancel();
                }
            }
            (KeyCode::Char('q'), KeyModifiers::NONE) => self.exit_sftp(),
            _ if is_switch_key(k) => self.trigger_ssh_resume = true,
            (KeyCode::Char('d'), KeyModifiers::NONE) => self.trigger_download = true,
            (KeyCode::Char('u'), KeyModifiers::NONE) => self.trigger_upload = true,
            // D delete and r rename are not implemented yet.
            (KeyCode::Tab, _) => self.toggle_pane(),
            (KeyCode::Up, _) => self.pane_select_prev(),
            (KeyCode::Down, _) => self.pane_select_next(),
            (KeyCode::Enter, _) => self.trigger_pane_enter = true,
            _ => {}
        }
    }

    fn exit_sftp(&mut self) {
        self.sftp_client = None;
        self.sftp_pane = None;
        self.active_transfer = None;
        self.current_host_alias = None;
        self.mode = AppMode::Main;
    }

    fn toggle_pane(&mut self) {
        if let Some(pane) = &mut self.sftp_pane {
            pane.side = match pane.side {
                PaneSide::Local => PaneSide::Remote,
                PaneSide::Remote => PaneSide::Local,
            };
            pane.list_state.select(Some(0));
        }
    }

    fn pane_select_prev(&mut self) {
        if let Some(pane) = &mut self.sftp_pane {
            let i = pane.list_state.selected().unwrap_or(0);
            pane.list_state.select(Some(i.saturating_sub(1)));
        }
    }

    fn pane_select_next(&mut self) {
        if let Some(pane) = &mut self.sftp_pane {
            let len = match pane.side {
                PaneSide::Local => pane.local_entries.len(),
                PaneSide::Remote => pane.remote_entries.len(),
            };
            if len == 0 {
                return;
            }
            let i = pane.list_state.selected().unwrap_or(0);
            pane.list_state.select(Some((i + 1).min(len - 1)));
        }
    }

    fn apply_search(&mut self) {
        let scoped_indices = if self.show_folder_sidebar {
            self.folder_search_prefix()
                .map(|path| hosts_in_path(path, &self.hosts))
                .unwrap_or_default()
        } else {
            (0..self.hosts.len()).collect()
        };

        let scoped_hosts: Vec<Host> = scoped_indices
            .iter()
            .map(|&idx| self.hosts[idx].clone())
            .collect();
        let result = crate::utils::fuzzy::search(
            &self.search_query,
            &scoped_hosts,
            &self.connection_history,
        );
        self.filtered_indices = result.into_iter().map(|i| scoped_indices[i]).collect();
        let sel = if self.filtered_indices.is_empty() {
            None
        } else {
            Some(0)
        };
        self.list_state.select(sel);
        self.on_selection_changed();
    }

    fn toggle_folder_sidebar(&mut self) {
        if self.show_folder_sidebar {
            self.show_folder_sidebar = false;
            if self.main_focus == MainFocus::Directory {
                self.main_focus = MainFocus::HostList;
            }
            if self.main_focus_before_search == MainFocus::Directory {
                self.main_focus_before_search = MainFocus::HostList;
            }
        } else {
            if self.folder_view_state.is_none() {
                self.folder_view_state = Some(FolderViewState::new(&self.hosts));
            }
            if let Some(fv) = self.folder_view_state.as_mut() {
                fv.jump = None;
            }
            self.show_folder_sidebar = true;
            self.main_focus = MainFocus::Directory;
        }
        self.apply_search();
    }

    fn directory_select_previous(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            if fv.sel_a > 0 {
                fv.sel_a -= 1;
            }
            fv.update_col_b();
        }
        self.apply_search();
    }

    fn directory_select_next(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            if fv.sel_a + 1 < fv.col_a.len() {
                fv.sel_a += 1;
            }
            fv.update_col_b();
        }
        self.apply_search();
    }

    fn directory_enter(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            let selected = fv.selected_path().to_string();
            let children = fv
                .tree
                .get(&selected)
                .map(|node| node.children.clone())
                .unwrap_or_default();
            if !children.is_empty() {
                fv.col_a = children;
                fv.sel_a = 0;
                fv.depth += 1;
                fv.update_col_b();
            }
        }
        self.apply_search();
    }

    fn directory_back(&mut self) {
        if let Some(fv) = self.folder_view_state.as_mut() {
            let current = fv.selected_path().to_string();
            if current == "/" {
                return;
            }

            let parent = parent_path(&current);
            if parent == "/" {
                fv.col_a = level_1_paths(&fv.tree);
                fv.sel_a = fv.col_a.iter().position(|path| path == "/").unwrap_or(0);
                fv.depth = 0;
            } else {
                let grandparent = parent_path(&parent);
                fv.col_a = fv
                    .tree
                    .get(&grandparent)
                    .map(|node| node.children.clone())
                    .unwrap_or_else(|| level_1_paths(&fv.tree));
                fv.sel_a = fv
                    .col_a
                    .iter()
                    .position(|path| path == &parent)
                    .unwrap_or(0);
                fv.depth = fv.depth.saturating_sub(1);
            }
            fv.update_col_b();
        }
        self.apply_search();
    }

    fn set_status(&mut self, msg: String) {
        self.status_msg = Some((msg, Instant::now()));
    }

    fn reset_probe(&mut self) {
        self.hover_host_id = None;
        self.hover_since = None;
        self.probe_rx = None;
        self.probe_result = None;
    }

    fn on_selection_changed(&mut self) {
        let new_id = self
            .filtered_indices
            .get(self.list_state.selected().unwrap_or(0))
            .map(|&i| self.hosts[i].id.clone());

        if self.hover_host_id != new_id {
            self.reset_probe();
            self.hover_host_id = new_id;
            self.hover_since = Some(Instant::now());
        }
    }

    fn open_import_view(&mut self) {
        let ssh_hosts = crate::config::ssh_config::import_ssh_config()
            .map(|(h, _)| h)
            .unwrap_or_default();
        if ssh_hosts.is_empty() {
            return;
        }
        self.import_state = Some(ImportViewState::new(ssh_hosts, &self.hosts));
        self.mode = AppMode::ImportSshConfig;
    }

    fn save_hosts_to_disk(&self) {
        let mut metadata = self.metadata.clone();
        metadata.import_prompted = self.import_prompted;
        let _ = store::save_store(
            &store::config_path(),
            &store::HostStore {
                metadata,
                hosts: self.hosts.clone(),
            },
        );
    }

    fn password_prompt_title(&mut self, user: &str, hostname: &str, account: &str) -> String {
        let title = format!("Password for {}@{}: ", user, hostname);
        let Some(failure) = self.metadata.take_secret_failure(account) else {
            return title;
        };

        self.save_hosts_to_disk();
        format!("Password was not saved last time: {}\n{title}", failure.reason)
    }

    fn record_secret_save_failure(&mut self, account: String, error: &SecretError) {
        let mut reason = error.user_message().to_string();
        if cfg!(target_os = "linux") && matches!(error, SecretError::Unavailable(_)) {
            reason.push_str(
                ". Install and unlock a Secret Service provider such as gnome-keyring or kwallet.",
            );
        }

        self.metadata.upsert_secret_failure(account, reason);
        self.save_hosts_to_disk();
    }

    fn clear_secret_save_failure(&mut self, account: &str) {
        if self.metadata.take_secret_failure(account).is_some() {
            self.save_hosts_to_disk();
        }
    }

    fn key_passphrase_prompt_title(&mut self, path: &str, account: &str) -> String {
        let title = format!("Key passphrase ({}): ", path);
        let Some(failure) = self.metadata.take_secret_failure(account) else {
            return title;
        };

        self.save_hosts_to_disk();
        format!("Password was not saved last time: {}\n{title}", failure.reason)
    }

    fn handle_edit_input(&mut self, k: KeyEvent) {
        let Some(draft) = self.edit_draft.as_mut() else {
            return;
        };

        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                self.edit_draft = None;
                self.mode = AppMode::Main;
                self.on_selection_changed();
                return;
            }
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                self.try_save_edit();
                return;
            }
            (KeyCode::Tab, _) | (KeyCode::Down, _) => {
                if draft.focused_field == EditField::Tags {
                    if !draft.tags.candidates.is_empty() {
                        draft.tags.handle_down();
                        return;
                    }
                    draft.tags.commit_pending();
                }
                draft.focused_field = draft.focused_field.next();
                return;
            }
            (KeyCode::BackTab, _) | (KeyCode::Up, _) => {
                if draft.focused_field == EditField::Tags {
                    if !draft.tags.candidates.is_empty() {
                        draft.tags.handle_up();
                        return;
                    }
                    draft.tags.commit_pending();
                }
                draft.focused_field = draft.focused_field.prev();
                return;
            }
            _ => {}
        }

        if draft.focused_field == EditField::Tags {
            let all_tags: Vec<String> = {
                let mut t: Vec<String> = self
                    .hosts
                    .iter()
                    .flat_map(|h| h.tags.iter().cloned())
                    .collect();
                t.sort();
                t.dedup();
                t
            };
            let draft = self.edit_draft.as_mut().unwrap();
            match (k.code, k.modifiers) {
                (KeyCode::Left, _) => draft.tags.handle_left(),
                (KeyCode::Right, _) => draft.tags.handle_right(),
                (KeyCode::Backspace, _) => draft.tags.handle_backspace(&all_tags),
                (KeyCode::Enter, _) | (KeyCode::Tab, _) => draft.tags.confirm_input(),
                (KeyCode::Esc, _) => draft.tags.cancel_input(),
                (KeyCode::Char(c), KeyModifiers::NONE)
                | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                    draft.tags.handle_char(c, &all_tags);
                }
                _ => {}
            }
            return;
        }

        let draft = self.edit_draft.as_mut().unwrap();
        // Text field handling
        match k.code {
            KeyCode::Backspace => {
                if let Some(v) = draft.active_text_mut() {
                    v.pop();
                }
            }
            KeyCode::Char(c)
                if k.modifiers == KeyModifiers::NONE || k.modifiers == KeyModifiers::SHIFT =>
            {
                if draft.focused_field == EditField::Port && !c.is_ascii_digit() {
                    return;
                }
                if let Some(v) = draft.active_text_mut() {
                    v.push(c);
                }
            }
            _ => {}
        }
    }

    fn try_save_edit(&mut self) {
        let Some(draft) = self.edit_draft.as_mut() else {
            return;
        };
        draft.error = None;

        if draft.is_new {
            let alias = draft.alias.trim().to_string();
            if self.hosts.iter().any(|h| h.alias == alias) {
                draft.error = Some("Alias already exists".into());
                return;
            }
        }

        match edit_view::validate(draft) {
            Err(e) => {
                draft.error = Some(e);
            }
            Ok(()) => {
                let host = edit_view::build_host(draft);
                if draft.is_new {
                    self.hosts.push(host);
                } else {
                    let id = draft.original_id.clone().unwrap_or_default();
                    if let Some(h) = self.hosts.iter_mut().find(|h| h.id == id) {
                        *h = host;
                    }
                }
                self.apply_search();
                self.save_hosts_to_disk();
                self.edit_draft = None;
                self.mode = AppMode::Main;
            }
        }
    }

    fn handle_confirm_delete(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_delete = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let id = self.hosts[idx].id.clone();
                    self.hosts.retain(|h| h.id != id);
                    self.apply_search();
                    self.save_hosts_to_disk();
                }
            }
            _ => {
                self.confirm_delete = false;
            }
        }
    }

    fn handle_import_prompt_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.show_import_prompt = false;
                self.open_import_view();
            }
            _ => {
                self.show_import_prompt = false;
                self.import_prompted = true;
                self.save_hosts_to_disk();
            }
        }
    }

    fn handle_folder_input(&mut self, k: KeyEvent) {
        let Some(fv) = self.folder_view_state.as_mut() else {
            return;
        };

        // Jump overlay takes priority
        if fv.jump.is_some() {
            self.handle_folder_jump_key(k);
            return;
        }

        // Search bar takes priority
        if fv.search.is_some() {
            self.handle_folder_search_key(k);
            return;
        }

        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                self.folder_view_state = None;
                self.mode = AppMode::Main;
            }

            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                let all_cands = jump_candidates("", &fv.tree);
                fv.jump = Some(JumpState {
                    input: String::new(),
                    candidates: all_cands,
                    sel: 0,
                });
            }

            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                let scope = fv.focused_path().to_string();
                fv.search = Some(SearchState {
                    scope_path: scope,
                    query: String::new(),
                });
            }

            (KeyCode::Up, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA => {
                        if fv.sel_a > 0 {
                            fv.sel_a -= 1;
                        }
                        fv.update_col_b();
                    }
                    FolderFocus::DirB => {
                        if fv.sel_b > 0 {
                            fv.sel_b -= 1;
                        }
                    }
                    FolderFocus::Hosts => {
                        if fv.host_sel > 0 {
                            fv.host_sel -= 1;
                        }
                    }
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Down, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA => {
                        if fv.sel_a + 1 < fv.col_a.len() {
                            fv.sel_a += 1;
                        }
                        fv.update_col_b();
                    }
                    FolderFocus::DirB => {
                        if fv.sel_b + 1 < fv.col_b.len() {
                            fv.sel_b += 1;
                        }
                    }
                    FolderFocus::Hosts => {
                        let max = self.folder_host_indices.len().saturating_sub(1);
                        let fv = self.folder_view_state.as_mut().unwrap();
                        if fv.host_sel < max {
                            fv.host_sel += 1;
                        }
                    }
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Enter, _) | (KeyCode::Right, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA => {
                        if !fv.col_b.is_empty() {
                            let new_col_a = fv.col_b.clone();
                            fv.col_a = new_col_a;
                            fv.sel_a = 0;
                            fv.depth += 1;
                            fv.update_col_b();
                        } else {
                            fv.focus = FolderFocus::Hosts;
                        }
                    }
                    FolderFocus::DirB => {
                        let children = fv
                            .tree
                            .get(fv.col_b.get(fv.sel_b).map(|s| s.as_str()).unwrap_or("/"))
                            .map(|n| n.children.clone())
                            .unwrap_or_default();
                        if !children.is_empty() {
                            fv.col_a = fv.col_b.clone();
                            fv.sel_a = fv.sel_b;
                            fv.depth += 1;
                            fv.update_col_b();
                            fv.focus = FolderFocus::DirA;
                        } else {
                            fv.focus = FolderFocus::Hosts;
                        }
                    }
                    FolderFocus::Hosts => {
                        let host_sel = fv.host_sel;
                        if let Some(&idx) = self.folder_host_indices.get(host_sel) {
                            self.folder_view_state = None;
                            self.mode = AppMode::Main;
                            self.trigger_connect = true;
                            if let Some(pos) = self.filtered_indices.iter().position(|&i| i == idx)
                            {
                                self.list_state.select(Some(pos));
                            }
                        }
                    }
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Backspace, _) | (KeyCode::Left, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                match fv.focus {
                    FolderFocus::DirA if fv.depth > 0 => {
                        let current_first = fv.col_a.first().cloned().unwrap_or_else(|| "/".into());
                        let parent = parent_path(&current_first);
                        let grandparent = parent_path(&parent);
                        fv.col_b = fv.col_a.clone();
                        fv.sel_b = fv.sel_a;
                        fv.col_a = fv
                            .tree
                            .get(&grandparent)
                            .map(|n| n.children.clone())
                            .unwrap_or_else(|| level_1_paths(&fv.tree));
                        fv.sel_a = fv.col_a.iter().position(|p| p == &parent).unwrap_or(0);
                        fv.depth -= 1;
                        fv.focus = FolderFocus::DirA;
                    }
                    FolderFocus::DirB | FolderFocus::Hosts => {
                        fv.focus = FolderFocus::DirA;
                    }
                    _ => {}
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Tab, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                fv.focus = match fv.focus {
                    FolderFocus::DirA => {
                        if !fv.col_b.is_empty() {
                            FolderFocus::DirB
                        } else {
                            FolderFocus::Hosts
                        }
                    }
                    FolderFocus::DirB => FolderFocus::Hosts,
                    FolderFocus::Hosts => FolderFocus::DirA,
                };
            }

            (KeyCode::BackTab, _) => {
                let fv = self.folder_view_state.as_mut().unwrap();
                fv.focus = match fv.focus {
                    FolderFocus::DirA => FolderFocus::Hosts,
                    FolderFocus::DirB => FolderFocus::DirA,
                    FolderFocus::Hosts => {
                        if !fv.col_b.is_empty() {
                            FolderFocus::DirB
                        } else {
                            FolderFocus::DirA
                        }
                    }
                };
            }

            (KeyCode::Char('s'), KeyModifiers::NONE)
                if matches!(
                    self.folder_view_state.as_ref().map(|f| f.focus),
                    Some(FolderFocus::Hosts)
                ) =>
            {
                let host_sel = self.folder_view_state.as_ref().unwrap().host_sel;
                if let Some(&idx) = self.folder_host_indices.get(host_sel) {
                    self.folder_view_state = None;
                    self.mode = AppMode::Main;
                    self.trigger_sftp = true;
                    if let Some(pos) = self.filtered_indices.iter().position(|&i| i == idx) {
                        self.list_state.select(Some(pos));
                    }
                }
            }

            _ => {}
        }
    }

    fn refresh_folder_hosts(&mut self) {
        if let Some(fv) = &self.folder_view_state {
            self.folder_host_indices = hosts_in_path(fv.focused_path(), &self.hosts);
        }
    }

    fn handle_folder_jump_key(&mut self, k: KeyEvent) {
        let Some(fv) = self.folder_view_state.as_mut() else {
            return;
        };

        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                fv.jump = None;
            }

            (KeyCode::Backspace, _) => {
                if let Some(jump) = fv.jump.as_mut() {
                    jump.input.pop();
                    let q = jump.input.clone();
                    jump.candidates = jump_candidates(&q, &fv.tree);
                    jump.sel = 0;
                }
            }

            (KeyCode::Up, _) => {
                if let Some(jump) = fv.jump.as_mut()
                    && jump.sel > 0
                {
                    jump.sel -= 1;
                }
            }

            (KeyCode::Down, _) => {
                if let Some(jump) = fv.jump.as_mut()
                    && !jump.candidates.is_empty()
                {
                    jump.sel = (jump.sel + 1).min(jump.candidates.len() - 1);
                }
            }

            (KeyCode::Tab, _) => {
                // Path completion: fill input with highlighted candidate
                let candidate = fv
                    .jump
                    .as_ref()
                    .and_then(|j| j.candidates.get(j.sel))
                    .cloned();
                if let Some(candidate) = candidate
                    && let Some(jump) = fv.jump.as_mut()
                {
                    jump.input = candidate.clone();
                    jump.candidates = jump_candidates(&candidate, &fv.tree);
                    jump.sel = 0;
                }
            }

            (KeyCode::Enter, _) => {
                // Clone target before re-borrowing fv to avoid borrow conflict
                let target = fv
                    .jump
                    .as_ref()
                    .and_then(|j| j.candidates.get(j.sel))
                    .cloned();
                if let Some(target) = target {
                    fv.jump_to(&target);
                    if self.mode == AppMode::Main {
                        self.apply_search();
                    } else {
                        self.refresh_folder_hosts();
                    }
                } else {
                    if let Some(fv) = self.folder_view_state.as_mut() {
                        fv.jump = None;
                    }
                }
            }

            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                if let Some(jump) = fv.jump.as_mut() {
                    jump.input.push(c);
                    let q = jump.input.clone();
                    jump.candidates = jump_candidates(&q, &fv.tree);
                    jump.sel = 0;
                }
            }

            _ => {}
        }
    }

    fn handle_folder_search_key(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Esc, _) => {
                if let Some(fv) = self.folder_view_state.as_mut() {
                    fv.search = None;
                }
                self.refresh_folder_hosts();
            }

            (KeyCode::Enter, _) => {
                if let Some(fv) = self.folder_view_state.as_mut() {
                    fv.search = None;
                }
                // Keep current filtered results
            }

            (KeyCode::Backspace, _) => {
                if let Some(fv) = self.folder_view_state.as_mut()
                    && let Some(search) = fv.search.as_mut()
                {
                    search.query.pop();
                }
                self.apply_scoped_search();
            }

            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                if let Some(fv) = self.folder_view_state.as_mut()
                    && let Some(search) = fv.search.as_mut()
                {
                    search.query.push(c);
                }
                self.apply_scoped_search();
            }

            _ => {}
        }
    }

    fn apply_scoped_search(&mut self) {
        let Some(fv) = &self.folder_view_state else {
            return;
        };
        let Some(search) = &fv.search else {
            self.refresh_folder_hosts();
            return;
        };

        let scope = search.scope_path.clone();
        let query = search.query.clone();

        let path_indices = hosts_in_path(&scope, &self.hosts);

        if query.is_empty() {
            self.folder_host_indices = path_indices;
            return;
        }

        // Build a scoped sub-list, fuzzy-search within it, map results back to original indices
        let scoped: Vec<(usize, crate::config::host::Host)> = path_indices
            .iter()
            .map(|&i| (i, self.hosts[i].clone()))
            .collect();
        let sub_hosts: Vec<crate::config::host::Host> =
            scoped.iter().map(|(_, h)| h.clone()).collect();
        let fuzzy_results =
            crate::utils::fuzzy::search(&query, &sub_hosts, &self.connection_history);
        self.folder_host_indices = fuzzy_results.into_iter().map(|fi| scoped[fi].0).collect();
    }

    fn handle_import_input(&mut self, k: KeyEvent) {
        let Some(state) = self.import_state.as_mut() else {
            return;
        };
        match k.code {
            KeyCode::Esc => {
                self.import_state = None;
                self.import_prompted = true;
                self.mode = AppMode::Main;
                self.save_hosts_to_disk();
                self.on_selection_changed();
            }
            KeyCode::Enter => {
                let selected = state.selected_hosts();
                for h in selected {
                    if !self.hosts.iter().any(|e| e.id == h.id) {
                        self.hosts.push(h);
                    }
                }
                self.apply_search();
                self.import_prompted = true;
                self.save_hosts_to_disk();
                self.import_state = None;
                self.mode = AppMode::Main;
            }
            KeyCode::Char(' ') => state.toggle_selected(),
            KeyCode::Char('a') => state.toggle_all(),
            KeyCode::Up => state.move_up(),
            KeyCode::Down => state.move_down(),
            _ => {}
        }
    }

    fn select_next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let next = (i + 1).min(self.filtered_indices.len() - 1);
        self.list_state.select(Some(next));
        self.on_selection_changed();
    }

    fn select_previous(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        let prev = i.saturating_sub(1);
        self.list_state.select(Some(prev));
        self.on_selection_changed();
    }

    async fn sftp_connect(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        host: &Host,
    ) -> Result<()> {
        let session = self.connect_with_prompt(terminal, bus, host).await?;
        let client = SftpClient::open(&session).await?;
        let home = client.home_dir().await;
        let remote_entries = client.list_dir(&home).await.unwrap_or_default();
        let local_path = std::env::current_dir().unwrap_or_default();
        let local_entries = list_local(&local_path).unwrap_or_default();

        let mut pane = SftpPaneState::new(home);
        pane.remote_entries = remote_entries;
        pane.local_entries = local_entries;

        self.sftp_client = Some(client);
        self.sftp_pane = Some(pane);
        self.current_host_alias = Some(host.alias.clone());
        self.active_session = Some(session);
        self.mode = AppMode::Sftp;
        self.connection_history.record(&host.alias);
        Ok(())
    }

    async fn pane_enter(&mut self) -> Result<()> {
        let Some(pane) = &mut self.sftp_pane else {
            return Ok(());
        };
        let idx = pane.list_state.selected().unwrap_or(0);

        match pane.side {
            PaneSide::Remote => {
                let entry = pane.remote_entries.get(idx).cloned();
                if let Some(e) = entry {
                    if !e.is_dir {
                        return Ok(());
                    }
                    let new_path = if e.name == ".." {
                        let p = std::path::Path::new(&pane.remote_path);
                        p.parent()
                            .map(|pp| pp.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "/".into())
                    } else if e.name == "." {
                        pane.remote_path.clone()
                    } else {
                        format!("{}/{}", pane.remote_path.trim_end_matches('/'), e.name)
                    };
                    if let Some(client) = &self.sftp_client {
                        match client.list_dir(&new_path).await {
                            Ok(entries) => {
                                let pane = self.sftp_pane.as_mut().unwrap();
                                pane.remote_path = new_path;
                                pane.remote_entries = entries;
                                pane.list_state.select(Some(0));
                            }
                            Err(e) => eprintln!("[List directory failed: {e}]"),
                        }
                    }
                }
            }
            PaneSide::Local => {
                let entry = pane.local_entries.get(idx).cloned();
                if let Some(e) = entry {
                    if !e.is_dir {
                        return Ok(());
                    }
                    let new_path = if e.name == ".." {
                        pane.local_path
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| pane.local_path.clone())
                    } else {
                        pane.local_path.join(&e.name)
                    };
                    match list_local(&new_path) {
                        Ok(entries) => {
                            let pane = self.sftp_pane.as_mut().unwrap();
                            pane.local_path = new_path;
                            pane.local_entries = entries;
                            pane.list_state.select(Some(0));
                        }
                        Err(e) => eprintln!("[List local directory failed: {e}]"),
                    }
                }
            }
        }
        Ok(())
    }

    async fn start_download(&mut self) -> Result<()> {
        let Some(pane) = &self.sftp_pane else {
            return Ok(());
        };
        if pane.side != PaneSide::Remote {
            return Ok(());
        }
        let idx = pane.list_state.selected().unwrap_or(0);
        let Some(entry) = pane.remote_entries.get(idx).cloned() else {
            return Ok(());
        };
        if entry.is_dir {
            return Ok(());
        }

        let remote_path = format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name);
        let local_dest = pane.local_path.join(&entry.name);
        let total = entry.size;

        let sftp_session = {
            let Some(session) = &self.active_session else {
                return Ok(());
            };
            let ch = session.handle.channel_open_session().await?;
            ch.request_subsystem(true, "sftp").await?;
            russh_sftp::client::SftpSession::new(ch.into_stream()).await?
        };

        let handle = download(sftp_session, remote_path, local_dest, total);
        let init_prog = TransferProgress {
            filename: entry.name.clone(),
            total_bytes: total,
            transferred_bytes: 0,
            state: TransferState::InProgress,
        };
        self.active_transfer = Some(ActiveTransfer {
            verb: "Downloading",
            dir: TransferDir::Download,
            progress: init_prog,
            rx: handle.rx,
            cancel: handle.cancel,
            done_at: None,
            needs_refresh: true,
        });
        Ok(())
    }

    async fn start_upload(&mut self) -> Result<()> {
        let Some(pane) = &self.sftp_pane else {
            return Ok(());
        };
        if pane.side != PaneSide::Local {
            return Ok(());
        }
        let idx = pane.list_state.selected().unwrap_or(0);
        let Some(entry) = pane.local_entries.get(idx).cloned() else {
            return Ok(());
        };
        if entry.is_dir {
            return Ok(());
        }

        let local_src = pane.local_path.join(&entry.name);
        let remote_dest = format!("{}/{}", pane.remote_path.trim_end_matches('/'), entry.name);

        let sftp_session = {
            let Some(session) = &self.active_session else {
                return Ok(());
            };
            let ch = session.handle.channel_open_session().await?;
            ch.request_subsystem(true, "sftp").await?;
            russh_sftp::client::SftpSession::new(ch.into_stream()).await?
        };

        let handle = upload(sftp_session, local_src, remote_dest)?;
        let total = entry.size;
        let init_prog = TransferProgress {
            filename: entry.name.clone(),
            total_bytes: total,
            transferred_bytes: 0,
            state: TransferState::InProgress,
        };
        self.active_transfer = Some(ActiveTransfer {
            verb: "Uploading",
            dir: TransferDir::Upload,
            progress: init_prog,
            rx: handle.rx,
            cancel: handle.cancel,
            done_at: None,
            needs_refresh: true,
        });
        Ok(())
    }

    async fn ssh_connect_and_takeover(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        host: &Host,
    ) -> Result<()> {
        let mut session = self.connect_with_prompt(terminal, bus, host).await?;
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        // Reserve 1 row for the status bar.
        let term_rows = rows.saturating_sub(3).max(1);
        session.request_pty(cols, term_rows).await?;

        self.terminal_emulator = Some(TerminalEmulator::new(cols, term_rows));
        self.active_session = Some(session);
        self.current_host_alias = Some(host.alias.clone());
        self.mode = AppMode::Ssh;
        self.connection_history.record(&host.alias);
        self.ssh_last_size = Some((cols, rows));
        Ok(())
    }

    async fn connect_with_prompt(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        host: &Host,
    ) -> Result<ActiveSession> {
        let mut session = ActiveSession::connect(&host.hostname, host.port).await?;

        if auth::try_agent_auth(&mut session.handle, &host.user)
            .await
            .unwrap_or(false)
        {
            return Ok(session);
        }

        for key_path in &host.identity_files {
            let expanded = expand_tilde(key_path);
            if try_key_auth(&mut session.handle, &host.user, &expanded, None)
                .await
                .unwrap_or(false)
            {
                return Ok(session);
            }

            let identity_hint = key_path.display().to_string();
            let key_passphrase_key = SecretKey::new(
                &host.id,
                SecretKind::KeyPassphrase,
                Some(identity_hint.as_str()),
            );
            if let Ok(Some(pass)) = self.secret_store.get(&key_passphrase_key)
                && try_key_auth(&mut session.handle, &host.user, &expanded, Some(&pass))
                    .await
                    .unwrap_or(false)
            {
                return Ok(session);
            }

            let title =
                self.key_passphrase_prompt_title(identity_hint.as_str(), &key_passphrase_key.account);
            if let Some(pass) = self.prompt_password(terminal, bus, &title).await?
                && try_key_auth(&mut session.handle, &host.user, &expanded, Some(&pass))
                    .await
                    .unwrap_or(false)
            {
                match self.secret_store.set(&key_passphrase_key, &pass) {
                    Ok(()) => self.clear_secret_save_failure(&key_passphrase_key.account),
                    Err(error) => self
                        .record_secret_save_failure(key_passphrase_key.account.clone(), &error),
                }
                return Ok(session);
            }
        }

        let password_key = SecretKey::new(&host.id, SecretKind::LoginPassword, None);
        if let Ok(Some(pass)) = self.secret_store.get(&password_key) {
            let ok = session
                .handle
                .authenticate_password(&host.user, &pass)
                .await?
                .success();
            if ok {
                return Ok(session);
            }
        }

        let title = self.password_prompt_title(&host.user, &host.hostname, &password_key.account);
        if let Some(pass) = self.prompt_password(terminal, bus, &title).await? {
            let ok = session
                .handle
                .authenticate_password(&host.user, &pass)
                .await?
                .success();
            if ok {
                match self.secret_store.set(&password_key, &pass) {
                    Ok(()) => self.clear_secret_save_failure(&password_key.account),
                    Err(error) => {
                        self.record_secret_save_failure(password_key.account.clone(), &error)
                    }
                }
                return Ok(session);
            }
        }

        bail!("all authentication methods failed")
    }

    async fn prompt_password(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
        title: &str,
    ) -> Result<Option<String>> {
        self.pwd_dialog = Some(PwdDialog {
            dialog: PasswordDialog::new(title),
            result: None,
        });

        loop {
            self.render(terminal)?;
            match bus.next().await {
                Some(AppEvent::Input(data)) => {
                    for key in decode_tui_keys(&data) {
                        self.handle_pwd_key(key);
                    }
                }
                Some(AppEvent::Tick) => {}
                None => {
                    self.pwd_dialog = None;
                    return Ok(None);
                }
            }

            if let Some(result) = self.pwd_dialog.as_mut().and_then(|pwd| pwd.result.take()) {
                self.pwd_dialog = None;
                return Ok(result);
            }
        }
    }

    async fn switch_ssh_to_sftp(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        if self.sftp_client.is_none() || self.sftp_pane.is_none() {
            let Some(session) = self.active_session.as_ref() else {
                bail!("SSH session does not exist");
            };
            let client = SftpClient::open(session).await?;
            let home = client.home_dir().await;
            let remote_entries = client.list_dir(&home).await.unwrap_or_default();
            let local_path = std::env::current_dir().unwrap_or_default();
            let local_entries = list_local(&local_path).unwrap_or_default();

            let mut pane = SftpPaneState::new(home);
            pane.remote_entries = remote_entries;
            pane.local_entries = local_entries;
            self.sftp_client = Some(client);
            self.sftp_pane = Some(pane);
        }

        terminal.clear()?;
        self.mode = AppMode::Sftp;
        Ok(())
    }

    async fn resume_ssh_from_sftp(&mut self) -> Result<()> {
        let Some(session) = self.active_session.as_mut() else {
            self.exit_sftp();
            return Ok(());
        };

        if !session.has_pty() {
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
            let term_rows = rows.saturating_sub(3).max(1);
            session.request_pty(cols, term_rows).await?;
            self.terminal_emulator = Some(TerminalEmulator::new(cols, term_rows));
            self.ssh_last_size = Some((cols, rows));
        }

        self.mode = AppMode::Ssh;
        Ok(())
    }

    async fn handle_ssh_input(&mut self, data: Vec<u8>) -> Result<()> {
        let (forward, switch) = split_ssh_input(&data, SWITCH_SEQ);
        if !forward.is_empty()
            && let Some(session) = self.active_session.as_mut()
        {
            session.write_input(&forward).await?;
        }
        if switch {
            self.trigger_ssh_to_sftp = true;
        }
        Ok(())
    }

    async fn handle_ssh_channel_msg(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        msg: Option<ChannelMsg>,
    ) -> Result<()> {
        match msg {
            Some(ChannelMsg::Data { ref data }) => {
                if let Some(emulator) = &mut self.terminal_emulator {
                    emulator.process(data);
                }
            }
            Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                if let Some(emulator) = &mut self.terminal_emulator {
                    emulator.process(data);
                }
            }
            Some(ChannelMsg::ExitStatus { .. }) | Some(ChannelMsg::Eof) | None => {
                self.leave_ssh_mode(terminal).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn leave_ssh_mode(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        terminal.clear()?;
        self.ssh_last_size = None;
        self.terminal_emulator = None;
        if let Some(session) = self.active_session.take() {
            let _ = session.disconnect().await;
        }
        self.sftp_client = None;
        self.sftp_pane = None;
        self.current_host_alias = None;
        self.mode = AppMode::Main;
        Ok(())
    }

    async fn sync_ssh_size(&mut self) -> Result<()> {
        let Some(session) = self.active_session.as_mut() else {
            return Ok(());
        };
        if !session.has_pty() {
            return Ok(());
        }
        let size = crossterm::terminal::size().unwrap_or((80, 24));
        if self.ssh_last_size != Some(size) {
            let term_rows = size.1.saturating_sub(3).max(1);
            session.resize_pty(size.0, term_rows).await?;
            if let Some(emulator) = &mut self.terminal_emulator {
                emulator.resize(size.0, term_rows);
            }
            self.ssh_last_size = Some(size);
        }
        Ok(())
    }

    fn render(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        let mut list_state = std::mem::take(&mut self.list_state);
        terminal.draw(|f| match self.mode {
            AppMode::Main => {
                main_view::render(f, self, &mut list_state);
                if let Some(pwd) = &self.pwd_dialog {
                    pwd.dialog.render(f);
                }
                if self.confirm_delete
                    && let Some(&idx) = self
                        .filtered_indices
                        .get(list_state.selected().unwrap_or(0))
                {
                    let alias = self.hosts[idx].alias.clone();
                    ConfirmDialog::new(
                        "Delete Host",
                        &format!("Delete \"{alias}\"? (sush config only)"),
                    )
                    .render(f);
                }
                if self.show_import_prompt {
                    ConfirmDialog::new(
                        "Import SSH Config",
                        "Found hosts in ~/.ssh/config. Import them?",
                    )
                    .render(f);
                }
            }
            AppMode::Sftp => {
                if let Some(pane) = &mut self.sftp_pane {
                    let alias = self.current_host_alias.as_deref().unwrap_or("");
                    let transfer_info =
                        self.active_transfer.as_ref().map(|t| (t.verb, &t.progress));
                    sftp_view::render(f, alias, pane, transfer_info);
                }
                if let Some(pwd) = &self.pwd_dialog {
                    pwd.dialog.render(f);
                }
            }
            AppMode::Ssh => {
                if let Some(emulator) = &self.terminal_emulator {
                    let alias = self.current_host_alias.as_deref().unwrap_or("");
                    ssh_view::render(f, alias, emulator);
                }
            }
            AppMode::Edit => {
                if let Some(draft) = &self.edit_draft {
                    let all_tags: Vec<String> = {
                        let mut t: Vec<String> = self
                            .hosts
                            .iter()
                            .flat_map(|h| h.tags.iter().cloned())
                            .collect();
                        t.sort();
                        t.dedup();
                        t
                    };
                    edit_view::render(f, draft, &all_tags);
                }
            }
            AppMode::ImportSshConfig => {
                if let Some(state) = &self.import_state {
                    import_view::render(f, state);
                }
            }
            AppMode::FolderView => {
                if let Some(fv) = &self.folder_view_state {
                    let probe: Option<Option<bool>> = if self.probe_result.is_some() {
                        Some(self.probe_result)
                    } else if self.probe_rx.is_some() {
                        Some(None)
                    } else {
                        None
                    };
                    folder_view::render(f, fv, &self.hosts, &self.folder_host_indices, probe);
                }
            }
        })?;
        self.list_state = list_state;
        Ok(())
    }
}

const SWITCH_SEQ: &[u8] = b"\x1c";

fn is_switch_key(k: KeyEvent) -> bool {
    matches!(
        (k.code, k.modifiers),
        (KeyCode::Char('\\'), KeyModifiers::CONTROL)
    )
}

fn split_ssh_input(data: &[u8], switch_seq: &[u8]) -> (Vec<u8>, bool) {
    if switch_seq.is_empty() {
        return (data.to_vec(), false);
    }

    if let Some(pos) = data
        .windows(switch_seq.len())
        .position(|window| window == switch_seq)
    {
        (data[..pos].to_vec(), true)
    } else {
        (data.to_vec(), false)
    }
}

fn decode_tui_keys(data: &[u8]) -> Vec<KeyEvent> {
    let mut keys = Vec::new();
    let mut i = 0;

    while i < data.len() {
        if let Some((key, consumed)) = decode_escape_key(&data[i..]) {
            keys.push(key);
            i += consumed;
            continue;
        }

        let byte = data[i];
        match byte {
            b'\r' | b'\n' => {
                keys.push(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                i += 1;
            }
            b'\t' => {
                keys.push(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                i += 1;
            }
            0x03 => {
                keys.push(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
                i += 1;
            }
            0x1c => {
                keys.push(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
                i += 1;
            }
            0x08 | 0x7f => {
                keys.push(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
                i += 1;
            }
            0x01..=0x1a => {
                let ch = ((byte - 1) + b'a') as char;
                keys.push(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL));
                i += 1;
            }
            0x1b => {
                keys.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                i += 1;
            }
            _ if byte.is_ascii() => {
                keys.push(KeyEvent::new(
                    KeyCode::Char(byte as char),
                    KeyModifiers::NONE,
                ));
                i += 1;
            }
            _ => {
                let width = utf8_char_width(byte).min(data.len() - i);
                if let Ok(text) = std::str::from_utf8(&data[i..i + width])
                    && let Some(ch) = text.chars().next()
                {
                    keys.push(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                    i += ch.len_utf8();
                    continue;
                }
                i += 1;
            }
        }
    }

    keys
}

fn decode_escape_key(data: &[u8]) -> Option<(KeyEvent, usize)> {
    let patterns = [
        (b"\x1b[A".as_slice(), KeyCode::Up),
        (b"\x1b[B".as_slice(), KeyCode::Down),
        (b"\x1b[C".as_slice(), KeyCode::Right),
        (b"\x1b[D".as_slice(), KeyCode::Left),
        (b"\x1b[H".as_slice(), KeyCode::Home),
        (b"\x1b[F".as_slice(), KeyCode::End),
        (b"\x1b[3~".as_slice(), KeyCode::Delete),
    ];

    patterns.iter().find_map(|(pattern, code)| {
        data.starts_with(pattern)
            .then(|| (KeyEvent::new(*code, KeyModifiers::NONE), pattern.len()))
    })
}

fn utf8_char_width(byte: u8) -> usize {
    match byte {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

fn expand_tilde(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    p.to_path_buf()
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut out = stdout();
    crossterm::execute!(out, crossterm::terminal::EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::history::ConnectionHistory;
    use crate::config::host::{Host, HostSource};
    use crate::config::secrets::{FakeBackend, SecretStore};
    use crate::config::store::{Metadata, SecretSaveFailure};

    fn app_with(hosts: Vec<Host>) -> App {
        App {
            mode: AppMode::Main,
            hosts,
            search_query: String::new(),
            filtered_indices: vec![],
            should_quit: false,
            main_focus: MainFocus::HostList,
            main_focus_before_search: MainFocus::HostList,
            show_folder_sidebar: false,
            list_state: ListState::default(),
            trigger_connect: false,
            trigger_sftp: false,
            trigger_pane_enter: false,
            trigger_download: false,
            trigger_upload: false,
            trigger_refresh_local: false,
            trigger_refresh_remote: false,
            trigger_ssh_resume: false,
            trigger_ssh_to_sftp: false,
            active_session: None,
            sftp_client: None,
            sftp_pane: None,
            current_host_alias: None,
            active_transfer: None,
            last_ctrl_c: None,
            ssh_last_size: None,
            terminal_emulator: None,
            edit_draft: None,
            import_state: None,
            confirm_delete: false,
            import_prompted: false,
            metadata: Metadata::default(),
            connection_history: ConnectionHistory::load(std::path::PathBuf::from(
                "/tmp/sush-test-history.toml",
            )),
            hover_host_id: None,
            hover_since: None,
            probe_rx: None,
            probe_result: None,
            status_msg: None,
            show_import_prompt: false,
            folder_view_state: None,
            folder_host_indices: vec![],
            secret_store: SecretStore::new(Box::new(FakeBackend::available())),
            pwd_dialog: None,
        }
    }

    fn app_with_metadata_failures() -> App {
        let mut app = app_with(vec![]);
        app.metadata = Metadata {
            ssh_config_hash: String::new(),
            import_prompted: false,
            secret_save_failures: vec![SecretSaveFailure {
                account: "host-1:login_password".into(),
                reason: "system keyring is unavailable".into(),
            }],
        };
        app.secret_store = SecretStore::new(Box::new(FakeBackend::available()));
        app
    }

    fn mk(id: &str) -> Host {
        Host {
            id: id.into(),
            alias: id.into(),
            hostname: "1".into(),
            port: 22,
            user: "u".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec![],
            description: String::new(),
            source: HostSource::Manual,
        }
    }

    #[test]
    fn search_filters_hosts() {
        let mut app = app_with(vec![mk("web"), mk("db")]);
        app.search_query = "web".into();
        app.apply_search();
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.hosts[app.filtered_indices[0]].alias, "web");
    }

    #[test]
    fn empty_search_shows_all() {
        let mut app = app_with(vec![mk("a"), mk("b"), mk("c")]);
        app.apply_search();
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
    }

    #[test]
    fn decode_switch_key_from_raw_input() {
        let keys = decode_tui_keys(b"\x1c");
        assert_eq!(keys.len(), 1);
        assert!(is_switch_key(keys[0]));
    }

    #[test]
    fn split_ssh_input_stops_before_switch_key() {
        let (forward, switched) = split_ssh_input(b"ls\x1c", SWITCH_SEQ);
        assert_eq!(forward, b"ls");
        assert!(switched);
    }

    #[test]
    fn app_test_helper_carries_secret_failure_metadata() {
        let app = app_with_metadata_failures();
        assert_eq!(app.metadata.secret_save_failures.len(), 1);
        assert_eq!(
            app.metadata.secret_save_failures[0].account,
            "host-1:login_password"
        );
    }

    #[test]
    fn password_prompt_title_includes_previous_save_failure_reason() {
        let mut app = app_with_metadata_failures();
        let title =
            app.password_prompt_title("deploy", "prod.example.com", "host-1:login_password");
        assert!(title.contains("Password was not saved last time"));
        assert!(title.contains("system keyring is unavailable"));
        assert!(title.contains("Password for deploy@prod.example.com:"));
    }

    #[test]
    fn record_secret_save_failure_stores_user_message() {
        let mut app = app_with(vec![]);
        app.record_secret_save_failure(
            "host-1:login_password".into(),
            &crate::config::secrets::SecretError::Unavailable(
                "linux secret service not found".into(),
            ),
        );
        assert_eq!(app.metadata.secret_save_failures.len(), 1);
        assert!(
            app.metadata.secret_save_failures[0]
                .reason
                .contains("system keyring is unavailable")
        );
    }

    #[test]
    fn key_passphrase_account_uses_identity_path() {
        let key = SecretKey::new(
            "host-1",
            SecretKind::KeyPassphrase,
            Some("/Users/me/.ssh/id_ed25519"),
        );
        assert!(key.account.contains("id_ed25519"));
    }

    #[test]
    fn key_passphrase_prompt_title_includes_failure_reason() {
        let mut app = app_with(vec![]);
        app.metadata.upsert_secret_failure(
            "host-1:key_passphrase:/Users/me/.ssh/id_ed25519".into(),
            "permission denied by system keyring".into(),
        );
        let title = app.key_passphrase_prompt_title(
            "/Users/me/.ssh/id_ed25519",
            "host-1:key_passphrase:/Users/me/.ssh/id_ed25519",
        );
        assert!(title.contains("Password was not saved last time"));
        assert!(title.contains("permission denied by system keyring"));
        assert!(title.contains("Key passphrase (/Users/me/.ssh/id_ed25519):"));
    }
}
