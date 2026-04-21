use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::ListState;

use crate::app::App;
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

    f.render_widget(
        SearchInput {
            query: &app.search_query,
            focused: true,
        },
        chunks[0],
    );

    f.render_stateful_widget(
        HostList {
            hosts: &app.hosts,
            indices: &app.filtered_indices,
            focused: false,
        },
        chunks[1],
        list_state,
    );

    f.render_widget(
        StatusBar {
            hints: &[
                ("Enter", "SSH"),
                ("F2", "SFTP"),
                ("F5", "新建"),
                ("?", "帮助"),
                ("Q", "退出"),
            ],
        },
        chunks[2],
    );
}
