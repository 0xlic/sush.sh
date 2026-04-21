use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, StatefulWidget, Widget};

use crate::config::host::Host;

pub struct HostList<'a> {
    pub hosts: &'a [Host],
    pub indices: &'a [usize],
    pub focused: bool,
}

impl<'a> StatefulWidget for HostList<'a> {
    type State = ListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let block = if self.focused {
            Block::bordered()
                .title(" 主机 ")
                .border_style(Style::default().fg(Color::Cyan))
        } else {
            Block::bordered().title(" 主机 ")
        };
        let inner = block.inner(area);
        block.render(area, buf);

        // inner 分为：列头 1 行 + 列表 + 描述 1 行
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        // 列头（与数据列对齐：2 空格偏移 + alias 16 + hostname 20 + tags）
        let header = Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<16}", "别名"),
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<20}", "地址"),
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "标签",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        Paragraph::new(header).render(sections[0], buf);

        // 描述行：显示当前选中主机的 description
        let desc = state
            .selected()
            .and_then(|sel| self.indices.get(sel))
            .map(|&i| self.hosts[i].description.as_str())
            .unwrap_or("");
        let desc_spans = if desc.is_empty() {
            Line::from(vec![])
        } else {
            Line::from(vec![
                Span::styled("  ● ", Style::default().fg(Color::Yellow)),
                Span::raw(desc),
            ])
        };
        Paragraph::new(desc_spans).render(sections[2], buf);

        // 数据列表
        let items: Vec<ListItem> = self
            .indices
            .iter()
            .map(|&i| {
                let h = &self.hosts[i];
                let tags = h.tags.join(",");
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{:<16}", h.alias), Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{:<20}", h.hostname)),
                    Span::styled(tags, Style::default().add_modifier(Modifier::DIM)),
                ]))
            })
            .collect();
        StatefulWidget::render(
            List::new(items)
                .highlight_style(
                    Style::default()
                        .add_modifier(Modifier::REVERSED)
                        .fg(Color::Yellow),
                )
                .highlight_symbol("● "),
            sections[1], // 数据区
            buf,
            state,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::host::{Host, HostSource};

    fn host_with_desc(alias: &str, desc: &str) -> Host {
        Host {
            id: alias.into(),
            alias: alias.into(),
            hostname: alias.into(),
            port: 22,
            user: "root".into(),
            identity_files: vec![],
            proxy_jump: None,
            tags: vec![],
            description: desc.into(),
            source: HostSource::SshConfig,
        }
    }

    #[test]
    fn description_renders_when_selected() {
        let hosts = vec![host_with_desc("192.168.7.2", "西城接诉即办开发环境")];
        let indices = vec![0usize];
        let mut state = ListState::default();
        state.select(Some(0));

        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);

        StatefulWidget::render(
            HostList {
                hosts: &hosts,
                indices: &indices,
                focused: false,
            },
            area,
            &mut buf,
            &mut state,
        );

        // ratatui 对双宽字符每个字符后跟一个空格占位，过滤掉再比对
        let content: String = buf
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect::<Vec<_>>()
            .join("")
            .replace(' ', "");
        assert!(
            content.contains("西城接诉即办开发环境"),
            "description not found in buffer:\n{content}"
        );
    }
}
