use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

use crate::sftp::transfer::TransferProgress;
use crate::sftp::{PaneSide, SftpPaneState};
use crate::tui::widgets::file_list::FileList;
use crate::tui::widgets::progress_bar::ProgressView;
use crate::tui::widgets::status_bar::StatusBar;

const DUAL_PANE_MIN_WIDTH: u16 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SftpLayoutMode {
    SinglePane,
    DualPane,
}

fn layout_mode_for_width(width: u16) -> SftpLayoutMode {
    if width >= DUAL_PANE_MIN_WIDTH {
        SftpLayoutMode::DualPane
    } else {
        SftpLayoutMode::SinglePane
    }
}

fn pane_focus_style(is_active: bool) -> Style {
    if is_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

pub fn render(
    f: &mut Frame,
    host_alias: &str,
    pane: &mut SftpPaneState,
    transfer: Option<(&'static str, &TransferProgress)>,
    status_msg: Option<&str>,
) {
    let layout_mode = layout_mode_for_width(f.area().width);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    match layout_mode {
        SftpLayoutMode::SinglePane => {
            let (label, path) = match pane.side {
                PaneSide::Local => ("Local", pane.local_path.display().to_string()),
                PaneSide::Remote => ("Remote", pane.remote_path.clone()),
            };
            f.render_widget(
                Paragraph::new(format!(" SFTP: {host_alias}  [{label}] {path}")),
                chunks[0],
            );

            let entries = match pane.side {
                PaneSide::Local => pane.local_entries.as_slice(),
                PaneSide::Remote => pane.remote_entries.as_slice(),
            };
            let list_state = match pane.side {
                PaneSide::Local => &mut pane.local_list_state,
                PaneSide::Remote => &mut pane.remote_list_state,
            };
            f.render_stateful_widget(
                FileList {
                    entries,
                    title: label,
                    chrome_style: pane_focus_style(true),
                },
                chunks[1],
                list_state,
            );
        }
        SftpLayoutMode::DualPane => {
            f.render_widget(
                Paragraph::new(format!(
                    " SFTP: {host_alias}  [Local] {}  [Remote] {}",
                    pane.local_path.display(),
                    pane.remote_path
                )),
                chunks[0],
            );

            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            let local_is_active = pane.side == PaneSide::Local;
            let remote_is_active = pane.side == PaneSide::Remote;

            f.render_stateful_widget(
                FileList {
                    entries: pane.local_entries.as_slice(),
                    title: "Local",
                    chrome_style: pane_focus_style(local_is_active),
                },
                panes[0],
                &mut pane.local_list_state,
            );
            f.render_stateful_widget(
                FileList {
                    entries: pane.remote_entries.as_slice(),
                    title: "Remote",
                    chrome_style: pane_focus_style(remote_is_active),
                },
                panes[1],
                &mut pane.remote_list_state,
            );
        }
    }

    if let Some((verb, prog)) = transfer {
        f.render_widget(
            ProgressView {
                progress: prog,
                verb,
            },
            chunks[2],
        );
    } else if let Some(status) = status_msg {
        f.render_widget(Paragraph::new(status), chunks[2]);
    } else {
        f.render_widget(
            StatusBar {
                hints: &[
                    ("Tab", "Focus"),
                    ("d", "Download"),
                    ("u", "Upload"),
                    ("e", "Edit"),
                    ("D", "Delete"),
                    ("r", "Rename"),
                    ("Ctrl+\\", "SSH"),
                    ("q", "Quit"),
                ],
            },
            chunks[2],
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};

    use super::{DUAL_PANE_MIN_WIDTH, SftpLayoutMode, layout_mode_for_width, pane_focus_style};

    #[test]
    fn wide_width_uses_dual_pane_layout() {
        assert_eq!(layout_mode_for_width(140), SftpLayoutMode::DualPane);
    }

    #[test]
    fn narrow_width_uses_single_active_pane_layout() {
        assert_eq!(layout_mode_for_width(70), SftpLayoutMode::SinglePane);
    }

    #[test]
    fn width_at_threshold_enables_dual_pane() {
        assert_eq!(
            layout_mode_for_width(DUAL_PANE_MIN_WIDTH),
            SftpLayoutMode::DualPane
        );
    }

    #[test]
    fn active_pane_uses_highlighted_focus_style() {
        assert_eq!(pane_focus_style(true), Style::default().fg(Color::Cyan));
        assert_eq!(pane_focus_style(false), Style::default());
    }
}
