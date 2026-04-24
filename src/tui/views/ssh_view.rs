use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget};
use ratatui::Frame;

use crate::ssh::terminal::TerminalEmulator;
use crate::tui::widgets::status_bar::StatusBar;

pub fn render(f: &mut Frame, host_alias: &str, emulator: &TerminalEmulator) {
    let [terminal_area, status_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let block = Block::bordered()
        .title(format!(" SSH: {host_alias} "))
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(terminal_area);
    f.render_widget(block, terminal_area);
    f.render_widget(TerminalView { emulator }, inner);

    f.render_widget(
        StatusBar {
            hints: &[("Ctrl-\\", "SFTP"), ("Ctrl-D", "Disconnect")],
        },
        status_area,
    );
}

struct TerminalView<'a> {
    emulator: &'a TerminalEmulator,
}

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let content = self.emulator.renderable_content();

        for ic in content.display_iter {
            let line = ic.point.line.0;
            if line < 0 {
                continue;
            }
            let x = area.x.saturating_add(ic.point.column.0 as u16);
            let y = area.y.saturating_add(line as u16);
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
        }

        // Invert colors at cursor position.
        let cursor = &content.cursor;
        let line = cursor.point.line.0;
        if line >= 0 {
            let x = area.x.saturating_add(cursor.point.column.0 as u16);
            let y = area.y.saturating_add(line as u16);
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
        let rgb = AColor::Spec(Rgb { r: 255, g: 128, b: 0 });
        assert_eq!(map_color(rgb), Color::Rgb(255, 128, 0));
    }
}
