pub mod effects;
pub mod layout;
pub mod theme;

use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, TableState,
        Wrap,
    },
};

use crate::{
    app::App,
    db::{catalog::CatalogKind, query::ResultSet},
    model::{
        editor::EditorMode,
        profile_manager::ProfileField,
        tab::{OutputKind, ResultView},
        workspace::{ConnectionStatus, Focus, Overlay, QueryStatus},
    },
    security::sanitize_terminal_text,
};

use self::{
    effects::UiEffects,
    layout::{AppLayout, LayoutMode},
    theme::Theme,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileButton {
    New,
    Edit,
    Delete,
    Connect,
    Close,
    Test,
    Save,
    SaveAndConnect,
    Cancel,
    ConfirmDelete,
    CancelDelete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HitTarget {
    Focus(Focus),
    Tab(usize),
    ExplorerRow(usize),
    ResultCell { row: usize, column: usize },
    Help,
    ToggleResultView,
    HeaderProfile,
    ProfileRow(usize),
    ProfileField(ProfileField),
    ProfileToggle(ProfileField),
    ProfileButton(ProfileButton),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HitRegion {
    pub area: Rect,
    pub target: HitTarget,
}

#[derive(Debug)]
pub struct UiState {
    pub hit_regions: Vec<HitRegion>,
    pub effects: UiEffects,
    last_focus: Option<Focus>,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new(false)
    }
}

impl UiState {
    pub fn new(reduced_motion: bool) -> Self {
        Self {
            hit_regions: Vec::new(),
            effects: UiEffects::new(reduced_motion),
            last_focus: None,
        }
    }

    pub fn target_at(&self, column: u16, row: u16) -> Option<&HitTarget> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| contains(region.area, column, row))
            .map(|region| &region.target)
    }
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let mut state = UiState::new(true);
    render_with_state(frame, app, &mut state);
}

pub fn render_with_state(frame: &mut Frame<'_>, app: &App, state: &mut UiState) {
    let theme = Theme::default();
    let area = frame.area();
    frame.render_widget(Block::new().style(theme.base()), area);
    let layout = AppLayout::calculate(area, app.focus);
    state.hit_regions.clear();

    if layout.mode == LayoutMode::TooSmall {
        render_too_small(frame, area, theme);
        return;
    }

    render_header(frame, layout.header, app, theme, state);
    render_tabs(frame, layout.tabs, app, theme, state);
    if let Some(area) = layout.explorer {
        state.hit_regions.push(HitRegion {
            area,
            target: HitTarget::Focus(Focus::Explorer),
        });
        render_explorer(frame, area, app, theme, state);
    }
    if let Some(area) = layout.editor {
        render_editor(frame, area, app, theme);
        state.hit_regions.push(HitRegion {
            area,
            target: HitTarget::Focus(Focus::Editor),
        });
    }
    if let Some(area) = layout.result_tabs {
        render_result_tabs(frame, area, app, theme);
        state.hit_regions.push(HitRegion {
            area,
            target: HitTarget::ToggleResultView,
        });
    }
    if let Some(area) = layout.results {
        state.hit_regions.push(HitRegion {
            area,
            target: HitTarget::Focus(Focus::Results),
        });
        render_results(frame, area, app, theme, state);
    }
    render_footer(frame, layout.footer, app, theme);
    state.hit_regions.push(HitRegion {
        area: layout.footer,
        target: HitTarget::Help,
    });

    if let Some(overlay) = &app.overlay {
        render_overlay(frame, area, overlay, theme);
    }

    if state.last_focus.is_some() && state.last_focus != Some(app.focus) {
        state.effects.focus_changed(theme.border);
    }
    state.last_focus = Some(app.focus);
    state.effects.render(frame, layout.body);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
    let profile = app.active_profile().map_or_else(
        || "NO PROFILE".to_owned(),
        |profile| header_text(&profile.name),
    );
    let database = app.connection.server.as_ref().map_or_else(
        || "not connected".to_owned(),
        |server| header_text(&server.database),
    );
    let profile_width = profile.as_str().cell_width();
    let connection = match app.connection.status {
        ConnectionStatus::Disconnected => ("OFFLINE", theme.muted),
        ConnectionStatus::Connecting => ("LINKING", theme.warning),
        ConnectionStatus::Connected => ("ONLINE", theme.accent),
        ConnectionStatus::Failed => ("FAILED", theme.error),
    };
    let query = match app.active_console().query_status {
        QueryStatus::Idle => ("IDLE", theme.muted),
        QueryStatus::Running => ("RUNNING", theme.action),
        QueryStatus::Cancelled => ("CANCELLED", theme.warning),
        QueryStatus::Failed => ("ERROR", theme.error),
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " LAZYDB ",
                Style::new()
                    .fg(theme.background)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::new().bg(theme.surface)),
            Span::styled(
                profile,
                Style::new()
                    .fg(theme.text)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  /  ", Style::new().fg(theme.border).bg(theme.surface)),
            Span::styled(database, Style::new().fg(theme.action).bg(theme.surface)),
        ]),
        Line::from(vec![
            Span::styled(
                format!(" {status} ", status = connection.0),
                Style::new()
                    .fg(connection.1)
                    .bg(theme.surface_raised)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  TX AUTO  ",
                Style::new().fg(theme.muted).bg(theme.surface),
            ),
            Span::styled(
                format!(" QUERY {} ", query.0),
                Style::new().fg(query.1).bg(theme.surface),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme.surface)),
        area,
    );
    let profile_x = area.x.saturating_add(10);
    let profile_width = profile_width.min(area.right().saturating_sub(profile_x));
    if profile_width > 0 {
        state.hit_regions.push(HitRegion {
            area: Rect::new(profile_x, area.y, profile_width, 1),
            target: HitTarget::HeaderProfile,
        });
    }
}

fn header_text(value: &str) -> String {
    sanitize_terminal_text(value)
        .replace('\n', "<LF>")
        .replace('\t', "<TAB>")
}

fn render_tabs(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
    let mut spans = vec![Span::styled(
        " WORKSPACE ",
        Style::new().fg(theme.muted).bg(theme.background),
    )];
    let mut x = area.x + 11;
    for (index, tab) in app.tabs.iter().enumerate() {
        let label = format!(" {:02} {} ", index + 1, tab.name);
        let width = label.chars().count().min(u16::MAX as usize) as u16;
        let active = index == app.active_tab;
        spans.push(Span::styled(
            label,
            if active {
                Style::new()
                    .fg(theme.background)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.muted).bg(theme.surface)
            },
        ));
        state.hit_regions.push(HitRegion {
            area: Rect::new(x, area.y, width.min(area.right().saturating_sub(x)), 1),
            target: HitTarget::Tab(index),
        });
        x = x.saturating_add(width);
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(theme.background)),
        area,
    );
}

fn render_explorer(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut UiState,
) {
    let block = panel_block(" EXPLORER ", app.focus == Focus::Explorer, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let visible = app.explorer.visible();
    if visible.is_empty() {
        let message = if app.connection.status == ConnectionStatus::Connecting {
            "Synchronizing catalog..."
        } else if app.connection.status == ConnectionStatus::Connected {
            "No visible objects"
        } else {
            "No active connection\n\nStart with --url or choose a profile"
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }

    let displayed = visible
        .iter()
        .enumerate()
        .skip(app.explorer.scroll)
        .take(inner.height as usize)
        .collect::<Vec<_>>();
    for (row, (visible_index, _)) in displayed.iter().enumerate() {
        state.hit_regions.push(HitRegion {
            area: Rect::new(inner.x, inner.y.saturating_add(row as u16), inner.width, 1),
            target: HitTarget::ExplorerRow(*visible_index),
        });
    }
    let items = displayed
        .into_iter()
        .map(|(visible_index, visible)| {
            let node = &app.explorer.nodes[visible.node_index];
            let expanded = app.explorer.expanded.contains(&node.id);
            let marker = if node.expandable {
                if expanded { "▾" } else { "▸" }
            } else {
                " "
            };
            let icon = catalog_icon(node.kind);
            let text = format!(
                "{}{} {} {}",
                "  ".repeat(visible.depth),
                marker,
                icon,
                node.name
            );
            let selected = visible_index == app.explorer.selected;
            ListItem::new(Line::from(Span::styled(
                text,
                if selected {
                    Style::new()
                        .fg(theme.accent)
                        .bg(theme.selection)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                        .fg(kind_color(node.kind, theme))
                        .bg(theme.surface)
                },
            )))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).style(Style::new().bg(theme.surface)),
        inner,
    );
}

fn render_editor(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let mode = match app.active_console().editor.mode {
        EditorMode::Normal => "NORMAL",
        EditorMode::Insert => "INSERT",
    };
    let title = format!(" SQL EDITOR  {mode} ");
    let block = panel_block(&title, app.focus == Focus::Editor, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let editor = &app.active_console().editor;
    let number_width = editor.lines().len().max(1).to_string().len().max(2);
    let lines = editor
        .lines()
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(index, text)| {
            let mut spans = vec![Span::styled(
                format!(" {:>width$} │ ", index + 1, width = number_width),
                Style::new().fg(theme.border).bg(theme.surface),
            )];
            spans.extend(highlight_sql(text, theme));
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().fg(theme.text).bg(theme.surface)),
        inner,
    );

    if app.focus == Focus::Editor && editor.row < inner.height as usize {
        let prefix = number_width as u16 + 4;
        let x = inner
            .x
            .saturating_add(prefix)
            .saturating_add(editor.column as u16)
            .min(inner.right().saturating_sub(1));
        let y = inner.y.saturating_add(editor.row as u16);
        if y < inner.bottom() {
            frame.set_cursor_position(Position::new(x, y));
        }
    }
}

fn render_result_tabs(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let active = app.active_console().result_view;
    let data_style = if active == ResultView::Data {
        Style::new()
            .fg(theme.background)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.muted).bg(theme.surface)
    };
    let output_style = if active == ResultView::Output {
        Style::new()
            .fg(theme.background)
            .bg(theme.action)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.muted).bg(theme.surface)
    };
    let stats = app.active_console().outcome.as_ref().map_or_else(
        || "no result".to_owned(),
        |outcome| {
            format!(
                "{} rows  ·  {} ms",
                outcome.stats.row_count,
                outcome.stats.total().as_millis()
            )
        },
    );
    let line = Line::from(vec![
        Span::styled(" DATA ", data_style),
        Span::raw(" "),
        Span::styled(" OUTPUT ", output_style),
        Span::styled(
            format!("    {stats}"),
            Style::new().fg(theme.muted).bg(theme.background),
        ),
    ]);
    frame.render_widget(Paragraph::new(line).style(theme.base()), area);
}

fn render_results(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
    match app.active_console().result_view {
        ResultView::Output | ResultView::Plan => render_output(frame, area, app, theme),
        ResultView::Data => render_data(frame, area, app, theme, state),
    }
}

fn render_data(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
    let block = panel_block(" RESULT SET ", app.focus == Focus::Results, theme);
    let Some(result) = app
        .active_console()
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.result_sets.last())
    else {
        frame.render_widget(
            Paragraph::new("Run a query to populate the data viewport")
                .block(block)
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(Alignment::Center),
            area,
        );
        return;
    };
    render_result_table(frame, area, result, app, theme, block, state);
}

fn render_result_table(
    frame: &mut Frame<'_>,
    area: Rect,
    result: &ResultSet,
    app: &App,
    theme: Theme,
    block: Block<'_>,
    state: &mut UiState,
) {
    if result.columns.is_empty() {
        frame.render_widget(
            Paragraph::new(format!(
                "Statement complete · {} row(s) affected",
                result.affected_rows
            ))
            .block(block)
            .style(Style::new().fg(theme.accent).bg(theme.surface))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }
    let available = area.width.saturating_sub(3).max(1);
    let each = (available / result.columns.len().max(1) as u16).clamp(8, 28);
    let widths = result
        .columns
        .iter()
        .map(|_| Constraint::Length(each))
        .collect::<Vec<_>>();
    let row_y = area.y.saturating_add(3);
    for (row_index, _) in result
        .rows
        .iter()
        .take(area.height.saturating_sub(4) as usize)
        .enumerate()
    {
        for column_index in 0..result.columns.len() {
            let x = area
                .x
                .saturating_add(2)
                .saturating_add(column_index as u16 * each.saturating_add(1));
            if x >= area.right() {
                break;
            }
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    x,
                    row_y.saturating_add(row_index as u16),
                    each.min(area.right().saturating_sub(x)),
                    1,
                ),
                target: HitTarget::ResultCell {
                    row: row_index,
                    column: column_index,
                },
            });
        }
    }
    let header = Row::new(result.columns.iter().map(|column| {
        Cell::from(column.name.clone())
            .style(Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))
    }))
    .height(1)
    .bottom_margin(1);
    let rows = result.rows.iter().map(|row| {
        Row::new(row.iter().map(|value| {
            let preview = value.preview(each.saturating_sub(2) as usize);
            let style = match value {
                crate::db::value::CellValue::Null => {
                    Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC)
                }
                crate::db::value::CellValue::Unsupported { .. } => Style::new().fg(theme.warning),
                _ => Style::new().fg(theme.text),
            };
            Cell::from(preview.text).style(style)
        }))
    });
    let table = Table::new(rows, widths)
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
    let mut state = TableState::new().with_selected_cell(Some((
        app.active_console().selected_row,
        app.active_console().selected_column,
    )));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_output(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let block = panel_block(" OUTPUT LOG ", app.focus == Focus::Results, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let entries = &app.active_console().output;
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("No execution output")
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    let lines = entries
        .iter()
        .rev()
        .take(inner.height as usize)
        .rev()
        .map(|entry| {
            let (marker, color) = match entry.kind {
                OutputKind::Info => ("·", theme.action),
                OutputKind::Success => ("✓", theme.accent),
                OutputKind::Error => ("!", theme.error),
                OutputKind::Cancelled => ("×", theme.warning),
            };
            Line::from(vec![
                Span::styled(
                    format!(" {marker} "),
                    Style::new().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(entry.message.clone(), Style::new().fg(theme.text)),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme.surface)),
        inner,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let (mode, mode_color) = match app.focus {
        Focus::Editor => match app.active_console().editor.mode {
            EditorMode::Normal => ("NORMAL", theme.accent),
            EditorMode::Insert => ("INSERT", theme.action),
        },
        Focus::Explorer => ("EXPLORE", theme.accent),
        Focus::Results => ("DATA", theme.warning),
    };
    let hints = match app.focus {
        Focus::Explorer => "j/k move   h/l collapse/expand   Enter open   r refresh",
        Focus::Editor => "Esc normal   i/a/o insert   F5 run   Ctrl+w pane   [t/]t tabs",
        Focus::Results => "h/j/k/l cells   Tab data/output   Ctrl+w pane",
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" {mode} "),
            Style::new()
                .fg(theme.background)
                .bg(mode_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {hints}"),
            Style::new().fg(theme.muted).bg(theme.surface),
        ),
        Span::styled(
            if app.focus == Focus::Editor && app.active_console().editor.mode == EditorMode::Insert
            {
                "   F1 help "
            } else {
                "   ? help "
            },
            Style::new()
                .fg(theme.action)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let second = Line::from(Span::styled(
        app.connection.error.as_deref().unwrap_or("Ready"),
        Style::new()
            .fg(if app.connection.error.is_some() {
                theme.error
            } else {
                theme.muted
            })
            .bg(theme.surface),
    ));
    frame.render_widget(
        Paragraph::new(vec![line, second]).style(Style::new().bg(theme.surface)),
        area,
    );
}

fn render_overlay(frame: &mut Frame<'_>, area: Rect, overlay: &Overlay, theme: Theme) {
    match overlay {
        Overlay::Help(focus) => render_help(frame, area, *focus, theme),
        Overlay::ProfileManager => render_message(
            frame,
            area,
            "CONNECTIONS",
            "Profile picker is not open in this view",
            theme,
        ),
        Overlay::Message { title, body } => render_message(frame, area, title, body, theme),
    }
}

fn render_help(frame: &mut Frame<'_>, area: Rect, focus: Focus, theme: Theme) {
    let popup = centered(area, 74, 22);
    frame.render_widget(Clear, popup);
    let title = format!(" KEYMAP // {} ", focus_name(focus));
    let mut lines = vec![
        key_line("? / F1", "open contextual keymap", theme),
        key_line("Esc", "close overlay / return to Normal", theme),
        key_line("Ctrl-w h/j/k/l", "move between panels", theme),
        key_line("[t / ]t", "previous / next tab", theme),
        key_line("Space n", "new SQL console", theme),
        Line::raw(""),
    ];
    match focus {
        Focus::Explorer => lines.extend([
            key_line("j / k", "move selection", theme),
            key_line("h / l / Enter", "collapse / expand / open", theme),
        ]),
        Focus::Editor => lines.extend([
            key_line("i / Esc", "Insert / Normal mode", theme),
            key_line("h j k l", "move cursor in Normal mode", theme),
            key_line("F5 / Space r", "execute SQL buffer", theme),
        ]),
        Focus::Results => lines.extend([
            key_line("h j k l", "move through cells", theme),
            key_line("Tab", "switch Data / Output", theme),
        ]),
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Context first: the footer always shows the shortest path.",
        Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .title_style(theme.title(true))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(theme.accent))
                    .style(Style::new().bg(theme.surface_raised)),
            )
            .style(Style::new().fg(theme.text).bg(theme.surface_raised))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_message(frame: &mut Frame<'_>, area: Rect, title: &str, body: &str, theme: Theme) {
    let popup = centered(area, 64, 12);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(body.to_owned())
            .block(panel_block(&format!(" {title} "), true, theme))
            .style(Style::new().fg(theme.text).bg(theme.surface_raised))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let text = Text::from(vec![
        Line::from(Span::styled(
            "TERMINAL TOO SMALL",
            Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "Resize to at least 56 × 16",
            Style::new().fg(theme.text),
        )),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .style(theme.base())
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(theme.border)),
            ),
        area,
    );
}

fn panel_block<'a>(title: &'a str, focused: bool, theme: Theme) -> Block<'a> {
    Block::default()
        .title(title)
        .title_style(theme.title(focused))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { theme.accent } else { theme.border }))
        .style(Style::new().bg(theme.surface))
}

fn highlight_sql(value: &str, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '-' && chars.get(index + 1) == Some(&'-') {
            spans.push(Span::styled(
                chars[index..].iter().collect::<String>(),
                Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC),
            ));
            break;
        }
        if matches!(chars[index], '\'' | '"') {
            let quote = chars[index];
            let start = index;
            index += 1;
            while index < chars.len() {
                if chars[index] == quote {
                    index += 1;
                    break;
                }
                index += 1;
            }
            spans.push(Span::styled(
                chars[start..index].iter().collect::<String>(),
                Style::new().fg(theme.warning),
            ));
            continue;
        }
        if chars[index].is_ascii_alphanumeric() || chars[index] == '_' {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            let token = chars[start..index].iter().collect::<String>();
            let keyword = is_sql_keyword(&token);
            spans.push(Span::styled(
                token,
                if keyword {
                    Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(theme.text)
                },
            ));
            continue;
        }
        spans.push(Span::styled(
            chars[index].to_string(),
            Style::new().fg(if ",;()=*<>.+-/".contains(chars[index]) {
                theme.action
            } else {
                theme.text
            }),
        ));
        index += 1;
    }
    spans
}

fn is_sql_keyword(value: &str) -> bool {
    matches!(
        value.to_ascii_uppercase().as_str(),
        "SELECT"
            | "FROM"
            | "WHERE"
            | "JOIN"
            | "LEFT"
            | "RIGHT"
            | "INNER"
            | "OUTER"
            | "ON"
            | "AS"
            | "AND"
            | "OR"
            | "NOT"
            | "NULL"
            | "TRUE"
            | "FALSE"
            | "INSERT"
            | "INTO"
            | "VALUES"
            | "UPDATE"
            | "SET"
            | "DELETE"
            | "CREATE"
            | "ALTER"
            | "DROP"
            | "TABLE"
            | "VIEW"
            | "INDEX"
            | "ORDER"
            | "BY"
            | "GROUP"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "WITH"
            | "UNION"
            | "ALL"
            | "DISTINCT"
            | "RETURNING"
            | "BEGIN"
            | "COMMIT"
            | "ROLLBACK"
            | "EXPLAIN"
    )
}

fn key_line<'a>(key: &'a str, description: &'a str, theme: Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {key:<18}"),
            Style::new().fg(theme.action).add_modifier(Modifier::BOLD),
        ),
        Span::styled(description, Style::new().fg(theme.text)),
    ])
}

fn catalog_icon(kind: CatalogKind) -> &'static str {
    match kind {
        CatalogKind::Database => "◆",
        CatalogKind::Schema => "◇",
        CatalogKind::Table => "▦",
        CatalogKind::View => "◈",
        CatalogKind::Column => "·",
        CatalogKind::Index => "⌘",
        CatalogKind::PrimaryKey => "◆",
        CatalogKind::UniqueConstraint => "◇",
        CatalogKind::ForeignKey => "↗",
        CatalogKind::CheckConstraint => "✓",
        CatalogKind::Function | CatalogKind::Procedure => "ƒ",
        CatalogKind::Trigger => "⚡",
        CatalogKind::Sequence => "#",
        CatalogKind::Type => "τ",
    }
}

fn kind_color(kind: CatalogKind, theme: Theme) -> Color {
    match kind {
        CatalogKind::Database | CatalogKind::Schema => theme.action,
        CatalogKind::Table | CatalogKind::View => theme.text,
        CatalogKind::PrimaryKey | CatalogKind::UniqueConstraint => theme.warning,
        CatalogKind::ForeignKey | CatalogKind::Trigger => theme.accent,
        _ => theme.muted,
    }
}

fn focus_name(focus: Focus) -> &'static str {
    match focus {
        Focus::Explorer => "EXPLORER",
        Focus::Editor => "EDITOR",
        Focus::Results => "RESULTS",
    }
}

fn centered(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.saturating_sub(4).min(max_width);
    let height = area.height.saturating_sub(2).min(max_height);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(horizontal[1]);
    vertical[1]
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.right() && row >= area.y && row < area.bottom()
}
