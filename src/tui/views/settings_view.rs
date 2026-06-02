use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::config::store::PuttyCompatMetadata;
use crate::putty_shim::PuttyShimStatus;
use crate::tui::widgets::status_bar::StatusBar;

pub fn settings_lines(status: &PuttyShimStatus, metadata: &PuttyCompatMetadata) -> Vec<String> {
    let enabled = if status.enabled { "on" } else { "off" };
    let supported = if status.supported {
        "supported"
    } else {
        "not supported"
    };
    let shim_path = status
        .shim_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "not installed".into());

    let mut lines = vec![
        "Settings".into(),
        format!("PuTTY compatibility launcher: {enabled}"),
        format!("Platform support: {supported}"),
        format!("Shim path: {shim_path}"),
        format!("Status: {}", status.message),
        format!("Next: {}", status.next_step),
    ];

    if let Some(error) = &metadata.last_error {
        lines.push(format!("Last error: {error}"));
    }

    lines
}

pub fn render(f: &mut Frame, status: &PuttyShimStatus, metadata: &PuttyCompatMetadata) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    let text = settings_lines(status, metadata)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                Line::from(Span::styled(
                    line,
                    Style::default().fg(Color::Cyan),
                ))
            } else {
                Line::from(line)
            }
        })
        .collect::<Vec<_>>();

    f.render_widget(
        Paragraph::new(text).block(Block::bordered().title(" sush settings ")),
        chunks[0],
    );
    f.render_widget(
        StatusBar {
            hints: &[("Space", "Toggle"), ("Esc/q", "Back")],
            transfer_badge: None,
        },
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_lines_show_platform_and_next_step() {
        let metadata = crate::config::store::PuttyCompatMetadata::default();
        let status = crate::putty_shim::status_for_platform(
            &metadata,
            std::path::Path::new("/home/me/.config/sush"),
            crate::putty_shim::Platform::MacOs,
        );

        let lines = settings_lines(&status, &metadata);

        assert!(lines.iter().any(|line| line.contains("PuTTY compatibility launcher")));
        assert!(lines.iter().any(|line| line.contains("not supported")));
        assert!(lines.iter().any(|line| line.contains("Next")));
    }
}
