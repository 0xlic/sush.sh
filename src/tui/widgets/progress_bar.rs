#![allow(dead_code)]

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
        let label = build_progress_label(self.verb, self.progress);
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

fn build_progress_label(verb: &str, progress: &TransferProgress) -> String {
    let file_position = if progress.current_file_index == 0 {
        1
    } else {
        progress.current_file_index
    };
    if progress.total_files > 1 {
        format!(
            "{} {}/{} {}  {}/{}",
            verb,
            file_position,
            progress.total_files,
            progress.filename,
            human(progress.transferred_bytes),
            human(progress.total_bytes),
        )
    } else {
        format!(
            "{} {}  {}/{}",
            verb,
            progress.filename,
            human(progress.transferred_bytes),
            human(progress.total_bytes),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sftp::transfer::TransferState;

    #[test]
    fn recursive_progress_label_includes_file_counts() {
        let progress = TransferProgress {
            filename: "a.txt".into(),
            total_bytes: 10,
            transferred_bytes: 4,
            state: TransferState::InProgress,
            current_file_index: 2,
            total_files: 5,
        };

        assert_eq!(
            build_progress_label("Uploading", &progress),
            "Uploading 2/5 a.txt  4.0 B/10.0 B"
        );
    }
}
