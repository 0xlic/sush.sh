use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::Paragraph;

use crate::sftp::transfer::TransferProgress;
use crate::sftp::{PaneSide, SftpPaneState};
use crate::tui::widgets::file_list::FileList;
use crate::tui::widgets::progress_bar::ProgressView;
use crate::tui::widgets::status_bar::StatusBar;

pub fn render(
    f: &mut Frame,
    host_alias: &str,
    pane: &mut SftpPaneState,
    transfer: Option<(&'static str, &TransferProgress)>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    let (label, path) = match pane.side {
        PaneSide::Local => ("本地", pane.local_path.display().to_string()),
        PaneSide::Remote => ("远程", pane.remote_path.clone()),
    };
    f.render_widget(
        Paragraph::new(format!(" SFTP: {host_alias}  [{label}] {path}")),
        chunks[0],
    );

    let entries = match pane.side {
        PaneSide::Local => pane.local_entries.as_slice(),
        PaneSide::Remote => pane.remote_entries.as_slice(),
    };
    f.render_stateful_widget(
        FileList {
            entries,
            title: label,
        },
        chunks[1],
        &mut pane.list_state,
    );

    if let Some((verb, prog)) = transfer {
        f.render_widget(
            ProgressView {
                progress: prog,
                verb,
            },
            chunks[2],
        );
    } else {
        f.render_widget(
            StatusBar {
                hints: &[
                    ("Tab", "本地/远程"),
                    ("d", "下载"),
                    ("u", "上传"),
                    ("D", "删除"),
                    ("r", "重命名"),
                    ("q", "退出"),
                ],
            },
            chunks[2],
        );
    }
}
