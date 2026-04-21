use std::io::{Stdout, stdout};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;

use crate::config::host::Host;
use crate::config::{ssh_config, store};
use crate::ssh::auth;
use crate::ssh::session::ActiveSession;
use crate::tui::event::{AppEvent, EventBus};
use crate::tui::views::{main_view, password_dialog::PasswordDialog};

pub enum AppMode {
    Main,
    #[allow(dead_code)]
    Ssh,
    #[allow(dead_code)]
    Sftp,
}

/// 密码弹窗状态（在需要用户输入时挂入 App）。
struct PwdDialog {
    dialog: PasswordDialog,
    /// 弹窗解决后，Some(密码) / None(取消)
    result: Option<Option<String>>,
}

pub struct App {
    pub mode: AppMode,
    pub hosts: Vec<Host>,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub should_quit: bool,
    pub list_state: ListState,
    /// 触发 SSH 连接的标志（由 Enter 键置位，事件循环清零）
    pub trigger_connect: bool,
    /// 当前活跃 SSH 会话（接管后保留以便切 SFTP）
    pub active_session: Option<ActiveSession>,
    /// 密码弹窗（pending 时挂在 App 上）
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
            list_state,
            trigger_connect: false,
            active_session: None,
            pwd_dialog: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = setup_terminal()?;
        let mut bus = EventBus::new();
        let result = self.event_loop(&mut terminal, &mut bus).await;
        restore_terminal(&mut terminal)?;
        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        bus: &mut EventBus,
    ) -> Result<()> {
        while !self.should_quit {
            // 触发 SSH 连接（Enter 键已置位）
            if self.trigger_connect {
                self.trigger_connect = false;
                if let Some(&idx) = self
                    .filtered_indices
                    .get(self.list_state.selected().unwrap_or(0))
                {
                    let host = self.hosts[idx].clone();
                    // 暂时离开 alternate screen（raw mode 保持），让 SSH 直接用终端
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

                    if let Err(e) = ssh_result {
                        // 错误暂时打印到 stderr，后续可以用状态栏显示
                        eprintln!("\r\n[连接错误: {e}]\r\n");
                    }
                }
            }

            let mut state = std::mem::take(&mut self.list_state);
            terminal.draw(|f| {
                if let AppMode::Main = self.mode {
                    main_view::render(f, self, &mut state);
                }
                // 密码弹窗叠加在主界面上
                if let Some(pwd) = &self.pwd_dialog {
                    pwd.dialog.render(f);
                }
            })?;
            self.list_state = state;

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
        // 密码弹窗优先拦截
        if self.pwd_dialog.is_some() {
            self.handle_pwd_key(k);
            return;
        }
        if let AppMode::Main = self.mode {
            self.handle_main_key(k);
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
        match (k.code, k.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if self.search_query.is_empty() {
                    self.should_quit = true;
                } else {
                    self.search_query.clear();
                    self.apply_search();
                }
            }
            (KeyCode::Backspace, _) => {
                self.search_query.pop();
                self.apply_search();
            }
            (KeyCode::Up, _) => self.select_previous(),
            (KeyCode::Down, _) => self.select_next(),
            (KeyCode::Enter, KeyModifiers::NONE) => {
                self.trigger_connect = true;
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                self.search_query.push(c);
                self.apply_search();
            }
            _ => {}
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

    /// 连接 SSH 并进入接管模式。
    /// `Ctrl-Space` 触发切 SFTP（v0.1 先返回主界面）；
    /// `exit`/`Ctrl-D` 正常退出 → 返回主界面。
    async fn ssh_connect_and_takeover(&mut self, host: &Host) -> Result<()> {
        // 密码 callback：通过 eprintln 直接在终端提示（raw mode 已关闭 alternate screen）
        let prompt: auth::PasswordPrompt = Box::new(|title: &str| -> Option<String> {
            use std::io::Write;
            eprint!("{title}");
            let _ = std::io::stderr().flush();
            let mut s = String::new();
            // 暂时禁用 raw mode 以便正常输入
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = std::io::stdin().read_line(&mut s);
            let _ = crossterm::terminal::enable_raw_mode();
            let pass = s.trim_end_matches(['\n', '\r']).to_string();
            if pass.is_empty() { None } else { Some(pass) }
        });

        let mut session = auth::connect_with_host(host, prompt).await?;
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        session.request_pty(cols, rows).await?;

        let switched = session.takeover(0x00).await?;
        if switched {
            // Ctrl-Space: v0.1 先返回主界面（阶段4实现 SFTP 切换）
            let _ = session.disconnect().await;
        } else {
            let _ = session.disconnect().await;
        }
        self.mode = AppMode::Main;
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
            list_state: ListState::default(),
            trigger_connect: false,
            active_session: None,
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
