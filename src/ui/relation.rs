use super::{animation, loading};
use super::{panel_block, render_text_input, theme::Theme};
use crate::{
    app::App,
    model::{
        editor::{EditorRenderSnapshot, EditorViewport},
        relation::{RelationLoad, RelationSnapshotProvenance, RelationView},
        tab::WorkspaceTab,
        workspace::Focus,
    },
    security::sanitize_terminal_text,
    ui::{CursorSpec, CursorStyle, HitRegion, HitTarget},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

struct CellEditorColumnContext {
    name: String,
    type_label: String,
    default: String,
    comment: String,
}

fn cell_editor_column_context(
    app: &App,
    tab: &crate::model::relation::RelationTab,
    column_index: usize,
) -> CellEditorColumnContext {
    let result_column = match &tab.data {
        RelationLoad::Ready(snapshot) => snapshot.value.result.result_sets.last(),
        RelationLoad::Loading { previous, .. }
        | RelationLoad::Failed { previous, .. }
        | RelationLoad::Cancelled { previous } => previous
            .as_ref()
            .and_then(|snapshot| snapshot.value.result.result_sets.last()),
        RelationLoad::Empty => None,
    }
    .and_then(|result| result.columns.get(column_index));

    let fallback_name = format!("Column {}", column_index + 1);
    let name = result_column
        .map(|column| sanitize_terminal_text(&column.name))
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback_name);
    let fallback_type = result_column
        .map(|column| sanitize_terminal_text(&column.type_name))
        .filter(|type_name| !type_name.is_empty())
        .unwrap_or_else(|| "Unavailable".to_owned());

    let entry = app
        .explorer
        .completion_indexes
        .get(&tab.descriptor.key.profile_id)
        .and_then(|index| {
            index
                .relation_columns(&tab.descriptor.key.object_id)
                .find(|entry| entry.qualified_name.object == name)
        })
        .or_else(|| match &tab.ddl {
            RelationLoad::Ready(snapshot) => snapshot.value.children.entries.iter().find(|entry| {
                entry.kind == crate::db::catalog::CatalogKind::Column
                    && entry.qualified_name.object == name
            }),
            _ => None,
        });

    let (type_label, default, comment) = entry.map_or_else(
        || {
            (
                fallback_type.clone(),
                "Unavailable".to_owned(),
                "Unavailable".to_owned(),
            )
        },
        |entry| {
            let (type_label, default) = match &entry.metadata {
                crate::db::catalog::CatalogMetadata::Column(metadata) => {
                    let type_label = if metadata.native_type.trim().is_empty() {
                        fallback_type.clone()
                    } else {
                        format!(
                            "{} · {}",
                            sanitize_terminal_text(&metadata.native_type),
                            if metadata.nullable {
                                "NULLABLE"
                            } else {
                                "NOT NULL"
                            }
                        )
                    };
                    let default = match &metadata.default_expression {
                        crate::db::catalog::OptionalMetadata::Unsupported => {
                            "Not supported".to_owned()
                        }
                        crate::db::catalog::OptionalMetadata::Supported(value) => value
                            .as_deref()
                            .map(sanitize_terminal_text)
                            .unwrap_or_else(|| "None".to_owned()),
                    };
                    (type_label, default)
                }
                _ => (fallback_type.clone(), "Unavailable".to_owned()),
            };
            let comment = match &entry.comment {
                crate::db::catalog::OptionalMetadata::Unsupported => "Not supported".to_owned(),
                crate::db::catalog::OptionalMetadata::Supported(value) => value
                    .as_deref()
                    .map(sanitize_terminal_text)
                    .unwrap_or_else(|| "None".to_owned()),
            };
            (type_label, default, comment)
        },
    );

    CellEditorColumnContext {
        name,
        type_label,
        default,
        comment,
    }
}

fn render_cell_metadata(
    frame: &mut Frame<'_>,
    area: Rect,
    column: &CellEditorColumnContext,
    theme: Theme,
) {
    let style = Style::new().fg(theme.text).add_modifier(Modifier::BOLD);
    frame.render_widget(
        Paragraph::new(column.type_label.as_str()).style(style),
        area,
    );
}

fn render_cell_metadata_line(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    value: &str,
    theme: Theme,
) {
    let value = truncate_cell_text(value, usize::from(area.width).saturating_sub(8));
    let line = Line::from(vec![
        Span::styled(format!("{label:<8}"), Style::new().fg(theme.muted)),
        Span::styled(value, Style::new().fg(theme.text)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn truncate_cell_text(value: &str, width: usize) -> String {
    let value = value.replace(['\n', '\r', '\t'], " ");
    if UnicodeWidthStr::width(value.as_str()) <= width {
        return value;
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut output = String::new();
    let mut used = 0;
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width - 1 {
            break;
        }
        output.push(character);
        used += character_width;
    }
    output.push('…');
    output
}

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
    let regions = super::render_tab_selectors(
        frame,
        chunks[0],
        &["DATA", "DDL"],
        usize::from(tab.view == RelationView::Ddl),
        theme,
    );
    for (region, view) in regions
        .into_iter()
        .zip([RelationView::Data, RelationView::Ddl])
    {
        state.hit_regions.push(HitRegion {
            area: region,
            target: HitTarget::RelationView(view),
        });
    }
    match tab.view {
        RelationView::Data => render_data(frame, chunks[1], app, theme, state),
        RelationView::Ddl => render_ddl(frame, chunks[1], app, theme, state),
    }
    if let Some(crate::model::relation_edit::RelationEditSession {
        mode: crate::model::relation_edit::RelationGridMode::EditCell(editor),
        ..
    }) = &tab.edit
    {
        let popup_width = area.width.min(72);
        let json = editor.input.json_buffer();
        let column = cell_editor_column_context(app, tab, editor.column);
        let popup_height = area.height.min(if json.is_some() { 22 } else { 16 });
        let popup = Rect::new(
            area.x
                .saturating_add(area.width.saturating_sub(popup_width) / 2),
            area.y
                .saturating_add(area.height.saturating_sub(popup_height) / 2),
            popup_width,
            popup_height,
        );
        frame.render_widget(ratatui::widgets::Clear, popup);
        let title = format!(" EDIT · {} ", column.name);
        let block = panel_block(&title, true, theme);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        state.cursor = None;
        let is_boolean = matches!(
            &editor.input,
            crate::model::cell_editor::CellEditorBuffer {
                content: crate::model::cell_editor::CellEditorContent::Typed {
                    kind: crate::model::cell_editor::CellEditorKind::Boolean,
                    draft: crate::model::cell_editor::TypedDraft::Boolean(_),
                    ..
                },
                ..
            }
        );
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
        render_cell_metadata(frame, sections[0], &column, theme);
        render_cell_metadata_line(frame, sections[1], "Default", &column.default, theme);
        render_cell_metadata_line(frame, sections[2], "Comment", &column.comment, theme);
        let presence_label = match editor.input.presence() {
            crate::model::cell_editor::CellEditorPresence::Unprovided => "DEFAULT (unprovided)",
            crate::model::cell_editor::CellEditorPresence::Null => "NULL (explicit)",
            crate::model::cell_editor::CellEditorPresence::Value => "",
        };
        if !presence_label.is_empty() {
            frame.render_widget(
                Paragraph::new(presence_label).style(Style::new().fg(theme.accent)),
                sections[3],
            );
        }
        let content_area = Rect::new(
            sections[4].x,
            sections[4].y,
            sections[4].width,
            sections[4].height,
        );
        if let Some(json) = json {
            let row_id = tab
                .edit
                .as_ref()
                .and_then(|edit| edit.rows.get(editor.row))
                .map(|row| row.id);
            render_json_editor(
                frame,
                content_area,
                json,
                editor.error.as_deref(),
                Some(JSON_EDITOR_FOOTER),
                theme,
                state,
            );
            if let Some(row_id) = row_id {
                register_json_selection_target(
                    state,
                    tab.id,
                    row_id,
                    editor.column,
                    content_area,
                    json,
                );
            }
        } else if is_boolean {
            render_boolean_editor(frame, content_area, editor, theme);
            frame.render_widget(
                Paragraph::new(
                    "Left/Right  Space  t/f  Alt-N NULL  Alt-D DEFAULT  Alt-V use value",
                )
                .style(Style::new().fg(theme.muted)),
                sections[6],
            );
        } else if let crate::model::cell_editor::CellEditorBuffer {
            content:
                crate::model::cell_editor::CellEditorContent::Typed {
                    draft: crate::model::cell_editor::TypedDraft::Temporal(draft),
                    ..
                },
            ..
        } = &editor.input
        {
            render_text_input(frame, content_area, "", draft.input(), theme.base(), state);
            if let Some(row_id) = tab
                .edit
                .as_ref()
                .and_then(|edit| edit.rows.get(editor.row).map(|row| row.id))
            {
                super::register_input_selection_target(
                    state,
                    super::text_selection::InputSelectionTarget::RelationTemporal {
                        tab_id: tab.id,
                        row_id,
                        column: editor.column,
                    },
                    content_area,
                    "",
                    draft.input(),
                    super::text_input_horizontal_offset(content_area, "", draft.input()),
                );
            }
            if let Some(label) = draft.calendar_label() {
                frame.render_widget(
                    Paragraph::new(label).style(Style::new().fg(theme.muted)),
                    sections[5],
                );
            }
            frame.render_widget(
                Paragraph::new(
                    "Left/Right field  [/] month  Alt-N NULL  Alt-D DEFAULT  Alt-V use value",
                )
                .style(Style::new().fg(theme.muted)),
                sections[6],
            );
        } else if let Some(input) = editor.input.input() {
            render_text_input(frame, content_area, "", input, theme.base(), state);
            if let Some(row_id) = tab
                .edit
                .as_ref()
                .and_then(|edit| edit.rows.get(editor.row).map(|row| row.id))
            {
                super::register_input_selection_target(
                    state,
                    super::text_selection::InputSelectionTarget::RelationText {
                        tab_id: tab.id,
                        row_id,
                        column: editor.column,
                    },
                    content_area,
                    "",
                    input,
                    super::text_input_horizontal_offset(content_area, "", input),
                );
            }
            frame.render_widget(
                Paragraph::new(
                    "Enter apply  Esc cancel  Alt-N NULL  Alt-D DEFAULT  Alt-V use value",
                )
                .style(Style::new().fg(theme.muted)),
                sections[6],
            );
        } else if editor.input.is_unprovided() {
            frame.render_widget(
                Paragraph::new("DEFAULT (unprovided)").style(Style::new().fg(theme.muted)),
                sections[4],
            );
        } else if editor.input.is_null() {
            frame.render_widget(
                Paragraph::new("NULL (explicit)").style(Style::new().fg(theme.muted)),
                sections[4],
            );
        }
        if json.is_none()
            && let Some(error) = &editor.error
        {
            frame.render_widget(
                Paragraph::new(Line::from(error.as_str()).style(Style::new().fg(theme.error))),
                sections[7],
            );
        }
    }
    if let Some(crate::model::relation_edit::RelationEditSession {
        mode: crate::model::relation_edit::RelationGridMode::EditDocument(editor),
        ..
    }) = &tab.edit
    {
        let popup_width = area.width.min(88);
        let popup_height = area.height.min(24);
        let popup = Rect::new(
            area.x
                .saturating_add(area.width.saturating_sub(popup_width) / 2),
            area.y
                .saturating_add(area.height.saturating_sub(popup_height) / 2),
            popup_width,
            popup_height,
        );
        frame.render_widget(ratatui::widgets::Clear, popup);
        let block = panel_block(" EDIT DOCUMENT ", true, theme);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        state.cursor = None;
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(Line::from(editor.title()).style(Style::new().fg(theme.muted))),
            sections[0],
        );
        let hint = match editor.view {
            crate::model::relation_edit::DocumentEditorView::Ejson => {
                render_json_editor(
                    frame,
                    sections[1],
                    &editor.input,
                    editor.error.as_deref(),
                    Some(JSON_EDITOR_FOOTER),
                    theme,
                    state,
                );
                "Tab field view"
            }
            crate::model::relation_edit::DocumentEditorView::Fields => {
                render_document_fields(frame, sections[1], editor, theme);
                if editor.field_edit.is_none() {
                    "j/k field  Enter edit  Ctrl-S apply document  Tab EJSON  Esc close"
                } else if editor.field_edit_is_multiline() {
                    "Ctrl-S apply field  Esc back to fields"
                } else {
                    "Enter apply field  Esc back to fields"
                }
            }
        };
        frame.render_widget(
            Paragraph::new(hint).style(Style::new().fg(theme.muted)),
            sections[2],
        );
        // Every field is edited in its own surface over this modal: a scalar on
        // one line, and a container as indented EJSON, which never fits here.
        if let Some(field) = &editor.field_edit {
            render_document_field_editor(frame, popup, editor, field, theme, state);
        }
    }
}

/// The footer of a field modal that edits a scalar on one line.
const FIELD_ONE_LINE_FOOTER: &str = "Enter apply field  Esc back to fields";

/// The footer of a field modal that edits an array or an embedded document.
const FIELD_MODAL_FOOTER: &str =
    "Enter newline  Ctrl-S apply field  Ctrl-F format  Esc back to fields";

/// Renders the editor of the open field.
///
/// The popup is sized by the value it edits — its widest line and how many
/// lines it spans — so it stays smaller than the document modal it opens over.
/// A scalar edits on one line; a container edits indented Canonical Extended
/// JSON, scrolled with the cursor by the shared JSON editor. The refusal of the
/// last apply renders beside the value, so a broken field is corrected in place.
fn render_document_field_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &crate::model::relation_edit::DocumentEditorState,
    field: &crate::model::relation_edit::DocumentFieldEdit,
    theme: Theme,
    state: &mut super::UiState,
) {
    let error = editor.field_error.as_deref();
    let footer = if field.input.is_multiline() {
        FIELD_MODAL_FOOTER
    } else {
        FIELD_ONE_LINE_FOOTER
    };
    let popup = field_popup_area(area, field, error.is_some());
    state.cursor = None;
    if popup.width < 3 || popup.height < 3 {
        return;
    }
    frame.render_widget(ratatui::widgets::Clear, popup);
    let title = format!(
        " EDIT FIELD {} · {} ",
        sanitize_terminal_text(&field.name),
        field.type_name
    );
    let block = panel_block(&title, true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    // Reserve context and margins when they fit; editing takes priority in a
    // short terminal. Every section stays inside the popup's actual viewport.
    let show_context = inner.height >= 4;
    let show_margins = inner.height >= 5 + u16::from(error.is_some());
    let show_error = error.is_some() && inner.height >= 2;
    let show_footer = inner.height >= 3;
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(u16::from(show_context)),
            Constraint::Length(u16::from(show_margins)),
            Constraint::Min(1),
            Constraint::Length(u16::from(show_error)),
            Constraint::Length(u16::from(show_margins)),
            Constraint::Length(u16::from(show_footer)),
        ])
        .split(inner);
    // The document this field belongs to stays named, so the modal keeps the
    // same bounded target the save uses.
    frame.render_widget(
        Paragraph::new(Line::from(editor.title()).style(Style::new().fg(theme.muted))),
        sections[0],
    );
    let value_area = sections[2];
    match &field.input {
        crate::model::relation_edit::DocumentFieldInput::Line(input) => {
            render_text_input(
                frame,
                Rect::new(value_area.x, value_area.y, value_area.width, 1),
                "",
                input,
                Style::new().fg(theme.text),
                state,
            );
        }
        crate::model::relation_edit::DocumentFieldInput::Block(buffer) => {
            render_json_editor(frame, value_area, buffer, None, None, theme, state);
        }
    }
    if let Some(error) = error.filter(|_| show_error) {
        frame.render_widget(
            Paragraph::new(Line::from(error).style(Style::new().fg(theme.error))),
            sections[3],
        );
    }
    if sections[5].height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(footer).style(Style::new().fg(theme.muted))),
            Rect::new(sections[5].x, sections[5].y, sections[5].width, 1),
        );
    }
}

/// The rows the field modal spends around the value: two borders, the document
/// header, the two margins and the footer.
const FIELD_MODAL_CHROME_ROWS: u16 = 6;

/// Where the field modal draws: centered over the document modal and sized to
/// the value, preserving a surrounding margin when there is room for an
/// editable row, and never wider than 72 columns or taller than 20 rows.
fn field_popup_area(
    area: Rect,
    field: &crate::model::relation_edit::DocumentFieldEdit,
    error: bool,
) -> Rect {
    const MIN_WIDTH: u16 = 36;
    const MAX_WIDTH: u16 = 72;
    const MAX_HEIGHT: u16 = 20;
    // The popup is wide enough for the widest of its own parts: the value with
    // the padding on both sides, the title on the border, and the footer, whose
    // keys are useless clipped.
    let footer = if field.input.is_multiline() {
        FIELD_MODAL_FOOTER
    } else {
        FIELD_ONE_LINE_FOOTER
    };
    let title = format!(
        " EDIT FIELD {} · {} ",
        sanitize_terminal_text(&field.name),
        field.type_name
    );
    let value_width = field.input.longest_line_columns().saturating_add(6);
    let wanted_width = value_width
        .max(title.width().saturating_add(2))
        .max(footer.width().saturating_add(2));
    let wanted_height = field
        .input
        .modal_value_rows(error)
        .saturating_add(usize::from(FIELD_MODAL_CHROME_ROWS));
    // The design range first, then the room the terminal actually leaves, so a
    // narrow area shrinks the popup instead of overflowing it.
    let available_width = if area.width >= 5 {
        area.width - 2
    } else {
        area.width
    };
    let available_height = if area.height >= 5 {
        area.height - 2
    } else {
        area.height
    };
    let width = wanted_width
        .clamp(usize::from(MIN_WIDTH), usize::from(MAX_WIDTH))
        .min(usize::from(available_width)) as u16;
    let height = wanted_height
        .clamp(7, usize::from(MAX_HEIGHT))
        .min(usize::from(available_height)) as u16;
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

/// Renders the field-by-field view of the document modal.
///
/// Every row is one top-level field with its observed BSON type and its
/// compact value. Every field is edited in its own modal, so the list is only
/// navigation; a refusal or the document-level error renders below it.
fn render_document_fields(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &crate::model::relation_edit::DocumentEditorState,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let fields = match editor.fields() {
        Ok(fields) => fields,
        Err(message) => {
            frame.render_widget(
                Paragraph::new(message).style(Style::new().fg(theme.error)),
                area,
            );
            return;
        }
    };
    let error = editor.field_error.as_deref().or(editor.error.as_deref());
    let error_height = u16::from(error.is_some() && area.height >= 3);
    let list_height = usize::from(area.height.saturating_sub(error_height));
    let selected = editor.selected_field_index(fields.len());
    let top = selected.saturating_sub(list_height.saturating_sub(1));
    let rows = fields
        .iter()
        .enumerate()
        .skip(top)
        .take(list_height)
        .map(|(index, field)| {
            let style = if index == selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text)
            };
            Line::from(vec![
                ratatui::text::Span::styled(format!(" {:<16}", field.name), style),
                ratatui::text::Span::styled(
                    format!(" {:<10}", field.type_name),
                    if index == selected {
                        style
                    } else {
                        Style::new().fg(theme.muted)
                    },
                ),
                ratatui::text::Span::styled(format!(" {}", field.list_value()), style),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(rows),
        Rect::new(area.x, area.y, area.width, list_height as u16),
    );
    let next_row = area.y.saturating_add(list_height as u16);
    if let Some(error) = error
        && next_row < area.bottom()
    {
        frame.render_widget(
            Paragraph::new(Line::from(error).style(Style::new().fg(theme.error))),
            Rect::new(area.x, next_row, area.width, 1),
        );
    }
}

pub(crate) fn ddl_editor_viewport(area: Rect, app: &App) -> Option<(uuid::Uuid, EditorViewport)> {
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        return None;
    };
    if tab.view != RelationView::Ddl {
        return None;
    }
    let status = !matches!(tab.ddl, RelationLoad::Ready(_));
    let layout = relation_ddl_layout(area, status);
    let inner = panel_block(
        " RELATION DDL ",
        app.focus == Focus::Results,
        Theme::default(),
    )
    .inner(layout[1]);
    Some((
        tab.ddl_editor_id,
        EditorViewport {
            width: inner.width as usize,
            height: inner.height as usize,
        },
    ))
}

fn register_json_selection_target(
    state: &mut super::UiState,
    tab_id: uuid::Uuid,
    row_id: crate::model::relation_edit::EditableRowId,
    column: usize,
    area: Rect,
    json: &crate::model::cell_editor::JsonBuffer,
) {
    let body_height = usize::from(area.height).saturating_sub(2).max(1);
    let lines = json.value().split('\n').collect::<Vec<_>>();
    let cursor_line = json.line().min(lines.len().saturating_sub(1));
    let cursor_cells = lines
        .get(cursor_line)
        .map(|line| {
            crate::security::project_editor_line(line).source_to_display_cells[json.column()]
        })
        .unwrap_or(0);
    let top = cursor_line.saturating_sub(body_height.saturating_sub(1));
    let left = cursor_cells.saturating_sub(usize::from(area.width).saturating_sub(1));
    let mut source_start = 0;
    for (line_index, line) in lines.iter().enumerate() {
        if line_index >= top && line_index < top + body_height {
            let projection = crate::security::project_editor_line(line);
            state.input_selection_targets.push((
                super::text_selection::InputSelectionTarget::RelationJson {
                    tab_id,
                    row_id,
                    column,
                },
                super::text_selection::InputHitMap {
                    area: Rect::new(area.x, area.y + (line_index - top) as u16, area.width, 1),
                    source_to_display_cells: projection.source_to_display_cells,
                    horizontal_offset: left,
                    prefix_width: 0,
                    source_start,
                },
            ));
        }
        source_start += line.chars().count() + 1;
    }
}

fn json_line_spans(
    source: &str,
    left: usize,
    width: usize,
    source_start: usize,
    selection: Option<(usize, usize)>,
) -> Vec<ratatui::text::Span<'static>> {
    let projection = crate::security::project_editor_line(source);
    let tokens = crate::model::cell_editor::tokenize_json(source);
    let mut spans = Vec::new();
    let mut current_kind = (crate::model::cell_editor::JsonTokenKind::Whitespace, false);
    let mut display_start = 0;
    let mut display_end = 0;
    for (index, (byte_start, character)) in source.char_indices().enumerate() {
        let byte_end = byte_start + character.len_utf8();
        let kind = tokens
            .iter()
            .find(|token| byte_start >= token.start && byte_end <= token.end)
            .map(|token| token.kind)
            .unwrap_or(crate::model::cell_editor::JsonTokenKind::Whitespace);
        let selected = selection.is_some_and(|(start, end)| {
            source_start + index < end && source_start + index + 1 > start
        });
        let kind = (kind, selected);
        let source_start = projection.source_to_display_cells[index];
        let source_end = projection.source_to_display_cells[index + 1];
        if index > 0 && kind != current_kind {
            push_json_span(
                &mut spans,
                &projection.text,
                display_start,
                display_end,
                current_kind.0,
                left,
                width,
                current_kind.1,
            );
            display_start = source_start;
        }
        current_kind = kind;
        display_end = source_end;
    }
    if !source.is_empty() {
        push_json_span(
            &mut spans,
            &projection.text,
            display_start,
            display_end,
            current_kind.0,
            left,
            width,
            current_kind.1,
        );
    }
    spans
}

#[allow(clippy::too_many_arguments)]
fn push_json_span(
    spans: &mut Vec<ratatui::text::Span<'static>>,
    projected: &str,
    display_start: usize,
    display_end: usize,
    kind: crate::model::cell_editor::JsonTokenKind,
    left: usize,
    width: usize,
    selected: bool,
) {
    let start = display_start.max(left);
    let end = display_end.min(left.saturating_add(width));
    if start >= end {
        return;
    }
    let text = display_slice(projected, start, end);
    if !text.is_empty() {
        spans.push(ratatui::text::Span::styled(
            text,
            Style::new().fg(json_token_color(kind)).bg(if selected {
                ratatui::style::Color::Blue
            } else {
                ratatui::style::Color::Reset
            }),
        ));
    }
}

fn display_slice(value: &str, start: usize, end: usize) -> String {
    let mut cells = 0;
    let mut output = String::new();
    for character in value.chars() {
        let width = character.width().unwrap_or(0);
        let begin = cells;
        cells += width;
        let visible_start = begin.max(start);
        let visible_end = cells.min(end);
        if visible_start >= visible_end {
            continue;
        }
        if begin >= start && cells <= end {
            output.push(character);
        } else {
            output.extend(std::iter::repeat_n(' ', visible_end - visible_start));
        }
    }
    output
}

fn json_token_color(kind: crate::model::cell_editor::JsonTokenKind) -> ratatui::style::Color {
    match kind {
        crate::model::cell_editor::JsonTokenKind::String => ratatui::style::Color::Green,
        crate::model::cell_editor::JsonTokenKind::Number => ratatui::style::Color::Yellow,
        crate::model::cell_editor::JsonTokenKind::Boolean
        | crate::model::cell_editor::JsonTokenKind::Null => ratatui::style::Color::Magenta,
        crate::model::cell_editor::JsonTokenKind::Punctuation => ratatui::style::Color::Cyan,
        _ => ratatui::style::Color::Reset,
    }
}

/// The footer of a JSON editor that saves and cancels the whole value it shows.
const JSON_EDITOR_FOOTER: &str = "Enter newline  Ctrl-S apply  Ctrl-F format  Esc cancel";

fn render_json_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    json: &crate::model::cell_editor::JsonBuffer,
    error: Option<&str>,
    footer: Option<&str>,
    theme: Theme,
    state: &mut super::UiState,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let show_footer = footer.is_some() && (error.is_none() || area.height >= 3);
    let error_height = usize::from(error.is_some() && area.height >= 3);
    let footer_height = usize::from(show_footer && area.height >= 2);
    let body_height = usize::from(area.height)
        .saturating_sub(error_height)
        .saturating_sub(footer_height);
    if body_height == 0 {
        frame.render_widget(
            Paragraph::new(error.or(footer).unwrap_or_default()).style(Style::new().fg(
                if error.is_some() {
                    theme.error
                } else {
                    theme.muted
                },
            )),
            area,
        );
        return;
    }

    let lines = json.value().split('\n').collect::<Vec<_>>();
    let cursor_line = json.line().min(lines.len().saturating_sub(1));
    let cursor_column = json.column();
    let cursor_cells = lines
        .get(cursor_line)
        .map(|line| {
            let projection = crate::security::project_editor_line(line);
            projection
                .source_to_display_cells
                .get(cursor_column)
                .copied()
                .unwrap_or_else(|| projection.text.width())
        })
        .unwrap_or_default();
    let body_width = usize::from(area.width);
    let top = cursor_line.saturating_sub(body_height.saturating_sub(1));
    let left = cursor_cells.saturating_sub(body_width.saturating_sub(1));
    let visible_lines = lines
        .iter()
        .skip(top)
        .take(body_height)
        .scan(
            lines
                .iter()
                .take(top)
                .map(|line| line.chars().count() + 1)
                .sum::<usize>(),
            |source_start, source| {
                let line_start = *source_start;
                *source_start += source.chars().count() + 1;
                Some(Line::from(json_line_spans(
                    source,
                    left,
                    body_width,
                    line_start,
                    json.selection(),
                )))
            },
        )
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible_lines),
        Rect::new(area.x, area.y, area.width, body_height as u16),
    );

    let cursor_x = area
        .x
        .saturating_add(cursor_cells.saturating_sub(left) as u16)
        .min(area.right().saturating_sub(1));
    let cursor_y = area.y.saturating_add((cursor_line - top) as u16);
    state.cursor = Some(CursorSpec {
        position: Position::new(cursor_x, cursor_y),
        style: CursorStyle::Bar,
    });

    let mut next_row = area.y.saturating_add(body_height as u16);
    if let Some(error) = error
        && next_row < area.bottom()
    {
        frame.render_widget(
            Paragraph::new(Line::from(error).style(Style::new().fg(theme.error))),
            Rect::new(area.x, next_row, area.width, 1),
        );
        next_row = next_row.saturating_add(1);
    }
    if let Some(footer) = footer
        && show_footer
        && next_row < area.bottom()
    {
        frame.render_widget(
            Paragraph::new(footer).style(Style::new().fg(theme.muted)),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
}

fn render_boolean_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &crate::model::relation_edit::CellEditorState,
    theme: Theme,
) {
    let selected = editor.input.boolean_selection();
    let true_style = if selected == Some(true) {
        Style::new().fg(theme.background).bg(theme.accent)
    } else {
        Style::new().fg(theme.muted)
    };
    let false_style = if selected == Some(false) {
        Style::new().fg(theme.background).bg(theme.accent)
    } else {
        Style::new().fg(theme.muted)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            ratatui::text::Span::styled(" true ", true_style),
            ratatui::text::Span::raw("  "),
            ratatui::text::Span::styled(" false ", false_style),
        ])),
        area,
    );
}

#[cfg(test)]
fn cell_editor_value(editor: &crate::model::relation_edit::CellEditorState) -> String {
    editor.input.value().unwrap_or_default().to_owned()
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
        RelationLoad::Loading { previous, .. } => (
            previous.as_ref(),
            Some((
                if previous.is_some() {
                    "Refreshing relation data"
                } else {
                    "Loading relation data"
                },
                false,
                true,
            )),
        ),
        RelationLoad::Failed { message, previous } => {
            (previous.as_ref(), Some((message.as_str(), true, false)))
        }
        RelationLoad::Cancelled { previous } => {
            (previous.as_ref(), Some(("Cancelled", true, false)))
        }
        RelationLoad::Empty => (None, Some(("Relation not loaded", false, false))),
    };
    let status = if matches!(tab.data, RelationLoad::Empty)
        && !matches!(
            tab.preparation,
            crate::model::relation::RelationPreparation::Idle
        ) {
        // Preparation has no relation request yet, so it cannot use the
        // request-backed loading animation or query cancellation controls.
        Some((
            match &tab.preparation {
                crate::model::relation::RelationPreparation::WaitingForSession { .. } => {
                    "Connecting to relation target"
                }
                crate::model::relation::RelationPreparation::ResolvingIdentity => {
                    "Resolving relation identity"
                }
                crate::model::relation::RelationPreparation::Failed { message } => message,
                crate::model::relation::RelationPreparation::Idle => "Relation not loaded",
            },
            matches!(
                tab.preparation,
                crate::model::relation::RelationPreparation::Failed { .. }
            ),
            false,
        ))
    } else {
        status
    };
    if let Some(snapshot) = snapshot {
        let empty_result = crate::db::query::ResultSet::default();
        let result = snapshot
            .value
            .result
            .result_sets
            .last()
            .unwrap_or(&empty_result);
        let row_count = tab
            .edit
            .as_ref()
            .map_or(result.rows.len(), |edit| edit.rows.len());
        let block = panel_block(" RELATION DATA ", app.focus == Focus::Results, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let query_height = super::query_bar::height(&tab.query, inner.width, state.activity_icons);
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(query_height),
                Constraint::Length(u16::from(status.is_some())),
                Constraint::Min(1),
                Constraint::Length(1),
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
            app.sql_dialect(),
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
            result,
            tab.generation,
            tab.grid.clone(),
            &tab.grid.column_widths,
            theme,
            ratatui::widgets::Block::default().style(Style::new().bg(theme.surface)),
            state,
            tab.edit.as_ref(),
            tab.query
                .submitted
                .order_by_clause
                .as_deref()
                .unwrap_or_default(),
            app.sql_dialect(),
        );
        let sql = sanitize_terminal_text(&snapshot.value.sql);
        let footer = body[3];
        let provenance = tab
            .provenance(
                RelationView::Data,
                app.connection.active_identity(),
                app.active_profile(),
            )
            .map(provenance_label)
            .unwrap_or("UNKNOWN");
        // A MongoDB collection has no SQL: it shows the find it executed and the
        // field under the cursor, the way a SQL snapshot shows its statement.
        let mongo = app.profiles.iter().any(|profile| {
            profile.id == tab.descriptor.key.profile_id
                && profile.kind == crate::profile::DatabaseKind::MongoDb
        });
        let statement_label = if mongo { "Find" } else { "SQL" };
        let active_field = mongo
            .then(|| {
                snapshot
                    .value
                    .result
                    .result_sets
                    .last()
                    .and_then(|set| set.columns.get(tab.grid.selected_column))
                    .map(|column| column.name.clone())
            })
            .flatten();
        let field_status = active_field
            .as_deref()
            .map(|field| format!("  field: {field}"))
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(format!(
                "{statement_label}: {sql}  {row_count} rows{field_status}  Snapshot: {provenance}",
            ))
            .style(Style::new().fg(theme.muted).bg(theme.surface)),
            footer,
        );
        state.hit_regions.push(HitRegion {
            area: footer,
            target: HitTarget::OpenTextDetail(super::readonly_detail_request(
                "Relation snapshot",
                format!(
                    "{statement_label}: {sql}\nRows: {row_count}{field_status}\nSnapshot: {provenance}",
                ),
            )),
        });
        super::pagination::render(
            frame,
            body[4],
            tab.pagination,
            super::pagination::PaginationKind::Relation,
            theme,
            state,
            matches!(tab.data, RelationLoad::Ready(_)),
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
            app.sql_dialect(),
        );
        if cancel {
            let identity = relation_loading_identity(tab, RelationView::Data);
            let elapsed = state.animations.elapsed(&identity).unwrap_or_default();
            frame.render_widget(
                loading::LoadingViewport {
                    mode: state.animation_mode(),
                    icons: state.activity_icons,
                    elapsed,
                    label: message,
                    helper: animation::show_loading_helper(elapsed)
                        .then_some("Waiting for the first result set..."),
                    cancellable: true,
                    theme,
                    block: ratatui::widgets::Block::default().style(Style::new().bg(theme.surface)),
                },
                body[1],
            );
            state.hit_regions.push(HitRegion {
                area: body[1],
                target: HitTarget::RelationCancel,
            });
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
            false,
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
    animation::LoadIdentity::Relation(Box::new(request))
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
    data_revision: u64,
    grid: crate::model::tab::DataGridState,
    overrides: &[Option<u16>],
    theme: Theme,
    block: ratatui::widgets::Block<'_>,
    state: &mut super::UiState,
    edit: Option<&crate::model::relation_edit::RelationEditSession>,
    order_by_clause: &str,
    dialect: crate::sql::SqlDialect,
) {
    let icons = state.activity_icons;
    let column_names = result
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let sort_projection =
        crate::sql::relation_column_sort_projection(order_by_clause, &column_names, dialect)
            .unwrap_or_else(|_| vec![None; column_names.len()]);
    super::data_grid::render(
        frame,
        area,
        tab_id,
        result,
        data_revision,
        grid,
        overrides,
        theme,
        block,
        state,
        edit,
        icons,
        Some(sort_projection.as_slice()),
        true,
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
    let detail_message = sanitize_terminal_text(message);
    let message = clean(message);
    let label = if retry {
        "r  retry"
    } else if cancel {
        "Ctrl-C  cancel"
    } else {
        ""
    };
    let text = if label.is_empty() {
        message.clone()
    } else {
        format!("{}  [{}]", message, label)
    };
    let retry_width = if retry { 10 } else { 0 };
    let cancel_width = if cancel { 14 } else { 0 };
    let message_area = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(retry_width + cancel_width),
        area.height,
    );
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(theme.warning).bg(theme.surface_raised)),
        message_area,
    );
    if !message_area.is_empty() {
        state.hit_regions.push(HitRegion {
            area: message_area,
            target: HitTarget::OpenTextDetail(crate::model::text_detail::TextDetailRequest::new(
                "Relation error",
                uuid::Uuid::nil(),
                0,
                detail_message.clone(),
                detail_message,
                None,
            )),
        });
    }
    if retry {
        state.hit_regions.push(HitRegion {
            area: Rect::new(message_area.right(), area.y, retry_width, area.height),
            target: HitTarget::RelationRetry,
        });
    }
    if cancel {
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                message_area.right().saturating_add(retry_width),
                area.y,
                cancel_width,
                area.height,
            ),
            target: HitTarget::RelationCancel,
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
    let mut block = panel_block(" RELATION DDL ", app.focus == Focus::Results, theme);
    let chunks = relation_ddl_layout(area, status.is_some());
    let viewport = {
        let inner = block.inner(chunks[1]);
        EditorViewport {
            width: inner.width as usize,
            height: inner.height as usize,
        }
    };
    let editor_snapshot = app.active_ddl_editor_snapshot(viewport).ok();
    let snapshot = match &tab.ddl {
        RelationLoad::Ready(snapshot)
        | RelationLoad::Loading {
            previous: Some(snapshot),
            ..
        }
        | RelationLoad::Failed {
            previous: Some(snapshot),
            ..
        }
        | RelationLoad::Cancelled {
            previous: Some(snapshot),
        } => Some(snapshot),
        _ => None,
    };
    if let Some(snapshot) = snapshot {
        let source = match snapshot.value.provenance {
            crate::db::catalog::DdlProvenance::NativeCatalog => "NATIVE CATALOG",
            crate::db::catalog::DdlProvenance::AdapterGenerated => "GENERATED",
        };
        let provenance = tab
            .provenance(
                RelationView::Ddl,
                app.connection.active_identity(),
                app.active_profile(),
            )
            .map(provenance_label)
            .unwrap_or("UNKNOWN");
        let position = Some(
            editor_snapshot
                .as_ref()
                .map(|snapshot| {
                    format!(
                        "ROW {}  COL {}",
                        snapshot.cursor.line.saturating_add(1),
                        snapshot.cursor.column.saturating_add(1)
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "ROW {}  COL {}",
                        tab.ddl_viewport.row_offset.saturating_add(1),
                        tab.ddl_viewport.column_offset.saturating_add(1)
                    )
                }),
        );
        let available = usize::from(area.width.saturating_sub(2));
        let left_width = UnicodeWidthStr::width(" RELATION DDL ");
        let retain_provenance = provenance != "LIVE";
        let full_context = position
            .as_ref()
            .map(|position| format!("{source}  {position}  {provenance}"));
        let source_and_provenance = format!("{source}  {provenance}");
        let parts = if full_context.as_ref().is_some_and(|context| {
            left_width + UnicodeWidthStr::width(context.as_str()) + 2 <= available
        }) {
            full_context.unwrap_or_default()
        } else if retain_provenance
            && left_width + UnicodeWidthStr::width(source_and_provenance.as_str()) + 2 <= available
        {
            source_and_provenance
        } else if retain_provenance {
            provenance.to_owned()
        } else {
            source.to_owned()
        };
        block = block.title_top(Line::raw(format!(" {parts} ")).right_aligned());
    }
    if let Some((message, retry, cancel)) = status {
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
        render_ddl_editor(
            frame,
            chunks[1],
            app,
            theme,
            _state,
            block,
            editor_snapshot.as_ref(),
        );
        return;
    }
    render_ddl_editor(
        frame,
        chunks[1],
        app,
        theme,
        _state,
        block,
        editor_snapshot.as_ref(),
    );
}

fn relation_ddl_layout(area: Rect, has_status: bool) -> [Rect; 2] {
    if has_status {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(area);
        [chunks[0], chunks[1]]
    } else {
        [Rect::default(), area]
    }
}

fn render_ddl_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut super::UiState,
    block: ratatui::widgets::Block<'_>,
    supplied_snapshot: Option<&EditorRenderSnapshot>,
) {
    let inner = block.inner(area);
    let viewport = EditorViewport {
        width: inner.width as usize,
        height: inner.height as usize,
    };
    let snapshot = supplied_snapshot
        .cloned()
        .or_else(|| app.active_ddl_editor_snapshot(viewport).ok());
    let Some(snapshot) = snapshot.as_ref() else {
        frame.render_widget(block, area);
        return;
    };
    let ddl_session_id = if let Some(tab) = app.tabs.get(app.active_tab)
        && let crate::model::tab::WorkspaceTab::Relation(tab) = tab
    {
        super::register_text_selection_target(
            state,
            tab.ddl_editor_id,
            Rect::new(inner.x, inner.y, inner.width, inner.height),
            snapshot,
        );
        Some(tab.ddl_editor_id)
    } else {
        None
    };
    crate::ui::read_only_sql::ReadOnlySqlEditor {
        session_id: ddl_session_id.unwrap_or_default(),
        snapshot,
        block,
        focused: app.focus == Focus::Results && app.overlay.is_none(),
        show_line_numbers: true,
    }
    .render(frame, area, theme, state);
}

#[cfg(test)]
fn ddl_text(sql: &str) -> String {
    sanitize_terminal_text(sql)
}

fn clean(value: &str) -> String {
    sanitize_terminal_text(value).chars().take(240).collect()
}

#[cfg(test)]
mod relation_status_tests {
    use super::*;

    #[test]
    fn relation_ddl_layout_reserves_status_rows_only_when_needed() {
        let area = Rect::new(2, 3, 40, 12);
        let without_status = relation_ddl_layout(area, false);
        assert_eq!(without_status[1], area);

        let with_status = relation_ddl_layout(area, true);
        assert_eq!(with_status[0], Rect::new(2, 3, 40, 2));
        assert_eq!(with_status[1], Rect::new(2, 5, 40, 10));
    }

    #[test]
    fn relation_error_detail_keeps_retry_and_cancel_targets_independent() {
        let mut state = super::super::UiState::new();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 4))
            .expect("test terminal");
        terminal
            .draw(|frame| {
                render_status(
                    frame,
                    frame.area(),
                    "relation failed",
                    true,
                    true,
                    Theme::default(),
                    &mut state,
                );
            })
            .expect("render relation error");

        let detail = state
            .hit_regions
            .iter()
            .find(|region| matches!(region.target, HitTarget::OpenTextDetail(_)))
            .expect("detail target");
        assert!(
            state
                .hit_regions
                .iter()
                .any(|region| region.target == HitTarget::RelationRetry)
        );
        assert!(
            state
                .hit_regions
                .iter()
                .any(|region| region.target == HitTarget::RelationCancel)
        );
        assert!(
            !state
                .hit_regions
                .iter()
                .filter(|region| region.target == HitTarget::RelationRetry)
                .any(|region| region.area.intersects(detail.area))
        );
        assert!(
            !state
                .hit_regions
                .iter()
                .filter(|region| region.target == HitTarget::RelationCancel)
                .any(|region| region.area.intersects(detail.area))
        );
    }
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
    use crate::model::{
        cell_editor::{CellEditorBuffer, CellEditorKind, JsonBuffer, TemporalDraft, TypedDraft},
        relation::RelationTab,
        relation_edit::{CellEditorState, RelationEditSession, RelationGridMode},
        tab::WorkspaceTab,
        text_input::TextInput,
    };
    use chrono::NaiveDate;
    use ratatui::{Terminal, backend::TestBackend};
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn field_modal_keeps_long_content_and_cursor_inside_its_viewport() {
        use crate::db::mongodb::types::{MongoDocument, MongoNamespace};
        use crate::model::relation_edit::{
            DocumentEditorState, DocumentFieldEdit, DocumentFieldInput,
        };
        use ratatui::{layout::Rect, style::Style, widgets::Paragraph};
        let bson = mongodb::bson::doc! { "_id": 1, "tags": [] };
        let document = MongoDocument {
            raw: mongodb::bson::raw::RawDocumentBuf::from_bytes(
                mongodb::bson::to_vec(&bson).unwrap(),
            )
            .unwrap(),
            bson,
            editable: true,
        };
        let mut editor =
            DocumentEditorState::new(0, &MongoNamespace::collection("db", "items"), &document)
                .unwrap();
        let text = (0..80)
            .map(|i| format!("\"item-{i:02}\"\n"))
            .collect::<String>();
        let mut field = DocumentFieldEdit {
            name: "tags".into(),
            type_name: "array",
            input: DocumentFieldInput::Block(JsonBuffer::new(text)),
        };
        for (width, height) in [
            (90, 30),
            (60, 18),
            (32, 10),
            (20, 7),
            (20, 4),
            (3, 3),
            (90, 30),
        ] {
            let area = Rect::new(0, 0, width, height);
            let popup = super::field_popup_area(area, &field, false);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut state = super::super::UiState::new();
            terminal
                .draw(|frame| {
                    frame.render_widget(
                        Paragraph::new(
                            vec![".".repeat(width as usize); height as usize].join("\n"),
                        )
                        .style(Style::default()),
                        area,
                    );
                    super::render_document_field_editor(
                        frame,
                        area,
                        &editor,
                        &field,
                        super::Theme::default(),
                        &mut state,
                    );
                })
                .unwrap();
            let cursor = state
                .cursor
                .expect("the field remains editable after resize")
                .position;
            assert!(
                popup.contains(cursor),
                "cursor {cursor:?} outside {popup:?}"
            );
            assert!(cursor.y > popup.y && cursor.y < popup.bottom() - 1);
            let buffer = terminal.backend().buffer();
            for y in 0..height {
                for x in 0..width {
                    if !popup.contains(ratatui::layout::Position::new(x, y)) {
                        assert_eq!(
                            buffer[(x, y)].symbol(),
                            ".",
                            "content escaped popup at {x},{y}"
                        );
                    }
                }
            }
            if width >= 60 {
                let output = (popup.y..popup.bottom())
                    .map(|y| {
                        (popup.x..popup.right())
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    output.contains("item-79"),
                    "the editor scrolls to the final value"
                );
            }
        }
        editor.field_error = Some("invalid value".into());
        if let DocumentFieldInput::Block(buffer) = &mut field.input {
            buffer.move_up();
        }
        let mut terminal = Terminal::new(TestBackend::new(60, 18)).unwrap();
        let mut state = super::super::UiState::new();
        terminal
            .draw(|frame| {
                super::render_document_field_editor(
                    frame,
                    frame.area(),
                    &editor,
                    &field,
                    super::Theme::default(),
                    &mut state,
                )
            })
            .unwrap();
        assert!(
            super::field_popup_area(Rect::new(0, 0, 60, 18), &field, true)
                .contains(state.cursor.unwrap().position)
        );
    }

    #[test]
    fn field_modal_sizes_use_display_width_and_saturate_large_values() {
        use crate::model::relation_edit::{DocumentFieldEdit, DocumentFieldInput};
        use ratatui::layout::Rect;
        let mut field = DocumentFieldEdit {
            name: "text".into(),
            type_name: "string",
            input: DocumentFieldInput::Line(TextInput::from("界".repeat(25))),
        };
        assert_eq!(
            super::field_popup_area(Rect::new(0, 0, 100, 30), &field, false).width,
            56
        );
        field.input = DocumentFieldInput::Block(JsonBuffer::new("\n".repeat(65_536)));
        assert_eq!(
            super::field_popup_area(Rect::new(0, 0, 100, 30), &field, false).height,
            20
        );
    }

    fn json_editor_app(value: &str) -> crate::app::App {
        let mut app = crate::app::App::new(Vec::new());
        app.tabs
            .push(WorkspaceTab::Relation(RelationTab::new("users")));
        app.active_tab = 1;
        if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
            let mut edit =
                RelationEditSession::from_rows(vec![vec![crate::db::value::CellValue::Text(
                    "x".into(),
                )]]);
            edit.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
                row: 0,
                column: 0,
                input: CellEditorBuffer {
                    presence: crate::model::cell_editor::CellEditorPresence::Value,
                    content: crate::model::cell_editor::CellEditorContent::Typed {
                        kind: CellEditorKind::Json,
                        draft: TypedDraft::Json(JsonBuffer::new(value)),
                    },
                    ..CellEditorBuffer::default()
                },
                error: None,
            }));
            tab.edit = Some(edit);
        }
        app
    }

    fn render_json_editor(app: &crate::app::App, state: &mut super::super::UiState) {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                super::render(frame, frame.area(), app, super::Theme::default(), state);
            })
            .unwrap();
    }

    fn temporal_editor_app() -> crate::app::App {
        let mut app = crate::app::App::new(Vec::new());
        app.tabs
            .push(WorkspaceTab::Relation(RelationTab::new("users")));
        app.active_tab = 1;
        if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
            let mut edit =
                RelationEditSession::from_rows(vec![vec![crate::db::value::CellValue::Date(
                    NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
                )]]);
            edit.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
                row: 0,
                column: 0,
                input: CellEditorBuffer {
                    presence: crate::model::cell_editor::CellEditorPresence::Value,
                    content: crate::model::cell_editor::CellEditorContent::Typed {
                        kind: CellEditorKind::Date,
                        draft: TypedDraft::Temporal(TemporalDraft::date(
                            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap(),
                        )),
                    },
                    ..CellEditorBuffer::default()
                },
                error: None,
            }));
            tab.edit = Some(edit);
        }
        app
    }

    #[test]
    fn temporal_cell_editor_registers_relation_input_selection_target() {
        let app = temporal_editor_app();
        let mut state = super::super::UiState::new();

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                super::render(
                    frame,
                    frame.area(),
                    &app,
                    super::Theme::default(),
                    &mut state,
                );
            })
            .unwrap();

        let WorkspaceTab::Relation(tab) = &app.tabs[1] else {
            panic!("relation tab")
        };
        let row_id = tab.edit.as_ref().unwrap().rows[0].id;
        let (target, map) = state
            .input_selection_targets
            .iter()
            .find(|(target, _)| {
                matches!(
                    target,
                    super::super::text_selection::InputSelectionTarget::RelationTemporal {
                        tab_id,
                        row_id: candidate_row_id,
                        column: 0,
                    } if *tab_id == tab.id && *candidate_row_id == row_id
                )
            })
            .expect("relation temporal input target");
        assert_eq!(
            target,
            &super::super::text_selection::InputSelectionTarget::RelationTemporal {
                tab_id: tab.id,
                row_id,
                column: 0,
            }
        );
        assert_eq!(map.source_at(map.area.x, map.area.y), Some(0));
    }

    #[test]
    fn generic_cell_editor_registers_relation_input_selection_target() {
        let mut app = crate::app::App::new(Vec::new());
        app.tabs
            .push(WorkspaceTab::Relation(RelationTab::new("users")));
        app.active_tab = 1;
        if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
            let mut edit =
                RelationEditSession::from_rows(vec![vec![crate::db::value::CellValue::Text(
                    "hello".into(),
                )]]);
            edit.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
                row: 0,
                column: 0,
                input: CellEditorBuffer::from_value(
                    &crate::db::value::CellValue::Text("hello".into()),
                    None,
                ),
                error: None,
            }));
            tab.edit = Some(edit);
        }
        let mut state = super::super::UiState::new();
        render_json_editor(&app, &mut state);

        let WorkspaceTab::Relation(tab) = &app.tabs[1] else {
            panic!("relation tab")
        };
        let row_id = tab.edit.as_ref().unwrap().rows[0].id;
        let (target, _) = state
            .input_selection_targets
            .iter()
            .find(|(target, _)| {
                matches!(
                    target,
                    super::super::text_selection::InputSelectionTarget::RelationText {
                        tab_id,
                        row_id: candidate_row_id,
                        column: 0,
                    } if *tab_id == tab.id && *candidate_row_id == row_id
                )
            })
            .expect("relation text input target");
        assert_eq!(
            target,
            &super::super::text_selection::InputSelectionTarget::RelationText {
                tab_id: tab.id,
                row_id,
                column: 0,
            }
        );
    }

    #[test]
    fn json_cell_editor_registers_bar_cursor_at_end() {
        let app = json_editor_app("{}");
        let mut state = super::super::UiState::new();

        render_json_editor(&app, &mut state);

        assert_eq!(
            state.cursor,
            Some(super::super::CursorSpec {
                position: ratatui::layout::Position::new(7, 6),
                style: super::super::CursorStyle::Bar,
            })
        );
    }

    #[test]
    fn json_cell_editor_cursor_tracks_movement() {
        let mut app = json_editor_app("{\"a\": 1}");
        let mut state = super::super::UiState::new();

        render_json_editor(&app, &mut state);
        assert_eq!(state.cursor.map(|cursor| cursor.position.x), Some(13));

        if let WorkspaceTab::Relation(tab) = &mut app.tabs[1]
            && let Some(edit) = &mut tab.edit
            && let RelationGridMode::EditCell(editor) = &mut edit.mode
        {
            editor.input.json_buffer_mut().unwrap().move_left();
        }
        state.cursor = None;
        render_json_editor(&app, &mut state);
        assert_eq!(state.cursor.map(|cursor| cursor.position.x), Some(12));
    }

    #[test]
    fn json_cell_editor_cursor_uses_display_cell_width() {
        let app = json_editor_app("\t{}");
        let mut state = super::super::UiState::new();

        render_json_editor(&app, &mut state);

        let cursor = state.cursor.expect("JSON cursor");
        let projection = crate::security::project_editor_line("\t{}");
        assert_eq!(cursor.position.x, 5 + projection.text.width() as u16);
    }

    #[test]
    fn json_cell_editor_projection_keeps_source_and_display_separate() {
        let source = "\t{\"名字\": \"猫\"}";
        let projection = crate::security::project_editor_line(source);
        let spans = super::json_line_spans(source, 0, 80, 0, None);
        let rendered = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, projection.text);
        assert_eq!(projection.source_to_display_cells[1], 4);
        assert_eq!(source, "\t{\"名字\": \"猫\"}");
    }

    #[test]
    fn json_cell_editor_horizontal_slice_preserves_wide_cell_width() {
        assert_eq!(super::display_slice("a猫b", 1, 3), "猫");
        assert_eq!(super::display_slice("a猫b", 2, 5), " b");
        assert_eq!(super::display_slice("a猫b", 3, 6), "b");
    }

    #[test]
    fn json_cell_editor_vertical_scroll_keeps_cursor_in_body() {
        let value = "line 0\nline 1\nline 2\nline 3\nline 4";
        let app = json_editor_app(value);
        let mut state = super::super::UiState::new();

        render_json_editor(&app, &mut state);

        let cursor = state.cursor.expect("JSON cursor");
        assert!(cursor.position.y < 21);
        assert_eq!(cursor.position.y, 10);
    }

    #[test]
    fn json_cell_editor_horizontal_scroll_keeps_cursor_in_body() {
        let value = format!("{}{{}}", "x".repeat(120));
        let app = json_editor_app(&value);
        let mut state = super::super::UiState::new();

        render_json_editor(&app, &mut state);

        let cursor = state.cursor.expect("JSON cursor");
        assert_eq!(cursor.position.x, 74);
        assert!(cursor.position.x < 75);
    }

    #[test]
    fn json_cell_editor_empty_trailing_line_keeps_cursor_visible() {
        let app = json_editor_app("{}\n");
        let mut state = super::super::UiState::new();

        render_json_editor(&app, &mut state);

        let cursor = state.cursor.expect("JSON cursor");
        assert!(cursor.position.y < 21);
    }

    #[test]
    fn json_cell_editor_error_does_not_replace_cursor_or_body() {
        let mut app = json_editor_app("{}");
        if let WorkspaceTab::Relation(tab) = &mut app.tabs[1]
            && let Some(edit) = &mut tab.edit
            && let RelationGridMode::EditCell(editor) = &mut edit.mode
        {
            editor.error = Some("invalid JSON".into());
        }
        let mut state = super::super::UiState::new();

        render_json_editor(&app, &mut state);

        let cursor = state.cursor.expect("JSON cursor");
        assert!(cursor.position.y < 20);
    }

    #[test]
    fn json_cell_editor_tiny_area_does_not_leave_a_stale_cursor() {
        let app = json_editor_app("{}");
        let mut state = super::super::UiState::new();
        state.cursor = Some(super::super::CursorSpec {
            position: ratatui::layout::Position::new(1, 1),
            style: super::super::CursorStyle::Bar,
        });
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();

        terminal
            .draw(|frame| {
                super::render(
                    frame,
                    frame.area(),
                    &app,
                    super::Theme::default(),
                    &mut state,
                );
            })
            .unwrap();

        assert_eq!(state.cursor, None);
    }

    #[test]
    fn json_cell_editor_editing_operations_update_cursor_position() {
        let mut app = json_editor_app("{}");
        let mut state = super::super::UiState::new();

        if let WorkspaceTab::Relation(tab) = &mut app.tabs[1]
            && let Some(edit) = &mut tab.edit
            && let RelationGridMode::EditCell(editor) = &mut edit.mode
        {
            let json = editor.input.json_buffer_mut().unwrap();
            json.move_home();
            json.insert('x');
        }
        render_json_editor(&app, &mut state);

        assert_eq!(state.cursor.map(|cursor| cursor.position.x), Some(6));
    }

    #[test]
    fn cell_editor_value_contains_only_the_cell_content() {
        let editor = CellEditorState {
            row: 5,
            column: 8,
            input: CellEditorBuffer::from_value(
                &crate::db::value::CellValue::Text("failed".into()),
                None,
            ),
            error: None,
        };

        assert_eq!(cell_editor_value(&editor), "failed");
        assert!(!cell_editor_value(&editor).contains("Edit cell"));
        assert!(!cell_editor_value(&editor).contains("[6, 9]"));
    }

    #[test]
    fn typed_cell_editor_rendering_is_safe_in_tiny_areas() {
        let editors = [
            CellEditorBuffer {
                presence: crate::model::cell_editor::CellEditorPresence::Value,
                content: crate::model::cell_editor::CellEditorContent::Typed {
                    kind: CellEditorKind::Json,
                    draft: TypedDraft::Json(JsonBuffer::new("{}")),
                },
                ..CellEditorBuffer::default()
            },
            CellEditorBuffer {
                presence: crate::model::cell_editor::CellEditorPresence::Value,
                content: crate::model::cell_editor::CellEditorContent::Typed {
                    kind: CellEditorKind::Date,
                    draft: TypedDraft::Temporal(TemporalDraft::date(
                        NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                    )),
                },
                ..CellEditorBuffer::default()
            },
            CellEditorBuffer {
                presence: crate::model::cell_editor::CellEditorPresence::Value,
                content: crate::model::cell_editor::CellEditorContent::Typed {
                    kind: CellEditorKind::Boolean,
                    draft: TypedDraft::Boolean(TextInput::from("true")),
                },
                ..CellEditorBuffer::default()
            },
        ];

        for input in editors {
            let mut app = crate::app::App::new(Vec::new());
            app.tabs
                .push(WorkspaceTab::Relation(RelationTab::new("users")));
            app.active_tab = 1;
            if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
                let mut edit =
                    RelationEditSession::from_rows(vec![vec![crate::db::value::CellValue::Text(
                        "x".into(),
                    )]]);
                edit.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
                    row: 0,
                    column: 0,
                    input,
                    error: None,
                }));
                tab.edit = Some(edit);
            }

            let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
            terminal
                .draw(|frame| {
                    super::render(
                        frame,
                        frame.area(),
                        &app,
                        super::Theme::default(),
                        &mut super::super::UiState::new(),
                    );
                })
                .unwrap();
        }
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
