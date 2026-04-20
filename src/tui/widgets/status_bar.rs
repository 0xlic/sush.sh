use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

pub struct StatusBar<'a> {
    pub hints: &'a [(&'a str, &'a str)],
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut spans: Vec<Span> = Vec::new();
        for (i, (key, action)) in self.hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(*key, Style::default().fg(Color::Yellow)));
            spans.push(Span::raw(":"));
            spans.push(Span::raw(*action));
        }
        Paragraph::new(Line::from(spans)).render(area, buf);
    }
}
