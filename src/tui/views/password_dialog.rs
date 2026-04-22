use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

pub struct PasswordDialog {
    pub title: String,
    pub input: String,
}

impl PasswordDialog {
    #[allow(dead_code)]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            input: String::new(),
        }
    }

    pub fn render(&self, f: &mut Frame) {
        let area = centered_rect(50, 20, f.area());
        f.render_widget(Clear, area);
        let masked: String = "*".repeat(self.input.chars().count());
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow));
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(block.inner(area));
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(masked), inner[0]);
        f.render_widget(
            Paragraph::new("Enter:Confirm  Esc:Cancel")
                .style(Style::default().fg(Color::DarkGray)),
            inner[1],
        );
    }
}

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
