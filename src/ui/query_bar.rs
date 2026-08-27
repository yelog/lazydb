use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::Style,
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use crate::model::data_query::{DataQueryCapability, DataQueryInput, DataQueryState};

use super::{HitRegion, HitTarget, UiState, theme::Theme};

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    query: &DataQueryState,
    theme: Theme,
    state: &mut UiState,
) {
    if area.height == 0 {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let enabled = matches!(
        query.capability,
        DataQueryCapability::Relation | DataQueryCapability::Sql
    );
    let fields = [
        (DataQueryInput::Where, "WHERE", query.where_input.value()),
        (
            DataQueryInput::OrderBy,
            "ORDER BY",
            query.order_by_input.value(),
        ),
    ];
    for ((input, label, value), chunk) in fields.into_iter().zip(chunks.iter().copied()) {
        let active = enabled && query.focus == Some(input);
        frame.render_widget(
            Paragraph::new(format!("{label}  {value}")).style(if !enabled {
                theme.muted
            } else if active {
                theme.accent
            } else {
                theme.muted
            }),
            chunk,
        );
        if enabled {
            state.hit_regions.push(HitRegion {
                area: chunk,
                target: HitTarget::DataQueryInput(input),
            });
        }
        if active {
            let cursor_index = match input {
                DataQueryInput::Where => query.where_input.cursor(),
                DataQueryInput::OrderBy => query.order_by_input.cursor(),
            };
            let cursor = UnicodeWidthStr::width(
                &value[..value
                    .char_indices()
                    .nth(cursor_index)
                    .map_or(value.len(), |(index, _)| index)],
            );
            frame.set_cursor_position(Position::new(
                chunk
                    .x
                    .saturating_add(UnicodeWidthStr::width(label) as u16)
                    .saturating_add(2)
                    .saturating_add(cursor as u16)
                    .min(chunk.right().saturating_sub(1)),
                chunk.y,
            ));
        }
    }
    let message = query.error.as_ref().or(match &query.capability {
        DataQueryCapability::Unavailable(reason) => Some(reason),
        DataQueryCapability::Relation | DataQueryCapability::Sql => None,
    });
    if let Some(error) = message
        && area.height > 2
    {
        frame.render_widget(
            Paragraph::new(crate::security::sanitize_terminal_text(error))
                .style(Style::new().fg(theme.warning)),
            Rect::new(area.x, area.y.saturating_add(2), area.width, 1),
        );
    }
}
