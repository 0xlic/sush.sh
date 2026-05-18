use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, RenderableContent, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

struct TermSize {
    cols: usize,
    lines: usize,
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn total_lines(&self) -> usize {
        self.lines
    }
}

struct VoidListener;

impl EventListener for VoidListener {
    fn send_event(&self, _: Event) {}
}

pub struct TerminalEmulator {
    term: Term<VoidListener>,
    processor: Processor<StdSyncHandler>,
    pub cols: u16,
    pub rows: u16,
}

impl TerminalEmulator {
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize {
            cols: cols as usize,
            lines: rows as usize,
        };
        let term = Term::new(Config::default(), &size, VoidListener);
        Self {
            term,
            processor: Processor::new(),
            cols,
            rows,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.clear_selection();
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let size = TermSize {
            cols: cols as usize,
            lines: rows as usize,
        };
        self.term.resize(size);
    }

    pub fn renderable_content(&self) -> RenderableContent<'_> {
        self.term.renderable_content()
    }

    pub fn is_alt_screen(&self) -> bool {
        self.renderable_content()
            .mode
            .contains(TermMode::ALT_SCREEN)
    }

    pub fn scroll_lines(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_page_up(&mut self) {
        self.term.scroll_display(Scroll::PageUp);
    }

    pub fn scroll_page_down(&mut self) {
        self.term.scroll_display(Scroll::PageDown);
    }

    pub fn begin_selection(&mut self, column: u16, line: u16) {
        let point = self.viewport_point(column, line);
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
    }

    pub fn update_selection(&mut self, column: u16, line: u16) {
        let point = self.viewport_point(column, line);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, Side::Right);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    fn viewport_point(&self, column: u16, line: u16) -> Point {
        let content = self.renderable_content();
        viewport_to_point(
            content.display_offset,
            Point::new(line as usize, Column(column as usize)),
        )
    }

    #[cfg(test)]
    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    #[cfg(test)]
    pub fn display_offset(&self) -> usize {
        self.renderable_content().display_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_correct_dimensions() {
        let em = TerminalEmulator::new(80, 24);
        assert_eq!(em.cols, 80);
        assert_eq!(em.rows, 24);
    }

    #[test]
    fn process_ascii_appears_in_grid() {
        let mut em = TerminalEmulator::new(80, 24);
        em.process(b"hi");
        let content = em.renderable_content();
        let chars: Vec<char> = content.display_iter.take(2).map(|ic| ic.cell.c).collect();
        assert_eq!(chars, vec!['h', 'i']);
    }

    #[test]
    fn resize_updates_dimensions() {
        let mut em = TerminalEmulator::new(80, 24);
        em.resize(120, 40);
        assert_eq!(em.cols, 120);
        assert_eq!(em.rows, 40);
    }

    #[test]
    fn scrollback_can_move_display_offset() {
        let mut em = TerminalEmulator::new(10, 3);
        em.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");

        assert_eq!(em.display_offset(), 0);

        em.scroll_lines(2);
        assert_eq!(em.display_offset(), 2);

        em.scroll_to_bottom();
        assert_eq!(em.display_offset(), 0);
    }

    #[test]
    fn selection_text_uses_viewport_coordinates() {
        let mut em = TerminalEmulator::new(10, 3);
        em.process(b"alpha\r\nbeta\r\ngamma");

        em.begin_selection(1, 0);
        em.update_selection(3, 1);

        assert_eq!(em.selected_text().as_deref(), Some("lpha\nbeta"));
    }

    #[test]
    fn selection_text_can_read_scrolled_history() {
        let mut em = TerminalEmulator::new(10, 3);
        em.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        em.scroll_lines(2);

        em.begin_selection(0, 0);
        em.update_selection(2, 1);

        assert_eq!(em.selected_text().as_deref(), Some("one\ntwo"));
    }
}
