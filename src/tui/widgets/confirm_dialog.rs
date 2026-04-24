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
        // Fixed height: 5 rows (2 borders + message + gap + hints), width 50%
        let area = centered_fixed(50, 5, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Yellow));
        let [msg_area, _, hints_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(block.inner(area));
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(self.message), msg_area);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("y", Style::default().fg(Color::Yellow)),
                Span::raw(":Yes   "),
                Span::styled("n/ESC", Style::default().fg(Color::Yellow)),
                Span::raw(":No"),
            ])),
            hints_area,
        );
    }
}

// Center a popup with a fixed height (rows) and percentage width.
#[allow(dead_code)]
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
