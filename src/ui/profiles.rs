use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::App,
    model::{
        profile_manager::{
            ProfileDraft, ProfileField, ProfileManagerPage, ProfileManagerState, ProfileOperation,
        },
        text_input::TextInput,
    },
    profile::{ConnectionProfile, DatabaseKind, Environment, SslMode},
    security::sanitize_terminal_text,
};

use super::{HitRegion, HitTarget, ProfileButton, Theme, UiState};

pub fn render_profile_manager(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    let Some(manager) = app.profile_manager.as_ref() else {
        return;
    };
    match manager.page {
        ProfileManagerPage::List => render_list(frame, area, app, manager, state, theme),
        ProfileManagerPage::Form => render_form(frame, area, app, manager, state, theme),
        ProfileManagerPage::ConfirmDelete => {
            render_confirmation(frame, area, app, manager, state, theme);
        }
    }
}

fn render_list(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    manager: &ProfileManagerState,
    state: &mut UiState,
    theme: Theme,
) {
    let panel = manager_panel(area, 108, 32);
    let title = format!(" CONNECTIONS // {} PROFILES ", app.profiles.len());
    let inner = render_panel(frame, panel, &title, theme);
    if inner.height < 5 {
        return;
    }

    let header = Rect::new(inner.x, inner.y, inner.width, 1);
    let compact = inner.width < 72;
    if compact {
        frame.render_widget(
            Paragraph::new("PROFILE / DRIVER / ENDPOINT / POLICY")
                .style(Style::new().fg(theme.muted).bg(theme.surface)),
            header,
        );
    } else {
        render_profile_columns(
            frame,
            header,
            ["", "PROFILE", "DRIVER", "ENDPOINT", "ENVIRONMENT", "ACCESS"],
            Style::new().fg(theme.muted).bg(theme.surface),
        );
    }

    let buttons_y = inner.bottom().saturating_sub(2);
    let message_y = buttons_y.saturating_sub(2);
    let rows_start = inner.y.saturating_add(2);
    let rows_height = message_y.saturating_sub(rows_start);
    let row_height = if compact { 2 } else { 1 };
    let row_capacity = usize::from(rows_height / row_height);
    if app.profiles.is_empty() {
        frame.render_widget(
            Paragraph::new("No saved connections. Press n to create one.")
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(ratatui::layout::Alignment::Center),
            Rect::new(inner.x, rows_start, inner.width, rows_height.max(1)),
        );
    } else {
        let start = viewport_start(manager.selected, app.profiles.len(), row_capacity);
        for (visible_index, (index, profile)) in app
            .profiles
            .iter()
            .enumerate()
            .skip(start)
            .take(row_capacity)
            .enumerate()
        {
            let row_area = Rect::new(
                inner.x,
                rows_start + visible_index as u16 * row_height,
                inner.width,
                row_height,
            );
            let selected = index == manager.selected;
            let status = if app.connection.profile_id == Some(profile.id) {
                "● ACTIVE"
            } else if app.connection.pending_profile_id == Some(profile.id) {
                "◌ LINKING"
            } else {
                "○"
            };
            let values = [
                status.to_owned(),
                safe_line(&profile.name),
                kind_name(profile.kind).to_owned(),
                profile_endpoint(profile),
                environment_name(profile.environment).to_owned(),
                if profile.read_only {
                    "READ ONLY".to_owned()
                } else {
                    "READ WRITE".to_owned()
                },
            ];
            let style = if selected {
                Style::new()
                    .fg(theme.text)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text).bg(theme.surface)
            };
            if compact {
                render_compact_profile_row(frame, row_area, &values, style, theme);
            } else {
                render_profile_columns_owned(frame, row_area, values, style);
            }
            if manager.operation.is_none() {
                state.hit_regions.push(HitRegion {
                    area: row_area,
                    target: HitTarget::ProfileRow(index),
                });
            }
        }
    }

    render_message_line(frame, manager, inner, message_y, theme);
    let list_buttons: &[(ProfileButton, &str, bool)] = if app.profiles.is_empty() {
        &[
            (ProfileButton::New, "NEW", false),
            (ProfileButton::Close, "CLOSE", false),
        ]
    } else {
        &[
            (ProfileButton::New, "NEW", false),
            (ProfileButton::Edit, "EDIT", false),
            (ProfileButton::Connect, "CONNECT", false),
            (ProfileButton::Delete, "DELETE", false),
            (ProfileButton::Close, "CLOSE", false),
        ]
    };
    render_buttons(
        frame,
        Rect::new(inner.x, buttons_y, inner.width, 1),
        list_buttons,
        manager.operation.is_none(),
        state,
        theme,
    );
    render_hint(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        if compact {
            "j/k move  Enter connect  n new  d delete  Esc close"
        } else {
            "j/k move   Enter connect   n new   e edit   d delete   Esc close"
        },
        theme,
    );
}

fn render_form(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    manager: &ProfileManagerState,
    state: &mut UiState,
    theme: Theme,
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
    let fields = draft
        .visible_fields()
        .iter()
        .copied()
        .filter(|field| !is_button_field(*field))
        .collect::<Vec<_>>();
    let row_capacity = usize::from(message_y.saturating_sub(inner.y.saturating_add(1)));
    let selected_index = fields
        .iter()
        .position(|field| *field == manager.selected_field)
        .unwrap_or(0);
    let start = viewport_start(selected_index, fields.len(), row_capacity);
    let mut row_y = inner.y.saturating_add(1);
    for field in fields.into_iter().skip(start).take(row_capacity) {
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
            theme,
        );
        if !busy {
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
    let profile = app.profiles.get(manager.selected);
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

fn render_profile_columns(frame: &mut Frame<'_>, area: Rect, values: [&str; 6], style: Style) {
    render_profile_columns_owned(frame, area, values.map(str::to_owned), style);
}

fn render_profile_columns_owned(
    frame: &mut Frame<'_>,
    area: Rect,
    values: [String; 6],
    style: Style,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Min(12),
            Constraint::Length(12),
            Constraint::Length(11),
        ])
        .split(area);
    for (column, value) in columns.iter().zip(values) {
        frame.render_widget(
            Paragraph::new(value)
                .style(style)
                .wrap(Wrap { trim: false }),
            *column,
        );
    }
}

fn render_compact_profile_row(
    frame: &mut Frame<'_>,
    area: Rect,
    values: &[String; 6],
    style: Style,
    theme: Theme,
) {
    frame.render_widget(Block::new().style(style), area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{}  ", values[0]), style),
            Span::styled(
                format!("{}  ", values[1]),
                style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                &values[2],
                Style::new()
                    .fg(theme.action)
                    .bg(style.bg.unwrap_or(theme.surface)),
            ),
        ]))
        .style(style),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.height > 1 {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{}  ", values[4]),
                    Style::new()
                        .fg(theme.muted)
                        .bg(style.bg.unwrap_or(theme.surface)),
                ),
                Span::styled(
                    format!("{}  ", values[5]),
                    Style::new()
                        .fg(theme.warning)
                        .bg(style.bg.unwrap_or(theme.surface)),
                ),
                Span::styled(&values[3], style),
            ]))
            .style(style),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
    }
}

fn render_field(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &ProfileDraft,
    field: ProfileField,
    selected: ProfileField,
    busy: bool,
    theme: Theme,
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

fn render_field_cursor(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &ProfileDraft,
    field: ProfileField,
) {
    if field == ProfileField::Password {
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
        ProfileField::Name => safe_line(draft.name.value()),
        ProfileField::Host => safe_line(draft.host.value()),
        ProfileField::Port => safe_line(draft.port.value()),
        ProfileField::User => safe_line(draft.user.value()),
        ProfileField::Password => {
            if draft.password_len() > 0 {
                "•".repeat(draft.password_len())
            } else if draft.has_stored_credential {
                "Stored in system keyring".to_owned()
            } else {
                "Not set".to_owned()
            }
        }
        ProfileField::Database => safe_line(draft.database.value()),
        ProfileField::Schema => safe_line(draft.schema.value()),
        ProfileField::SslMode => ssl_name(draft.ssl_mode).to_owned(),
        ProfileField::Environment => environment_name(draft.environment).to_owned(),
        ProfileField::ReadOnly => toggle_value(draft.read_only),
        ProfileField::RememberPassword => toggle_value(draft.remember_password),
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
        ProfileField::Name => "NAME",
        ProfileField::Host => "HOST",
        ProfileField::Port => "PORT",
        ProfileField::User => "USER",
        ProfileField::Password => "PASSWORD",
        ProfileField::Database => "DATABASE",
        ProfileField::Schema => "SCHEMA",
        ProfileField::SslMode => "SSL MODE",
        ProfileField::Environment => "ENVIRONMENT",
        ProfileField::ReadOnly => "READ ONLY",
        ProfileField::RememberPassword => "REMEMBER PASSWORD",
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
    matches!(
        field,
        ProfileField::ReadOnly | ProfileField::RememberPassword | ProfileField::SqliteMemory
    )
}

fn toggle_value(enabled: bool) -> String {
    if enabled {
        "[x] ON".to_owned()
    } else {
        "[ ] OFF".to_owned()
    }
}

fn kind_name(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::Postgres => "POSTGRES",
        DatabaseKind::MySql => "MYSQL",
        DatabaseKind::Sqlite => "SQLITE",
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

fn profile_endpoint(profile: &ConnectionProfile) -> String {
    match profile.kind {
        DatabaseKind::Postgres | DatabaseKind::MySql => {
            let host = profile.host.as_deref().unwrap_or("unknown");
            let port = profile
                .port
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            let database = profile
                .database
                .as_deref()
                .map(|database| format!("/{database}"))
                .unwrap_or_default();
            safe_line(&format!("{host}{port}{database}"))
        }
        DatabaseKind::Sqlite => profile
            .sqlite_path
            .as_ref()
            .map(|path| safe_line(&path.to_string_lossy()))
            .or_else(|| profile.database.as_deref().map(safe_line))
            .unwrap_or_else(|| ":memory:".to_owned()),
    }
}

fn safe_line(value: &str) -> String {
    sanitize_terminal_text(value)
        .replace('\n', "<LF>")
        .replace('\t', "<TAB>")
}
