pub mod data_grid;
pub mod icons;
pub mod layout;
pub mod profiles;
pub mod query_bar;
pub mod relation;
pub mod theme;

use ratatui::{
    Frame,
    buffer::CellWidth,
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use crate::{
    app::App,
    db::{catalog::CatalogKind, query::ResultSet},
    model::{
        editor::{EditorHighlightKind, EditorMode, EditorViewport},
        explorer::{ExplorerConnectionStatus, ProfileProvenance},
        profile_manager::ProfileField,
        tab::{OutputKind, ResultView, WorkspaceTab},
        workspace::{ConnectionStatus, Focus, Overlay, QueryStatus},
    },
    security::sanitize_terminal_text,
};

use self::{
    layout::{AppLayout, LayoutMode},
    theme::Theme,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileButton {
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
    ExplorerRow(crate::model::explorer::ExplorerNodeId),
    ResultCell { row: usize, column: usize },
    Help,
    ToggleResultView,
    RelationView(crate::model::relation::RelationView),
    RelationRetry,
    RelationCancel,
    DataQueryInput(crate::model::data_query::DataQueryInput),
    RelationColumnResize { column: usize, width: u16 },
    HeaderProfile,
    ProfileField(ProfileField),
    ProfileDriver(crate::profile::DatabaseKind),
    ProfileToggle(ProfileField),
    ProfileScopeRow(String),
    ProfileButton(ProfileButton),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HitRegion {
    pub area: Rect,
    pub target: HitTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

#[derive(Debug)]
pub struct UiState {
    pub hit_regions: Vec<HitRegion>,
    pub editor_viewport: Option<EditorViewport>,
    pub completion_popup: Option<Rect>,
    pub cursor_style: Option<CursorStyle>,
    pub click_tracker: RefCell<Option<(crate::model::explorer::ExplorerNodeId, Instant)>>,
    pub relation_resize: RefCell<Option<(usize, u16, u16)>>,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            hit_regions: Vec::new(),
            editor_viewport: None,
            completion_popup: None,
            cursor_style: None,
            click_tracker: RefCell::new(None),
            relation_resize: RefCell::new(None),
        }
    }

    pub fn target_at(&self, column: u16, row: u16) -> Option<&HitTarget> {
        self.hit_regions
            .iter()
            .rev()
            .find(|region| contains(region.area, column, row))
            .map(|region| &region.target)
    }

    pub fn track_explorer_click(
        &self,
        id: &crate::model::explorer::ExplorerNodeId,
        now: Instant,
    ) -> bool {
        let double = self
            .click_tracker
            .borrow()
            .as_ref()
            .is_some_and(|(previous, timestamp)| {
                previous == id && now.duration_since(*timestamp) <= Duration::from_millis(500)
            });
        if !double
            && self
                .click_tracker
                .borrow()
                .as_ref()
                .is_some_and(|(_, timestamp)| {
                    now.duration_since(*timestamp) > Duration::from_millis(500)
                })
        {
            self.clear_click_tracker();
        }
        *self.click_tracker.borrow_mut() = Some((id.clone(), now));
        double
    }

    pub fn clear_click_tracker(&self) {
        *self.click_tracker.borrow_mut() = None;
    }
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let mut state = UiState::new();
    render_with_state(frame, app, &mut state);
}

pub fn render_with_state(frame: &mut Frame<'_>, app: &App, state: &mut UiState) {
    render_with_state_using_icons(frame, app, state, icons::IconSet::default());
}

pub fn render_with_state_using_icons(
    frame: &mut Frame<'_>,
    app: &App,
    state: &mut UiState,
    icons: icons::IconSet,
) {
    let theme = Theme::default();
    let area = frame.area();
    frame.render_widget(Block::new().style(theme.base()), area);
    let is_relation = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(_))
    );
    let layout = AppLayout::calculate(area, app.focus, is_relation);
    state.hit_regions.clear();
    state.editor_viewport = None;
    state.completion_popup = None;

    if layout.mode == LayoutMode::TooSmall {
        render_too_small(frame, area, theme);
        return;
    }

    render_header(frame, layout.header, app, theme, state);
    if let Some(area) = layout.tabs {
        render_tabs(frame, area, app, theme, state);
    }
    if is_relation {
        if let Some(area) = layout.explorer {
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Explorer),
            });
            render_explorer(frame, area, app, theme, state, icons);
        }
        if let Some(area) = layout.relation {
            relation::render(frame, area, app, theme, state);
        }
        render_footer(frame, layout.footer, app, theme);
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
    } else {
        if let Some(area) = layout.explorer {
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Explorer),
            });
            render_explorer(frame, area, app, theme, state, icons);
        }
        if let Some(area) = layout.editor {
            let completion_anchor = render_editor(frame, area, app, theme, state);
            render_completion_popup(frame, app, theme, state, completion_anchor, icons);
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
    }

    if let Some(overlay) = &app.overlay {
        render_overlay(frame, area, overlay, app, state, theme, icons);
    }
}

// Relation pages are rendered by `ui::relation`; keeping them out of the SQL path
// prevents accidental editor access when a relation tab is active.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompletionAnchor {
    viewport: Rect,
    cursor: Position,
}

fn render_completion_popup(
    frame: &mut Frame<'_>,
    app: &App,
    theme: Theme,
    state: &mut UiState,
    anchor: Option<CompletionAnchor>,
    icons: icons::IconSet,
) {
    if app.active_editor_mode() != crate::model::editor::EditorMode::Insert {
        return;
    }
    let Some(popup) = app
        .active_console_opt()
        .and_then(|tab| tab.completion.as_ref())
    else {
        return;
    };
    let Some(anchor) = anchor else {
        return;
    };
    if popup.candidates.is_empty() {
        return;
    }
    let desired_height = popup.candidates.len().min(10) as u16;
    let desired_width = popup
        .candidates
        .iter()
        .map(|candidate| {
            let detail = candidate.detail.as_deref().unwrap_or("");
            format!(
                "{} {}  {}",
                icons.completion(candidate.kind),
                candidate.label,
                detail
            )
            .as_str()
            .cell_width()
            .saturating_add(1)
        })
        .max()
        .unwrap_or(4)
        .max(4);
    let Some(area) = completion_popup_rect(anchor, desired_width, desired_height) else {
        return;
    };
    state.completion_popup = Some(area);
    let items = popup
        .candidates
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, candidate)| {
            let detail = candidate.detail.as_deref().unwrap_or("");
            let row_style = if index == popup.selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", icons.completion(candidate.kind)), row_style),
                Span::styled(candidate.label.clone(), row_style),
                Span::styled(
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!("  {detail}")
                    },
                    row_style.fg(if index == popup.selected {
                        theme.background
                    } else {
                        theme.muted
                    }),
                ),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, area);
    frame.render_widget(List::new(items), area);
}

fn completion_popup_rect(
    anchor: CompletionAnchor,
    desired_width: u16,
    desired_height: u16,
) -> Option<Rect> {
    let viewport = anchor.viewport;
    if viewport.is_empty() || desired_height == 0 {
        return None;
    }

    let width = desired_width.min(viewport.width).max(1);
    let x = anchor
        .cursor
        .x
        .clamp(viewport.x, viewport.right().saturating_sub(1))
        .min(viewport.right().saturating_sub(width));
    let below_y = anchor.cursor.y.saturating_add(1);
    let below = viewport.bottom().saturating_sub(below_y);
    let above = anchor.cursor.y.saturating_sub(viewport.y);
    let (height, y) = if below >= desired_height {
        (desired_height, below_y)
    } else if above >= desired_height {
        (
            desired_height,
            anchor.cursor.y.saturating_sub(desired_height),
        )
    } else if below >= above && below > 0 {
        (below, below_y)
    } else if above > 0 {
        (above, viewport.y)
    } else {
        return None;
    };

    Some(Rect::new(x, y, width, height))
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
    let query = match app.active_console_opt().map(|tab| tab.query_status) {
        None => ("N/A", theme.muted),
        Some(QueryStatus::Idle) => ("IDLE", theme.muted),
        Some(QueryStatus::Running) => ("RUNNING", theme.action),
        Some(QueryStatus::Cancelled) => ("CANCELLED", theme.warning),
        Some(QueryStatus::Failed) => ("ERROR", theme.error),
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
    let mut spans = Vec::new();
    let mut x = area.x;
    for (index, tab) in app.tabs.iter().enumerate() {
        let title = sanitize_terminal_text(tab.title())
            .chars()
            .take(48)
            .collect::<String>();
        let label = format!(" {:02} {} ", index + 1, title);
        let width = label.cell_width();
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
    icons: icons::IconSet,
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
            "No active connection\n\nPress n or Enter to create a profile"
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .wrap(Wrap { trim: true }),
            inner,
        );
        if app.explorer.selected_id()
            == Some(&crate::model::explorer::ExplorerNodeId::EmptyProfiles)
        {
            state.hit_regions.push(HitRegion {
                area: Rect::new(inner.x, inner.y, inner.width, 1),
                target: HitTarget::ExplorerRow(
                    crate::model::explorer::ExplorerNodeId::EmptyProfiles,
                ),
            });
        }
        return;
    }

    let displayed = visible
        .iter()
        .enumerate()
        .skip(app.explorer.normalized.scroll)
        .take(inner.height as usize)
        .collect::<Vec<_>>();
    for (row, (_, visible)) in displayed.iter().enumerate() {
        state.hit_regions.push(HitRegion {
            area: Rect::new(inner.x, inner.y.saturating_add(row as u16), inner.width, 1),
            target: HitTarget::ExplorerRow(visible.id.clone()),
        });
    }
    let items = displayed
        .into_iter()
        .map(|(_, visible)| {
            let expanded = app.explorer.normalized.expanded.contains(&visible.id);
            let marker = if visible.expandable {
                if expanded { "▾" } else { "▸" }
            } else {
                " "
            };
            let icon = match &visible.id {
                crate::model::explorer::ExplorerNodeId::Group { group, .. } => {
                    icons.group(*group, expanded)
                }
                _ => visible.kind.map_or("·", |kind| icons.catalog(kind)),
            };
            let label = sanitize_terminal_text(&visible.label);
            let selected = app.explorer.selected_id() == Some(&visible.id);
            let label_style = if selected {
                Style::new()
                    .fg(theme.accent)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text).bg(theme.surface)
            };
            let base = format!("{}{} ", "  ".repeat(visible.depth), marker);
            let mut spans = vec![Span::styled(base, label_style)];
            if let Some(kind) = visible.profile_kind {
                spans.push(Span::styled(
                    format!("{} ", icons.database(kind)),
                    Style::new().fg(theme.action).bg(if selected {
                        theme.selection
                    } else {
                        theme.surface
                    }),
                ));
            } else {
                spans.push(Span::styled(
                    format!("{} ", icon),
                    Style::new()
                        .fg(visible
                            .kind
                            .map_or(theme.muted, |kind| kind_color(kind, theme)))
                        .bg(if selected {
                            theme.selection
                        } else {
                            theme.surface
                        }),
                ));
            }
            spans.push(Span::styled(label, label_style));
            let secondary_style = Style::new().fg(theme.muted).bg(if selected {
                theme.selection
            } else {
                theme.surface
            });
            if visible.provenance == Some(ProfileProvenance::Session) {
                spans.push(Span::styled("  SESSION", secondary_style));
            }
            if let Some(status) = visible.connection_status {
                spans.extend(connection_status_spans(status, theme, selected));
            }
            if let Some(endpoint) = visible
                .endpoint
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                spans.push(Span::styled(
                    format!("  {}", sanitize_terminal_text(endpoint)),
                    secondary_style,
                ));
            }
            if let Some(metadata) = visible
                .metadata
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                spans.push(Span::styled(
                    format!("  {}", sanitize_terminal_text(metadata)),
                    secondary_style,
                ));
            }
            if let Some(comment) = visible.comment.as_deref().filter(|value| !value.is_empty()) {
                spans.push(Span::styled(
                    format!("  {}", sanitize_terminal_text(comment)),
                    secondary_style.add_modifier(Modifier::DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).style(Style::new().bg(theme.surface)),
        inner,
    );
}

fn render_editor(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut UiState,
) -> Option<CompletionAnchor> {
    let base_block = panel_block("", app.focus == Focus::Editor, theme);
    let inner = base_block.inner(area);
    let number_width = app
        .active_editor_render_snapshot(EditorViewport {
            width: 0,
            height: 0,
        })
        .map(|snapshot| snapshot.total_lines.max(1).to_string().len().max(2))
        .unwrap_or(2);
    let gutter = number_width.saturating_add(4);
    let viewport = EditorViewport {
        width: inner.width.saturating_sub(gutter as u16) as usize,
        height: inner.height as usize,
    };
    state.editor_viewport = Some(viewport);
    let Ok(snapshot) = app.active_editor_render_snapshot(viewport) else {
        return None;
    };
    let text_viewport = Rect::new(
        inner.x.saturating_add(gutter as u16),
        inner.y,
        viewport.width.min(u16::MAX as usize) as u16,
        viewport.height.min(u16::MAX as usize) as u16,
    );
    let completion_anchor = snapshot
        .prompt
        .is_none()
        .then_some(snapshot.cursor_screen_cell)
        .flatten()
        .map(|(x, y)| CompletionAnchor {
            viewport: text_viewport,
            cursor: Position::new(
                text_viewport.x.saturating_add(x),
                text_viewport.y.saturating_add(y),
            ),
        });
    let mode = match snapshot.mode {
        EditorMode::Normal => "NORMAL",
        EditorMode::Insert => "INSERT",
        EditorMode::Replace => "REPLACE",
        EditorMode::VisualChar => "VISUAL",
        EditorMode::VisualLine => "VISUAL LINE",
        EditorMode::VisualBlock => "VISUAL BLOCK",
    };
    let target = app
        .active_console_opt()
        .and_then(|tab| tab.execution_target.as_ref())
        .and_then(|target| {
            app.profiles
                .iter()
                .find(|profile| profile.id == target.profile_id)
                .map(|profile| {
                    let target = format!(
                        "[{}] {}{}",
                        profile.name,
                        target.database,
                        target
                            .schema
                            .as_deref()
                            .map(|schema| format!(".{schema}"))
                            .unwrap_or_default()
                    );
                    if app.connection.active_identity().is_some() {
                        target
                    } else {
                        format!("{target} OFFLINE")
                    }
                })
        })
        .unwrap_or_else(|| {
            if app.connection.active_identity().is_some() {
                "TARGET REQUIRED".to_owned()
            } else {
                "OFFLINE / NO TARGET".to_owned()
            }
        });
    let transaction = match app
        .active_console_opt()
        .map(|tab| (tab.transaction_mode, tab.transaction_state))
    {
        Some((crate::model::transaction::TransactionMode::Auto, _)) => "TX AUTO",
        Some((_, crate::model::transaction::TransactionState::Active)) => "TX MANUAL:ACTIVE",
        Some((_, crate::model::transaction::TransactionState::Aborted)) => "TX ABORTED",
        Some((_, crate::model::transaction::TransactionState::OutcomeUnknown)) => "TX UNKNOWN",
        Some((_, _)) => "TX MANUAL:IDLE",
        None => "TX N/A",
    };
    let block = base_block
        .title_top(Line::raw(format!(" SQL EDITOR  {mode} ")).left_aligned())
        .title_top(Line::raw(format!(" {target}  {transaction} ")).right_aligned());
    state.cursor_style = Some(if snapshot.prompt.is_some() {
        CursorStyle::Bar
    } else {
        match snapshot.mode {
            EditorMode::Insert => CursorStyle::Bar,
            EditorMode::Replace => CursorStyle::Underline,
            _ => CursorStyle::Block,
        }
    });
    frame.render_widget(block, area);
    for (row, line) in snapshot.lines.iter().take(viewport.height).enumerate() {
        let y = inner.y.saturating_add(row as u16);
        let selected = snapshot.selections.iter().any(|selection| {
            line.line >= selection.start.line.min(selection.end.line)
                && line.line <= selection.start.line.max(selection.end.line)
        });
        let line_style = Style::new().fg(theme.border).bg(if selected {
            theme.selection
        } else {
            theme.surface
        });
        frame.render_widget(
            Paragraph::new(format!(" {:>number_width$} │ ", line.line + 1)).style(line_style),
            Rect::new(inner.x, y, gutter as u16, 1),
        );
        let content = line
            .spans
            .iter()
            .map(|span| {
                let foreground = match span.kind {
                    EditorHighlightKind::Keyword => theme.accent,
                    EditorHighlightKind::Identifier => theme.action,
                    EditorHighlightKind::String => theme.warning,
                    EditorHighlightKind::Number | EditorHighlightKind::Parameter => theme.action,
                    EditorHighlightKind::Comment => theme.muted,
                    EditorHighlightKind::Operator | EditorHighlightKind::Punctuation => theme.text,
                    EditorHighlightKind::Plain => theme.text,
                };
                Span::styled(
                    span.text.clone(),
                    Style::new()
                        .fg(foreground)
                        .bg(if selected {
                            theme.selection
                        } else {
                            theme.surface
                        })
                        .add_modifier(if span.current_statement {
                            Modifier::UNDERLINED
                        } else {
                            Modifier::empty()
                        }),
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Line::from(content))
                .style(Style::new().bg(if selected {
                    theme.selection
                } else {
                    theme.surface
                }))
                .scroll((0, snapshot.horizontal_offset.min(u16::MAX as usize) as u16)),
            Rect::new(
                inner.x.saturating_add(gutter as u16),
                y,
                viewport.width as u16,
                1,
            ),
        );
    }

    if app.overlay.is_none()
        && app.focus == Focus::Editor
        && let Some(prompt) = snapshot.prompt.as_ref()
    {
        let prompt_text = match prompt.error.as_deref() {
            Some(error) => format!("{}{}  [{}]", prompt.prefix, prompt.text, error),
            None => format!("{}{}", prompt.prefix, prompt.text),
        };
        let prompt_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
        frame.render_widget(
            Paragraph::new(prompt_text).style(Style::new().fg(theme.accent).bg(theme.surface)),
            prompt_area,
        );
        let cursor_x = inner
            .x
            .saturating_add(prompt.prefix.chars().count() as u16)
            .saturating_add(prompt.cursor as u16)
            .min(prompt_area.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, prompt_area.y));
    } else if app.overlay.is_none()
        && app.focus == Focus::Editor
        && let Some((x, y)) = snapshot.cursor_screen_cell
    {
        let x = inner
            .x
            .saturating_add(gutter as u16)
            .saturating_add(x)
            .min(inner.right().saturating_sub(1));
        let y = inner.y.saturating_add(y);
        if y < inner.bottom() {
            frame.set_cursor_position(Position::new(x, y));
        }
    }
    completion_anchor
}

fn render_result_tabs(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let active = app
        .active_console_opt()
        .map_or(ResultView::Data, |tab| tab.result_view);
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
    let stats = app
        .active_console_opt()
        .and_then(|tab| tab.outcome.as_ref())
        .map_or_else(
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
    match app
        .active_console_opt()
        .map_or(ResultView::Data, |tab| tab.result_view)
    {
        ResultView::Output | ResultView::Plan => render_output(frame, area, app, theme),
        ResultView::Data => render_data(frame, area, app, theme, state),
    }
}

fn render_data(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
    let Some(tab) = app.active_console_opt() else {
        return;
    };
    let query_height = if !matches!(
        tab.query.capability,
        crate::model::data_query::DataQueryCapability::Relation
            | crate::model::data_query::DataQueryCapability::Sql
    ) {
        3
    } else {
        2
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(query_height), Constraint::Min(1)])
        .split(area);
    query_bar::render(frame, chunks[0], &tab.query, theme, state);
    let area = chunks[1];
    let block = panel_block(" RESULT SET ", app.focus == Focus::Results, theme);
    let Some(result) = tab
        .derived
        .as_ref()
        .and_then(|derived| derived.outcome.as_ref())
        .or(tab.outcome.as_ref())
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
    render_result_table(frame, area, result, tab.grid.clone(), theme, block, state);
}

pub(crate) fn render_result_table(
    frame: &mut Frame<'_>,
    area: Rect,
    result: &ResultSet,
    grid: crate::model::tab::GridState,
    theme: Theme,
    block: Block<'_>,
    state: &mut UiState,
) {
    let overrides = grid.column_widths.clone();
    data_grid::render(frame, area, result, grid, &overrides, theme, block, state);
}

fn render_output(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let block = panel_block(" OUTPUT LOG ", app.focus == Focus::Results, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let entries = app
        .active_console_opt()
        .map_or(&[][..], |tab| tab.output.as_slice());
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
        Focus::Editor => match app.active_editor_mode() {
            EditorMode::Normal => ("NORMAL", theme.accent),
            EditorMode::Insert => ("INSERT", theme.action),
            EditorMode::Replace => ("REPLACE", theme.action),
            EditorMode::VisualChar => ("VISUAL", theme.accent),
            EditorMode::VisualLine => ("VISUAL LINE", theme.accent),
            EditorMode::VisualBlock => ("VISUAL BLOCK", theme.accent),
        },
        Focus::Explorer => ("EXPLORE", theme.accent),
        Focus::Results => ("DATA", theme.warning),
    };
    let hints = match app.focus {
        Focus::Explorer => "j/k move   o toggle   Enter open   r refresh",
        Focus::Editor => "Esc normal   i/a/o insert   F5 run   [ then t / ] then t tabs",
        Focus::Results if app.is_active_relation_tab() => {
            "h/j/k/l cells   Space s SQL console   Ctrl+w pane"
        }
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
            "   F1 help ",
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

fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay: &Overlay,
    app: &App,
    state: &mut UiState,
    theme: Theme,
    icons: icons::IconSet,
) {
    match overlay {
        Overlay::Help(help) => render_help(frame, area, help, app, state, theme),
        Overlay::ProfileManager => {
            profiles::render_profile_manager(frame, area, app, state, theme, icons)
        }
        Overlay::Message { title, body } => render_message(frame, area, title, body, theme),
        Overlay::SubstituteConfirm { remaining } => {
            render_substitute_confirm(frame, area, *remaining, theme)
        }
        Overlay::ExecutionConfirm { draft, focus } => {
            render_execution_confirm(frame, area, draft, *focus, app, theme)
        }
        Overlay::ManualCancelConfirm { focus, .. } => {
            use crate::model::workspace::ManualCancelFocus;
            let popup = centered(area, 76, 12);
            frame.render_widget(Clear, popup);
            let cancel = *focus == ManualCancelFocus::CancelQueryAndRollback;
            let lines = vec![
                Line::from(Span::styled(" CANCEL ACTIVE QUERY? ", theme.title(true))),
                Line::raw("Cancelling rolls back all uncommitted work in this transaction"),
                Line::raw(""),
                Line::raw(format!(
                    "{}   {}",
                    if cancel {
                        " Keep Running "
                    } else {
                        "[Keep Running]"
                    },
                    if cancel {
                        "[Cancel Query + Roll Back]"
                    } else {
                        " Cancel Query + Roll Back "
                    }
                )),
                Line::raw("Tab/Left/Right focus, Enter confirm, Esc keep running"),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(" CANCELLATION CONFIRMATION ", true, theme))
                    .style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                popup,
            );
        }
        Overlay::TransactionExitConfirm { prompt, choice } => {
            use crate::model::transaction::TransactionExitChoice;
            let popup = centered(area, 78, 12);
            frame.render_widget(Clear, popup);
            let tab = app.tabs.iter().find(|tab| tab.id() == prompt.console_id);
            let state = tab
                .and_then(|tab| tab.as_console())
                .map(|tab| format!("{:?}", tab.transaction_state))
                .unwrap_or_else(|| "gone".into());
            let running = tab.is_some_and(|tab| {
                tab.as_console()
                    .is_some_and(|tab| tab.query_status == QueryStatus::Running)
            });
            let commit_disabled = tab.is_some_and(|tab| {
                tab.as_console().is_some_and(|tab| {
                    tab.transaction_state == crate::model::transaction::TransactionState::Aborted
                })
            });
            let buttons = if running {
                "Query running: wait or Ctrl-C to cancel"
            } else {
                &format!(
                    "{}   {}   {}",
                    if *choice == TransactionExitChoice::Commit && !commit_disabled {
                        "[Commit]"
                    } else {
                        " Commit "
                    },
                    if *choice == TransactionExitChoice::Rollback {
                        "[Rollback]"
                    } else {
                        " Rollback "
                    },
                    " Cancel "
                )
            };
            let lines = vec![
                Line::from(Span::styled(" TRANSACTION EXIT ", theme.title(true))),
                Line::raw(format!(
                    "console: {}   state: {state}",
                    tab.map(|tab| tab.title()).unwrap_or("unknown")
                )),
                Line::raw(buttons),
                Line::raw(
                    "Rollback is the default. Tab/Left/Right choose; Enter confirms; Esc cancels",
                ),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(" TRANSACTION EXIT CONFIRMATION ", true, theme))
                    .style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                popup,
            );
        }
        Overlay::ClearTransactionOutcome { .. } => {
            let popup = centered(area, 78, 12);
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(" VERIFY UNKNOWN OUTCOME ", theme.title(true))),
                    Line::raw("LazyDB cannot know whether commit/rollback reached the server."),
                    Line::raw("Verify externally before clearing this transaction state."),
                    Line::raw("Cancel (default)   [Clear after verification]"),
                    Line::raw("Enter confirms clear; Esc cancels"),
                ])
                .block(panel_block(" TRANSACTION OUTCOME UNKNOWN ", true, theme))
                .style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                popup,
            );
        }
        Overlay::TargetSelector {
            candidates,
            selected,
        } => {
            let height = (candidates.len() as u16).saturating_add(6).clamp(8, 24);
            let popup = centered(area, 68, height);
            frame.render_widget(Clear, popup);
            let current = app
                .active_console_opt()
                .and_then(|tab| tab.execution_target.as_ref());
            let mut lines = vec![Line::from(Span::styled(
                " EXECUTION TARGET ",
                theme.title(true),
            ))];
            lines.extend(candidates.iter().enumerate().map(|(index, target)| {
                let marker = if index == *selected { ">" } else { " " };
                let current_marker = if current == Some(target) {
                    " current"
                } else {
                    ""
                };
                let label = format!(
                    "{marker} {}{}{}",
                    target.database,
                    target
                        .schema
                        .as_deref()
                        .map(|schema| format!(".{schema}"))
                        .unwrap_or_default(),
                    current_marker,
                );
                Line::from(Span::styled(
                    label,
                    if index == *selected {
                        Style::new()
                            .fg(theme.text)
                            .bg(theme.selection)
                            .add_modifier(Modifier::BOLD)
                    } else if current == Some(target) {
                        Style::new().fg(theme.accent)
                    } else {
                        Style::new().fg(theme.text)
                    },
                ))
            }));
            lines.push(Line::raw(""));
            lines.push(Line::raw(
                "j/k or Up/Down select  Enter confirm  Esc cancel",
            ));
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(" TARGET SELECTOR ", true, theme))
                    .style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                popup,
            );
        }
    }
}

fn render_execution_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    draft: &crate::sql::ExecutionDraft,
    focus: crate::model::workspace::ExecutionConfirmFocus,
    app: &App,
    theme: Theme,
) {
    use crate::model::workspace::ExecutionConfirmFocus;

    let popup = centered(area, 86, 24);
    frame.render_widget(Clear, popup);
    let risk = draft
        .risks
        .iter()
        .map(|risk| format!("{risk:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let database = app
        .active_profile()
        .map(|profile| profile.name.as_str())
        .unwrap_or("disconnected");
    let mut lines = vec![
        Line::from(Span::styled(" EXECUTE SQL? ", theme.title(true))),
        Line::raw(format!(
            "scope: {:?}   lines: {}   statements: {}",
            draft.scope,
            draft.sql.lines().count().max(1),
            draft.statement_count
        )),
        Line::raw(format!("risk: {risk}   database: {database}")),
        Line::raw(format!(
            "transaction: {:?}/{:?}",
            draft.transaction_mode, draft.transaction_state
        )),
    ];
    if draft.dialect == crate::sql::SqlDialect::MySql
        && draft.transaction_mode == crate::model::transaction::TransactionMode::Manual
        && draft.risks.contains(&crate::sql::SqlRisk::Ddl)
    {
        lines.push(Line::from(Span::styled(
            "WARNING: MySQL DDL may implicitly commit before and after execution",
            Style::new().fg(theme.warning).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::raw(""));
    let sanitized_sql = sanitize_terminal_text(&draft.sql);
    lines.extend(sanitized_sql.lines().map(Line::raw));
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!(
        "{}   {}   (Tab/Left/Right focus)",
        if focus == ExecutionConfirmFocus::Cancel {
            "[Cancel]"
        } else {
            " Cancel "
        },
        if focus == ExecutionConfirmFocus::Execute {
            "[Execute]"
        } else {
            " Execute "
        }
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" EXECUTION CONFIRMATION ", true, theme))
            .style(Style::new().fg(theme.text).bg(theme.surface_raised))
            .scroll((0, 0))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_substitute_confirm(frame: &mut Frame<'_>, area: Rect, remaining: usize, theme: Theme) {
    let popup = centered(area, 56, 7);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(" SUBSTITUTE CONFIRM ", theme.title(true))),
            Line::raw(format!("{remaining} match(es) remaining")),
            Line::raw("y yes   n no   a all   l yes and stop   q quit"),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .style(Style::new().fg(theme.text).bg(theme.surface)),
        ),
        popup,
    );
}

fn render_help(
    frame: &mut Frame<'_>,
    area: Rect,
    help: &crate::help::HelpState,
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    let focus = help.context;
    let popup = centered(area, 74, area.height.saturating_sub(2).clamp(12, 28));
    frame.render_widget(Clear, popup);
    let title = format!(" KEYMAP // {} ", focus_name(focus));
    let block = Block::default()
        .title(title)
        .title_style(theme.title(true))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.accent))
        .style(Style::new().bg(theme.surface_raised));
    frame.render_widget(block, popup);
    let relation_data = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(tab))
            if tab.view == crate::model::relation::RelationView::Data
    );
    let entries = crate::help::filtered_shortcuts(focus, relation_data, &help.query);
    let inner = Block::default().borders(Borders::ALL).inner(popup);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(format!("Search {query}", query = help.query))
            .style(Style::new().fg(theme.accent).bg(theme.surface_raised)),
        chunks[0],
    );
    let visible_height = chunks[2].height as usize;
    let start = if visible_height == 0 {
        0
    } else {
        help.selected
            .saturating_sub(visible_height.saturating_sub(1))
    };
    let rows = entries
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_height)
        .map(|(index, shortcut)| {
            let marker = if index == help.selected { ">" } else { " " };
            Line::from(vec![
                Span::styled(
                    format!("{marker} {:<18}", shortcut.key),
                    Style::new().fg(theme.action).add_modifier(Modifier::BOLD),
                ),
                Span::styled(shortcut.description, Style::new().fg(theme.text)),
            ])
        })
        .collect::<Vec<_>>();
    let list = if rows.is_empty() {
        vec![Line::from(Span::styled(
            "No matching shortcuts",
            Style::new().fg(theme.muted),
        ))]
    } else {
        rows
    };
    frame.render_widget(
        Paragraph::new(list)
            .style(Style::new().fg(theme.text).bg(theme.surface_raised))
            .wrap(Wrap { trim: false }),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new("Up/Down select   Enter run   Esc close   Ctrl-u clear")
            .style(Style::new().fg(theme.muted).bg(theme.surface_raised)),
        chunks[3],
    );
    state.cursor_style = Some(CursorStyle::Bar);
    let cursor_x = chunks[0]
        .x
        .saturating_add("Search ".cell_width())
        .saturating_add(help.query.cell_width())
        .min(chunks[0].right().saturating_sub(1));
    frame.set_cursor_position(Position::new(cursor_x, chunks[0].y));
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

fn connection_status_spans(
    status: ExplorerConnectionStatus,
    theme: Theme,
    selected: bool,
) -> Vec<Span<'static>> {
    let background = if selected {
        theme.selection
    } else {
        theme.surface
    };
    let (marker, text, color) = match status {
        ExplorerConnectionStatus::Online => ("●", "", theme.accent),
        ExplorerConnectionStatus::Offline => ("○", "", theme.muted),
        ExplorerConnectionStatus::Linking => ("◐", " CONNECTING", theme.warning),
        ExplorerConnectionStatus::Syncing => ("◐", " SYNCING", theme.action),
        ExplorerConnectionStatus::Failed => ("●", " FAILED", theme.error),
    };
    vec![Span::styled(
        format!("  {marker}{text}"),
        Style::new().fg(color).bg(background),
    )]
}

fn kind_color(kind: CatalogKind, theme: Theme) -> Color {
    match kind {
        CatalogKind::Database | CatalogKind::Schema => theme.action,
        CatalogKind::Table | CatalogKind::View | CatalogKind::MaterializedView => theme.text,
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

#[cfg(test)]
mod completion_popup_tests {
    use super::*;

    #[test]
    fn places_popup_below_the_cursor_when_it_fits() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(12, 6),
        };

        assert_eq!(
            completion_popup_rect(anchor, 20, 4),
            Some(Rect::new(12, 7, 20, 4))
        );
    }

    #[test]
    fn places_popup_above_the_cursor_when_below_is_too_short() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(12, 13),
        };

        assert_eq!(
            completion_popup_rect(anchor, 20, 4),
            Some(Rect::new(12, 9, 20, 4))
        );
    }

    #[test]
    fn clamps_popup_to_the_text_viewport() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(48, 6),
        };

        assert_eq!(
            completion_popup_rect(anchor, 20, 4),
            Some(Rect::new(30, 7, 20, 4))
        );
    }
}
