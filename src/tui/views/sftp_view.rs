use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

use crate::sftp::{PaneSide, SftpPaneState};
use crate::tui::widgets::file_list::FileList;
use crate::tui::widgets::status_bar::{StatusBar, TransferBadge, build_status_message_line};

const DUAL_PANE_MIN_WIDTH: u16 = 100;
const DEFAULT_HINTS: [(&str, &str); 8] = [
    ("Tab", "Focus"),
    ("d", "Download"),
    ("u", "Upload"),
    ("e", "Edit"),
    ("D", "Delete"),
    ("r", "Rename"),
    ("Ctrl+\\", "SSH"),
    ("q", "Quit"),
];

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

fn multi_select_hints(side: PaneSide) -> [(&'static str, &'static str); 3] {
    match side {
        PaneSide::Local => [("u", "Upload"), ("D", "Delete"), ("Esc", "Cancel")],
        PaneSide::Remote => [("d", "Download"), ("D", "Delete"), ("Esc", "Cancel")],
    }
}

pub fn render(
    f: &mut Frame,
    host_address: &str,
    pane: &mut SftpPaneState,
    status_msg: Option<&str>,
    transfer_badge: Option<&TransferBadge>,
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
            let (label, path, selected_indices) = match pane.side {
                PaneSide::Local => (
                    "Local",
                    pane.local_path.display().to_string(),
                    &pane.local_selection,
                ),
                PaneSide::Remote => ("Remote", pane.remote_path.clone(), &pane.remote_selection),
            };
            f.render_widget(Paragraph::new(format!(" SFTP: {host_address}")), chunks[0]);

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
                    title: &pane_title(label, &path, chunks[1].width),
                    chrome_style: pane_focus_style(true),
                    selected_indices,
                    focused: true,
                },
                chunks[1],
                list_state,
            );
        }
        SftpLayoutMode::DualPane => {
            f.render_widget(Paragraph::new(format!(" SFTP: {host_address}")), chunks[0]);

            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);
            let local_is_active = pane.side == PaneSide::Local;
            let remote_is_active = pane.side == PaneSide::Remote;

            f.render_stateful_widget(
                FileList {
                    entries: pane.local_entries.as_slice(),
                    title: &pane_title(
                        "Local",
                        &pane.local_path.display().to_string(),
                        panes[0].width,
                    ),
                    chrome_style: pane_focus_style(local_is_active),
                    selected_indices: &pane.local_selection,
                    focused: local_is_active,
                },
                panes[0],
                &mut pane.local_list_state,
            );
            f.render_stateful_widget(
                FileList {
                    entries: pane.remote_entries.as_slice(),
                    title: &pane_title("Remote", &pane.remote_path, panes[1].width),
                    chrome_style: pane_focus_style(remote_is_active),
                    selected_indices: &pane.remote_selection,
                    focused: remote_is_active,
                },
                panes[1],
                &mut pane.remote_list_state,
            );
        }
    }

    if let Some(status) = status_msg {
        f.render_widget(
            Paragraph::new(build_status_message_line(
                status,
                transfer_badge,
                chunks[2].width,
            )),
            chunks[2],
        );
    } else {
        let active_multi_select = match pane.side {
            PaneSide::Local => !pane.local_selection.is_empty(),
            PaneSide::Remote => !pane.remote_selection.is_empty(),
        };
        let batch_hints = multi_select_hints(pane.side);
        let hints: &[(&str, &str)] = if active_multi_select {
            &batch_hints
        } else {
            &DEFAULT_HINTS
        };
        f.render_widget(
            StatusBar {
                hints,
                transfer_badge,
            },
            chunks[2],
        );
    }
}

fn pane_title(label: &str, path: &str, pane_width: u16) -> String {
    let prefix = format!("{label} ");
    let available_path_width = pane_width
        .saturating_sub(4)
        .saturating_sub(prefix.chars().count() as u16) as usize;
    format!(
        "{prefix}{}",
        truncate_path_start(path, available_path_width)
    )
}

fn truncate_path_start(path: &str, max_width: usize) -> String {
    let path_len = path.chars().count();
    if path_len <= max_width {
        return path.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let tail_width = max_width - 3;
    let start = path_len.saturating_sub(tail_width);
    let tail = path.chars().skip(start).collect::<String>();
    let tail = tail
        .find(['/', '\\'])
        .map(|index| &tail[index..])
        .filter(|tail| tail.chars().count() + 3 <= max_width)
        .unwrap_or(tail.as_str());
    format!("...{tail}")
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Style};

    use crate::sftp::{PaneSide, SftpPaneState};

    use super::{
        DUAL_PANE_MIN_WIDTH, SftpLayoutMode, layout_mode_for_width, multi_select_hints,
        pane_focus_style, truncate_path_start,
    };

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

    #[test]
    fn multi_select_status_bar_shows_batch_actions_for_local_pane() {
        assert_eq!(
            multi_select_hints(PaneSide::Local),
            [("u", "Upload"), ("D", "Delete"), ("Esc", "Cancel")]
        );
    }

    #[test]
    fn multi_select_status_bar_shows_batch_actions_for_remote_pane() {
        assert_eq!(
            multi_select_hints(PaneSide::Remote),
            [("d", "Download"), ("D", "Delete"), ("Esc", "Cancel")]
        );
    }

    #[test]
    fn long_paths_keep_tail_when_truncated() {
        let path = "/Users/alice/projects/work/sush/src/tui/views";

        assert_eq!(truncate_path_start(path, 24), ".../sush/src/tui/views");
    }

    #[test]
    fn short_paths_are_not_truncated() {
        assert_eq!(truncate_path_start("/tmp/sush", 20), "/tmp/sush");
    }

    #[test]
    fn header_line_shows_host_without_local_or_remote_paths() {
        let backend = TestBackend::new(120, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut pane = SftpPaneState::new("/remote/projects/sush/src/tui".into());
        pane.local_path = "/Users/alice/projects/sush/src/tui".into();

        terminal
            .draw(|f| super::render(f, "10.0.0.7", &mut pane, None, None))
            .unwrap();

        let header = line_content(terminal.backend().buffer(), 0, 120);
        assert!(header.contains("SFTP: 10.0.0.7"));
        assert!(!header.contains("Local"));
        assert!(!header.contains("/Users"));
        assert!(!header.contains("/remote"));
    }

    fn line_content(buffer: &Buffer, y: u16, width: u16) -> String {
        (0..width)
            .filter_map(|x| buffer.cell((x, y)))
            .map(|cell| cell.symbol().to_string())
            .collect::<Vec<_>>()
            .join("")
    }
}
