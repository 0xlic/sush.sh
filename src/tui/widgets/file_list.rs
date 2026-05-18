use std::collections::BTreeSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, StatefulWidget};

use crate::sftp::client::FileEntry;
use crate::tui::theme::selection_highlight_style;

pub struct FileList<'a> {
    pub entries: &'a [FileEntry],
    pub title: &'a str,
    pub chrome_style: Style,
    pub selected_indices: &'a BTreeSet<usize>,
    pub focused: bool,
}

impl<'a> StatefulWidget for FileList<'a> {
    type State = ListState;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, e)| {
                let size = if e.is_dir {
                    "<DIR>".to_string()
                } else {
                    human_size(e.size)
                };
                let prefix = entry_prefix(self.selected_indices.contains(&index), e.is_dir);
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(
                        format!("{:<40}", e.name),
                        if e.is_dir {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        },
                    ),
                    Span::raw(size),
                ]))
            })
            .collect();
        let title = Span::styled(
            self.title.to_string(),
            self.chrome_style.add_modifier(Modifier::BOLD),
        );
        let highlight_style = if self.focused {
            selection_highlight_style()
        } else {
            Style::default()
        };
        List::new(items)
            .block(
                Block::bordered()
                    .border_style(self.chrome_style)
                    .title(title),
            )
            .highlight_style(highlight_style)
            .render(area, buf, state);
    }
}

fn entry_prefix(is_selected: bool, is_dir: bool) -> &'static str {
    if is_selected {
        "[x] "
    } else if is_dir {
        "▸ "
    } else {
        "  "
    }
}

fn human_size(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{} {}", n, UNITS[u])
    } else {
        format!("{:.1} {}", v, UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;
    use ratatui::widgets::StatefulWidget;

    use crate::sftp::client::FileEntry;

    use super::FileList;
    use super::entry_prefix;

    #[test]
    fn selected_file_uses_marked_prefix() {
        assert_eq!(entry_prefix(true, false), "[x] ");
    }

    #[test]
    fn unselected_directory_keeps_directory_prefix() {
        assert_eq!(entry_prefix(false, true), "▸ ");
    }

    #[test]
    fn selected_row_is_not_highlighted_when_list_is_unfocused() {
        let entries = vec![FileEntry {
            name: "file.txt".into(),
            size: 10,
            is_dir: false,
        }];
        let selected_indices = Default::default();
        let mut state = ratatui::widgets::ListState::default();
        state.select(Some(0));
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);

        StatefulWidget::render(
            FileList {
                entries: &entries,
                title: "Local",
                chrome_style: Default::default(),
                selected_indices: &selected_indices,
                focused: false,
            },
            area,
            &mut buf,
            &mut state,
        );

        let selected_cell = buf.cell((1, 1)).expect("selected row cell should exist");
        assert!(!selected_cell.modifier.contains(Modifier::REVERSED));
    }
}
