use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::App,
    model::{
        profile_manager::{
            DRIVER_ORDER, ProfileDraft, ProfileField, ProfileManagerPage, ProfileManagerState,
            ProfileOperation,
        },
        text_input::TextInput,
    },
    profile::{DatabaseKind, Environment, PasswordStorageChoice, SslMode},
    security::sanitize_terminal_text,
};

use super::{HitRegion, HitTarget, ProfileButton, Theme, UiState, icons::IconSet};

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
        ProfileManagerPage::Scope => render_scope(frame, area, manager, state, theme),
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
    let panel = manager_panel(area, 96, 34);
    let inner = render_panel(frame, panel, title, theme);
    if inner.height < 8 {
        return;
    }
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
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let buttons_y = inner.bottom().saturating_sub(2);
    let message_y = buttons_y.saturating_sub(2);
    let structured_fields = draft
        .visible_fields()
        .iter()
        .copied()
        .filter(|field| !is_button_field(*field) && *field != ProfileField::Url)
        .collect::<Vec<_>>();
    let content_height = usize::from(message_y.saturating_sub(inner.y.saturating_add(1)));
    let example_count = examples(draft.kind).len();
    let fixed_height = 2usize.saturating_add(example_count.min(content_height.saturating_sub(2)));
    let fixed_height = fixed_height.min(content_height);
    let row_capacity = content_height.saturating_sub(fixed_height);
    let selected_index = structured_fields
        .iter()
        .position(|field| *field == manager.selected_field)
        .unwrap_or(0);
    let start = viewport_start(selected_index, structured_fields.len(), row_capacity);
    let mut row_y = inner.y.saturating_add(1);
    for field in structured_fields.into_iter().skip(start).take(row_capacity) {
        if row_y >= message_y {
            break;
        }
        let row = Rect::new(inner.x, row_y, inner.width, 1);
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
        if !busy && field != ProfileField::Kind {
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
            render_field_cursor(frame, row, draft, field);
        }
        row_y = row_y.saturating_add(1);
    }

    let url_y = message_y
        .saturating_sub(u16::try_from(fixed_height).unwrap_or(u16::MAX))
        .saturating_add(1);
    if fixed_height >= 2 {
        let url_row = Rect::new(inner.x, url_y, inner.width, 1);
        render_field(
            frame,
            url_row,
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
                area: url_row,
                target: HitTarget::ProfileField(ProfileField::Url),
            });
        }
        if manager.selected_field == ProfileField::Url && !busy {
            render_field_cursor(frame, url_row, draft, ProfileField::Url);
        }

        for (index, example) in examples(draft.kind)
            .iter()
            .take(fixed_height.saturating_sub(2))
            .enumerate()
        {
            let y = url_y.saturating_add(1 + index as u16);
            let label = if index == 0 { "EXAMPLES" } else { "         " };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("  {label:<20}"),
                        Style::new().fg(theme.muted).bg(theme.surface),
                    ),
                    Span::styled(*example, Style::new().fg(theme.muted).bg(theme.surface)),
                ]))
                .style(Style::new().fg(theme.muted).bg(theme.surface)),
                Rect::new(inner.x, y, inner.width, 1),
            );
        }
    }

    render_message_line(frame, manager, inner, message_y, theme);
    render_buttons(
        frame,
        Rect::new(inner.x, buttons_y, inner.width, 1),
        &[
            (
                ProfileButton::Test,
                "TEST",
                manager.selected_field == ProfileField::Test,
            ),
            (
                ProfileButton::Save,
                "SAVE",
                manager.selected_field == ProfileField::Save,
            ),
            (
                ProfileButton::SaveAndConnect,
                "SAVE & CONNECT",
                manager.selected_field == ProfileField::SaveAndConnect,
            ),
            (
                ProfileButton::Cancel,
                "CANCEL",
                manager.selected_field == ProfileField::Cancel,
            ),
        ],
        !busy,
        state,
        theme,
    );
    render_hint(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        if inner.width < 70 {
            "Tab fields  F5 test  Ctrl-s save  Esc cancel"
        } else {
            "Tab fields   F5 test   Ctrl-s save   Ctrl-Enter connect   Esc cancel"
        },
        theme,
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
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::new().fg(theme.text).bg(theme.surface))
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true }),
        Rect::new(
            inner.x.saturating_add(2),
            inner.y.saturating_add(1),
            inner.width.saturating_sub(4),
            inner.height.saturating_sub(4),
        ),
    );
    let buttons_y = inner.bottom().saturating_sub(2);
    render_buttons(
        frame,
        Rect::new(inner.x, buttons_y, inner.width, 1),
        &[
            (ProfileButton::ConfirmDelete, "DELETE PERMANENTLY", true),
            (ProfileButton::CancelDelete, "CANCEL", false),
        ],
        !busy,
        state,
        theme,
    );
    render_hint(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        "Enter confirm   Esc cancel",
        theme,
    );
}

fn render_scope(
    frame: &mut Frame<'_>,
    area: Rect,
    manager: &ProfileManagerState,
    state: &mut UiState,
    theme: Theme,
) {
    let inner = render_panel(
        frame,
        manager_panel(area, 96, 34),
        " VISIBLE OBJECTS ",
        theme,
    );
    let rows = manager.scope_rows_for_render();
    for (offset, row) in rows
        .iter()
        .enumerate()
        .skip(manager.scope_viewport)
        .take(inner.height.saturating_sub(3) as usize)
    {
        let y = inner
            .y
            .saturating_add(offset.saturating_sub(manager.scope_viewport) as u16);
        let active = manager.scope_selected_row.as_deref() == Some(row.id.as_str());
        let marker = match row.selection {
            crate::model::profile_manager::ScopeSelectionState::Unchecked => "[ ]",
            crate::model::profile_manager::ScopeSelectionState::Partial => "[-]",
            crate::model::profile_manager::ScopeSelectionState::Checked => "[x]",
        };
        let prefix = if row.database { "" } else { "  " };
        let text = format!(
            "{prefix}{marker} {}{}",
            row.name,
            if row.read_only { " (mirrored)" } else { "" }
        );
        frame.render_widget(
            Paragraph::new(text).style(
                Style::new()
                    .fg(if row.unavailable {
                        theme.warning
                    } else {
                        theme.text
                    })
                    .bg(if active {
                        theme.selection
                    } else {
                        theme.surface
                    }),
            ),
            Rect::new(inner.x, y, inner.width, 1),
        );
        state.hit_regions.push(HitRegion {
            area: Rect::new(inner.x, y, inner.width, 1),
            target: HitTarget::ProfileScopeRow(row.id.clone()),
        });
    }
    if let Some(warning) = manager.scope_warning() {
        frame.render_widget(
            Paragraph::new(sanitize_terminal_text(warning))
                .style(Style::new().fg(theme.warning).bg(theme.surface)),
            Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
        );
    }
    render_hint(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        "Space toggle   r refresh   Enter back   Esc back",
        theme,
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
    let row_style = if active {
        Style::new().fg(theme.text).bg(theme.selection)
    } else {
        Style::new().fg(theme.text).bg(theme.surface)
    };
    let indicator = if active { "› " } else { "  " };
    frame.render_widget(Block::new().style(row_style), area);
    let label_width = area.width.min(22);
    let label_area = Rect::new(area.x, area.y, label_width, 1);
    let value_area = Rect::new(
        area.x.saturating_add(label_width),
        area.y,
        area.width.saturating_sub(label_width),
        1,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                indicator,
                Style::new()
                    .fg(if active { theme.accent } else { theme.border })
                    .bg(row_style.bg.unwrap_or(theme.surface)),
            ),
            Span::styled(
                format!("{:<20}", field_label(field)),
                Style::new()
                    .fg(if active { theme.action } else { theme.muted })
                    .bg(row_style.bg.unwrap_or(theme.surface))
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .style(row_style),
        label_area,
    );
    if field == ProfileField::Kind {
        render_driver_options(frame, value_area, draft.kind, busy, state, theme, icons);
        return;
    }
    let value = field_value(draft, field);
    frame.render_widget(
        Paragraph::new(value)
            .style(
                Style::new()
                    .fg(if busy { theme.muted } else { theme.text })
                    .bg(row_style.bg.unwrap_or(theme.surface)),
            )
            .scroll((0, field_scroll_offset(draft, field, value_area.width))),
        value_area,
    );
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
    let mut x = area.x;
    for kind in DRIVER_ORDER {
        let label = format!("{} {}", icons.database(kind), kind_name(kind));
        let width = label.cell_width();
        if width > area.right().saturating_sub(x) {
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
        frame.render_widget(Paragraph::new(label).style(style), option_area);
        if !busy {
            state.hit_regions.push(HitRegion {
                area: option_area,
                target: HitTarget::ProfileDriver(kind),
            });
        }
        x = x.saturating_add(width).saturating_add(1);
    }
}

fn render_field_cursor(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &ProfileDraft,
    field: ProfileField,
) {
    if field == ProfileField::Password {
        return;
    }
    if field == ProfileField::Url {
        let value_width = area.width.saturating_sub(22);
        let offset = field_scroll_offset(draft, field, value_width);
        let x = area
            .x
            .saturating_add(22)
            .saturating_add((draft.url_cursor() as u16).saturating_sub(offset))
            .min(area.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(x, area.y));
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
    let value_width = area.width.saturating_sub(22);
    let offset = field_scroll_offset(draft, field, value_width);
    let x = area
        .x
        .saturating_add(22)
        .saturating_add(cursor_width.saturating_sub(offset))
        .min(area.right().saturating_sub(1));
    frame.set_cursor_position(Position::new(x, area.y));
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
    inner: Rect,
    y: u16,
    theme: Theme,
) {
    let Some(message) = manager.message.as_deref() else {
        return;
    };
    frame.render_widget(
        Paragraph::new(sanitize_terminal_text(message))
            .style(Style::new().fg(theme.warning).bg(theme.surface))
            .wrap(Wrap { trim: true }),
        Rect::new(
            inner.x,
            y,
            inner.width,
            2.min(inner.bottom().saturating_sub(y)),
        ),
    );
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

fn render_hint(frame: &mut Frame<'_>, area: Rect, hint: &str, theme: Theme) {
    frame.render_widget(
        Paragraph::new(hint)
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .alignment(ratatui::layout::Alignment::Center),
        area,
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
        _ => None,
    }
}

fn field_value(draft: &ProfileDraft, field: ProfileField) -> String {
    match field {
        ProfileField::Kind => kind_name(draft.kind).to_owned(),
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
                        "Stored locally (encrypted)".to_owned()
                    }
                    PasswordStorageChoice::System => "Stored in system credential store".to_owned(),
                }
            } else {
                "Not set".to_owned()
            }
        }
        ProfileField::Database => safe_line(draft.database.value()),
        ProfileField::Schema => safe_line(draft.schema.value()),
        ProfileField::VisibleObjects => draft.visible_objects_summary(),
        ProfileField::SslMode => ssl_name(draft.ssl_mode).to_owned(),
        ProfileField::Environment => environment_name(draft.environment).to_owned(),
        ProfileField::ReadOnly => toggle_value(draft.read_only),
        ProfileField::PasswordStorage => password_storage_name(draft.password_storage).to_owned(),
        ProfileField::SqliteMemory => toggle_value(draft.sqlite_memory),
        ProfileField::SqlitePath => safe_line(draft.sqlite_path.value()),
        ProfileField::Test
        | ProfileField::Save
        | ProfileField::SaveAndConnect
        | ProfileField::Cancel => String::new(),
    }
}

fn field_label(field: ProfileField) -> &'static str {
    match field {
        ProfileField::Kind => "DRIVER",
        ProfileField::UrlFormat => "URL FORMAT",
        ProfileField::Url => "URL",
        ProfileField::Name => "NAME",
        ProfileField::Host => "HOST",
        ProfileField::Port => "PORT",
        ProfileField::User => "USER",
        ProfileField::Password => "PASSWORD",
        ProfileField::Database => "DATABASE",
        ProfileField::Schema => "DEFAULT SCHEMA",
        ProfileField::VisibleObjects => "VISIBLE OBJECTS",
        ProfileField::SslMode => "SSL MODE",
        ProfileField::Environment => "ENVIRONMENT",
        ProfileField::ReadOnly => "READ ONLY",
        ProfileField::PasswordStorage => "PASSWORD STORAGE",
        ProfileField::SqliteMemory => "MEMORY DATABASE",
        ProfileField::SqlitePath => "PATH",
        ProfileField::Test => "TEST",
        ProfileField::Save => "SAVE",
        ProfileField::SaveAndConnect => "SAVE & CONNECT",
        ProfileField::Cancel => "CANCEL",
    }
}

fn is_button_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::Test
            | ProfileField::Save
            | ProfileField::SaveAndConnect
            | ProfileField::Cancel
    )
}

fn is_toggle_field(field: ProfileField) -> bool {
    matches!(field, ProfileField::ReadOnly | ProfileField::SqliteMemory)
}

fn toggle_value(enabled: bool) -> String {
    if enabled {
        "[x] ON".to_owned()
    } else {
        "[ ] OFF".to_owned()
    }
}

fn password_storage_name(storage: PasswordStorageChoice) -> &'static str {
    match storage {
        PasswordStorageChoice::LocalEncrypted => "LOCAL ENCRYPTED",
        PasswordStorageChoice::System => {
            if cfg!(target_os = "macos") {
                "macOS LOGIN KEYCHAIN"
            } else {
                "SECRET SERVICE"
            }
        }
    }
}

fn kind_name(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::Postgres => "POSTGRES",
        DatabaseKind::MySql => "MYSQL",
        DatabaseKind::Sqlite => "SQLITE",
    }
}

fn examples(kind: DatabaseKind) -> &'static [&'static str] {
    match kind {
        DatabaseKind::Postgres => &[
            "postgres://user:password@host:5432/database",
            "postgresql://user:password@host:5432/database",
            "jdbc:postgresql://host:5432/database",
        ],
        DatabaseKind::MySql => &[
            "mysql://user:password@host:3306/database",
            "jdbc:mysql://host:3306/database",
        ],
        DatabaseKind::Sqlite => &[
            "sqlite:///path/to/database.db",
            "file:/path/to/database.db",
            "jdbc:sqlite:/path/to/database.db",
        ],
    }
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
