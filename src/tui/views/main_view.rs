use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};

use crate::app::{App, MainFocus};
use crate::tui::views::folder_view::{FolderNode, FolderViewState, JumpState};
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
            prefix: app.folder_search_prefix(),
        },
        chunks[0],
    );

    let probe: Option<Option<bool>> = if app.main_focus == MainFocus::HostList {
        if app.probe_result.is_some() {
            Some(app.probe_result)
        } else if app.probe_rx.is_some() {
            Some(None)
        } else {
            None
        }
    } else {
        None
    };

    let status_msg = app.main_status_message();

    let content_chunks = if app.show_folder_sidebar {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
            .split(chunks[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(chunks[1])
    };

    if app.show_folder_sidebar
        && let Some(folder_state) = &app.folder_view_state
    {
        render_directory_panel(
            f,
            content_chunks[0],
            folder_state,
            app.main_focus == MainFocus::Directory,
        );
    }

    let host_area = if app.show_folder_sidebar {
        content_chunks[1]
    } else {
        content_chunks[0]
    };
    f.render_stateful_widget(
        HostList {
            hosts: &app.hosts,
            indices: &app.filtered_indices,
            focused: app.main_focus == MainFocus::HostList,
            show_selection: app.main_focus == MainFocus::HostList,
            probe,
            status_msg,
        },
        host_area,
        list_state,
    );

    let hints: &[(&str, &str)] = if search_focused {
        &[("Enter/Esc", "Back")]
    } else if app.show_folder_sidebar && app.main_focus == MainFocus::Directory {
        &[
            ("Tab", "Hosts"),
            ("Enter", "Dive"),
            ("j", "Jump"),
            ("/", "Search"),
            ("f", "Hide"),
            ("q", "Quit"),
        ]
    } else if app.show_folder_sidebar {
        &[
            ("Tab", "Folders"),
            ("/", "Search"),
            ("Enter", "SSH"),
            ("s", "SFTP"),
            ("f", "Hide"),
            ("q", "Quit"),
        ]
    } else {
        &[
            ("/", "Search"),
            ("Enter", "SSH"),
            ("s", "SFTP"),
            ("p", "Forwards"),
            ("n", "New"),
            ("e", "Edit"),
            ("d", "Delete"),
            ("i", "Import"),
            ("f", "Folders"),
            ("q", "Quit"),
        ]
    };
    f.render_widget(
        StatusBar {
            hints,
            transfer_badge: app.global_transfer_badge().as_ref(),
        },
        chunks[2],
    );

    if app.show_folder_sidebar
        && app.main_focus == MainFocus::Directory
        && let Some(folder_state) = &app.folder_view_state
        && let Some(jump) = &folder_state.jump
    {
        render_jump_overlay(f, jump);
    }
}

fn render_directory_panel(f: &mut Frame, area: Rect, state: &FolderViewState, focused: bool) {
    let block = if focused {
        Block::bordered()
            .title(" Folders ")
            .border_style(Style::default().fg(Color::Cyan))
    } else {
        Block::bordered().title(" Folders ")
    };

    if state.col_a.is_empty() {
        f.render_widget(block, area);
        return;
    }

    let items: Vec<ListItem> = state
        .col_a
        .iter()
        .map(|path| {
            let node = state.tree.get(path);
            let has_children = node.is_some_and(|n| !n.children.is_empty());
            let label = folder_label(path, node);
            let indicator = if has_children { "▶ " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::raw(indicator),
                Span::styled(label, Style::default().fg(Color::Cyan)),
            ]))
        })
        .collect();

    let mut dir_state = ListState::default();
    dir_state.select(Some(state.sel_a));

    f.render_stateful_widget(
        List::new(items).block(block).highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .fg(Color::Yellow),
        ),
        area,
        &mut dir_state,
    );
}

fn folder_label(path: &str, node: Option<&FolderNode>) -> String {
    if path == "/" {
        "/".into()
    } else {
        node.map(|n| n.name.clone())
            .unwrap_or_else(|| path.trim_start_matches('/').to_string())
    }
}

fn render_jump_overlay(f: &mut Frame, jump: &JumpState) {
    let area = centered_rect(50, 12, f.area());
    f.render_widget(Clear, area);

    let block = Block::bordered()
        .title(" Jump to ")
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [input_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    f.render_widget(
        Paragraph::new(format!("path:{}{}", jump.input, "█")),
        input_area,
    );

    let items: Vec<ListItem> = jump
        .candidates
        .iter()
        .map(|candidate| ListItem::new(candidate.as_str()))
        .collect();
    let mut jump_state = ListState::default();
    jump_state.select(if jump.candidates.is_empty() {
        None
    } else {
        Some(jump.sel)
    });

    f.render_stateful_widget(
        List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        list_area,
        &mut jump_state,
    );
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = area.width * percent_x / 100;
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
