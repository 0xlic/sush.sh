use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, StatefulWidget};

use crate::sftp::client::FileEntry;

pub struct FileList<'a> {
    pub entries: &'a [FileEntry],
    pub title: &'a str,
    pub chrome_style: Style,
}

impl<'a> StatefulWidget for FileList<'a> {
    type State = ListState;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|e| {
                let size = if e.is_dir {
                    "<DIR>".to_string()
                } else {
                    human_size(e.size)
                };
                let prefix = if e.is_dir { "▸ " } else { "  " };
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
        List::new(items)
            .block(
                Block::bordered()
                    .border_style(self.chrome_style)
                    .title(title),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .render(area, buf, state);
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
