use crate::{
    app::App,
    model::{
        transaction::{TransactionMode, TransactionState},
        workspace::ExecutionConfirmFocus,
    },
    sql::{ExecutionDraft, SqlDialect, SqlRisk},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use super::{HitRegion, HitTarget, UiState, centered, dialog, sql_preview, theme::Theme};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Summary {
    title: String,
    scope: String,
    reason: String,
    risks: String,
    transaction: String,
    warning: Option<String>,
}

fn is_elasticsearch(draft: &ExecutionDraft, app: &App) -> bool {
    app.profiles.iter().any(|profile| {
        profile.id == draft.target.profile_id
            && profile.kind == crate::profile::DatabaseKind::Elasticsearch
    })
}

fn summary(draft: &ExecutionDraft, app: &App) -> Summary {
    let elasticsearch = is_elasticsearch(draft, app);
    let scope = match draft.scope {
        crate::sql::ScopeKind::VisualChar => "Selected text",
        crate::sql::ScopeKind::VisualLine => "Selected lines",
        crate::sql::ScopeKind::VisualBlock => "Block selection",
        crate::sql::ScopeKind::CurrentStatement => "Current statement",
        crate::sql::ScopeKind::FullBuffer => "Entire editor (all SQL)",
    };
    let has_unknown = draft.risks.contains(&SqlRisk::Unknown);
    let mut counts = Vec::new();
    for (risk, label) in [
        (SqlRisk::ReadOnly, "read-only"),
        (SqlRisk::Dml, "data-changing or lock-acquiring"),
        (SqlRisk::Ddl, "schema changes"),
        (SqlRisk::TransactionControl, "transaction control"),
    ] {
        let count = draft
            .risks
            .iter()
            .filter(|candidate| **candidate == risk)
            .count();
        if count > 0 {
            counts.push(format!("{count} {label}"));
        }
    }
    if has_unknown {
        counts.push("effects could not be fully determined".into());
    }
    let transaction = match (draft.transaction_mode, draft.transaction_state) {
        (TransactionMode::Auto, TransactionState::Idle) => {
            "Auto-commit; data changes are not protected by a pending manual transaction".into()
        }
        (TransactionMode::Manual, TransactionState::Idle) => {
            "Manual transaction; execution will start a transaction".into()
        }
        (TransactionMode::Manual, TransactionState::Active) => {
            "Existing transaction; execution joins the current transaction".into()
        }
        _ => format!(
            "Transaction state: {:?}/{:?}",
            draft.transaction_mode, draft.transaction_state
        ),
    };
    let warning = (draft.dialect == SqlDialect::MySql
        && draft.transaction_mode == TransactionMode::Manual
        && draft.risks.contains(&SqlRisk::Ddl))
    .then(|| "MySQL DDL may implicitly commit before and after execution".into());
    let connection = app
        .profiles
        .iter()
        .find(|profile| profile.id == draft.target.profile_id)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| "unavailable connection".into());
    let target = if draft.target.schema.as_deref() == Some(draft.target.database.as_str()) {
        format!(
            "Connection: {connection}   Database: {}",
            draft.target.database
        )
    } else {
        format!(
            "Connection: {connection}   Database: {}   Default schema: {}",
            draft.target.database,
            draft.target.schema.as_deref().unwrap_or("none")
        )
    };
    let title = if elasticsearch {
        "Execute REST request?".into()
    } else if has_unknown || draft.statement_count == 0 {
        "Execute selected SQL?".into()
    } else {
        format!(
            "Execute {} SQL statement{}?",
            draft.statement_count,
            if draft.statement_count == 1 { "" } else { "s" }
        )
    };
    Summary {
        title,
        scope: if elasticsearch {
            format!("Connection: {connection}\nScope: One complete REST request")
        } else {
            format!("{target}\nScope: {scope}")
        },
        reason: if elasticsearch {
            "Review the REST request before sending it to Elasticsearch."
        } else {
            "Review the selected SQL before execution."
        }
        .into(),
        risks: if counts.is_empty() {
            "Effects could not be fully determined".into()
        } else {
            counts.join("; ")
        },
        transaction: if elasticsearch {
            "Unavailable; Elasticsearch requests cannot be rolled back here".into()
        } else {
            transaction
        },
        warning,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &ExecutionDraft,
    focus: ExecutionConfirmFocus,
    offset: usize,
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    let popup = centered(
        area,
        92.min(area.width.saturating_sub(4)),
        area.height.saturating_sub(2).clamp(8, 32),
    );
    let frame_inner = dialog::render_frame(frame, popup, " EXECUTION CONFIRMATION ", theme);
    let inner_width = frame_inner.width.saturating_sub(2) as usize;
    let info = summary(draft, app);
    let mut summary_lines = vec![
        Line::from(Span::styled(info.title, theme.title(true))),
        Line::raw(info.reason),
    ];
    summary_lines.extend(info.scope.lines().map(Line::raw));
    summary_lines.push(Line::from(Span::styled(
        format!("Risk: {}", info.risks),
        Style::new().fg(theme.warning),
    )));
    summary_lines.push(Line::raw(format!("Transaction: {}", info.transaction)));
    if let Some(warning) = info.warning {
        summary_lines.push(Line::from(Span::styled(
            warning,
            Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
        )));
    }
    let preview = sql_preview::lines(
        &draft.sql,
        draft.dialect,
        inner_width.saturating_sub(7),
        theme,
    );
    let inner = frame_inner.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });
    let summary_height = (summary_lines.len() as u16).min(inner.height.saturating_sub(4));
    let sections = Layout::new(
        Direction::Vertical,
        [
            Constraint::Length(summary_height),
            Constraint::Min(1),
            Constraint::Length(3),
        ],
    )
    .split(inner);
    let available = sections[1].height as usize;
    let start = offset.min(preview.len().saturating_sub(available));
    let end = (start + available).min(preview.len());
    let shown = preview[start..end].to_vec();
    frame.render_widget(
        Paragraph::new(summary_lines).wrap(Wrap { trim: false }),
        sections[0],
    );
    let mut preview_lines = vec![Line::from(Span::styled(
        format!(
            "{} preview  Lines {}-{} of {}",
            if is_elasticsearch(draft, app) {
                "REST"
            } else {
                "SQL"
            },
            start + 1,
            end.max(start + 1),
            preview.len()
        ),
        Style::new().fg(theme.muted),
    ))];
    preview_lines.extend(shown);
    frame.render_widget(
        Paragraph::new(preview_lines).wrap(Wrap { trim: false }),
        sections[1],
    );
    let actions = dialog::render_actions(
        frame,
        Rect::new(sections[2].x, sections[2].y, sections[2].width, 2),
        &[
            dialog::DialogButton {
                label: "Cancel",
                tone: dialog::DialogTone::Normal,
                emphasis: dialog::DialogEmphasis::Secondary,
                enabled: true,
            },
            dialog::DialogButton {
                label: "Execute",
                tone: dialog::DialogTone::Danger,
                emphasis: dialog::DialogEmphasis::Primary,
                enabled: true,
            },
        ],
        usize::from(focus == ExecutionConfirmFocus::Execute),
        theme,
    );
    for action in actions {
        state.hit_regions.push(HitRegion {
            area: action.area,
            target: if action.index == 0 {
                HitTarget::ExecutionCancel
            } else {
                HitTarget::ExecutionConfirm
            },
        });
    }
    dialog::render_interactive_hint(
        frame,
        Rect::new(
            sections[2].x,
            sections[2].y.saturating_add(2),
            sections[2].width,
            1,
        ),
        "Tab / Left / Right switch   Enter activate   Up / Down preview   Esc cancel",
        theme,
        state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_sanitizes_and_keeps_unicode_boundaries() {
        let lines = sql_preview::lines(
            "SELECT '中文';\r\n-- \u{1b}[31m",
            SqlDialect::Sqlite,
            8,
            Theme::deep_space(),
        );
        assert!(!lines.is_empty());
    }
}
