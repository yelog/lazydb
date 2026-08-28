use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::{app::App, db::value::CellValue, model::record_view::RecordViewState};

use super::{centered, panel_block, theme::Theme};

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    view: &RecordViewState,
    theme: Theme,
    state: &mut super::UiState,
) {
    let popup = centered(
        area,
        88,
        area.height.saturating_mul(7).saturating_div(10).max(10),
    );
    frame.render_widget(Clear, popup);
    let block = panel_block(" RECORD VIEW ", true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let Some((columns, values, row, total)) = app.active_record_snapshot() else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw("Record no longer available"),
                Line::raw("Esc/q/v close"),
            ])
            .style(Style::new().fg(theme.muted).bg(theme.surface_raised)),
            inner,
        );
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("ROW {} / {}", row + 1, total), theme.title(true)),
            Span::raw(format!("   {} fields", columns.len())),
        ])),
        chunks[0],
    );

    let field_width = columns
        .iter()
        .map(|column| column.name.chars().count())
        .max()
        .unwrap_or(5)
        .clamp(5, 24);
    let type_width = columns
        .iter()
        .map(|column| column.type_name.chars().count())
        .max()
        .unwrap_or(4)
        .clamp(4, 18);
    let value_width = usize::from(chunks[1].width)
        .saturating_sub(field_width + type_width + 6)
        .max(1);
    if let Some(tab) = app.tabs.get(app.active_tab) {
        state.record_view_fields = Some((tab.id(), chunks[1].height as usize));
    }
    let lines = columns
        .iter()
        .enumerate()
        .skip(view.field_offset)
        .take(chunks[1].height as usize)
        .map(|(index, column)| {
            let value = values.get(index).unwrap_or(&CellValue::Null);
            let preview = value.preview(value_width);
            let value_style = match value {
                CellValue::Null => Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC),
                CellValue::Unsupported { .. } => Style::new().fg(theme.warning),
                _ => Style::new().fg(theme.text),
            };
            Line::from(vec![
                Span::styled(format!("{:<field_width$}  ", column.name), theme.action),
                Span::styled(format!("{:<type_width$}  ", column.type_name), theme.muted),
                Span::styled(
                    crate::security::sanitize_terminal_text(&preview.text),
                    value_style,
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme.surface_raised)),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new("j/k fields   h/l records   gg/G first/last field   Esc close")
            .style(Style::new().fg(theme.muted).bg(theme.surface_raised)),
        chunks[2],
    );
}
