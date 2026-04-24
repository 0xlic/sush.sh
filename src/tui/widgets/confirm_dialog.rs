use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

#[allow(dead_code)]
pub struct ConfirmDialog<'a> {
    pub title: &'a str,
    pub message: &'a str,
}

#[allow(dead_code)]
impl<'a> ConfirmDialog<'a> {
    pub fn new(title: &'a str, message: &'a str) -> Self {
        Self { title, message }
    }

    pub fn render(&self, f: &mut Frame) {
        let area = centered_rect(50, 25, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow));
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(block.inner(area));
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(self.message), inner[0]);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("y", Style::default().fg(Color::Yellow)),
                Span::raw(":Yes   "),
                Span::styled("n/ESC", Style::default().fg(Color::Yellow)),
                Span::raw(":No"),
            ])),
            inner[2],
        );
    }
}

#[allow(dead_code)]
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
