#![allow(clippy::if_same_then_else)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Alignment, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::App,
    model::{
        profile_manager::{
            ProfileDraft, ProfileField, ProfileManagerPage, ProfileManagerState,
            ProfileMessageLevel, ProfileOperation,
        },
        text_input::TextInput,
    },
    profile::{DatabaseKind, Environment, PasswordStorageChoice, SslMode},
    security::sanitize_terminal_text,
};

use super::{
    HitRegion, HitTarget, ProfileButton, Theme, UiState,
    dialog::{self, DialogButton, DialogTone},
    icons::{IconSet, SelectionIcon},
    loading::ActivityIndicator,
    shortcut_hints::{self, ShortcutHint},
    text_input_horizontal_offset,
};

const FORM_MAX_WIDTH: u16 = 106;
const FORM_MAX_HEIGHT: u16 = 34;

#[derive(Clone, Copy, Debug)]
struct FormLayout {
    header: Rect,
    body: Rect,
    url: Rect,
    url_help: Rect,
    feedback: Rect,
    actions: Rect,
    hint: Rect,
}

pub fn render_profile_manager(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    state: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
    let Some(manager) = app.profile_manager.as_ref() else {
        return;
    };
    match manager.page {
        ProfileManagerPage::Form => render_form(frame, area, app, manager, state, theme, icons),
        ProfileManagerPage::Scope => render_scope(frame, area, manager, state, theme, icons),
        ProfileManagerPage::ConfirmDelete => {
            render_confirmation(frame, area, app, manager, state, theme);
        }
    }
}

fn render_form(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    manager: &ProfileManagerState,
    state: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
    let Some(draft) = manager.draft.as_ref() else {
        return;
    };
    let existing = app
        .profiles
        .iter()
        .any(|profile| profile.id == draft.profile_id());
    let title = if existing {
        " EDIT CONNECTION "
    } else {
        " NEW CONNECTION "
    };
    let panel = manager_panel(area, FORM_MAX_WIDTH, FORM_MAX_HEIGHT);
    let inner = render_panel(frame, panel, title, theme);
    if inner.height < 8 {
        return;
    }
    let layout = form_layout(inner);
    let busy = manager.operation.is_some();
    let status = manager.operation.map(operation_name).map_or_else(
        || format!("{} PROFILE", kind_name(draft.kind)),
        |operation| format!("BUSY // {operation}"),
    );
    frame.render_widget(
        Paragraph::new(status).style(
            Style::new()
                .fg(if busy { theme.warning } else { theme.muted })
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        layout.header,
    );

    let structured_fields = draft
        .visible_fields()
        .iter()
        .copied()
        .filter(|field| !is_button_field(*field) && *field != ProfileField::Url)
        .collect::<Vec<_>>();
    let rows = form_rows(&structured_fields, layout.body.height >= 24);
    let selected_index = rows
        .iter()
        .position(|row| matches!(row, FormRow::Field(field) if *field == manager.selected_field))
        .unwrap_or(0);
    let row_capacity = usize::from(layout.body.height);
    let start = viewport_start(selected_index, rows.len(), row_capacity);
    for (offset, form_row) in rows.into_iter().skip(start).enumerate() {
        let row_y = layout.body.y.saturating_add(offset as u16);
        if row_y >= layout.body.bottom() {
            break;
        }
        match form_row {
            FormRow::Section(section) => render_section(
                frame,
                Rect::new(layout.body.x, row_y, layout.body.width, 1),
                section,
                theme,
            ),
            FormRow::Field(field) => {
                let row = Rect::new(layout.body.x, row_y, layout.body.width, 1);
                render_field(
                    frame,
                    row,
                    draft,
                    field,
                    manager.selected_field,
                    busy,
                    state,
                    theme,
                    icons,
                );
                if !busy && !matches!(field, ProfileField::Kind | ProfileField::DatabaseCategory) {
                    state.hit_regions.push(HitRegion {
                        area: row,
                        target: if is_toggle_field(field) {
                            HitTarget::ProfileToggle(field)
                        } else {
                            HitTarget::ProfileField(field)
                        },
                    });
                }
                if manager.selected_field == field && !busy {
                    render_field_cursor(row, draft, field, state);
                }
            }
        }
    }

    render_field(
        frame,
        layout.url,
        draft,
        ProfileField::Url,
        manager.selected_field,
        busy,
        state,
        theme,
        icons,
    );
    if !busy {
        state.hit_regions.push(HitRegion {
            area: layout.url,
            target: HitTarget::ProfileField(ProfileField::Url),
        });
    }
    if manager.selected_field == ProfileField::Url && !busy {
        render_field_cursor(layout.url, draft, ProfileField::Url, state);
    }
    let help = if draft.url_is_pending() {
        "URL syncing…".to_owned()
    } else if let Some(error) = draft.url_error() {
        format!("Invalid URL · {error}")
    } else if let Some(error) = draft.url_generation_error() {
        if draft.oracle_url_preview().is_some() {
            format!("Preview only · {}", error.message)
        } else if manager.selected_field == ProfileField::Url {
            format!("{} · {}", url_help(draft.kind), error.message)
        } else {
            error.message.clone()
        }
    } else if manager.selected_field == ProfileField::Url {
        url_help(draft.kind).to_owned()
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(help).style(Style::new().fg(theme.muted).bg(theme.surface)),
        layout.url_help,
    );

    render_message_line(frame, manager, layout.feedback, theme);
    render_buttons(
        frame,
        layout.actions,
        &[
            (
                ProfileButton::Test,
                "Test",
                manager.selected_field == ProfileField::Test,
            ),
            (
                ProfileButton::Save,
                "Save",
                manager.selected_field == ProfileField::Save,
            ),
            (
                ProfileButton::Cancel,
                "Cancel",
                manager.selected_field == ProfileField::Cancel,
            ),
        ],
        !busy,
        state,
        theme,
    );
    render_interactive_hint(
        frame,
        layout.hint,
        &form_hints(manager.selected_field, inner.width),
        theme,
        state,
    );
}

fn render_confirmation(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    manager: &ProfileManagerState,
    state: &mut UiState,
    theme: Theme,
) {
    let panel = manager_panel(area, 68, 14);
    let inner = render_panel(frame, panel, " DELETE CONNECTION ", theme);
    let profile = manager
        .delete_profile_id
        .and_then(|profile_id| app.profiles.iter().find(|profile| profile.id == profile_id));
    let name = profile
        .map(|profile| safe_line(&profile.name))
        .unwrap_or_else(|| "selected connection".to_owned());
    let busy = manager.operation.is_some();
    let body = if busy {
        format!("BUSY // DELETING {name}\n\nClosing credentials and persisted metadata...")
    } else {
        format!(
            "Delete {name}?\n\nThis removes its saved metadata and remembered credential. This action cannot be undone."
        )
    };
    let buttons_y = inner.bottom().saturating_sub(2);
    let content = Rect::new(
        inner.x.saturating_add(2),
        inner.y.saturating_add(1),
        inner.width.saturating_sub(4),
        buttons_y.saturating_sub(inner.y.saturating_add(1)),
    );
    let feedback_height = if manager.message.is_some() {
        content.height.min(3)
    } else {
        0
    };
    let body_height = content.height.saturating_sub(feedback_height);
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::new().fg(theme.text).bg(theme.surface))
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true }),
        Rect::new(content.x, content.y, content.width, body_height),
    );
    if feedback_height > 0 {
        render_message_line(
            frame,
            manager,
            Rect::new(
                content.x,
                content.y.saturating_add(body_height),
                content.width,
                feedback_height,
            ),
            theme,
        );
    }
    let actions = dialog::render_actions(
        frame,
        Rect::new(inner.x, buttons_y, inner.width, 1),
        &[
            DialogButton {
                label: "Cancel",
                tone: DialogTone::Normal,
                emphasis: dialog::DialogEmphasis::Secondary,
                enabled: !busy,
            },
            DialogButton {
                label: "Delete permanently",
                tone: DialogTone::Danger,
                emphasis: dialog::DialogEmphasis::Secondary,
                enabled: !busy,
            },
        ],
        usize::from(
            manager.delete_focus == crate::model::profile_manager::ProfileDeleteFocus::Delete,
        ),
        theme,
    );
    for action in actions {
        state.hit_regions.push(HitRegion {
            area: action.area,
            target: if action.index == 0 {
                HitTarget::ProfileButton(ProfileButton::CancelDelete)
            } else {
                HitTarget::ProfileButton(ProfileButton::ConfirmDelete)
            },
        });
    }
    shortcut_hints::render_interactive(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        &[
            ShortcutHint::with_keys(
                "Tab/←/→",
                "switch",
                [KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Enter",
                "activate",
                [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Esc",
                "cancel",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
        ],
        theme,
        theme.surface,
        Alignment::Center,
        state,
    );
}

fn render_scope(
    frame: &mut Frame<'_>,
    area: Rect,
    manager: &ProfileManagerState,
    state: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
    let inner = render_panel(
        frame,
        manager_panel(area, 96, 34),
        " VISIBLE OBJECTS ",
        theme,
    );
    let rows = manager.scope_rows_for_render();
    let loading = manager.scope_discovery_loading();
    let loading_offset = usize::from(loading);
    if loading {
        let elapsed = manager
            .scope_discovery_request
            .map(|(request_id, _)| state.profile_scope_loading_elapsed(request_id))
            .unwrap_or_default();
        frame.render_widget(
            ActivityIndicator {
                mode: state.animation_mode(),
                icons,
                elapsed,
                label: "Loading visible objects",
                detail: Some("discovering databases and schemas"),
                cancellable: false,
                style: Style::new().fg(theme.action).bg(theme.surface),
            },
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new("Waiting for catalog discovery...")
                    .style(Style::new().fg(theme.muted).bg(theme.surface)),
                Rect::new(inner.x, inner.y.saturating_add(1), inner.width, 1),
            );
        }
    }
    let row_capacity = inner
        .height
        .saturating_sub(3)
        .saturating_sub(loading_offset as u16);
    for (offset, row) in rows
        .iter()
        .enumerate()
        .skip(manager.scope_viewport)
        .take(row_capacity as usize)
    {
        let y = inner
            .y
            .saturating_add(loading_offset as u16)
            .saturating_add(offset.saturating_sub(manager.scope_viewport) as u16);
        let active = manager.scope_selected_row.as_deref() == Some(row.id.as_str());
        let (selection_icon, icon_color) = match row.selection {
            crate::model::profile_manager::ScopeSelectionState::Unchecked => {
                (SelectionIcon::Unchecked, theme.muted)
            }
            crate::model::profile_manager::ScopeSelectionState::Partial => {
                (SelectionIcon::Partial, theme.warning)
            }
            crate::model::profile_manager::ScopeSelectionState::Checked => {
                (SelectionIcon::Checked, theme.accent)
            }
        };
        let prefix = if row.database { "" } else { "  " };
        let background = if active {
            theme.selection
        } else {
            theme.surface
        };
        let name_color = if loading {
            theme.muted
        } else if row.unavailable {
            theme.warning
        } else {
            theme.text
        };
        let suffix = if row.read_only { " (mirrored)" } else { "" };
        let line = Line::from(vec![
            Span::styled(prefix, Style::new().fg(name_color).bg(background)),
            Span::styled(
                icons.selection(selection_icon),
                Style::new().fg(icon_color).bg(background),
            ),
            Span::styled(" ", Style::new().fg(name_color).bg(background)),
            Span::styled(row.name.clone(), Style::new().fg(name_color).bg(background)),
            Span::styled(suffix, Style::new().fg(name_color).bg(background)),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(Style::new().fg(name_color).bg(background)),
            Rect::new(inner.x, y, inner.width, 1),
        );
        if !loading {
            state.hit_regions.push(HitRegion {
                area: Rect::new(inner.x, y, inner.width, 1),
                target: HitTarget::ProfileScopeRow(row.id.clone()),
            });
        }
    }
    if !loading && let Some(warning) = manager.scope_warning() {
        frame.render_widget(
            Paragraph::new(sanitize_terminal_text(warning))
                .style(Style::new().fg(theme.warning).bg(theme.surface)),
            Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
        );
    }
    let hint_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    let hints = if loading {
        vec![
            ShortcutHint::new("Enter", "back"),
            ShortcutHint::new("Esc", "back"),
        ]
    } else {
        vec![
            ShortcutHint::new("Space", "toggle"),
            ShortcutHint::new("r", "refresh"),
            ShortcutHint::new("Enter", "back"),
            ShortcutHint::new("Esc", "back"),
        ]
    };
    let hints = hints
        .into_iter()
        .map(|hint| match hint.key.as_ref() {
            "Enter" => ShortcutHint::with_keys(
                hint.key,
                hint.description,
                [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            ),
            "Esc" => ShortcutHint::with_keys(
                hint.key,
                hint.description,
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
            "Space" => ShortcutHint::with_keys(
                hint.key,
                hint.description,
                [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
            ),
            "r" => ShortcutHint::with_keys(
                hint.key,
                hint.description,
                [KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)],
            ),
            _ => hint,
        })
        .collect::<Vec<_>>();
    if loading {
        frame.render_widget(
            Paragraph::new("Loading...   ").style(Style::new().fg(theme.muted).bg(theme.surface)),
            Rect::new(hint_area.x, hint_area.y, 12.min(hint_area.width), 1),
        );
    }
    let hint_area = if loading {
        Rect::new(
            hint_area.x.saturating_add(12),
            hint_area.y,
            hint_area.width.saturating_sub(12),
            hint_area.height,
        )
    } else {
        hint_area
    };
    shortcut_hints::render_interactive(
        frame,
        hint_area,
        &hints,
        theme,
        theme.surface,
        Alignment::Center,
        state,
    );
}

fn render_panel(frame: &mut Frame<'_>, area: Rect, title: &str, theme: Theme) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .title_style(
            Style::new()
                .fg(theme.accent)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.accent))
        .style(Style::new().fg(theme.text).bg(theme.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

#[allow(clippy::too_many_arguments)]
fn render_field(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &ProfileDraft,
    field: ProfileField,
    selected: ProfileField,
    busy: bool,
    state: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
    let active = field == selected;
    let row_style = Style::new().fg(theme.text).bg(theme.surface);
    let indicator = if active { "› " } else { "  " };
    frame.render_widget(Block::new().style(row_style), area);
    let label_width = field_label_width(field, area.width);
    let label_area = Rect::new(area.x, area.y, label_width, 1);
    let value_area = Rect::new(
        area.x.saturating_add(label_width),
        area.y,
        value_width(field, area.width.saturating_sub(label_width)),
        1,
    );
    let value_style = Style::new()
        .fg(if busy { theme.muted } else { theme.text })
        .bg(if active {
            theme.selection
        } else {
            theme.surface
        });
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                indicator,
                Style::new()
                    .fg(if active { theme.accent } else { theme.border })
                    .bg(theme.surface),
            ),
            Span::styled(
                format!(
                    "{:<20}",
                    if field == ProfileField::Password
                        && draft.kind == DatabaseKind::Elasticsearch
                        && draft.elasticsearch_auth == crate::profile::ElasticsearchAuth::ApiKey
                    {
                        "API key (encoded)"
                    } else {
                        field_label(field, draft.kind)
                    }
                ),
                Style::new()
                    .fg(if active { theme.action } else { theme.muted })
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .style(Style::new().fg(theme.text).bg(theme.surface)),
        label_area,
    );
    if field == ProfileField::Kind {
        render_driver_options(frame, value_area, draft.kind, busy, state, theme, icons);
        return;
    }
    if field == ProfileField::DatabaseCategory {
        let mut x = value_area.x;
        for category in crate::db::descriptor::DatabaseCategory::ALL {
            let label = category.label();
            let width = label.cell_width();
            if x + width > value_area.right() {
                break;
            }
            let option_area = Rect::new(x, value_area.y, width, 1);
            let style = if category == draft.category() {
                Style::new()
                    .fg(theme.background)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text).bg(theme.surface)
            };
            frame.render_widget(Paragraph::new(label).style(style), option_area);
            if !busy {
                state.hit_regions.push(HitRegion {
                    area: option_area,
                    target: HitTarget::ProfileCategory(category),
                });
            }
            x += width + 2;
        }
        return;
    }
    if let Some(input) = text_input(draft, field) {
        state.profile_input_targets.push((
            field,
            super::text_selection::InputHitMap {
                area: value_area,
                source_to_display_cells: crate::security::project_editor_line(input.value())
                    .source_to_display_cells,
                horizontal_offset: text_input_horizontal_offset(value_area, "", input),
                prefix_width: 0,
                source_start: 0,
            },
        ));
        state.input_selection_targets.push((
            super::text_selection::InputSelectionTarget::Profile(field),
            super::text_selection::InputHitMap {
                area: value_area,
                source_to_display_cells: crate::security::project_editor_line(input.value())
                    .source_to_display_cells,
                horizontal_offset: text_input_horizontal_offset(value_area, "", input),
                prefix_width: 0,
                source_start: 0,
            },
        ));
    } else if field == ProfileField::Url && (active || draft.oracle_url_preview().is_none()) {
        let display = safe_line(&draft.url_display());
        let projection = crate::security::project_editor_line(&display);
        state.input_selection_targets.push((
            super::text_selection::InputSelectionTarget::ProfileUrl,
            super::text_selection::InputHitMap {
                area: value_area,
                source_to_display_cells: projection.source_to_display_cells,
                horizontal_offset: field_scroll_offset(draft, field, value_area.width) as usize,
                prefix_width: 0,
                source_start: 0,
            },
        ));
    }
    let showing_preview =
        field == ProfileField::Url && !active && draft.oracle_url_preview().is_some();
    let mut value = if showing_preview {
        draft
            .oracle_url_preview()
            .map_or_else(|| field_value(draft, field), |preview| safe_line(&preview))
    } else {
        field_value(draft, field)
    };
    if field == ProfileField::Url && !active {
        value = truncate_display(&value, value_area.width);
    }
    let scroll = if field == ProfileField::Url && !active {
        0
    } else {
        field_scroll_offset(draft, field, value_area.width)
    };
    if field == ProfileField::Url && active {
        let projection = crate::security::project_editor_line(&value);
        let selection = draft.url_selection_range();
        let offset = usize::from(scroll);
        let mut cells = 0;
        let mut spans = Vec::new();
        for character in projection.text.chars() {
            let width = usize::from(character.to_string().cell_width());
            let source = projection
                .source_to_display_cells
                .partition_point(|&boundary| boundary <= cells)
                .saturating_sub(1);
            if cells + width > offset && cells < offset + usize::from(value_area.width) {
                let style = if selection
                    .as_ref()
                    .is_some_and(|range| range.contains(&source))
                {
                    value_style.add_modifier(Modifier::REVERSED)
                } else {
                    value_style
                };
                spans.push(Span::styled(character.to_string(), style));
            }
            cells += width;
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(value_style),
            value_area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(value)
                .style(if showing_preview {
                    value_style.fg(theme.muted)
                } else {
                    value_style
                })
                .scroll((0, scroll)),
            value_area,
        );
    }
}

fn render_driver_options(
    frame: &mut Frame<'_>,
    area: Rect,
    selected: DatabaseKind,
    busy: bool,
    state: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
    let drivers =
        crate::db::descriptor::drivers_in(crate::db::descriptor::descriptor(selected).category)
            .map(|driver| driver.kind)
            .collect::<Vec<_>>();
    let selected_index = drivers
        .iter()
        .position(|kind| *kind == selected)
        .unwrap_or(0);
    let width_of = |kind| icons.database(kind).cell_width() + kind_name(kind).cell_width() + 2;
    let mut start = 0;
    while start < selected_index
        && drivers[start..=selected_index]
            .iter()
            .map(|kind| u32::from(width_of(*kind)))
            .sum::<u32>()
            > u32::from(area.width.saturating_sub(4))
    {
        start += 1;
    }
    let mut x = area.x;
    if start > 0 {
        frame.render_widget(
            Paragraph::new("‹ ").style(Style::new().fg(theme.muted)),
            Rect::new(x, area.y, 2, 1),
        );
        if !busy {
            state.hit_regions.push(HitRegion {
                area: Rect::new(x, area.y, 2, 1),
                target: HitTarget::ProfileDriver(drivers[selected_index.saturating_sub(1)]),
            });
        }
        x += 2;
    }
    for (index, kind) in drivers.iter().copied().enumerate().skip(start) {
        let icon = icons.database(kind);
        let name = kind_name(kind);
        let label = format!("{icon} {name}");
        let width = label.cell_width();
        if width + u16::from(index + 1 < drivers.len()) * 2 > area.right().saturating_sub(x) {
            if area.right().saturating_sub(x) >= 1 {
                frame.render_widget(
                    Paragraph::new("›").style(Style::new().fg(theme.muted)),
                    Rect::new(x, area.y, 1, 1),
                );
                if !busy {
                    state.hit_regions.push(HitRegion {
                        area: Rect::new(x, area.y, 1, 1),
                        target: HitTarget::ProfileDriver(
                            drivers[(selected_index + 1).min(drivers.len() - 1)],
                        ),
                    });
                }
            }
            break;
        }
        let option_area = Rect::new(x, area.y, width, 1);
        let style = if kind == selected {
            Style::new()
                .fg(if busy { theme.muted } else { theme.background })
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
                .fg(if busy { theme.muted } else { theme.text })
                .bg(theme.surface)
        };
        let background = style.bg.unwrap_or(theme.surface);
        let icon_color = if busy {
            theme.muted
        } else {
            icons.database_color(kind)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(icon, Style::new().fg(icon_color).bg(background)),
                Span::styled(" ", style),
                Span::styled(name, style),
            ])),
            option_area,
        );
        if !busy {
            state.hit_regions.push(HitRegion {
                area: option_area,
                target: HitTarget::ProfileDriver(kind),
            });
        }
        x = x.saturating_add(width).saturating_add(1);
    }
}

fn render_field_cursor(area: Rect, draft: &ProfileDraft, field: ProfileField, state: &mut UiState) {
    let value_area = field_value_area(area, field);
    if field == ProfileField::Password {
        return;
    }
    if field == ProfileField::Url {
        let value_width = value_area.width;
        let offset = field_scroll_offset(draft, field, value_width);
        let x = area
            .x
            .saturating_add(value_area.x.saturating_sub(area.x))
            .saturating_add((draft.url_cursor() as u16).saturating_sub(offset))
            .min(value_area.right().saturating_sub(1));
        state.cursor = Some(super::CursorSpec {
            position: Position::new(x, area.y),
            style: super::CursorStyle::Bar,
        });
        return;
    }
    let Some(input) = text_input(draft, field) else {
        return;
    };
    let raw_prefix = input
        .value()
        .chars()
        .take(input.cursor())
        .collect::<String>();
    let cursor_width = safe_line(&raw_prefix).as_str().cell_width();
    let value_width = value_area.width;
    let offset = field_scroll_offset(draft, field, value_width);
    let x = area
        .x
        .saturating_add(value_area.x.saturating_sub(area.x))
        .saturating_add(cursor_width.saturating_sub(offset))
        .min(value_area.right().saturating_sub(1));
    state.cursor = Some(super::CursorSpec {
        position: Position::new(x, area.y),
        style: super::CursorStyle::Bar,
    });
}

fn field_value_area(area: Rect, field: ProfileField) -> Rect {
    let label_width = field_label_width(field, area.width);
    Rect::new(
        area.x.saturating_add(label_width),
        area.y,
        value_width(field, area.width.saturating_sub(label_width)),
        1,
    )
}

fn field_label_width(field: ProfileField, width: u16) -> u16 {
    if field == ProfileField::Url {
        width.min(6)
    } else {
        width.min(22)
    }
}

fn value_width(field: ProfileField, remaining: u16) -> u16 {
    if field == ProfileField::Url {
        remaining
    } else {
        remaining.min(68)
    }
}

fn truncate_display(value: &str, width: u16) -> String {
    if value.cell_width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let limit = width.saturating_sub(1);
    let mut output = String::new();
    let mut used = 0u16;
    for character in value.chars() {
        let character_width = character.to_string().cell_width();
        if used.saturating_add(character_width) > limit {
            break;
        }
        output.push(character);
        used = used.saturating_add(character_width);
    }
    output.push('…');
    output
}

fn field_scroll_offset(draft: &ProfileDraft, field: ProfileField, width: u16) -> u16 {
    if field == ProfileField::Url {
        return (draft.url_cursor() as u16).saturating_sub(width.saturating_sub(1));
    }
    let Some(input) = text_input(draft, field) else {
        return 0;
    };
    if width == 0 {
        return 0;
    }
    let prefix = input
        .value()
        .chars()
        .take(input.cursor())
        .collect::<String>();
    let cursor_width = safe_line(&prefix).as_str().cell_width();
    cursor_width.saturating_sub(width.saturating_sub(1))
}

fn render_message_line(
    frame: &mut Frame<'_>,
    manager: &ProfileManagerState,
    area: Rect,
    theme: Theme,
) {
    let Some(message) = manager.message.as_ref() else {
        return;
    };
    let (marker, color) = profile_message_style(message.level, theme);
    let message = format!("{marker} {}", sanitize_terminal_text(&message.text));
    frame.render_widget(
        Paragraph::new(message)
            .style(Style::new().fg(color).bg(theme.surface))
            .wrap(Wrap { trim: true }),
        Rect::new(area.x, area.y, area.width, area.height),
    );
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        model::profile_manager::{ProfileManagerState, ProfileMessageLevel},
        ui::Theme,
    };

    #[test]
    fn delete_confirmation_shows_error_without_hiding_actions() {
        let mut app = App::new(Vec::new());
        let mut manager = ProfileManagerState {
            page: ProfileManagerPage::ConfirmDelete,
            delete_focus: crate::model::profile_manager::ProfileDeleteFocus::Delete,
            ..ProfileManagerState::default()
        };
        manager.set_message(
            ProfileMessageLevel::Error,
            "Unable to save connection profiles: duplicate key",
        );
        app.profile_manager = Some(manager);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = UiState::default();
        terminal
            .draw(|frame| {
                if let Some(manager) = app.profile_manager.as_ref() {
                    render_confirmation(
                        frame,
                        frame.area(),
                        &app,
                        manager,
                        &mut state,
                        Theme::default(),
                    );
                }
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Unable to save connection profiles"));
        assert!(text.contains("Cancel"));
        assert!(text.contains("Delete permanently"));
        assert_eq!(
            state
                .hit_regions
                .iter()
                .filter(|region| matches!(region.target, HitTarget::ProfileButton(_)))
                .count(),
            2
        );
    }

    #[test]
    fn delete_confirmation_shows_warning_on_compact_terminal() {
        let mut app = App::new(Vec::new());
        let mut manager = ProfileManagerState {
            page: ProfileManagerPage::ConfirmDelete,
            ..ProfileManagerState::default()
        };
        manager.set_message(
            ProfileMessageLevel::Warning,
            "Cancel the running query first",
        );
        app.profile_manager = Some(manager);
        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        let mut state = UiState::default();
        terminal
            .draw(|frame| {
                if let Some(manager) = app.profile_manager.as_ref() {
                    render_confirmation(
                        frame,
                        frame.area(),
                        &app,
                        manager,
                        &mut state,
                        Theme::default(),
                    );
                }
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Cancel the running query"));
        assert_eq!(
            state
                .hit_regions
                .iter()
                .filter(|region| matches!(region.target, HitTarget::ProfileButton(_)))
                .count(),
            2
        );
    }
}

fn profile_message_style(
    level: ProfileMessageLevel,
    theme: Theme,
) -> (&'static str, ratatui::style::Color) {
    match level {
        ProfileMessageLevel::Info => ("·", theme.muted),
        ProfileMessageLevel::Success => ("✓", theme.success),
        ProfileMessageLevel::Warning => ("!", theme.warning),
        ProfileMessageLevel::Error => ("×", theme.error),
    }
}

fn render_buttons(
    frame: &mut Frame<'_>,
    area: Rect,
    buttons: &[(ProfileButton, &str, bool)],
    enabled: bool,
    state: &mut UiState,
    theme: Theme,
) {
    let total_width = buttons
        .iter()
        .map(|(_, label, _)| label.cell_width().saturating_add(4))
        .sum::<u16>()
        .saturating_add(buttons.len().saturating_sub(1) as u16);
    let mut x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);
    for (button, label, selected) in buttons {
        let text = format!("[ {label} ]");
        let width = text.as_str().cell_width();
        if x >= area.right() {
            break;
        }
        let button_area = Rect::new(x, area.y, width.min(area.right().saturating_sub(x)), 1);
        let style = if !enabled {
            Style::new().fg(theme.muted).bg(theme.surface_raised)
        } else if *selected {
            Style::new()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else if *button == ProfileButton::Cancel {
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
                .fg(theme.action)
                .bg(theme.surface_raised)
                .add_modifier(Modifier::BOLD)
        };
        frame.render_widget(Paragraph::new(text).style(style), button_area);
        if enabled {
            state.hit_regions.push(HitRegion {
                area: button_area,
                target: HitTarget::ProfileButton(*button),
            });
        }
        x = x.saturating_add(width).saturating_add(1);
    }
}

fn form_hints(field: ProfileField, width: u16) -> Vec<ShortcutHint<'static>> {
    let ctrl = KeyModifiers::CONTROL;
    let enter_description = match field {
        ProfileField::Test => "test",
        ProfileField::Cancel => "cancel",
        _ => "save",
    };
    if width < 70 {
        let mut hints = vec![
            ShortcutHint::with_keys("^T", "Test", [KeyEvent::new(KeyCode::Char('t'), ctrl)]),
            ShortcutHint::with_keys(
                "Enter",
                enter_description,
                [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
            ),
            ShortcutHint::with_keys(
                "Esc",
                "Close",
                [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
            ),
        ];
        if field == ProfileField::VisibleObjects {
            hints.insert(
                1,
                ShortcutHint::with_keys(
                    "Space",
                    "visible objects",
                    [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
                ),
            );
        }
        return hints;
    }
    let mut hints = vec![
        ShortcutHint::with_keys("Ctrl+T", "test", [KeyEvent::new(KeyCode::Char('t'), ctrl)]),
        ShortcutHint::with_keys(
            "Enter",
            enter_description,
            [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
        ),
        ShortcutHint::with_keys(
            "Esc",
            "cancel",
            [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
        ),
    ];
    if is_text_field(field) {
        hints.extend([
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
        ]);
    } else if field == ProfileField::VisibleObjects {
        hints.push(ShortcutHint::with_keys(
            "Space",
            "visible objects",
            [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
        ));
    } else if is_cycle_field(field) {
        hints.push(ShortcutHint::new("Left/Right", "change"));
        hints.push(ShortcutHint::with_keys(
            "Space",
            "change",
            [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
        ));
    } else if field == ProfileField::Kind {
        hints.push(ShortcutHint::new("Left/Right", "change"));
    } else if is_toggle_field(field) {
        hints.push(ShortcutHint::with_keys(
            "Space",
            "toggle",
            [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
        ));
    } else if !is_button_field(field) {
    }
    hints
}

fn is_text_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::Url
            | ProfileField::Name
            | ProfileField::Host
            | ProfileField::Port
            | ProfileField::User
            | ProfileField::Password
            | ProfileField::Database
            | ProfileField::Schema
            | ProfileField::SqlitePath
            | ProfileField::DuckDbPath
            | ProfileField::CaCertPath
    )
}

fn is_cycle_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::UrlFormat
            | ProfileField::ElasticsearchAuth
            | ProfileField::SslMode
            | ProfileField::Environment
            | ProfileField::PasswordStorage
    )
}

fn render_interactive_hint(
    frame: &mut Frame<'_>,
    area: Rect,
    hints: &[ShortcutHint<'_>],
    theme: Theme,
    state: &mut UiState,
) {
    shortcut_hints::render_interactive(
        frame,
        area,
        hints,
        theme,
        theme.surface,
        Alignment::Left,
        state,
    );
}

fn manager_panel(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.saturating_sub(2).min(max_width);
    let height = area.height.saturating_sub(2).min(max_height);
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

fn form_layout(inner: Rect) -> FormLayout {
    let hint = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    let actions = Rect::new(inner.x, hint.y.saturating_sub(1), inner.width, 1);
    let feedback = Rect::new(inner.x, actions.y.saturating_sub(2), inner.width, 2);
    let url_help = Rect::new(inner.x, feedback.y.saturating_sub(1), inner.width, 1);
    let url = Rect::new(inner.x, url_help.y.saturating_sub(1), inner.width, 1);
    let header = Rect::new(inner.x, inner.y, inner.width, 1);
    let body = Rect::new(
        inner.x,
        header.bottom(),
        inner.width,
        url.y.saturating_sub(header.bottom()),
    );
    FormLayout {
        header,
        body,
        url,
        url_help,
        feedback,
        actions,
        hint,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FormSection {
    Connection,
    Authentication,
    Options,
}

impl FormSection {
    const fn title(self) -> &'static str {
        match self {
            Self::Connection => "CONNECTION",
            Self::Authentication => "AUTHENTICATION",
            Self::Options => "OPTIONS",
        }
    }
}

enum FormRow {
    Section(FormSection),
    Field(ProfileField),
}

fn field_section(field: ProfileField) -> Option<FormSection> {
    match field {
        ProfileField::DatabaseCategory
        | ProfileField::Kind
        | ProfileField::Name
        | ProfileField::Host
        | ProfileField::Port
        | ProfileField::Database
        | ProfileField::Schema
        | ProfileField::VisibleObjects
        | ProfileField::SqliteMemory
        | ProfileField::SqlitePath
        | ProfileField::DuckDbPath => Some(FormSection::Connection),
        ProfileField::ElasticsearchAuth
        | ProfileField::User
        | ProfileField::Password
        | ProfileField::PasswordStorage => Some(FormSection::Authentication),
        ProfileField::CaCertPath
        | ProfileField::SslMode
        | ProfileField::Environment
        | ProfileField::ReadOnly => Some(FormSection::Options),
        _ => None,
    }
}

fn form_rows(fields: &[ProfileField], show_sections: bool) -> Vec<FormRow> {
    let mut rows = Vec::new();
    let mut section = None;
    for &field in fields {
        let next_section = field_section(field);
        if show_sections && next_section != section {
            if let Some(next_section) = next_section {
                rows.push(FormRow::Section(next_section));
            }
            section = next_section;
        }
        rows.push(FormRow::Field(field));
    }
    rows
}

fn render_section(frame: &mut Frame<'_>, area: Rect, section: FormSection, theme: Theme) {
    frame.render_widget(
        Paragraph::new(section.title()).style(
            Style::new()
                .fg(theme.border)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
}

fn viewport_start(selected: usize, total: usize, capacity: usize) -> usize {
    if capacity == 0 || total <= capacity {
        0
    } else {
        selected
            .saturating_add(1)
            .saturating_sub(capacity)
            .min(total - capacity)
    }
}

fn text_input(draft: &ProfileDraft, field: ProfileField) -> Option<&TextInput> {
    match field {
        ProfileField::Name => Some(&draft.name),
        ProfileField::Host => Some(&draft.host),
        ProfileField::Port => Some(&draft.port),
        ProfileField::User => Some(&draft.user),
        ProfileField::Database => Some(&draft.database),
        ProfileField::Schema => Some(&draft.schema),
        ProfileField::SqlitePath => Some(&draft.sqlite_path),
        ProfileField::DuckDbPath => Some(&draft.duckdb_path),
        ProfileField::CaCertPath => Some(&draft.ca_cert_path),
        _ => None,
    }
}

fn field_value(draft: &ProfileDraft, field: ProfileField) -> String {
    match field {
        ProfileField::Kind => kind_name(draft.kind).to_owned(),
        ProfileField::DatabaseCategory => draft.category().label().to_owned(),
        ProfileField::UrlFormat => String::new(),
        ProfileField::Url => safe_line(&draft.url_display()),
        ProfileField::Name => safe_line(draft.name.value()),
        ProfileField::Host => safe_line(draft.host.value()),
        ProfileField::Port => safe_line(draft.port.value()),
        ProfileField::User => safe_line(draft.user.value()),
        ProfileField::Password => {
            if draft.password_len() > 0 {
                "•".repeat(draft.password_len())
            } else if draft.has_stored_credential {
                match draft.password_storage {
                    PasswordStorageChoice::LocalEncrypted => {
                        "Stored locally · encrypted".to_owned()
                    }
                    PasswordStorageChoice::System => "Stored in system credential store".to_owned(),
                }
            } else {
                "Not set".to_owned()
            }
        }
        ProfileField::Database => safe_line(draft.database.value()),
        ProfileField::Schema => safe_line(draft.schema.value()),
        ProfileField::VisibleObjects => format!("{} ›", draft.visible_objects_summary()),
        ProfileField::ElasticsearchAuth => format!(
            "‹ {} ›",
            match draft.elasticsearch_auth {
                crate::profile::ElasticsearchAuth::Basic => "Basic Auth",
                crate::profile::ElasticsearchAuth::ApiKey => "API Key",
            }
        ),
        ProfileField::CaCertPath => safe_line(draft.ca_cert_path.value()),
        ProfileField::SslMode => format!("‹ {} ›", ssl_name(draft.ssl_mode)),
        ProfileField::Environment => format!("‹ {} ›", environment_name(draft.environment)),
        ProfileField::ReadOnly => toggle_value(draft.read_only),
        ProfileField::PasswordStorage => {
            format!("‹ {} ›", password_storage_name(draft.password_storage))
        }
        ProfileField::SqliteMemory => toggle_value(draft.sqlite_memory),
        ProfileField::SqlitePath => safe_line(draft.sqlite_path.value()),
        ProfileField::DuckDbPath => safe_line(draft.duckdb_path.value()),
        ProfileField::Test | ProfileField::Save | ProfileField::Cancel => String::new(),
    }
}

fn url_help(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::Postgres => "Accepts postgres://, postgresql://, and jdbc:postgresql://",
        DatabaseKind::CockroachDb => {
            "Accepts cockroachdb://, postgres://, postgresql://, and jdbc:postgresql://"
        }
        DatabaseKind::MySql => "Accepts mysql:// and jdbc:mysql://",
        DatabaseKind::MariaDb => "Accepts mariadb://",
        DatabaseKind::TiDb => "Accepts tidb:// (default port 4000)",
        DatabaseKind::Oracle => "Accepts jdbc:oracle:thin:@host:port/service",
        DatabaseKind::SqlServer => "Accepts sqlserver://, mssql://, and jdbc:sqlserver://",
        DatabaseKind::Sqlite => "Accepts sqlite://, file:, and jdbc:sqlite:",
        DatabaseKind::DuckDb => "Accepts duckdb:<path> and duckdb::memory:",
        DatabaseKind::Redis => "Accepts redis:// and rediss://",
        DatabaseKind::Valkey => "Accepts valkey:// and valkeys://",
        DatabaseKind::MongoDb => "Accepts mongodb:// and mongodb+srv://",
        DatabaseKind::ClickHouse => "Accepts clickhouse:// (HTTP) and clickhouses:// (HTTPS)",
        DatabaseKind::Elasticsearch => {
            "Accepts elasticsearch:// (HTTP) and elasticsearchs:// (HTTPS)"
        }
        DatabaseKind::Snowflake => {
            "Accepts snowflake://user:token@full-host/database?warehouse=...&role=..."
        }
    }
}

fn field_label(field: ProfileField, kind: DatabaseKind) -> &'static str {
    match field {
        ProfileField::Kind => "Driver",
        ProfileField::DatabaseCategory => "Category",
        ProfileField::UrlFormat => "URL format",
        ProfileField::Url => "URL",
        ProfileField::Name => "Name",
        ProfileField::Host => "Host",
        ProfileField::Port => "Port",
        ProfileField::User => "User",
        ProfileField::Password if kind == DatabaseKind::Snowflake => "Access token",
        ProfileField::Password => "Password",
        ProfileField::Database if kind == DatabaseKind::Oracle => "Service Name",
        ProfileField::Database if kind == DatabaseKind::Elasticsearch => "Namespace (optional)",
        ProfileField::Database => "Database",
        ProfileField::Schema => "Default schema",
        ProfileField::VisibleObjects => "Visible objects",
        ProfileField::ElasticsearchAuth => "Authentication",
        ProfileField::CaCertPath => "CA file (optional)",
        ProfileField::SslMode => "SSL mode",
        ProfileField::Environment => "Environment",
        ProfileField::ReadOnly => "Read only",
        ProfileField::PasswordStorage if kind == DatabaseKind::Snowflake => "Token storage",
        ProfileField::PasswordStorage => "Password storage",
        ProfileField::SqliteMemory => "Memory database",
        ProfileField::SqlitePath => "Path",
        ProfileField::DuckDbPath => "DuckDB file",
        ProfileField::Test => "Test",
        ProfileField::Save => "Save",
        ProfileField::Cancel => "Cancel",
    }
}

fn is_button_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::Test | ProfileField::Save | ProfileField::Cancel
    )
}

fn is_toggle_field(field: ProfileField) -> bool {
    matches!(field, ProfileField::ReadOnly | ProfileField::SqliteMemory)
}

fn toggle_value(enabled: bool) -> String {
    if enabled {
        "[x] On".to_owned()
    } else {
        "[ ] Off".to_owned()
    }
}

fn password_storage_name(storage: PasswordStorageChoice) -> &'static str {
    match storage {
        PasswordStorageChoice::LocalEncrypted => "Local encrypted",
        PasswordStorageChoice::System => {
            if cfg!(target_os = "macos") {
                "macOS login keychain"
            } else {
                "Secret service"
            }
        }
    }
}

fn kind_name(kind: DatabaseKind) -> &'static str {
    crate::db::descriptor::descriptor(kind).display_name
}

fn ssl_name(mode: SslMode) -> &'static str {
    match mode {
        SslMode::Disable => "DISABLE",
        SslMode::Prefer => "PREFER",
        SslMode::Require => "REQUIRE",
        SslMode::VerifyCa => "VERIFY CA",
        SslMode::VerifyFull => "VERIFY FULL",
    }
}

fn environment_name(environment: Environment) -> &'static str {
    match environment {
        Environment::Development => "DEVELOPMENT",
        Environment::Staging => "STAGING",
        Environment::Production => "PRODUCTION",
    }
}

fn operation_name(operation: ProfileOperation) -> &'static str {
    match operation {
        ProfileOperation::Testing => "TESTING CONNECTION",
        ProfileOperation::Saving => "SAVING PROFILE",
        ProfileOperation::SavingAndConnecting => "SAVING & CONNECTING",
        ProfileOperation::Deleting => "DELETING PROFILE",
        ProfileOperation::Connecting => "CONNECTING",
    }
}

fn safe_line(value: &str) -> String {
    sanitize_terminal_text(value)
        .replace('\n', "<LF>")
        .replace('\t', "<TAB>")
}
