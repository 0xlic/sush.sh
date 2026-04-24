use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, RenderableContent};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use alacritty_terminal::Term;

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
        let chars: Vec<char> = content
            .display_iter
            .take(2)
            .map(|ic| ic.cell.c)
            .collect();
        assert_eq!(chars, vec!['h', 'i']);
    }

    #[test]
    fn resize_updates_dimensions() {
        let mut em = TerminalEmulator::new(80, 24);
        em.resize(120, 40);
        assert_eq!(em.cols, 120);
        assert_eq!(em.rows, 40);
    }
}
