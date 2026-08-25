use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub background: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub action: Color,
    pub warning: Color,
    pub error: Color,
    pub selection: Color,
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
            text: Color::Rgb(215, 226, 237),
            muted: Color::Rgb(105, 126, 146),
            accent: Color::Rgb(99, 230, 216),
            action: Color::Rgb(101, 167, 255),
            warning: Color::Rgb(244, 184, 96),
            error: Color::Rgb(255, 107, 122),
            selection: Color::Rgb(26, 55, 70),
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
}
