use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, StatefulWidget, Widget};

use crate::config::host::Host;

pub struct HostList<'a> {
    pub hosts: &'a [Host],
    pub indices: &'a [usize],
    pub focused: bool,
    pub show_selection: bool,
    pub probe: Option<Option<bool>>, // None=no dot, Some(None)=gray, Some(Some(bool))=colored
    pub status_msg: Option<&'a str>, // error / info shown in description row (red)
}

impl<'a> StatefulWidget for HostList<'a> {
    type State = ListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = if self.focused {
            Block::bordered()
                .title(" Hosts ")
                .border_style(Style::default().fg(Color::Cyan))
        } else {
            Block::bordered().title(" Hosts ")
        };
        let inner = block.inner(area);
        block.render(area, buf);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        // Header — 2-space indent aligned with list items.
        let header = Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<16}", "Alias"),
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<20}", "Address"),
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Tags",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        Paragraph::new(header).render(sections[0], buf);

        // Description row: shows ● + error/description text.
        // Error message (red) takes priority over description; probe color applied to dot.
        let selected = if self.show_selection {
            state.selected()
        } else {
            None
        };
        let desc = selected
            .and_then(|sel| self.indices.get(sel))
            .map(|&i| self.hosts[i].description.as_str())
            .unwrap_or("");
        let text = self.status_msg.unwrap_or(desc);
        let show_probe = self.show_selection && self.probe.is_some();
        let dot_color = match self.probe {
            Some(None) => Color::DarkGray,
            Some(Some(true)) => Color::Green,
            Some(Some(false)) => Color::Red,
            None => Color::Yellow,
        };
        let show_row = show_probe || !text.is_empty();
        let desc_spans = if show_row {
            let text_style = if self.status_msg.is_some() {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            let mut spans = if show_probe {
                vec![Span::styled("  ● ", Style::default().fg(dot_color))]
            } else {
                vec![Span::raw("  ")]
            };
            if !text.is_empty() {
                spans.push(Span::styled(text, text_style));
            }
            Line::from(spans)
        } else {
            Line::from(vec![])
        };
        Paragraph::new(desc_spans).render(sections[2], buf);

        let _selected_pos = state.selected();
        let items: Vec<ListItem> = self
            .indices
            .iter()
            .map(|&host_idx| {
                let h = &self.hosts[host_idx];
                let tags = h.tags.join(",");
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{:<16}", h.alias), Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{:<20}", h.hostname)),
                    Span::styled(tags, Style::default().fg(Color::Yellow)),
                ]))
            })
            .collect();

        let mut list = List::new(items);
        if self.show_selection {
            list = list.highlight_style(
                Style::default()
                    .add_modifier(Modifier::REVERSED)
                    .fg(Color::Yellow),
            );
        }

        let mut render_state = ListState::default();
        render_state.select(selected);

        StatefulWidget::render(list, sections[1], buf, &mut render_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::host::{Host, HostSource};

    fn host_with_desc(alias: &str, desc: &str) -> Host {
        Host {
            id: alias.into(),
            alias: alias.into(),
            hostname: alias.into(),
            port: 22,
            user: "root".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec![],
            description: desc.into(),
            source: HostSource::SshConfig,
        }
    }

    fn render_content(widget: HostList<'_>, state: &mut ListState) -> String {
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        StatefulWidget::render(widget, area, &mut buf, state);
        buf.content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect::<Vec<_>>()
            .join("")
            .replace(' ', "")
    }

    #[test]
    fn description_renders_when_selected() {
        let hosts = vec![host_with_desc(
            "192.168.7.2",
            "West District dev environment",
        )];
        let indices = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));
        let content = render_content(
            HostList {
                hosts: &hosts,
                indices: &indices,
                focused: false,
                show_selection: true,
                probe: None,
                status_msg: None,
            },
            &mut state,
        );
        assert!(
            content.contains("WestDistrictdevenvironment"),
            "description not found in buffer:\n{content}"
        );
    }

    #[test]
    fn description_does_not_render_when_selection_hidden() {
        let hosts = vec![host_with_desc(
            "192.168.7.2",
            "West District dev environment",
        )];
        let indices = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));

        let content = render_content(
            HostList {
                hosts: &hosts,
                indices: &indices,
                focused: false,
                show_selection: false,
                probe: None,
                status_msg: None,
            },
            &mut state,
        );

        assert!(
            !content.contains("WestDistrictdevenvironment"),
            "description should be hidden when selection is disabled:\n{content}"
        );
    }

    #[test]
    fn probe_does_not_render_when_selection_hidden() {
        let hosts = vec![host_with_desc("192.168.7.2", "")];
        let indices = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));

        let content = render_content(
            HostList {
                hosts: &hosts,
                indices: &indices,
                focused: false,
                show_selection: false,
                probe: Some(Some(true)),
                status_msg: None,
            },
            &mut state,
        );

        assert!(
            !content.contains('●'),
            "probe dot should be hidden when selection is disabled:\n{content}"
        );
    }
}
