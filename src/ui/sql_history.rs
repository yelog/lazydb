use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use crate::{app::App, model::tab::WorkspaceTab, ui::theme::Theme};

pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let Some(WorkspaceTab::History(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let lines = if tab.loading {
        vec![Line::from("Loading SQL history…")]
    } else if tab.items.is_empty() {
        vec![Line::from("No SQL execution history")]
    } else {
        tab.items
            .iter()
            .map(|item| {
                let rows = item
                    .affected_rows
                    .map_or_else(|| "—".into(), |rows| rows.to_string());
                Line::from(format!(
                    "{}  {:<10} {:<8} {:<8} {}",
                    item.requested_at,
                    format_status(item.status),
                    format_transaction(item.transaction_outcome),
                    rows,
                    item.sql.lines().next().unwrap_or_default()
                ))
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(theme.text))
            .block(Block::default().borders(Borders::ALL).title("SQL History")),
        area,
    );
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
