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
        CatalogFormFocus, ConstraintDraft, DatabaseDraft, IndexDraft, MaterializedViewDraft,
        RoleDraft, SchemaDraft, SequenceDraft, TableActionField, TableColumnField, TableDraft,
        TableEditorFocus, TableGeneralField, ViewDraft,
    },
    model::text_input::TextInput,
    security::sanitize_terminal_text,
};

use super::{
    HitRegion, HitTarget, Theme, UiState,
    icons::IconSet,
    render_text_input,
    shortcut_hints::{self, ShortcutHint},
};

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
        CatalogEditorPage::Form => form(frame, inner, app, editor, ui, theme),
        CatalogEditorPage::SqlPreview => preview(frame, inner, editor, theme),
    }
    if editor.page == CatalogEditorPage::Form
        && let Some(CatalogDraft::Table(draft)) = editor.draft.as_ref()
        && let Some(session) = draft.column_editor.as_ref()
    {
        render_table_column_details_modal(frame, inner, draft.focus, session, ui, theme);
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
        Paragraph::new(shortcut_hints::line(
            &[
                ShortcutHint::new("j/k · ↑/↓", "select"),
                ShortcutHint::new("Enter", "continue"),
                ShortcutHint::new("Esc", "close"),
            ],
            area.width,
            theme,
            theme.surface,
        ))
        .style(Style::new().bg(theme.surface))
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
    let content_area = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
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
        content_area,
    );
    frame.render_widget(
        Paragraph::new(shortcut_hints::line(
            &[ShortcutHint::new("Esc", "cancel")],
            area.width,
            theme,
            theme.surface,
        ))
        .style(Style::new().bg(theme.surface)),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

fn form(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
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
        let owner_choices = app.catalog_owner_choices();
        render_schema(
            frame,
            chunks[1],
            draft,
            ui,
            theme,
            owner_choices,
            &app.connection.owner_context,
            &editor.owner_picker,
        );
    } else if let Some(CatalogDraft::Table(draft)) = editor.draft.as_ref() {
        render_table(frame, chunks[1], draft, ui, theme);
    } else if let Some(CatalogDraft::Index(draft)) = editor.draft.as_ref() {
        render_index(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::Constraint(draft)) = editor.draft.as_ref() {
        render_constraint(frame, chunks[1], draft, theme);
    } else if let Some(CatalogDraft::View(draft)) = editor.draft.as_ref() {
        render_view(
            frame,
            chunks[1],
            draft,
            ui,
            theme,
            app.catalog_owner_choices(),
            &editor.owner_picker,
        );
    } else if let Some(CatalogDraft::MaterializedView(draft)) = editor.draft.as_ref() {
        render_materialized_view(
            frame,
            chunks[1],
            draft,
            editor.mode == crate::db::catalog_mutation::CatalogMutationMode::Create,
            ui,
            theme,
            app.catalog_owner_choices(),
            &editor.owner_picker,
        );
    } else if let Some(CatalogDraft::Sequence(draft)) = editor.draft.as_ref() {
        render_sequence(
            frame,
            chunks[1],
            draft,
            ui,
            theme,
            app.catalog_owner_choices(),
            &editor.owner_picker,
        );
    } else {
        frame.render_widget(
            Paragraph::new("Definition form is ready for the selected object type."),
            chunks[1],
        );
    }
    if let Some(error) = editor.error.as_deref() {
        frame.render_widget(
            Paragraph::new(format!("× {}", sanitize_terminal_text(error)))
                .style(Style::new().fg(theme.error).bg(theme.surface)),
            chunks[2],
        );
    } else if matches!(
        editor.draft.as_ref(),
        Some(CatalogDraft::Table(draft)) if draft.column_editor.is_some()
    ) {
        frame.render_widget(
            Paragraph::new("").style(Style::new().bg(theme.surface)),
            chunks[2],
        );
    } else {
        let mut hints = vec![ShortcutHint::new("Tab/Shift-Tab", "fields")];
        if matches!(
            editor.draft.as_ref(),
            Some(CatalogDraft::MaterializedView(_))
        ) && editor.mode == crate::db::catalog_mutation::CatalogMutationMode::Create
        {
            hints.push(ShortcutHint::new("Space", "toggle data"));
        }
        if editor.owner_picker_active() {
            hints.extend([
                ShortcutHint::new("Up/Down", "role"),
                ShortcutHint::new("Enter", "choose owner"),
                ShortcutHint::new("Esc", "close list"),
            ]);
        } else {
            if editor.owner_field_focused() && app.catalog_owner_choices().is_some() {
                hints.push(ShortcutHint::new("Enter", "owner list"));
            } else {
                hints.push(ShortcutHint::new("Enter", "preview"));
            }
            hints.push(ShortcutHint::new("Esc", "cancel"));
        }
        frame.render_widget(
            Paragraph::new(shortcut_hints::line(
                &hints,
                chunks[2].width,
                theme,
                theme.surface,
            ))
            .style(Style::new().bg(theme.surface)),
            chunks[2],
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_schema(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &SchemaDraft,
    ui: &mut UiState,
    theme: Theme,
    owner_choices: Option<&[crate::db::catalog_mutation::CatalogOwnerChoice]>,
    owner_context: &crate::model::workspace::CatalogOwnerContextState,
    picker: &crate::model::catalog_editor::OwnerPickerState,
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
    render_owner_picker(frame, area, owner_choices, picker, ui, theme);
    if draft.selected_field == crate::model::catalog_editor::SCHEMA_OWNER_FIELD
        && !picker.open
        && let crate::model::workspace::CatalogOwnerContextState::Failed { message, .. } =
            owner_context
    {
        frame.render_widget(
            Paragraph::new(format!(
                "Owner roles unavailable: {}",
                sanitize_terminal_text(message)
            ))
            .style(Style::new().fg(theme.warning).bg(theme.surface)),
            Rect::new(area.x, area.y.saturating_add(4), area.width, 1),
        );
    }
}

fn render_owner_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    owner_choices: Option<&[crate::db::catalog_mutation::CatalogOwnerChoice]>,
    picker: &crate::model::catalog_editor::OwnerPickerState,
    ui: &mut UiState,
    theme: Theme,
) {
    let Some(choices) = owner_choices else {
        return;
    };
    if !picker.open {
        return;
    }
    let picker_y = area.y.saturating_add(4);
    let visible = picker.visible(choices);
    let content_bottom = area.bottom().saturating_sub(2);
    let max_rows = usize::from(content_bottom.saturating_sub(picker_y + 1));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "OWNER ROLE",
                Style::new()
                    .fg(theme.muted)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  /{}", sanitize_terminal_text(picker.filter.value())),
                Style::new().fg(theme.action).bg(theme.surface),
            ),
        ]))
        .style(Style::new().bg(theme.surface)),
        Rect::new(area.x, picker_y, area.width, 1),
    );
    for (offset, choice) in visible.into_iter().take(max_rows).enumerate() {
        let row = Rect::new(
            area.x,
            picker_y.saturating_add(1 + offset as u16),
            area.width,
            1,
        );
        ui.hit_regions.push(HitRegion {
            area: row,
            target: HitTarget::CatalogOwnerChoice(choice.name.clone()),
        });
        let selected = Some(choice.name.as_str()) == picker.selected_name.as_deref();
        frame.render_widget(
            Paragraph::new(format!(
                "{}{}  {}",
                if selected { "› " } else { "  " },
                choice.name,
                if choice.selectable {
                    "SELECTABLE"
                } else {
                    "DISABLED"
                }
            ))
            .style(
                Style::new()
                    .fg(if choice.selectable {
                        theme.text
                    } else {
                        theme.muted
                    })
                    .bg(if selected {
                        theme.selection
                    } else {
                        theme.surface
                    }),
            ),
            row,
        );
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

fn render_sequence(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &SequenceDraft,
    ui: &mut UiState,
    theme: Theme,
    owner_choices: Option<&[crate::db::catalog_mutation::CatalogOwnerChoice]>,
    picker: &crate::model::catalog_editor::OwnerPickerState,
) {
    let bottom = area.bottom().saturating_sub(2);
    let compact = area.height < 18;
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        "GENERAL",
        matches!(
            draft.focus,
            CatalogFormFocus::Name
                | CatalogFormFocus::Schema
                | CatalogFormFocus::Owner
                | CatalogFormFocus::Comment
        ),
        theme,
    );
    let general = [
        (CatalogFormFocus::Name, "Name", &draft.name),
        (CatalogFormFocus::Schema, "Schema", &draft.schema),
        (CatalogFormFocus::Owner, "Owner", &draft.owner),
        (CatalogFormFocus::Comment, "Comment", &draft.comment),
    ];
    render_sequence_fields(
        frame,
        area,
        bottom,
        1,
        &general,
        draft.focus,
        compact,
        ui,
        theme,
    );
    let values_y = area.y + if compact { 3 } else { 5 };
    if values_y < bottom {
        render_catalog_section_heading(
            frame,
            Rect::new(area.x, values_y, area.width, 1),
            "VALUES",
            !matches!(
                draft.focus,
                CatalogFormFocus::Name
                    | CatalogFormFocus::Schema
                    | CatalogFormFocus::Owner
                    | CatalogFormFocus::Comment
                    | CatalogFormFocus::OwnedBy
            ),
            theme,
        );
        let values = [
            (CatalogFormFocus::DataType, "Data type", &draft.data_type),
            (CatalogFormFocus::Increment, "Increment", &draft.increment),
            (CatalogFormFocus::StartValue, "Start", &draft.start_value),
            (
                CatalogFormFocus::RestartValue,
                "Restart",
                &draft.restart_value,
            ),
            (CatalogFormFocus::Cache, "Cache", &draft.cache),
        ];
        render_sequence_fields(
            frame,
            area,
            bottom,
            values_y + 1 - area.y,
            &values,
            draft.focus,
            compact,
            ui,
            theme,
        );
        let min_y = if compact
            && matches!(
                draft.focus,
                CatalogFormFocus::MinValue | CatalogFormFocus::MaxValue
            ) {
            bottom.saturating_sub(3)
        } else {
            values_y + if compact { 1 } else { 6 }
        };
        if min_y < bottom {
            render_sequence_bound(
                frame,
                Rect::new(area.x, min_y, area.width, 1),
                "Minimum",
                &draft.min_value,
                CatalogFormFocus::MinValue,
                draft.focus,
                ui,
                theme,
                "No min value",
            );
            let max_y = min_y
                + if draft.min_value.kind == crate::model::catalog_editor::SequenceBoundKind::Custom
                {
                    2
                } else {
                    1
                };
            if max_y < bottom {
                render_sequence_bound(
                    frame,
                    Rect::new(area.x, max_y, area.width, 1),
                    "Maximum",
                    &draft.max_value,
                    CatalogFormFocus::MaxValue,
                    draft.focus,
                    ui,
                    theme,
                    "No max value",
                );
            }
        }
    }
    let ownership_y = if compact {
        if matches!(
            draft.focus,
            CatalogFormFocus::Cycle | CatalogFormFocus::OwnedBy
        ) {
            bottom.saturating_sub(4)
        } else {
            bottom
        }
    } else {
        area.y + 16
    };
    if ownership_y < bottom {
        render_catalog_section_heading(
            frame,
            Rect::new(area.x, ownership_y, area.width, 1),
            "OWNERSHIP",
            matches!(
                draft.focus,
                CatalogFormFocus::Cycle | CatalogFormFocus::OwnedBy
            ),
            theme,
        );
        if ownership_y + 1 < bottom {
            render_catalog_toggle_field(
                frame,
                Rect::new(area.x, ownership_y + 1, area.width, 1),
                "Cycle",
                draft.cycle,
                "On",
                "Off",
                draft.focus == CatalogFormFocus::Cycle,
                true,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::Cycle),
                ui,
                theme,
            );
        }
        if ownership_y + 2 < bottom {
            render_catalog_text_field(
                frame,
                Rect::new(area.x, ownership_y + 2, area.width, 1),
                "Owned by",
                &draft.owned_by,
                draft.focus == CatalogFormFocus::OwnedBy,
                true,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::OwnedBy),
                ui,
                theme,
            );
        }
    }
    render_catalog_actions(frame, area, draft.focus, ui, theme);
    render_owner_picker(frame, area, owner_choices, picker, ui, theme);
}

fn render_materialized_view(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &MaterializedViewDraft,
    query_editable: bool,
    ui: &mut UiState,
    theme: Theme,
    owner_choices: Option<&[crate::db::catalog_mutation::CatalogOwnerChoice]>,
    picker: &crate::model::catalog_editor::OwnerPickerState,
) {
    let bottom = area.bottom().saturating_sub(2);
    let compact = area.height < 16;
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        "GENERAL",
        matches!(
            draft.focus,
            CatalogFormFocus::Name
                | CatalogFormFocus::Schema
                | CatalogFormFocus::Owner
                | CatalogFormFocus::Comment
        ),
        theme,
    );
    let general = [
        (CatalogFormFocus::Name, "Name", &draft.name),
        (CatalogFormFocus::Schema, "Schema", &draft.schema),
        (CatalogFormFocus::Owner, "Owner", &draft.owner),
        (CatalogFormFocus::Comment, "Comment", &draft.comment),
    ];
    render_sequence_fields(
        frame,
        area,
        bottom,
        1,
        &general,
        draft.focus,
        compact,
        ui,
        theme,
    );
    let definition_y = if compact && draft.focus == CatalogFormFocus::Query {
        bottom.saturating_sub(2)
    } else {
        area.y + if compact { 3 } else { 5 }
    };
    if definition_y < bottom {
        render_catalog_section_heading(
            frame,
            Rect::new(area.x, definition_y, area.width, 1),
            "DEFINITION",
            matches!(draft.focus, CatalogFormFocus::Query),
            theme,
        );
        let query_y = if compact && draft.focus == CatalogFormFocus::Query {
            bottom.saturating_sub(1)
        } else {
            definition_y + 1
        };
        if query_y < bottom {
            render_catalog_text_field(
                frame,
                Rect::new(area.x, query_y, area.width, 1),
                "Query",
                &draft.query,
                draft.focus == CatalogFormFocus::Query,
                query_editable,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::Query),
                ui,
                theme,
            );
        }
    }
    let storage_y = if compact && draft.focus == CatalogFormFocus::Query {
        bottom
    } else if compact {
        bottom.saturating_sub(2)
    } else {
        area.y + 8
    };
    if storage_y < bottom {
        render_catalog_section_heading(
            frame,
            Rect::new(area.x, storage_y, area.width, 1),
            "STORAGE",
            matches!(
                draft.focus,
                CatalogFormFocus::Tablespace | CatalogFormFocus::WithData
            ),
            theme,
        );
        if storage_y + 1 < bottom {
            render_catalog_text_field(
                frame,
                Rect::new(area.x, storage_y + 1, area.width, 1),
                "Tablespace",
                &draft.tablespace,
                draft.focus == CatalogFormFocus::Tablespace,
                true,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::Tablespace),
                ui,
                theme,
            );
        }
        if storage_y + 2 < bottom || (compact && draft.focus == CatalogFormFocus::WithData) {
            let toggle_y = if compact && draft.focus == CatalogFormFocus::WithData {
                bottom.saturating_sub(1)
            } else {
                storage_y + 2
            };
            render_catalog_toggle_field(
                frame,
                Rect::new(area.x, toggle_y, area.width, 1),
                "With data",
                draft.with_data,
                "WITH DATA",
                "WITH NO DATA",
                draft.focus == CatalogFormFocus::WithData,
                query_editable,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::WithData),
                ui,
                theme,
            );
        }
    }
    render_catalog_actions(frame, area, draft.focus, ui, theme);
    render_owner_picker(frame, area, owner_choices, picker, ui, theme);
}

fn render_sequence_fields(
    frame: &mut Frame<'_>,
    area: Rect,
    bottom: u16,
    start: u16,
    fields: &[(CatalogFormFocus, &str, &TextInput)],
    focus: CatalogFormFocus,
    compact: bool,
    ui: &mut UiState,
    theme: Theme,
) {
    for (offset, (field, label, input)) in fields.iter().enumerate() {
        if compact && focus != *field {
            continue;
        }
        let y = area.y + start + offset as u16;
        if y < bottom {
            render_catalog_text_field(
                frame,
                Rect::new(area.x, y, area.width, 1),
                label,
                input,
                focus == *field,
                true,
                HitTarget::CatalogEditorFormField(*field),
                ui,
                theme,
            );
        }
    }
}

fn render_sequence_bound(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    bound: &crate::model::catalog_editor::SequenceBoundDraft,
    field: CatalogFormFocus,
    focus: CatalogFormFocus,
    ui: &mut UiState,
    theme: Theme,
    no_limit: &str,
) {
    use crate::model::catalog_editor::SequenceBoundKind;
    let value = match bound.kind {
        SequenceBoundKind::Default => "Default",
        SequenceBoundKind::NoLimit => no_limit,
        SequenceBoundKind::Custom => "Custom",
    };
    render_catalog_choice_field(
        frame,
        area,
        label,
        value,
        focus == field,
        true,
        HitTarget::CatalogEditorFormField(field),
        ui,
        theme,
    );
    if bound.kind == SequenceBoundKind::Custom && area.y + 1 < frame.area().bottom() {
        let input_area = Rect::new(area.x, area.y + 1, area.width, 1);
        render_text_input(
            frame,
            catalog_field_areas(input_area).1,
            "",
            &bound.value,
            Style::new().fg(theme.text).bg(if focus == field {
                theme.selection
            } else {
                theme.surface
            }),
            ui,
        );
    }
}

fn render_catalog_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    focus: CatalogFormFocus,
    ui: &mut UiState,
    theme: Theme,
) {
    let y = area.bottom().saturating_sub(2);
    let review = if area.width < 70 {
        "[ SQL ]"
    } else {
        "[ Review SQL ]"
    };
    let actions = [
        (
            review,
            CatalogFormFocus::Review,
            HitTarget::CatalogEditorReview,
        ),
        (
            "[ Cancel ]",
            CatalogFormFocus::Cancel,
            HitTarget::CatalogEditorCancel,
        ),
    ];
    let mut x = area.x;
    for (label, field, target) in actions {
        let width = (label.len() as u16).min(area.right().saturating_sub(x));
        if width > 0 {
            render_catalog_action(
                frame,
                Rect::new(x, y, width, 1),
                label,
                focus == field,
                true,
                target,
                ui,
                theme,
            );
            x = x.saturating_add(width + 3);
        }
    }
}

fn render_view(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &ViewDraft,
    ui: &mut UiState,
    theme: Theme,
    owner_choices: Option<&[crate::db::catalog_mutation::CatalogOwnerChoice]>,
    picker: &crate::model::catalog_editor::OwnerPickerState,
) {
    let content_bottom = area.bottom().saturating_sub(2);
    let compact = area.height < 16;
    let heading = |frame: &mut Frame<'_>, y: u16, label: &str, selected: bool| {
        render_catalog_section_heading(
            frame,
            Rect::new(area.x, y, area.width, 1),
            label,
            selected,
            theme,
        );
    };
    heading(
        frame,
        area.y,
        "GENERAL",
        matches!(
            draft.focus,
            crate::model::catalog_editor::CatalogFormFocus::Name
                | crate::model::catalog_editor::CatalogFormFocus::Schema
                | crate::model::catalog_editor::CatalogFormFocus::Owner
                | crate::model::catalog_editor::CatalogFormFocus::Comment
        ),
    );
    let general = [
        (CatalogFormFocus::Name, "Name", &draft.name, true),
        (CatalogFormFocus::Schema, "Schema", &draft.schema, true),
        (CatalogFormFocus::Owner, "Owner", &draft.owner, true),
        (CatalogFormFocus::Comment, "Comment", &draft.comment, true),
    ];
    for (offset, (field, label, input, enabled)) in general.into_iter().enumerate() {
        if compact && draft.focus != field {
            continue;
        }
        let y = area.y.saturating_add(1 + offset as u16);
        if y < content_bottom {
            render_catalog_text_field(
                frame,
                Rect::new(area.x, y, area.width, 1),
                label,
                input,
                draft.focus == field,
                enabled,
                HitTarget::CatalogEditorFormField(field),
                ui,
                theme,
            );
        }
    }
    let definition_y = area.y.saturating_add(if compact { 3 } else { 6 });
    if definition_y < content_bottom {
        heading(
            frame,
            definition_y,
            "DEFINITION",
            matches!(
                draft.focus,
                CatalogFormFocus::OutputColumns | CatalogFormFocus::Query
            ),
        );
        if !compact || draft.focus == CatalogFormFocus::OutputColumns {
            render_catalog_text_field(
                frame,
                Rect::new(area.x, definition_y + 1, area.width, 1),
                "Output columns",
                &draft.output_columns,
                draft.focus == CatalogFormFocus::OutputColumns,
                true,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::OutputColumns),
                ui,
                theme,
            );
        }
        let query_y = definition_y.saturating_add(
            if compact && draft.focus != CatalogFormFocus::OutputColumns {
                1
            } else {
                2
            },
        );
        if query_y < content_bottom {
            render_catalog_text_field(
                frame,
                Rect::new(area.x, query_y, area.width, 1),
                "Query",
                &draft.query,
                draft.focus == CatalogFormFocus::Query,
                true,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::Query),
                ui,
                theme,
            );
        }
    }
    let options_y = if compact
        && matches!(
            draft.focus,
            CatalogFormFocus::SecurityBarrier
                | CatalogFormFocus::SecurityInvoker
                | CatalogFormFocus::CheckOption
        ) {
        content_bottom.saturating_sub(2)
    } else {
        area.y.saturating_add(if compact { 5 } else { 9 })
    };
    if options_y < content_bottom {
        heading(
            frame,
            options_y,
            "OPTIONS",
            matches!(
                draft.focus,
                CatalogFormFocus::SecurityBarrier
                    | CatalogFormFocus::SecurityInvoker
                    | CatalogFormFocus::CheckOption
            ),
        );
        let rows = [
            (
                CatalogFormFocus::SecurityBarrier,
                "Security barrier",
                &draft.security_barrier,
                "Default",
                "On",
                "Off",
            ),
            (
                CatalogFormFocus::SecurityInvoker,
                "Security invoker",
                &draft.security_invoker,
                "Default",
                "On",
                "Off",
            ),
        ];
        for (offset, (field, label, option, default, on, off)) in rows.into_iter().enumerate() {
            if compact && draft.focus != field {
                continue;
            }
            let y = options_y.saturating_add(if compact { 1 } else { 1 + offset as u16 });
            if y < content_bottom {
                let available = option.availability.is_available();
                let value = option
                    .value
                    .map_or(default, |value| if value { on } else { off });
                render_catalog_choice_field(
                    frame,
                    Rect::new(area.x, y, area.width, 1),
                    label,
                    value,
                    draft.focus == field,
                    available,
                    HitTarget::CatalogEditorFormField(field),
                    ui,
                    theme,
                );
            }
        }
        let y = options_y.saturating_add(if compact { 1 } else { 3 });
        if (!compact || draft.focus == CatalogFormFocus::CheckOption) && y < content_bottom {
            let available = draft.check_option.availability.is_available();
            let value = draft
                .check_option
                .value
                .as_deref()
                .map_or("None", |value| match value {
                    "LOCAL" => "Local",
                    "CASCADED" => "Cascaded",
                    _ => value,
                });
            render_catalog_choice_field(
                frame,
                Rect::new(area.x, y, area.width, 1),
                "Check option",
                value,
                draft.focus == CatalogFormFocus::CheckOption,
                available,
                HitTarget::CatalogEditorFormField(CatalogFormFocus::CheckOption),
                ui,
                theme,
            );
        }
    }
    let actions_y = area.bottom().saturating_sub(2);
    let actions = if compact {
        [
            (
                "[ SQL ]",
                CatalogFormFocus::Review,
                HitTarget::CatalogEditorReview,
            ),
            (
                "[ Cancel ]",
                CatalogFormFocus::Cancel,
                HitTarget::CatalogEditorCancel,
            ),
        ]
    } else {
        [
            (
                "[ Review SQL ]",
                CatalogFormFocus::Review,
                HitTarget::CatalogEditorReview,
            ),
            (
                "[ Cancel ]",
                CatalogFormFocus::Cancel,
                HitTarget::CatalogEditorCancel,
            ),
        ]
    };
    let mut x = area.x;
    for (label, field, target) in actions {
        let width = (label.len() as u16).min(area.right().saturating_sub(x));
        if width > 0 {
            render_catalog_action(
                frame,
                Rect::new(x, actions_y, width, 1),
                label,
                draft.focus == field,
                true,
                target,
                ui,
                theme,
            );
            x = x.saturating_add(width + 3);
        }
    }
    render_owner_picker(frame, area, owner_choices, picker, ui, theme);
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
    let general_focus = matches!(draft.focus, TableEditorFocus::General(_));
    let columns_focus = matches!(
        draft.focus,
        TableEditorFocus::Columns | TableEditorFocus::ColumnDetails(_)
    );
    // Fall back to the compact layout before the full layout can starve the
    // column list of its selected row.
    let full_list_capacity = area.height.saturating_sub(16);
    let compact = area.height <= 10 || full_list_capacity == 0;
    let general_heading = Rect::new(area.x, area.y, area.width / 2, 1);
    let columns_heading = Rect::new(
        general_heading.right(),
        area.y,
        area.width.saturating_sub(general_heading.width),
        1,
    );
    render_catalog_section_heading(frame, general_heading, "GENERAL", general_focus, theme);
    render_catalog_section_heading(frame, columns_heading, "COLUMNS", columns_focus, theme);
    ui.hit_regions.push(HitRegion {
        area: Rect::new(area.x, area.y, area.width / 2, 1),
        target: HitTarget::CatalogEditorTableField(TableEditorFocus::General(
            TableGeneralField::Name,
        )),
    });
    ui.hit_regions.push(HitRegion {
        area: Rect::new(
            area.x + area.width / 2,
            area.y,
            area.width.saturating_sub(area.width / 2),
            1,
        ),
        target: HitTarget::CatalogEditorTableField(TableEditorFocus::Columns),
    });
    if !compact {
        let general = [
            (
                TableEditorFocus::General(TableGeneralField::Name),
                "Name",
                &draft.name,
            ),
            (
                TableEditorFocus::General(TableGeneralField::Schema),
                "Schema",
                &draft.schema,
            ),
            (
                TableEditorFocus::General(TableGeneralField::Owner),
                "Owner",
                &draft.owner,
            ),
            (
                TableEditorFocus::General(TableGeneralField::Comment),
                "Comment",
                &draft.comment,
            ),
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
                draft.focus,
                ui,
                theme,
            );
        }
    }
    if compact {
        render_table_text_field(
            frame,
            Rect::new(area.x, area.y + 1, area.width, 1),
            TableEditorFocus::General(TableGeneralField::Name),
            "Name",
            &draft.name,
            draft.focus,
            ui,
            theme,
        );
    }
    // Keep the action row and shortcut footer inside the form's available area.
    let content_bottom = area.bottom().saturating_sub(2);
    let columns_y = if compact {
        area.y.saturating_add(2)
    } else {
        area.y.saturating_add(7)
    };
    if columns_y < content_bottom {
        render_catalog_section_heading(
            frame,
            Rect::new(area.x, columns_y, area.width, 1),
            "COLUMNS",
            columns_focus,
            theme,
        );
    }
    let list_start = columns_y.saturating_add(1);
    let list_capacity = content_bottom.saturating_sub(list_start);
    let visible_start = if list_capacity == 0 {
        0
    } else {
        draft
            .selected_column
            .min(draft.columns.len().saturating_sub(1))
            .saturating_sub(usize::from(list_capacity).saturating_sub(1))
    };
    for (index, column) in draft
        .columns
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(usize::from(list_capacity))
    {
        let y = columns_y.saturating_add(1 + (index - visible_start) as u16);
        let active = index == draft.selected_column
            && matches!(
                draft.focus,
                TableEditorFocus::Columns | TableEditorFocus::ColumnDetails(_)
            );
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
    let actions = if compact {
        [
            (
                "[ Add ]",
                TableEditorFocus::Action(TableActionField::AddColumn),
                HitTarget::CatalogEditorAddTableColumn,
            ),
            (
                "[ Remove ]",
                TableEditorFocus::Action(TableActionField::RemoveColumn),
                HitTarget::CatalogEditorRemoveTableColumn,
            ),
            (
                "[ SQL ]",
                TableEditorFocus::Action(TableActionField::Review),
                HitTarget::CatalogEditorReview,
            ),
            (
                "[ Cancel ]",
                TableEditorFocus::Action(TableActionField::Cancel),
                HitTarget::CatalogEditorCancel,
            ),
        ]
    } else {
        [
            (
                "[ Add Column ]",
                TableEditorFocus::Action(TableActionField::AddColumn),
                HitTarget::CatalogEditorAddTableColumn,
            ),
            (
                "[ Remove Column ]",
                TableEditorFocus::Action(TableActionField::RemoveColumn),
                HitTarget::CatalogEditorRemoveTableColumn,
            ),
            (
                "[ Review SQL ]",
                TableEditorFocus::Action(TableActionField::Review),
                HitTarget::CatalogEditorReview,
            ),
            (
                "[ Cancel ]",
                TableEditorFocus::Action(TableActionField::Cancel),
                HitTarget::CatalogEditorCancel,
            ),
        ]
    };
    let mut x = area.x;
    for (label, field, target) in actions {
        let width = label.len() as u16;
        let action_area = Rect::new(
            x,
            area.bottom().saturating_sub(2),
            width.min(area.right().saturating_sub(x)),
            1,
        );
        if action_area.width == 0 {
            continue;
        }
        render_catalog_action(
            frame,
            action_area,
            label,
            draft.focus == field,
            true,
            target,
            ui,
            theme,
        );
        x = x.saturating_add(width + 3);
    }
    let hints = if draft.column_editor.is_some() {
        Vec::new()
    } else {
        match draft.focus {
            TableEditorFocus::Columns => vec![
                ShortcutHint::new("Tab/Shift-Tab/Up/Down", "move focus"),
                ShortcutHint::new("a", "add column below"),
                ShortcutHint::new("e", "edit selected column"),
                ShortcutHint::new("Esc", "close/cancel editor"),
            ],
            TableEditorFocus::ColumnDetails(_) => Vec::new(),
            TableEditorFocus::Action(_) => vec![
                ShortcutHint::new("Enter/Space", "activate"),
                ShortcutHint::new("↑/↓", "move"),
                ShortcutHint::new("Esc", "close/cancel editor"),
            ],
            TableEditorFocus::General(_) => vec![
                ShortcutHint::new("Tab/Shift-Tab/Up/Down", "move focus"),
                ShortcutHint::new("Enter", "preview"),
                ShortcutHint::new("Esc", "cancel"),
            ],
        }
    };
    frame.render_widget(
        Paragraph::new(shortcut_hints::line(
            &hints,
            area.width,
            theme,
            theme.surface,
        ))
        .style(Style::new().bg(theme.surface))
        .alignment(ratatui::layout::Alignment::Center),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

fn render_table_column_details_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    focus: TableEditorFocus,
    session: &crate::model::catalog_editor::TableColumnEditSession,
    ui: &mut UiState,
    theme: Theme,
) {
    let popup = super::centered(
        area,
        72.min(area.width.saturating_sub(4)),
        14.min(area.height),
    );
    frame.render_widget(Clear, popup);
    let block = super::panel_block(" COLUMN DETAILS ", true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 8 || inner.width < 20 {
        return;
    }
    let column = &session.draft;
    let fields = [
        (TableColumnField::Name, "Name", &column.name),
        (TableColumnField::Type, "Type", &column.native_type),
        (
            TableColumnField::Default,
            "Default",
            &column.default_expression,
        ),
        (TableColumnField::Comment, "Comment", &column.comment),
    ];
    for (offset, (field, label, input)) in fields.into_iter().enumerate() {
        render_table_text_field(
            frame,
            Rect::new(
                inner.x,
                inner.y.saturating_add(offset as u16),
                inner.width,
                1,
            ),
            TableEditorFocus::ColumnDetails(field),
            label,
            input,
            focus,
            ui,
            theme,
        );
    }
    let nullable = Rect::new(inner.x, inner.y.saturating_add(4), inner.width, 1);
    let identity = Rect::new(inner.x, inner.y.saturating_add(5), inner.width, 1);
    for (row, field, label, value) in [
        (
            nullable,
            TableColumnField::Nullable,
            "Nullable",
            column.nullable,
        ),
        (
            identity,
            TableColumnField::Identity,
            "Identity",
            column.identity,
        ),
    ] {
        frame.render_widget(
            Paragraph::new(format!(
                "  {label:<12} {}",
                if value { "[x] On" } else { "[ ] Off" }
            ))
            .style(Style::new().fg(theme.text).bg(
                if focus == TableEditorFocus::ColumnDetails(field) {
                    theme.selection
                } else {
                    theme.surface
                },
            )),
            row,
        );
        ui.hit_regions.push(HitRegion {
            area: row,
            target: HitTarget::CatalogEditorTableField(TableEditorFocus::ColumnDetails(field)),
        });
    }
    let footer = Rect::new(inner.x, inner.y.saturating_add(7), inner.width, 1);
    frame.render_widget(
        Paragraph::new(shortcut_hints::line(
            &[
                ShortcutHint::new("Tab/Shift-Tab/Up/Down", "move field"),
                ShortcutHint::new("Enter", "confirm"),
                ShortcutHint::new("Esc", "cancel"),
                ShortcutHint::new("Space", "toggle"),
            ],
            footer.width,
            theme,
            theme.surface,
        ))
        .style(Style::new().bg(theme.surface))
        .alignment(ratatui::layout::Alignment::Center),
        footer,
    );
    let controls = Rect::new(inner.x, inner.y.saturating_add(6), inner.width, 1);
    let confirm_width = 13.min(controls.width);
    let cancel_x = controls.x.saturating_add(confirm_width.saturating_add(3));
    let cancel_width = 12.min(controls.right().saturating_sub(cancel_x));
    for (x, width, label, target) in [
        (
            controls.x,
            confirm_width,
            "[ Confirm ]",
            HitTarget::CatalogEditorColumnDetailsConfirm,
        ),
        (
            cancel_x,
            cancel_width,
            "[ Cancel ]",
            HitTarget::CatalogEditorColumnDetailsCancel,
        ),
    ] {
        if width == 0 {
            continue;
        }
        let row = Rect::new(x, controls.y, width, 1);
        frame.render_widget(
            Paragraph::new(label).style(Style::new().fg(theme.action).bg(theme.surface)),
            row,
        );
        ui.hit_regions.push(HitRegion { area: row, target });
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

const CATALOG_FIELD_LABEL_WIDTH: u16 = 18;

fn catalog_field_areas(area: Rect) -> (Rect, Rect) {
    let label_width = area.width.min(CATALOG_FIELD_LABEL_WIDTH);
    (
        Rect::new(area.x, area.y, label_width, area.height),
        Rect::new(
            area.x.saturating_add(label_width),
            area.y,
            area.width.saturating_sub(label_width),
            area.height,
        ),
    )
}

fn render_catalog_field_label(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    active: bool,
    enabled: bool,
    theme: Theme,
) {
    let active = active && enabled;
    frame.render_widget(
        Paragraph::new(format!(
            "{} {:<15}",
            if active { "›" } else { " " },
            sanitize_terminal_text(label)
        ))
        .style(
            Style::new()
                .fg(if active { theme.action } else { theme.muted })
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_catalog_text_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    input: &TextInput,
    active: bool,
    enabled: bool,
    target: HitTarget,
    ui: &mut UiState,
    theme: Theme,
) {
    let (label_area, value_area) = catalog_field_areas(area);
    render_catalog_field_label(frame, label_area, label, active, enabled, theme);
    if enabled {
        ui.hit_regions.push(HitRegion { area, target });
    }
    if active && enabled {
        render_text_input(
            frame,
            value_area,
            "",
            input,
            Style::new().fg(theme.text).bg(theme.selection),
            ui,
        );
    } else {
        let value = sanitize_terminal_text(input.value());
        let value = if enabled {
            value
        } else {
            format!("{value}  READ ONLY")
        };
        frame.render_widget(
            Paragraph::new(value).style(
                Style::new()
                    .fg(if enabled { theme.text } else { theme.muted })
                    .bg(theme.surface),
            ),
            value_area,
        );
    }
}

#[allow(clippy::too_many_arguments, dead_code)]
fn render_catalog_choice_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    value: &str,
    active: bool,
    enabled: bool,
    target: HitTarget,
    ui: &mut UiState,
    theme: Theme,
) {
    let (label_area, value_area) = catalog_field_areas(area);
    render_catalog_field_label(frame, label_area, label, active, enabled, theme);
    let value = sanitize_terminal_text(value);
    let value = if enabled {
        format!("‹ {value} ›")
    } else {
        format!("{value}  DISABLED")
    };
    frame.render_widget(
        Paragraph::new(value).style(
            Style::new()
                .fg(if enabled { theme.text } else { theme.muted })
                .bg(if active && enabled {
                    theme.selection
                } else {
                    theme.surface
                }),
        ),
        value_area,
    );
    if enabled {
        ui.hit_regions.push(HitRegion { area, target });
    }
}

#[allow(clippy::too_many_arguments, dead_code)]
fn render_catalog_toggle_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    value: bool,
    enabled_label: &str,
    disabled_label: &str,
    active: bool,
    enabled: bool,
    target: HitTarget,
    ui: &mut UiState,
    theme: Theme,
) {
    let (label_area, value_area) = catalog_field_areas(area);
    render_catalog_field_label(frame, label_area, label, active, enabled, theme);
    let state_label = sanitize_terminal_text(if value { enabled_label } else { disabled_label });
    let value = format!("[{}] {state_label}", if value { "x" } else { " " });
    let value = if enabled {
        value
    } else {
        format!("{value}  DISABLED")
    };
    frame.render_widget(
        Paragraph::new(value).style(
            Style::new()
                .fg(if enabled { theme.text } else { theme.muted })
                .bg(if active && enabled {
                    theme.selection
                } else {
                    theme.surface
                }),
        ),
        value_area,
    );
    if enabled {
        ui.hit_regions.push(HitRegion { area, target });
    }
}

#[allow(clippy::too_many_arguments)]
fn render_catalog_action(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    active: bool,
    enabled: bool,
    target: HitTarget,
    ui: &mut UiState,
    theme: Theme,
) {
    frame.render_widget(
        Paragraph::new(sanitize_terminal_text(label)).style(
            Style::new()
                .fg(if !enabled {
                    theme.muted
                } else if active {
                    theme.background
                } else {
                    theme.action
                })
                .bg(if active && enabled {
                    theme.accent
                } else {
                    theme.surface
                }),
        ),
        area,
    );
    if enabled {
        ui.hit_regions.push(HitRegion { area, target });
    }
}

fn render_catalog_section_heading(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    active: bool,
    theme: Theme,
) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            sanitize_terminal_text(label),
            section_style(active, theme),
        )),
        area,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_table_text_field(
    frame: &mut Frame<'_>,
    area: Rect,
    field: TableEditorFocus,
    label: &str,
    input: &crate::model::text_input::TextInput,
    selected: TableEditorFocus,
    ui: &mut UiState,
    theme: Theme,
) {
    render_catalog_text_field(
        frame,
        area,
        label,
        input,
        field == selected,
        true,
        HitTarget::CatalogEditorTableField(field),
        ui,
        theme,
    );
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
    let footer_hints = if editor.is_busy() {
        vec![ShortcutHint::new("Esc", "cancel")]
    } else {
        vec![
            ShortcutHint::new("Enter", "apply"),
            ShortcutHint::new("Esc", "return to form"),
        ]
    };
    let footer = shortcut_hints::line(&footer_hints, area.width, theme, theme.surface);
    let mut lines = vec![
        Line::from(Span::styled(
            "SQL PREVIEW",
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::raw(format!(
            "target: {}",
            sanitize_terminal_text(&target_label(editor))
        )),
    ];
    if editor.is_busy() {
        lines.push(Line::styled("Applying...", theme.warning));
    }
    lines.extend([Line::raw(""), Line::raw(sql), Line::raw("")]);
    if let Some(error) = editor.error.as_deref() {
        lines.push(Line::styled(
            format!("× {}", sanitize_terminal_text(error)),
            Style::new().fg(theme.error),
        ));
    }
    let footer_area = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    let body_area = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), body_area);
    frame.render_widget(Paragraph::new(footer), footer_area);
}

fn target_label(editor: &CatalogEditorState) -> String {
    match &editor.anchor {
        crate::db::catalog_mutation::CatalogMutationAnchor::Profile { profile_id } => {
            format!("profile {profile_id}")
        }
        crate::db::catalog_mutation::CatalogMutationAnchor::Catalog(id) => id.native_path.join("."),
        crate::db::catalog_mutation::CatalogMutationAnchor::Group { schema, group } => {
            let label = match group {
                crate::db::catalog::ObjectGroup::Tables => "Tables",
                crate::db::catalog::ObjectGroup::Views => "Views",
                crate::db::catalog::ObjectGroup::MaterializedViews => "Materialized Views",
                crate::db::catalog::ObjectGroup::Sequences => "Sequences",
                crate::db::catalog::ObjectGroup::Functions => "Functions",
                crate::db::catalog::ObjectGroup::Procedures => "Procedures",
                crate::db::catalog::ObjectGroup::Types => "Types",
                crate::db::catalog::ObjectGroup::Triggers => "Triggers",
            };
            format!("{}. {label}", schema.native_path.join("."))
        }
    }
}
