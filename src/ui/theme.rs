use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxColor {
    Keyword,
    Identifier,
    Relation,
    RelationAlias,
    Column,
    Function,
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
    pub syntax_relation: Color,
    pub syntax_relation_alias: Color,
    pub syntax_column: Color,
    pub syntax_function: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub selection: Color,
    pub row_updated: Color,
    pub row_deleted: Color,
    pub row_deleted_background: Color,
    pub row_inserted: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::deep_space()
    }
}

impl Theme {
    pub const fn for_color_mode(mode: crate::cli::ColorMode) -> Self {
        match mode {
            crate::cli::ColorMode::Auto | crate::cli::ColorMode::Always => Self::deep_space(),
            crate::cli::ColorMode::Never => Self::plain(),
        }
    }

    const fn plain() -> Self {
        Self {
            background: Color::Reset,
            surface: Color::Reset,
            surface_raised: Color::Reset,
            border: Color::Reset,
            grid_header: Color::Reset,
            grid_header_text: Color::Reset,
            grid_border: Color::Reset,
            text: Color::Reset,
            muted: Color::Reset,
            accent: Color::Reset,
            action: Color::Reset,
            syntax_relation: Color::Reset,
            syntax_relation_alias: Color::Reset,
            syntax_column: Color::Reset,
            syntax_function: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
            selection: Color::Reset,
            row_updated: Color::Reset,
            row_deleted: Color::Reset,
            row_deleted_background: Color::Reset,
            row_inserted: Color::Reset,
        }
    }

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
            syntax_relation: Color::Rgb(92, 200, 150),
            syntax_relation_alias: Color::Rgb(199, 146, 234),
            syntax_column: Color::Rgb(101, 167, 255),
            syntax_function: Color::Rgb(130, 170, 255),
            success: Color::Rgb(92, 200, 150),
            warning: Color::Rgb(244, 184, 96),
            error: Color::Rgb(255, 107, 122),
            selection: Color::Rgb(26, 55, 70),
            row_updated: Color::Rgb(36, 78, 102),
            row_deleted: Color::Rgb(255, 107, 122),
            row_deleted_background: Color::Rgb(44, 49, 56),
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
            SyntaxColor::Relation => self.syntax_relation,
            SyntaxColor::RelationAlias => self.syntax_relation_alias,
            SyntaxColor::Column => self.syntax_column,
            SyntaxColor::Function => self.syntax_function,
            SyntaxColor::String => self.warning,
            SyntaxColor::Comment => self.muted,
            SyntaxColor::Operator | SyntaxColor::Punctuation | SyntaxColor::Plain => self.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::{SyntaxColor, Theme};

    #[test]
    fn syntax_categories_match_the_editor_palette() {
        let theme = Theme::deep_space();

        assert_eq!(theme.syntax_color(SyntaxColor::Keyword), theme.accent);
        assert_eq!(theme.syntax_color(SyntaxColor::Identifier), theme.action);
        assert_eq!(theme.syntax_color(SyntaxColor::String), theme.warning);
        assert_eq!(theme.syntax_color(SyntaxColor::Comment), theme.muted);
        assert_eq!(theme.syntax_color(SyntaxColor::Plain), theme.text);
        assert_eq!(
            theme.syntax_color(SyntaxColor::Relation),
            theme.syntax_relation
        );
        assert_eq!(
            theme.syntax_color(SyntaxColor::RelationAlias),
            theme.syntax_relation_alias
        );
        assert_eq!(theme.syntax_color(SyntaxColor::Column), theme.syntax_column);
        assert_eq!(
            theme.syntax_color(SyntaxColor::Function),
            theme.syntax_function
        );
        assert_ne!(theme.syntax_relation, theme.syntax_relation_alias);
        assert_ne!(theme.syntax_relation, theme.syntax_column);
        assert_ne!(theme.syntax_relation_alias, theme.syntax_column);
    }

    #[test]
    fn plain_theme_resets_semantic_syntax_colors() {
        let theme = Theme::for_color_mode(crate::cli::ColorMode::Never);
        for kind in [
            SyntaxColor::Relation,
            SyntaxColor::RelationAlias,
            SyntaxColor::Column,
            SyntaxColor::Function,
        ] {
            assert_eq!(theme.syntax_color(kind), Color::Reset);
        }
    }
}
