use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use crate::{
    app::App,
    db::{catalog::CatalogKind, catalog_mutation::CatalogObjectType},
    model::catalog_editor::{
        CatalogDraft, CatalogEditorOperation, CatalogEditorPage, CatalogEditorState,
        ConstraintDraft, DatabaseDraft, IndexDraft, MaterializedViewDraft, RoleDraft, SchemaDraft,
        SequenceDraft, TableDraft, TableEditorField, ViewDraft,
    },
    security::sanitize_terminal_text,
};

use super::{HitRegion, HitTarget, Theme, UiState, icons::IconSet, render_text_input};

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    ui: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
    let Some(editor) = app.catalog_editor.as_ref() else {
        return;
    };
    let popup = super::centered(area, 106.min(area.width), 34.min(area.height));
    frame.render_widget(Clear, popup);
    let title = panel_title(editor);
    let block = super::panel_block(&title, true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    match editor.page {
        CatalogEditorPage::ObjectPicker => picker(frame, inner, editor, theme, icons),
        CatalogEditorPage::Loading => loading(frame, inner, editor, theme),
        CatalogEditorPage::Form => form(frame, inner, editor, ui, theme),
        CatalogEditorPage::SqlPreview => preview(frame, inner, editor, theme),
    }
}

fn panel_title(editor: &CatalogEditorState) -> String {
    match editor.page {
        CatalogEditorPage::ObjectPicker => " NEW CATALOG OBJECT ".into(),
        CatalogEditorPage::Loading => " CATALOG EDITOR // LOADING ".into(),
        CatalogEditorPage::Form => {
            let verb = match editor.mode {
                crate::db::catalog_mutation::CatalogMutationMode::Create => "NEW",
                crate::db::catalog_mutation::CatalogMutationMode::Edit => "EDIT",
            };
            format!(
                " {verb} {} ",
                editor
                    .object_type
                    .map_or("OBJECT", |kind| kind.display_label())
            )
        }
        CatalogEditorPage::SqlPreview => " REVIEW SQL ".into(),
    }
}

fn picker(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &CatalogEditorState,
    theme: Theme,
    icons: IconSet,
) {
    frame.render_widget(
        Paragraph::new(format!("TARGET  {}", target_label(editor)))
            .style(Style::new().fg(theme.muted).bg(theme.surface)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    for (index, option) in editor.options.iter().enumerate() {
        let row = Rect::new(
            area.x,
            area.y.saturating_add(2 + index as u16),
            area.width,
            1,
        );
        if row.y >= area.bottom().saturating_sub(1) {
            break;
        }
        let selected = index == editor.selected_option;
        let background = if selected {
            theme.selection
        } else {
            theme.surface
        };
        let label_style = Style::new()
            .fg(theme.text)
            .bg(background)
            .add_modifier(if selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let icon = icons.catalog_object(option.object_type);
        let icon_style = Style::new()
            .fg(object_color(option.object_type, theme))
            .bg(background);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(if selected { "› " } else { "  " }, label_style),
                Span::styled(format!("{icon} "), icon_style),
                Span::styled(sanitize_terminal_text(&option.label), label_style),
            ])),
            row,
        );
    }
    frame.render_widget(
        Paragraph::new("j/k · ↑/↓ select   Enter continue   Esc close")
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .alignment(ratatui::layout::Alignment::Center),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

fn object_color(object_type: CatalogObjectType, theme: Theme) -> Color {
    match object_type {
        CatalogObjectType::Catalog(CatalogKind::Database | CatalogKind::Schema) => theme.action,
        CatalogObjectType::Catalog(
            CatalogKind::Table | CatalogKind::View | CatalogKind::MaterializedView,
        ) => theme.text,
        CatalogObjectType::Catalog(CatalogKind::PrimaryKey | CatalogKind::UniqueConstraint) => {
            theme.warning
        }
        CatalogObjectType::Catalog(CatalogKind::ForeignKey | CatalogKind::Trigger) => theme.accent,
        CatalogObjectType::LoginRole => theme.success,
        CatalogObjectType::Role => theme.warning,
        CatalogObjectType::Catalog(_) => theme.muted,
    }
}

fn loading(frame: &mut Frame<'_>, area: Rect, editor: &CatalogEditorState, theme: Theme) {
    let operation = editor
        .operation
        .map(|operation| match operation {
            CatalogEditorOperation::LoadingDefinition { .. } => "Loading definition",
            CatalogEditorOperation::Planning { .. } => "Planning mutation",
            CatalogEditorOperation::Applying { .. } => "Applying mutation",
        })
        .unwrap_or("Working");
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                operation,
                Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
            )),
            Line::raw("Please wait..."),
            Line::raw("Esc cancels when the operation is safe to cancel"),
        ])
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn form(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &CatalogEditorState,
    ui: &mut UiState,
    theme: Theme,
) {
    let title = editor
        .object_type
        .map(|kind| kind.display_label())
        .unwrap_or("Object");
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} DETAILS", title.to_uppercase()),
                Style::new()
                    .fg(theme.muted)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "  TARGET  {}",
                    sanitize_terminal_text(&target_label(editor))
                ),
                Style::new().fg(theme.muted).bg(theme.surface),
            ),
        ])),
        chunks[0],
    );
    if let Some(CatalogDraft::Database(draft)) = editor.draft.as_ref() {
        render_database(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::Role(draft)) = editor.draft.as_ref() {
        render_role(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::Schema(draft)) = editor.draft.as_ref() {
        render_schema(frame, chunks[1], draft, ui, theme);
    } else if let Some(CatalogDraft::Table(draft)) = editor.draft.as_ref() {
        render_table(frame, chunks[1], draft, ui, theme);
    } else if let Some(CatalogDraft::Index(draft)) = editor.draft.as_ref() {
        render_index(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::Constraint(draft)) = editor.draft.as_ref() {
        render_constraint(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::View(draft)) = editor.draft.as_ref() {
        render_view(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::MaterializedView(draft)) = editor.draft.as_ref() {
        render_materialized_view(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::Sequence(draft)) = editor.draft.as_ref() {
        render_sequence(frame, chunks[1], draft, theme);
    } else {
        frame.render_widget(
            Paragraph::new("Definition form is ready for the selected object type."),
            chunks[1],
        );
    }
    let feedback = editor
        .error
        .as_deref()
        .map(|error| format!("× {}", sanitize_terminal_text(error)))
        .unwrap_or_else(|| {
            if matches!(
                editor.draft.as_ref(),
                Some(CatalogDraft::MaterializedView(_))
            ) && editor.mode == crate::db::catalog_mutation::CatalogMutationMode::Create
            {
                "Tab/Shift-Tab fields   Space toggle data   Enter preview   Esc cancel".into()
            } else {
                "Tab/Shift-Tab fields   Enter preview   Esc cancel".into()
            }
        });
    frame.render_widget(
        Paragraph::new(feedback)
            .style(Style::new().fg(editor.error.as_ref().map_or(theme.muted, |_| theme.error))),
        chunks[2],
    );
}

fn render_schema(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &SchemaDraft,
    ui: &mut UiState,
    theme: Theme,
) {
    let rows = [
        ("Name", &draft.name),
        ("Owner", &draft.owner),
        ("Comment", &draft.comment),
    ];
    for (index, (label, input)) in rows.into_iter().enumerate() {
        let row = Rect::new(area.x, area.y.saturating_add(index as u16), area.width, 1);
        ui.hit_regions.push(HitRegion {
            area: row,
            target: HitTarget::CatalogEditorField(index),
        });
        let active = draft.selected_field == index;
        let label_width = row.width.min(18);
        frame.render_widget(
            Paragraph::new(if active {
                format!("› {label:<15}")
            } else {
                format!("  {label:<15}")
            })
            .style(
                Style::new()
                    .fg(if active { theme.action } else { theme.muted })
                    .bg(theme.surface)
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Rect::new(row.x, row.y, label_width, 1),
        );
        let value_area = Rect::new(
            row.x.saturating_add(label_width),
            row.y,
            row.width.saturating_sub(label_width),
            1,
        );
        let style = Style::new().fg(theme.text).bg(if active {
            theme.selection
        } else {
            theme.surface
        });
        if active {
            render_text_input(frame, value_area, "", input, style, ui);
        } else {
            frame.render_widget(
                Paragraph::new(sanitize_terminal_text(input.value())).style(style),
                value_area,
            );
        }
    }
}

fn render_role(frame: &mut Frame<'_>, area: Rect, draft: &RoleDraft, theme: Theme) {
    let password = draft.password.as_ref().map_or("<unchanged>", |_| "<set>");
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!(
                "Name: {}  Login: {}",
                draft.name.value(),
                draft.login
            )),
            Line::raw(format!(
                "Superuser: {}  Create DB: {}  Create Role: {}",
                draft.superuser, draft.createdb, draft.createrole
            )),
            Line::raw(format!(
                "Inherit: {}  Replication: {}  Bypass RLS: {}",
                draft.inherit, draft.replication, draft.bypass_rls
            )),
            Line::raw(format!(
                "Connection limit: {}  Valid until: {}",
                draft.connection_limit.value(),
                draft.valid_until.value()
            )),
            Line::raw(format!(
                "Password: {}  Memberships: {}",
                password,
                draft.memberships.value()
            )),
            Line::raw(format!("Comment: {}", draft.comment.value())),
        ])
        .style(Style::new().fg(theme.text))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_database(frame: &mut Frame<'_>, area: Rect, draft: &DatabaseDraft, theme: Theme) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!(
                "Name: {}  Owner: {}",
                draft.name.value(),
                draft.owner.value()
            )),
            Line::raw(format!(
                "Template: {}  Encoding: {}",
                draft.template.value(),
                draft.encoding.value()
            )),
            Line::raw(format!(
                "Locale provider: {}  Locale: {}",
                draft.locale_provider.value(),
                draft.locale.value()
            )),
            Line::raw(format!(
                "Collation: {}  Ctype: {}",
                draft.collation.value(),
                draft.ctype.value()
            )),
            Line::raw(format!(
                "Tablespace: {}  Connection limit: {}",
                draft.tablespace.value(),
                draft.connection_limit.value()
            )),
            Line::raw(format!(
                "Allow connections: {}  Is template: {}",
                draft.allow_connections, draft.is_template
            )),
            Line::raw(format!("Comment: {}", draft.comment.value())),
        ])
        .style(Style::new().fg(theme.text))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_sequence(frame: &mut Frame<'_>, area: Rect, draft: &SequenceDraft, theme: Theme) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!(
                "Name: {}  Schema: {}  Owner: {}",
                draft.name.value(),
                draft.schema.value(),
                draft.owner.value()
            )),
            Line::raw(format!("Comment: {}", draft.comment.value())),
            Line::raw(format!(
                "Type: {}  Increment: {}  Start: {}  Restart: {}",
                draft.data_type.value(),
                draft.increment.value(),
                draft.start_value.value(),
                draft.restart_value.value()
            )),
            Line::raw(format!(
                "Min: {:?}  Max: {:?}  Cache: {}  Cycle: {}",
                draft.min_value,
                draft.max_value,
                draft.cache.value(),
                draft.cycle
            )),
            Line::raw(format!("Owned by: {}", draft.owned_by.value())),
        ])
        .style(Style::new().fg(theme.text))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_materialized_view(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &MaterializedViewDraft,
    theme: Theme,
) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!("Name: {}", draft.name.value())),
            Line::raw(format!(
                "Schema: {}   Owner: {}",
                draft.schema.value(),
                draft.owner.value()
            )),
            Line::raw(format!("Comment: {}", draft.comment.value())),
            Line::raw(format!("Tablespace: {}", draft.tablespace.value())),
            Line::raw(format!(
                "Query (read-only on edit): {}",
                draft.query.value()
            )),
            Line::raw(format!(
                "WITH {}DATA",
                if draft.with_data { "" } else { "NO " }
            )),
        ])
        .style(Style::new().fg(theme.text))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_view(frame: &mut Frame<'_>, area: Rect, draft: &ViewDraft, theme: Theme) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!("Name: {}", draft.name.value())),
            Line::raw(format!(
                "Schema: {}   Owner: {}",
                draft.schema.value(),
                draft.owner.value()
            )),
            Line::raw(format!("Comment: {}", draft.comment.value())),
            Line::raw(format!("Output columns: {}", draft.output_columns.value())),
            Line::raw(format!("Query: {}", draft.query.value())),
            Line::raw(format!(
                "Security barrier: {:?}  invoker: {:?}  check: {:?}",
                draft.security_barrier, draft.security_invoker, draft.check_option
            )),
            Line::raw(format!(
                "Availability: barrier={}  invoker={}  check={}",
                view_option_status(&draft.security_barrier.availability),
                view_option_status(&draft.security_invoker.availability),
                view_option_status(&draft.check_option.availability),
            )),
        ])
        .style(Style::new().fg(theme.text))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn view_option_status(
    availability: &crate::db::catalog_mutation::ViewMutationOptionAvailability,
) -> String {
    match availability {
        crate::db::catalog_mutation::ViewMutationOptionAvailability::Available => {
            "available".into()
        }
        crate::db::catalog_mutation::ViewMutationOptionAvailability::Unavailable { reason } => {
            format!("disabled ({reason})")
        }
    }
}

fn render_constraint(frame: &mut Frame<'_>, area: Rect, draft: &ConstraintDraft, theme: Theme) {
    let kind = crate::db::catalog_mutation::CatalogObjectType::Catalog(draft.kind.catalog_kind())
        .display_label();
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!("Kind: {kind}")),
            Line::raw(format!("Name: {}", draft.name.value())),
            Line::raw(format!(
                "Relation: {}.{}",
                draft.schema.value(),
                draft.relation.value()
            )),
            Line::raw(format!("Columns: {}", draft.columns.value())),
            Line::raw(format!(
                "References: {}.{} ({})",
                draft.referenced_schema.value(),
                draft.referenced_relation.value(),
                draft.referenced_columns.value()
            )),
            Line::raw(format!(
                "MATCH {}  ON UPDATE {}  ON DELETE {}",
                draft.match_type.value(),
                draft.on_update.value(),
                draft.on_delete.value()
            )),
            Line::raw(format!("Expression: {}", draft.expression.value())),
            Line::raw(format!(
                "Deferrable: {}  Initially deferred: {}  NOT VALID: {}",
                draft.deferrable, draft.initially_deferred, draft.not_valid
            )),
        ])
        .style(Style::new().fg(theme.text)),
        area,
    );
}

fn render_index(frame: &mut Frame<'_>, area: Rect, draft: &IndexDraft, theme: Theme) {
    let columns = draft
        .columns
        .iter()
        .map(|column| {
            format!(
                "{}{}",
                column.expression.value(),
                if column.descending { " DESC" } else { " ASC" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!("Name: {}", draft.name.value())),
            Line::raw(format!(
                "Relation: {}.{}",
                draft.schema.value(),
                draft.relation.value()
            )),
            Line::raw(format!(
                "Unique: {}   Method: {}",
                draft.unique,
                draft.access_method.value()
            )),
            Line::raw(format!("Columns: {columns}")),
            Line::raw(format!("INCLUDE: {}", draft.include_columns.value())),
            Line::raw(format!("Predicate: {}", draft.predicate.value())),
            Line::raw(format!("Tablespace: {}", draft.tablespace.value())),
        ])
        .style(Style::new().fg(theme.text)),
        area,
    );
}

fn render_table(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &TableDraft,
    ui: &mut UiState,
    theme: Theme,
) {
    let heading = Line::from(vec![
        Span::styled("GENERAL", section_style(true, theme)),
        Span::raw("    "),
        Span::styled("COLUMNS", section_style(false, theme)),
    ]);
    frame.render_widget(
        Paragraph::new(heading),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let general = [
        (TableEditorField::Name, "Name", &draft.name),
        (TableEditorField::Schema, "Schema", &draft.schema),
        (TableEditorField::Owner, "Owner", &draft.owner),
        (TableEditorField::Comment, "Comment", &draft.comment),
    ];
    for (offset, (field, label, input)) in general.into_iter().enumerate() {
        render_table_text_field(
            frame,
            Rect::new(
                area.x,
                area.y.saturating_add(2 + offset as u16),
                area.width,
                1,
            ),
            field,
            label,
            input,
            draft.selected_field,
            ui,
            theme,
        );
    }
    let columns_y = area.y.saturating_add(7);
    frame.render_widget(
        Paragraph::new(Span::styled("COLUMNS", section_style(true, theme))),
        Rect::new(area.x, columns_y, area.width, 1),
    );
    for (index, column) in draft
        .columns
        .iter()
        .enumerate()
        .take(usize::from(area.height.saturating_sub(9)))
    {
        let y = columns_y.saturating_add(1 + index as u16);
        let active =
            draft.selected_field == TableEditorField::ColumnList && index == draft.selected_column;
        let style = Style::new().fg(theme.text).bg(if active {
            theme.selection
        } else {
            theme.surface
        });
        frame.render_widget(
            Paragraph::new(format!(
                "{} {:<20} {:<20} {}",
                if active { "›" } else { " " },
                sanitize_terminal_text(column.name.value()).if_empty("<unnamed>"),
                sanitize_terminal_text(column.native_type.value()),
                if column.nullable { "NULL" } else { "NOT NULL" }
            ))
            .style(style),
            Rect::new(area.x, y, area.width, 1),
        );
        ui.hit_regions.push(HitRegion {
            area: Rect::new(area.x, y, area.width, 1),
            target: HitTarget::CatalogEditorTableColumn(index),
        });
    }
    let detail_y = columns_y.saturating_add(
        2 + draft
            .columns
            .len()
            .min(usize::from(area.height.saturating_sub(11))) as u16,
    );
    if let Some(column) = draft.selected_column() {
        frame.render_widget(
            Paragraph::new("COLUMN DETAILS")
                .style(Style::new().fg(theme.muted).add_modifier(Modifier::BOLD)),
            Rect::new(area.x, detail_y, area.width, 1),
        );
        render_table_text_field(
            frame,
            Rect::new(area.x, detail_y + 1, area.width, 1),
            TableEditorField::ColumnName,
            "Name",
            &column.name,
            draft.selected_field,
            ui,
            theme,
        );
        render_table_text_field(
            frame,
            Rect::new(area.x, detail_y + 2, area.width, 1),
            TableEditorField::ColumnType,
            "Type",
            &column.native_type,
            draft.selected_field,
            ui,
            theme,
        );
        render_table_text_field(
            frame,
            Rect::new(area.x, detail_y + 3, area.width, 1),
            TableEditorField::ColumnDefault,
            "Default",
            &column.default_expression,
            draft.selected_field,
            ui,
            theme,
        );
        render_table_text_field(
            frame,
            Rect::new(area.x, detail_y + 4, area.width, 1),
            TableEditorField::ColumnComment,
            "Comment",
            &column.comment,
            draft.selected_field,
            ui,
            theme,
        );
        let nullable_area = Rect::new(area.x, detail_y + 5, area.width / 2, 1);
        let identity_area = Rect::new(
            area.x + area.width / 2,
            detail_y + 5,
            area.width.saturating_sub(area.width / 2),
            1,
        );
        frame.render_widget(
            Paragraph::new(format!(
                "  Nullable       {}",
                if column.nullable { "[x] On" } else { "[ ] Off" }
            ))
            .style(Style::new().fg(theme.text).bg(
                if draft.selected_field == TableEditorField::ColumnNullable {
                    theme.selection
                } else {
                    theme.surface
                },
            )),
            nullable_area,
        );
        frame.render_widget(
            Paragraph::new(format!(
                "  Identity       {}",
                if column.identity { "[x] On" } else { "[ ] Off" }
            ))
            .style(Style::new().fg(theme.text).bg(
                if draft.selected_field == TableEditorField::ColumnIdentity {
                    theme.selection
                } else {
                    theme.surface
                },
            )),
            identity_area,
        );
        ui.hit_regions.push(HitRegion {
            area: nullable_area,
            target: HitTarget::CatalogEditorTableField(TableEditorField::ColumnNullable),
        });
        ui.hit_regions.push(HitRegion {
            area: identity_area,
            target: HitTarget::CatalogEditorTableField(TableEditorField::ColumnIdentity),
        });
    }
    let actions = [
        (
            "[ Add Column ]",
            TableEditorField::AddColumn,
            HitTarget::CatalogEditorAddTableColumn,
        ),
        (
            "[ Remove Column ]",
            TableEditorField::RemoveColumn,
            HitTarget::CatalogEditorRemoveTableColumn,
        ),
        (
            "[ Review SQL ]",
            TableEditorField::Review,
            HitTarget::CatalogEditorReview,
        ),
        (
            "[ Cancel ]",
            TableEditorField::Cancel,
            HitTarget::CatalogEditorCancel,
        ),
    ];
    let mut x = area.x;
    for (label, field, target) in actions {
        let width = label.len() as u16;
        let action_area = Rect::new(x, area.bottom().saturating_sub(1), width, 1);
        frame.render_widget(
            Paragraph::new(label).style(
                Style::new()
                    .fg(if draft.selected_field == field {
                        theme.background
                    } else {
                        theme.action
                    })
                    .bg(if draft.selected_field == field {
                        theme.accent
                    } else {
                        theme.surface
                    }),
            ),
            action_area,
        );
        ui.hit_regions.push(HitRegion {
            area: action_area,
            target,
        });
        x = x.saturating_add(width + 3);
    }
}

trait EmptyText {
    fn if_empty(self, fallback: &str) -> String;
}

impl EmptyText for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_owned()
        } else {
            self
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_table_text_field(
    frame: &mut Frame<'_>,
    area: Rect,
    field: TableEditorField,
    label: &str,
    input: &crate::model::text_input::TextInput,
    selected: TableEditorField,
    ui: &mut UiState,
    theme: Theme,
) {
    let active = field == selected;
    let label_width = area.width.min(18);
    let value_area = Rect::new(
        area.x + label_width,
        area.y,
        area.width.saturating_sub(label_width),
        1,
    );
    frame.render_widget(
        Paragraph::new(format!("{} {:<15}", if active { "›" } else { " " }, label)).style(
            Style::new()
                .fg(if active { theme.action } else { theme.muted })
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(area.x, area.y, label_width, 1),
    );
    ui.hit_regions.push(HitRegion {
        area,
        target: HitTarget::CatalogEditorTableField(field),
    });
    if active {
        render_text_input(
            frame,
            value_area,
            "",
            input,
            Style::new().fg(theme.text).bg(theme.selection),
            ui,
        );
    } else {
        frame.render_widget(
            Paragraph::new(sanitize_terminal_text(input.value()))
                .style(Style::new().fg(theme.text).bg(theme.surface)),
            value_area,
        );
    }
}

fn section_style(selected: bool, theme: Theme) -> Style {
    if selected {
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.muted)
    }
}

fn preview(frame: &mut Frame<'_>, area: Rect, editor: &CatalogEditorState, theme: Theme) {
    let sql = editor
        .plan
        .as_ref()
        .map(|plan| sanitize_terminal_text(&plan.sql()))
        .unwrap_or_else(|| "No mutation plan is available yet.".into());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "SQL PREVIEW",
                Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
            )),
            Line::raw(format!("target: {}", target_label(editor))),
            Line::raw(""),
            Line::raw(sql),
            Line::raw(""),
            Line::raw("Enter apply   Esc return to form"),
        ])
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn target_label(editor: &CatalogEditorState) -> String {
    match &editor.anchor {
        crate::db::catalog_mutation::CatalogMutationAnchor::Profile { profile_id } => {
            format!("profile {profile_id}")
        }
        crate::db::catalog_mutation::CatalogMutationAnchor::Catalog(id) => id.native_path.join("."),
        crate::db::catalog_mutation::CatalogMutationAnchor::Group { schema, group } => {
            format!("{}. {:?}", schema.native_path.join("."), group)
        }
    }
}
