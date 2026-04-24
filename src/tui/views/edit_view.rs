use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Paragraph};

use crate::config::host::{Host, HostSource};
use crate::tui::widgets::status_bar::StatusBar;
use crate::tui::widgets::tag_editor::{TagEditor, TagEditorState};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Alias,
    Hostname,
    Port,
    User,
    Identity,
    ProxyJump,
    Tags,
    Description,
}

#[allow(dead_code)]
impl EditField {
    pub fn next(self) -> Self {
        match self {
            Self::Alias => Self::Hostname,
            Self::Hostname => Self::Port,
            Self::Port => Self::User,
            Self::User => Self::Identity,
            Self::Identity => Self::ProxyJump,
            Self::ProxyJump => Self::Tags,
            Self::Tags => Self::Description,
            Self::Description => Self::Alias,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Alias => Self::Description,
            Self::Hostname => Self::Alias,
            Self::Port => Self::Hostname,
            Self::User => Self::Port,
            Self::Identity => Self::User,
            Self::ProxyJump => Self::Identity,
            Self::Tags => Self::ProxyJump,
            Self::Description => Self::Tags,
        }
    }
}

#[allow(dead_code)]
pub struct EditDraft {
    pub is_new: bool,
    pub original_id: Option<String>,
    pub alias: String,
    pub hostname: String,
    pub port: String,
    pub user: String,
    pub identity: String,
    pub proxy_jump: String,
    pub description: String,
    pub tags: TagEditorState,
    pub focused_field: EditField,
    pub error: Option<String>,
}

#[allow(dead_code)]
impl EditDraft {
    pub fn new_host() -> Self {
        Self {
            is_new: true,
            original_id: None,
            alias: String::new(),
            hostname: String::new(),
            port: "22".into(),
            user: String::new(),
            identity: String::new(),
            proxy_jump: String::new(),
            description: String::new(),
            tags: TagEditorState::new(vec![]),
            focused_field: EditField::Alias,
            error: None,
        }
    }

    pub fn from_host(host: &Host) -> Self {
        Self {
            is_new: false,
            original_id: Some(host.id.clone()),
            alias: host.alias.clone(),
            hostname: host.hostname.clone(),
            port: host.port.to_string(),
            user: host.user.clone(),
            identity: host
                .identity_files
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            proxy_jump: host.proxy_jump.clone().unwrap_or_default(),
            description: host.description.clone(),
            tags: TagEditorState::new(host.tags.clone()),
            focused_field: EditField::Alias,
            error: None,
        }
    }

    pub fn active_text_mut(&mut self) -> Option<&mut String> {
        match self.focused_field {
            EditField::Alias => Some(&mut self.alias),
            EditField::Hostname => Some(&mut self.hostname),
            EditField::Port => Some(&mut self.port),
            EditField::User => Some(&mut self.user),
            EditField::Identity => Some(&mut self.identity),
            EditField::ProxyJump => Some(&mut self.proxy_jump),
            EditField::Description => Some(&mut self.description),
            EditField::Tags => None,
        }
    }
}

#[allow(dead_code)]
pub fn validate(draft: &EditDraft) -> Result<(), String> {
    if draft.alias.trim().is_empty() {
        return Err("Alias is required".into());
    }
    if draft.hostname.trim().is_empty() {
        return Err("Hostname is required".into());
    }
    let port: u16 = draft
        .port
        .trim()
        .parse()
        .ok()
        .filter(|&p: &u16| p > 0)
        .ok_or("Port must be between 1 and 65535")?;
    let _ = port;
    Ok(())
}

#[allow(dead_code)]
pub fn build_host(draft: &mut EditDraft) -> Host {
    draft.tags.commit_pending();
    let port: u16 = draft.port.trim().parse().unwrap_or(22);
    let identity_files = if draft.identity.trim().is_empty() {
        vec![]
    } else {
        vec![PathBuf::from(draft.identity.trim())]
    };
    let proxy_jump = if draft.proxy_jump.trim().is_empty() {
        None
    } else {
        Some(draft.proxy_jump.trim().to_string())
    };
    Host {
        id: if draft.is_new {
            draft.alias.trim().to_string()
        } else {
            draft
                .original_id
                .clone()
                .unwrap_or_else(|| draft.alias.trim().to_string())
        },
        alias: draft.alias.trim().to_string(),
        hostname: draft.hostname.trim().to_string(),
        port,
        user: draft.user.trim().to_string(),
        identity_files,
        proxy_jump,
        tags: draft.tags.tags.clone(),
        description: draft.description.trim().to_string(),
        source: HostSource::Manual,
    }
}

#[allow(dead_code)]
pub fn render(f: &mut Frame, draft: &EditDraft, all_tags: &[String]) {
    let [form_area, status_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let title = if draft.is_new { " New Host " } else { " Edit Host " };
    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(form_area);
    f.render_widget(block, form_area);

    // 8 fields × 2 rows each (field row + gap row)
    let rows: Vec<Constraint> = (0..8)
        .flat_map(|_| [Constraint::Length(1), Constraint::Length(1)])
        .collect();
    let cells = Layout::vertical(rows).split(inner);

    let field_defs: &[(&str, EditField)] = &[
        ("Alias      ", EditField::Alias),
        ("Hostname   ", EditField::Hostname),
        ("Port       ", EditField::Port),
        ("User       ", EditField::User),
        ("Identity   ", EditField::Identity),
        ("Proxy Jump ", EditField::ProxyJump),
        ("Tags       ", EditField::Tags),
        ("Description", EditField::Description),
    ];

    for (i, (label, field)) in field_defs.iter().enumerate() {
        let row_area = cells[i * 2];
        let focused = draft.focused_field == *field;
        let label_style = if focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let [label_area, value_area] =
            Layout::horizontal([Constraint::Length(13), Constraint::Min(1)]).areas(row_area);

        f.render_widget(
            Paragraph::new(Span::styled(*label, label_style)),
            label_area,
        );

        if *field == EditField::Tags {
            f.render_widget(TagEditor { state: &draft.tags, focused }, value_area);
            if focused && !draft.tags.candidates.is_empty() {
                render_candidates(f, value_area, &draft.tags.candidates, draft.tags.candidate_sel);
            }
        } else {
            let value = field_value(draft, *field);
            let display = if focused {
                format!("{value}█")
            } else {
                value.to_string()
            };
            f.render_widget(Paragraph::new(display), value_area);
        }
    }

    // Error message (shown in the gap row below Alias)
    if let Some(err) = &draft.error {
        f.render_widget(
            Paragraph::new(Span::styled(err.as_str(), Style::default().fg(Color::Red))),
            cells[1],
        );
    }

    let _ = all_tags;

    f.render_widget(
        StatusBar {
            hints: &[
                ("Ctrl-S", "Save"),
                ("ESC", "Cancel"),
                ("Tab/↑↓", "Next field"),
            ],
        },
        status_area,
    );
}

fn field_value(draft: &EditDraft, field: EditField) -> &str {
    match field {
        EditField::Alias => &draft.alias,
        EditField::Hostname => &draft.hostname,
        EditField::Port => &draft.port,
        EditField::User => &draft.user,
        EditField::Identity => &draft.identity,
        EditField::ProxyJump => &draft.proxy_jump,
        EditField::Description => &draft.description,
        EditField::Tags => "",
    }
}

fn render_candidates(f: &mut Frame, anchor: Rect, candidates: &[String], sel: usize) {
    use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};

    let height = candidates.len() as u16 + 2;
    let y = anchor.y.saturating_sub(height);
    if y == anchor.y {
        return; // not enough room
    }
    let popup = Rect {
        x: anchor.x,
        y,
        width: anchor.width.min(30),
        height,
    };

    f.render_widget(Clear, popup);
    let items: Vec<ListItem> = candidates.iter().map(|c| ListItem::new(c.as_str())).collect();
    let mut list_state = ListState::default();
    list_state.select(Some(sel));
    f.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        popup,
        &mut list_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_draft() -> EditDraft {
        EditDraft::new_host()
    }

    #[test]
    fn validate_requires_alias() {
        let mut d = blank_draft();
        d.hostname = "1.2.3.4".into();
        assert!(validate(&d).is_err());
    }

    #[test]
    fn validate_requires_hostname() {
        let mut d = blank_draft();
        d.alias = "my-host".into();
        assert!(validate(&d).is_err());
    }

    #[test]
    fn validate_rejects_bad_port() {
        let mut d = blank_draft();
        d.alias = "h".into();
        d.hostname = "h".into();
        d.port = "999999".into();
        assert!(validate(&d).is_err());
    }

    #[test]
    fn validate_accepts_valid_draft() {
        let mut d = blank_draft();
        d.alias = "my-host".into();
        d.hostname = "1.2.3.4".into();
        assert!(validate(&d).is_ok());
    }

    #[test]
    fn from_host_populates_fields() {
        use crate::config::host::HostSource;
        let h = Host {
            id: "web".into(),
            alias: "web".into(),
            hostname: "10.0.0.1".into(),
            port: 2222,
            user: "deploy".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec!["prod".into()],
            description: "test".into(),
            source: HostSource::Manual,
        };
        let d = EditDraft::from_host(&h);
        assert_eq!(d.alias, "web");
        assert_eq!(d.port, "2222");
        assert_eq!(d.tags.tags, vec!["prod"]);
        assert!(!d.is_new);
    }

    #[test]
    fn build_host_returns_correct_host() {
        let mut d = blank_draft();
        d.alias = "srv".into();
        d.hostname = "1.2.3.4".into();
        d.port = "2222".into();
        d.user = "root".into();
        let h = build_host(&mut d);
        assert_eq!(h.alias, "srv");
        assert_eq!(h.port, 2222);
        assert!(matches!(h.source, HostSource::Manual));
    }
}
