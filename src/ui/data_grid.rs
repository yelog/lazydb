use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Cell, Paragraph, Row, Table, TableState},
};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::{
    db::{query::ResultSet, value::CellValue},
    security::sanitize_terminal_text,
};

use super::{HitRegion, HitTarget, UiState, theme::Theme};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    tab_id: Uuid,
    result: &ResultSet,
    grid: crate::model::tab::DataGridState,
    overrides: &[Option<u16>],
    theme: Theme,
    block: Block<'_>,
    state: &mut UiState,
    edit: Option<&crate::model::relation_edit::RelationEditSession>,
) {
    if result.columns.is_empty() {
        frame.render_widget(
            ratatui::widgets::Paragraph::new(format!(
                "Statement complete · {} row(s) affected",
                result.affected_rows
            ))
            .block(block)
            .style(Style::new().fg(theme.accent).bg(theme.surface))
            .alignment(ratatui::layout::Alignment::Center),
            area,
        );
        return;
    }

    let widths = automatic_widths(result)
        .into_iter()
        .enumerate()
        .map(|(index, width)| {
            overrides
                .get(index)
                .and_then(|value| *value)
                .unwrap_or(width)
        })
        .collect::<Vec<_>>();
    let number_width = row_number_width(result.rows.len());
    let fixed_width = number_width.saturating_add(1);
    let available = area
        .width
        .saturating_sub(4)
        .saturating_sub(fixed_width)
        .max(1);
    let overflow = total_width(&widths) > available;
    let first = viewport_start(&widths, grid.column_offset, grid.selected_column, available);
    let visible = visible_columns(&widths, first, available);
    let constraints = grid_constraints(&visible, &widths, number_width);

    let visible_rows = area.height.saturating_sub(3 + u16::from(overflow)) as usize;
    let row_offset = row_viewport_start(
        result.rows.len(),
        visible_rows,
        grid.row_offset,
        grid.selected_row,
    );
    state.grid_viewport = Some(crate::model::tab::DataGridViewport {
        tab_id,
        column_offset: first,
        row_offset,
        visible_rows,
    });
    let row_y = area.y.saturating_add(2);
    for (screen_row, row_index) in result
        .rows
        .iter()
        .skip(row_offset)
        .take(visible_rows)
        .enumerate()
        .map(|(screen_row, _)| (screen_row, row_offset.saturating_add(screen_row)))
    {
        let mut x = data_start_x(area, number_width);
        for column_index in &visible {
            let width = widths[*column_index];
            if x >= area.right() {
                break;
            }
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    x,
                    row_y.saturating_add(screen_row as u16),
                    width.min(area.right().saturating_sub(x)),
                    1,
                ),
                target: HitTarget::ResultCell {
                    row: row_index,
                    column: *column_index,
                },
            });
            x = x.saturating_add(width).saturating_add(1);
        }
    }

    let mut boundary_x = data_start_x(area, number_width);
    for column_index in &visible {
        boundary_x = boundary_x.saturating_add(widths[*column_index]);
        if boundary_x < area.right().saturating_sub(1) {
            state.hit_regions.push(HitRegion {
                area: Rect::new(boundary_x, row_y, 1, 1),
                target: HitTarget::RelationColumnResize {
                    column: *column_index,
                    width: widths[*column_index],
                },
            });
        }
        boundary_x = boundary_x.saturating_add(1);
    }

    let header = Row::new(header_cells(&visible, result, number_width, theme)).height(1);
    let rows = result
        .rows
        .iter()
        .skip(row_offset)
        .take(visible_rows)
        .enumerate()
        .map(|(screen_row, row)| {
            let row_index = row_offset.saturating_add(screen_row);
            let editable = edit.and_then(|session| session.rows.get(row_index));
            let row_style = editable.map(|row| match row.state {
                crate::model::relation_edit::EditableRowState::Deleted => {
                    Style::new().fg(theme.row_deleted)
                }
                crate::model::relation_edit::EditableRowState::Updated { .. } => {
                    Style::new().fg(theme.row_updated)
                }
                crate::model::relation_edit::EditableRowState::InsertDraft
                | crate::model::relation_edit::EditableRowState::Inserted => {
                    Style::new().fg(theme.row_inserted)
                }
                crate::model::relation_edit::EditableRowState::Conflict { .. } => {
                    Style::new().fg(theme.row_deleted)
                }
                crate::model::relation_edit::EditableRowState::Clean => Style::new().fg(theme.text),
            });
            let row = editable.map(|row| row.current.as_slice()).unwrap_or(row);
            Row::new(body_cells(
                &visible,
                &widths,
                row_index,
                number_width,
                row,
                row_style,
                theme,
            ))
        });
    let table = Table::new(rows, constraints)
        .header(header)
        .block(block)
        .column_spacing(0)
        .row_highlight_style(Style::new().bg(theme.selection).fg(theme.text))
        .cell_highlight_style(
            Style::new()
                .bg(theme.accent)
                .fg(theme.background)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");
    let selected_cell = (!result.rows.is_empty()).then(|| {
        let selected_column = visible
            .iter()
            .position(|index| *index == grid.selected_column)
            .map_or_else(|| selected_data_cell(0), selected_data_cell);
        let selected_row = grid.selected_row.saturating_sub(row_offset);
        (selected_row, selected_column)
    });
    let mut table_state = TableState::new().with_selected_cell(selected_cell);
    frame.render_stateful_widget(table, area, &mut table_state);
    if result.rows.is_empty() && area.height >= 3 {
        frame.render_widget(
            Paragraph::new("No rows")
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(Alignment::Center),
            Rect::new(
                area.x.saturating_add(2),
                area.y.saturating_add(2),
                area.width.saturating_sub(4),
                1,
            ),
        );
    }
    if overflow {
        render_scrollbar(
            frame,
            area,
            first,
            &visible,
            widths.len(),
            last_page_start(&widths, available),
            number_width,
            theme,
            state,
        );
    }
}

fn automatic_widths(result: &ResultSet) -> Vec<u16> {
    result
        .columns
        .iter()
        .enumerate()
        .map(|(column_index, column)| {
            let header = sanitize_terminal_text(&column.name);
            let content = result
                .rows
                .iter()
                .filter_map(|row| row.get(column_index))
                .map(|value| value.preview(40).text)
                .map(|text| UnicodeWidthStr::width(text.as_str()))
                .max()
                .unwrap_or(0);
            (UnicodeWidthStr::width(header.as_str()).max(content) + 2).clamp(6, 40) as u16
        })
        .collect()
}

fn grid_constraints(visible: &[usize], widths: &[u16], number_width: u16) -> Vec<Constraint> {
    let mut constraints = Vec::with_capacity(visible.len().saturating_mul(2).saturating_add(2));
    constraints.push(Constraint::Length(number_width));
    constraints.push(Constraint::Length(1));
    for (position, index) in visible.iter().enumerate() {
        if position > 0 {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(widths[*index]));
    }
    constraints
}

fn row_viewport_start(
    row_count: usize,
    visible_rows: usize,
    offset: usize,
    selected: usize,
) -> usize {
    if row_count == 0 || visible_rows == 0 {
        return 0;
    }
    let selected = selected.min(row_count - 1);
    let mut offset = offset.min(row_count.saturating_sub(visible_rows.min(row_count)));
    if selected < offset {
        offset = selected;
    } else if selected >= offset.saturating_add(visible_rows) {
        offset = selected + 1 - visible_rows;
    }
    offset.min(row_count.saturating_sub(visible_rows.min(row_count)))
}

fn header_cells(
    visible: &[usize],
    result: &ResultSet,
    number_width: u16,
    theme: Theme,
) -> Vec<Cell<'static>> {
    let header_style = Style::new()
        .fg(theme.text)
        .bg(theme.grid_header)
        .add_modifier(Modifier::BOLD);
    let separator_style = Style::new().fg(theme.grid_border).bg(theme.grid_header);
    let mut cells = Vec::with_capacity(visible.len().saturating_mul(2).saturating_add(2));
    cells.push(
        Cell::from(format!("{:>width$}", "#", width = number_width as usize)).style(header_style),
    );
    cells.push(Cell::from("│").style(separator_style));
    for (position, index) in visible.iter().enumerate() {
        if position > 0 {
            cells.push(Cell::from("│").style(separator_style));
        }
        cells.push(
            Cell::from(sanitize_terminal_text(&result.columns[*index].name)).style(header_style),
        );
    }
    cells
}

fn body_cells(
    visible: &[usize],
    widths: &[u16],
    row_index: usize,
    number_width: u16,
    row: &[CellValue],
    row_style: Option<Style>,
    theme: Theme,
) -> Vec<Cell<'static>> {
    let separator_style = Style::new().fg(theme.grid_border).bg(theme.surface);
    let row_number_style = row_number_style(row_style, theme);
    let mut cells = Vec::with_capacity(visible.len().saturating_mul(2).saturating_add(2));
    cells.push(
        Cell::from(format!(
            "{:>width$}",
            row_index.saturating_add(1),
            width = number_width as usize
        ))
        .style(row_number_style),
    );
    cells.push(Cell::from("│").style(separator_style));
    for (position, index) in visible.iter().enumerate() {
        if position > 0 {
            cells.push(Cell::from("│").style(separator_style));
        }
        let value = row.get(*index).unwrap_or(&CellValue::Null);
        let preview = value.preview(widths[*index].saturating_sub(2) as usize);
        let style = match value {
            CellValue::Null => Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC),
            CellValue::Unsupported { .. } => Style::new().fg(theme.warning),
            _ => Style::new().fg(theme.text),
        };
        cells.push(
            Cell::from(sanitize_terminal_text(&preview.text)).style(row_style.unwrap_or(style)),
        );
    }
    cells
}

fn row_number_style(row_style: Option<Style>, theme: Theme) -> Style {
    row_style
        .map(|style| style.fg(theme.muted))
        .unwrap_or_else(|| Style::new().fg(theme.muted))
}

fn total_width(widths: &[u16]) -> u16 {
    widths
        .iter()
        .enumerate()
        .fold(0u16, |total, (index, width)| {
            total
                .saturating_add(u16::from(index > 0))
                .saturating_add(*width)
        })
}

fn row_number_width(row_count: usize) -> u16 {
    row_count
        .max(1)
        .to_string()
        .len()
        .saturating_add(2)
        .min(u16::MAX as usize) as u16
}

fn selected_data_cell(visible_position: usize) -> usize {
    2usize.saturating_add(visible_position.saturating_mul(2))
}

fn data_start_x(area: Rect, number_width: u16) -> u16 {
    area.x
        .saturating_add(2)
        .saturating_add(number_width)
        .saturating_add(1)
}

fn visible_columns(widths: &[u16], first: usize, available: u16) -> Vec<usize> {
    widths
        .iter()
        .enumerate()
        .skip(first.min(widths.len()))
        .scan(0u16, |used, (index, width)| {
            let spacing = u16::from(index > first);
            let next = used.saturating_add(spacing).saturating_add(*width);
            if *used == 0 || next <= available {
                *used = next;
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

fn viewport_start(widths: &[u16], offset: usize, selected: usize, available: u16) -> usize {
    if widths.is_empty() {
        return 0;
    }
    let selected = selected.min(widths.len() - 1);
    let mut start = offset.min(last_page_start(widths, available));
    if selected < start {
        return selected;
    }
    while !visible_columns(widths, start, available).contains(&selected) && start + 1 < widths.len()
    {
        start += 1;
    }
    start
}

fn last_page_start(widths: &[u16], available: u16) -> usize {
    let mut used = 0u16;
    for index in (0..widths.len()).rev() {
        let spacing = u16::from(used > 0);
        let next = used.saturating_add(spacing).saturating_add(widths[index]);
        if used != 0 && next > available {
            return index + 1;
        }
        used = next;
    }
    0
}

#[allow(clippy::too_many_arguments)]
fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    first: usize,
    visible: &[usize],
    column_count: usize,
    max_offset: usize,
    number_width: u16,
    theme: Theme,
    state: &mut UiState,
) {
    let track = Rect::new(
        data_start_x(area, number_width),
        area.bottom().saturating_sub(2),
        area.width
            .saturating_sub(4)
            .saturating_sub(number_width.saturating_add(1)),
        1,
    );
    if track.width < 3 || column_count == 0 {
        return;
    }
    let rail_width = track.width.saturating_sub(2);
    let thumb_width = ((rail_width as usize * visible.len().max(1)) / column_count)
        .clamp(1, rail_width as usize) as u16;
    let travel = rail_width.saturating_sub(thumb_width);
    let thumb_offset = ((travel as usize * first)
        .checked_div(max_offset)
        .unwrap_or(0)) as u16;
    let thumb_x = track.x.saturating_add(1).saturating_add(thumb_offset);
    let before = thumb_x.saturating_sub(track.x.saturating_add(1));
    let after = rail_width
        .saturating_sub(before)
        .saturating_sub(thumb_width);
    let line = Line::from(vec![
        Span::styled("‹", Style::new().fg(theme.muted)),
        Span::styled("─".repeat(before as usize), Style::new().fg(theme.muted)),
        Span::styled(
            "━".repeat(thumb_width as usize),
            Style::new().fg(theme.accent),
        ),
        Span::styled("─".repeat(after as usize), Style::new().fg(theme.muted)),
        Span::styled("›", Style::new().fg(theme.muted)),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(theme.surface)),
        track,
    );

    let page = visible.len().max(1) as isize;
    state.hit_regions.push(HitRegion {
        area: Rect::new(track.x, track.y, thumb_x.saturating_sub(track.x), 1),
        target: HitTarget::GridScrollbarPage {
            offset: first.saturating_sub(page as usize),
        },
    });
    state.hit_regions.push(HitRegion {
        area: Rect::new(thumb_x, track.y, thumb_width, 1),
        target: HitTarget::GridScrollbarThumb {
            track_x: track.x.saturating_add(1),
            track_width: rail_width,
            thumb_x,
            thumb_width,
            offset: first,
            max_offset,
        },
    });
    let after_x = thumb_x.saturating_add(thumb_width);
    state.hit_regions.push(HitRegion {
        area: Rect::new(after_x, track.y, track.right().saturating_sub(after_x), 1),
        target: HitTarget::GridScrollbarPage {
            offset: first.saturating_add(page as usize).min(max_offset),
        },
    });
}

#[cfg(test)]
mod tests {
    use super::{
        row_number_style, row_number_width, row_viewport_start, selected_data_cell, total_width,
        viewport_start, visible_columns,
    };
    use ratatui::style::Style;

    use crate::ui::theme::Theme;

    #[test]
    fn row_number_width_tracks_absolute_result_size() {
        assert_eq!(row_number_width(0), 3);
        assert_eq!(row_number_width(9), 3);
        assert_eq!(row_number_width(10), 4);
        assert_eq!(row_number_width(500), 5);
    }

    #[test]
    fn selected_data_cells_follow_the_fixed_gutter() {
        assert_eq!(selected_data_cell(0), 2);
        assert_eq!(selected_data_cell(1), 4);
    }

    #[test]
    fn row_number_stays_muted_when_row_is_selected() {
        let theme = Theme::deep_space();
        let selected_style = Style::new().fg(theme.text).bg(theme.selection);
        let style = row_number_style(Some(selected_style), theme);

        assert_eq!(style.fg, Some(theme.muted));
        assert_eq!(style.bg, Some(theme.selection));
    }

    #[test]
    fn narrow_grid_starts_with_first_columns() {
        let widths = vec![6; 10];
        let start = viewport_start(&widths, 0, 0, 20);
        assert_eq!(start, 0);
        assert_eq!(visible_columns(&widths, start, 20), vec![0, 1, 2]);
    }

    #[test]
    fn viewport_follows_selection_across_right_edge() {
        let widths = vec![6; 10];
        assert_eq!(viewport_start(&widths, 0, 3, 20), 1);
    }

    #[test]
    fn viewport_follows_selection_across_left_edge() {
        let widths = vec![6; 10];
        assert_eq!(viewport_start(&widths, 4, 2, 20), 2);
    }

    #[test]
    fn viewport_stays_put_at_left_edge_until_selection_crosses_it() {
        let widths = vec![6; 10];
        assert_eq!(viewport_start(&widths, 4, 4, 20), 4);
        assert_eq!(viewport_start(&widths, 4, 3, 20), 3);
    }

    #[test]
    fn explicit_last_offset_reaches_last_column() {
        let widths = vec![6; 10];
        let start = viewport_start(&widths, 9, 9, 20);
        assert_eq!(visible_columns(&widths, start, 20), vec![7, 8, 9]);
    }

    #[test]
    fn exact_fit_does_not_add_trailing_spacing() {
        let widths = vec![6, 6];
        assert_eq!(total_width(&widths), 13);
        assert_eq!(visible_columns(&widths, 0, 13), vec![0, 1]);
    }

    #[test]
    fn row_viewport_scrolls_only_after_selection_crosses_an_edge() {
        assert_eq!(row_viewport_start(10, 3, 0, 2), 0);
        assert_eq!(row_viewport_start(10, 3, 0, 3), 1);
        assert_eq!(row_viewport_start(10, 3, 3, 3), 3);
        assert_eq!(row_viewport_start(10, 3, 3, 2), 2);
    }
}
