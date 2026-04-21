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

    f.render_stateful_widget(
        HostList {
            hosts: &app.hosts,
            indices: &app.filtered_indices,
            focused: !search_focused,
        },
        chunks[1],
        list_state,
    );

    let hints: &[(&str, &str)] = if search_focused {
        &[("Enter", "SSH"), ("S+Enter", "SFTP")]
    } else {
        &[
            ("/", "搜索"),
            ("Enter", "SSH"),
            ("S+Enter", "SFTP"),
            ("n", "新建"),
            ("e", "编辑"),
            ("d", "删除"),
            ("?", "帮助"),
        ]
    };

    f.render_widget(StatusBar { hints }, chunks[2]);
}
