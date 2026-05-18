use ratatui::style::{Modifier, Style};

pub fn selection_highlight_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}
