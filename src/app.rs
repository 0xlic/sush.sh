use std::io::{Stdout, stdout};
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;

use crate::config::host::Host;
use crate::config::{ssh_config, store};
use crate::sftp::client::{SftpClient, list_local};
use crate::sftp::transfer::{TransferProgress, TransferState, download, upload};
use crate::sftp::{PaneSide, SftpPaneState};
use crate::ssh::auth;
use crate::ssh::session::ActiveSession;
use crate::tui::event::{AppEvent, EventBus};
use crate::tui::views::{main_view, password_dialog::PasswordDialog, sftp_view};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainFocus {
    HostList,
    Search,
}

pub enum AppMode {
    Main,
    #[allow(dead_code)]
    Ssh,
    Sftp,
}

struct PwdDialog {
    dialog: PasswordDialog,
    result: Option<Option<String>>,
}

/// 传输方向：决定完成后刷新哪一侧目录。
#[derive(Clone, Copy)]
pub enum TransferDir {
    Download, // remote → local，完成后刷新本地
    Upload,   // local → remote，完成后刷新远程
}

pub struct ActiveTransfer {
    pub verb: &'static str,
    pub dir: TransferDir,
    pub progress: TransferProgress,
    pub rx: tokio::sync::mpsc::Receiver<TransferProgress>,
    pub cancel: tokio_util::sync::CancellationToken,
    pub done_at: Option<Instant>,
    pub needs_refresh: bool, // 完成后还未刷新时为 true
}

pub struct App {
    pub mode: AppMode,
    pub hosts: Vec<Host>,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub should_quit: bool,
    pub main_focus: MainFocus,
    pub list_state: ListState,
    pub trigger_connect: bool,
    pub trigger_sftp: bool,
    pub trigger_pane_enter: bool,
    pub trigger_download: bool,
    pub trigger_upload: bool,
    pub trigger_refresh_local: bool,
    pub trigger_refresh_remote: bool,
    pub trigger_ssh_resume: bool,
    pub active_session: Option<ActiveSession>,
    pub sftp_client: Option<SftpClient>,
    pub sftp_pane: Option<SftpPaneState>,
    pub current_host_alias: Option<String>,
    pub active_transfer: Option<ActiveTransfer>,
    pub last_ctrl_c: Option<Instant>,
    pwd_dialog: Option<PwdDialog>,
}

impl App {
    pub fn new() -> Result<Self> {
        let (existing, prev_hash) = store::load_from(&store::config_path())?;
        let (imported, new_hash) = ssh_config::import_ssh_config()?;

        let hosts = if !new_hash.is_empty() && new_hash != prev_hash {
            let merged = store::merge_ssh_config_hosts(existing, imported);
            store::save_to(&store::config_path(), &merged, &new_hash)?;
            merged
        } else {
            existing
        };

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
            list_state,
            trigger_connect: false,
            trigger_sftp: false,
            trigger_pane_enter: false,
            trigger_download: false,
            trigger_upload: false,
            trigger_refresh_local: false,
            trigger_refresh_remote: false,
            trigger_ssh_resume: false,
            active_session: None,
            sftp_client: None,
            sftp_pane: None,
            current_host_alias: None,
            active_transfer: None,
            last_ctrl_c: None,
            pwd_dialog: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;
        let result = self.event_loop(&mut terminal).await;
        restore_terminal(&mut terminal)?;
        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let mut bus = EventBus::new();

        while !self.should_quit {
            // SSH 连接触发
            if self.trigger_connect {
                self.trigger_connect = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let host = self.hosts[idx].clone();

                    // 停止后台 reader，避免与 takeover 的 stdin 竞争
                    std::mem::take(&mut bus).shutdown();

                    crossterm::execute!(
                        std::io::stdout(),
                        crossterm::terminal::LeaveAlternateScreen
                    )?;

                    let ssh_result = self.ssh_connect_and_takeover(&host).await;

                    crossterm::execute!(
                        std::io::stdout(),
                        crossterm::terminal::EnterAlternateScreen
                    )?;
                    terminal.clear()?;

                    // 重建 EventBus：丢弃 SSH 期间所有积压的按键
                    bus = EventBus::new();
                    // 清空搜索框，防止 SSH 中输入的字符带回主界面
                    self.search_query.clear();
                    self.apply_search();

                    if let Err(e) = ssh_result {
                        eprintln!("\r\n[连接错误: {e}]\r\n");
                    }
                }
            }

            // SFTP 连接触发
            if self.trigger_sftp {
                self.trigger_sftp = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let host = self.hosts[idx].clone();
                    if let Err(e) = self.sftp_connect(&host).await {
                        eprintln!("[SFTP 错误: {e}]");
                    }
                }
            }

            // SFTP 目录导航触发
            if self.trigger_pane_enter {
                self.trigger_pane_enter = false;
                if let Err(e) = self.pane_enter().await {
                    eprintln!("[导航错误: {e}]");
                }
            }

            // 下载触发
            if self.trigger_download {
                self.trigger_download = false;
                if let Err(e) = self.start_download().await {
                    eprintln!("[下载错误: {e}]");
                }
            }

            // 上传触发
            if self.trigger_upload {
                self.trigger_upload = false;
                if let Err(e) = self.start_upload().await {
                    eprintln!("[上传错误: {e}]");
                }
            }

            // 传输完成后刷新本地目录
            if self.trigger_refresh_local {
                self.trigger_refresh_local = false;
                if let Some(pane) = &mut self.sftp_pane
                    && let Ok(entries) = list_local(&pane.local_path)
                {
                    pane.local_entries = entries;
                }
            }

            // 传输完成后刷新远程目录
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

            // SSH 恢复触发（从 SFTP 切回 SSH）
            if self.trigger_ssh_resume {
                self.trigger_ssh_resume = false;
                std::mem::take(&mut bus).shutdown();
                crossterm::execute!(
                    std::io::stdout(),
                    crossterm::terminal::LeaveAlternateScreen
                )?;

                let remote_closed = if let Some(session) = &mut self.active_session {
                    !session.takeover(b"\x1c").await.unwrap_or(false)
                } else {
                    true
                };

                crossterm::execute!(
                    std::io::stdout(),
                    crossterm::terminal::EnterAlternateScreen
                )?;
                terminal.clear()?;
                bus = EventBus::new();

                if remote_closed {
                    if let Some(s) = self.active_session.take() {
                        let _ = s.disconnect().await;
                    }
                    self.exit_sftp();
                }
            }

            // 轮询传输进度
            if let Some(tr) = &mut self.active_transfer {
                while let Ok(prog) = tr.rx.try_recv() {
                    tr.progress = prog;
                }
                let done = matches!(
                    tr.progress.state,
                    TransferState::Completed | TransferState::Failed(_) | TransferState::Cancelled
                );
                if done {
                    // 第一次检测到完成：触发目录刷新
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

            // 渲染
            let mut list_state = std::mem::take(&mut self.list_state);
            terminal.draw(|f| match self.mode {
                AppMode::Main => {
                    main_view::render(f, self, &mut list_state);
                    if let Some(pwd) = &self.pwd_dialog {
                        pwd.dialog.render(f);
                    }
                }
                AppMode::Sftp => {
                    if let Some(pane) = &mut self.sftp_pane {
                        let alias = self.current_host_alias.as_deref().unwrap_or("");
                        let transfer_info =
                            self.active_transfer.as_ref().map(|t| (t.verb, &t.progress));
                        sftp_view::render(f, alias, pane, transfer_info);
                    }
                }
                AppMode::Ssh => {}
            })?;
            self.list_state = list_state;

            if let Some(ev) = bus.next().await {
                self.handle_event(ev);
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, ev: AppEvent) {
        if let AppEvent::Key(k) = ev {
            self.handle_key(k);
        }
    }

    fn handle_key(&mut self, k: KeyEvent) {
        if self.pwd_dialog.is_some() {
            self.handle_pwd_key(k);
            return;
        }
        match self.mode {
            AppMode::Main => self.handle_main_key(k),
            AppMode::Sftp => self.handle_sftp_key(k),
            AppMode::Ssh => {}
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

    fn handle_main_key(&mut self, k: KeyEvent) {
        match self.main_focus {
            MainFocus::HostList => self.handle_main_key_hostlist(k),
            MainFocus::Search => self.handle_main_key_search(k),
        }
    }

    fn handle_main_key_hostlist(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) | (KeyCode::Char('/'), KeyModifiers::NONE) => {
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
            // n/e/d/? 暂无实现
            _ => {}
        }
    }

    fn handle_main_key_search(&mut self, k: KeyEvent) {
        match (k.code, k.modifiers) {
            (KeyCode::Tab, _) | (KeyCode::Esc, _) | (KeyCode::Enter, _) => {
                self.main_focus = MainFocus::HostList;
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
            (KeyCode::Char('\\'), KeyModifiers::CONTROL) => self.trigger_ssh_resume = true,
            (KeyCode::Char('d'), KeyModifiers::NONE) => self.trigger_download = true,
            (KeyCode::Char('u'), KeyModifiers::NONE) => self.trigger_upload = true,
            // D 删除、r 重命名暂无实现
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
        self.filtered_indices = crate::utils::fuzzy::search(&self.search_query, &self.hosts);
        let sel = if self.filtered_indices.is_empty() {
            None
        } else {
            Some(0)
        };
        self.list_state.select(sel);
    }

    fn select_next(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let next = (i + 1).min(self.filtered_indices.len() - 1);
        self.list_state.select(Some(next));
    }

    fn select_previous(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        let prev = i.saturating_sub(1);
        self.list_state.select(Some(prev));
    }

    async fn sftp_connect(&mut self, host: &Host) -> Result<()> {
        let prompt: auth::PasswordPrompt = Box::new(|title: &str| -> Option<String> {
            use std::io::Write;
            eprint!("{title}");
            let _ = std::io::stderr().flush();
            let mut s = String::new();
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = std::io::stdin().read_line(&mut s);
            let _ = crossterm::terminal::enable_raw_mode();
            let pass = s.trim_end_matches(['\n', '\r']).to_string();
            if pass.is_empty() { None } else { Some(pass) }
        });

        let session = auth::connect_with_host(host, prompt).await?;
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
                            Err(e) => eprintln!("[列目录失败: {e}]"),
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
                        Err(e) => eprintln!("[列本地目录失败: {e}]"),
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
            verb: "下载",
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
            verb: "上传",
            dir: TransferDir::Upload,
            progress: init_prog,
            rx: handle.rx,
            cancel: handle.cancel,
            done_at: None,
            needs_refresh: true,
        });
        Ok(())
    }

    async fn ssh_connect_and_takeover(&mut self, host: &Host) -> Result<()> {
        let prompt: auth::PasswordPrompt = Box::new(|title: &str| -> Option<String> {
            use std::io::Write;
            eprint!("{title}");
            let _ = std::io::stderr().flush();
            let mut s = String::new();
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = std::io::stdin().read_line(&mut s);
            let _ = crossterm::terminal::enable_raw_mode();
            let pass = s.trim_end_matches(['\n', '\r']).to_string();
            if pass.is_empty() { None } else { Some(pass) }
        });

        let mut session = auth::connect_with_host(host, prompt).await?;
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        session.request_pty(cols, rows).await?;

        let switched = session.takeover(b"\x1c").await.unwrap_or(false);
        if switched {
            // 保留 session，打开 SFTP，切换到 SFTP 界面
            match SftpClient::open(&session).await {
                Ok(client) => {
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
                }
                Err(_) => {
                    let _ = session.disconnect().await;
                    self.mode = AppMode::Main;
                }
            }
        } else {
            let _ = session.disconnect().await;
            self.mode = AppMode::Main;
        }
        Ok(())
    }
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
    use crate::config::host::{Host, HostSource};

    fn app_with(hosts: Vec<Host>) -> App {
        App {
            mode: AppMode::Main,
            hosts,
            search_query: String::new(),
            filtered_indices: vec![],
            should_quit: false,
            main_focus: MainFocus::HostList,
            list_state: ListState::default(),
            trigger_connect: false,
            trigger_sftp: false,
            trigger_pane_enter: false,
            trigger_download: false,
            trigger_upload: false,
            trigger_refresh_local: false,
            trigger_refresh_remote: false,
            trigger_ssh_resume: false,
            active_session: None,
            sftp_client: None,
            sftp_pane: None,
            current_host_alias: None,
            active_transfer: None,
            last_ctrl_c: None,
            pwd_dialog: None,
        }
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
}
