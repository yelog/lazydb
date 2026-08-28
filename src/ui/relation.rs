use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{panel_block, theme::Theme};
use crate::{
    app::App,
    model::{
        relation::{RelationLoad, RelationSnapshotProvenance, RelationView},
        tab::WorkspaceTab,
        workspace::Focus,
    },
    security::sanitize_terminal_text,
    sql::{self, HighlightKind, SqlDialect},
    ui::{HitRegion, HitTarget},
};

pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
) {
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(area);
    let data_style = if tab.view == RelationView::Data {
        theme.accent
    } else {
        theme.muted
    };
    let ddl_style = if tab.view == RelationView::Ddl {
        theme.accent
    } else {
        theme.muted
    };
    let tabs = Line::from(vec![
        Span::styled(
            " DATA ",
            Style::new().fg(data_style).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " DDL ",
            Style::new().fg(ddl_style).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", sanitize_terminal_text(&tab.descriptor.title)),
            Style::new().fg(theme.text),
        ),
    ]);
    frame.render_widget(Paragraph::new(tabs).style(theme.base()), chunks[0]);
    state.hit_regions.push(HitRegion {
        area: Rect::new(chunks[0].x, chunks[0].y, 6, 1),
        target: HitTarget::RelationView(RelationView::Data),
    });
    state.hit_regions.push(HitRegion {
        area: Rect::new(chunks[0].x.saturating_add(7), chunks[0].y, 11, 1),
        target: HitTarget::RelationView(RelationView::Ddl),
    });
    match tab.view {
        RelationView::Data => render_data(frame, chunks[1], app, theme, state),
        RelationView::Ddl => render_ddl(frame, chunks[1], app, theme, state),
    }
    if let Some(crate::model::relation_edit::RelationEditSession {
        mode: crate::model::relation_edit::RelationGridMode::EditCell(editor),
        ..
    }) = &tab.edit
    {
        let popup = Rect::new(
            area.x.saturating_add(4),
            area.y.saturating_add(3),
            area.width.saturating_sub(8).min(72),
            3,
        );
        frame.render_widget(ratatui::widgets::Clear, popup);
        frame.render_widget(
            Paragraph::new(cell_editor_value(editor))
                .block(panel_block(" CELL EDITOR ", true, theme))
                .style(theme.base()),
            popup,
        );
    }
}

fn cell_editor_value(editor: &crate::model::relation_edit::CellEditorState) -> String {
    editor.input.value().to_owned()
}

fn render_data(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
) {
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let (snapshot, status) = match &tab.data {
        RelationLoad::Ready(snapshot) => (Some(snapshot), None),
        RelationLoad::Loading { previous, .. } => {
            (previous.as_ref(), Some(("Refreshing", false, true)))
        }
        RelationLoad::Failed { message, previous } => {
            (previous.as_ref(), Some((message.as_str(), true, false)))
        }
        RelationLoad::Cancelled { previous } => {
            (previous.as_ref(), Some(("Cancelled", true, false)))
        }
        RelationLoad::Empty => (None, Some(("No relation data", false, false))),
    };
    if let Some(snapshot) = snapshot {
        let mut result = snapshot
            .value
            .result
            .result_sets
            .last()
            .cloned()
            .unwrap_or_default();
        if let Some(edit) = &tab.edit {
            result.rows = edit.rows.iter().map(|row| row.current.clone()).collect();
        }
        let query_height = if tab.query.error.is_some() { 3 } else { 2 };
        let body = if status.is_some() {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(query_height),
                    Constraint::Length(2),
                    Constraint::Min(1),
                ])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(query_height),
                    Constraint::Length(0),
                    Constraint::Min(1),
                ])
                .split(area)
        };
        super::query_bar::render(frame, body[0], &tab.query, theme, state);
        if let Some((message, retry, cancel)) = status {
            render_status(frame, body[1], message, retry, cancel, theme, state);
        }
        let block = panel_block(" RELATION DATA ", app.focus == Focus::Results, theme);
        render_relation_result_table(
            frame,
            body[2],
            tab.id,
            &result,
            tab.grid.clone(),
            &tab.grid.column_widths,
            theme,
            block,
            state,
            tab.edit.as_ref(),
        );
        let sql = sanitize_terminal_text(&snapshot.value.sql);
        let footer = Rect::new(
            body[2].x,
            body[2].bottom().saturating_sub(1),
            body[2].width,
            1,
        );
        let provenance = tab
            .provenance(
                RelationView::Data,
                app.connection.active_identity(),
                app.active_profile(),
            )
            .map(provenance_label)
            .unwrap_or("UNKNOWN");
        frame.render_widget(
            Paragraph::new(format!(
                "SQL: {sql}  [500 row limit]  {} rows  Snapshot: {provenance}",
                result.rows.len()
            ))
            .style(Style::new().fg(theme.muted).bg(theme.surface)),
            footer,
        );
    } else if let Some((message, retry, cancel)) = status {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(area);
        super::query_bar::render(frame, body[0], &tab.query, theme, state);
        render_status(frame, body[1], message, retry, cancel, theme, state);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_relation_result_table(
    frame: &mut Frame<'_>,
    area: Rect,
    tab_id: uuid::Uuid,
    result: &crate::db::query::ResultSet,
    grid: crate::model::tab::DataGridState,
    overrides: &[Option<u16>],
    theme: Theme,
    block: ratatui::widgets::Block<'_>,
    state: &mut super::UiState,
    edit: Option<&crate::model::relation_edit::RelationEditSession>,
) {
    super::data_grid::render(
        frame, area, tab_id, result, grid, overrides, theme, block, state, edit,
    );
}

fn render_status(
    frame: &mut Frame<'_>,
    area: Rect,
    message: &str,
    retry: bool,
    cancel: bool,
    theme: Theme,
    state: &mut super::UiState,
) {
    let message = clean(message);
    let label = if retry {
        "r  retry"
    } else if cancel {
        "Ctrl-C  cancel"
    } else {
        ""
    };
    let text = if label.is_empty() {
        message
    } else {
        format!("{}  [{}]", message, label)
    };
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(theme.warning).bg(theme.surface_raised)),
        area,
    );
    if retry || cancel {
        state.hit_regions.push(HitRegion {
            area,
            target: if retry {
                HitTarget::RelationRetry
            } else {
                HitTarget::RelationCancel
            },
        });
    }
}

fn render_ddl(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    _state: &mut super::UiState,
) {
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let (body, status) = match &tab.ddl {
        RelationLoad::Ready(snapshot) => (ddl_text(&snapshot.value.sql), None),
        RelationLoad::Loading { previous, .. } => (
            previous
                .as_ref()
                .map(|s| ddl_text(&s.value.sql))
                .unwrap_or_default(),
            Some(("Refreshing", false, true)),
        ),
        RelationLoad::Failed { message, previous } => (
            previous
                .as_ref()
                .map(|s| ddl_text(&s.value.sql))
                .unwrap_or_default(),
            Some((message.as_str(), true, false)),
        ),
        RelationLoad::Cancelled { previous } => (
            previous
                .as_ref()
                .map(|s| ddl_text(&s.value.sql))
                .unwrap_or_default(),
            Some(("Cancelled", true, false)),
        ),
        RelationLoad::Empty => (String::new(), Some(("No DDL available", false, false))),
    };
    let block = panel_block(" RELATION DDL ", app.focus == Focus::Results, theme);
    let dialect = app.sql_dialect();
    if let Some((message, retry, cancel)) = status {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(area);
        render_status(frame, chunks[0], message, retry, cancel, theme, _state);
        render_ddl_body(
            frame,
            chunks[1],
            &body,
            tab.ddl_viewport.row_offset,
            tab.ddl_viewport.column_offset,
            block,
            theme,
            dialect,
        );
        set_ddl_metrics(_state, chunks[1], &body);
        return;
    }
    render_ddl_body(
        frame,
        area,
        &body,
        tab.ddl_viewport.row_offset,
        tab.ddl_viewport.column_offset,
        block,
        theme,
        dialect,
    );
    set_ddl_metrics(_state, area, &body);
}

#[allow(clippy::too_many_arguments)]
fn render_ddl_body(
    frame: &mut Frame<'_>,
    area: Rect,
    body: &str,
    row_offset: usize,
    column_offset: usize,
    block: ratatui::widgets::Block<'_>,
    theme: Theme,
    dialect: SqlDialect,
) {
    let inner = block.inner(area);
    let lines = highlighted_ddl_lines(
        body,
        dialect,
        row_offset,
        column_offset,
        inner.width as usize,
        inner.height as usize,
        theme,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(Style::new().fg(theme.text).bg(theme.surface))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn set_ddl_metrics(state: &mut super::UiState, area: Rect, body: &str) {
    let inner = panel_block("", false, Theme::default()).inner(area);
    state.ddl_viewport = Some(super::DdlViewportMetrics {
        visible_rows: inner.height as usize,
        visible_columns: inner.width as usize,
        total_rows: body.lines().count().max(1),
        max_line_width: body.lines().map(UnicodeWidthStr::width).max().unwrap_or(0),
    });
}

fn ddl_text(sql: &str) -> String {
    sanitize_terminal_text(sql)
}

fn highlighted_ddl_lines(
    body: &str,
    dialect: SqlDialect,
    row_offset: usize,
    column_offset: usize,
    width: usize,
    height: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let highlights = sql::highlight_sql(body, dialect);
    let mut lines = Vec::new();
    let mut line_start = 0;

    for (line_number, raw_line) in body.split('\n').enumerate() {
        if line_number >= row_offset && lines.len() < height {
            let line_end = line_start + raw_line.len();
            let mut spans = Vec::new();
            let mut byte = line_start;
            for highlight in highlights
                .iter()
                .filter(|item| item.range.start < line_end && item.range.end > line_start)
            {
                let start = highlight.range.start.max(line_start).min(line_end);
                let end = highlight.range.end.min(line_end);
                if start > byte {
                    push_ddl_span(&mut spans, &body[byte..start], HighlightKind::Plain, theme);
                }
                if end > start {
                    push_ddl_span(&mut spans, &body[start..end], highlight.kind, theme);
                    byte = end;
                }
            }
            if byte < line_end {
                push_ddl_span(
                    &mut spans,
                    &body[byte..line_end],
                    HighlightKind::Plain,
                    theme,
                );
            }
            lines.push(styled_horizontal_slice(spans, column_offset, width));
        }
        line_start += raw_line.len() + 1;
    }
    lines
}

fn push_ddl_span(spans: &mut Vec<Span<'static>>, text: &str, kind: HighlightKind, theme: Theme) {
    if text.is_empty() {
        return;
    }
    let style = Style::new().fg(theme.syntax_color(ddl_syntax_color(kind)));
    if let Some(previous) = spans.last_mut()
        && previous.style == style
    {
        previous.content.to_mut().push_str(text);
    } else {
        spans.push(Span::styled(text.to_owned(), style));
    }
}

fn styled_horizontal_slice(
    spans: Vec<Span<'static>>,
    offset: usize,
    width: usize,
) -> Line<'static> {
    let end = offset.saturating_add(width);
    let mut result: Vec<Span<'static>> = Vec::new();
    let mut cursor: usize = 0;
    for span in spans {
        let style = span.style;
        for character in span.content.chars() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            let character_end = cursor.saturating_add(character_width);
            if character_width > 0 && cursor >= offset && character_end <= end {
                if let Some(previous) = result.last_mut()
                    && previous.style == style
                {
                    previous.content.to_mut().push(character);
                } else {
                    result.push(Span::styled(character.to_string(), style));
                }
            }
            cursor = character_end;
        }
    }
    Line::from(result)
}

fn ddl_syntax_color(kind: HighlightKind) -> super::theme::SyntaxColor {
    match kind {
        HighlightKind::Keyword => super::theme::SyntaxColor::Keyword,
        HighlightKind::Identifier => super::theme::SyntaxColor::Identifier,
        HighlightKind::String => super::theme::SyntaxColor::String,
        HighlightKind::Number => super::theme::SyntaxColor::Number,
        HighlightKind::Comment => super::theme::SyntaxColor::Comment,
        HighlightKind::Operator => super::theme::SyntaxColor::Operator,
        HighlightKind::Punctuation => super::theme::SyntaxColor::Punctuation,
        HighlightKind::Parameter => super::theme::SyntaxColor::Parameter,
        HighlightKind::Plain => super::theme::SyntaxColor::Plain,
    }
}
fn clean(value: &str) -> String {
    sanitize_terminal_text(value).chars().take(240).collect()
}
pub(crate) fn provenance_label(value: RelationSnapshotProvenance) -> &'static str {
    match value {
        RelationSnapshotProvenance::Live => "LIVE",
        RelationSnapshotProvenance::OfflineSnapshot => "OFFLINE SNAPSHOT",
        RelationSnapshotProvenance::ProfileDeletedSnapshot => "PROFILE DELETED SNAPSHOT",
        RelationSnapshotProvenance::OutOfScopeSnapshot => "OUT OF SCOPE SNAPSHOT",
    }
}

#[cfg(test)]
mod tests {
    use super::{cell_editor_value, highlighted_ddl_lines};
    use crate::{
        model::{relation_edit::CellEditorState, text_input::TextInput},
        sql::SqlDialect,
    };

    #[test]
    fn cell_editor_value_contains_only_the_cell_content() {
        let editor = CellEditorState {
            row: 5,
            column: 8,
            input: TextInput::from("failed"),
        };

        assert_eq!(cell_editor_value(&editor), "failed");
        assert!(!cell_editor_value(&editor).contains("Edit cell"));
        assert!(!cell_editor_value(&editor).contains("[6, 9]"));
    }

    #[test]
    fn ddl_text_sanitizes_without_the_clean_length_limit() {
        let sql = "SELECT [31m".to_owned() + &"x".repeat(300);
        let rendered = super::ddl_text(&sql);
        assert!(rendered.len() > 240);
        assert!(rendered.contains("<ESC>"));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn ddl_lines_apply_sql_token_styles() {
        let theme = super::Theme::deep_space();
        let lines = highlighted_ddl_lines(
            "CREATE TABLE users (name TEXT DEFAULT 'Ada'); -- note",
            SqlDialect::Postgres,
            0,
            0,
            120,
            10,
            theme,
        );

        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.content == "CREATE" && span.style.fg == Some(theme.accent))
        );
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.content == "users" && span.style.fg == Some(theme.action))
        );
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.content == "'Ada'" && span.style.fg == Some(theme.warning))
        );
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.content.contains("-- note") && span.style.fg == Some(theme.muted))
        );
    }

    #[test]
    fn ddl_highlighting_preserves_original_text_and_lines() {
        let sql = "CREATE TABLE users (\n  id INTEGER,\n  note TEXT\n);";
        let lines = highlighted_ddl_lines(
            sql,
            SqlDialect::Sqlite,
            0,
            0,
            120,
            20,
            super::Theme::deep_space(),
        );
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(rendered, sql);
    }

    #[test]
    fn ddl_horizontal_slice_preserves_style_and_display_width() {
        let theme = super::Theme::deep_space();
        let lines = highlighted_ddl_lines(
            "SELECT 数据, '值' FROM users;",
            SqlDialect::Postgres,
            0,
            7,
            8,
            1,
            theme,
        );

        assert_eq!(lines.len(), 1);
        assert!(lines[0].width() <= 8);
        assert!(
            lines[0]
                .spans
                .iter()
                .any(|span| span.style.fg == Some(theme.action))
        );
    }

    #[test]
    fn ddl_horizontal_slice_never_emits_replacement_characters() {
        let line = super::styled_horizontal_slice(
            vec![ratatui::text::Span::styled(
                "A数据B",
                ratatui::style::Style::new().fg(ratatui::style::Color::Blue),
            )],
            2,
            3,
        );

        assert!(line.width() <= 3);
        assert!(
            !line
                .spans
                .iter()
                .any(|span| span.content.contains('\u{fffd}'))
        );
    }
}
