use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, StatefulWidget};

use crate::config::host::Host;

pub struct HostList<'a> {
    pub hosts: &'a [Host],
    pub indices: &'a [usize],
}

impl<'a> StatefulWidget for HostList<'a> {
    type State = ListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let items: Vec<ListItem> = self
            .indices
            .iter()
            .map(|&i| {
                let h = &self.hosts[i];
                let tags = h.tags.join(",");
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{:<16}", h.alias), Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{:<20}", h.hostname)),
                    Span::styled(tags, Style::default().add_modifier(Modifier::DIM)),
                ]))
            })
            .collect();
        StatefulWidget::render(
            List::new(items)
                .block(Block::bordered().title(" 主机 "))
                .highlight_style(
                    Style::default()
                        .add_modifier(Modifier::REVERSED)
                        .fg(Color::Yellow),
                )
                .highlight_symbol("● "),
            area,
            buf,
            state,
        );
    }
}
