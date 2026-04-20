use std::io::{Stdout, stdout};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;

use crate::config::host::Host;
use crate::config::{ssh_config, store};
use crate::tui::event::{AppEvent, EventBus};
use crate::tui::views::main_view;

pub enum AppMode {
    Main,
    #[allow(dead_code)]
    Ssh,
    #[allow(dead_code)]
    Sftp,
}

pub struct App {
    pub mode: AppMode,
    pub hosts: Vec<Host>,
    pub search_query: String,
    pub filtered_indices: Vec<usize>,
    pub should_quit: bool,
    pub list_state: ListState,
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
            let mut state = std::mem::take(&mut self.list_state);
            terminal.draw(|f| {
                if let AppMode::Main = self.mode {
                    main_view::render(f, self, &mut state);
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
        if let AppMode::Main = self.mode {
            self.handle_main_key(k);
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
