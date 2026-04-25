use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::ListState;

use crate::app::{App, MainFocus};
use crate::tui::widgets::host_list::HostList;
use crate::tui::widgets::search_input::SearchInput;
use crate::tui::widgets::status_bar::StatusBar;

pub fn render(f: &mut Frame, app: &App, list_state: &mut ListState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    let search_focused = app.main_focus == MainFocus::Search;

    f.render_widget(
        SearchInput {
            query: &app.search_query,
            focused: search_focused,
        },
        chunks[0],
    );

    let probe: Option<Option<bool>> = if app.probe_result.is_some() {
        Some(app.probe_result)
    } else if app.probe_rx.is_some() {
        Some(None)
    } else {
        None
    };

    let status_msg = app.status_msg.as_ref().map(|(s, _)| s.as_str());

    f.render_stateful_widget(
        HostList {
            hosts: &app.hosts,
            indices: &app.filtered_indices,
            focused: !search_focused,
            probe,
            status_msg,
        },
        chunks[1],
        list_state,
    );

    let hints: &[(&str, &str)] = if search_focused {
        &[("Enter/Esc", "Back")]
    } else {
        &[
            ("/", "Search"),
            ("Enter", "SSH"),
            ("s", "SFTP"),
            ("n", "New"),
            ("e", "Edit"),
            ("d", "Delete"),
            ("i", "Import"),
            ("f", "Folders"),
            ("q", "Quit"),
        ]
    };
    f.render_widget(StatusBar { hints }, chunks[2]);
}
