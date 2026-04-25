use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

pub struct SearchInput<'a> {
    pub query: &'a str,
    pub focused: bool,
    pub prefix: Option<&'a str>,
}

impl<'a> Widget for SearchInput<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let cursor = if self.focused { "█" } else { "" };
        let block = if self.focused {
            Block::bordered()
                .title(" > ")
                .border_style(Style::default().fg(Color::Cyan))
        } else {
            Block::bordered().title(" > ")
        };

        let mut spans = Vec::new();
        if let Some(prefix) = self.prefix {
            spans.push(Span::styled(
                format!("path:{prefix} "),
                Style::default().fg(Color::DarkGray),
            ));
        }

        if self.query.is_empty() && !self.focused {
            spans.push(Span::styled(
                "Search...",
                Style::default().add_modifier(Modifier::DIM),
            ));
        } else {
            spans.push(Span::raw(self.query));
            if self.focused {
                spans.push(Span::raw(cursor));
            }
        }

        Paragraph::new(Line::from(spans))
            .block(block)
            .render(area, buf);
    }
}
