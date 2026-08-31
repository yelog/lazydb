use crate::model::data_query::{DataQueryCapability, DataQueryInput, DataQueryState};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::Style,
    widgets::Paragraph,
};

use super::{HitRegion, HitTarget, UiState, icons::IconSet, render_text_input, theme::Theme};

const FIELD_HEIGHT: u16 = 2;
const HORIZONTAL_MIN_WIDTH: u16 = 56;

fn fields_height(width: u16) -> u16 {
    if width >= HORIZONTAL_MIN_WIDTH {
        FIELD_HEIGHT
    } else {
        FIELD_HEIGHT * 2
    }
}

pub(crate) fn height(query: &DataQueryState, width: u16, _icons: IconSet) -> u16 {
    fields_height(width)
        + u16::from(
            query.error.is_some()
                || matches!(query.capability, DataQueryCapability::Unavailable(_)),
        )
}

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    query: &DataQueryState,
    theme: Theme,
    state: &mut UiState,
    icons: IconSet,
) -> Option<Position> {
    if area.height == 0 {
        return None;
    }
    let horizontal = area.width >= HORIZONTAL_MIN_WIDTH;
    let fields_height = fields_height(area.width);
    let fields_area = Rect::new(area.x, area.y, area.width, area.height.min(fields_height));
    let chunks = if horizontal {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(fields_area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(FIELD_HEIGHT),
                Constraint::Length(FIELD_HEIGHT),
            ])
            .split(fields_area)
    };
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
    let mut cursor = None;
    for ((input, label, value), chunk) in fields.into_iter().zip(chunks.iter().copied()) {
        let active = enabled && query.focus == Some(input);
        let icon = match input {
            DataQueryInput::Where => icons.query_filter(),
            DataQueryInput::OrderBy => icons.query_sort(),
        };
        let field = Rect::new(chunk.x, chunk.y, chunk.width, 1);
        let underline = Rect::new(chunk.x, chunk.y.saturating_add(1), chunk.width, 1);
        let label = format!("{icon} {label}");
        if active {
            let text_input = match input {
                DataQueryInput::Where => &query.where_input,
                DataQueryInput::OrderBy => &query.order_by_input,
            };
            cursor = render_text_input(
                frame,
                field,
                &format!("{label}  "),
                text_input,
                Style::new().fg(theme.accent),
                state,
            );
        } else {
            frame.render_widget(
                Paragraph::new(format!("{label}  {value}")).style(Style::new().fg(theme.muted)),
                field,
            );
        }
        frame.render_widget(
            Paragraph::new(icons.query_underline().repeat(usize::from(chunk.width)))
                .style(Style::new().fg(if active { theme.accent } else { theme.border })),
            underline,
        );
        if enabled {
            state.hit_regions.push(HitRegion {
                area: chunk,
                target: HitTarget::DataQueryInput(input),
            });
        }
    }
    let message = query.error.as_ref().or(match &query.capability {
        DataQueryCapability::Unavailable(reason) => Some(reason),
        DataQueryCapability::Relation
        | DataQueryCapability::Sql
        | DataQueryCapability::AwaitingResult => None,
    });
    if let Some(error) = message {
        let error_y = area.y.saturating_add(fields_height);
        if error_y >= area.bottom() {
            return cursor;
        }
        frame.render_widget(
            Paragraph::new(crate::security::sanitize_terminal_text(error))
                .style(Style::new().fg(theme.warning)),
            Rect::new(area.x, error_y, area.width, 1),
        );
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::{FIELD_HEIGHT, fields_height};

    #[test]
    fn switches_to_horizontal_layout_at_the_minimum_usable_width() {
        assert_eq!(fields_height(55), FIELD_HEIGHT * 2);
        assert_eq!(fields_height(56), FIELD_HEIGHT);
    }
}
