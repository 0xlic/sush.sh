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
    pub probe: Option<Option<bool>>, // None=no dot, Some(None)=gray, Some(Some(bool))=colored
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

        // Header — 4-space indent to align with indicator column.
        let header = Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!("{:<16}", "Alias"),
                Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<20}", "Address"),
                Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Tags",
                Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD),
            ),
        ]);
        Paragraph::new(header).render(sections[0], buf);

        // Description row for selected host.
        let desc = state
            .selected()
            .and_then(|sel| self.indices.get(sel))
            .map(|&i| self.hosts[i].description.as_str())
            .unwrap_or("");
        let desc_spans = if desc.is_empty() {
            Line::from(vec![])
        } else {
            Line::from(vec![
                Span::styled("  ● ", Style::default().fg(Color::Yellow)),
                Span::raw(desc),
            ])
        };
        Paragraph::new(desc_spans).render(sections[2], buf);

        let selected_pos = state.selected();
        let items: Vec<ListItem> = self
            .indices
            .iter()
            .map(|&host_idx| {
                let h = &self.hosts[host_idx];
                let tags = h.tags.join(",");
                ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(format!("{:<16}", h.alias), Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{:<20}", h.hostname)),
                    Span::styled(tags, Style::default().fg(Color::Yellow)),
                ]))
            })
            .collect();

        StatefulWidget::render(
            List::new(items).highlight_style(
                Style::default()
                    .add_modifier(Modifier::REVERSED)
                    .fg(Color::Yellow),
            ),
            sections[1],
            buf,
            state,
        );

        // Draw probe dot directly to buffer after List renders, bypassing highlight_style color
        // interference. The dot is written at the selected row's position in the list area.
        if let (Some(sel), Some(dot_color)) = (
            selected_pos,
            match self.probe {
                Some(None) => Some(Color::DarkGray),
                Some(Some(true)) => Some(Color::Green),
                Some(Some(false)) => Some(Color::Red),
                None => None,
            },
        ) {
            let y = sections[1].y + sel as u16;
            let x = sections[1].x + 2; // 2-space indent before dot
            if y < sections[1].bottom() && x < sections[1].right() {
                let cell = &mut buf[(x, y)];
                cell.set_symbol("●");
                cell.set_fg(dot_color);
                // Ensure the dot isn't reversed by the highlight style
                cell.set_style(cell.style().remove_modifier(Modifier::REVERSED));
            }
        }
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

    #[test]
    fn description_renders_when_selected() {
        let hosts = vec![host_with_desc(
            "192.168.7.2",
            "West District dev environment",
        )];
        let indices = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));

        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);

        StatefulWidget::render(
            HostList {
                hosts: &hosts,
                indices: &indices,
                focused: false,
                probe: None,
            },
            area,
            &mut buf,
            &mut state,
        );

        // ratatui stores double-width characters with an extra placeholder cell; strip spaces before asserting.
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect::<Vec<_>>()
            .join("")
            .replace(' ', "");
        assert!(
            content.contains("WestDistrictdevenvironment"),
            "description not found in buffer:\n{content}"
        );
    }
}
