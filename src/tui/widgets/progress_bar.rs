use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Gauge, Widget};

use crate::sftp::transfer::TransferProgress;

pub struct ProgressView<'a> {
    pub progress: &'a TransferProgress,
    pub verb: &'a str,
}

impl<'a> Widget for ProgressView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let ratio = if self.progress.total_bytes == 0 {
            0.0
        } else {
            self.progress.transferred_bytes as f64 / self.progress.total_bytes as f64
        };
        let label = format!(
            "{} {}  {}/{}",
            self.verb,
            self.progress.filename,
            human(self.progress.transferred_bytes),
            human(self.progress.total_bytes),
        );
        Gauge::default()
            .gauge_style(Style::default().fg(Color::Black).bg(Color::Green))
            .style(Style::default().fg(Color::White))
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label)
            .render(area, buf);
    }
}

fn human(n: u64) -> String {
    const U: &[&str] = &["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, U[i])
}
