use super::{animation, loading};
use super::{panel_block, render_text_input, theme::Theme};
use crate::{
    app::App,
    model::{
        editor::EditorViewport,
        relation::{RelationLoad, RelationSnapshotProvenance, RelationView},
        tab::WorkspaceTab,
        workspace::Focus,
    },
    security::sanitize_terminal_text,
    ui::{HitRegion, HitTarget},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

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
    let data_label = " DATA ";
    let ddl_label = " DDL ";
    let data_width = data_label.width() as u16;
    let ddl_x = chunks[0].x.saturating_add(data_width).saturating_add(1);
    let ddl_width = ddl_label.width() as u16;
    let tabs = Line::from(vec![
        Span::styled(
            data_label,
            Style::new().fg(data_style).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            ddl_label,
            Style::new().fg(ddl_style).add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(tabs).style(theme.base()), chunks[0]);
    let active_x = if tab.view == RelationView::Data {
        chunks[0].x
    } else {
        ddl_x
    };
    let active_width = if tab.view == RelationView::Data {
        data_width
    } else {
        ddl_width
    };
    frame.render_widget(
        Paragraph::new("━".repeat(usize::from(active_width))).style(Style::new().fg(theme.accent)),
        Rect::new(active_x, chunks[0].y.saturating_add(1), active_width, 1),
    );
    state.hit_regions.push(HitRegion {
        area: Rect::new(chunks[0].x, chunks[0].y, data_width, 1),
        target: HitTarget::RelationView(RelationView::Data),
    });
    state.hit_regions.push(HitRegion {
        area: Rect::new(ddl_x, chunks[0].y, ddl_width, 1),
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
        let block = panel_block(" CELL EDITOR ", true, theme);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        render_text_input(frame, inner, "", &editor.input, theme.base(), state);
    }
}

#[cfg(test)]
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
        let block = panel_block(" RELATION DATA ", app.focus == Focus::Results, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let query_height = super::query_bar::height(&tab.query, inner.width, state.activity_icons);
        let body = if status.is_some() {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(query_height),
                    Constraint::Length(2),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(query_height),
                    Constraint::Length(0),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner)
        };
        let query_cursor = super::query_bar::render(
            frame,
            body[0],
            &tab.query,
            theme,
            state,
            state.activity_icons,
        );
        if let Some((message, retry, cancel)) = status {
            if cancel {
                render_loading_status(
                    frame,
                    body[1],
                    message,
                    theme,
                    state,
                    tab,
                    RelationView::Data,
                );
            } else {
                render_status(frame, body[1], message, retry, cancel, theme, state);
            }
        }
        render_relation_result_table(
            frame,
            body[2],
            tab.id,
            &result,
            tab.grid.clone(),
            &tab.grid.column_widths,
            theme,
            ratatui::widgets::Block::default().style(Style::new().bg(theme.surface)),
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
                "SQL: {sql}  {} rows  Snapshot: {provenance}",
                result.rows.len()
            ))
            .style(Style::new().fg(theme.muted).bg(theme.surface)),
            footer,
        );
        super::pagination::render(
            frame,
            body[3],
            tab.pagination,
            super::pagination::PaginationKind::Relation,
            theme,
            state,
        );
        if let (Some(completion), Some(cursor)) = (&tab.query.completion, query_cursor) {
            super::render_data_query_completion_popup(
                frame,
                completion,
                theme,
                state,
                super::CompletionAnchor {
                    viewport: area,
                    cursor,
                    replacement_start_x: None,
                },
            );
        }
    } else if let Some((message, retry, cancel)) = status {
        let block = panel_block(" RELATION DATA ", app.focus == Focus::Results, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(super::query_bar::height(
                    &tab.query,
                    inner.width,
                    state.activity_icons,
                )),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
        let query_cursor = super::query_bar::render(
            frame,
            body[0],
            &tab.query,
            theme,
            state,
            state.activity_icons,
        );
        if cancel {
            let identity = relation_loading_identity(tab, RelationView::Data);
            let elapsed = state.animations.elapsed(&identity).unwrap_or_default();
            if animation::show_skeleton(elapsed) {
                frame.render_widget(
                    loading::TableSkeleton {
                        mode: state.animation_mode(),
                        icons: state.activity_icons,
                        elapsed,
                        theme,
                        block: ratatui::widgets::Block::default()
                            .style(Style::new().bg(theme.surface)),
                    },
                    body[1],
                );
            } else {
                render_loading_status(
                    frame,
                    body[1],
                    message,
                    theme,
                    state,
                    tab,
                    RelationView::Data,
                );
            }
        } else {
            render_status(frame, body[1], message, retry, cancel, theme, state);
        }
        if let (Some(completion), Some(cursor)) = (&tab.query.completion, query_cursor) {
            super::render_data_query_completion_popup(
                frame,
                completion,
                theme,
                state,
                super::CompletionAnchor {
                    viewport: area,
                    cursor,
                    replacement_start_x: None,
                },
            );
        }
        super::pagination::render(
            frame,
            body[2],
            tab.pagination,
            super::pagination::PaginationKind::Relation,
            theme,
            state,
        );
    }
}

fn relation_loading_identity(
    tab: &crate::model::relation::RelationTab,
    view: RelationView,
) -> animation::LoadIdentity {
    let request = match view {
        RelationView::Data => match &tab.data {
            RelationLoad::Loading { request, .. } => request.clone(),
            _ => panic!("relation data loading identity requested while idle"),
        },
        RelationView::Ddl => match &tab.ddl {
            RelationLoad::Loading { request, .. } => request.clone(),
            _ => panic!("relation ddl loading identity requested while idle"),
        },
    };
    animation::LoadIdentity::Relation(request)
}

fn render_loading_status(
    frame: &mut Frame<'_>,
    area: Rect,
    message: &str,
    theme: Theme,
    state: &mut super::UiState,
    tab: &crate::model::relation::RelationTab,
    view: RelationView,
) {
    let identity = relation_loading_identity(tab, view);
    let elapsed = state.animations.elapsed(&identity).unwrap_or_default();
    let detail = if match view {
        RelationView::Data => matches!(
            &tab.data,
            RelationLoad::Loading {
                previous: Some(_),
                ..
            }
        ),
        RelationView::Ddl => matches!(
            &tab.ddl,
            RelationLoad::Loading {
                previous: Some(_),
                ..
            }
        ),
    } {
        Some("showing previous snapshot")
    } else {
        None
    };
    frame.render_widget(
        loading::ActivityIndicator {
            mode: state.animation_mode(),
            icons: state.activity_icons,
            elapsed,
            label: message,
            detail,
            cancellable: true,
            style: Style::new().fg(theme.action).bg(theme.surface_raised),
        },
        area,
    );
    state.hit_regions.push(HitRegion {
        area,
        target: HitTarget::RelationCancel,
    });
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
    let icons = state.activity_icons;
    super::data_grid::render(
        frame, area, tab_id, result, grid, overrides, theme, block, state, edit, icons,
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
    let status = match &tab.ddl {
        RelationLoad::Ready(_) => None,
        RelationLoad::Loading { .. } => Some(("Refreshing", false, true)),
        RelationLoad::Failed { message, .. } => Some((message.as_str(), true, false)),
        RelationLoad::Cancelled { .. } => Some(("Cancelled", true, false)),
        RelationLoad::Empty => Some(("No DDL available", false, false)),
    };
    let block = panel_block(" RELATION DDL ", app.focus == Focus::Results, theme);
    if let Some((message, retry, cancel)) = status {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(area);
        if cancel {
            render_loading_status(
                frame,
                chunks[0],
                message,
                theme,
                _state,
                tab,
                RelationView::Ddl,
            );
        } else {
            render_status(frame, chunks[0], message, retry, cancel, theme, _state);
        }
        render_ddl_editor(frame, chunks[1], app, theme, _state, block);
        return;
    }
    render_ddl_editor(frame, area, app, theme, _state, block);
}

fn render_ddl_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
    block: ratatui::widgets::Block<'_>,
) {
    let inner = block.inner(area);
    let viewport = EditorViewport {
        width: inner.width as usize,
        height: inner.height as usize,
    };
    state.editor_viewport = Some(viewport);
    let Ok(snapshot) = app.active_ddl_editor_snapshot(viewport) else {
        frame.render_widget(block, area);
        return;
    };
    frame.render_widget(block, area);
    for (row, line) in snapshot.lines.iter().take(viewport.height).enumerate() {
        let y = inner.y.saturating_add(row as u16);
        let spans = super::editor_line_spans(line, &snapshot, theme, true);
        let selected = snapshot
            .selection_cells
            .iter()
            .any(|(selected_line, _, _)| *selected_line == line.line);
        frame.render_widget(
            Paragraph::new(Line::from(spans))
                .style(Style::new().bg(if selected {
                    theme.selection
                } else {
                    theme.surface
                }))
                .scroll((0, snapshot.horizontal_offset.min(u16::MAX as usize) as u16)),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
    if app.focus == Focus::Results
        && app.overlay.is_none()
        && let Some((x, y)) = snapshot.cursor_screen_cell
    {
        frame.set_cursor_position(Position::new(
            inner.x.saturating_add(x),
            inner.y.saturating_add(y),
        ));
        state.cursor_style = Some(super::CursorStyle::Block);
    }
}

#[cfg(test)]
fn ddl_text(sql: &str) -> String {
    sanitize_terminal_text(sql)
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
    use super::cell_editor_value;
    use crate::model::{relation_edit::CellEditorState, text_input::TextInput};

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
}
