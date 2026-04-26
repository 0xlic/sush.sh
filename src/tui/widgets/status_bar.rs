use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferBadge {
    pub direction_symbol: &'static str,
    pub current_index: usize,
    pub total_count: usize,
    pub percent: u8,
}

impl TransferBadge {
    pub fn to_text(&self) -> String {
        format!(
            "{} {}/{} {}%",
            self.direction_symbol, self.current_index, self.total_count, self.percent
        )
    }
}

pub struct StatusBar<'a> {
    pub hints: &'a [(&'a str, &'a str)],
    pub transfer_badge: Option<&'a TransferBadge>,
}

pub fn build_status_line(
    hints: &[(&str, &str)],
    transfer_badge: Option<&TransferBadge>,
    width: u16,
) -> String {
    let left = hints
        .iter()
        .map(|(key, action)| format!("{key}:{action}"))
        .collect::<Vec<_>>()
        .join("  ");
    build_right_aligned_line(&left, transfer_badge, width)
}

pub fn build_status_message_line(
    status: &str,
    transfer_badge: Option<&TransferBadge>,
    width: u16,
) -> String {
    build_right_aligned_line(status, transfer_badge, width)
}

fn build_right_aligned_line(
    left: &str,
    transfer_badge: Option<&TransferBadge>,
    width: u16,
) -> String {
    let Some(badge) = transfer_badge else {
        return left.to_string();
    };
    let right = badge.to_text();
    let width = width as usize;
    let left_len = left.chars().count();
    let right_len = right.chars().count();
    if width <= right_len {
        return right;
    }
    if left_len + 1 + right_len >= width {
        return format!("{left} {right}");
    }
    format!("{left}{}{right}", " ".repeat(width - left_len - right_len))
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let text = build_status_line(self.hints, self.transfer_badge, area.width);
        let mut spans: Vec<Span> = Vec::new();
        let mut pending = text.as_str();
        while let Some(idx) = pending.find(':') {
            let (before, after_colon) = pending.split_at(idx);
            if let Some(key_start) = before.rfind("  ").map(|pos| pos + 2) {
                spans.push(Span::raw(&before[..key_start]));
                spans.push(Span::styled(
                    &before[key_start..],
                    Style::default().fg(Color::Yellow),
                ));
            } else if !before.is_empty() {
                spans.push(Span::styled(before, Style::default().fg(Color::Yellow)));
            }
            spans.push(Span::raw(":"));
            pending = &after_colon[1..];
            if let Some(next_sep) = pending.find("  ") {
                spans.push(Span::raw(&pending[..next_sep]));
                pending = &pending[next_sep..];
            } else {
                spans.push(Span::raw(pending));
                pending = "";
            }
        }
        if !pending.is_empty() {
            spans.push(Span::raw(pending));
        }
        Paragraph::new(Line::from(spans)).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_transfer_badge_renders_expected_text() {
        let badge = TransferBadge {
            direction_symbol: "↑",
            current_index: 2,
            total_count: 10,
            percent: 37,
        };

        assert_eq!(badge.to_text(), "↑ 2/10 37%");
    }

    #[test]
    fn status_bar_keeps_hints_when_transfer_badge_is_present() {
        let badge = TransferBadge {
            direction_symbol: "↓",
            current_index: 2,
            total_count: 10,
            percent: 37,
        };

        let line = build_status_line(&[("d", "Download"), ("u", "Upload")], Some(&badge), 40);

        assert!(line.contains("d:Download"));
        assert!(line.contains("u:Upload"));
        assert!(line.contains("↓ 2/10 37%"));
    }

    #[test]
    fn status_message_keeps_transfer_badge_when_present() {
        let badge = TransferBadge {
            direction_symbol: "↑",
            current_index: 1,
            total_count: 3,
            percent: 12,
        };

        let line = build_status_message_line("Queued batch upload", Some(&badge), 40);

        assert!(line.contains("Queued batch upload"));
        assert!(line.contains("↑ 1/3 12%"));
    }
}
