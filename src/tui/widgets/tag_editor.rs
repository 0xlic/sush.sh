use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TagEditorState {
    pub tags: Vec<String>,
    pub cursor: usize,
    pub input: String,
    pub candidates: Vec<String>,
    pub candidate_sel: usize,
}

#[allow(dead_code)]
impl TagEditorState {
    pub fn new(tags: Vec<String>) -> Self {
        let cursor = tags.len();
        Self {
            tags,
            cursor,
            input: String::new(),
            candidates: Vec::new(),
            candidate_sel: 0,
        }
    }

    pub fn handle_left(&mut self) {
        if !self.input.is_empty() {
            return;
        }
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn handle_right(&mut self) {
        if !self.input.is_empty() {
            return;
        }
        self.cursor = (self.cursor + 1).min(self.tags.len());
    }

    pub fn handle_char(&mut self, c: char, all_tags: &[String]) {
        self.input.push(c);
        self.recompute_candidates(all_tags);
    }

    pub fn handle_backspace(&mut self) {
        if !self.input.is_empty() {
            self.input.pop();
            self.recompute_candidates(&[]);
        } else if self.cursor > 0 {
            self.tags.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    pub fn handle_up(&mut self) {
        if !self.candidates.is_empty() {
            self.candidate_sel = self.candidate_sel.saturating_sub(1);
        }
    }

    pub fn handle_down(&mut self) {
        if !self.candidates.is_empty() {
            self.candidate_sel = (self.candidate_sel + 1).min(self.candidates.len() - 1);
        }
    }

    pub fn confirm_input(&mut self) {
        let text = if !self.candidates.is_empty() {
            self.candidates
                .get(self.candidate_sel)
                .cloned()
                .unwrap_or_else(|| self.input.trim().to_string())
        } else {
            self.input.trim().to_string()
        };
        if text.is_empty() {
            return;
        }
        self.tags.insert(self.cursor, text);
        self.cursor += 1;
        self.input.clear();
        self.candidates.clear();
        self.candidate_sel = 0;
    }

    pub fn cancel_input(&mut self) {
        self.input.clear();
        self.candidates.clear();
        self.candidate_sel = 0;
    }

    pub fn commit_pending(&mut self) {
        let text = self.input.trim().to_string();
        if !text.is_empty() {
            self.tags.insert(self.cursor, text);
            self.cursor += 1;
            self.input.clear();
            self.candidates.clear();
        }
    }

    fn recompute_candidates(&mut self, all_tags: &[String]) {
        if self.input.is_empty() {
            self.candidates.clear();
            self.candidate_sel = 0;
            return;
        }
        let lower = self.input.to_lowercase();
        self.candidates = all_tags
            .iter()
            .filter(|t| !self.tags.contains(t) && t.to_lowercase().contains(&lower))
            .take(5)
            .cloned()
            .collect();
        self.candidate_sel = 0;
    }
}

#[allow(dead_code)]
pub struct TagEditor<'a> {
    pub state: &'a TagEditorState,
    pub focused: bool,
}

impl Widget for TagEditor<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut spans: Vec<Span> = Vec::new();
        for (i, tag) in self.state.tags.iter().enumerate() {
            // Show cursor marker between tags when focused and no active input
            if self.focused && self.state.input.is_empty() && self.state.cursor == i {
                spans.push(Span::styled("|", Style::default().fg(Color::Cyan)));
            }
            spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(tag.as_str(), Style::default().fg(Color::Cyan)));
            spans.push(Span::styled("] ", Style::default().fg(Color::DarkGray)));
        }
        // Input area at the cursor (after all tags or inline)
        if self.focused && self.state.cursor == self.state.tags.len() {
            if !self.state.input.is_empty() {
                spans.push(Span::raw(self.state.input.as_str()));
                spans.push(Span::styled(
                    "█",
                    Style::default().add_modifier(Modifier::REVERSED),
                ));
            } else {
                spans.push(Span::styled("█", Style::default().fg(Color::Cyan)));
            }
        }
        Line::from(spans).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(tags: &[&str]) -> TagEditorState {
        TagEditorState::new(tags.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn new_cursor_at_end() {
        let s = state(&["web", "prod"]);
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn move_left_decrements_cursor() {
        let mut s = state(&["web", "prod"]);
        s.handle_left();
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn move_left_stops_at_zero() {
        let mut s = state(&["web"]);
        s.cursor = 0;
        s.handle_left();
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn move_right_increments_cursor() {
        let mut s = state(&["web", "prod"]);
        s.cursor = 0;
        s.handle_right();
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn move_right_stops_at_end() {
        let mut s = state(&["web"]);
        s.handle_right();
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn backspace_without_input_deletes_left_tag() {
        let mut s = state(&["web", "prod"]);
        s.handle_backspace();
        assert_eq!(s.tags, vec!["web"]);
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn backspace_without_input_at_zero_does_nothing() {
        let mut s = state(&["web"]);
        s.cursor = 0;
        s.handle_backspace();
        assert_eq!(s.tags, vec!["web"]);
    }

    #[test]
    fn char_input_starts_input_string() {
        let mut s = state(&["web"]);
        s.handle_char('p', &[]);
        assert_eq!(s.input, "p");
    }

    #[test]
    fn backspace_with_input_removes_last_char() {
        let mut s = state(&[]);
        s.input = "te".into();
        s.handle_backspace();
        assert_eq!(s.input, "t");
    }

    #[test]
    fn backspace_empties_input_then_deletes_tag() {
        let mut s = state(&["web"]);
        s.input = "x".into();
        s.handle_backspace(); // clears input
        assert_eq!(s.input, "");
        assert_eq!(s.tags, vec!["web"]); // tag not yet deleted
        s.handle_backspace(); // now deletes tag
        assert_eq!(s.tags, Vec::<String>::new());
    }

    #[test]
    fn confirm_input_creates_tag_at_cursor() {
        let mut s = state(&["web", "prod"]);
        s.cursor = 1;
        s.input = "nginx".into();
        s.confirm_input();
        assert_eq!(s.tags, vec!["web", "nginx", "prod"]);
        assert_eq!(s.cursor, 2);
        assert_eq!(s.input, "");
    }

    #[test]
    fn confirm_empty_input_does_nothing() {
        let mut s = state(&["web"]);
        s.confirm_input();
        assert_eq!(s.tags, vec!["web"]);
    }

    #[test]
    fn candidates_filter_by_input() {
        let mut s = state(&[]);
        let all = vec!["web".into(), "nginx".into(), "prod".into()];
        s.handle_char('n', &all);
        assert_eq!(s.candidates, vec!["nginx"]);
    }

    #[test]
    fn candidates_exclude_already_added_tags() {
        let mut s = state(&["nginx"]);
        let all = vec!["web".into(), "nginx".into()];
        s.handle_char('n', &all);
        assert!(s.candidates.is_empty());
    }

    #[test]
    fn commit_pending_creates_tag_if_input_nonempty() {
        let mut s = state(&[]);
        s.input = "new-tag".into();
        s.commit_pending();
        assert_eq!(s.tags, vec!["new-tag"]);
        assert_eq!(s.input, "");
    }
}
