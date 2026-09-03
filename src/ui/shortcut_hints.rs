use std::borrow::Cow;

use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthChar;

use super::Theme;

const SEPARATOR: &str = "   ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ShortcutHint<'a> {
    pub key: Cow<'a, str>,
    pub description: Cow<'a, str>,
}

impl<'a> ShortcutHint<'a> {
    pub(super) fn new(key: impl Into<Cow<'a, str>>, description: impl Into<Cow<'a, str>>) -> Self {
        Self {
            key: key.into(),
            description: description.into(),
        }
    }
}

pub(super) fn line(
    hints: &[ShortcutHint<'_>],
    width: u16,
    theme: Theme,
    background: Color,
) -> Line<'static> {
    let width = usize::from(width);
    if width == 0 || hints.is_empty() {
        return Line::default();
    }

    let selected = packed_count(hints, width);
    let mut spans = Vec::new();
    for (index, hint) in hints.iter().take(selected).enumerate() {
        if index > 0 {
            spans.push(separator_span(theme, background));
        }
        spans.push(Span::styled(
            hint.key.to_string(),
            Style::new()
                .fg(theme.action)
                .bg(background)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" ", Style::new().bg(background)));
        spans.push(Span::styled(
            hint.description.to_string(),
            Style::new().fg(theme.text).bg(background),
        ));
    }

    let omitted = hints.len() - selected;
    if omitted > 0 {
        let used_width = visible_width(hints, selected);
        if selected > 0 {
            spans.push(separator_span(theme, background));
        }
        let separator_width = usize::from(selected > 0) * SEPARATOR.len();
        let remaining = width.saturating_sub(used_width + separator_width);
        let marker = truncate_to_cells(&format!("... (+{omitted})"), remaining);
        spans.push(Span::styled(
            marker,
            Style::new().fg(theme.muted).bg(background),
        ));
    }

    Line::from(spans)
}

pub(super) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    hints: &[ShortcutHint<'_>],
    theme: Theme,
    background: Color,
    alignment: Alignment,
) {
    frame.render_widget(
        Paragraph::new(line(hints, area.width, theme, background))
            .style(Style::new().bg(background))
            .alignment(alignment),
        area,
    );
}

fn packed_count(hints: &[ShortcutHint<'_>], width: usize) -> usize {
    for count in (0..=hints.len()).rev() {
        let omitted = hints.len() - count;
        let marker_width = if omitted > 0 {
            usize::from(format!("... (+{omitted})").cell_width())
        } else {
            0
        };
        let marker_separator = usize::from(count > 0 && omitted > 0) * SEPARATOR.len();
        let candidate_width = visible_width(hints, count) + marker_separator + marker_width;
        if candidate_width <= width {
            return count;
        }
    }
    0
}

fn visible_width(hints: &[ShortcutHint<'_>], count: usize) -> usize {
    hints
        .iter()
        .take(count)
        .map(|hint| {
            usize::from(hint.key.as_ref().cell_width())
                + 1
                + usize::from(hint.description.as_ref().cell_width())
        })
        .sum::<usize>()
        + count.saturating_sub(1) * SEPARATOR.len()
}

fn separator_span(theme: Theme, background: Color) -> Span<'static> {
    Span::styled(SEPARATOR, Style::new().fg(theme.muted).bg(background))
}

fn truncate_to_cells(value: &str, width: usize) -> String {
    let mut used = 0;
    value
        .chars()
        .take_while(|character| {
            let character_width = character.width().unwrap_or(0);
            if used + character_width > width {
                false
            } else {
                used += character_width;
                true
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier};

    use super::{ShortcutHint, line};
    use crate::{cli::ColorMode, ui::Theme};

    fn plain_text(line: &ratatui::text::Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn line_styles_keys_descriptions_and_separators_independently() {
        let theme = Theme::deep_space();
        let rendered = line(
            &[
                ShortcutHint::new("Enter", "save"),
                ShortcutHint::new("Esc", "cancel"),
            ],
            80,
            theme,
            theme.surface,
        );

        assert_eq!(plain_text(&rendered), "Enter save   Esc cancel");
        assert_eq!(rendered.spans[0].style.fg, Some(theme.action));
        assert!(
            rendered.spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(rendered.spans[2].style.fg, Some(theme.text));
        assert_eq!(rendered.spans[3].style.fg, Some(theme.muted));
        assert_eq!(rendered.spans[4].style.fg, Some(theme.action));
        assert_eq!(rendered.spans[6].style.fg, Some(theme.text));
    }

    #[test]
    fn packing_keeps_complete_hints_and_reports_omissions() {
        let theme = Theme::deep_space();
        let hints = [
            ShortcutHint::new("j/k", "move"),
            ShortcutHint::new("Enter", "open"),
            ShortcutHint::new("/", "find"),
        ];

        assert_eq!(
            plain_text(&line(&hints, 19, theme, theme.surface)),
            "j/k move   ... (+2)"
        );
        assert_eq!(
            plain_text(&line(&hints, 40, theme, theme.surface)),
            "j/k move   Enter open   / find"
        );
    }

    #[test]
    fn packing_measures_unicode_terminal_cells_and_handles_tiny_widths() {
        let theme = Theme::deep_space();
        let hints = [
            ShortcutHint::new("界", "move"),
            ShortcutHint::new("Enter", "open"),
        ];

        assert_eq!(
            plain_text(&line(&hints, 18, theme, theme.surface)),
            "界 move   ... (+1)"
        );
        assert_eq!(
            plain_text(&line(
                &[ShortcutHint::new("long", "hint")],
                3,
                theme,
                theme.surface
            )),
            "..."
        );
        assert!(plain_text(&line(&[], 20, theme, theme.surface)).is_empty());
        assert!(
            plain_text(&line(
                &[ShortcutHint::new("a", "b")],
                0,
                theme,
                theme.surface
            ))
            .is_empty()
        );
    }

    #[test]
    fn packing_uses_final_omitted_count_when_selecting_units() {
        let theme = Theme::deep_space();
        let hints = [
            ShortcutHint::new("aaaa", "x"),
            ShortcutHint::new("bb", "x"),
            ShortcutHint::new("cc", "x"),
        ];

        let rendered = plain_text(&line(&hints, 17, theme, theme.surface));
        assert_eq!(rendered, "aaaa x   ... (+2)");
    }

    #[test]
    fn plain_theme_keeps_keys_bold_when_colors_are_reset() {
        let theme = Theme::for_color_mode(ColorMode::Never);
        let rendered = line(
            &[ShortcutHint::new("Enter", "save")],
            20,
            theme,
            theme.surface,
        );

        assert_eq!(rendered.spans[0].style.fg, Some(Color::Reset));
        assert_eq!(rendered.spans[2].style.fg, Some(Color::Reset));
        assert!(
            rendered.spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            !rendered.spans[2]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }
}
