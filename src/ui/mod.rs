pub mod animation;
pub mod data_grid;
pub mod icons;
pub mod layout;
pub mod loading;
pub mod notifications;
pub mod pagination;
pub mod profiles;
pub mod query_bar;
pub mod record_view;
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

fn pack_hints<S: AsRef<str>>(hints: &[S], width: u16) -> String {
    let width = usize::from(width);
    if width == 0 || hints.is_empty() {
        return String::new();
    }

    // Select against the final omission count, not the provisional count for
    // each item. This keeps the marker width accurate as items are added.
    let mut selected = 0;
    for count in (0..=hints.len()).rev() {
        let visible = hints[..count]
            .iter()
            .map(|hint| hint.as_ref().to_owned())
            .collect::<Vec<_>>();
        let omitted = hints.len() - count;
        let marker = (omitted > 0).then(|| format!("... (+{omitted})"));
        let mut candidate = visible.join("   ");
        if let Some(marker) = marker {
            if !candidate.is_empty() {
                candidate.push_str("   ");
            }
            candidate.push_str(&marker);
        }
        if usize::from(candidate.cell_width()) <= width {
            selected = count;
            break;
        }
    }

    let visible = hints[..selected]
        .iter()
        .map(|hint| hint.as_ref().to_owned())
        .collect::<Vec<_>>();
    let omitted = hints.len() - selected;
    let mut result = visible.join("   ");
    if omitted > 0 {
        let marker = format!("... (+{omitted})");
        if !result.is_empty() {
            result.push_str("   ");
        }
        let remaining = width.saturating_sub(usize::from(result.cell_width()));
        if remaining >= 3 {
            result.push_str(&truncate_to_cells(&marker, remaining));
            if usize::from(result.cell_width()) > width {
                result = truncate_to_cells(&result, width);
            }
        } else if result.is_empty() {
            result = truncate_to_cells(&marker, width);
        }
    }
    result
}

fn truncate_to_cells(value: &str, width: usize) -> String {
    let mut used = 0;
    value
        .chars()
        .take_while(|character| {
            let character_width = character.width().unwrap_or(0);
            if used + character_width > width {
                false
            } else {
                used += character_width;
                true
            }
        })
        .collect()
}

fn footer_hint_width(mode_badge: &str, area_width: u16) -> u16 {
    area_width.saturating_sub(mode_badge.cell_width().saturating_add(2))
}
use uuid::Uuid;

use crate::{
    app::App,
    cli::MotionMode,
    db::{catalog::CatalogKind, query::ResultSet},
    model::{
        editor::{EditorHighlightKind, EditorMode, EditorViewport},
        explorer::{ExplorerConnectionStatus, ProfileProvenance},
        profile_manager::ProfileField,
        relation::RelationLoad,
        tab::{DataGridViewport, ResultView, WorkspaceTab},
        workspace::{
            ConnectionStatus, ExplorerSearchPhase, Focus, Overlay, PaneLayoutMetrics, QueryStatus,
            VisibleCatalogNode,
        },
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
    CloseTab(Uuid),
    ExplorerRow(crate::model::explorer::ExplorerNodeId),
    ResultCell {
        row: usize,
        column: usize,
    },
    Help,
    ToggleResultView,
    ResultView(ResultView),
    RelationView(crate::model::relation::RelationView),
    RelationRetry,
    RelationCancel,
    DataQueryInput(crate::model::data_query::DataQueryInput),
    RelationColumnResize {
        column: usize,
        width: u16,
    },
    GridScrollbarThumb {
        track_x: u16,
        track_width: u16,
        thumb_x: u16,
        thumb_width: u16,
        offset: usize,
        max_offset: usize,
    },
    GridScrollbarPage {
        offset: usize,
    },
    HeaderProfile,
    ProfileField(ProfileField),
    ProfileDriver(crate::profile::DatabaseKind),
    ProfileToggle(ProfileField),
    ProfileScopeRow(String),
    ProfileButton(ProfileButton),
    DismissNotification(u64),
    RelationFirstPage,
    RelationPreviousPage,
    RelationPageSize,
    RelationNextPage,
    RelationLastPage,
    ResultFirstPage,
    ResultPreviousPage,
    ResultPageSize,
    ResultNextPage,
    ResultLastPage,
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
    pub grid_viewport: Option<DataGridViewport>,
    pub grid_horizontal_scroll: Option<GridHorizontalScrollTargets>,
    pub record_view_fields: Option<(Uuid, usize)>,
    pub explorer_viewport_rows: Option<usize>,
    pub ddl_viewport: Option<DdlViewportMetrics>,
    pub cursor_style: Option<CursorStyle>,
    pub pane_layout: PaneLayoutMetrics,
    pub click_tracker: RefCell<Option<(crate::model::explorer::ExplorerNodeId, Instant)>>,
    pub relation_resize: RefCell<Option<(usize, u16, u16)>>,
    pub grid_scrollbar_drag: RefCell<Option<GridScrollbarDrag>>,
    pub(crate) animations: animation::AnimationState,
    pub(crate) result_area: Option<Rect>,
    pub(crate) activity_icons: icons::IconSet,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DdlViewportMetrics {
    pub visible_rows: usize,
    pub visible_columns: usize,
    pub total_rows: usize,
    pub max_line_width: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridScrollbarDrag {
    pub track_x: u16,
    pub track_width: u16,
    pub thumb_width: u16,
    pub pointer_offset: u16,
    pub max_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridHorizontalScrollTarget {
    pub offset: usize,
    pub first_visible: usize,
    pub last_visible: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridHorizontalScrollTargets {
    pub left: GridHorizontalScrollTarget,
    pub right: GridHorizontalScrollTarget,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self::with_motion(MotionMode::Full)
    }

    pub fn with_motion(mode: MotionMode) -> Self {
        Self {
            hit_regions: Vec::new(),
            editor_viewport: None,
            completion_popup: None,
            grid_viewport: None,
            grid_horizontal_scroll: None,
            record_view_fields: None,
            explorer_viewport_rows: None,
            ddl_viewport: None,
            cursor_style: None,
            pane_layout: PaneLayoutMetrics::default(),
            click_tracker: RefCell::new(None),
            relation_resize: RefCell::new(None),
            grid_scrollbar_drag: RefCell::new(None),
            animations: animation::AnimationState::new(mode, Instant::now()),
            result_area: None,
            activity_icons: icons::IconSet::default(),
        }
    }

    pub(crate) fn animation_mode(&self) -> MotionMode {
        self.animations.mode()
    }

    pub(crate) fn observe_animations(&mut self, app: &App, now: Instant) {
        self.animations.set_now(now);
        self.animations.observe(animation_observation(app));
    }

    pub(crate) fn profile_scope_loading_elapsed(&self, request_id: u64) -> Duration {
        self.animations
            .elapsed(&animation::LoadIdentity::ProfileScope { request_id })
            .unwrap_or_default()
    }

    pub(crate) fn advance_animations(&mut self, now: Instant) -> bool {
        self.animations.advance(now)
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
    render_with_state_using_icons_and_sequence(frame, app, state, icons, None);
}

pub fn render_with_state_using_icons_and_sequence(
    frame: &mut Frame<'_>,
    app: &App,
    state: &mut UiState,
    icons: icons::IconSet,
    sequence: Option<&crate::input::keymap::KeySequenceState>,
) {
    let theme = Theme::default();
    let area = frame.area();
    state.activity_icons = icons;
    state.observe_animations(app, Instant::now());
    frame.render_widget(Block::new().style(theme.base()), area);
    let is_relation = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(_))
    );
    let layout = AppLayout::calculate(area, app.focus, is_relation, app.pane_sizes);
    state.pane_layout = layout.pane_metrics;
    state.hit_regions.clear();
    state.editor_viewport = None;
    state.completion_popup = None;
    state.grid_viewport = None;
    state.grid_horizontal_scroll = None;
    state.record_view_fields = None;
    state.explorer_viewport_rows = None;
    state.ddl_viewport = None;
    state.cursor_style = None;
    state.result_area = None;

    if layout.mode == LayoutMode::TooSmall {
        render_too_small(frame, area, theme);
        return;
    }

    render_header(frame, layout.header, app, theme, state);
    if let Some(area) = layout.tabs {
        render_tabs(frame, area, app, theme, state, icons);
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
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Results),
            });
            relation::render(frame, area, app, theme, state);
        }
        render_footer(frame, layout.footer, app, theme, sequence);
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
            render_result_tabs(frame, area, app, theme, state);
        }
        if let Some(area) = layout.results {
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Results),
            });
            render_results(frame, area, app, theme, state);
        }
        render_footer(frame, layout.footer, app, theme, sequence);
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
    }

    if let Some(overlay) = &app.overlay {
        state
            .animations
            .prepare_overlay(overlay_key(overlay), centered(area, 80, 20));
        dim_background(frame, area, theme);
        render_overlay(frame, area, overlay, app, state, theme, icons);
        state.animations.render_effect(frame, Instant::now());
    } else {
        state.animations.clear_overlay();
        if state.animations.take_result_ready().is_some()
            && let Some(result_area) = state.result_area
        {
            state
                .animations
                .start_effect(animation::EffectKind::Result, result_area);
        }
        state.animations.render_effect(frame, Instant::now());
    }
    if let Some(sequence) = sequence {
        render_key_sequence_popup(frame, area, app, theme, sequence);
    }
    if app.overlay.is_none() {
        notifications::render(frame, area, app, theme, state, icons);
    }
}

fn render_key_sequence_popup(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    sequence: &crate::input::keymap::KeySequenceState,
) {
    let shortcuts = crate::help::prefix_shortcuts(
        crate::help::shortcut_context(app),
        crate::help::shortcut_capabilities(app),
        sequence.prefix,
    );
    if shortcuts.is_empty() || area.width < 16 || area.height < 5 {
        return;
    }

    let columns = if area.width >= 100 {
        3
    } else if area.width >= 60 {
        2
    } else {
        1
    };
    let rows = shortcuts.len().div_ceil(columns);
    let height = (rows as u16)
        .saturating_add(2)
        .min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x.saturating_add(1),
        area.bottom().saturating_sub(2).saturating_sub(height),
        area.width.saturating_sub(2),
        height,
    );
    let inner_width = popup.width.saturating_sub(2);
    let column_width = usize::from(inner_width) / columns;
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut spans = Vec::new();
        for column in 0..columns {
            let index = column * rows + row;
            let Some(shortcut) = shortcuts.get(index) else {
                continue;
            };
            let suffix = shortcut.suffix.unwrap_or("");
            let key_width = shortcuts
                .iter()
                .filter_map(|shortcut| shortcut.suffix)
                .map(UnicodeWidthStr::width)
                .max()
                .unwrap_or(1);
            let label = format!(
                " {:key_width$}  {}",
                suffix,
                shortcut.description,
                key_width = key_width
            );
            let label = truncate_to_cells(&label, column_width.saturating_sub(1));
            let padding = column_width.saturating_sub(usize::from(label.cell_width()));
            let key_end = 1 + key_width.min(label.len().saturating_sub(1));
            let (key, description) = label.split_at(key_end.min(label.len()));
            let background = (index == sequence.selected).then_some(theme.selection);
            spans.push(Span::styled(
                key.to_owned(),
                Style::new()
                    .fg(theme.accent)
                    .bg(background.unwrap_or(theme.surface_raised))
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                description.to_owned(),
                Style::new()
                    .fg(theme.text)
                    .bg(background.unwrap_or(theme.surface_raised)),
            ));
            spans.push(Span::styled(
                " ".repeat(padding),
                Style::new().bg(background.unwrap_or(theme.surface_raised)),
            ));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::new().fg(theme.text).bg(theme.surface_raised))
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(theme.accent))
                    .title(format!(
                        " {}  Up/Down select  Enter run  Esc/Ctrl-C cancel ",
                        sequence.display
                    )),
            ),
        popup,
    );
}

fn overlay_key(overlay: &Overlay) -> u8 {
    match overlay {
        Overlay::Help(_) => 1,
        Overlay::NotificationHistory(_) => 17,
        Overlay::RecordView(_) => 2,
        Overlay::ProfileManager => 3,
        Overlay::ProfileAccess { .. } => 4,
        Overlay::Message { .. } => 5,
        Overlay::SubstituteConfirm { .. } => 6,
        Overlay::ExecutionConfirm { .. } => 7,
        Overlay::ManualCancelConfirm { .. } => 8,
        Overlay::TransactionExitConfirm { .. } => 9,
        Overlay::RelationTransactionConfirm { .. } => 10,
        Overlay::ClearTransactionOutcome { .. } => 11,
        Overlay::TargetSelector { .. } => 12,
        Overlay::DeleteConsole { .. } => 13,
        Overlay::SqlEditorList(_) => 14,
        Overlay::PageSizeSelector { .. } => 15,
        Overlay::CatalogDropConfirm { .. } => 16,
    }
}

fn dim_background(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let buffer = frame.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buffer[(x, y)];
            if cell.fg == theme.accent || cell.bg == theme.surface_raised {
                continue;
            }
            cell.set_fg(theme.muted);
            cell.set_bg(theme.background);
        }
    }
}

fn animation_observation(app: &App) -> animation::AnimationObservation {
    let mut observation = animation::AnimationObservation::default();
    if let Some((request_id, _)) = app
        .profile_manager
        .as_ref()
        .and_then(|manager| manager.scope_discovery_request)
    {
        observation
            .active_loads
            .insert(animation::LoadIdentity::ProfileScope { request_id });
    }
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return observation;
    };

    match tab {
        WorkspaceTab::Sql(tab) => {
            if tab.query_status == QueryStatus::Running {
                observation
                    .active_loads
                    .insert(animation::LoadIdentity::Query {
                        tab_id: tab.id,
                        generation: tab.generation,
                    });
            }
            if let Some(derived) = &tab.derived {
                if derived.running {
                    observation
                        .active_loads
                        .insert(animation::LoadIdentity::Derived {
                            tab_id: tab.id,
                            generation: derived.generation,
                        });
                } else if derived.outcome.is_some() {
                    observation.result = Some(animation::ResultIdentity::Derived {
                        tab_id: tab.id,
                        generation: derived.generation,
                    });
                }
            }
            if observation.result.is_none() && tab.outcome.is_some() {
                observation.result = Some(animation::ResultIdentity::Query {
                    tab_id: tab.id,
                    generation: tab.generation,
                });
            }
        }
        WorkspaceTab::Relation(tab) => {
            if let RelationLoad::Loading { request, .. } = &tab.data {
                observation
                    .active_loads
                    .insert(animation::LoadIdentity::Relation(request.clone()));
            }
            if let RelationLoad::Loading { request, .. } = &tab.ddl {
                observation
                    .active_loads
                    .insert(animation::LoadIdentity::Relation(request.clone()));
            }
        }
    }
    observation
}

pub(crate) fn render_text_input(
    frame: &mut Frame<'_>,
    area: Rect,
    prefix: &str,
    input: &crate::model::text_input::TextInput,
    style: Style,
    state: &mut UiState,
) -> Option<Position> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let projection = crate::security::project_editor_line(input.value());
    let prefix_width = prefix.width();
    let available = usize::from(area.width).saturating_sub(prefix_width);
    let cursor_cells = projection
        .source_to_display_cells
        .get(input.cursor())
        .copied()
        .unwrap_or_else(|| projection.text.width());
    let offset = cursor_cells
        .saturating_sub(available.saturating_sub(1))
        .min(projection.text.width());
    let mut visible = String::new();
    let mut cells = 0;
    for character in projection.text.chars() {
        let width = character.width().unwrap_or(0);
        let end = cells + width;
        if end > offset && cells < offset + available {
            visible.push(character);
        }
        cells = end;
        if cells >= offset + available {
            break;
        }
    }
    frame.render_widget(
        Paragraph::new(format!("{prefix}{visible}")).style(style),
        area,
    );
    state.cursor_style = Some(CursorStyle::Bar);
    let cursor_x = area
        .x
        .saturating_add(prefix_width as u16)
        .saturating_add(cursor_cells.saturating_sub(offset) as u16)
        .min(area.right().saturating_sub(1));
    let cursor = Position::new(cursor_x, area.y);
    frame.set_cursor_position(cursor);
    Some(cursor)
}

// Relation pages are rendered by `ui::relation`; keeping them out of the SQL path
// prevents accidental editor access when a relation tab is active.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompletionAnchor {
    pub(crate) viewport: Rect,
    pub(crate) cursor: Position,
    pub(crate) replacement_start_x: Option<u16>,
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
    let icon_width = popup
        .candidates
        .iter()
        .map(|candidate| icons.completion(candidate.kind).cell_width())
        .max()
        .unwrap_or(0);
    let label_offset = icon_width.saturating_add(1);
    let desired_height = popup.candidates.len().min(10) as u16;
    let desired_width = popup
        .candidates
        .iter()
        .map(|candidate| {
            let detail = candidate.detail.as_deref().unwrap_or("");
            label_offset
                .saturating_add(candidate.label.as_str().cell_width())
                .saturating_add(if detail.is_empty() {
                    0
                } else {
                    2u16.saturating_add(detail.cell_width())
                })
                .saturating_add(1)
        })
        .max()
        .unwrap_or(4)
        .max(4);
    let popup_x = anchor
        .replacement_start_x
        .map(|label_x| label_x.saturating_sub(label_offset))
        .unwrap_or(anchor.cursor.x);
    let layout_anchor = CompletionAnchor {
        cursor: Position::new(popup_x, anchor.cursor.y),
        replacement_start_x: None,
        ..anchor
    };
    let Some(area) = completion_popup_rect(layout_anchor, desired_width, desired_height) else {
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
            let icon = icons.completion(candidate.kind);
            let icon_padding =
                " ".repeat(usize::from(icon_width.saturating_sub(icon.cell_width())));
            let row_style = if index == popup.selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{icon_padding}{icon} "), row_style),
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

pub(crate) fn render_data_query_completion_popup(
    frame: &mut Frame<'_>,
    completion: &crate::model::data_query::DataQueryCompletion,
    theme: Theme,
    state: &mut UiState,
    anchor: CompletionAnchor,
) {
    if completion.candidates.is_empty() {
        return;
    }
    let desired_height = completion.candidates.len().min(10) as u16;
    let desired_width = completion
        .candidates
        .iter()
        .map(|candidate| {
            format!(
                "CL {}  {}",
                crate::security::sanitize_terminal_text(&candidate.name),
                crate::security::sanitize_terminal_text(
                    candidate.type_name.as_deref().unwrap_or_default()
                )
            )
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
    let items = completion
        .candidates
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, candidate)| {
            let selected = index == completion.selected;
            let row_style = if selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            let name = crate::security::sanitize_terminal_text(&candidate.name);
            let detail = candidate
                .type_name
                .as_deref()
                .map(crate::security::sanitize_terminal_text)
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::styled("CL ", row_style),
                Span::styled(name, row_style),
                Span::styled(
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!("  {detail}")
                    },
                    row_style.fg(if selected {
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

    let x = anchor
        .cursor
        .x
        .clamp(viewport.x, viewport.right().saturating_sub(1));
    let width = desired_width.min(viewport.right().saturating_sub(x)).max(1);
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

fn render_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut UiState,
    icons: icons::IconSet,
) {
    let mut spans = Vec::new();
    let mut x = area.x;
    for (index, tab) in app.tabs.iter().enumerate() {
        let title = sanitize_terminal_text(tab.title())
            .chars()
            .take(48)
            .collect::<String>();
        let icon = match tab {
            WorkspaceTab::Relation(tab) => icons.catalog(tab.descriptor.kind),
            WorkspaceTab::Sql(tab) => tab
                .execution_target
                .as_ref()
                .and_then(|target| {
                    app.profiles
                        .iter()
                        .find(|profile| profile.id == target.profile_id)
                })
                .or_else(|| app.active_profile())
                .map(|profile| icons.database(profile.kind))
                .or_else(|| {
                    app.connection
                        .server
                        .as_ref()
                        .map(|server| icons.database(server.kind))
                })
                .unwrap_or_else(|| icons.catalog(CatalogKind::Database)),
        };
        let label = format!(" {icon} {title} ");
        let close = format!("{} ", icons.close());
        let can_close = tab.as_console().is_none_or(|console| !console.is_default());
        let label_width = label.cell_width();
        let close_width = if can_close { close.cell_width() } else { 0 };
        let width = label_width + close_width;
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
        if can_close {
            spans.push(Span::styled(
                close,
                if active {
                    Style::new()
                        .fg(theme.background)
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(theme.muted).bg(theme.surface)
                },
            ));
        }
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                x,
                area.y,
                label_width.min(area.right().saturating_sub(x)),
                1,
            ),
            target: HitTarget::Tab(index),
        });
        let close_x = x.saturating_add(label_width);
        if can_close && close_x < area.right() {
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    close_x,
                    area.y,
                    close_width.min(area.right().saturating_sub(close_x)),
                    1,
                ),
                target: HitTarget::CloseTab(tab.id()),
            });
        }
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
    let tree_height = inner
        .height
        .saturating_sub(u16::from(app.explorer.find.is_some()));
    state.explorer_viewport_rows = Some(tree_height as usize);
    if let Some(find) = app.explorer.find.as_ref() {
        render_explorer_find(frame, inner, app, find, theme, state, icons);
        return;
    }
    if let Some(search) = app.explorer.search.as_ref() {
        render_explorer_search(frame, inner, app, search, theme, icons);
        return;
    }
    let viewport = app.explorer.viewport(inner.height as usize);
    if viewport.pinned.is_empty() && viewport.rows.is_empty() {
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

    let pinned_rows = viewport.pinned.len();
    let indicator_rows = usize::from(viewport.show_ancestor_indicator);
    let displayed = viewport.rows.iter().collect::<Vec<_>>();
    for (row, visible) in viewport.pinned.iter().enumerate() {
        state.hit_regions.push(HitRegion {
            area: Rect::new(inner.x, inner.y.saturating_add(row as u16), inner.width, 1),
            target: HitTarget::ExplorerRow(visible.id.clone()),
        });
    }
    for (row, visible) in displayed.iter().enumerate() {
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                inner.x,
                inner
                    .y
                    .saturating_add((pinned_rows + indicator_rows + row) as u16),
                inner.width,
                1,
            ),
            target: HitTarget::ExplorerRow(visible.id.clone()),
        });
    }
    let mut items = viewport
        .pinned
        .iter()
        .map(|visible| explorer_list_item(visible, app, theme, icons))
        .collect::<Vec<_>>();
    if viewport.show_ancestor_indicator {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  ⋮ {} ancestors", viewport.hidden_ancestor_count),
            Style::new().fg(theme.muted).bg(theme.surface),
        ))));
    }
    items.extend(
        displayed
            .into_iter()
            .map(|visible| explorer_list_item(visible, app, theme, icons)),
    );
    frame.render_widget(
        List::new(items).style(Style::new().bg(theme.surface)),
        inner,
    );
}

fn explorer_list_item(
    visible: &VisibleCatalogNode,
    app: &App,
    theme: Theme,
    icons: icons::IconSet,
) -> ListItem<'static> {
    let is_others = matches!(visible.id, crate::model::explorer::ExplorerNodeId::Others);
    let expanded = explorer_node_is_expanded(&visible.id, visible.connection_status, app);
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
            .add_modifier(if is_others {
                Modifier::empty()
            } else {
                Modifier::BOLD
            })
    } else if is_others {
        Style::new()
            .fg(theme.muted)
            .bg(theme.surface)
            .add_modifier(Modifier::DIM)
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
    } else if !is_others {
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
    } else if let Some(placement) = visible.placement {
        let label = match placement {
            crate::model::explorer::ProfilePlacement::CurrentProject => "  PROJECT",
            crate::model::explorer::ProfilePlacement::Global => "  GLOBAL",
            crate::model::explorer::ProfilePlacement::OtherProject => "  OTHER",
        };
        spans.push(Span::styled(label, secondary_style));
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
}

fn render_explorer_find(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    find: &crate::model::workspace::ExplorerFindState,
    theme: Theme,
    state: &mut UiState,
    icons: icons::IconSet,
) {
    if area.is_empty() {
        return;
    }
    let (current, total) = app.explorer.find_match_position();
    let query = format!("/ {}", sanitize_terminal_text(&find.query));
    let input = format!("{query} ({current}/{total})");
    frame.render_widget(
        Paragraph::new(input.clone()).style(Style::new().fg(theme.action).bg(theme.surface)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if find.phase == ExplorerSearchPhase::Editing {
        frame.set_cursor_position(Position::new(
            area.x
                .saturating_add(explorer_search_cursor_column(&query, area.width)),
            area.y,
        ));
    }
    let tree_area = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );
    let viewport = app.explorer.viewport(tree_area.height as usize);
    let pinned_rows = viewport.pinned.len();
    let indicator_rows = usize::from(viewport.show_ancestor_indicator);
    let rows = viewport.pinned.iter().chain(viewport.rows.iter());
    for (row, visible) in rows.clone().enumerate() {
        // Find rows use the same stable IDs as normal Explorer rows.
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                tree_area.x,
                tree_area.y.saturating_add(
                    (row + usize::from(row >= pinned_rows) * indicator_rows) as u16,
                ),
                tree_area.width,
                1,
            ),
            target: HitTarget::ExplorerRow(visible.id.clone()),
        });
    }
    let mut items = viewport
        .pinned
        .iter()
        .map(|visible| explorer_find_list_item(visible, app, find, theme, icons))
        .collect::<Vec<_>>();
    if viewport.show_ancestor_indicator {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  ⋮ {} ancestors", viewport.hidden_ancestor_count),
            Style::new().fg(theme.muted).bg(theme.surface),
        ))));
    }
    items.extend(
        rows.skip(pinned_rows)
            .map(|visible| explorer_find_list_item(visible, app, find, theme, icons))
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        List::new(items).style(Style::new().bg(theme.surface)),
        tree_area,
    );
}

fn explorer_find_list_item(
    visible: &VisibleCatalogNode,
    app: &App,
    find: &crate::model::workspace::ExplorerFindState,
    theme: Theme,
    icons: icons::IconSet,
) -> ListItem<'static> {
    let selected = app.explorer.selected_id() == Some(&visible.id);
    let background = if selected {
        theme.selection
    } else {
        theme.surface
    };
    let base_style = Style::new()
        .fg(if selected { theme.accent } else { theme.text })
        .bg(background);
    let expanded = explorer_node_is_expanded(&visible.id, visible.connection_status, app);
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
    let label = find
        .rows
        .iter()
        .find(|row| row.id == visible.id)
        .map(|row| row.label.as_str())
        .unwrap_or(&visible.label);
    let mut spans = vec![Span::styled(
        format!("{}{} {} ", "  ".repeat(visible.depth), marker, icon),
        base_style,
    )];
    spans.extend(match_spans(
        sanitize_terminal_text(label),
        &find.query,
        base_style,
        Style::new()
            .fg(theme.action)
            .bg(background)
            .add_modifier(Modifier::BOLD),
    ));
    if let Some(metadata) = visible
        .metadata
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        spans.push(Span::styled(
            format!("  {}", sanitize_terminal_text(metadata)),
            Style::new().fg(theme.muted).bg(background),
        ));
    }
    if let Some(comment) = visible.comment.as_deref().filter(|value| !value.is_empty()) {
        spans.push(Span::styled(
            format!("  {}", sanitize_terminal_text(comment)),
            Style::new()
                .fg(theme.muted)
                .bg(background)
                .add_modifier(Modifier::DIM),
        ));
    }
    ListItem::new(Line::from(spans))
}

fn match_spans(text: String, query: &str, base: Style, matched: Style) -> Vec<Span<'static>> {
    let matches = crate::db::catalog::search_text_match_ranges(&text, query);
    if matches.is_empty() {
        return vec![Span::styled(text, base)];
    }
    let mut spans = Vec::new();
    let mut cursor = 0;
    for (start, end) in matches {
        if cursor < start {
            spans.push(Span::styled(text[cursor..start].to_owned(), base));
        }
        spans.push(Span::styled(text[start..end].to_owned(), matched));
        cursor = end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_owned(), base));
    }
    spans
}

fn render_explorer_search(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    search: &crate::model::workspace::ExplorerSearchState,
    theme: Theme,
    icons: icons::IconSet,
) {
    if area.is_empty() {
        return;
    }
    let input = format!("/ {}", sanitize_terminal_text(&search.query));
    frame.render_widget(
        Paragraph::new(input.clone()).style(
            Style::new()
                .fg(theme.action)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if search.phase == ExplorerSearchPhase::Editing {
        frame.set_cursor_position(Position::new(
            area.x
                .saturating_add(explorer_search_cursor_column(&input, area.width)),
            area.y,
        ));
    }

    let result_height = area.height.saturating_sub(2) as usize;
    let visible = app.explorer.visible_search();
    let start = search
        .scroll
        .max(
            search
                .selected
                .saturating_sub(result_height.saturating_sub(1)),
        )
        .min(visible.len().saturating_sub(1));
    let items = visible
        .iter()
        .enumerate()
        .skip(start)
        .take(result_height)
        .map(|(index, row)| {
            let selected = index == search.selected;
            let background = if selected {
                theme.selection
            } else {
                theme.surface
            };
            let expanded = explorer_node_is_expanded(&row.id, None, app);
            let marker = if row.expandable
                && !matches!(
                    row.kind,
                    Some(CatalogKind::Table | CatalogKind::View | CatalogKind::MaterializedView)
                ) {
                if expanded { "▾" } else { "▸" }
            } else {
                " "
            };
            let icon = match &row.id {
                crate::model::explorer::ExplorerNodeId::Group { group, .. } => {
                    icons.group(*group, expanded)
                }
                _ => row.kind.map_or("·", |kind| icons.catalog(kind)),
            };
            let label_style = Style::new()
                .fg(if selected { theme.accent } else { theme.text })
                .bg(background)
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });
            let mut spans = vec![Span::styled(
                format!("{}{} ", "  ".repeat(row.depth), marker),
                label_style,
            )];
            if let Some(kind) = row.profile_kind {
                spans.push(Span::styled(
                    format!("{} ", icons.database(kind)),
                    Style::new().fg(theme.action).bg(background),
                ));
            } else {
                spans.push(Span::styled(
                    format!("{} ", icon),
                    Style::new()
                        .fg(row.kind.map_or(theme.muted, |kind| kind_color(kind, theme)))
                        .bg(background),
                ));
            }
            spans.extend(match_spans(
                sanitize_terminal_text(&row.label),
                &search.query,
                label_style,
                Style::new()
                    .fg(theme.action)
                    .bg(background)
                    .add_modifier(Modifier::BOLD),
            ));
            if let Some(metadata) = row.metadata.as_deref().filter(|value| !value.is_empty()) {
                spans.push(Span::styled(
                    format!("  {}", sanitize_terminal_text(metadata)),
                    Style::new().fg(theme.muted).bg(background),
                ));
            }
            if let Some(comment) = row.comment.as_deref().filter(|value| !value.is_empty()) {
                spans.push(Span::styled(
                    format!("  {}", sanitize_terminal_text(comment)),
                    Style::new()
                        .fg(theme.muted)
                        .bg(background)
                        .add_modifier(Modifier::DIM),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    if !items.is_empty() {
        frame.render_widget(
            List::new(items).style(Style::new().bg(theme.surface)),
            Rect::new(
                area.x,
                area.y.saturating_add(1),
                area.width,
                result_height as u16,
            ),
        );
    } else if result_height > 0 {
        let message = match &search.lifecycle {
            crate::model::workspace::ExplorerSearchLifecycle::Idle => {
                "Type to search all objects".to_owned()
            }
            crate::model::workspace::ExplorerSearchLifecycle::Loading => {
                "Indexing catalog...".to_owned()
            }
            crate::model::workspace::ExplorerSearchLifecycle::Ready => {
                format!(
                    "No objects match \"{}\"",
                    sanitize_terminal_text(&search.query)
                )
            }
            crate::model::workspace::ExplorerSearchLifecycle::Failed(message) => {
                format!("{}  Ctrl+R retry", sanitize_terminal_text(message))
            }
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .wrap(Wrap { trim: true }),
            Rect::new(
                area.x,
                area.y.saturating_add(1),
                area.width,
                result_height as u16,
            ),
        );
    }

    if area.height > 1 {
        let status = match &search.lifecycle {
            crate::model::workspace::ExplorerSearchLifecycle::Loading => "Searching...".to_owned(),
            crate::model::workspace::ExplorerSearchLifecycle::Failed(message) => {
                if search.frontend_rows.is_empty() {
                    format!(
                        "Search failed: {}  Ctrl+R retry",
                        sanitize_terminal_text(message)
                    )
                } else {
                    format!(
                        "{} retained | {}",
                        search.frontend_rows.len(),
                        sanitize_terminal_text(message)
                    )
                }
            }
            _ => format!(
                "{} results  n/N next/prev  Enter locate  Esc close",
                search.frontend_match_rows.len()
            ),
        };
        frame.render_widget(
            Paragraph::new(status).style(Style::new().fg(theme.muted).bg(theme.surface)),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
}

fn explorer_node_is_expanded(
    id: &crate::model::explorer::ExplorerNodeId,
    connection_status: Option<ExplorerConnectionStatus>,
    app: &App,
) -> bool {
    if matches!(id, crate::model::explorer::ExplorerNodeId::Profile(_))
        && !matches!(
            connection_status.or_else(|| {
                app.explorer
                    .normalized
                    .profiles
                    .get(&id.profile_id()?)
                    .map(|profile| profile.status)
            }),
            Some(ExplorerConnectionStatus::Online | ExplorerConnectionStatus::Syncing)
        )
    {
        return false;
    }
    app.explorer.normalized.expanded.contains(id)
}

fn explorer_search_cursor_column(input: &str, width: u16) -> u16 {
    input.cell_width().min(width.saturating_sub(1))
}

fn source_byte_to_visible_cell(
    source: &str,
    source_to_display_cells: &[usize],
    byte: usize,
    horizontal_offset: usize,
    viewport_width: usize,
) -> Option<u16> {
    if viewport_width == 0 || byte > source.len() || !source.is_char_boundary(byte) {
        return None;
    }
    let column = source[..byte].chars().count();
    let cell = *source_to_display_cells.get(column)?;
    Some(
        cell.saturating_sub(horizontal_offset)
            .min(viewport_width.saturating_sub(1)) as u16,
    )
}

fn completion_replacement_start_cell(
    text: &str,
    snapshot: &crate::model::editor::EditorRenderSnapshot,
    replace: crate::sql::TextRange,
) -> Option<u16> {
    if replace.start > text.len() || !text.is_char_boundary(replace.start) {
        return None;
    }
    let line_start = text
        .split_inclusive('\n')
        .take(snapshot.cursor.line)
        .map(str::len)
        .sum::<usize>();
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |offset| line_start + offset);
    if replace.start < line_start || replace.start > line_end {
        return None;
    }
    let line = snapshot
        .lines
        .iter()
        .find(|line| line.line == snapshot.cursor.line)?;
    source_byte_to_visible_cell(
        &text[line_start..line_end],
        &line.source_to_display_cells,
        replace.start - line_start,
        snapshot.horizontal_offset,
        snapshot.viewport.width,
    )
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
    let replacement_start_x = app
        .active_console_opt()
        .and_then(|tab| tab.completion.as_ref())
        .and_then(|popup| popup.candidates.first())
        .and_then(|candidate| {
            let text = app.active_editor_text().ok()?;
            completion_replacement_start_cell(&text, &snapshot, candidate.replace)
        })
        .map(|x| text_viewport.x.saturating_add(x));
    let completion_anchor = snapshot
        .prompt
        .is_none()
        .then_some(snapshot.cursor_screen_cell)
        .flatten()
        .map(|(x, y)| CompletionAnchor {
            viewport: inner,
            cursor: Position::new(
                text_viewport.x.saturating_add(x),
                text_viewport.y.saturating_add(y),
            ),
            replacement_start_x,
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
                let foreground = theme.syntax_color(editor_syntax_color(span.kind));
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

pub(crate) fn editor_syntax_color(kind: EditorHighlightKind) -> theme::SyntaxColor {
    match kind {
        EditorHighlightKind::Keyword => theme::SyntaxColor::Keyword,
        EditorHighlightKind::Identifier => theme::SyntaxColor::Identifier,
        EditorHighlightKind::String => theme::SyntaxColor::String,
        EditorHighlightKind::Number => theme::SyntaxColor::Number,
        EditorHighlightKind::Comment => theme::SyntaxColor::Comment,
        EditorHighlightKind::Operator => theme::SyntaxColor::Operator,
        EditorHighlightKind::Punctuation => theme::SyntaxColor::Punctuation,
        EditorHighlightKind::Parameter => theme::SyntaxColor::Parameter,
        EditorHighlightKind::Plain => theme::SyntaxColor::Plain,
    }
}

pub(crate) fn editor_line_spans(
    line: &crate::model::editor::EditorRenderLine,
    snapshot: &crate::model::editor::EditorRenderSnapshot,
    theme: Theme,
    syntax: bool,
) -> Vec<Span<'static>> {
    let selected = snapshot
        .selection_cells
        .iter()
        .filter(|(selected_line, _, _)| *selected_line == line.line)
        .map(|(_, start, end)| (*start, *end))
        .collect::<Vec<_>>();
    let mut display_cell = 0usize;
    let mut result: Vec<Span<'static>> = Vec::new();
    for source_span in &line.spans {
        let foreground = if syntax {
            theme.syntax_color(editor_syntax_color(source_span.kind))
        } else {
            theme.text
        };
        for character in source_span.text.chars() {
            let width = character.width().unwrap_or(0);
            let highlighted = selected.iter().any(|(start, end)| {
                display_cell < *end && display_cell.saturating_add(width) > *start
            });
            let style = Style::new().fg(foreground).bg(if highlighted {
                theme.selection
            } else {
                theme.surface
            });
            if let Some(previous) = result.last_mut()
                && previous.style == style
            {
                previous.content.to_mut().push(character);
            } else {
                result.push(Span::styled(character.to_string(), style));
            }
            display_cell = display_cell.saturating_add(width);
        }
    }
    result
}

pub(crate) fn render_tab_selectors(
    frame: &mut Frame<'_>,
    area: Rect,
    labels: &[&str],
    active: usize,
    theme: Theme,
) -> Vec<Rect> {
    let mut spans = Vec::new();
    let mut regions = Vec::new();
    let mut x = area.x;
    for (index, label) in labels.iter().enumerate() {
        let text = format!(" {label} ");
        let width = text.cell_width() as u16;
        spans.push(Span::styled(
            text,
            Style::new()
                .fg(if index == active {
                    theme.accent
                } else {
                    theme.muted
                })
                .add_modifier(Modifier::BOLD),
        ));
        regions.push(Rect::new(
            x,
            area.y,
            width.min(area.right().saturating_sub(x)),
            1,
        ));
        x = x.saturating_add(width);
        if index + 1 < labels.len() {
            spans.push(Span::raw(" "));
            x = x.saturating_add(1);
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.base()), area);
    if let Some(region) = regions.get(active).copied() {
        frame.render_widget(
            Paragraph::new("━".repeat(usize::from(region.width)))
                .style(Style::new().fg(theme.accent)),
            Rect::new(region.x, area.y.saturating_add(1), region.width, 1),
        );
    }
    regions
}

fn render_result_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut UiState,
) {
    let active = app
        .active_console_opt()
        .map_or(ResultView::Data, |tab| tab.result_view);
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
    let regions = render_tab_selectors(
        frame,
        area,
        &["DATA", "OUTPUT"],
        usize::from(matches!(active, ResultView::Output | ResultView::Plan)),
        theme,
    );
    for (region, view) in regions
        .into_iter()
        .zip([ResultView::Data, ResultView::Output])
    {
        state.hit_regions.push(HitRegion {
            area: region,
            target: HitTarget::ResultView(view),
        });
    }
    let stats_x = area.x.saturating_add(18).min(area.right());
    frame.render_widget(
        Paragraph::new(stats.to_string()).style(Style::new().fg(theme.muted).bg(theme.background)),
        Rect::new(stats_x, area.y, area.right().saturating_sub(stats_x), 1),
    );
}

fn render_results(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
    match app
        .active_console_opt()
        .map_or(ResultView::Data, |tab| tab.result_view)
    {
        ResultView::Output | ResultView::Plan => render_output(frame, area, app, theme, state),
        ResultView::Data => render_data(frame, area, app, theme, state),
    }
}

fn render_data(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
    let Some(tab) = app.active_console_opt() else {
        return;
    };
    let block = panel_block(" RESULT SET ", app.focus == Focus::Results, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let query_height = query_bar::height(&tab.query, inner.width, state.activity_icons);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(query_height),
            Constraint::Min(2),
            Constraint::Length(1),
        ])
        .split(inner);
    let query_cursor = query_bar::render(
        frame,
        chunks[0],
        &tab.query,
        theme,
        state,
        state.activity_icons,
    );
    let result_area = chunks[1];
    let loading_identity = if tab.query_status == QueryStatus::Running {
        Some(animation::LoadIdentity::Query {
            tab_id: tab.id,
            generation: tab.generation,
        })
    } else if tab.derived.as_ref().is_some_and(|derived| derived.running) {
        tab.derived
            .as_ref()
            .map(|derived| animation::LoadIdentity::Derived {
                tab_id: tab.id,
                generation: derived.generation,
            })
    } else {
        None
    };
    let elapsed = loading_identity
        .as_ref()
        .and_then(|identity| state.animations.elapsed(identity))
        .unwrap_or_default();
    let result = tab
        .derived
        .as_ref()
        .and_then(|derived| derived.outcome.as_ref())
        .or(tab.outcome.as_ref())
        .and_then(|outcome| outcome.result_sets.last());
    if let Some(identity) = loading_identity {
        if let Some(result) = result {
            let body = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(result_area);
            frame.render_widget(
                loading::ActivityIndicator {
                    mode: state.animation_mode(),
                    icons: state.activity_icons,
                    elapsed,
                    label: "Executing query",
                    detail: Some("showing previous result"),
                    cancellable: true,
                    style: Style::new().fg(theme.action).bg(theme.surface_raised),
                },
                body[0],
            );
            render_result_table(
                frame,
                body[1],
                tab.id,
                result,
                tab.grid.clone(),
                theme,
                Block::default().style(Style::new().bg(theme.surface)),
                state,
            );
        } else if animation::show_skeleton(elapsed) {
            frame.render_widget(
                loading::TableSkeleton {
                    mode: state.animation_mode(),
                    icons: state.activity_icons,
                    elapsed,
                    theme,
                    block: Block::default().style(Style::new().bg(theme.surface)),
                },
                result_area,
            );
        } else {
            frame.render_widget(
                loading::ActivityIndicator {
                    mode: state.animation_mode(),
                    icons: state.activity_icons,
                    elapsed,
                    label: "Executing query",
                    detail: None,
                    cancellable: true,
                    style: Style::new().fg(theme.action).bg(theme.surface),
                },
                result_area,
            );
        }
        let _ = identity;
    } else if let Some(result) = result {
        render_result_table(
            frame,
            result_area,
            tab.id,
            result,
            tab.grid.clone(),
            theme,
            Block::default().style(Style::new().bg(theme.surface)),
            state,
        );
    } else {
        frame.render_widget(
            Paragraph::new("Run a query to populate the data viewport")
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(Alignment::Center),
            result_area,
        );
    }
    pagination::render(
        frame,
        chunks[2],
        tab.pagination,
        pagination::PaginationKind::Result,
        theme,
        state,
    );
    if let (Some(completion), Some(cursor)) = (&tab.query.completion, query_cursor) {
        render_data_query_completion_popup(
            frame,
            completion,
            theme,
            state,
            CompletionAnchor {
                viewport: area,
                cursor,
                replacement_start_x: None,
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_result_table(
    frame: &mut Frame<'_>,
    area: Rect,
    tab_id: Uuid,
    result: &ResultSet,
    grid: crate::model::tab::GridState,
    theme: Theme,
    block: Block<'_>,
    state: &mut UiState,
) {
    state.result_area = Some(area);
    let overrides = grid.column_widths.clone();
    let icons = state.activity_icons;
    data_grid::render(
        frame, area, tab_id, result, grid, &overrides, theme, block, state, None, icons,
    );
}

fn render_output(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme, state: &mut UiState) {
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
    let viewport = EditorViewport {
        width: inner.width.saturating_sub(3) as usize,
        height: inner.height as usize,
    };
    state.editor_viewport = Some(viewport);
    let Ok(snapshot) = app.active_output_editor_snapshot(viewport) else {
        return;
    };
    for (row, line) in snapshot.lines.iter().take(viewport.height).enumerate() {
        let y = inner.y.saturating_add(row as u16);
        let content = editor_line_spans(line, &snapshot, theme, true);
        frame.render_widget(
            Paragraph::new(Line::from(content))
                .style(Style::new().bg(theme.surface))
                .scroll((0, snapshot.horizontal_offset.min(u16::MAX as usize) as u16)),
            Rect::new(inner.x.saturating_add(3), y, viewport.width as u16, 1),
        );
    }
    if app.focus == Focus::Results
        && app.overlay.is_none()
        && let Some((x, y)) = snapshot.cursor_screen_cell
    {
        frame.set_cursor_position(Position::new(
            inner.x.saturating_add(3).saturating_add(x),
            inner.y.saturating_add(y),
        ));
        state.cursor_style = Some(CursorStyle::Block);
    }
}

fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    _sequence: Option<&crate::input::keymap::KeySequenceState>,
) {
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
    let context = crate::help::shortcut_context(app);
    let capabilities = crate::help::shortcut_capabilities(app);
    let hint_values = crate::help::footer_shortcuts(context, capabilities)
        .into_iter()
        .map(|shortcut| format!("{} {}", shortcut.sequence, shortcut.description))
        .collect::<Vec<_>>();
    let mode_badge = format!(" {mode} ");
    let hints = pack_hints(&hint_values, footer_hint_width(&mode_badge, area.width));
    let line = Line::from(vec![
        Span::styled(
            mode_badge,
            Style::new()
                .fg(theme.background)
                .bg(mode_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {hints}"),
            Style::new().fg(theme.muted).bg(theme.surface),
        ),
        Span::styled("", Style::new().bg(theme.surface)),
    ]);
    let relation_context = app.tabs.get(app.active_tab).and_then(|tab| match tab {
        WorkspaceTab::Relation(tab) if tab.view == crate::model::relation::RelationView::Ddl => {
            let ddl_provenance = match &tab.ddl {
                crate::model::relation::RelationLoad::Ready(snapshot)
                | crate::model::relation::RelationLoad::Loading {
                    previous: Some(snapshot),
                    ..
                }
                | crate::model::relation::RelationLoad::Failed {
                    previous: Some(snapshot),
                    ..
                }
                | crate::model::relation::RelationLoad::Cancelled {
                    previous: Some(snapshot),
                } => format!("DDL: {:?}", snapshot.value.provenance),
                _ => "DDL: NONE".to_owned(),
            };
            let snapshot = tab
                .provenance(
                    crate::model::relation::RelationView::Ddl,
                    app.connection.active_identity(),
                    app.active_profile(),
                )
                .map(crate::ui::relation::provenance_label)
                .unwrap_or("UNKNOWN");
            Some(format!(
                "{ddl_provenance}  Rows: {}  Cols: {}  Snapshot: {snapshot}",
                tab.ddl_viewport.row_offset.saturating_add(1),
                tab.ddl_viewport.column_offset.saturating_add(1)
            ))
        }
        _ => None,
    });
    let (second_text, second_color) = if let Some(context) = relation_context.as_deref() {
        (context, theme.muted)
    } else {
        ("Ready", theme.muted)
    };
    let second = Line::from(Span::styled(
        second_text,
        Style::new().fg(second_color).bg(theme.surface),
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
        Overlay::Help(help) => render_help(frame, area, help, state, theme),
        Overlay::NotificationHistory(history) => {
            notifications::render_history(frame, area, app, history, theme, state)
        }
        Overlay::RecordView(view) => record_view::render(frame, area, app, view, theme, state),
        Overlay::ProfileManager => {
            profiles::render_profile_manager(frame, area, app, state, theme, icons)
        }
        Overlay::ProfileAccess {
            profile_id,
            selected,
            options,
        } => render_profile_access(frame, area, app, *profile_id, *selected, options, theme),
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
            let pending = std::iter::once(prompt.console_id)
                .chain(
                    app.deferred_transaction_prompts()
                        .filter(|queued| queued.intent == prompt.intent)
                        .map(|queued| queued.console_id),
                )
                .collect::<Vec<_>>();
            let popup = centered(area, 78, (pending.len() as u16).saturating_add(7));
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
            let outcome_unknown = tab.is_some_and(|tab| {
                tab.as_console().is_some_and(|tab| {
                    tab.transaction_state
                        == crate::model::transaction::TransactionState::OutcomeUnknown
                })
            });
            let buttons = if running {
                "Query running: wait or Ctrl-C to cancel"
            } else if outcome_unknown {
                "[Abandon local state]   Cancel"
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
            let title = if prompt.intent == crate::model::transaction::DeferredIntent::Quit {
                " PENDING TRANSACTIONS "
            } else {
                " TRANSACTION "
            };
            let mut lines = vec![Line::from(Span::styled(title, theme.title(true)))];
            if pending.len() > 1 {
                for (index, id) in pending.iter().enumerate() {
                    let pending_tab = app.tabs.iter().find(|tab| tab.id() == *id);
                    let marker = if index == 0 { ">" } else { " " };
                    let pending_state = pending_tab
                        .and_then(|tab| tab.as_console())
                        .map(|tab| format!("{:?}", tab.transaction_state))
                        .unwrap_or_else(|| "gone".into());
                    lines.push(Line::raw(format!(
                        "{marker} {}   {pending_state}",
                        pending_tab.map(|tab| tab.title()).unwrap_or("unknown")
                    )));
                }
            } else {
                lines.push(Line::raw(format!(
                    "console: {}   state: {state}",
                    tab.map(|tab| tab.title()).unwrap_or("unknown")
                )));
            }
            lines.push(Line::raw(buttons));
            lines.push(Line::raw(if outcome_unknown {
                "Enter or 'a' abandons local state; Esc cancels"
            } else {
                "Rollback is the default. Tab/Left/Right choose; Enter confirms; Esc cancels"
            }));
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(title, true, theme))
                    .style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                popup,
            );
        }
        Overlay::RelationTransactionConfirm { tab_id, choice } => {
            use crate::model::transaction::TransactionExitChoice;
            let popup = centered(area, 78, 10);
            frame.render_widget(Clear, popup);
            let title = app
                .tabs
                .iter()
                .find(|tab| tab.id() == *tab_id)
                .map(|tab| tab.title())
                .unwrap_or("unknown");
            let commit = *choice == TransactionExitChoice::Commit;
            let lines = vec![
                Line::from(Span::styled(" TRANSACTION ", theme.title(true))),
                Line::raw(format!("console: {title}")),
                Line::raw(format!(
                    "{}   {}   Cancel",
                    if commit { "[Commit]" } else { " Commit " },
                    if !commit { "[Rollback]" } else { " Rollback " }
                )),
                Line::raw("Tab/Left/Right choose; Enter confirms; Esc cancels"),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(" TRANSACTION CONTROL ", true, theme))
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
        Overlay::PageSizeSelector { relation, selected } => {
            let sizes = pagination::selector_items();
            let popup = centered(area, 28, sizes.len() as u16 + 4);
            frame.render_widget(Clear, popup);
            let mut lines = vec![Line::from(Span::styled(" PAGE SIZE ", theme.title(true)))];
            lines.extend(sizes.iter().enumerate().map(|(index, size)| {
                Line::from(Span::styled(
                    format!(
                        "{} {}",
                        if index == *selected { ">" } else { " " },
                        size.get()
                    ),
                    if index == *selected {
                        theme.base().bg(theme.selection)
                    } else {
                        theme.base()
                    },
                ))
            }));
            lines.push(Line::raw("j/k select  Enter apply  Esc cancel"));
            frame.render_widget(
                Paragraph::new(lines).block(panel_block(
                    if *relation {
                        " RELATION PAGE SIZE "
                    } else {
                        " RESULT PAGE SIZE "
                    },
                    true,
                    theme,
                )),
                popup,
            );
        }
        Overlay::DeleteConsole { console_id } => {
            let popup = centered(area, 76, 8);
            frame.render_widget(Clear, popup);
            let name = app
                .sql_editors
                .iter()
                .find(|record| record.id == *console_id)
                .map_or("unknown", |record| record.name.as_str());
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(" DELETE SQL EDITOR? ", theme.title(true))),
                    Line::raw(format!(
                        "Permanently delete '{name}' and its saved SQL file?"
                    )),
                    Line::raw("Enter confirms; Esc cancels"),
                ])
                .block(panel_block(" DELETE CONFIRMATION ", true, theme))
                .style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                popup,
            );
        }
        Overlay::SqlEditorList(list) => {
            let popup = centered(area, 72, (app.sql_editors.len() as u16 + 5).clamp(8, 24));
            frame.render_widget(Clear, popup);
            let mut lines = vec![Line::raw(format!(" SQL EDITORS  search: {}", list.query))];
            lines.extend(
                app.sql_editors
                    .iter()
                    .filter(|record| {
                        crate::model::sql_editor_list::SqlEditorListState::matches(
                            &record.name,
                            &list.query,
                        )
                    })
                    .enumerate()
                    .map(|(index, record)| {
                        let marker = if index == list.selected { ">" } else { " " };
                        let open = if record.open { " OPEN" } else { " hidden" };
                        Line::raw(format!("{marker} {}{open}", record.name))
                    }),
            );
            lines.push(Line::raw("j/k select  Enter activate  Esc cancel"));
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(" SQL EDITOR LIST ", true, theme))
                    .style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                popup,
            );
        }
        Overlay::CatalogDropConfirm {
            plan,
            input,
            busy,
            error,
        } => {
            render_catalog_drop_confirm(frame, area, plan, input, *busy, error.as_deref(), theme);
        }
    }
}

fn render_catalog_drop_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &crate::db::catalog_drop::CatalogDropPlan,
    input: &crate::model::text_input::TextInput,
    busy: bool,
    error: Option<&str>,
    theme: Theme,
) {
    let popup = centered(area, 82, 16);
    frame.render_widget(Clear, popup);
    let state = if busy { "DROPPING..." } else { "" };
    let mut lines = vec![
        Line::from(Span::styled(
            format!(" DROP {:?} {state}", plan.kind).to_uppercase(),
            theme.title(true),
        )),
        Line::raw("This operation will execute:"),
        Line::raw(""),
        Line::raw(plan.sql()),
        Line::raw(""),
        Line::raw("This action cannot be undone."),
        Line::raw("Type exactly lowercase y and press Enter to execute:"),
        Line::from(Span::styled(
            format!("> {}", input.value()),
            Style::new().fg(theme.accent),
        )),
        Line::raw(if busy {
            "Execution in progress"
        } else {
            "Esc cancel"
        }),
    ];
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            crate::security::sanitize_terminal_text(error),
            Style::new().fg(theme.error),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" CATALOG DROP CONFIRMATION ", true, theme))
            .style(Style::new().fg(theme.text).bg(theme.surface_raised))
            .wrap(Wrap { trim: true }),
        popup,
    );
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
    state: &mut UiState,
    theme: Theme,
) {
    let popup = centered(area, 74, area.height.saturating_sub(2).clamp(12, 28));
    frame.render_widget(Clear, popup);
    let title = format!(" KEYMAP // {} ", crate::help::context_name(help.context));
    let block = Block::default()
        .title(title)
        .title_style(theme.title(true))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.accent))
        .style(Style::new().bg(theme.surface_raised));
    frame.render_widget(block, popup);
    let entries = crate::help::filtered_shortcuts(help.context, help.capabilities, &help.query);
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
                    format!("{marker} {:<18}", shortcut.sequence),
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

fn render_profile_access(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    profile_id: Uuid,
    selected: usize,
    options: &[crate::model::workspace::ProfileAccessOption],
    theme: Theme,
) {
    let name = app
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| crate::security::sanitize_terminal_text(&profile.name))
        .unwrap_or_else(|| "connection".to_owned());
    let popup = centered(area, 64, (options.len() as u16).saturating_add(7));
    frame.render_widget(Clear, popup);
    let mut lines = vec![
        Line::from(Span::styled(
            format!("Connection access · {name}"),
            theme.title(true),
        )),
        Line::raw(format!("Project: {}", app.project.display_name)),
        Line::raw(""),
    ];
    lines.extend(options.iter().enumerate().map(|(index, option)| {
        let marker = if index == selected { "> " } else { "  " };
        Line::from(Span::styled(
            format!(
                "{marker}{}",
                crate::security::sanitize_terminal_text(&option.label)
            ),
            if index == selected {
                Style::new()
                    .fg(theme.text)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            },
        ))
    }));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Enter apply   Esc close",
        Style::new().fg(theme.muted).bg(theme.surface_raised),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" ACCESS ", true, theme))
            .style(Style::new().bg(theme.surface_raised)),
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
mod footer_tests {
    use super::*;

    #[test]
    fn footer_hint_width_uses_the_rendered_badge_width() {
        assert_eq!(footer_hint_width(" NORMAL ", 40), 30);
        assert_eq!(footer_hint_width(" VISUAL LINE ", 40), 25);
        assert_eq!(footer_hint_width(" NORMAL ", 4), 0);
    }

    #[test]
    fn pack_hints_keeps_complete_units_and_reports_omissions() {
        assert_eq!(
            pack_hints(&["j/k move", "Enter open", "/ find"], 19),
            "j/k move   ... (+2)"
        );
        assert_eq!(
            pack_hints(&["j/k move", "Enter open", "/ find"], 40),
            "j/k move   Enter open   / find"
        );
    }

    #[test]
    fn pack_hints_measures_terminal_cells_and_handles_no_fit() {
        assert_eq!(
            pack_hints(&["界 move", "Enter open"], 18),
            "界 move   ... (+1)"
        );
        assert_eq!(pack_hints(&["long hint"], 3), "...");
        assert_eq!(pack_hints::<&str>(&[], 20), "");
        assert!(pack_hints(&["a", "b"], 0).is_empty());
        assert!(pack_hints(&["a", "b"], 2).cell_width() <= 2);
    }

    #[test]
    fn pack_hints_uses_the_final_omitted_count_before_selecting_units() {
        let packed = pack_hints(&["aaaa", "bb", "cc"], 15);
        assert_eq!(packed, "aaaa   bb   cc");
        assert!(packed.cell_width() <= 15);
    }
}

#[cfg(test)]
mod completion_popup_tests {
    use super::*;

    #[test]
    fn replacement_start_uses_display_cells() {
        let ascii = crate::security::project_editor_line("SELECT * FROM sys_u");
        assert_eq!(
            source_byte_to_visible_cell(
                "SELECT * FROM sys_u",
                &ascii.source_to_display_cells,
                14,
                0,
                40,
            ),
            Some(14)
        );

        let wide = crate::security::project_editor_line("界🙂 sys_u");
        assert_eq!(
            source_byte_to_visible_cell(
                "界🙂 sys_u",
                &wide.source_to_display_cells,
                "界🙂 ".len(),
                0,
                40,
            ),
            Some(5)
        );

        let tab = crate::security::project_editor_line("\tsys_u");
        assert_eq!(
            source_byte_to_visible_cell("\tsys_u", &tab.source_to_display_cells, 1, 0, 40),
            Some(4)
        );
    }

    #[test]
    fn replacement_start_accounts_for_horizontal_scroll_and_invalid_offsets() {
        let source = "SELECT * FROM sys_u";
        let projection = crate::security::project_editor_line(source);

        assert_eq!(
            source_byte_to_visible_cell(source, &projection.source_to_display_cells, 14, 10, 40,),
            Some(4)
        );
        assert_eq!(
            source_byte_to_visible_cell(source, &projection.source_to_display_cells, 4, 10, 40,),
            Some(0)
        );

        let wide = crate::security::project_editor_line("界sys_u");
        assert_eq!(
            source_byte_to_visible_cell("界sys_u", &wide.source_to_display_cells, 1, 0, 40),
            None
        );
        assert_eq!(
            source_byte_to_visible_cell(source, &projection.source_to_display_cells, 100, 0, 40),
            None
        );
    }

    #[test]
    fn explorer_search_cursor_uses_terminal_cell_width() {
        assert_eq!(explorer_search_cursor_column("/ 界", 20), 4);
        assert_eq!(explorer_search_cursor_column("/ 界", 4), 3);
    }

    #[test]
    fn explorer_find_cursor_stops_before_match_status() {
        let query = "/ time";
        let rendered = format!("{query} (1/4)");

        assert_eq!(explorer_search_cursor_column(query, 20), 6);
        assert_ne!(
            explorer_search_cursor_column(&rendered, 20),
            explorer_search_cursor_column(query, 20)
        );
    }

    #[test]
    fn places_popup_below_the_cursor_when_it_fits() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(12, 6),
            replacement_start_x: None,
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
            replacement_start_x: None,
        };

        assert_eq!(
            completion_popup_rect(anchor, 20, 4),
            Some(Rect::new(12, 9, 20, 4))
        );
    }

    #[test]
    fn keeps_popup_origin_and_shrinks_width_at_the_right_edge() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(48, 6),
            replacement_start_x: None,
        };

        assert_eq!(
            completion_popup_rect(anchor, 20, 4),
            Some(Rect::new(48, 7, 2, 4))
        );
    }

    #[test]
    fn clamps_popup_origin_to_the_viewport_left_edge() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(4, 6),
            replacement_start_x: None,
        };

        assert_eq!(
            completion_popup_rect(anchor, 20, 4),
            Some(Rect::new(10, 7, 20, 4))
        );
    }
}
