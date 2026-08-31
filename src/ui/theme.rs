use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxColor {
    Keyword,
    Identifier,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Parameter,
    Plain,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub background: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub border: Color,
    pub grid_header: Color,
    pub grid_header_text: Color,
    pub grid_border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub action: Color,
    pub warning: Color,
    pub error: Color,
    pub selection: Color,
    pub row_updated: Color,
    pub row_deleted: Color,
    pub row_inserted: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::deep_space()
    }
}

impl Theme {
    pub const fn deep_space() -> Self {
        Self {
            background: Color::Rgb(7, 11, 18),
            surface: Color::Rgb(12, 19, 30),
            surface_raised: Color::Rgb(17, 28, 43),
            border: Color::Rgb(43, 66, 86),
            grid_header: Color::Rgb(15, 34, 44),
            grid_header_text: Color::Rgb(184, 235, 229),
            grid_border: Color::Rgb(58, 72, 84),
            text: Color::Rgb(215, 226, 237),
            muted: Color::Rgb(105, 126, 146),
            accent: Color::Rgb(99, 230, 216),
            action: Color::Rgb(101, 167, 255),
            warning: Color::Rgb(244, 184, 96),
            error: Color::Rgb(255, 107, 122),
            selection: Color::Rgb(26, 55, 70),
            row_updated: Color::Rgb(244, 184, 96),
            row_deleted: Color::Rgb(255, 107, 122),
            row_inserted: Color::Rgb(101, 167, 255),
        }
    }

    pub fn base(self) -> Style {
        Style::new().fg(self.text).bg(self.background)
    }

    pub fn title(self, focused: bool) -> Style {
        Style::new()
            .fg(if focused { self.accent } else { self.muted })
            .bg(self.surface)
            .add_modifier(Modifier::BOLD)
    }

    pub(crate) const fn syntax_color(self, kind: SyntaxColor) -> Color {
        match kind {
            SyntaxColor::Keyword => self.accent,
            SyntaxColor::Identifier | SyntaxColor::Number | SyntaxColor::Parameter => self.action,
            SyntaxColor::String => self.warning,
            SyntaxColor::Comment => self.muted,
            SyntaxColor::Operator | SyntaxColor::Punctuation | SyntaxColor::Plain => self.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SyntaxColor, Theme};

    #[test]
    fn syntax_categories_match_the_editor_palette() {
        let theme = Theme::deep_space();

        assert_eq!(theme.syntax_color(SyntaxColor::Keyword), theme.accent);
        assert_eq!(theme.syntax_color(SyntaxColor::Identifier), theme.action);
        assert_eq!(theme.syntax_color(SyntaxColor::String), theme.warning);
        assert_eq!(theme.syntax_color(SyntaxColor::Comment), theme.muted);
        assert_eq!(theme.syntax_color(SyntaxColor::Plain), theme.text);
    }
}
