use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};

use crate::config::host::Host;
use crate::tui::widgets::status_bar::StatusBar;

#[allow(dead_code)]
pub struct ImportEntry {
    pub host: Host,
    pub selected: bool,
    pub already_exists: bool,
}

#[allow(dead_code)]
pub struct ImportViewState {
    pub entries: Vec<ImportEntry>,
    pub cursor: usize,
}

#[allow(dead_code)]
impl ImportViewState {
    pub fn new(ssh_hosts: Vec<Host>, existing: &[Host]) -> Self {
        let entries = ssh_hosts
            .into_iter()
            .map(|h| {
                let already_exists = existing.iter().any(|e| e.id == h.id);
                ImportEntry {
                    selected: !already_exists,
                    host: h,
                    already_exists,
                }
            })
            .collect();
        Self { entries, cursor: 0 }
    }

    pub fn toggle_selected(&mut self) {
        if let Some(e) = self.entries.get_mut(self.cursor)
            && !e.already_exists
        {
            e.selected = !e.selected;
        }
    }

    pub fn toggle_all(&mut self) {
        let all_on = self
            .entries
            .iter()
            .filter(|e| !e.already_exists)
            .all(|e| e.selected);
        for e in self.entries.iter_mut().filter(|e| !e.already_exists) {
            e.selected = !all_on;
        }
    }

    pub fn selected_hosts(&self) -> Vec<Host> {
        self.entries
            .iter()
            .filter(|e| e.selected && !e.already_exists)
            .map(|e| e.host.clone())
            .collect()
    }

    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        self.cursor = (self.cursor + 1).min(self.entries.len().saturating_sub(1));
    }
}

#[allow(dead_code)]
pub fn render(f: &mut Frame, state: &ImportViewState) {
    let [list_area, status_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let block = Block::bordered()
        .title(" Import from ~/.ssh/config ")
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = state
        .entries
        .iter()
        .map(|e| {
            let checkbox = if e.already_exists {
                "[~]"
            } else if e.selected {
                "[x]"
            } else {
                "[ ]"
            };
            let style = if e.already_exists {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(checkbox, style),
                Span::raw(" "),
                Span::styled(format!("{:<20}", e.host.alias), style.fg(Color::Cyan)),
                Span::styled(format!("{:<20}", e.host.hostname), style),
                Span::styled(e.host.user.as_str(), style),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.cursor));

    f.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        list_area,
        &mut list_state,
    );

    f.render_widget(
        StatusBar {
            hints: &[
                ("Space", "Toggle"),
                ("a", "All"),
                ("Enter", "Import"),
                ("ESC", "Cancel"),
            ],
        },
        status_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::host::{Host, HostSource};

    fn make_host(id: &str) -> Host {
        Host {
            id: id.into(),
            alias: id.into(),
            hostname: id.into(),
            port: 22,
            user: "u".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec![],
            description: String::new(),
            source: HostSource::SshConfig,
        }
    }

    #[test]
    fn existing_hosts_marked_already_exists() {
        let ssh_hosts = vec![make_host("web"), make_host("db")];
        let existing = vec![make_host("web")];
        let state = ImportViewState::new(ssh_hosts, &existing);
        assert!(state.entries[0].already_exists);
        assert!(!state.entries[1].already_exists);
    }

    #[test]
    fn toggle_skips_already_existing() {
        let ssh_hosts = vec![make_host("web"), make_host("db")];
        let existing = vec![make_host("web")];
        let mut state = ImportViewState::new(ssh_hosts, &existing);
        state.cursor = 0;
        let was = state.entries[0].selected;
        state.toggle_selected();
        assert_eq!(state.entries[0].selected, was);
    }

    #[test]
    fn toggle_selects_available_entry() {
        let ssh_hosts = vec![make_host("web"), make_host("db")];
        let mut state = ImportViewState::new(ssh_hosts, &[]);
        state.cursor = 1;
        state.entries[1].selected = false;
        state.toggle_selected();
        assert!(state.entries[1].selected);
    }

    #[test]
    fn selected_hosts_returns_only_selected_non_existing() {
        let ssh_hosts = vec![make_host("web"), make_host("db")];
        let mut state = ImportViewState::new(ssh_hosts, &[]);
        state.entries[0].selected = true;
        state.entries[1].selected = false;
        let selected = state.selected_hosts();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "web");
    }

    #[test]
    fn navigation_moves_cursor() {
        let ssh_hosts = vec![make_host("a"), make_host("b")];
        let mut state = ImportViewState::new(ssh_hosts, &[]);
        state.move_down();
        assert_eq!(state.cursor, 1);
        state.move_up();
        assert_eq!(state.cursor, 0);
    }
}
