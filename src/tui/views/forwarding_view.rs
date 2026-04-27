use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::app::{ForwardingFocus, ForwardingViewState};
use crate::config::host::{ForwardKind, Host};
use crate::tunnel::ipc::{ForwardState, ForwardStatus};

const HINTS: [(&str, &str); 6] = [
    ("Tab", "Switch"),
    ("Enter", "Start/Stop"),
    ("n", "New"),
    ("e", "Edit"),
    ("d", "Delete"),
    ("q", "Back"),
];

pub fn render(f: &mut Frame, state: &mut ForwardingViewState, hosts: &[Host]) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(10)])
        .split(chunks[0]);

    render_host_panel(f, panels[0], state, hosts);
    render_rule_panel(f, panels[1], state, hosts);
    render_hints(f, chunks[1]);
}

fn host_has_active(host: &Host, statuses: &[ForwardStatus]) -> bool {
    host.forwards.iter().any(|rule| {
        statuses
            .iter()
            .find(|status| status.id == rule.id)
            .map(|status| status.state.is_active())
            .unwrap_or(false)
    })
}

fn render_host_panel(f: &mut Frame, area: Rect, state: &mut ForwardingViewState, hosts: &[Host]) {
    let focused = state.focus == ForwardingFocus::HostList;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let block = Block::bordered()
        .title(" Hosts ")
        .border_style(border_style);

    let items: Vec<ListItem> = state
        .host_indices
        .iter()
        .map(|&index| {
            let host = &hosts[index];
            let bullet = if host_has_active(host, &state.statuses) {
                Span::styled("● ", Style::default().fg(Color::Green))
            } else {
                Span::raw("  ")
            };
            ListItem::new(Line::from(vec![bullet, Span::raw(host.alias.clone())]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(list, area, &mut state.host_list_state);
}

fn rule_state_style(state: &ForwardState) -> Style {
    match state {
        ForwardState::Running => Style::default().fg(Color::Green),
        ForwardState::Reconnecting => Style::default().fg(Color::Yellow),
        ForwardState::Error => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::DarkGray),
    }
}

fn render_rule_panel(f: &mut Frame, area: Rect, state: &mut ForwardingViewState, hosts: &[Host]) {
    let focused = state.focus == ForwardingFocus::RuleList;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let selected_host = state.selected_host_idx().map(|index| &hosts[index]);
    let title = selected_host
        .map(|host| format!(" Rules: {} ", host.alias))
        .unwrap_or_else(|| " Rules ".into());
    let block = Block::bordered().title(title).border_style(border_style);

    let items: Vec<ListItem> = match selected_host {
        None => vec![ListItem::new("(no host selected)")],
        Some(host) => host
            .forwards
            .iter()
            .map(|rule| {
                let status = state.statuses.iter().find(|status| status.id == rule.id);
                let forward_state = status
                    .map(|status| &status.state)
                    .unwrap_or(&ForwardState::Stopped);
                let retry_count = status.map(|status| status.retry_count).unwrap_or(0);
                let error_msg = status
                    .and_then(|status| status.error.as_deref())
                    .unwrap_or("");

                let bullet = if forward_state.is_active() {
                    Span::styled("● ", Style::default().fg(Color::Green))
                } else {
                    Span::raw("  ")
                };

                let kind_label = match rule.kind {
                    ForwardKind::Local => "L",
                    ForwardKind::Remote => "R",
                    ForwardKind::Dynamic => "D",
                };
                let route = match rule.kind {
                    ForwardKind::Dynamic => format!("{}", rule.local_port),
                    ForwardKind::Remote => {
                        format!("{} <- {}", rule.remote_port.unwrap_or(0), rule.local_port)
                    }
                    ForwardKind::Local => format!(
                        "{} -> {}:{}",
                        rule.local_port,
                        rule.remote_host.as_deref().unwrap_or("?"),
                        rule.remote_port.unwrap_or(0)
                    ),
                };

                let state_label = match forward_state {
                    ForwardState::Stopped => "[stopped]".to_string(),
                    ForwardState::Connecting => "[connecting]".to_string(),
                    ForwardState::Running => "[running]".to_string(),
                    ForwardState::Reconnecting => format!("[reconnecting {retry_count}/5]"),
                    ForwardState::Error => {
                        if error_msg.is_empty() {
                            "[error]".to_string()
                        } else {
                            format!("[error: {error_msg}]")
                        }
                    }
                };

                ListItem::new(Line::from(vec![
                    bullet,
                    Span::raw(format!("{:<18}", rule.name)),
                    Span::raw(format!("{kind_label}  {:<30}", route)),
                    Span::styled(state_label, rule_state_style(forward_state)),
                ]))
            })
            .collect(),
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(list, area, &mut state.rule_list_state);
}

fn render_hints(f: &mut Frame, area: Rect) {
    let spans: Vec<Span> = HINTS
        .iter()
        .flat_map(|(key, action)| {
            vec![
                Span::styled(*key, Style::default().fg(Color::Cyan)),
                Span::raw(format!(":{action}  ")),
            ]
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
