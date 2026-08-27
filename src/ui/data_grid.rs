use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Cell, Row, Table, TableState},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    db::{query::ResultSet, value::CellValue},
    security::sanitize_terminal_text,
};

use super::{HitRegion, HitTarget, UiState, theme::Theme};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    result: &ResultSet,
    grid: crate::model::tab::DataGridState,
    overrides: &[Option<u16>],
    theme: Theme,
    block: Block<'_>,
    state: &mut UiState,
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
    let available = area.width.saturating_sub(4).max(1);
    let first = visible_column_start(&widths, grid.selected_column, available);
    let visible = widths
        .iter()
        .enumerate()
        .skip(first)
        .scan(0u16, |used, (index, width)| {
            let next = used.saturating_add(*width).saturating_add(1);
            if *used == 0 || next <= available {
                *used = next;
                Some(index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let constraints = visible
        .iter()
        .map(|index| Constraint::Length(widths[*index]))
        .collect::<Vec<_>>();

    let row_y = area.y.saturating_add(3);
    for (row_index, _) in result
        .rows
        .iter()
        .take(area.height.saturating_sub(4) as usize)
        .enumerate()
    {
        let mut x = area.x.saturating_add(2);
        for column_index in &visible {
            let width = widths[*column_index];
            if x >= area.right() {
                break;
            }
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    x,
                    row_y.saturating_add(row_index as u16),
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

    let mut boundary_x = area.x.saturating_add(2);
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

    let header = Row::new(visible.iter().map(|index| {
        Cell::from(sanitize_terminal_text(&result.columns[*index].name))
            .style(Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))
    }))
    .height(1)
    .bottom_margin(1);
    let rows = result.rows.iter().map(|row| {
        Row::new(visible.iter().map(|index| {
            let value = row.get(*index).unwrap_or(&CellValue::Null);
            let preview = value.preview(widths[*index].saturating_sub(2) as usize);
            let style = match value {
                CellValue::Null => Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC),
                CellValue::Unsupported { .. } => Style::new().fg(theme.warning),
                _ => Style::new().fg(theme.text),
            };
            Cell::from(sanitize_terminal_text(&preview.text)).style(style)
        }))
    });
    let table = Table::new(rows, constraints)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(theme.selection).fg(theme.text))
        .cell_highlight_style(
            Style::new()
                .bg(theme.accent)
                .fg(theme.background)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");
    let selected_column = visible
        .iter()
        .position(|index| *index == grid.selected_column)
        .unwrap_or(0);
    let mut table_state =
        TableState::new().with_selected_cell(Some((grid.selected_row, selected_column)));
    frame.render_stateful_widget(table, area, &mut table_state);
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

fn visible_column_start(widths: &[u16], selected: usize, available: u16) -> usize {
    if widths.is_empty() {
        return 0;
    }
    let mut start = selected.min(widths.len() - 1);
    loop {
        let total = widths[start..].iter().fold(0u16, |sum, width| {
            sum.saturating_add(*width).saturating_add(1)
        });
        if total <= available || start + 1 >= widths.len() {
            return start;
        }
        start += 1;
    }
}
