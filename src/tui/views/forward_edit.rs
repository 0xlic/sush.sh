use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::config::host::{ForwardKind, ForwardRule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Name,
    Kind,
    LocalPort,
    RemoteHost,
    RemotePort,
    AutoStart,
}

static KIND_LABELS: &[&str] = &["Local", "Remote", "Dynamic"];

impl EditField {
    pub fn next(self, kind_idx: usize) -> Self {
        match self {
            Self::Name => Self::Kind,
            Self::Kind => Self::LocalPort,
            Self::LocalPort => {
                if kind_idx == 2 {
                    Self::AutoStart
                } else {
                    Self::RemoteHost
                }
            }
            Self::RemoteHost => Self::RemotePort,
            Self::RemotePort => Self::AutoStart,
            Self::AutoStart => Self::Name,
        }
    }

    pub fn prev(self, kind_idx: usize) -> Self {
        match self {
            Self::Name => Self::AutoStart,
            Self::Kind => Self::Name,
            Self::LocalPort => Self::Kind,
            Self::RemoteHost => Self::LocalPort,
            Self::RemotePort => Self::RemoteHost,
            Self::AutoStart => {
                if kind_idx == 2 {
                    Self::LocalPort
                } else {
                    Self::RemotePort
                }
            }
        }
    }
}

pub struct ForwardEditState {
    pub host_id: String,
    pub host_alias: String,
    pub forward_id: Option<String>,
    pub name: String,
    pub kind_idx: usize,
    pub local_port: String,
    pub remote_host: String,
    pub remote_port: String,
    pub auto_start: bool,
    pub focused: EditField,
    pub error: Option<String>,
}

impl ForwardEditState {
    pub fn new(host_id: String, host_alias: String) -> Self {
        Self {
            host_id,
            host_alias,
            forward_id: None,
            name: String::new(),
            kind_idx: 0,
            local_port: String::new(),
            remote_host: String::new(),
            remote_port: String::new(),
            auto_start: false,
            focused: EditField::Name,
            error: None,
        }
    }

    pub fn from_rule(host_id: String, host_alias: String, rule: &ForwardRule) -> Self {
        Self {
            host_id,
            host_alias,
            forward_id: Some(rule.id.clone()),
            name: rule.name.clone(),
            kind_idx: match rule.kind {
                ForwardKind::Local => 0,
                ForwardKind::Remote => 1,
                ForwardKind::Dynamic => 2,
            },
            local_port: rule.local_port.to_string(),
            remote_host: rule.remote_host.clone().unwrap_or_default(),
            remote_port: rule
                .remote_port
                .map(|port| port.to_string())
                .unwrap_or_default(),
            auto_start: rule.auto_start,
            focused: EditField::Name,
            error: None,
        }
    }

    pub fn validate(&self) -> Result<ForwardRule, String> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err("Name is required".into());
        }
        let local_port: u16 = self
            .local_port
            .trim()
            .parse()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| "Local Port must be 1-65535".to_string())?;
        let kind = match self.kind_idx {
            0 => ForwardKind::Local,
            1 => ForwardKind::Remote,
            _ => ForwardKind::Dynamic,
        };
        let (remote_host, remote_port) = if self.kind_idx == 2 {
            (None, None)
        } else {
            let remote_host = self.remote_host.trim().to_string();
            if remote_host.is_empty() {
                return Err("Remote Host is required".into());
            }
            let remote_port: u16 = self
                .remote_port
                .trim()
                .parse()
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| "Remote Port must be 1-65535".to_string())?;
            (Some(remote_host), Some(remote_port))
        };
        let id = self
            .forward_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(ForwardRule {
            id,
            name,
            kind,
            local_port,
            remote_host,
            remote_port,
            auto_start: self.auto_start,
        })
    }
}

fn field_style(focused: EditField, field: EditField) -> Style {
    if focused == field {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

pub fn render(f: &mut Frame, state: &ForwardEditState) {
    let area = centered_rect(60, 12, f.area());
    f.render_widget(Clear, area);

    let title = if state.forward_id.is_none() {
        " New Forward Rule "
    } else {
        " Edit Forward Rule "
    };
    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(block, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    let is_dynamic = state.kind_idx == 2;
    let row_count = if is_dynamic { 6 } else { 8 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); row_count])
        .split(inner);

    let mut row = 0usize;
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(format!("{:<14}", "Host:")),
            Span::styled(&state.host_alias, Style::default().fg(Color::DarkGray)),
        ])),
        rows[row],
    );
    row += 1;

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:<14}", "Name:"),
                field_style(state.focused, EditField::Name),
            ),
            Span::raw(&state.name),
            if state.focused == EditField::Name {
                Span::styled("█", Style::default().fg(Color::Cyan))
            } else {
                Span::raw("")
            },
        ])),
        rows[row],
    );
    row += 1;

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:<14}", "Kind:"),
                field_style(state.focused, EditField::Kind),
            ),
            Span::styled(
                format!("[{}]", KIND_LABELS[state.kind_idx]),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  ←/→ cycle"),
        ])),
        rows[row],
    );
    row += 1;

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:<14}", "Local Port:"),
                field_style(state.focused, EditField::LocalPort),
            ),
            Span::raw(&state.local_port),
            if state.focused == EditField::LocalPort {
                Span::styled("█", Style::default().fg(Color::Cyan))
            } else {
                Span::raw("")
            },
        ])),
        rows[row],
    );
    row += 1;

    if !is_dynamic {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{:<14}", "Remote Host:"),
                    field_style(state.focused, EditField::RemoteHost),
                ),
                Span::raw(&state.remote_host),
                if state.focused == EditField::RemoteHost {
                    Span::styled("█", Style::default().fg(Color::Cyan))
                } else {
                    Span::raw("")
                },
            ])),
            rows[row],
        );
        row += 1;

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{:<14}", "Remote Port:"),
                    field_style(state.focused, EditField::RemotePort),
                ),
                Span::raw(&state.remote_port),
                if state.focused == EditField::RemotePort {
                    Span::styled("█", Style::default().fg(Color::Cyan))
                } else {
                    Span::raw("")
                },
            ])),
            rows[row],
        );
        row += 1;
    }

    let auto_label = if state.auto_start { "[x]" } else { "[ ]" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:<14}", "Auto Start:"),
                field_style(state.focused, EditField::AutoStart),
            ),
            Span::styled(auto_label, Style::default().fg(Color::Yellow)),
            Span::raw("  Space toggle"),
        ])),
        rows[row],
    );
    row += 1;

    let hint_text = if let Some(error) = &state.error {
        Span::styled(error.as_str(), Style::default().fg(Color::Red))
    } else {
        Span::raw("Tab:next  ←/→:cycle kind  Space:toggle  Ctrl-S:save  Esc:cancel")
    };
    if row < rows.len() {
        f.render_widget(Paragraph::new(Line::from(vec![hint_text])), rows[row]);
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup_width = area.width.min(width);
    let popup_height = area.height.min(height);
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_dynamic_rule_skips_remote_fields() {
        let mut state = ForwardEditState::new("h1".into(), "host".into());
        state.name = "SOCKS".into();
        state.kind_idx = 2;
        state.local_port = "1080".into();

        let rule = state.validate().unwrap();

        assert_eq!(rule.kind, ForwardKind::Dynamic);
        assert_eq!(rule.local_port, 1080);
        assert!(rule.remote_host.is_none());
        assert!(rule.remote_port.is_none());
    }

    #[test]
    fn validate_local_rule_requires_remote_fields() {
        let mut state = ForwardEditState::new("h1".into(), "host".into());
        state.name = "Web".into();
        state.kind_idx = 0;
        state.local_port = "8080".into();

        let error = state.validate().unwrap_err();

        assert_eq!(error, "Remote Host is required");
    }
}
