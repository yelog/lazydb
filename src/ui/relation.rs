use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
};
use unicode_width::UnicodeWidthStr;

use super::{panel_block, theme::Theme};
use crate::{
    app::App,
    db::catalog::{CatalogMetadata, ConstraintMetadata, IndexMetadata, OptionalMetadata},
    model::{
        relation::{RelationLoad, RelationSnapshotProvenance, RelationView},
        tab::WorkspaceTab,
        workspace::Focus,
    },
    security::sanitize_terminal_text,
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
    let structure_style = if tab.view == RelationView::Structure {
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
            " STRUCTURE ",
            Style::new()
                .fg(structure_style)
                .add_modifier(Modifier::BOLD),
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
        target: HitTarget::RelationView(RelationView::Structure),
    });
    match tab.view {
        RelationView::Data => render_data(frame, chunks[1], app, theme, state),
        RelationView::Structure => render_structure(frame, chunks[1], app, theme, state),
    }
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
        let result = snapshot
            .value
            .result
            .result_sets
            .last()
            .cloned()
            .unwrap_or_default();
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
        render_query_inputs(frame, body[0], tab, theme, state);
        if let Some((message, retry, cancel)) = status {
            render_status(frame, body[1], message, retry, cancel, theme, state);
        }
        let block = panel_block(" RELATION DATA ", app.focus == Focus::Results, theme);
        render_relation_result_table(
            frame,
            body[2],
            &result,
            tab.grid.clone(),
            &tab.column_widths,
            theme,
            block,
            state,
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
        render_query_inputs(frame, body[0], tab, theme, state);
        render_status(frame, body[1], message, retry, cancel, theme, state);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_relation_result_table(
    frame: &mut Frame<'_>,
    area: Rect,
    result: &crate::db::query::ResultSet,
    grid: crate::model::tab::GridState,
    overrides: &[Option<u16>],
    theme: Theme,
    block: ratatui::widgets::Block<'_>,
    state: &mut super::UiState,
) {
    if result.columns.is_empty() {
        super::render_result_table(frame, area, result, grid, theme, block, state);
        return;
    }
    let auto = crate::model::relation::automatic_relation_column_widths(result);
    let widths = auto
        .iter()
        .enumerate()
        .map(|(index, width)| {
            overrides
                .get(index)
                .and_then(|value| *value)
                .unwrap_or(*width)
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
    for (row_index, _row) in result
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
                    row_y + row_index as u16,
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
    }
    let header = Row::new(visible.iter().map(|index| {
        let column = &result.columns[*index];
        Cell::from(sanitize_terminal_text(&column.name))
            .style(Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))
    }))
    .height(1)
    .bottom_margin(1);
    let rows = result.rows.iter().map(|row| {
        Row::new(visible.iter().map(|index| {
            let value = row
                .get(*index)
                .unwrap_or(&crate::db::value::CellValue::Null);
            let width = widths[*index];
            let preview = value.preview(width.saturating_sub(2) as usize);
            let style = match value {
                crate::db::value::CellValue::Null => {
                    Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC)
                }
                crate::db::value::CellValue::Unsupported { .. } => Style::new().fg(theme.warning),
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
    let mut table_state = ratatui::widgets::TableState::new()
        .with_selected_cell(Some((grid.selected_row, selected_column)));
    frame.render_stateful_widget(table, area, &mut table_state);
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

fn render_query_inputs(
    frame: &mut Frame<'_>,
    area: Rect,
    tab: &crate::model::relation::RelationTab,
    theme: Theme,
    state: &mut super::UiState,
) {
    if area.height == 0 {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let fields = [
        (
            crate::model::relation::RelationQueryInput::Where,
            "WHERE",
            tab.query.where_input.value(),
        ),
        (
            crate::model::relation::RelationQueryInput::OrderBy,
            "ORDER BY",
            tab.query.order_by_input.value(),
        ),
    ];
    let input_area = Rect::new(area.x, area.y, area.width, 1);
    for ((input, label, value), chunk) in fields.into_iter().zip(chunks.iter().copied()) {
        let active = tab.query.focus == Some(input);
        let text = format!("{label}  {value}");
        frame.render_widget(
            Paragraph::new(text).style(if active { theme.accent } else { theme.muted }),
            chunk,
        );
        state.hit_regions.push(HitRegion {
            area: chunk,
            target: HitTarget::RelationQueryInput(input),
        });
        if active {
            let cursor_index = match input {
                crate::model::relation::RelationQueryInput::Where => tab.query.where_input.cursor(),
                crate::model::relation::RelationQueryInput::OrderBy => {
                    tab.query.order_by_input.cursor()
                }
            };
            let cursor = UnicodeWidthStr::width(
                &value[..value
                    .char_indices()
                    .nth(cursor_index)
                    .map_or(value.len(), |(index, _)| index)],
            );
            frame.set_cursor_position(ratatui::layout::Position::new(
                chunk
                    .x
                    .saturating_add(UnicodeWidthStr::width(label) as u16)
                    .saturating_add(2)
                    .saturating_add(cursor as u16)
                    .min(chunk.right().saturating_sub(1)),
                input_area.y,
            ));
        }
    }
    if let Some(error) = &tab.query.error
        && area.height > 2
    {
        let error_area = Rect::new(area.x, area.y.saturating_add(2), area.width, 1);
        frame.render_widget(
            Paragraph::new(clean(error)).style(Style::new().fg(theme.warning)),
            error_area,
        );
    }
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

fn render_structure(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    _state: &mut super::UiState,
) {
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        return;
    };
    let (body, status) = match &tab.structure {
        RelationLoad::Ready(snapshot) => (structure_text(&snapshot.value, tab, app), None),
        RelationLoad::Loading { previous, .. } => (
            previous
                .as_ref()
                .map(|s| structure_text(&s.value, tab, app))
                .unwrap_or_default(),
            Some(("Refreshing", false, true)),
        ),
        RelationLoad::Failed { message, previous } => (
            previous
                .as_ref()
                .map(|s| structure_text(&s.value, tab, app))
                .unwrap_or_default(),
            Some((message.as_str(), true, false)),
        ),
        RelationLoad::Cancelled { previous } => (
            previous
                .as_ref()
                .map(|s| structure_text(&s.value, tab, app))
                .unwrap_or_default(),
            Some(("Cancelled", true, false)),
        ),
        RelationLoad::Empty => (String::new(), Some(("No structure data", false, false))),
    };
    if let Some((message, retry, cancel)) = status {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(area);
        render_status(frame, chunks[0], message, retry, cancel, theme, _state);
        frame.render_widget(
            Paragraph::new(body)
                .block(panel_block(
                    " RELATION STRUCTURE ",
                    app.focus == Focus::Results,
                    theme,
                ))
                .style(Style::new().fg(theme.text).bg(theme.surface))
                .wrap(Wrap { trim: true }),
            chunks[1],
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(body)
            .block(panel_block(
                " RELATION STRUCTURE ",
                app.focus == Focus::Results,
                theme,
            ))
            .style(Style::new().fg(theme.text).bg(theme.surface))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn structure_text(
    structure: &crate::db::catalog::RelationStructure,
    tab: &crate::model::relation::RelationTab,
    app: &App,
) -> String {
    let relation = &structure.relation;
    let mut lines = vec![format!(
        "{}  {}",
        clean(&relation.qualified_name.object),
        clean(&relation.native_kind)
    )];
    if let OptionalMetadata::Supported(Some(comment)) = &relation.comment {
        lines.push(format!("Comment: {}", clean(comment)));
    }
    let mut columns = structure
        .children
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Column(column) => Some((
                column.ordinal_position,
                format!(
                    "{}  {}  {}{}",
                    clean(&entry.qualified_name.object),
                    clean(&column.native_type),
                    if column.nullable { "NULL" } else { "NOT NULL" },
                    typed_metadata(column)
                ),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    columns.sort_by_key(|(ordinal, _)| *ordinal);
    lines.push("\nCOLUMNS".into());
    lines.extend(columns.into_iter().map(|(_, line)| line));
    lines.push("\nINDEXES / CONSTRAINTS".into());
    for entry in &structure.children.entries {
        match &entry.metadata {
            CatalogMetadata::Index(IndexMetadata { columns, unique }) => lines.push(format!(
                "{}INDEX {} ({})",
                if *unique { "UNIQUE " } else { "" },
                clean(&entry.qualified_name.object),
                columns
                    .iter()
                    .map(|s| clean(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            CatalogMetadata::Constraint(c) => lines.push(clean(&format_constraint(c))),
            _ => {}
        }
    }
    lines.push("\nTRIGGERS".into());
    for entry in &structure.children.entries {
        if entry.kind == crate::db::catalog::CatalogKind::Trigger {
            lines.push(format!(
                "{}  {}",
                clean(&entry.qualified_name.object),
                clean(&entry.native_kind)
            ));
        }
    }
    if let Some(sql) = &structure.ddl.sql {
        lines.push(format!(
            "\nDDL ({:?}):\n{}",
            structure.ddl.provenance,
            clean(sql)
        ));
    }
    if let Some(provenance) = tab.provenance(
        RelationView::Structure,
        app.connection.active_identity(),
        app.active_profile(),
    ) {
        lines.push(format!("\nSnapshot: {}", provenance_label(provenance)));
    }
    lines.join("\n")
}

fn format_constraint(c: &ConstraintMetadata) -> String {
    match c {
        ConstraintMetadata::PrimaryKey { columns } => format!(
            "PRIMARY KEY ({})",
            columns
                .iter()
                .map(|s| clean(s))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ConstraintMetadata::Unique { columns } => format!(
            "UNIQUE ({})",
            columns
                .iter()
                .map(|s| clean(s))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ConstraintMetadata::ForeignKey {
            columns,
            referenced_relation,
            referenced_columns,
        } => format!(
            "FOREIGN KEY ({}) -> {} ({})",
            columns
                .iter()
                .map(|s| clean(s))
                .collect::<Vec<_>>()
                .join(", "),
            clean(&referenced_relation.object),
            referenced_columns
                .iter()
                .map(|s| clean(s))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ConstraintMetadata::Check { expression } => format!("CHECK ({})", clean(expression)),
    }
}
fn clean(value: &str) -> String {
    sanitize_terminal_text(value).chars().take(240).collect()
}

fn typed_metadata(column: &crate::db::catalog::ColumnMetadata) -> String {
    let mut fields = Vec::new();
    if let OptionalMetadata::Supported(Some(value)) = &column.default_expression {
        fields.push(format!("DEFAULT {}", clean(value)));
    }
    if matches!(
        &column.generated_expression,
        OptionalMetadata::Supported(Some(_))
    ) && let OptionalMetadata::Supported(Some(value)) = &column.generated_expression
    {
        fields.push(format!("GENERATED {}", clean(value)));
    }
    if matches!(column.identity, OptionalMetadata::Supported(Some(true))) {
        fields.push("IDENTITY".into());
    }
    if matches!(
        column.auto_increment,
        OptionalMetadata::Supported(Some(true))
    ) {
        fields.push("AUTO_INCREMENT".into());
    }
    if let (
        OptionalMetadata::Supported(Some(precision)),
        OptionalMetadata::Supported(Some(scale)),
    ) = (&column.numeric_precision, &column.numeric_scale)
    {
        fields.push(format!("PRECISION {precision}/{scale}"));
    }
    if let OptionalMetadata::Supported(Some(value)) = &column.collation {
        fields.push(format!("COLLATION {}", clean(value)));
    }
    if !column.constraint_memberships.is_empty() {
        fields.push(format!(
            "MEMBERSHIPS {}",
            column.constraint_memberships.len()
        ));
    }
    if fields.is_empty() {
        String::new()
    } else {
        format!(
            "  [{}]",
            fields.join(" ").chars().take(180).collect::<String>()
        )
    }
}
fn provenance_label(value: RelationSnapshotProvenance) -> &'static str {
    match value {
        RelationSnapshotProvenance::Live => "LIVE",
        RelationSnapshotProvenance::OfflineSnapshot => "OFFLINE SNAPSHOT",
        RelationSnapshotProvenance::ProfileDeletedSnapshot => "PROFILE DELETED SNAPSHOT",
        RelationSnapshotProvenance::OutOfScopeSnapshot => "OUT OF SCOPE SNAPSHOT",
    }
}
