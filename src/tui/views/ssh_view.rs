use alacritty_terminal::term::{cell::Flags, point_to_viewport};
use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::ssh::terminal::TerminalEmulator;
use crate::tui::widgets::status_bar::{StatusBar, TransferBadge, build_status_message_line};

pub fn render(
    f: &mut Frame,
    host_alias: &str,
    emulator: &TerminalEmulator,
    status_msg: Option<&str>,
    transfer_badge: Option<&TransferBadge>,
) {
    let [terminal_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());

    let block = Block::bordered()
        .title(format!(" SSH: {host_alias} "))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(terminal_area);
    f.render_widget(block, terminal_area);
    f.render_widget(TerminalView { emulator }, inner);

    if let Some(status) = status_msg {
        f.render_widget(
            Paragraph::new(build_status_message_line(
                status,
                transfer_badge,
                status_area.width,
            )),
            status_area,
        );
    } else {
        f.render_widget(
            StatusBar {
                hints: &[("Ctrl-\\", "SFTP"), ("Ctrl-D", "Disconnect")],
                transfer_badge,
            },
            status_area,
        );
    }
}

struct TerminalView<'a> {
    emulator: &'a TerminalEmulator,
}

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let content = self.emulator.renderable_content();

        for ic in content.display_iter {
            let Some(point) = point_to_viewport(content.display_offset, ic.point) else {
                continue;
            };
            let x = area.x.saturating_add(point.column.0 as u16);
            let y = area.y.saturating_add(point.line as u16);
            if x >= area.right() || y >= area.bottom() {
                continue;
            }

            let cell = &ic.cell;
            let buf_cell = &mut buf[(x, y)];
            buf_cell.set_char(cell.c);
            buf_cell.set_fg(map_color(cell.fg));
            buf_cell.set_bg(map_color(cell.bg));

            let mut modifier = Modifier::empty();
            if cell.flags.contains(Flags::BOLD) {
                modifier |= Modifier::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                modifier |= Modifier::ITALIC;
            }
            if cell.flags.contains(Flags::UNDERLINE) {
                modifier |= Modifier::UNDERLINED;
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                modifier |= Modifier::CROSSED_OUT;
            }
            if !modifier.is_empty() {
                buf_cell.set_style(Style::default().add_modifier(modifier));
            }
            if content
                .selection
                .as_ref()
                .is_some_and(|selection| selection.contains(ic.point))
            {
                buf_cell.modifier |= Modifier::REVERSED;
            }
        }

        // Invert colors at cursor position.
        let cursor = &content.cursor;
        if let Some(point) = point_to_viewport(content.display_offset, cursor.point) {
            let x = area.x.saturating_add(point.column.0 as u16);
            let y = area.y.saturating_add(point.line as u16);
            if x < area.right() && y < area.bottom() {
                let c = &mut buf[(x, y)];
                let fg = c.fg;
                let bg = c.bg;
                c.set_fg(bg).set_bg(fg);
            }
        }
    }
}

pub fn map_color(color: AColor) -> Color {
    match color {
        AColor::Named(named) => map_named(named),
        AColor::Indexed(idx) => Color::Indexed(idx),
        AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn map_named(named: NamedColor) -> Color {
    match named {
        NamedColor::Black => Color::Black,
        NamedColor::Red => Color::Red,
        NamedColor::Green => Color::Green,
        NamedColor::Yellow => Color::Yellow,
        NamedColor::Blue => Color::Blue,
        NamedColor::Magenta => Color::Magenta,
        NamedColor::Cyan => Color::Cyan,
        NamedColor::White => Color::White,
        NamedColor::BrightBlack => Color::DarkGray,
        NamedColor::BrightRed => Color::LightRed,
        NamedColor::BrightGreen => Color::LightGreen,
        NamedColor::BrightYellow => Color::LightYellow,
        NamedColor::BrightBlue => Color::LightBlue,
        NamedColor::BrightMagenta => Color::LightMagenta,
        NamedColor::BrightCyan => Color::LightCyan,
        NamedColor::BrightWhite => Color::White,
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_terminal_rows(emulator: &TerminalEmulator, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        TerminalView { emulator }.render(area, &mut buf);

        (0..height)
            .map(|y| {
                let mut row = String::new();
                for x in 0..width {
                    row.push_str(buf[(x, y)].symbol());
                }
                row.trim_end().to_string()
            })
            .collect()
    }

    #[test]
    fn indexed_color_passes_through() {
        assert_eq!(map_color(AColor::Indexed(42)), Color::Indexed(42));
    }

    #[test]
    fn indexed_zero_maps_correctly() {
        assert_eq!(map_color(AColor::Indexed(0)), Color::Indexed(0));
    }

    #[test]
    fn spec_rgb_maps_to_ratatui_rgb() {
        use alacritty_terminal::vte::ansi::Rgb;
        let rgb = AColor::Spec(Rgb {
            r: 255,
            g: 128,
            b: 0,
        });
        assert_eq!(map_color(rgb), Color::Rgb(255, 128, 0));
    }

    #[test]
    fn ssh_view_passes_through_global_transfer_badge() {
        let badge = TransferBadge {
            direction_symbol: "↑",
            current_index: 1,
            total_count: 3,
            percent: 12,
        };
        assert_eq!(badge.to_text(), "↑ 1/3 12%");
    }

    #[test]
    fn terminal_view_renders_scrolled_history_rows() {
        let mut emulator = TerminalEmulator::new(10, 3);
        emulator.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");

        emulator.scroll_lines(2);

        assert_eq!(
            render_terminal_rows(&emulator, 10, 3),
            vec!["one", "two", "three"]
        );
    }

    #[test]
    fn terminal_view_highlights_selected_cells() {
        let mut emulator = TerminalEmulator::new(10, 3);
        emulator.process(b"alpha\r\nbeta\r\ngamma");
        emulator.begin_selection(1, 0);
        emulator.update_selection(3, 0);

        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        TerminalView {
            emulator: &emulator,
        }
        .render(area, &mut buf);

        assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(3, 0)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(4, 0)].modifier.contains(Modifier::REVERSED));
    }
}
