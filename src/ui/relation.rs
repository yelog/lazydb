use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

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
        super::query_bar::render(frame, body[0], &tab.query, theme, state);
        if let Some((message, retry, cancel)) = status {
            render_status(frame, body[1], message, retry, cancel, theme, state);
        }
        let block = panel_block(" RELATION DATA ", app.focus == Focus::Results, theme);
        render_relation_result_table(
            frame,
            body[2],
            &result,
            tab.grid.clone(),
            &tab.grid.column_widths,
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
        super::query_bar::render(frame, body[0], &tab.query, theme, state);
        render_status(frame, body[1], message, retry, cancel, theme, state);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_relation_result_table(
    frame: &mut Frame<'_>,
    area: Rect,
    result: &crate::db::query::ResultSet,
    grid: crate::model::tab::DataGridState,
    overrides: &[Option<u16>],
    theme: Theme,
    block: ratatui::widgets::Block<'_>,
    state: &mut super::UiState,
) {
    super::data_grid::render(frame, area, result, grid, overrides, theme, block, state);
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
