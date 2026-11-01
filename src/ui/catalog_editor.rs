use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use unicode_width::UnicodeWidthStr;

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
    sql::SqlDialect,
};

use super::{
    HitRegion, HitTarget, Theme, UiState,
    dialog_footer::{self, FooterDensity},
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
        CatalogEditorPage::ObjectPicker => picker(frame, inner, editor, theme, icons, ui),
        CatalogEditorPage::Loading => loading(frame, inner, editor, theme, ui),
        CatalogEditorPage::Form => form(frame, inner, app, editor, ui, theme, icons),
        CatalogEditorPage::SqlPreview => preview(frame, inner, app, editor, ui, theme),
    }
    if editor.page == CatalogEditorPage::Form
        && let Some(CatalogDraft::Table(draft)) = editor.draft.as_ref()
        && let Some(session) = draft.column_editor.as_ref()
    {
        render_table_column_details_modal(
            frame,
            inner,
            draft.focus,
            session,
            editor.database_kind,
            ui,
            theme,
        );
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
    ui: &mut UiState,
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
    render_interactive_hints(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        &[
            ShortcutHint::with_keys(
                "j/k · ↑/↓",
                "select",
                [KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Enter",
                "continue",
                [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Esc",
                "close",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
        ],
        theme,
        ui,
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

fn loading(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &CatalogEditorState,
    theme: Theme,
    ui: &mut UiState,
) {
    let operation = editor
        .operation
        .map(|operation| match operation {
            CatalogEditorOperation::LoadingDefinition { .. } => "Loading definition",
            CatalogEditorOperation::Planning { .. } => "Planning mutation",
            CatalogEditorOperation::Applying { .. } => "Applying mutation",
        })
        .unwrap_or("Working");
    let content_area = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let applying = matches!(
        editor.operation,
        Some(CatalogEditorOperation::Applying { .. })
    );
    let message = if applying {
        "The schema mutation is running. Please wait."
    } else {
        "Please wait..."
    };
    let safe_cancel_message = if applying {
        ""
    } else {
        "Esc cancels when the operation is safe to cancel"
    };
    let footer = if applying {
        vec![ShortcutHint::with_keys(
            "Esc",
            "wait for completion",
            [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
        )]
    } else {
        vec![ShortcutHint::with_keys(
            "Esc",
            "cancel",
            [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
        )]
    };
    let mut lines = vec![
        Line::from(Span::styled(
            operation,
            Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
        )),
        Line::raw(message),
    ];
    if !safe_cancel_message.is_empty() {
        lines.push(Line::raw(safe_cancel_message));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        content_area,
    );
    render_interactive_hints(
        frame,
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        &footer,
        theme,
        ui,
    );
}

fn render_interactive_hints(
    frame: &mut Frame<'_>,
    area: Rect,
    hints: &[ShortcutHint<'_>],
    theme: Theme,
    ui: &mut UiState,
) {
    shortcut_hints::render_interactive(
        frame,
        area,
        hints,
        theme,
        theme.surface,
        ratatui::layout::Alignment::Left,
        ui,
    );
}

fn form(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    editor: &CatalogEditorState,
    ui: &mut UiState,
    theme: Theme,
    icons: IconSet,
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
    let target = format!("TARGET  {}", sanitize_terminal_text(&target_label(editor)));
    let header = if matches!(editor.draft, Some(CatalogDraft::Table(_))) {
        Line::from(Span::styled(
            target,
            Style::new().fg(theme.muted).bg(theme.surface),
        ))
    } else {
        Line::from(vec![
            Span::styled(
                format!("{} DETAILS", title.to_uppercase()),
                Style::new()
                    .fg(theme.muted)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {target}"),
                Style::new().fg(theme.muted).bg(theme.surface),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(header), chunks[0]);
    if let Some(CatalogDraft::Database(draft)) = editor.draft.as_ref() {
        render_database(
            frame,
            chunks[1],
            draft,
            ui,
            theme,
            app.catalog_owner_choices(),
            &editor.owner_picker,
        );
    } else if let Some(CatalogDraft::Role(draft)) = editor.draft.as_ref() {
        render_role(frame, chunks[1], draft, ui, theme);
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
        render_table(
            frame,
            chunks[1],
            draft,
            editor.baseline.as_ref(),
            ui,
            theme,
            icons,
        );
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
        let prefix = if editor.outcome_unknown {
            "? OUTCOME UNCONFIRMED: "
        } else {
            "× "
        };
        frame.render_widget(
            Paragraph::new(format!("{prefix}{}", sanitize_terminal_text(error))).style(
                Style::new()
                    .fg(if editor.outcome_unknown {
                        theme.warning
                    } else {
                        theme.error
                    })
                    .bg(theme.surface),
            ),
            chunks[2],
        );
    } else if matches!(editor.draft.as_ref(), Some(CatalogDraft::Table(_))) {
        frame.render_widget(
            Paragraph::new("").style(Style::new().bg(theme.surface)),
            chunks[2],
        );
    } else {
        let mut hints = vec![
            ShortcutHint::with_keys(
                "Tab",
                "next field",
                [KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Shift+Tab",
                "previous field",
                [KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)],
            ),
        ];
        if let Some(
            draft @ (CatalogDraft::View(_)
            | CatalogDraft::MaterializedView(_)
            | CatalogDraft::Sequence(_)
            | CatalogDraft::Database(_)
            | CatalogDraft::Role(_)),
        ) = editor.draft.as_ref()
        {
            match draft
                .focus_kind(editor.mode == crate::db::catalog_mutation::CatalogMutationMode::Create)
            {
                crate::model::catalog_editor::CatalogFormFocusKind::Text => {
                    hints.push(ShortcutHint::new("Type", "edit"));
                    if matches!(draft, CatalogDraft::Database(_) | CatalogDraft::Role(_)) {
                        hints.push(ShortcutHint::with_keys(
                            "Enter",
                            "preview",
                            [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                        ));
                    }
                }
                crate::model::catalog_editor::CatalogFormFocusKind::Choice => {
                    hints.push(ShortcutHint::new("Left/Right", "change"));
                }
                crate::model::catalog_editor::CatalogFormFocusKind::Toggle => {
                    hints.push(ShortcutHint::with_keys(
                        "Space",
                        "toggle",
                        [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
                    ));
                }
                crate::model::catalog_editor::CatalogFormFocusKind::Action => {
                    hints.push(ShortcutHint::with_keys(
                        "Enter",
                        "activate",
                        [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                    ));
                }
                crate::model::catalog_editor::CatalogFormFocusKind::Disabled => {
                    hints.push(ShortcutHint::new("Unavailable", "field"));
                }
                crate::model::catalog_editor::CatalogFormFocusKind::ReadOnly => {
                    hints.push(ShortcutHint::new("Read only", "field"));
                }
            }
        }
        if editor.owner_picker_active() {
            hints.extend([
                ShortcutHint::with_keys(
                    "Up/Down",
                    "role",
                    [KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "Enter",
                    "choose owner",
                    [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "Esc",
                    "close list",
                    [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
                ),
            ]);
        } else {
            if editor.owner_field_focused() && app.catalog_owner_choices().is_some() {
                hints.push(ShortcutHint::with_keys(
                    "Enter",
                    "owner list",
                    [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                ));
            } else {
                hints.push(ShortcutHint::with_keys(
                    "Enter",
                    "preview",
                    [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                ));
            }
            hints.push(ShortcutHint::with_keys(
                "Esc",
                "cancel",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ));
        }
        render_interactive_hints(frame, chunks[2], &hints, theme, ui);
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
        ui.catalog_input_targets.push((
            crate::action::CatalogEditorCursorTarget::SchemaField(index),
            super::text_selection::InputHitMap {
                area: value_area,
                source_to_display_cells: crate::security::project_editor_line(input.value())
                    .source_to_display_cells,
                horizontal_offset: super::text_input_horizontal_offset(value_area, "", input),
                prefix_width: 0,
                source_start: 0,
            },
        ));
        ui.input_selection_targets.push((
            super::text_selection::InputSelectionTarget::Catalog(
                crate::action::CatalogEditorCursorTarget::SchemaField(index),
            ),
            super::text_selection::InputHitMap {
                area: value_area,
                source_to_display_cells: crate::security::project_editor_line(input.value())
                    .source_to_display_cells,
                horizontal_offset: super::text_input_horizontal_offset(value_area, "", input),
                prefix_width: 0,
                source_start: 0,
            },
        ));
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

fn render_role(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &RoleDraft,
    ui: &mut UiState,
    theme: Theme,
) {
    let mut y = area.y;
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "GENERAL",
        false,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Name *",
        &draft.name,
        draft.focus == CatalogFormFocus::Name,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::Name),
        ui,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Comment",
        &draft.comment,
        draft.focus == CatalogFormFocus::Comment,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::Comment),
        ui,
        theme,
    );
    y = y.saturating_add(2);
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "AUTHENTICATION",
        false,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_toggle_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Login",
        draft.login,
        "On",
        "Off",
        draft.focus == CatalogFormFocus::Login,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::Login),
        ui,
        theme,
    );
    y = y.saturating_add(1);
    render_role_password(frame, Rect::new(area.x, y, area.width, 1), draft, ui, theme);
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Valid until",
        &draft.valid_until,
        draft.focus == CatalogFormFocus::ValidUntil,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::ValidUntil),
        ui,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Connection limit",
        &draft.connection_limit,
        draft.focus == CatalogFormFocus::ConnectionLimit,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::ConnectionLimit),
        ui,
        theme,
    );
    y = y.saturating_add(2);
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "PRIVILEGES",
        false,
        theme,
    );
    y = y.saturating_add(1);
    for (label, focus, value) in [
        ("Superuser", CatalogFormFocus::Superuser, draft.superuser),
        (
            "Create database",
            CatalogFormFocus::CreateDb,
            draft.createdb,
        ),
        (
            "Create role",
            CatalogFormFocus::CreateRole,
            draft.createrole,
        ),
        ("Inherit", CatalogFormFocus::Inherit, draft.inherit),
        (
            "Replication",
            CatalogFormFocus::Replication,
            draft.replication,
        ),
        ("Bypass RLS", CatalogFormFocus::BypassRls, draft.bypass_rls),
    ] {
        render_catalog_toggle_field(
            frame,
            Rect::new(area.x, y, area.width, 1),
            label,
            value,
            "On",
            "Off",
            draft.focus == focus,
            true,
            HitTarget::CatalogEditorFormField(focus),
            ui,
            theme,
        );
        y = y.saturating_add(1);
    }
    y = y.saturating_add(1);
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "MEMBERSHIP",
        false,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Member of",
        &draft.memberships,
        draft.focus == CatalogFormFocus::Memberships,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::Memberships),
        ui,
        theme,
    );
    render_catalog_actions(frame, area, draft.focus, ui, theme);
}

fn render_database(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &DatabaseDraft,
    ui: &mut UiState,
    theme: Theme,
    owner_choices: Option<&[crate::db::catalog_mutation::CatalogOwnerChoice]>,
    picker: &crate::model::catalog_editor::OwnerPickerState,
) {
    let mysql = matches!(
        draft.database_kind,
        crate::profile::DatabaseKind::MySql | crate::profile::DatabaseKind::MariaDb
    );
    let mut y = area.y;
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "GENERAL",
        false,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Name *",
        &draft.name,
        draft.focus == CatalogFormFocus::Name,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::Name),
        ui,
        theme,
    );
    y = y.saturating_add(1);
    if !mysql {
        render_catalog_text_field(
            frame,
            Rect::new(area.x, y, area.width, 1),
            "Owner *",
            &draft.owner,
            draft.focus == CatalogFormFocus::Owner,
            true,
            HitTarget::CatalogEditorFormField(CatalogFormFocus::Owner),
            ui,
            theme,
        );
        y = y.saturating_add(1);
    }
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Comment",
        &draft.comment,
        draft.focus == CatalogFormFocus::Comment,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::Comment),
        ui,
        theme,
    );
    if mysql {
        render_catalog_actions(frame, area, draft.focus, ui, theme);
        return;
    }
    y = y.saturating_add(2);
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "LOCALE & ENCODING",
        false,
        theme,
    );
    y = y.saturating_add(1);
    for (label, focus, input) in [
        ("Template *", CatalogFormFocus::Template, &draft.template),
        ("Encoding *", CatalogFormFocus::Encoding, &draft.encoding),
        (
            "Locale provider",
            CatalogFormFocus::LocaleProvider,
            &draft.locale_provider,
        ),
        ("Locale", CatalogFormFocus::Locale, &draft.locale),
        ("Collation", CatalogFormFocus::Collation, &draft.collation),
        ("Ctype", CatalogFormFocus::Ctype, &draft.ctype),
    ] {
        render_catalog_text_field(
            frame,
            Rect::new(area.x, y, area.width, 1),
            label,
            input,
            draft.focus == focus,
            draft.focus_enabled(focus),
            HitTarget::CatalogEditorFormField(focus),
            ui,
            theme,
        );
        y = y.saturating_add(1);
    }
    y = y.saturating_add(1);
    render_catalog_section_heading(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "OPTIONS",
        false,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Tablespace",
        &draft.tablespace,
        draft.focus == CatalogFormFocus::Tablespace,
        draft.focus_enabled(CatalogFormFocus::Tablespace),
        HitTarget::CatalogEditorFormField(CatalogFormFocus::Tablespace),
        ui,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_text_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Connection limit",
        &draft.connection_limit,
        draft.focus == CatalogFormFocus::ConnectionLimit,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::ConnectionLimit),
        ui,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_toggle_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Allow connections",
        draft.allow_connections,
        "On",
        "Off",
        draft.focus == CatalogFormFocus::AllowConnections,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::AllowConnections),
        ui,
        theme,
    );
    y = y.saturating_add(1);
    render_catalog_toggle_field(
        frame,
        Rect::new(area.x, y, area.width, 1),
        "Is template",
        draft.is_template,
        "On",
        "Off",
        draft.focus == CatalogFormFocus::IsTemplate,
        true,
        HitTarget::CatalogEditorFormField(CatalogFormFocus::IsTemplate),
        ui,
        theme,
    );
    render_owner_picker(frame, area, owner_choices, picker, ui, theme);
    render_catalog_actions(frame, area, draft.focus, ui, theme);
}

fn render_role_password(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &RoleDraft,
    ui: &mut UiState,
    theme: Theme,
) {
    let (label_area, value_area) = catalog_field_areas(area);
    let active = draft.focus == CatalogFormFocus::Password;
    render_catalog_field_label(frame, label_area, "Password", active, true, theme);
    let value = if draft.password_value().is_empty() {
        if draft.password.is_some() {
            "Set"
        } else {
            "Unchanged"
        }
        .to_owned()
    } else if active {
        "•".repeat(draft.password_value().chars().count())
    } else {
        "Set".to_owned()
    };
    let value = if active && !draft.password_value().is_empty() {
        format!("{value}▌")
    } else {
        value
    };
    frame.render_widget(
        Paragraph::new(value).style(Style::new().fg(theme.text).bg(if active {
            theme.selection
        } else {
            theme.surface
        })),
        value_area,
    );
    ui.hit_regions.push(HitRegion {
        area,
        target: HitTarget::CatalogEditorFormField(CatalogFormFocus::Password),
    });
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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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
        let value_area = catalog_field_areas(input_area).1;
        super::register_input_selection_target(
            ui,
            super::text_selection::InputSelectionTarget::Catalog(
                crate::action::CatalogEditorCursorTarget::FormField(field),
            ),
            value_area,
            "",
            &bound.value,
            super::text_input_horizontal_offset(value_area, "", &bound.value),
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
    baseline: Option<&crate::db::catalog_mutation::CatalogObjectDefinition>,
    ui: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
    const ACTION_WIDTH: u16 = 4;
    let general_focus = matches!(draft.focus, TableEditorFocus::General(_));
    let columns_focus = matches!(
        draft.focus,
        TableEditorFocus::Columns | TableEditorFocus::ColumnDetails(_)
    );
    // Fall back to the compact layout before the full layout can starve the
    // column list of its selected row.
    let general_field_count = if draft.database_kind == crate::profile::DatabaseKind::ClickHouse {
        if draft.clickhouse_native_ddl.is_some() {
            4
        } else {
            8
        }
    } else {
        4
    };
    let columns_section_offset = general_field_count as u16 + 2;
    let full_list_capacity = area.height.saturating_sub(columns_section_offset + 9);
    let compact = area.height <= 10 || full_list_capacity == 0;
    let general_heading = Rect::new(area.x, area.y, area.width, 1);
    render_catalog_section_heading(frame, general_heading, "GENERAL", general_focus, theme);
    ui.hit_regions.push(HitRegion {
        area: general_heading,
        target: HitTarget::CatalogEditorTableField(TableEditorFocus::General(
            TableGeneralField::Name,
        )),
    });
    if !compact {
        let mut general = vec![
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
        if draft.database_kind == crate::profile::DatabaseKind::ClickHouse
            && draft.clickhouse_native_ddl.is_none()
        {
            general.extend([
                (
                    TableEditorFocus::General(TableGeneralField::ClickHouseEngine),
                    "Engine",
                    &draft.clickhouse_engine,
                ),
                (
                    TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy),
                    "ORDER BY",
                    &draft.clickhouse_order_by,
                ),
                (
                    TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy),
                    "PARTITION BY",
                    &draft.clickhouse_partition_by,
                ),
                (
                    TableEditorFocus::General(TableGeneralField::ClickHouseTtl),
                    "TTL",
                    &draft.clickhouse_ttl,
                ),
            ]);
        }
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
    if compact && general_focus {
        let (field, label, input) = match draft.focus {
            TableEditorFocus::General(TableGeneralField::Schema) => (
                TableEditorFocus::General(TableGeneralField::Schema),
                "Schema",
                &draft.schema,
            ),
            TableEditorFocus::General(TableGeneralField::Owner) => (
                TableEditorFocus::General(TableGeneralField::Owner),
                "Owner",
                &draft.owner,
            ),
            TableEditorFocus::General(TableGeneralField::Comment) => (
                TableEditorFocus::General(TableGeneralField::Comment),
                "Comment",
                &draft.comment,
            ),
            TableEditorFocus::General(TableGeneralField::ClickHouseEngine) => (
                TableEditorFocus::General(TableGeneralField::ClickHouseEngine),
                "Engine",
                &draft.clickhouse_engine,
            ),
            TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy) => (
                TableEditorFocus::General(TableGeneralField::ClickHouseOrderBy),
                "ORDER BY",
                &draft.clickhouse_order_by,
            ),
            TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy) => (
                TableEditorFocus::General(TableGeneralField::ClickHousePartitionBy),
                "PARTITION BY",
                &draft.clickhouse_partition_by,
            ),
            TableEditorFocus::General(TableGeneralField::ClickHouseTtl) => (
                TableEditorFocus::General(TableGeneralField::ClickHouseTtl),
                "TTL",
                &draft.clickhouse_ttl,
            ),
            _ => (
                TableEditorFocus::General(TableGeneralField::Name),
                "Name",
                &draft.name,
            ),
        };
        render_table_text_field(
            frame,
            Rect::new(area.x, area.y + 1, area.width, 1),
            field,
            label,
            input,
            draft.focus,
            ui,
            theme,
        );
    }
    if let TableEditorFocus::Action(TableActionField::Constraint(index)) = draft.focus
        && let Some(constraint) = draft.table_constraints.get(index)
    {
        let fields: Vec<(usize, &str, &TextInput)> = match constraint.kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { .. }
            | crate::db::catalog_mutation::ConstraintDefinitionKind::Unique { .. } => {
                vec![
                    (0, "Name", &constraint.name),
                    (1, "Columns", &constraint.columns),
                ]
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => vec![
                (0, "Name", &constraint.name),
                (1, "Expression", &constraint.expression),
            ],
            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => vec![
                (0, "Name", &constraint.name),
                (1, "Columns", &constraint.columns),
                (2, "Target schema", &constraint.referenced_schema),
                (3, "Target table", &constraint.referenced_relation),
                (4, "Target columns", &constraint.referenced_columns),
                (5, "On update", &constraint.on_update),
                (6, "On delete", &constraint.on_delete),
            ],
        };
        let top = area.y.saturating_add(2);
        let content_bottom = area.y.saturating_add(area.height.saturating_sub(10));
        for (field_index, (selection_index, label, input)) in fields.iter().enumerate() {
            let y = top.saturating_add(field_index as u16);
            if y >= content_bottom {
                break;
            }
            let row = Rect::new(area.x, y, area.width, 1);
            frame.render_widget(
                Paragraph::new(format!(
                    "{label:<16} {}",
                    sanitize_terminal_text(input.value())
                ))
                .style(Style::new().fg(theme.text).bg(
                    if constraint.selected_field == *selection_index {
                        theme.selection
                    } else {
                        theme.surface
                    },
                )),
                row,
            );
            ui.hit_regions.push(HitRegion {
                area: row,
                target: HitTarget::CatalogEditorConstraintField {
                    constraint: index,
                    field: *selection_index,
                },
            });
        }
    }
    let hints = table_shortcut_hints(draft, compact);
    let footer_action_width = dialog_footer::action_width(
        &[
            "[ Add Column ]",
            "[ Add PK ]",
            "[ Add UNIQUE ]",
            "[ Add CHECK ]",
            "[ Add FK ]",
            "[ Edit name ]",
            "[ Edit columns ]",
            "[ Edit expression ]",
            "[ Edit target schema ]",
            "[ Edit target table ]",
            "[ Edit target columns ]",
            "[ Edit on update ]",
            "[ Edit on delete ]",
            "[ Remove Constraint ]",
            "[ Review SQL ]",
            "[ Cancel ]",
        ],
        1,
    );
    let footer = dialog_footer::measure(
        area,
        footer_action_width,
        1,
        if compact {
            FooterDensity::Compact
        } else {
            FooterDensity::Standard
        },
    );
    if let Some(separator) = footer.content_separator {
        frame.render_widget(
            Paragraph::new("─").style(Style::new().fg(theme.border).bg(theme.surface)),
            separator,
        );
    }
    if let Some(separator) = footer.help_separator {
        frame.render_widget(
            Paragraph::new("─").style(Style::new().fg(theme.border).bg(theme.surface)),
            separator,
        );
    }
    let content_bottom = footer
        .content_separator
        .map_or(footer.status_and_actions.y, |separator| separator.y);
    if let Some(ddl) = draft.clickhouse_native_ddl.as_deref() {
        let preview = Rect::new(area.x, area.y.saturating_add(1), area.width, 2);
        frame.render_widget(
            Paragraph::new(format!(
                "Native baseline: {}",
                truncate_cells(
                    sanitize_terminal_text(ddl),
                    usize::from(area.width.saturating_sub(17))
                )
            ))
            .style(Style::new().fg(theme.muted)),
            preview,
        );
    }
    let clickhouse_baseline = match baseline {
        Some(crate::db::catalog_mutation::CatalogObjectDefinition::ClickHouseTable(definition)) => {
            Some(definition)
        }
        _ => None,
    };
    let baseline = match baseline {
        Some(crate::db::catalog_mutation::CatalogObjectDefinition::Table(definition)) => {
            Some(definition)
        }
        _ => None,
    };
    let summary = if draft.clickhouse_native_ddl.is_some() {
        crate::model::catalog_editor::TableChangeSummary {
            properties_changed: clickhouse_baseline.is_some_and(|definition| {
                draft.name.value() != definition.name
                    || match &definition.comment {
                        crate::db::catalog::OptionalMetadata::Unsupported => {
                            !draft.comment.value().is_empty()
                        }
                        crate::db::catalog::OptionalMetadata::Supported(value) => {
                            draft.comment.value() != value.as_deref().unwrap_or_default()
                        }
                    }
            }),
            added_columns: draft
                .columns
                .iter()
                .filter(|column| {
                    matches!(
                        column.state,
                        crate::model::catalog_editor::DraftRowState::Added
                    )
                })
                .count(),
            ..Default::default()
        }
    } else {
        draft.change_summary(baseline)
    };
    let list_bottom = content_bottom;
    let columns_y = if compact {
        area.y.saturating_add(if columns_focus { 1 } else { 2 })
    } else {
        area.y.saturating_add(columns_section_offset)
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
    let header_y = columns_y.saturating_add(1);
    let list_start = columns_y.saturating_add(2);
    let list_capacity = list_bottom.saturating_sub(list_start);
    let (name_width, type_width, nullable_width, comment_width) =
        table_column_widths(draft, area.width.saturating_sub(ACTION_WIDTH + 1));
    let visible_start = if list_capacity == 0 {
        0
    } else {
        draft
            .selected_column
            .min(draft.columns.len().saturating_sub(1))
            .saturating_sub(usize::from(list_capacity).saturating_sub(1))
    };
    let rows = draft
        .columns
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(usize::from(list_capacity))
        .map(|(index, column)| {
            let active = index == draft.selected_column
                && matches!(
                    draft.focus,
                    TableEditorFocus::Columns | TableEditorFocus::ColumnDetails(_)
                );
            let removed = matches!(
                &column.state,
                crate::model::catalog_editor::DraftRowState::Removed { .. }
            );
            let added = matches!(
                &column.state,
                crate::model::catalog_editor::DraftRowState::Added
            );
            let row_background = if removed {
                theme.row_deleted_background
            } else if added {
                theme.row_inserted
            } else if active {
                theme.selection
            } else {
                theme.surface
            };
            let row_style = Style::new()
                .fg(if removed { theme.muted } else { theme.text })
                .bg(row_background)
                .add_modifier(if removed {
                    Modifier::DIM
                } else {
                    Modifier::empty()
                });
            let name = truncate_cells(
                sanitize_terminal_text(column.name.value()).if_empty("<unnamed>"),
                usize::from(name_width),
            );
            let native_type = truncate_cells(
                sanitize_terminal_text(column.native_type.value()),
                usize::from(type_width),
            );
            let nullable = if removed {
                "REMOVED"
            } else if column.nullable {
                "NULL"
            } else {
                "NOT NULL"
            };
            let comment = truncate_cells(
                sanitize_terminal_text(column.comment.value()),
                usize::from(comment_width),
            );
            let state_marker = if removed {
                '-'
            } else if matches!(
                &column.state,
                crate::model::catalog_editor::DraftRowState::Added
            ) {
                '+'
            } else {
                ' '
            };
            let focus_marker = if active { '▌' } else { ' ' };
            let row_number = format!("{focus_marker}{state_marker}{:>1}", index + 1);
            let separator =
                Cell::from("│").style(Style::new().fg(theme.grid_border).bg(row_background));
            let action = if removed {
                icons.catalog_column_restore()
            } else {
                icons.catalog_column_delete()
            };
            let field_style = |field| {
                if column.field_changed_against(field, baseline) {
                    Style::new().bg(theme.row_updated)
                } else {
                    Style::new()
                }
            };
            (
                index,
                Row::new([
                    Cell::from(row_number),
                    separator.clone(),
                    Cell::from(name).style(field_style(TableColumnField::Name)),
                    separator.clone(),
                    Cell::from(native_type).style(field_style(TableColumnField::Type)),
                    separator.clone(),
                    Cell::from(truncate_cells(
                        nullable.to_owned(),
                        usize::from(nullable_width),
                    ))
                    .style(field_style(TableColumnField::Nullable)),
                    separator.clone(),
                    Cell::from(comment).style(field_style(TableColumnField::Comment)),
                    separator.clone(),
                    Cell::from(action).style(
                        Style::new()
                            .fg(if removed { theme.success } else { theme.error })
                            .bg(row_background)
                            .remove_modifier(Modifier::DIM),
                    ),
                ])
                .style(row_style),
            )
        })
        .collect::<Vec<_>>();
    if header_y < list_bottom {
        let header_style = Style::new()
            .fg(theme.grid_header_text)
            .bg(theme.grid_header)
            .add_modifier(Modifier::BOLD);
        let header_separator = Cell::from("│").style(
            Style::new()
                .fg(theme.grid_border)
                .bg(theme.grid_header)
                .add_modifier(Modifier::BOLD),
        );
        let header = Row::new([
            Cell::from("#").style(header_style),
            header_separator.clone(),
            Cell::from("NAME").style(header_style),
            header_separator.clone(),
            Cell::from("TYPE").style(header_style),
            header_separator.clone(),
            Cell::from("NULLABLE").style(header_style),
            header_separator.clone(),
            Cell::from("COMMENT").style(header_style),
            header_separator.clone(),
            Cell::from("ACTION").style(header_style),
        ]);
        let constraints =
            table_column_constraints(name_width, type_width, nullable_width, comment_width);
        let mut table_state = TableState::default();
        if columns_focus && !rows.is_empty() {
            table_state.select(Some(draft.selected_column.saturating_sub(visible_start)));
        }
        frame.render_stateful_widget(
            Table::new(
                rows.iter().enumerate().map(|(offset, (index, row))| {
                    ui.hit_regions.push(HitRegion {
                        area: Rect::new(
                            area.x,
                            list_start + offset as u16,
                            area.width.saturating_sub(ACTION_WIDTH + 1),
                            1,
                        ),
                        target: HitTarget::CatalogEditorTableColumn(*index),
                    });
                    let action = &draft.columns[*index];
                    let action_target = if matches!(
                        action.state,
                        crate::model::catalog_editor::DraftRowState::Removed { .. }
                    ) {
                        HitTarget::CatalogEditorRestoreTableColumnRow(action.row_id)
                    } else if draft
                        .columns
                        .iter()
                        .filter(|column| {
                            !matches!(
                                column.state,
                                crate::model::catalog_editor::DraftRowState::Removed { .. }
                            )
                        })
                        .count()
                        > 1
                    {
                        HitTarget::CatalogEditorRemoveTableColumnRow(action.row_id)
                    } else {
                        HitTarget::CatalogEditorTableColumn(*index)
                    };
                    if !matches!(action_target, HitTarget::CatalogEditorTableColumn(_)) {
                        ui.hit_regions.push(HitRegion {
                            area: Rect::new(
                                area.right().saturating_sub(ACTION_WIDTH),
                                list_start + offset as u16,
                                ACTION_WIDTH,
                                1,
                            ),
                            target: action_target,
                        });
                    }
                    row.clone()
                }),
                constraints,
            )
            .header(header)
            .column_spacing(0)
            .row_highlight_style(Style::new())
            .highlight_symbol(""),
            Rect::new(
                area.x,
                header_y,
                area.width,
                list_capacity.saturating_add(1),
            ),
            &mut table_state,
        );
    }
    let summary_line = table_change_summary_line(summary, theme);
    frame.render_widget(
        Paragraph::new(summary_line).style(Style::new().bg(theme.surface)),
        footer.status_area(footer_action_width, 3),
    );
    let mut bottom_actions = vec![
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
        (
            "[ Add Column ]",
            TableEditorFocus::Action(TableActionField::AddColumn),
            HitTarget::CatalogEditorAddTableColumn,
        ),
        (
            "[ Add PK ]",
            TableEditorFocus::Action(TableActionField::AddConstraintPrimaryKey),
            HitTarget::CatalogEditorAddTableConstraintPrimaryKey,
        ),
        (
            "[ Add UNIQUE ]",
            TableEditorFocus::Action(TableActionField::AddConstraintUnique),
            HitTarget::CatalogEditorAddTableConstraintUnique,
        ),
        (
            "[ Add CHECK ]",
            TableEditorFocus::Action(TableActionField::AddConstraintCheck),
            HitTarget::CatalogEditorAddTableConstraintCheck,
        ),
        (
            "[ Add FK ]",
            TableEditorFocus::Action(TableActionField::AddConstraintForeignKey),
            HitTarget::CatalogEditorAddTableConstraintForeignKey,
        ),
    ];
    if !draft.table_constraints.is_empty() {
        let constraint_index = match draft.focus {
            TableEditorFocus::Action(
                TableActionField::Constraint(index) | TableActionField::RemoveConstraint(index),
            ) => index.min(draft.table_constraints.len() - 1),
            _ => 0,
        };
        bottom_actions.push((
            "[ Remove Constraint ]",
            TableEditorFocus::Action(TableActionField::RemoveConstraint(constraint_index)),
            HitTarget::CatalogEditorRemoveTableConstraint(constraint_index),
        ));
        let constraint = &draft.table_constraints[constraint_index];
        let field_labels: Vec<(usize, &str)> = match constraint.kind {
            crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { .. }
            | crate::db::catalog_mutation::ConstraintDefinitionKind::Unique { .. } => {
                vec![(0, "Edit name"), (1, "Edit columns")]
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {
                vec![(0, "Edit name"), (1, "Edit expression")]
            }
            crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { .. } => vec![
                (0, "Edit name"),
                (1, "Edit columns"),
                (2, "Edit target schema"),
                (3, "Edit target table"),
                (4, "Edit target columns"),
                (5, "Edit on update"),
                (6, "Edit on delete"),
            ],
        };
        for (field, label) in field_labels {
            bottom_actions.push((
                label,
                TableEditorFocus::Action(TableActionField::Constraint(constraint_index)),
                HitTarget::CatalogEditorConstraintField {
                    constraint: constraint_index,
                    field,
                },
            ));
        }
    }
    let action_width = footer_action_width;
    let mut x = footer.action_area(action_width, 1).x;
    let action_right = footer.status_and_actions.right();
    for (label, field, target) in bottom_actions {
        let width = label.width() as u16;
        if x.saturating_add(width) > action_right {
            break;
        }
        let action_area = Rect::new(x, footer.status_and_actions.y, width, 1);
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
    render_interactive_hints(frame, footer.help, &hints, theme, ui);
}

fn table_change_summary_line(
    summary: crate::model::catalog_editor::TableChangeSummary,
    theme: Theme,
) -> Line<'static> {
    if !summary.is_dirty() {
        return Line::from(Span::styled(
            "No changes",
            Style::new().fg(theme.muted).bg(theme.surface),
        ));
    }
    let pending = summary.added_columns
        + summary.modified_columns
        + summary.removed_columns
        + usize::from(summary.properties_changed)
        + usize::from(summary.column_order_changed);
    let label = |count: usize, text: &'static str, color| {
        Span::styled(
            format!("{count} {text}"),
            Style::new()
                .fg(if count > 0 { color } else { theme.muted })
                .bg(theme.surface),
        )
    };
    Line::from(vec![
        Span::styled(
            format!("{pending} pending changes"),
            Style::new()
                .fg(theme.text)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::new().fg(theme.muted).bg(theme.surface)),
        label(summary.added_columns, "added", theme.success),
        Span::styled(" · ", Style::new().fg(theme.muted).bg(theme.surface)),
        label(summary.modified_columns, "modified", theme.action),
        Span::styled(" · ", Style::new().fg(theme.muted).bg(theme.surface)),
        label(summary.removed_columns, "removed", theme.error),
    ])
}

fn table_shortcut_hints(draft: &TableDraft, compact: bool) -> Vec<ShortcutHint<'static>> {
    if draft.column_editor.is_some() {
        return Vec::new();
    }
    match draft.focus {
        TableEditorFocus::Columns if compact => vec![
            ShortcutHint::with_keys(
                "Esc",
                "cancel",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
            ShortcutHint::new("A/a", "add"),
            ShortcutHint::new("J/K", "reorder"),
            ShortcutHint::with_keys(
                "dd",
                "delete",
                [
                    KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                    KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                ],
            ),
        ],
        TableEditorFocus::Columns => vec![
            ShortcutHint::new("j/k · Up/Down", "move row"),
            ShortcutHint::with_keys(
                "Tab",
                "next focus",
                [KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Shift+Tab",
                "previous focus",
                [KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "a",
                "add below",
                [KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "A",
                "add above",
                [KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE)],
            ),
            ShortcutHint::new("J/K", "reorder"),
            ShortcutHint::with_keys(
                "e",
                "edit column",
                [KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "dd",
                "delete column",
                [
                    KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                    KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                ],
            ),
            ShortcutHint::with_keys(
                "r",
                "restore",
                [KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Esc",
                "close/cancel editor",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
        ],
        TableEditorFocus::ColumnDetails(_) => Vec::new(),
        TableEditorFocus::Action(_) => vec![
            ShortcutHint::with_keys(
                "Enter",
                "activate",
                [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            ),
            ShortcutHint::new("↑/↓", "move"),
            ShortcutHint::with_keys(
                "Esc",
                "close/cancel editor",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
        ],
        TableEditorFocus::General(_) => vec![
            ShortcutHint::with_keys(
                "Esc",
                "cancel",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
            ShortcutHint::new("Tab/Shift-Tab/Up/Down", "move focus"),
            ShortcutHint::with_keys(
                "Enter",
                "preview",
                [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            ),
        ],
    }
}

fn table_column_widths(draft: &TableDraft, width: u16) -> (u16, u16, u16, u16) {
    let mut ideal = [
        "NAME".width(),
        "TYPE".width(),
        "NULLABLE".width(),
        "COMMENT".width(),
    ];
    for column in &draft.columns {
        let values = [
            sanitize_terminal_text(column.name.value())
                .if_empty("<unnamed>")
                .width(),
            sanitize_terminal_text(column.native_type.value()).width(),
            if matches!(
                column.state,
                crate::model::catalog_editor::DraftRowState::Removed { .. }
            ) {
                "REMOVED".width()
            } else if column.nullable {
                "NULL".width()
            } else {
                "NOT NULL".width()
            },
            sanitize_terminal_text(column.comment.value()).width(),
        ];
        for (target, measured) in ideal.iter_mut().zip(values) {
            *target = (*target).max(measured);
        }
    }
    for column in &mut ideal[..3] {
        *column = (*column + 2).clamp(6, 40);
    }
    ideal[3] = 9;
    let minimum = [6usize, 6, 10];
    let budget = usize::from(width.saturating_sub(7));
    while ideal[..3].iter().sum::<usize>() + ideal[3] > budget {
        let Some(index) = (0..3)
            .filter(|&index| ideal[index] > minimum[index])
            .max_by_key(|&index| ideal[index] - minimum[index])
        else {
            break;
        };
        ideal[index] -= 1;
    }
    if ideal[..3].iter().sum::<usize>() + ideal[3] > budget {
        let mut remaining = budget;
        for column in &mut ideal[..3] {
            let value = (*column).min(remaining);
            *column = value;
            remaining = remaining.saturating_sub(value);
        }
        ideal[3] = remaining;
    } else {
        ideal[3] = budget.saturating_sub(ideal[..3].iter().sum());
    }
    (
        ideal[0] as u16,
        ideal[1] as u16,
        ideal[2] as u16,
        ideal[3] as u16,
    )
}

fn table_column_constraints(
    name_width: u16,
    type_width: u16,
    nullable_width: u16,
    comment_width: u16,
) -> [Constraint; 11] {
    [
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(name_width),
        Constraint::Length(1),
        Constraint::Length(type_width),
        Constraint::Length(1),
        Constraint::Length(nullable_width),
        Constraint::Length(1),
        Constraint::Length(comment_width),
        Constraint::Length(1),
        Constraint::Length(4),
    ]
}

fn truncate_cells(value: String, width: usize) -> String {
    if value.width() <= width {
        return value;
    }
    let mut output = String::new();
    let mut used = 0;
    for character in value.chars() {
        let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width + 1 > width {
            break;
        }
        output.push(character);
        used += character_width;
    }
    if width > 0 {
        output.push('…');
    }
    output
}

fn render_table_column_details_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    focus: TableEditorFocus,
    session: &crate::model::catalog_editor::TableColumnEditSession,
    database_kind: Option<crate::profile::DatabaseKind>,
    ui: &mut UiState,
    theme: Theme,
) {
    let popup = super::centered(
        area,
        72.min(area.width.saturating_sub(4)),
        16.min(area.height),
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
    if let Some(error) = session.error.as_deref() {
        frame.render_widget(
            Paragraph::new(format!("x {}", sanitize_terminal_text(error)))
                .wrap(Wrap { trim: true })
                .style(Style::new().fg(theme.error).bg(theme.surface)),
            Rect::new(inner.x, inner.y.saturating_add(6), inner.width, 2),
        );
    }
    if focus == TableEditorFocus::ColumnDetails(TableColumnField::Type)
        && let Some(database_kind) = database_kind
    {
        let hint = match database_kind {
            crate::profile::DatabaseKind::Oracle => {
                "Oracle: VARCHAR2(n CHAR), NUMBER(p,s), DATE, TIMESTAMP, CLOB"
            }
            _ => "Enter the native type for the selected database",
        };
        frame.render_widget(
            Paragraph::new(hint).style(Style::new().fg(theme.muted).bg(theme.surface)),
            Rect::new(inner.x, inner.y.saturating_add(7), inner.width, 1),
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
    let footer = Rect::new(inner.x, inner.y.saturating_add(8), inner.width, 1);
    render_interactive_hints(
        frame,
        footer,
        &[
            ShortcutHint::new("Tab/Shift-Tab/Up/Down", "move field"),
            ShortcutHint::with_keys(
                "Enter",
                "confirm",
                [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Esc",
                "cancel",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Space",
                "toggle",
                [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
            ),
        ],
        theme,
        ui,
    );
    let controls = Rect::new(inner.x, inner.y.saturating_add(7), inner.width, 1);
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
        let cursor_target = match &ui.hit_regions.last().map(|region| &region.target) {
            Some(HitTarget::CatalogEditorFormField(field)) => {
                Some(crate::action::CatalogEditorCursorTarget::FormField(*field))
            }
            Some(HitTarget::CatalogEditorTableField(field)) => {
                Some(crate::action::CatalogEditorCursorTarget::TableField(*field))
            }
            _ => None,
        };
        if let Some(cursor_target) = cursor_target {
            ui.catalog_input_targets.push((
                cursor_target.clone(),
                super::text_selection::InputHitMap {
                    area: value_area,
                    source_to_display_cells: crate::security::project_editor_line(input.value())
                        .source_to_display_cells,
                    horizontal_offset: super::text_input_horizontal_offset(value_area, "", input),
                    prefix_width: 0,
                    source_start: 0,
                },
            ));
            ui.input_selection_targets.push((
                super::text_selection::InputSelectionTarget::Catalog(cursor_target),
                super::text_selection::InputHitMap {
                    area: value_area,
                    source_to_display_cells: crate::security::project_editor_line(input.value())
                        .source_to_display_cells,
                    horizontal_offset: super::text_input_horizontal_offset(value_area, "", input),
                    prefix_width: 0,
                    source_start: 0,
                },
            ));
        }
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

fn preview(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    editor: &CatalogEditorState,
    ui: &mut UiState,
    theme: Theme,
) {
    let sql = editor
        .plan
        .as_ref()
        .map(|plan| sanitize_terminal_text(&plan.sql()))
        .unwrap_or_else(|| "No mutation plan is available yet.".into());
    let footer_hints = if editor.is_busy() {
        vec![ShortcutHint::new("Esc", "wait for completion")]
    } else {
        vec![
            ShortcutHint::new("Enter", "apply"),
            ShortcutHint::new("Esc", "return to form"),
        ]
    };
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
    if matches!(
        editor.baseline.as_ref(),
        Some(crate::db::catalog_mutation::CatalogObjectDefinition::Table(
            _
        ))
    ) && let Some(summary) = editor
        .clickhouse_table_change_summary()
        .or_else(|| editor.table_change_summary())
    {
        lines.push(Line::raw(format!(
            "changes: {} added, {} modified, {} removed",
            summary.added_columns, summary.modified_columns, summary.removed_columns
        )));
    }
    if let Some(plan) = editor.plan.as_ref() {
        if plan.destructive {
            lines.push(Line::styled("WARNING: destructive mutation", theme.warning));
        }
        lines.extend(
            plan.warnings
                .iter()
                .map(|warning| Line::styled(sanitize_terminal_text(warning), theme.warning)),
        );
    }
    if editor.is_busy() {
        lines.push(Line::styled("Applying changes...", theme.warning));
    }
    let dialect = editor
        .plan
        .as_ref()
        .and_then(|plan| {
            app.profiles
                .iter()
                .find(|profile| profile.id == plan.request.connection.profile_id)
        })
        .map(|profile| match profile.kind {
            crate::profile::DatabaseKind::Postgres | crate::profile::DatabaseKind::CockroachDb => {
                SqlDialect::Postgres
            }
            crate::profile::DatabaseKind::MySql
            | crate::profile::DatabaseKind::MariaDb
            | crate::profile::DatabaseKind::TiDb => SqlDialect::MySql,
            crate::profile::DatabaseKind::Oracle => SqlDialect::Generic,
            crate::profile::DatabaseKind::Sqlite => SqlDialect::Sqlite,
            crate::profile::DatabaseKind::DuckDb => SqlDialect::DuckDb,
            crate::profile::DatabaseKind::SqlServer => SqlDialect::SqlServer,
            crate::profile::DatabaseKind::Redis
            | crate::profile::DatabaseKind::Valkey
            | crate::profile::DatabaseKind::MongoDb
            | crate::profile::DatabaseKind::Elasticsearch => SqlDialect::Generic,
            crate::profile::DatabaseKind::Snowflake => SqlDialect::Snowflake,
            crate::profile::DatabaseKind::ClickHouse => SqlDialect::ClickHouse,
        })
        .unwrap_or(SqlDialect::Generic);
    lines.push(Line::raw(""));
    lines.extend(crate::ui::sql_preview::lines(
        &sql,
        dialect,
        area.width.saturating_sub(4) as usize,
        theme,
    ));
    lines.push(Line::raw(""));
    if let Some(error) = editor.error.as_deref() {
        for (index, line) in sanitize_terminal_text(error).lines().enumerate() {
            lines.push(Line::styled(
                if index == 0 {
                    format!("× {line}")
                } else {
                    format!("  {line}")
                },
                Style::new().fg(theme.error),
            ));
        }
    }
    let footer_area = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    let body_area = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let max_scroll = lines.len().saturating_sub(usize::from(body_area.height));
    let scroll = editor.preview_scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll.min(usize::from(u16::MAX)) as u16, 0)),
        body_area,
    );
    render_interactive_hints(frame, footer_area, &footer_hints, theme, ui);
}

fn target_label(editor: &CatalogEditorState) -> String {
    match &editor.anchor {
        crate::db::catalog_mutation::CatalogMutationAnchor::Profile { profile_id } => {
            format!("profile {profile_id}")
        }
        crate::db::catalog_mutation::CatalogMutationAnchor::Catalog(id) => {
            let visible = match id.kind {
                crate::db::catalog::CatalogKind::Table
                | crate::db::catalog::CatalogKind::View
                | crate::db::catalog::CatalogKind::MaterializedView => id.native_path.len().min(3),
                crate::db::catalog::CatalogKind::Column => id.native_path.len().min(4),
                _ => id.native_path.len(),
            };
            id.native_path
                .iter()
                .take(visible)
                .cloned()
                .collect::<Vec<_>>()
                .join(" / ")
        }
        crate::db::catalog_mutation::CatalogMutationAnchor::Group { schema, group } => {
            let label = match group {
                crate::db::catalog::ObjectGroup::Tables => "Tables",
                crate::db::catalog::ObjectGroup::Dictionaries => "Dictionaries",
                crate::db::catalog::ObjectGroup::UserDefinedFunctions => "User defined functions",
                crate::db::catalog::ObjectGroup::Views => "Views",
                crate::db::catalog::ObjectGroup::MaterializedViews => "Materialized Views",
                crate::db::catalog::ObjectGroup::Sequences => "Sequences",
                crate::db::catalog::ObjectGroup::Functions => "Functions",
                crate::db::catalog::ObjectGroup::Procedures => "Procedures",
                crate::db::catalog::ObjectGroup::Types => "Types",
                crate::db::catalog::ObjectGroup::Triggers => "Triggers",
            };
            let path = schema
                .native_path
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>();
            format!("{} / {label}", path.join(" / "))
        }
        crate::db::catalog_mutation::CatalogMutationAnchor::Principal(entry) => {
            format!("{} @ {}", entry.name, entry.id.profile_id)
        }
    }
}
