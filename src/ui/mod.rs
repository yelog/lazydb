pub mod animation;
pub mod catalog_editor;
pub(crate) mod dashboard;
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
mod shortcut_hints;
pub mod text_detail;
pub mod text_selection;
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

fn footer_hint_width(mode_badge: &str, area_width: u16) -> u16 {
    area_width.saturating_sub(mode_badge.cell_width().saturating_add(2))
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
            ConnectionStatus, ExplorerSearchPhase, Focus, Overlay, PaneLayoutMetrics, PaneSplit,
            QueryStatus, VisibleCatalogNode,
        },
    },
    security::sanitize_terminal_text,
};

use self::{
    layout::{AppLayout, LayoutMode},
    shortcut_hints::ShortcutHint,
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
    TabScrollLeft(usize),
    TabScrollRight(usize),
    CloseTab(Uuid),
    ExplorerRow(crate::model::explorer::ExplorerNodeId),
    ResultCell {
        row: usize,
        column: usize,
    },
    Help,
    UpdateCenter,
    UpdateButton {
        primary: bool,
    },
    ToggleResultView,
    ResultView(ResultView),
    RelationView(crate::model::relation::RelationView),
    DashboardView(crate::model::dashboard::DashboardPage),
    RelationRetry,
    RelationCancel,
    DataQueryInput(crate::model::data_query::DataQueryInput),
    PaneResize(PaneSplit),
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
    ProfileGroupOption(usize),
    ProfileGroupConfirm,
    ProfileGroupCancel,
    ExplorerAddOption(usize),
    CatalogEditorField(usize),
    CatalogEditorFormField(crate::model::catalog_editor::CatalogFormFocus),
    CatalogEditorTableField(crate::model::catalog_editor::TableEditorFocus),
    CatalogEditorTableColumn(usize),
    CatalogEditorAddTableColumn,
    CatalogEditorRemoveTableColumn,
    CatalogEditorRestoreTableColumn,
    CatalogEditorReview,
    CatalogEditorCancel,
    CatalogEditorDiscardKeepEditing,
    CatalogEditorDiscardChanges,
    CatalogEditorColumnDetailsConfirm,
    CatalogEditorColumnDetailsCancel,
    CatalogOwnerChoice(String),
    DismissNotification(u64),
    OpenTextDetail(crate::model::text_detail::TextDetailRequest),
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
    EditorExecutionTarget,
    EditorTransactionMenu,
    TargetSelectorRow(usize),
    TargetSelectorCancel,
    TransactionMenuItem(usize),
    TransactionMenuCancel,
    TransactionExitChoice(crate::model::transaction::TransactionExitChoice),
    TransactionExitCancel,
    TextDetailCopySelection,
    TextDetailCopyAll,
    TextDetailClose,
    RecordViewCopyCell,
    RecordViewCopyRow,
    RecordViewViewValue,
}

pub(crate) fn readonly_detail_request(
    title: impl Into<String>,
    text: impl Into<String>,
) -> crate::model::text_detail::TextDetailRequest {
    let text = crate::security::sanitize_terminal_text(&text.into());
    crate::model::text_detail::TextDetailRequest::new(
        title,
        Uuid::nil(),
        0,
        text.clone(),
        text,
        None,
    )
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
    pub terminal_selection_mode: bool,
    pub pane_layout: PaneLayoutMetrics,
    pub click_tracker: RefCell<Option<(crate::model::explorer::ExplorerNodeId, Instant)>>,
    pub relation_resize: RefCell<Option<(usize, u16, u16)>>,
    pub grid_scrollbar_drag: RefCell<Option<GridScrollbarDrag>>,
    pub pane_resize_drag: RefCell<Option<PaneResizeDrag>>,
    pub mouse_gesture: RefCell<Option<text_selection::GestureOwner>>,
    pub text_gesture: RefCell<Option<text_selection::TextGesture>>,
    pub text_selection_target: Option<text_selection::TextSelectionTarget>,
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
pub struct PaneResizeDrag {
    pub split: PaneSplit,
    pub start_pointer: u16,
    pub start_size: u16,
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
            terminal_selection_mode: false,
            pane_layout: PaneLayoutMetrics::default(),
            click_tracker: RefCell::new(None),
            relation_resize: RefCell::new(None),
            grid_scrollbar_drag: RefCell::new(None),
            pane_resize_drag: RefCell::new(None),
            mouse_gesture: RefCell::new(None),
            text_gesture: RefCell::new(None),
            text_selection_target: None,
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

    pub fn begin_text_gesture(&self, gesture: text_selection::TextGesture) -> bool {
        let mut owner = self.mouse_gesture.borrow_mut();
        if owner.is_some() {
            return false;
        }
        *owner = Some(text_selection::GestureOwner::Text);
        *self.text_gesture.borrow_mut() = Some(gesture);
        true
    }

    pub fn update_text_gesture(&self, end: text_selection::TextPosition) -> bool {
        if *self.mouse_gesture.borrow() != Some(text_selection::GestureOwner::Text) {
            return false;
        }
        if let Some(gesture) = self.text_gesture.borrow_mut().as_mut() {
            gesture.end = end;
            true
        } else {
            false
        }
    }

    pub fn end_mouse_gesture(&self) -> Option<text_selection::GestureOwner> {
        let owner = self.mouse_gesture.borrow_mut().take();
        if owner == Some(text_selection::GestureOwner::Text) {
            self.text_gesture.borrow_mut().take();
        }
        owner
    }

    pub fn cancel_mouse_gesture(&self) {
        self.mouse_gesture.borrow_mut().take();
        self.text_gesture.borrow_mut().take();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisconnectedWorkspace {
    NoProfiles,
    NoActiveConnection,
}

impl DisconnectedWorkspace {
    fn for_app(app: &App) -> Option<Self> {
        (app.connection.status == ConnectionStatus::Disconnected).then_some({
            if app.profiles.is_empty() {
                Self::NoProfiles
            } else {
                Self::NoActiveConnection
            }
        })
    }

    const fn title(self) -> &'static str {
        match self {
            Self::NoProfiles => "NO CONNECTIONS YET",
            Self::NoActiveConnection => "NO ACTIVE CONNECTION",
        }
    }

    const fn instruction(self, compact: bool) -> &'static str {
        match (self, compact) {
            (Self::NoProfiles, false) => "Select NEW in Explorer, then press Enter.",
            (Self::NoActiveConnection, false) => {
                "Select a connection in Explorer, then press Enter."
            }
            (Self::NoProfiles, true) => "Select NEW in Explorer; press Enter.",
            (Self::NoActiveConnection, true) => "Select a connection; press Enter.",
        }
    }
}

const LAZYDB_ASCII: [&str; 5] = [
    " L     A   ZZZZZ  Y   Y DDDD  BBBB ",
    " L    A A     Z   Y Y  D   D B   B",
    " L   AAAAA   Z     Y   D   D BBBB ",
    " L  A     A Z      Y   D   D B   B",
    " L A       AZZZZZ  Y   DDDD  BBBB ",
];

fn disconnected_workspace_area(layout: AppLayout) -> Option<Rect> {
    [
        layout.tabs,
        layout.editor,
        layout.result_tabs,
        layout.results,
    ]
    .into_iter()
    .flatten()
    .reduce(|left, right| {
        let x = left.x.min(right.x);
        let y = left.y.min(right.y);
        let right_edge = left.right().max(right.right());
        let bottom = left.bottom().max(right.bottom());
        Rect::new(x, y, right_edge.saturating_sub(x), bottom.saturating_sub(y))
    })
}

fn render_disconnected_workspace(
    frame: &mut Frame<'_>,
    area: Rect,
    workspace: DisconnectedWorkspace,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }

    let spacious = area.width >= 47 && area.height >= 11;
    let medium = area.width >= 40 && area.height >= 5;
    let compact = !spacious && !medium;
    let lines = if spacious {
        LAZYDB_ASCII
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    *line,
                    Style::new()
                        .fg(theme.muted)
                        .bg(theme.background)
                        .add_modifier(Modifier::DIM),
                ))
            })
            .chain([
                Line::from(""),
                Line::from(Span::styled(
                    workspace.title(),
                    Style::new()
                        .fg(theme.text)
                        .bg(theme.background)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    workspace.instruction(false),
                    Style::new().fg(theme.muted).bg(theme.background),
                )),
            ])
            .collect::<Vec<_>>()
    } else if medium {
        vec![
            Line::from(Span::styled(
                workspace.title(),
                Style::new()
                    .fg(theme.text)
                    .bg(theme.background)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                workspace.instruction(false),
                Style::new().fg(theme.muted).bg(theme.background),
            )),
        ]
    } else if compact {
        vec![
            Line::from(Span::styled(
                workspace.title(),
                Style::new()
                    .fg(theme.text)
                    .bg(theme.background)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                workspace.instruction(true),
                Style::new().fg(theme.muted).bg(theme.background),
            )),
        ]
    } else {
        Vec::new()
    };
    let content_height = lines.len().min(usize::from(area.height)) as u16;
    let content = Rect::new(area.x, area.y, area.width, content_height);
    let top = area
        .y
        .saturating_add(area.height.saturating_sub(content_height) / 2);
    let content = Rect::new(content.x, top, content.width, content.height);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .style(Style::new().bg(theme.background)),
        content,
    );
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
    render_with_state_using_icons_sequence_and_theme(
        frame,
        app,
        state,
        icons,
        sequence,
        Theme::default(),
    );
}

pub fn render_with_state_using_icons_sequence_and_theme(
    frame: &mut Frame<'_>,
    app: &App,
    state: &mut UiState,
    icons: icons::IconSet,
    sequence: Option<&crate::input::keymap::KeySequenceState>,
    theme: Theme,
) {
    let area = frame.area();
    state.activity_icons = icons;
    state.observe_animations(app, Instant::now());
    frame.render_widget(Block::new().style(theme.base()), area);
    let is_relation = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(_))
    );
    let is_dashboard = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Dashboard(_))
    );
    let layout = AppLayout::calculate(
        area,
        app.focus,
        is_relation || is_dashboard,
        app.pane_sizes,
        app.pane_maximized,
    );
    if app.overlay.is_some()
        || layout
            .pane_resize_region(PaneSplit::ExplorerWidth)
            .is_none()
    {
        state.pane_resize_drag.borrow_mut().take();
    }
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
    state.text_selection_target = None;
    state.result_area = None;

    if layout.mode == LayoutMode::TooSmall {
        render_too_small(frame, area, theme);
        return;
    }

    render_header(frame, layout.header, app, theme, state);
    if let Some(area) = layout.tabs {
        render_tabs(frame, area, app, theme, state, icons);
    }
    if is_dashboard {
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
            dashboard::render(frame, area, app, theme, state);
        }
        render_footer(frame, layout.footer, app, theme, sequence, state);
    } else if is_relation {
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
        render_footer(frame, layout.footer, app, theme, sequence, state);
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
        if let Some(workspace) = DisconnectedWorkspace::for_app(app) {
            if let Some(area) = disconnected_workspace_area(layout) {
                render_disconnected_workspace(frame, area, workspace, theme);
            }
        } else {
            if let Some(area) = layout.editor {
                state.hit_regions.push(HitRegion {
                    area,
                    target: HitTarget::Focus(Focus::Editor),
                });
                let completion_anchor = render_editor(frame, area, app, theme, state);
                render_completion_popup(frame, app, theme, state, completion_anchor, icons);
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
        }
        if !matches!(app.overlay, Some(Overlay::CatalogEditor)) {
            render_footer(frame, layout.footer, app, theme, sequence, state);
        }
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
    }

    if app.overlay.is_none()
        && let Some(area) = layout.pane_resize_region(PaneSplit::ExplorerWidth)
    {
        state.hit_regions.push(HitRegion {
            area,
            target: HitTarget::PaneResize(PaneSplit::ExplorerWidth),
        });
    }

    if app.overlay.is_none()
        && state.pane_resize_drag.borrow().is_some()
        && let Some(area) = layout.pane_resize_region(PaneSplit::ExplorerWidth)
    {
        let buffer = frame.buffer_mut();
        for y in area.y..area.bottom() {
            let cell = &mut buffer[(area.x, y)];
            cell.set_fg(theme.accent);
            cell.set_bg(theme.surface_raised);
            cell.set_style(Style::new().add_modifier(Modifier::BOLD));
        }
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
        Overlay::Update(_) => 21,
        Overlay::NotificationHistory(_) => 17,
        Overlay::RecordView(_) => 2,
        Overlay::TextDetail(_) => 23,
        Overlay::ProfileManager => 3,
        Overlay::CatalogEditor => 18,
        Overlay::ProfileAccess { .. } => 4,
        Overlay::ProfileGroup(_) => 19,
        Overlay::ExplorerAdd(_) => 20,
        Overlay::Message { .. } => 5,
        Overlay::SubstituteConfirm { .. } => 6,
        Overlay::ExecutionConfirm { .. } => 7,
        Overlay::ManualCancelConfirm { .. } => 8,
        Overlay::TransactionExitConfirm { .. } => 9,
        Overlay::RelationTransactionConfirm { .. } => 10,
        Overlay::ClearTransactionOutcome { .. } => 11,
        Overlay::TransactionMenu { .. } => 22,
        Overlay::TargetSelector { .. } => 12,
        Overlay::DeleteConsole { .. } => 13,
        Overlay::SqlEditorList(_) => 14,
        Overlay::PageSizeSelector { .. } => 15,
        Overlay::CatalogDropConfirm { .. } | Overlay::CatalogEditorDestructiveConfirm { .. } => 16,
        Overlay::CatalogEditorDiscardConfirm { .. } => 17,
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
        WorkspaceTab::Dashboard(_) => {}
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

const COMPLETION_DETAIL_GAP: u16 = 2;
const COMPLETION_ROW_RIGHT_PADDING: u16 = 1;
const COMPLETION_DETAIL_MAX_CELLS: u16 = 24;
const COMPLETION_DETAIL_MIN_CELLS: u16 = 4;

/// Column widths for a completion popup row: icon, label and type detail.
///
/// `detail == 0` means the type column is hidden because the popup is too
/// narrow to carry it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CompletionColumns {
    icon: u16,
    label: u16,
    detail: u16,
}

impl CompletionColumns {
    /// Measures `(icon cells, label, detail)` over every candidate, not just the
    /// visible ones, so a clipped popup does not shift its columns.
    fn measure<'a>(rows: impl Iterator<Item = (u16, &'a str, &'a str)>) -> Self {
        let mut columns = Self::default();
        for (icon, label, detail) in rows {
            columns.icon = columns.icon.max(icon);
            columns.label = columns.label.max(label.cell_width());
            columns.detail = columns.detail.max(detail.cell_width());
        }
        columns.detail = columns.detail.min(COMPLETION_DETAIL_MAX_CELLS);
        columns
    }

    /// Start of the label column. The popup anchor is derived from this, so it
    /// must stay `icon + 1`.
    fn label_offset(self) -> u16 {
        self.icon.saturating_add(1)
    }

    fn content_width(self) -> u16 {
        self.label_offset()
            .saturating_add(self.label)
            .saturating_add(if self.detail == 0 {
                0
            } else {
                COMPLETION_DETAIL_GAP.saturating_add(self.detail)
            })
            .saturating_add(COMPLETION_ROW_RIGHT_PADDING)
            .max(4)
    }

    /// Re-converges the columns against the clamped popup width: labels keep
    /// their space first, the type column takes what is left and disappears
    /// entirely once it would be too narrow to read.
    fn fit(self, inner_width: u16) -> Self {
        let label = self
            .label
            .min(inner_width.saturating_sub(self.label_offset()));
        let detail = self.detail.min(
            inner_width
                .saturating_sub(self.label_offset())
                .saturating_sub(label)
                .saturating_sub(COMPLETION_DETAIL_GAP)
                .saturating_sub(COMPLETION_ROW_RIGHT_PADDING),
        );
        Self {
            icon: self.icon,
            label,
            detail: if detail >= COMPLETION_DETAIL_MIN_CELLS {
                detail
            } else {
                0
            },
        }
    }
}

/// Builds one popup row: icon, left-aligned label, right-aligned type detail
/// and trailing padding so a selected row highlights as a full-width bar.
fn completion_row(
    columns: CompletionColumns,
    inner_width: u16,
    icon: &str,
    label_spans: Vec<Span<'static>>,
    detail: &str,
    row_style: Style,
    detail_style: Style,
) -> ListItem<'static> {
    let label_cells = label_spans.iter().fold(0u16, |total, span| {
        total.saturating_add(span.content.as_ref().cell_width())
    });
    let icon_padding = " ".repeat(usize::from(columns.icon.saturating_sub(icon.cell_width())));
    let mut spans = Vec::with_capacity(label_spans.len() + 3);
    spans.push(Span::styled(format!("{icon_padding}{icon} "), row_style));
    spans.extend(label_spans);
    let mut used = columns.label_offset().saturating_add(label_cells);
    if columns.detail > 0 && !detail.is_empty() {
        let detail = truncate_to_cell_width(detail, columns.detail);
        let detail_cells = detail.as_str().cell_width();
        let padding = columns
            .label
            .saturating_sub(label_cells)
            .saturating_add(COMPLETION_DETAIL_GAP)
            .saturating_add(columns.detail.saturating_sub(detail_cells));
        spans.push(Span::styled(" ".repeat(usize::from(padding)), row_style));
        spans.push(Span::styled(detail, detail_style));
        used = used.saturating_add(padding).saturating_add(detail_cells);
    }
    let trailing = inner_width.saturating_sub(used);
    if trailing > 0 {
        spans.push(Span::styled(" ".repeat(usize::from(trailing)), row_style));
    }
    ListItem::new(Line::from(spans))
}

fn render_completion_popup(
    frame: &mut Frame<'_>,
    app: &App,
    theme: Theme,
    state: &mut UiState,
    anchor: Option<CompletionAnchor>,
    icons: icons::IconSet,
) {
    const POPUP_BORDER_WIDTH: u16 = 2;
    const POPUP_BORDER_HEIGHT: u16 = 2;

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
    let columns = CompletionColumns::measure(popup.candidates.iter().map(|candidate| {
        (
            icons.completion(candidate.kind).cell_width(),
            candidate.label.as_str(),
            candidate.detail.as_deref().unwrap_or(""),
        )
    }));
    let visible_rows = popup.candidates.len().min(10) as u16;
    let desired_width = columns.content_width().saturating_add(POPUP_BORDER_WIDTH);
    let desired_height = visible_rows.saturating_add(POPUP_BORDER_HEIGHT);
    let popup_x = anchor
        .replacement_start_x
        .map(|label_x| label_x.saturating_sub(columns.label_offset().saturating_add(1)))
        .unwrap_or(anchor.cursor.x);
    let layout_anchor = CompletionAnchor {
        cursor: Position::new(popup_x, anchor.cursor.y),
        replacement_start_x: None,
        ..anchor
    };
    let Some(area) = completion_popup_rect(layout_anchor, desired_width, desired_height) else {
        return;
    };
    if area.width < 3 || area.height < 3 {
        return;
    }
    state.completion_popup = Some(area);
    let inner_width = area.width.saturating_sub(POPUP_BORDER_WIDTH);
    let columns = columns.fit(inner_width);
    let editor_text = app.active_editor_text().ok();
    let items = popup
        .candidates
        .iter()
        .take(usize::from(area.height.saturating_sub(POPUP_BORDER_HEIGHT)).min(10))
        .enumerate()
        .map(|(index, candidate)| {
            let row_style = if index == popup.selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            let match_query = editor_text
                .as_deref()
                .and_then(|text| text.get(candidate.replace.start..candidate.replace.end));
            let label_spans = match_query.map_or_else(
                || vec![Span::styled(candidate.label.clone(), row_style)],
                |query| completion_label_spans(&candidate.label, query, row_style),
            );
            completion_row(
                columns,
                inner_width,
                icons.completion(candidate.kind),
                label_spans,
                candidate.detail.as_deref().unwrap_or(""),
                row_style,
                row_style.fg(if index == popup.selected {
                    theme.background
                } else {
                    theme.muted
                }),
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, area);
    frame.render_widget(List::new(items).block(completion_popup_block(theme)), area);
}

fn completion_label_spans(label: &str, query: &str, row_style: Style) -> Vec<Span<'static>> {
    let Some(positions) = crate::sql::identifier_match_positions(label, query) else {
        return vec![Span::styled(label.to_owned(), row_style)];
    };
    let matched = positions
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut spans = Vec::new();
    let mut segment_start = 0;
    let mut segment_matched = None;
    for (position, _) in label.char_indices() {
        let is_matched = matched.contains(&position);
        if segment_matched.is_some_and(|previous| previous != is_matched) {
            spans.push(Span::styled(
                label[segment_start..position].to_owned(),
                if segment_matched.unwrap() {
                    row_style.add_modifier(Modifier::BOLD)
                } else {
                    row_style
                },
            ));
            segment_start = position;
        }
        segment_matched = Some(is_matched);
    }
    if segment_start < label.len() {
        spans.push(Span::styled(
            label[segment_start..].to_owned(),
            if segment_matched.unwrap_or(false) {
                row_style.add_modifier(Modifier::BOLD)
            } else {
                row_style
            },
        ));
    }
    spans
}

pub(crate) fn render_data_query_completion_popup(
    frame: &mut Frame<'_>,
    completion: &crate::model::data_query::DataQueryCompletion,
    theme: Theme,
    state: &mut UiState,
    anchor: CompletionAnchor,
) {
    const POPUP_BORDER_WIDTH: u16 = 2;
    const POPUP_BORDER_HEIGHT: u16 = 2;

    const ICON: &str = "CL";

    if completion.candidates.is_empty() {
        return;
    }
    // Sanitize once up front: both the column measurement and the rows below
    // read the display text, and it must never reach the terminal raw.
    let rows = completion
        .candidates
        .iter()
        .map(|candidate| {
            (
                crate::security::sanitize_terminal_text(&candidate.name),
                candidate
                    .type_name
                    .as_deref()
                    .map(crate::security::sanitize_terminal_text)
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let columns = CompletionColumns::measure(
        rows.iter()
            .map(|(name, detail)| (ICON.cell_width(), name.as_str(), detail.as_str())),
    );
    let visible_rows = rows.len().min(10) as u16;
    let desired_width = columns.content_width().saturating_add(POPUP_BORDER_WIDTH);
    let desired_height = visible_rows.saturating_add(POPUP_BORDER_HEIGHT);
    let Some(area) = completion_popup_rect(anchor, desired_width, desired_height) else {
        return;
    };
    if area.width < 3 || area.height < 3 {
        return;
    }
    state.completion_popup = Some(area);
    let inner_width = area.width.saturating_sub(POPUP_BORDER_WIDTH);
    let columns = columns.fit(inner_width);
    let items = rows
        .iter()
        .take(usize::from(area.height.saturating_sub(POPUP_BORDER_HEIGHT)).min(10))
        .enumerate()
        .map(|(index, (name, detail))| {
            let selected = index == completion.selected;
            let row_style = if selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            completion_row(
                columns,
                inner_width,
                ICON,
                vec![Span::styled(name.clone(), row_style)],
                detail,
                row_style,
                row_style.fg(if selected {
                    theme.background
                } else {
                    theme.muted
                }),
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, area);
    frame.render_widget(List::new(items).block(completion_popup_block(theme)), area);
}

fn completion_popup_block(theme: Theme) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.border))
        .style(Style::new().bg(theme.surface_raised))
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
    let status = if app.is_editor_target_switch_pending() {
        Some(("TARGET", theme.warning))
    } else {
        match app.connection.status {
            ConnectionStatus::Connecting => Some(("LINKING", theme.warning)),
            ConnectionStatus::Failed => Some(("FAILED", theme.error)),
            ConnectionStatus::Disconnected | ConnectionStatus::Connected => None,
        }
    };
    let update_badge = app
        .update_inspection()
        .and_then(|inspection| match inspection.status {
            crate::update::UpdateStatus::Available => inspection
                .target_version
                .as_ref()
                .map(|version| format!(" UPDATE {version} ")),
            crate::update::UpdateStatus::ReadyToRestart => inspection
                .installed_version
                .as_ref()
                .map(|version| format!(" RESTART {version} ")),
            _ => None,
        });
    let update_width = update_badge
        .as_deref()
        .map_or(0, |value| value.cell_width());
    let status_width = status.map_or(0, |(status, _)| status.cell_width().saturating_add(2));
    let right_width = update_width.saturating_add(status_width);
    let main_area = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(right_width),
        area.height,
    );
    let running_version = format!(" v{} ", env!("CARGO_PKG_VERSION"));
    let spans = vec![
        Span::styled(
            " LAZYDB ",
            Style::new()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            running_version.clone(),
            Style::new().fg(theme.action).bg(theme.surface),
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
        Span::styled(
            database.clone(),
            Style::new().fg(theme.action).bg(theme.surface),
        ),
    ];
    let line = Line::from(spans);
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(theme.surface)),
        main_area,
    );
    let version_x = area.x.saturating_add(8);
    let version_width = running_version.cell_width() as u16;
    if version_width > 0 && version_x < main_area.right() {
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                version_x,
                area.y,
                version_width.min(main_area.right().saturating_sub(version_x)),
                1,
            ),
            target: HitTarget::UpdateCenter,
        });
    }
    if let Some((status, color)) = status {
        frame.render_widget(
            Paragraph::new(format!(" {status} "))
                .style(
                    Style::new()
                        .fg(color)
                        .bg(theme.surface)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Right),
            Rect::new(
                area.right().saturating_sub(status_width),
                area.y,
                status_width,
                area.height,
            ),
        );
    }
    if let Some(update_badge) = update_badge {
        let update_area = Rect::new(
            area.right().saturating_sub(right_width),
            area.y,
            update_width,
            area.height,
        );
        frame.render_widget(
            Paragraph::new(update_badge)
                .style(
                    Style::new()
                        .fg(theme.warning)
                        .bg(theme.surface)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Right),
            update_area,
        );
        state.hit_regions.push(HitRegion {
            area: update_area,
            target: HitTarget::UpdateCenter,
        });
    }
    let profile_x = area
        .x
        .saturating_add(10 + running_version.cell_width() as u16 + 2);
    let profile_width = profile_width.min(main_area.right().saturating_sub(profile_x));
    if profile_width > 0 {
        state.hit_regions.push(HitRegion {
            area: Rect::new(profile_x, area.y, profile_width, 1),
            target: HitTarget::HeaderProfile,
        });
    }
    let database_x = profile_x
        .saturating_add(profile_width)
        .saturating_add("  /  ".cell_width());
    let database_width = database.cell_width();
    if database_width > 0 && database_x < main_area.right() {
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                database_x,
                area.y,
                database_width.min(main_area.right().saturating_sub(database_x)),
                1,
            ),
            target: HitTarget::OpenTextDetail(readonly_detail_request(
                "Connection database",
                database,
            )),
        });
    }
}

fn header_text(value: &str) -> String {
    sanitize_terminal_text(value)
        .replace('\n', "<LF>")
        .replace('\t', "<TAB>")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TabViewport {
    start: usize,
    end: usize,
    overflowed: bool,
}

struct RenderedTab {
    index: usize,
    id: Uuid,
    label: String,
    close: Option<String>,
    width: u16,
}

const TAB_OVERFLOW_CONTROLS_WIDTH: u16 = 2;

fn tab_viewport(widths: &[u16], active: usize, area_width: u16) -> TabViewport {
    if widths.is_empty() {
        return TabViewport {
            start: 0,
            end: 0,
            overflowed: false,
        };
    }

    let total = widths.iter().copied().fold(0_u16, u16::saturating_add);
    if total <= area_width {
        return TabViewport {
            start: 0,
            end: widths.len(),
            overflowed: false,
        };
    }

    let active = active.min(widths.len() - 1);
    let available = area_width.saturating_sub(TAB_OVERFLOW_CONTROLS_WIDTH);
    let mut start = active;
    let mut end = active + 1;
    let mut used = widths[active].min(available);

    while end < widths.len() && used.saturating_add(widths[end]) <= available {
        used = used.saturating_add(widths[end]);
        end += 1;
    }
    while start > 0 && used.saturating_add(widths[start - 1]) <= available {
        start -= 1;
        used = used.saturating_add(widths[start]);
    }

    TabViewport {
        start,
        end,
        overflowed: true,
    }
}

fn truncate_to_cell_width(value: &str, max_width: u16) -> String {
    if value.cell_width() <= max_width {
        return value.to_owned();
    }
    if max_width == 0 {
        return String::new();
    }

    let ellipsis_width = '…'.width().unwrap_or(1) as u16;
    let limit = max_width.saturating_sub(ellipsis_width);
    let mut result = String::new();
    let mut width: u16 = 0;
    for character in value.chars() {
        let character_width = character.width().unwrap_or(0) as u16;
        if width.saturating_add(character_width) > limit {
            break;
        }
        result.push(character);
        width = width.saturating_add(character_width);
    }
    if max_width >= ellipsis_width {
        result.push('…');
    }
    result
}

fn render_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut UiState,
    icons: icons::IconSet,
) {
    let rendered_tabs = app
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            let title = sanitize_terminal_text(tab.title())
                .chars()
                .take(48)
                .collect::<String>();
            let icon = match tab {
                WorkspaceTab::Relation(tab) => icons.catalog(tab.descriptor.kind),
                WorkspaceTab::Dashboard(_) => {
                    icons.database(crate::profile::DatabaseKind::Postgres)
                }
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
            let can_close = true;
            let label_width = label.cell_width();
            let close_width = if can_close { close.cell_width() } else { 0 };
            let width = label_width + close_width;
            RenderedTab {
                index,
                id: tab.id(),
                label,
                close: can_close.then_some(close),
                width,
            }
        })
        .collect::<Vec<_>>();
    let widths = rendered_tabs
        .iter()
        .map(|tab| tab.width)
        .collect::<Vec<_>>();
    let viewport = tab_viewport(&widths, app.active_tab, area.width);
    let (tabs_area, left_area, right_area) = if viewport.overflowed {
        (
            Rect::new(
                area.x.saturating_add(1),
                area.y,
                area.width.saturating_sub(2),
                1,
            ),
            Rect::new(area.x, area.y, 1, 1),
            Rect::new(area.right().saturating_sub(1), area.y, 1, 1),
        )
    } else {
        (area, Rect::default(), Rect::default())
    };
    let active_style = Style::new()
        .fg(theme.background)
        .bg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::new().fg(theme.muted).bg(theme.surface);
    let mut spans = Vec::new();
    let mut x = tabs_area.x;
    for tab in &rendered_tabs[viewport.start..viewport.end] {
        let active = tab.index == app.active_tab;
        let style = if active { active_style } else { inactive_style };
        let close_width = tab.close.as_ref().map_or(0, |close| close.cell_width());
        let max_label_width = tabs_area.width.saturating_sub(close_width);
        let label = truncate_to_cell_width(&tab.label, max_label_width);
        let label_width = label.cell_width();
        spans.push(Span::styled(label, style));
        if let Some(close) = &tab.close {
            spans.push(Span::styled(close.clone(), style));
        }
        if x < tabs_area.right() {
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    x,
                    tabs_area.y,
                    label_width.min(tabs_area.right().saturating_sub(x)),
                    1,
                ),
                target: HitTarget::Tab(tab.index),
            });
        }
        let close_x = x.saturating_add(label_width);
        if close_width > 0 && close_x < tabs_area.right() {
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    close_x,
                    tabs_area.y,
                    close_width.min(tabs_area.right().saturating_sub(close_x)),
                    1,
                ),
                target: HitTarget::CloseTab(tab.id),
            });
        }
        x = x.saturating_add(tab.width);
    }
    if viewport.overflowed {
        let left_enabled = viewport.start > 0;
        let right_enabled = viewport.end < rendered_tabs.len();
        let arrow_style = |enabled| {
            if enabled {
                Style::new().fg(theme.action).bg(theme.background)
            } else {
                Style::new().fg(theme.border).bg(theme.background)
            }
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                icons.tab_previous(),
                arrow_style(left_enabled),
            ))
            .alignment(Alignment::Center),
            left_area,
        );
        frame.render_widget(
            Paragraph::new(Span::styled(icons.tab_next(), arrow_style(right_enabled)))
                .alignment(Alignment::Center),
            right_area,
        );
        if left_enabled {
            state.hit_regions.push(HitRegion {
                area: left_area,
                target: HitTarget::TabScrollLeft(viewport.start - 1),
            });
        }
        if right_enabled {
            state.hit_regions.push(HitRegion {
                area: right_area,
                target: HitTarget::TabScrollRight(viewport.end),
            });
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(theme.background)),
        tabs_area,
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
        crate::model::explorer::ExplorerNodeId::ConnectionGroup { .. } => {
            icons.group(crate::db::catalog::ObjectGroup::Tables, expanded)
        }
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
    let is_connection_group = matches!(
        visible.id,
        crate::model::explorer::ExplorerNodeId::ConnectionGroup { .. }
    );
    if is_connection_group {
        // Organization rows intentionally do not expose connection metadata.
    } else if visible.provenance == Some(ProfileProvenance::Session) {
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
    let query = format!("/ {}", sanitize_terminal_text(find.query.value()));
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
        find.query.value(),
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
    let input = format!("/ {}", sanitize_terminal_text(search.query.value()));
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
                search.query.value(),
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
                    sanitize_terminal_text(search.query.value())
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

pub(crate) fn register_text_selection_target(
    state: &mut UiState,
    session_id: Uuid,
    text_viewport: Rect,
    snapshot: &crate::model::editor::EditorRenderSnapshot,
) {
    state.text_selection_target = Some(text_selection::TextSelectionTarget {
        session_id,
        hit_maps: snapshot
            .lines
            .iter()
            .take(usize::from(text_viewport.height))
            .enumerate()
            .map(|(row, line)| text_selection::TextHitMap {
                area: Rect::new(
                    text_viewport.x,
                    text_viewport.y.saturating_add(row as u16),
                    text_viewport.width,
                    1,
                ),
                line: line.line,
                source_to_display_cells: line.source_to_display_cells.clone(),
                horizontal_offset: snapshot.horizontal_offset,
            })
            .collect(),
    });
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
        .active_editor_line_count()
        .map(|line_count| line_count.to_string().len().max(2))
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
    if let Some(session_id) = app.active_console_opt().map(|tab| tab.id) {
        register_text_selection_target(state, session_id, text_viewport, &snapshot);
    }
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
                    let target_label = format!(
                        "[{}] {}{}",
                        profile.name,
                        target.database,
                        target
                            .schema
                            .as_deref()
                            .map(|schema| format!(".{schema}"))
                            .unwrap_or_default()
                    );
                    if app.connection.active_identity().is_some()
                        && app.connection.target.as_ref() == Some(target)
                    {
                        target_label
                    } else if app.connection.active_identity().is_some() {
                        format!("{target_label} NOT CONNECTED")
                    } else {
                        format!("{target_label} OFFLINE")
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
    let full_left_title = format!(" SQL EDITOR  {mode} ");
    let compact_left_title = " SQL EDITOR ";
    let query_status = app
        .active_console_opt()
        .and_then(|tab| match tab.query_status {
            QueryStatus::Idle => None,
            QueryStatus::Running => Some(("QUERY RUNNING", theme.action)),
            QueryStatus::Cancelled => Some(("QUERY CANCELLED", theme.warning)),
            QueryStatus::Failed => Some(("QUERY ERROR", theme.error)),
        });
    let transaction_segment = format!(" {transaction} ");
    let query_segment_width =
        query_status.map_or(0, |(label, _)| format!(" {label} ").cell_width());
    let available_width = area.width.saturating_sub(2);
    let required_context_width =
        query_segment_width.saturating_add(transaction_segment.cell_width());
    let left_title = if full_left_title
        .cell_width()
        .saturating_add(required_context_width)
        <= available_width
    {
        full_left_title
    } else {
        compact_left_title.to_owned()
    };
    let required_width = left_title
        .cell_width()
        .saturating_add(query_segment_width)
        .saturating_add(transaction_segment.cell_width());
    let target_segment = format!(" {target} ");
    let show_target = required_width.saturating_add(target_segment.cell_width()) <= available_width;
    let mut context = Vec::new();
    if let Some((label, color)) = query_status {
        context.push(Span::styled(
            format!(" {label} "),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    if show_target {
        context.push(Span::raw(&target_segment));
    }
    context.push(Span::raw(&transaction_segment));
    let context_right = area.right().saturating_sub(1);
    let transaction_x = context_right.saturating_sub(transaction_segment.cell_width() as u16);
    state.hit_regions.push(HitRegion {
        area: Rect::new(
            transaction_x,
            area.y,
            transaction_segment.cell_width() as u16,
            1,
        ),
        target: HitTarget::EditorTransactionMenu,
    });
    if show_target {
        let target_x = transaction_x.saturating_sub(target_segment.cell_width() as u16);
        state.hit_regions.push(HitRegion {
            area: Rect::new(target_x, area.y, target_segment.cell_width() as u16, 1),
            target: HitTarget::EditorExecutionTarget,
        });
    }
    let block = base_block
        .title_top(Line::raw(left_title).left_aligned())
        .title_top(Line::from(context).right_aligned());
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
        let statement_indicator = if line.current_statement
            && matches!(
                snapshot.mode,
                EditorMode::Normal | EditorMode::Insert | EditorMode::Replace
            ) {
            Span::styled(
                "┃",
                Style::new().fg(if app.focus == Focus::Editor && app.overlay.is_none() {
                    theme.accent
                } else {
                    theme.muted
                }),
            )
        } else {
            Span::styled("│", line_style)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {:>number_width$} ", line.line + 1), line_style),
                statement_indicator,
                Span::styled(" ", line_style),
            ])),
            Rect::new(inner.x, y, gutter as u16, 1),
        );
        let content = editor_line_spans(
            line,
            &snapshot,
            theme,
            true,
            (!selected
                && snapshot.selections.is_empty()
                && matches!(
                    snapshot.mode,
                    EditorMode::Normal | EditorMode::Insert | EditorMode::Replace
                )
                && app.focus == Focus::Editor
                && app.overlay.is_none())
            .then_some(line.statement_background_cells)
            .flatten(),
        );
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
        EditorHighlightKind::Relation => theme::SyntaxColor::Relation,
        EditorHighlightKind::RelationAlias => theme::SyntaxColor::RelationAlias,
        EditorHighlightKind::Column => theme::SyntaxColor::Column,
        EditorHighlightKind::Function => theme::SyntaxColor::Function,
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
    statement_background_cells: Option<(usize, usize)>,
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
            } else if statement_background_cells.is_some_and(|(start, end)| {
                display_cell < end && display_cell.saturating_add(width) > start
            }) {
                theme.surface_raised
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
        } else {
            frame.render_widget(
                loading::LoadingViewport {
                    mode: state.animation_mode(),
                    icons: state.activity_icons,
                    elapsed,
                    label: "Executing query",
                    helper: animation::show_loading_helper(elapsed)
                        .then_some("Waiting for the first result set..."),
                    cancellable: true,
                    theme,
                    block: Block::default().style(Style::new().bg(theme.surface)),
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
    if let Some(session_id) = app.active_console_opt().map(|tab| tab.output_editor_id) {
        register_text_selection_target(
            state,
            session_id,
            Rect::new(
                inner.x.saturating_add(3),
                inner.y,
                viewport.width as u16,
                viewport.height as u16,
            ),
            &snapshot,
        );
    }
    for (row, line) in snapshot.lines.iter().take(viewport.height).enumerate() {
        let y = inner.y.saturating_add(row as u16);
        let content = editor_line_spans(line, &snapshot, theme, true, None);
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
    state: &UiState,
) {
    if state.terminal_selection_mode {
        frame.render_widget(
            Paragraph::new(" TERMINAL SELECTION  |  mouse released  |  press Esc to return ")
                .style(Style::new().fg(theme.warning).bg(theme.surface)),
            area,
        );
        return;
    }
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
    let hint_values =
        crate::help::footer_shortcuts_with_bindings(context, capabilities, Some(&app.key_bindings))
            .into_iter()
            .map(|shortcut| {
                ShortcutHint::new(
                    crate::help::configured_sequence(&shortcut, Some(&app.key_bindings)),
                    shortcut.description,
                )
            })
            .collect::<Vec<_>>();
    let mode_badge = format!(" {mode} ");
    let hint_line = shortcut_hints::line(
        &hint_values,
        footer_hint_width(&mode_badge, area.width),
        theme,
        theme.surface,
    );
    let mut spans = vec![
        Span::styled(
            mode_badge,
            Style::new()
                .fg(theme.background)
                .bg(mode_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::new().bg(theme.surface)),
        Span::styled("", Style::new().bg(theme.surface)),
    ];
    spans.pop();
    spans.extend(hint_line.spans);
    spans.push(Span::styled("", Style::new().bg(theme.surface)));
    let line = Line::from(spans);
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(theme.surface)),
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
        Overlay::Update(_) => render_update_overlay(frame, area, app, state, theme),
        Overlay::NotificationHistory(history) => {
            notifications::render_history(frame, area, app, history, theme, state)
        }
        Overlay::RecordView(view) => record_view::render(frame, area, app, view, theme, state),
        Overlay::TextDetail(view) => text_detail::render(frame, area, app, view, theme, state),
        Overlay::ProfileManager => {
            profiles::render_profile_manager(frame, area, app, state, theme, icons)
        }
        Overlay::CatalogEditor => catalog_editor::render(frame, area, app, state, theme, icons),
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
            render_transaction_exit_overlay(frame, area, app, prompt, *choice, theme, state);
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
        Overlay::TransactionMenu { selected } => {
            render_transaction_menu(frame, area, app, *selected, theme, state);
        }
        Overlay::TargetSelector {
            candidates,
            selected,
        } => {
            const MAX_VISIBLE_ROWS: usize = 16;
            let visible_count = candidates.len().min(MAX_VISIBLE_ROWS);
            let height = (visible_count as u16).saturating_add(6).clamp(8, 24);
            let popup = centered(area, 68, height);
            frame.render_widget(Clear, popup);
            let current = app
                .active_console_opt()
                .and_then(|tab| tab.execution_target.as_ref());
            let start = selected
                .saturating_sub(visible_count.saturating_sub(1))
                .min(candidates.len().saturating_sub(visible_count));
            let end = start.saturating_add(visible_count);
            let mut lines = vec![Line::from(Span::styled(
                " EXECUTION TARGET ",
                theme.title(true),
            ))];
            lines.extend(
                candidates[start..end]
                    .iter()
                    .enumerate()
                    .map(|(offset, target)| {
                        let index = start + offset;
                        let marker = if index == *selected { ">" } else { " " };
                        let current_marker = if current == Some(target) {
                            " current"
                        } else {
                            ""
                        };
                        let label = format!(
                            "{marker} {}{}{}",
                            sanitize_terminal_text(&target.database),
                            target
                                .schema
                                .as_deref()
                                .map(|schema| format!(".{}", sanitize_terminal_text(schema)))
                                .unwrap_or_default(),
                            current_marker,
                        );
                        Line::from(Span::styled(
                            truncate_to_cells(&label, popup.width.saturating_sub(2) as usize),
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
                    }),
            );
            lines.push(Line::raw(""));
            lines.push(Line::raw(
                "j/k or Up/Down select  Enter confirm  Esc cancel",
            ));
            lines.push(Line::from(Span::styled(
                " Cancel ",
                Style::new().fg(theme.text).bg(theme.surface_raised),
            )));
            let cancel_y = popup.y.saturating_add(4 + visible_count as u16);
            if cancel_y < popup.bottom() {
                state.hit_regions.push(HitRegion {
                    area: Rect::new(
                        popup.x.saturating_add(1),
                        cancel_y,
                        popup.width.saturating_sub(2),
                        1,
                    ),
                    target: HitTarget::TargetSelectorCancel,
                });
            }
            for (offset, _) in candidates[start..end].iter().enumerate() {
                let index = start + offset;
                let row = popup.y.saturating_add(2 + offset as u16);
                if row < popup.bottom() {
                    state.hit_regions.push(HitRegion {
                        area: Rect::new(
                            popup.x.saturating_add(1),
                            row,
                            popup.width.saturating_sub(2),
                            1,
                        ),
                        target: HitTarget::TargetSelectorRow(index),
                    });
                }
            }
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
            render_console_manager(frame, area, app, list, state, theme)
        }
        Overlay::CatalogDropConfirm {
            plan,
            input,
            busy,
            error,
        } => {
            render_catalog_drop_confirm(frame, area, plan, input, *busy, error.as_deref(), theme);
        }
        Overlay::CatalogEditorDestructiveConfirm { plan, input } => {
            render_catalog_mutation_confirm(frame, area, plan, input, theme);
        }
        Overlay::CatalogEditorDiscardConfirm { focus } => {
            use crate::model::workspace::CatalogEditorDiscardFocus;
            let popup = centered(area, 72.min(area.width), 9.min(area.height));
            frame.render_widget(Clear, popup);
            let selected = |candidate| {
                if *focus == candidate {
                    theme.base().bg(theme.selection)
                } else {
                    theme.base()
                }
            };
            let lines = vec![
                Line::from(Span::styled(
                    "Discard unsaved table changes?",
                    theme.title(true),
                )),
                Line::raw("Your draft has not been applied to the database."),
                Line::styled(
                    "Keep Editing",
                    selected(CatalogEditorDiscardFocus::KeepEditing),
                ),
                Line::styled(
                    "Discard Changes",
                    selected(CatalogEditorDiscardFocus::DiscardChanges),
                ),
                Line::raw("Up/Down select  Enter confirm  Esc keep editing"),
            ];
            for (offset, target) in [
                (2, HitTarget::CatalogEditorDiscardKeepEditing),
                (3, HitTarget::CatalogEditorDiscardChanges),
            ] {
                state.hit_regions.push(HitRegion {
                    area: Rect::new(
                        popup.x + 1,
                        popup.y + 1 + offset,
                        popup.width.saturating_sub(2),
                        1,
                    ),
                    target,
                });
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(" TABLE EDITOR ", true, theme))
                    .style(Style::new().bg(theme.surface_raised)),
                popup,
            );
        }
        Overlay::ProfileGroup(group) => {
            render_profile_group_overlay(frame, area, app, group, state, theme);
        }
        Overlay::ExplorerAdd(menu) => {
            render_explorer_add(frame, area, app, menu, state, theme, icons)
        }
    }
}

fn render_transaction_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    selected: usize,
    theme: Theme,
    state: &mut UiState,
) {
    use crate::model::transaction::TransactionMode;

    let popup = centered(area, 58, 9);
    frame.render_widget(Clear, popup);
    let availability = app.transaction_menu_availability();
    let labels = ["Auto", "Manual", "Resolve Transaction", "Cancel"];
    let mut lines = vec![Line::from(Span::styled(
        " TRANSACTION MODE ",
        theme.title(true),
    ))];
    for (index, label) in labels.iter().enumerate() {
        let (enabled, reason) = availability[index];
        let current = app.active_console_opt().is_some_and(|tab| {
            matches!(
                (index, tab.transaction_mode),
                (0, TransactionMode::Auto) | (1, TransactionMode::Manual)
            )
        });
        let marker = if index == selected { ">" } else { " " };
        let suffix = if !enabled {
            reason
        } else if current {
            " (current)"
        } else {
            ""
        };
        lines.push(Line::from(Span::styled(
            format!("{marker} {label}{suffix}"),
            if index == selected && enabled {
                Style::new()
                    .fg(theme.text)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else if !enabled {
                Style::new().fg(theme.muted).bg(theme.surface_raised)
            } else if current {
                Style::new().fg(theme.accent).bg(theme.surface_raised)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            },
        )));
        let row = popup.y.saturating_add(2 + index as u16);
        if row < popup.bottom() {
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    popup.x.saturating_add(1),
                    row,
                    popup.width.saturating_sub(2),
                    1,
                ),
                target: if index == 3 {
                    HitTarget::TransactionMenuCancel
                } else {
                    HitTarget::TransactionMenuItem(index)
                },
            });
        }
    }
    lines.push(Line::raw("Up/Down select; Enter choose; Esc cancels"));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" TRANSACTION ", true, theme))
            .style(Style::new().bg(theme.surface_raised)),
        popup,
    );
}

fn render_explorer_add(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    menu: &crate::model::explorer_add::ExplorerAddMenu,
    state: &mut UiState,
    theme: Theme,
    icons: icons::IconSet,
) {
    use crate::model::explorer_add::{ExplorerAddAvailability, ExplorerAddKind};

    let popup = centered(area, 64.min(area.width), 14.min(area.height));
    frame.render_widget(Clear, popup);
    let block = panel_block(" ADD TO CONNECTION ", true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let profile = app
        .profiles
        .iter()
        .find(|profile| profile.id == menu.profile_id);
    let target = profile.map_or_else(
        || "TARGET  connection".to_owned(),
        |profile| {
            format!(
                "TARGET  {} · {:?}",
                sanitize_terminal_text(&profile.name),
                profile.kind
            )
        },
    );
    frame.render_widget(
        Paragraph::new(target).style(Style::new().fg(theme.muted).bg(theme.surface)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let row_start = inner.y.saturating_add(2);
    for (index, option) in menu.options.iter().enumerate() {
        let y = row_start.saturating_add(index as u16);
        if y >= inner.bottom().saturating_sub(1) {
            break;
        }
        let selected = index == menu.selected;
        let available = option.availability.is_available();
        let icon = match option.kind {
            ExplorerAddKind::Connection => icons.explorer_add(icons::ExplorerAddIcon::Connection),
            ExplorerAddKind::ConnectionGroup => {
                icons.explorer_add(icons::ExplorerAddIcon::ConnectionGroup)
            }
            ExplorerAddKind::Database => icons.catalog(crate::db::catalog::CatalogKind::Database),
            ExplorerAddKind::User => icons.explorer_add(icons::ExplorerAddIcon::User),
            ExplorerAddKind::Role => icons.explorer_add(icons::ExplorerAddIcon::Role),
        };
        let icon_color = match option.kind {
            ExplorerAddKind::Connection => theme.action,
            ExplorerAddKind::ConnectionGroup => theme.warning,
            ExplorerAddKind::Database => theme.accent,
            ExplorerAddKind::User => theme.success,
            ExplorerAddKind::Role => theme.warning,
        };
        let detail = match option.availability {
            ExplorerAddAvailability::Available => option.kind.description(),
            ExplorerAddAvailability::Unavailable(reason) => reason,
        };
        let row = Rect::new(inner.x, y, inner.width, 1);
        let background = if selected {
            theme.selection
        } else {
            theme.surface
        };
        let label_style = Style::new()
            .fg(if available { theme.text } else { theme.muted })
            .bg(background)
            .add_modifier(if selected && available {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let icon_style = Style::new()
            .fg(if available { icon_color } else { theme.muted })
            .bg(background);
        let detail_style = Style::new().fg(theme.muted).bg(background);
        let mut spans = vec![
            Span::styled(if selected { "› " } else { "  " }, label_style),
            Span::styled(format!("{icon} "), icon_style),
            Span::styled(format!("{:<19}", option.kind.label()), label_style),
        ];
        if inner.width >= 52 {
            spans.push(Span::styled(detail, detail_style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
        if available {
            state.hit_regions.push(HitRegion {
                area: row,
                target: HitTarget::ExplorerAddOption(index),
            });
        }
    }
    frame.render_widget(
        Paragraph::new(shortcut_hints::line(
            &[
                ShortcutHint::new("j/k · ↑/↓", "select"),
                ShortcutHint::new("Enter", "continue"),
                ShortcutHint::new("Esc", "close"),
            ],
            inner.width,
            theme,
            theme.surface,
        ))
        .style(Style::new().bg(theme.surface))
        .alignment(Alignment::Center),
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
    );
}

fn render_transaction_exit_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    prompt: &crate::model::transaction::DeferredTransactionPrompt,
    choice: crate::model::transaction::TransactionExitChoice,
    theme: Theme,
    state: &mut UiState,
) {
    use crate::model::transaction::{DeferredIntent, TransactionState};

    let pending = std::iter::once(prompt.console_id)
        .chain(
            app.deferred_transaction_prompts()
                .filter(|queued| queued.intent == prompt.intent)
                .map(|queued| queued.console_id),
        )
        .collect::<Vec<_>>();
    let popup = centered(area, 68, (pending.len() as u16).saturating_add(7).max(9));
    frame.render_widget(Clear, popup);

    let title = if prompt.intent == DeferredIntent::Quit {
        " PENDING TRANSACTIONS "
    } else {
        " TRANSACTION "
    };
    let block = panel_block(title, true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    frame.render_widget(
        Paragraph::new("TRANSACTION SUMMARY").style(
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let row_start = inner.y.saturating_add(2);
    for (index, id) in pending.iter().enumerate() {
        let y = row_start.saturating_add(index as u16);
        if y >= inner.bottom().saturating_sub(2) {
            break;
        }
        let tab = app.tabs.iter().find(|tab| tab.id() == *id);
        let transaction_state = tab
            .and_then(|tab| tab.as_console())
            .map(|console| console.transaction_state);
        render_transaction_summary_row(
            frame,
            Rect::new(inner.x, y, inner.width, 1),
            index == 0,
            tab.map_or("unknown", |tab| tab.title()),
            transaction_state,
            theme,
        );
    }

    let current_console = app
        .tabs
        .iter()
        .find(|tab| tab.id() == prompt.console_id)
        .and_then(|tab| tab.as_console());
    let running =
        current_console.is_some_and(|console| console.query_status == QueryStatus::Running);
    let outcome_unknown = current_console
        .is_some_and(|console| console.transaction_state == TransactionState::OutcomeUnknown);
    let commit_enabled = !current_console
        .is_some_and(|console| console.transaction_state == TransactionState::Aborted);
    let action_area = Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1);
    let footer_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);

    if running {
        frame.render_widget(
            Paragraph::new("QUERY IN PROGRESS  wait or Ctrl-C to cancel")
                .style(
                    Style::new()
                        .fg(theme.warning)
                        .bg(theme.surface)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center),
            action_area,
        );
        frame.render_widget(
            Paragraph::new("Esc return")
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(Alignment::Center),
            footer_area,
        );
    } else if outcome_unknown {
        render_unknown_transaction_actions(frame, action_area, choice, theme);
        frame.render_widget(
            Paragraph::new("A abandon   Esc cancel")
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(Alignment::Center),
            footer_area,
        );
    } else {
        render_transaction_exit_actions(frame, action_area, choice, commit_enabled, theme, state);
        frame.render_widget(
            Paragraph::new("Tab/←/→ select   Enter confirm   Esc cancel")
                .style(Style::new().fg(theme.muted).bg(theme.surface))
                .alignment(Alignment::Center),
            footer_area,
        );
    }
}

fn render_transaction_summary_row(
    frame: &mut Frame<'_>,
    area: Rect,
    current: bool,
    title: &str,
    state: Option<crate::model::transaction::TransactionState>,
    theme: Theme,
) {
    let (state_label, state_color) = transaction_state_display(state, theme);
    let marker = if current { "› " } else { "  " };
    let state_width = state_label.cell_width();
    let title_width = area
        .width
        .saturating_sub(marker.cell_width())
        .saturating_sub(state_width)
        .saturating_sub(2);
    let sanitized_title = sanitize_terminal_text(title);
    let title = truncate_to_cell_width(&sanitized_title, title_width);
    let padding = " ".repeat(usize::from(title_width.saturating_sub(title.cell_width())) + 2);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                marker,
                Style::new()
                    .fg(if current { theme.action } else { theme.muted })
                    .bg(theme.surface)
                    .add_modifier(if current {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                title,
                Style::new()
                    .fg(if current { theme.text } else { theme.muted })
                    .bg(theme.surface),
            ),
            Span::styled(padding, Style::new().bg(theme.surface)),
            Span::styled(
                state_label,
                Style::new()
                    .fg(state_color)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        area,
    );
}

fn transaction_state_display(
    state: Option<crate::model::transaction::TransactionState>,
    theme: Theme,
) -> (&'static str, Color) {
    use crate::model::transaction::TransactionState;

    match state {
        Some(TransactionState::Active) => ("ACTIVE", theme.warning),
        Some(TransactionState::Aborted) => ("ABORTED", theme.error),
        Some(TransactionState::Starting) => ("STARTING", theme.action),
        Some(TransactionState::Committing) => ("COMMITTING", theme.action),
        Some(TransactionState::RollingBack) => ("ROLLING BACK", theme.action),
        Some(TransactionState::OutcomeUnknown) => ("OUTCOME UNKNOWN", theme.error),
        Some(TransactionState::Idle) => ("IDLE", theme.muted),
        None => ("GONE", theme.muted),
    }
}

fn render_transaction_exit_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    choice: crate::model::transaction::TransactionExitChoice,
    commit_enabled: bool,
    theme: Theme,
    state: &mut UiState,
) {
    use crate::model::transaction::TransactionExitChoice;

    let commit = "[ Commit ]";
    let rollback = "[ Rollback ]";
    let cancel = "Cancel";
    let gap = 2;
    let total_width = commit
        .cell_width()
        .saturating_add(rollback.cell_width())
        .saturating_add(cancel.cell_width())
        .saturating_add(gap * 2);
    let mut x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);

    let actions = [
        (commit, TransactionExitChoice::Commit, commit_enabled),
        (rollback, TransactionExitChoice::Rollback, true),
    ];
    for (label, action, enabled) in actions {
        let width = label.cell_width();
        let selected = enabled && choice == action;
        let style = if selected {
            Style::new()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else if enabled {
            Style::new().fg(theme.text).bg(theme.surface)
        } else {
            Style::new().fg(theme.muted).bg(theme.surface)
        };
        frame.render_widget(
            Paragraph::new(label).style(style),
            Rect::new(x, area.y, width, 1),
        );
        if enabled {
            state.hit_regions.push(HitRegion {
                area: Rect::new(x, area.y, width, 1),
                target: HitTarget::TransactionExitChoice(action),
            });
        }
        x = x.saturating_add(width).saturating_add(gap);
    }
    frame.render_widget(
        Paragraph::new(cancel).style(
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(x, area.y, cancel.cell_width(), 1),
    );
    state.hit_regions.push(HitRegion {
        area: Rect::new(x, area.y, cancel.cell_width(), 1),
        target: HitTarget::TransactionExitCancel,
    });
}

fn render_unknown_transaction_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    choice: crate::model::transaction::TransactionExitChoice,
    theme: Theme,
) {
    use crate::model::transaction::TransactionExitChoice;

    let abandon = "[ Abandon local state ]";
    let cancel = "Cancel";
    let gap = 2;
    let total_width = abandon
        .cell_width()
        .saturating_add(cancel.cell_width())
        .saturating_add(gap);
    let x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);
    frame.render_widget(
        Paragraph::new(abandon).style(if choice == TransactionExitChoice::Abandon {
            Style::new()
                .fg(theme.background)
                .bg(theme.error)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text).bg(theme.surface)
        }),
        Rect::new(x, area.y, abandon.cell_width(), 1),
    );
    frame.render_widget(
        Paragraph::new(cancel).style(
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            x.saturating_add(abandon.cell_width()).saturating_add(gap),
            area.y,
            cancel.cell_width(),
            1,
        ),
    );
}

fn render_profile_group_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    group: &crate::model::profile_group::ProfileGroupOverlay,
    state: &mut UiState,
    theme: Theme,
) {
    use crate::model::profile_group::ProfileGroupOverlay;

    match group {
        ProfileGroupOverlay::Picker { selected, busy, .. } => {
            let option_count = app.connection_groups.len() + 2;
            let height = (option_count as u16 + 5).clamp(8, 22);
            let popup = centered(area, 64, height);
            frame.render_widget(Clear, popup);
            let block = panel_block(" SELECT CONNECTION GROUP ", true, theme);
            let inner = block.inner(popup);
            frame.render_widget(block, popup);

            frame.render_widget(
                Paragraph::new("ASSIGN CONNECTION").style(
                    Style::new()
                        .fg(theme.muted)
                        .bg(theme.surface)
                        .add_modifier(Modifier::BOLD),
                ),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
            let names = std::iter::once("Ungrouped".to_owned())
                .chain(app.connection_groups.iter().map(|group| group.name.clone()))
                .chain(std::iter::once("+ Create group...".to_owned()));
            for (index, name) in names.enumerate() {
                let row = Rect::new(
                    inner.x,
                    inner.y.saturating_add(1 + index as u16),
                    inner.width,
                    1,
                );
                if row.y >= inner.bottom().saturating_sub(1) {
                    break;
                }
                let active = index == *selected;
                frame.render_widget(
                    Paragraph::new(format!("{} {name}", if active { "›" } else { " " })).style(
                        Style::new()
                            .fg(if active { theme.text } else { theme.muted })
                            .bg(if active {
                                theme.selection
                            } else {
                                theme.surface
                            })
                            .add_modifier(if active {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                    row,
                );
                if !busy {
                    state.hit_regions.push(HitRegion {
                        area: row,
                        target: HitTarget::ProfileGroupOption(index),
                    });
                }
            }
            frame.render_widget(
                Paragraph::new(if *busy {
                    Line::from(Span::styled(
                        "Updating group...",
                        Style::new().fg(theme.muted).bg(theme.surface),
                    ))
                } else {
                    shortcut_hints::line(
                        &[
                            ShortcutHint::new("↑/↓", "select"),
                            ShortcutHint::new("Enter", "apply"),
                            ShortcutHint::new("Esc", "cancel"),
                        ],
                        inner.width,
                        theme,
                        theme.surface,
                    )
                })
                .style(Style::new().bg(theme.surface))
                .alignment(Alignment::Center),
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            );
        }
        ProfileGroupOverlay::Edit {
            group_id,
            name,
            error,
            busy,
        } => {
            let title = if group_id.is_some() {
                " EDIT CONNECTION GROUP "
            } else {
                " NEW CONNECTION GROUP "
            };
            let popup = centered(area, 64, 8);
            frame.render_widget(Clear, popup);
            let block = panel_block(title, true, theme);
            let inner = block.inner(popup);
            frame.render_widget(block, popup);

            frame.render_widget(
                Paragraph::new(if *busy {
                    "BUSY // SAVING GROUP"
                } else {
                    "GROUP DETAILS"
                })
                .style(
                    Style::new()
                        .fg(if *busy { theme.warning } else { theme.muted })
                        .bg(theme.surface)
                        .add_modifier(Modifier::BOLD),
                ),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );

            let field_y = inner.y.saturating_add(2);
            let label_width = inner.width.min(16);
            let label_area = Rect::new(inner.x, field_y, label_width, 1);
            let input_area = Rect::new(
                inner.x.saturating_add(label_width),
                field_y,
                inner.width.saturating_sub(label_width),
                1,
            );
            frame.render_widget(
                Paragraph::new("› Group name").style(
                    Style::new()
                        .fg(theme.action)
                        .bg(theme.surface)
                        .add_modifier(Modifier::BOLD),
                ),
                label_area,
            );
            let input_style = Style::new()
                .fg(if *busy { theme.muted } else { theme.text })
                .bg(theme.selection);
            if *busy {
                frame.render_widget(Paragraph::new(name.value()).style(input_style), input_area);
            } else {
                render_text_input(frame, input_area, "", name, input_style, state);
            }

            if let Some(error) = error {
                frame.render_widget(
                    Paragraph::new(format!("× {}", sanitize_terminal_text(error)))
                        .style(Style::new().fg(theme.error).bg(theme.surface)),
                    Rect::new(inner.x, field_y.saturating_add(1), inner.width, 1),
                );
            }
            render_profile_group_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                if *busy { "Saving..." } else { "Save group" },
                !busy,
                state,
                theme,
            );
            frame.render_widget(
                Paragraph::new(shortcut_hints::line(
                    &[
                        ShortcutHint::new("Enter", "save"),
                        ShortcutHint::new("Esc", "cancel"),
                        ShortcutHint::new("Ctrl-W/U/A/E", "edit"),
                    ],
                    inner.width,
                    theme,
                    theme.surface,
                ))
                .style(Style::new().bg(theme.surface))
                .alignment(Alignment::Center),
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            );
        }
        ProfileGroupOverlay::DeleteConfirm {
            member_count, busy, ..
        } => {
            let popup = centered(area, 64, 8);
            frame.render_widget(Clear, popup);
            let block = panel_block(" DELETE CONNECTION GROUP ", true, theme);
            let inner = block.inner(popup);
            frame.render_widget(block, popup);
            frame.render_widget(
                Paragraph::new(format!(
                    "Delete this group?\n\n{member_count} connection(s) will move to Ungrouped."
                ))
                .style(Style::new().fg(theme.text).bg(theme.surface))
                .alignment(Alignment::Center),
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(2),
                ),
            );
            render_profile_group_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                if *busy { "Deleting..." } else { "Delete group" },
                !busy,
                state,
                theme,
            );
            frame.render_widget(
                Paragraph::new(shortcut_hints::line(
                    &[
                        ShortcutHint::new("Enter", "delete"),
                        ShortcutHint::new("Esc", "cancel"),
                    ],
                    inner.width,
                    theme,
                    theme.surface,
                ))
                .style(Style::new().bg(theme.surface))
                .alignment(Alignment::Center),
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            );
        }
    }
}

fn render_profile_group_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    confirm_label: &str,
    enabled: bool,
    state: &mut UiState,
    theme: Theme,
) {
    let confirm = format!("[ {confirm_label} ]");
    let cancel = "[ Cancel ]";
    let total_width = confirm.cell_width() + cancel.cell_width() + 1;
    let x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);
    let confirm_area = Rect::new(x, area.y, confirm.cell_width(), 1);
    let cancel_area = Rect::new(
        confirm_area.right().saturating_add(1),
        area.y,
        cancel.cell_width(),
        1,
    );
    frame.render_widget(
        Paragraph::new(confirm).style(if enabled {
            Style::new()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.muted).bg(theme.surface_raised)
        }),
        confirm_area,
    );
    frame.render_widget(
        Paragraph::new(cancel).style(
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        cancel_area,
    );
    if enabled {
        state.hit_regions.push(HitRegion {
            area: confirm_area,
            target: HitTarget::ProfileGroupConfirm,
        });
        state.hit_regions.push(HitRegion {
            area: cancel_area,
            target: HitTarget::ProfileGroupCancel,
        });
    }
}

fn render_console_manager(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    list: &crate::model::sql_editor_list::SqlEditorListState,
    state: &mut UiState,
    theme: Theme,
) {
    use crate::model::sql_editor_list::SqlEditorListMode;

    let records = app.visible_console_records(list.visible_query());
    let mode_height = match &list.mode {
        SqlEditorListMode::Browse | SqlEditorListMode::Search => 4,
        SqlEditorListMode::Rename { error, .. } => 5 + u16::from(error.is_some()),
        SqlEditorListMode::DeleteConfirm { .. } => 5,
    };
    let desired_height = match &list.mode {
        SqlEditorListMode::Browse | SqlEditorListMode::Search => {
            records
                .len()
                .min(usize::from(area.height.saturating_sub(mode_height))) as u16
                + mode_height
        }
        _ => mode_height,
    };
    let popup = centered(area, 72, desired_height.clamp(8, 24));
    frame.render_widget(Clear, popup);

    let title = match &list.mode {
        SqlEditorListMode::Browse => " CONSOLES ",
        SqlEditorListMode::Search => " CONSOLES // SEARCH ",
        SqlEditorListMode::Rename { .. } => " CONSOLES // RENAME ",
        SqlEditorListMode::DeleteConfirm { .. } => " CONSOLES // DELETE ",
    };
    let inner = popup.inner(ratatui::layout::Margin::new(1, 1));
    let mut lines = Vec::new();
    match &list.mode {
        SqlEditorListMode::Browse | SqlEditorListMode::Search => {
            if matches!(&list.mode, SqlEditorListMode::Search) {
                lines.push(Line::raw(format!("/{}", list.visible_query())));
            }
            if records.is_empty() {
                lines.push(Line::from(Span::styled(
                    "No matching consoles",
                    theme.muted,
                )));
            } else {
                let status_width = 6usize;
                let name_width = usize::from(inner.width).saturating_sub(4 + status_width);
                lines.extend(records.iter().map(|record| {
                    let selected = list.selected_id == Some(record.id);
                    let name = truncate_to_cells(&record.name, name_width);
                    let status = if record.open { "OPEN" } else { "CLOSED" };
                    let background = if selected {
                        theme.selection
                    } else {
                        theme.surface
                    };
                    let prefix = if selected { "> " } else { "  " };
                    let padding =
                        status_width + name_width.saturating_sub(usize::from(name.cell_width()));
                    Line::from(vec![
                        Span::styled(
                            format!("{prefix}{name}"),
                            theme.base().bg(background).add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                        ),
                        Span::styled(
                            format!("{:>padding$}", status),
                            theme
                                .base()
                                .fg(if record.open {
                                    theme.success
                                } else {
                                    theme.muted
                                })
                                .bg(background),
                        ),
                    ])
                }));
            }
            lines.push(Line::raw(""));
            let footer = if matches!(&list.mode, SqlEditorListMode::Search) {
                "Enter open  Esc cancel"
            } else {
                "j/k move  Enter open  a new  d delete  r rename  / search  Esc close"
            };
            lines.push(Line::from(Span::styled(
                truncate_to_cells(footer, usize::from(inner.width)),
                theme.muted,
            )));
        }
        SqlEditorListMode::Rename {
            console_id,
            input,
            error,
        } => {
            let old_name = app
                .sql_editors
                .iter()
                .find(|record| record.id == *console_id)
                .map(|record| record.name.as_str())
                .unwrap_or("unknown");
            lines.push(Line::raw(format!("Rename {old_name}")));
            lines.push(Line::raw(""));
            lines.push(Line::raw(format!("Name: {}", input.value())));
            if let Some(error) = error {
                lines.push(Line::from(Span::styled(error.clone(), theme.error)));
            }
            lines.push(Line::from(Span::styled(
                "Enter save  Esc cancel",
                theme.muted,
            )));
        }
        SqlEditorListMode::DeleteConfirm { console_id } => {
            let name = app
                .sql_editors
                .iter()
                .find(|record| record.id == *console_id)
                .map(|record| record.name.as_str())
                .unwrap_or("unknown");
            lines.push(Line::raw(format!(
                "Permanently delete '{name}' and its saved SQL file?"
            )));
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "Enter delete  Esc cancel",
                theme.muted,
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(title, true, theme))
            .style(theme.base()),
        popup,
    );
    match &list.mode {
        SqlEditorListMode::Search => {
            render_text_input(
                frame,
                Rect::new(inner.x, inner.y, inner.width, 1),
                "/",
                &list.query,
                theme.base(),
                state,
            );
        }
        SqlEditorListMode::Rename { input, .. } => {
            render_text_input(
                frame,
                Rect::new(inner.x, inner.y + 2, inner.width, 1),
                "Name: ",
                input,
                theme.base(),
                state,
            );
        }
        _ => {}
    }
}

fn render_catalog_mutation_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &crate::db::catalog_mutation::CatalogMutationPlan,
    input: &crate::model::text_input::TextInput,
    theme: Theme,
) {
    let popup = centered(area, 82, 16);
    frame.render_widget(Clear, popup);
    let mut lines = vec![
        Line::from(Span::styled(
            " DESTRUCTIVE CATALOG MUTATION ",
            theme.title(true),
        )),
        Line::raw("This operation may lose data:"),
        Line::raw(plan.sql()),
        Line::raw(""),
    ];
    lines.extend(
        plan.warnings
            .iter()
            .map(|warning| Line::raw(sanitize_terminal_text(warning))),
    );
    lines.extend([
        Line::raw("Type exactly lowercase y and press Enter to execute:"),
        Line::from(Span::styled(
            format!("> {}", input.value()),
            Style::new().fg(theme.accent),
        )),
        Line::raw("Esc cancel"),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" CATALOG MUTATION CONFIRMATION ", true, theme))
            .wrap(Wrap { trim: true }),
        popup,
    );
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
    let entries = crate::help::filtered_shortcuts_with_bindings(
        help.context,
        help.capabilities,
        help.query.value(),
        Some(&help.bindings),
    );
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
    render_text_input(
        frame,
        chunks[0],
        "Search ",
        &help.query,
        Style::new().fg(theme.accent).bg(theme.surface_raised),
        state,
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
                    format!(
                        "{marker} {:<18}",
                        crate::help::configured_sequence(shortcut, Some(&help.bindings))
                    ),
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
    for (offset, shortcut) in entries.iter().enumerate().skip(start).take(visible_height) {
        let row = Rect::new(
            chunks[2].x,
            chunks[2].y.saturating_add(offset as u16 - start as u16),
            chunks[2].width,
            1,
        );
        state.hit_regions.push(HitRegion {
            area: row,
            target: HitTarget::OpenTextDetail(readonly_detail_request(
                "Keyboard shortcut",
                format!(
                    "{}  {}",
                    crate::help::configured_sequence(shortcut, Some(&help.bindings)),
                    shortcut.description
                ),
            )),
        });
    }
    frame.render_widget(
        Paragraph::new("Up/Down select   Enter run   Esc close   Ctrl-W/U/A/E edit")
            .style(Style::new().fg(theme.muted).bg(theme.surface_raised)),
        chunks[3],
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

fn render_update_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    use crate::model::update::{UpdateOverlayFocus, UpdateState};

    let popup = centered(area, 76, 16);
    frame.render_widget(Clear, popup);
    let block = panel_block(" UPDATE CENTER ", true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let mut lines = vec![Line::from(Span::styled(
        " LAZYDB UPDATE",
        theme.title(true),
    ))];
    let (primary, primary_enabled, status_line) = match &app.update_state {
        UpdateState::Idle => ("Check now", true, "No update check has run".to_owned()),
        UpdateState::Checking { .. } => {
            ("Checking...", false, "Checking for updates...".to_owned())
        }
        UpdateState::UpToDate(inspection) => (
            "Check again",
            true,
            format!(
                "Running {} · Latest {}",
                inspection.running_version,
                inspection.target_version.as_deref().unwrap_or("unknown")
            ),
        ),
        UpdateState::Available(inspection) => (
            "Update now",
            inspection.manager == crate::update::InstallationManager::Native,
            format!(
                "Running {} · Latest {} · {:?}",
                inspection.running_version,
                inspection.target_version.as_deref().unwrap_or("unknown"),
                inspection.channel
            ),
        ),
        UpdateState::Installing { inspection, .. } => (
            "Installing...",
            false,
            format!(
                "Installing {}. You can continue using LazyDB.",
                inspection.target_version.as_deref().unwrap_or("the update")
            ),
        ),
        UpdateState::ReadyToRestart(inspection) => (
            "Restart now",
            true,
            format!(
                "Running {} · Installed {}",
                inspection.running_version,
                inspection.installed_version.as_deref().unwrap_or("unknown")
            ),
        ),
        UpdateState::ManagerActionRequired(inspection) => (
            "Copy command",
            false,
            format!(
                "Latest {} · {:?}: {}",
                inspection.target_version.as_deref().unwrap_or("unknown"),
                inspection.manager,
                inspection
                    .action
                    .as_deref()
                    .unwrap_or("use the installation manager to update")
            ),
        ),
        UpdateState::Failed { message, .. } => (
            "Retry",
            true,
            format!("Update failed: {}", sanitize_terminal_text(message)),
        ),
    };
    lines.push(Line::raw(status_line));
    lines.push(Line::raw(""));
    if let UpdateState::Available(inspection) = &app.update_state {
        lines.push(Line::raw(format!("Installation: {:?}", inspection.manager)));
    }
    lines.push(Line::raw(""));
    let later_selected = matches!(
        app.overlay,
        Some(Overlay::Update(ref overlay)) if overlay.focus == UpdateOverlayFocus::Later
    );
    let primary_selected = !later_selected;
    let primary_label = if primary_enabled && primary_selected {
        format!("[ {primary} ]")
    } else {
        format!("  {primary}  ")
    };
    let later_label = if later_selected {
        "[ Later ]"
    } else {
        "  Later  "
    };
    lines.push(Line::from(vec![
        Span::styled(
            primary_label,
            Style::new()
                .fg(if primary_enabled {
                    theme.action
                } else {
                    theme.muted
                })
                .bg(theme.surface),
        ),
        Span::raw("   "),
        Span::styled(later_label, Style::new().fg(theme.text).bg(theme.surface)),
    ]));
    lines.push(Line::from(Span::styled(
        "Tab/Left/Right select   Enter confirm   Esc/q close",
        Style::new().fg(theme.muted),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::new().fg(theme.text).bg(theme.surface))
            .wrap(Wrap { trim: true }),
        inner,
    );
    let action_y = inner.bottom().saturating_sub(2);
    let button_width = (inner.width / 2).max(1);
    if primary_enabled {
        state.hit_regions.push(HitRegion {
            area: Rect::new(inner.x, action_y, button_width, 1),
            target: HitTarget::UpdateButton { primary: true },
        });
    }
    state.hit_regions.push(HitRegion {
        area: Rect::new(
            inner.x.saturating_add(button_width),
            action_y,
            inner.width.saturating_sub(button_width),
            1,
        ),
        target: HitTarget::UpdateButton { primary: false },
    });
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
}

#[cfg(test)]
mod tab_viewport_tests {
    use super::*;

    #[test]
    fn tab_viewport_uses_full_width_without_overflow_controls() {
        assert_eq!(
            tab_viewport(&[8, 10, 12], 1, 30),
            TabViewport {
                start: 0,
                end: 3,
                overflowed: false,
            }
        );
    }

    #[test]
    fn tab_viewport_keeps_active_tab_visible_at_each_position() {
        let widths = [8, 8, 8, 8];
        for active in 0..widths.len() {
            let viewport = tab_viewport(&widths, active, 20);
            assert!(viewport.overflowed);
            assert!(viewport.start <= active);
            assert!(active < viewport.end);
            assert!(viewport.end <= widths.len());
        }
    }

    #[test]
    fn tab_viewport_handles_empty_and_oversized_tabs() {
        assert_eq!(
            tab_viewport(&[], 0, 20),
            TabViewport {
                start: 0,
                end: 0,
                overflowed: false,
            }
        );
        let viewport = tab_viewport(&[40], 0, 12);
        assert_eq!(viewport.start, 0);
        assert_eq!(viewport.end, 1);
        assert!(viewport.overflowed);
    }

    #[test]
    fn tab_viewport_treats_exact_fit_as_not_overflowed() {
        assert!(!tab_viewport(&[8, 10], 1, 18).overflowed);
    }

    #[test]
    fn truncate_to_cell_width_does_not_split_wide_characters() {
        assert_eq!(truncate_to_cell_width("界abc", 4), "界a…");
        assert_eq!(truncate_to_cell_width("界abc", 1), "…");
        assert_eq!(truncate_to_cell_width("short", 10), "short");
    }
}

#[cfg(test)]
mod completion_popup_tests {
    use super::*;

    #[test]
    fn completion_columns_prefer_labels_over_details_when_space_is_tight() {
        let columns = CompletionColumns::measure(
            [(2u16, "create_time", "timestamp"), (2, "id", "bigint")].into_iter(),
        );

        assert_eq!(columns.label_offset(), 3);
        assert_eq!(columns.label, 11);
        assert_eq!(columns.detail, 9);
        assert_eq!(columns.content_width(), 26);
        assert_eq!(columns.fit(26), columns);

        let clipped = columns.fit(24);
        assert_eq!(clipped.label, 11);
        assert_eq!(clipped.detail, 7);

        let tight = columns.fit(20);
        assert_eq!(tight.label, 11);
        assert_eq!(tight.detail, 0);
    }

    #[test]
    fn completion_columns_cap_overlong_details() {
        let columns = CompletionColumns::measure(
            [(2u16, "code", "a_very_long_user_defined_type_name")].into_iter(),
        );

        assert_eq!(columns.detail, COMPLETION_DETAIL_MAX_CELLS);
    }

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

    #[test]
    fn bordered_popup_reserves_two_rows_and_columns() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(12, 6),
            replacement_start_x: None,
        };

        assert_eq!(
            completion_popup_rect(anchor, 20 + 2, 4 + 2),
            Some(Rect::new(12, 7, 22, 6))
        );
    }

    #[test]
    fn bordered_popup_geometry_reports_constrained_height_for_callers_to_reject() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 4),
            cursor: Position::new(12, 6),
            replacement_start_x: None,
        };

        assert_eq!(
            completion_popup_rect(anchor, 12, 3),
            Some(Rect::new(12, 7, 12, 2))
        );
        assert_eq!(
            completion_popup_rect(anchor, 12, 3),
            Some(Rect::new(12, 7, 12, 2))
        );
    }

    #[test]
    fn bordered_popup_shrinks_outer_width_without_moving_origin() {
        let anchor = CompletionAnchor {
            viewport: Rect::new(10, 5, 40, 10),
            cursor: Position::new(48, 6),
            replacement_start_x: None,
        };

        assert_eq!(
            completion_popup_rect(anchor, 22, 6),
            Some(Rect::new(48, 7, 2, 6))
        );
    }
}
