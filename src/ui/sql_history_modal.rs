use chrono::{DateTime, Local};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::collections::HashSet;

use crate::{
    app::App,
    model::{
        editor::EditorViewport,
        sql_history_view::{SqlHistoryMode, SqlHistoryState},
    },
    sql::SqlStatementKind,
    ui::{HitRegion, HitTarget, render_text_input, sql_preview, theme::Theme},
};

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    view: &SqlHistoryState,
    theme: Theme,
    state: &mut super::UiState,
) {
    let popup = super::centered(
        area,
        area.width.saturating_sub(4).min(150),
        area.height.saturating_sub(4).max(10),
    );
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.accent))
        .style(Style::new().bg(theme.surface_raised))
        .title(" EXECUTION HISTORY ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width < 12 || inner.height < 5 {
        frame.render_widget(
            Paragraph::new("Terminal too small for SQL History")
                .style(Style::new().fg(theme.warning)),
            inner,
        );
        return;
    }

    state
        .sql_history_kind_cache
        .begin(view.overlay_id, view.query_generation);

    let header_height = 1;
    let footer_height = 1;
    let content = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(1),
        Constraint::Length(footer_height),
    ])
    .split(inner);
    render_header(frame, content[0], view, theme, state);
    render_content(frame, content[1], app, view, theme, state);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ ", Style::new().fg(theme.action)),
            Span::raw("select  "),
            Span::styled("Enter ", Style::new().fg(theme.action)),
            Span::raw("SQL  "),
            Span::styled("y ", Style::new().fg(theme.action)),
            Span::raw("copy  "),
            Span::styled("Esc ", Style::new().fg(theme.action)),
            Span::raw("close"),
        ]))
        .style(Style::new().fg(theme.muted)),
        content[2],
    );
}

fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &SqlHistoryState,
    theme: Theme,
    state: &mut super::UiState,
) {
    let status = view
        .status_filter
        .map_or("all", |value| format_status(value));
    let transaction = view
        .transaction_filter
        .map_or("all", |value| format_transaction(value));
    let header = Layout::horizontal([
        Constraint::Min(12),
        Constraint::Length((status.len() + transaction.len() + 24).min(area.width as usize) as u16),
    ])
    .split(area);
    let search_area = header[0];
    let filters_area = header[1];
    if view.mode == SqlHistoryMode::Search {
        render_text_input(
            frame,
            search_area,
            " Search: ",
            &view.search,
            Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
            state,
        );
    } else {
        let query = if view.search.value().is_empty() {
            "Search: all SQL".to_owned()
        } else {
            format!("Search: {}", view.search.value())
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {query} "),
                Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
            )),
            search_area,
        );
    }
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!("status: {status}  transaction: {transaction}"),
            Style::new().fg(theme.muted),
        )),
        filters_area,
    );
}

fn render_content(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    view: &SqlHistoryState,
    theme: Theme,
    state: &mut super::UiState,
) {
    let split = if area.width >= 92 {
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area)
    } else {
        Layout::vertical([Constraint::Percentage(48), Constraint::Percentage(52)]).split(area)
    };
    render_list(frame, split[0], app, view, theme, state);
    render_detail(frame, split[1], app, view, theme, state);
}

fn render_list(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    view: &SqlHistoryState,
    theme: Theme,
    state: &mut super::UiState,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.border))
        .title(" SQL / MONGO ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    const TYPE_WIDTH: u16 = 6;
    let has_type_column = inner.width > TYPE_WIDTH;
    let content_x = inner
        .x
        .saturating_add(if has_type_column { TYPE_WIDTH } else { 0 });
    let content_width = inner
        .width
        .saturating_sub(if has_type_column { TYPE_WIDTH } else { 0 });
    let mut visible_ids = HashSet::new();
    let mut y = inner.y;
    for (index, item) in view.items.iter().enumerate().skip(view.list_offset) {
        if y >= inner.bottom() {
            break;
        }
        let selected = view.selected_execution == Some(item.execution_id);
        let remaining = inner.bottom().saturating_sub(y);
        if remaining < 2 {
            break;
        }
        let dialect = app.sql_history_dialect(item.profile_id);
        let kind = (item.language == crate::model::sql_history::HistoryLanguage::Sql).then(|| {
            state
                .sql_history_kind_cache
                .kind(item.execution_id, &item.sql, dialect)
        });
        visible_ids.insert(item.execution_id);
        let preview_height = remaining.saturating_sub(1).min(3);
        let display_text = if item.language == crate::model::sql_history::HistoryLanguage::Mongo {
            format!(
                "db.{} · {}",
                item.database.as_deref().unwrap_or("?"),
                item.mongo_command.as_deref().unwrap_or("command")
            )
        } else {
            item.sql.clone()
        };
        let lines = if item.language != crate::model::sql_history::HistoryLanguage::Sql {
            display_text
                .lines()
                .map(|line| Line::raw(crate::security::sanitize_terminal_text(line)))
                .collect()
        } else {
            sql_preview::lines_without_line_numbers(
                &item.sql,
                dialect,
                content_width as usize,
                theme,
            )
        };
        let shown = lines
            .into_iter()
            .take(preview_height as usize)
            .collect::<Vec<_>>();
        let metadata = Line::from(Span::styled(
            format!(
                "{} · {} · {}",
                format_timestamp(item.requested_at),
                item.database.as_deref().unwrap_or("—"),
                format_status(item.status)
            ),
            Style::new().fg(theme.muted),
        ));
        let row_height = shown.len().max(1) as u16 + 1;
        if y.saturating_add(row_height) > inner.bottom() {
            break;
        }
        let row_area = Rect::new(inner.x, y, inner.width, row_height);
        state.hit_regions.push(HitRegion {
            area: row_area,
            target: HitTarget::SqlHistoryRow(index),
        });
        let row_style = Style::new().bg(if selected {
            theme.selection
        } else {
            theme.surface_raised
        });
        frame.render_widget(Paragraph::new(" ").style(row_style), row_area);
        if has_type_column {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    if item.language == crate::model::sql_history::HistoryLanguage::Mongo {
                        "[MONGO] "
                    } else if item.language == crate::model::sql_history::HistoryLanguage::Rest {
                        "[REST]  "
                    } else {
                        type_label(kind.unwrap_or(SqlStatementKind::Other))
                    },
                    if item.language == crate::model::sql_history::HistoryLanguage::Mongo {
                        Style::new().fg(theme.action).add_modifier(Modifier::BOLD)
                    } else {
                        type_style(kind.unwrap_or(SqlStatementKind::Other), theme)
                    },
                ))
                .style(row_style),
                Rect::new(inner.x, y, TYPE_WIDTH, 1),
            );
        }
        for (line_index, line) in shown.into_iter().enumerate() {
            frame.render_widget(
                Paragraph::new(line).style(row_style),
                Rect::new(
                    content_x,
                    y.saturating_add(line_index as u16),
                    content_width,
                    1,
                ),
            );
        }
        frame.render_widget(
            Paragraph::new(metadata).style(row_style),
            Rect::new(
                content_x,
                y.saturating_add(row_height - 1),
                content_width,
                1,
            ),
        );
        y = y.saturating_add(row_height);
    }
    state.sql_history_kind_cache.retain(&visible_ids);
    if view.loading && view.items.is_empty() {
        frame.render_widget(Paragraph::new(" Loading…"), inner);
    } else if view.items.is_empty() && view.error.is_none() {
        frame.render_widget(Paragraph::new(" No execution history"), inner);
    }
}

fn type_label(kind: SqlStatementKind) -> &'static str {
    match kind {
        SqlStatementKind::Dql => "[DQL] ",
        SqlStatementKind::Dml => "[DML] ",
        SqlStatementKind::Ddl => "[DDL] ",
        SqlStatementKind::Dcl => "[DCL] ",
        SqlStatementKind::Tcl => "[TCL] ",
        SqlStatementKind::Mixed => "[MIX] ",
        SqlStatementKind::Other => "[SQL] ",
    }
}

fn type_style(kind: SqlStatementKind, theme: Theme) -> Style {
    let color = match kind {
        SqlStatementKind::Dql => theme.action,
        SqlStatementKind::Dml => theme.warning,
        SqlStatementKind::Ddl => theme.syntax_type,
        SqlStatementKind::Dcl => theme.syntax_parameter,
        SqlStatementKind::Tcl => theme.accent,
        SqlStatementKind::Mixed | SqlStatementKind::Other => theme.muted,
    };
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

fn render_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    view: &SqlHistoryState,
    theme: Theme,
    state: &mut super::UiState,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.border))
        .title(" DETAILS ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(item) = view.selected_item() else {
        frame.render_widget(Paragraph::new(" Select an execution record"), inner);
        return;
    };
    let info_height = inner.height.min(7);
    let sections =
        Layout::vertical([Constraint::Length(info_height), Constraint::Min(1)]).split(inner);
    let info = vec![
        Line::from(format!(
            "Time       {}",
            format_timestamp(item.requested_at)
        )),
        Line::from(format!(
            "Database   {}",
            item.database.as_deref().unwrap_or("—")
        )),
        Line::from(format!(
            "Schema     {}",
            item.schema.as_deref().unwrap_or("—")
        )),
        Line::from(format!("Status     {}", format_status(item.status))),
        Line::from(format!(
            "Elapsed    {}",
            item.elapsed_millis
                .map_or("—".into(), |v| format!("{v} ms"))
        )),
        Line::from(format!(
            "Rows       {}",
            item.returned_rows.map_or("—".into(), |v| v.to_string())
        )),
        Line::from(format!(
            "Affected   {}",
            item.affected_rows.map_or("—".into(), |v| v.to_string())
        )),
    ];
    frame.render_widget(
        Paragraph::new(info).style(Style::new().fg(theme.text)),
        sections[0],
    );
    if view.selected_item().is_some() {
        let code_area = sections[1];
        let viewport = EditorViewport {
            // The read-only renderer reserves a five-cell gutter for the
            // common case (up to four-digit line numbers plus a separator).
            width: code_area.width.saturating_sub(2).saturating_sub(5) as usize,
            height: code_area.height.saturating_sub(1) as usize,
        };
        if let Ok(snapshot) = app.sql_history_editor_snapshot(viewport) {
            let mongo = view.selected_item().is_some_and(|item| {
                item.language == crate::model::sql_history::HistoryLanguage::Mongo
            });
            super::read_only_sql::ReadOnlySqlEditor {
                session_id: view.editor_session_id,
                snapshot: &snapshot,
                block: Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::new().fg(theme.border))
                    .title(if mongo { " MONGO SUMMARY " } else { " SQL " }),
                focused: view.mode == SqlHistoryMode::Sql,
                show_line_numbers: true,
            }
            .render(frame, code_area, theme, state);
        }
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "SQL is shown on Enter",
                Style::new().fg(theme.muted),
            )))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::new().fg(theme.border)),
            ),
            sections[1],
        );
    }
}

fn format_timestamp(millis: i64) -> String {
    DateTime::from_timestamp_millis(millis)
        .map(|value| {
            value
                .with_timezone(&Local)
                .format("%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "unknown time".into())
}

fn format_status(status: crate::model::sql_history::HistoryExecutionStatus) -> &'static str {
    use crate::model::sql_history::HistoryExecutionStatus::*;
    match status {
        Queued => "queued",
        Running => "running",
        Succeeded => "success",
        Failed => "failed",
        TimedOut => "timeout",
        Cancelled => "cancelled",
        Interrupted => "interrupted",
        NotExecuted => "not executed",
    }
}

fn format_transaction(
    outcome: crate::model::sql_history::HistoryTransactionOutcome,
) -> &'static str {
    use crate::model::sql_history::HistoryTransactionOutcome::*;
    match outcome {
        NotApplicable => "—",
        Pending => "pending",
        AutoCommitted => "auto-commit",
        Committed => "committed",
        RolledBack => "rolled back",
        RolledBackToSavepoint => "savepoint",
        Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_labels_have_a_fixed_six_cell_width() {
        for kind in [
            SqlStatementKind::Dql,
            SqlStatementKind::Dml,
            SqlStatementKind::Ddl,
            SqlStatementKind::Dcl,
            SqlStatementKind::Tcl,
            SqlStatementKind::Mixed,
            SqlStatementKind::Other,
        ] {
            assert_eq!(unicode_width::UnicodeWidthStr::width(type_label(kind)), 6);
        }
    }
}
