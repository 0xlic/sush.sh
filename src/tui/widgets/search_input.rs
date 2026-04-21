use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Widget};

pub struct SearchInput<'a> {
    pub query: &'a str,
    pub focused: bool,
}

impl<'a> Widget for SearchInput<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let cursor = if self.focused { "█" } else { "" };
        let text = if self.query.is_empty() && !self.focused {
            "搜索...".to_string()
        } else {
            format!("{}{}", self.query, cursor)
        };
        let style = if self.query.is_empty() && !self.focused {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default()
        };
        let block = if self.focused {
            Block::bordered()
                .title(" > ")
                .border_style(Style::default().fg(Color::Cyan))
        } else {
            Block::bordered().title(" > ")
        };
        Paragraph::new(Line::from(text).style(style))
            .block(block)
            .render(area, buf);
    }
}
