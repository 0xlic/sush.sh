use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
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
        let area = centered_fixed(60, 6, f.area());
        f.render_widget(Clear, area);
        let masked: String = "*".repeat(self.input.chars().count());
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow));
        let inner_area = block.inner(area);
        f.render_widget(block, area);
        let [input_area, _, hints_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner_area)[..3]
        else {
            return;
        };
        f.render_widget(Paragraph::new(masked), input_area);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(":Confirm  "),
                Span::styled("Esc", Style::default().fg(Color::Yellow)),
                Span::raw(":Cancel"),
            ])),
            hints_area,
        );
    }
}

fn centered_fixed(percent_x: u16, height: u16, r: Rect) -> Rect {
    let vert_pad = r.height.saturating_sub(height) / 2;
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vert_pad),
            Constraint::Length(height),
            Constraint::Min(0),
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
