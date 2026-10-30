pub mod animation;
pub mod catalog_editor;
pub(crate) mod dashboard;
pub mod data_grid;
pub(crate) mod dialog;
pub(crate) mod dialog_footer;
pub(crate) mod execution_confirm;
mod footer;
pub mod icons;
pub mod layout;
pub mod loading;
pub mod mongodb_browser;
pub mod mongodb_console;
pub mod mongodb_dashboard;
pub mod mongodb_management;
pub mod mongodb_principal;
pub mod notifications;
mod omni;
pub mod pagination;
pub(crate) mod principal;
pub(crate) mod principal_mutation_confirm;
pub(crate) mod principal_mutation_form;
pub mod profiles;
pub mod query_bar;
pub(crate) mod read_only_sql;
pub mod record_view;
pub mod redis_browser;
pub(crate) mod redis_dashboard;
pub mod redis_object_editor;
pub mod redis_table_editor;
pub mod redis_value;
pub mod relation;
pub(crate) mod scrollbar;
mod shortcut_hints;
pub(crate) mod sql_history_modal;
pub(crate) mod sql_preview;
pub mod text_detail;
pub mod text_selection;
pub mod theme;
pub(crate) mod update;

use crate::profile::DatabaseKind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
            QueryStatus, TargetSelectorCandidate, VisibleCatalogNode,
        },
    },
    security::sanitize_terminal_text,
};

use self::{
    dialog::{DialogButton, DialogTone},
    layout::{AppLayout, LayoutMode},
    shortcut_hints::ShortcutHint,
    theme::Theme,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileButton {
    Test,
    Save,
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
    RedisKeyNode {
        tab_id: Uuid,
        node: crate::model::redis_key_tree::KeyTreeNodeId,
    },
    RedisKeyToggle {
        tab_id: Uuid,
        node: crate::model::redis_key_tree::KeyTreeNodeId,
    },
    RedisFindInput(Uuid),
    RedisValueFilter(Uuid),
    RedisPreviewFormat(Uuid),
    RedisPreviewWrap(Uuid),
    RedisPreviewFocus(Uuid),
    RedisPreviewTableCell {
        tab_id: Uuid,
        row: usize,
        column: usize,
    },
    RedisPreviewLoadMore(Uuid),
    MongoPageLoadMore(Uuid),
    MongoQueryField(Uuid, crate::model::mongodb_browser::MongoQueryField),
    RedisValueSaveAction(usize),
    RedisUnsavedValueAction(usize),
    WorkspaceSaveAction(usize),
    ExplorerToggle(crate::model::explorer::ExplorerNodeId),
    ExplorerFind,
    ExplorerSearch,
    ResultCell {
        row: usize,
        column: usize,
    },
    Help,
    UpdateCenter,
    UpdateButton {
        action: crate::model::update::UpdateDialogAction,
    },
    ToggleResultView,
    ResultView(ResultView),
    RelationView(crate::model::relation::RelationView),
    PrincipalView(crate::model::principal::PrincipalView),
    DashboardView(crate::model::dashboard::DashboardPage),
    RelationRetry,
    RelationCancel,
    DataQueryInput(crate::model::data_query::DataQueryInput),
    PaneResize(PaneSplit),
    RelationColumnResize {
        column: usize,
        width: u16,
    },
    GridColumnSort(usize),
    GridScrollbarThumb {
        axis: GridScrollAxis,
        track_start: u16,
        track_length: u16,
        thumb_start: u16,
        thumb_length: u16,
        offset: usize,
        max_offset: usize,
    },
    GridScrollbarPage {
        axis: GridScrollAxis,
        offset: usize,
    },
    EditorScrollbarPage {
        session_id: Uuid,
        rows: isize,
        columns: isize,
    },
    EditorScrollbarThumb {
        session_id: Uuid,
        vertical: bool,
        track_start: u16,
        track_length: u16,
        thumb_start: u16,
        thumb_length: u16,
        offset: usize,
        max_offset: usize,
    },
    ExplorerScrollbarPage {
        offset: usize,
    },
    ExplorerScrollbarThumb {
        track_start: u16,
        track_length: u16,
        thumb_start: u16,
        thumb_length: u16,
        max_offset: usize,
    },
    PrincipalAccessScrollbarPage {
        tab_id: Uuid,
        section: crate::model::principal::PrincipalAccessSection,
        offset: usize,
    },
    PrincipalAccessBody {
        tab_id: Uuid,
        section: crate::model::principal::PrincipalAccessSection,
    },
    PrincipalAccessScrollbarThumb {
        tab_id: Uuid,
        section: crate::model::principal::PrincipalAccessSection,
        track_start: u16,
        track_length: u16,
        thumb_start: u16,
        thumb_length: u16,
        max_offset: usize,
    },
    ProfileField(ProfileField),
    ProfileDriver(crate::profile::DatabaseKind),
    ProfileCategory(crate::db::descriptor::DatabaseCategory),
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
    CatalogEditorAddTableConstraintPrimaryKey,
    CatalogEditorAddTableConstraintUnique,
    CatalogEditorAddTableConstraintCheck,
    CatalogEditorAddTableConstraintForeignKey,
    CatalogEditorFocusTableConstraint(usize),
    CatalogEditorRemoveTableConstraint(usize),
    CatalogEditorConstraintField {
        constraint: usize,
        field: usize,
    },
    CatalogEditorRemoveTableColumn,
    CatalogEditorRestoreTableColumn,
    CatalogEditorRemoveTableColumnRow(uuid::Uuid),
    CatalogEditorRestoreTableColumnRow(uuid::Uuid),
    CatalogEditorReview,
    CatalogEditorCancel,
    CatalogEditorDiscardKeepEditing,
    CatalogEditorDiscardChanges,
    CatalogEditorColumnDetailsConfirm,
    CatalogEditorColumnDetailsCancel,
    CatalogOwnerChoice(String),
    DismissNotification(u64),
    OpenNotificationHistoryAt(u64),
    OpenTextDetail(crate::model::text_detail::TextDetailRequest),
    NotificationHistoryRow(usize),
    SqlHistoryRow(usize),
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
    DatabaseSelectorRow(usize),
    TransactionMenuItem(usize),
    TransactionMenuCancel,
    TransactionExitChoice(crate::model::transaction::TransactionExitChoice),
    TransactionExitCancel,
    ManualCancellationKeepRunning,
    ManualCancellationConfirm,
    ExecutionConfirm,
    PrincipalMutationCancel,
    PrincipalMutationApply,
    PrincipalPermission(usize),
    PrincipalAccessSection(crate::model::principal::PrincipalAccessSection),
    PrincipalAccessItem(usize),
    PrincipalAccessDetails,
    PrincipalMutationForm,
    ExecutionCancel,
    ClearTransactionConfirm,
    ClearTransactionCancel,
    DeleteConsoleConfirm,
    DeleteConsoleCancel,
    RedisDeleteConfirm,
    RedisDeleteCancel,
    SqlEditorListDeleteConfirm,
    CatalogDropCancel,
    CatalogDropConfirm,
    PrincipalDropCancel,
    PrincipalDropConfirm,
    SqlEditorListDeleteCancel,
    SqlEditorListSearch,
    SqlEditorListRename,
    HelpSearch,
    HelpItem(usize),
    HelpTogglePanel,
    HelpScrollbarPage {
        offset: usize,
    },
    HelpScrollbarThumb {
        track_start: u16,
        track_length: u16,
        thumb_start: u16,
        thumb_length: u16,
        max_offset: usize,
    },
    ProfileGroupName,
    KeySequencePopup,
    TextDetailCopyAll,
    TextDetailClose,
    Omni,
    OmniItem(usize),
    OmniTogglePanel,
    OmniScrollbarPage {
        offset: usize,
    },
    OmniScrollbarThumb {
        track_start: u16,
        track_length: u16,
        thumb_start: u16,
        thumb_length: u16,
        max_offset: usize,
    },
    RecordViewCopyCell,
    RecordViewCopyRow,
    RecordViewViewValue,
    Shortcut(Vec<crossterm::event::KeyEvent>),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorSpec {
    pub position: Position,
    pub style: CursorStyle,
}

#[derive(Debug)]
pub struct UiState {
    pub hit_regions: Vec<HitRegion>,
    pub editor_viewport: Option<EditorViewport>,
    pub output_viewport: Option<(Uuid, EditorViewport)>,
    pub transaction_review_viewport: Option<(Uuid, EditorViewport)>,
    pub completion_popup: Option<Rect>,
    pub grid_viewport: Option<DataGridViewport>,
    pub grid_horizontal_scroll: Option<GridHorizontalScrollTargets>,
    pub record_view_fields: Option<(Uuid, usize)>,
    pub explorer_viewport_rows: Option<usize>,
    pub principal_access_viewport: Option<(Uuid, usize, Rect)>,
    pub help_viewport_rows: Option<usize>,
    pub omni_viewport_rows: Option<usize>,
    pub redis_keys_viewport_rows: Option<(Uuid, usize)>,
    pub redis_preview_viewport_rows: Option<(Uuid, usize, usize)>,
    pub redis_editor_viewport: Option<(Uuid, crate::model::editor::EditorViewport)>,
    pub redis_info_scroll: u16,
    pub ddl_viewport: Option<DdlViewportMetrics>,
    pub cursor: Option<CursorSpec>,
    pub terminal_selection_mode: bool,
    pub pane_layout: PaneLayoutMetrics,
    pub click_tracker: RefCell<Option<(crate::model::explorer::ExplorerNodeId, Instant)>>,
    pub redis_click_tracker:
        RefCell<Option<(Uuid, crate::model::redis_key_tree::KeyTreeNodeId, Instant)>>,
    pub relation_resize: RefCell<Option<(usize, u16, u16)>>,
    pub grid_scrollbar_drag: RefCell<Option<GridScrollbarDrag>>,
    pub editor_scrollbar_drag: RefCell<Option<EditorScrollbarDrag>>,
    pub explorer_scrollbar_drag: RefCell<Option<ExplorerScrollbarDrag>>,
    pub principal_access_scrollbar_drag: RefCell<Option<PrincipalAccessScrollbarDrag>>,
    pub panel_scrollbar_drag: RefCell<Option<PanelScrollbarDrag>>,
    pub pane_resize_drag: RefCell<Option<PaneResizeDrag>>,
    pub mouse_gesture: RefCell<Option<text_selection::GestureOwner>>,
    pub text_gesture: RefCell<Option<text_selection::TextGesture>>,
    pub input_gesture: RefCell<Option<text_selection::InputGesture>>,
    pub text_selection_targets: Vec<text_selection::TextSelectionTarget>,
    pub profile_input_targets: Vec<(ProfileField, text_selection::InputHitMap)>,
    pub catalog_input_targets: Vec<(
        crate::action::CatalogEditorCursorTarget,
        text_selection::InputHitMap,
    )>,
    pub input_selection_targets: Vec<(
        text_selection::InputSelectionTarget,
        text_selection::InputHitMap,
    )>,
    pub(crate) query_bar_highlights: query_bar::QueryBarHighlightCache,
    pub(crate) sql_history_kind_cache: SqlHistoryKindCache,
    pub(crate) animations: animation::AnimationState,
    pub(crate) result_area: Option<Rect>,
    pub(crate) activity_icons: icons::IconSet,
    pub(crate) grid_width_cache: Option<GridWidthCache>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GridWidthCache {
    pub tab_id: Uuid,
    pub result_revision: u64,
    pub edit_revision: u64,
    pub row_count: usize,
    pub column_count: usize,
    pub widths: Vec<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DdlViewportMetrics {
    pub visible_rows: usize,
    pub visible_columns: usize,
    pub total_rows: usize,
    pub max_line_width: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridScrollAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridScrollbarDrag {
    pub axis: GridScrollAxis,
    pub track_start: u16,
    pub track_length: u16,
    pub thumb_length: u16,
    pub pointer_offset: u16,
    pub max_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorScrollbarDrag {
    pub session_id: Uuid,
    pub vertical: bool,
    pub track_start: u16,
    pub track_length: u16,
    pub thumb_length: u16,
    pub pointer_offset: u16,
    pub max_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExplorerScrollbarDrag {
    pub track_start: u16,
    pub track_length: u16,
    pub thumb_length: u16,
    pub pointer_offset: u16,
    pub max_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrincipalAccessScrollbarDrag {
    pub tab_id: Uuid,
    pub section: crate::model::principal::PrincipalAccessSection,
    pub track_start: u16,
    pub track_length: u16,
    pub thumb_length: u16,
    pub pointer_offset: u16,
    pub max_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanelScrollbarDrag {
    pub help: bool,
    pub track_start: u16,
    pub track_length: u16,
    pub thumb_length: u16,
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
    pub fn track_redis_click(
        &self,
        tab_id: Uuid,
        node: &crate::model::redis_key_tree::KeyTreeNodeId,
        now: Instant,
    ) -> bool {
        let mut tracker = self.redis_click_tracker.borrow_mut();
        let double = tracker.as_ref().is_some_and(|(last_tab, last_node, at)| {
            *last_tab == tab_id
                && last_node == node
                && now.duration_since(*at) <= Duration::from_millis(400)
        });
        *tracker = if double {
            None
        } else {
            Some((tab_id, node.clone(), now))
        };
        double
    }

    pub fn new() -> Self {
        Self::with_motion(MotionMode::Full)
    }

    pub fn with_motion(mode: MotionMode) -> Self {
        Self {
            hit_regions: Vec::new(),
            editor_viewport: None,
            output_viewport: None,
            transaction_review_viewport: None,
            completion_popup: None,
            grid_viewport: None,
            grid_horizontal_scroll: None,
            record_view_fields: None,
            explorer_viewport_rows: None,
            principal_access_viewport: None,
            help_viewport_rows: None,
            omni_viewport_rows: None,
            redis_keys_viewport_rows: None,
            redis_preview_viewport_rows: None,
            redis_editor_viewport: None,
            redis_info_scroll: 0,
            ddl_viewport: None,
            cursor: None,
            terminal_selection_mode: false,
            pane_layout: PaneLayoutMetrics::default(),
            click_tracker: RefCell::new(None),
            redis_click_tracker: RefCell::new(None),
            relation_resize: RefCell::new(None),
            grid_scrollbar_drag: RefCell::new(None),
            editor_scrollbar_drag: RefCell::new(None),
            explorer_scrollbar_drag: RefCell::new(None),
            principal_access_scrollbar_drag: RefCell::new(None),
            panel_scrollbar_drag: RefCell::new(None),
            pane_resize_drag: RefCell::new(None),
            mouse_gesture: RefCell::new(None),
            text_gesture: RefCell::new(None),
            input_gesture: RefCell::new(None),
            text_selection_targets: Vec::new(),
            profile_input_targets: Vec::new(),
            catalog_input_targets: Vec::new(),
            input_selection_targets: Vec::new(),
            query_bar_highlights: query_bar::QueryBarHighlightCache::default(),
            sql_history_kind_cache: SqlHistoryKindCache::default(),
            animations: animation::AnimationState::new(mode, Instant::now()),
            result_area: None,
            activity_icons: icons::IconSet::default(),
            grid_width_cache: None,
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

    pub(crate) fn update_progress_elapsed(&self, request_id: u64) -> Duration {
        self.animations
            .elapsed(&animation::LoadIdentity::Update { request_id })
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

    pub fn text_selection_target_at(
        &self,
        column: u16,
        row: u16,
    ) -> Option<(
        &text_selection::TextSelectionTarget,
        text_selection::TextPosition,
    )> {
        self.text_selection_targets.iter().rev().find_map(|target| {
            target
                .source_at(column, row)
                .map(|position| (target, position))
        })
    }

    pub fn profile_input_at(&self, column: u16, row: u16) -> Option<(ProfileField, usize)> {
        self.profile_input_targets
            .iter()
            .rev()
            .find_map(|(field, map)| {
                map.source_at(column, row)
                    .map(|position| (*field, position))
            })
    }

    pub fn catalog_input_at(
        &self,
        column: u16,
        row: u16,
    ) -> Option<(crate::action::CatalogEditorCursorTarget, usize)> {
        self.catalog_input_targets
            .iter()
            .rev()
            .find_map(|(target, map)| {
                map.source_at(column, row)
                    .map(|position| (target.clone(), position))
            })
    }

    pub fn input_selection_target_at(
        &self,
        column: u16,
        row: u16,
    ) -> Option<(&text_selection::InputSelectionTarget, usize)> {
        self.input_selection_targets
            .iter()
            .rev()
            .find_map(|(target, map)| {
                map.source_at(column, row)
                    .map(|position| (target, position))
            })
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
            gesture.has_dragged |= gesture.end != end;
            gesture.end = end;
            true
        } else {
            false
        }
    }

    pub fn end_mouse_gesture(&self) -> Option<text_selection::GestureOwner> {
        let owner = self.mouse_gesture.borrow_mut().take();
        self.text_gesture.borrow_mut().take();
        self.input_gesture.borrow_mut().take();
        owner
    }

    pub fn cancel_mouse_gesture(&self) {
        self.mouse_gesture.borrow_mut().take();
        self.text_gesture.borrow_mut().take();
        self.input_gesture.borrow_mut().take();
    }

    pub fn input_selection_is_current(&self, gesture: &text_selection::InputGesture) -> bool {
        self.input_selection_targets
            .iter()
            .any(|(target, map)| target == &gesture.target && map == &gesture.hit_map)
    }
}

#[derive(Debug, Default)]
pub(crate) struct SqlHistoryKindCache {
    identity: Option<(Uuid, u64)>,
    entries: HashMap<Uuid, SqlHistoryKindCacheEntry>,
}

#[derive(Debug)]
struct SqlHistoryKindCacheEntry {
    sql: String,
    dialect: crate::sql::SqlDialect,
    kind: crate::sql::SqlStatementKind,
}

impl SqlHistoryKindCache {
    pub(crate) fn begin(&mut self, overlay_id: Uuid, generation: u64) {
        if self.identity != Some((overlay_id, generation)) {
            self.identity = Some((overlay_id, generation));
            self.entries.clear();
        }
    }

    pub(crate) fn kind(
        &mut self,
        execution_id: Uuid,
        sql: &str,
        dialect: crate::sql::SqlDialect,
    ) -> crate::sql::SqlStatementKind {
        if let Some(entry) = self.entries.get(&execution_id)
            && entry.sql == sql
            && entry.dialect == dialect
        {
            return entry.kind;
        }
        let kind = crate::sql::classify_statement_kind(sql, dialect);
        self.entries.insert(
            execution_id,
            SqlHistoryKindCacheEntry {
                sql: sql.to_owned(),
                dialect,
                kind,
            },
        );
        kind
    }

    pub(crate) fn retain(&mut self, visible_ids: &HashSet<Uuid>) {
        self.entries
            .retain(|execution_id, _| visible_ids.contains(execution_id));
    }
}

#[cfg(test)]
mod sql_history_kind_cache_tests {
    use super::*;

    #[test]
    fn cache_reuses_entries_and_invalidates_changed_inputs() {
        let id = Uuid::new_v4();
        let overlay = Uuid::new_v4();
        let mut cache = SqlHistoryKindCache::default();
        cache.begin(overlay, 1);
        assert_eq!(
            cache.kind(id, "SELECT 1", crate::sql::SqlDialect::Postgres),
            crate::sql::SqlStatementKind::Dql
        );
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(
            cache.kind(
                id,
                "ALTER TABLE users ADD COLUMN active BOOLEAN",
                crate::sql::SqlDialect::Postgres
            ),
            crate::sql::SqlStatementKind::Ddl
        );
        assert_eq!(cache.entries.len(), 1);
        cache.begin(overlay, 2);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn cache_retain_removes_non_visible_entries() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut cache = SqlHistoryKindCache::default();
        cache.begin(Uuid::new_v4(), 1);
        cache.kind(first, "SELECT 1", crate::sql::SqlDialect::Generic);
        cache.kind(second, "SELECT 2", crate::sql::SqlDialect::Generic);
        cache.retain(&HashSet::from([first]));
        assert!(cache.entries.contains_key(&first));
        assert!(!cache.entries.contains_key(&second));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceEmptyState {
    Profiles,
    ActiveConnection,
    OpenTabs,
}

impl WorkspaceEmptyState {
    fn for_app(app: &App) -> Option<Self> {
        if !app.tabs.is_empty() {
            return None;
        }
        if app.connection.status == ConnectionStatus::Disconnected
            && app.sessions.iter().next().is_none()
        {
            return Some(if app.profiles.is_empty() {
                Self::Profiles
            } else {
                Self::ActiveConnection
            });
        }
        app.tabs.is_empty().then_some(Self::OpenTabs)
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Profiles => "NO CONNECTIONS YET",
            Self::ActiveConnection => "NO ACTIVE CONNECTION",
            Self::OpenTabs => "NO OPEN TABS",
        }
    }

    const fn instruction(self, compact: bool) -> &'static str {
        match (self, compact) {
            (Self::Profiles, false) => "Select NEW in Explorer, then press Enter.",
            (Self::ActiveConnection, false) => "Select a connection in Explorer, then press Enter.",
            (Self::OpenTabs, false) => "Open a tab from Explorer or the console list.",
            (Self::Profiles, true) => "Select NEW in Explorer; press Enter.",
            (Self::ActiveConnection, true) => "Select a connection; press Enter.",
            (Self::OpenTabs, true) => "Open a tab from Explorer or consoles.",
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

fn workspace_empty_area(layout: AppLayout) -> Option<Rect> {
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

fn render_empty_workspace(
    frame: &mut Frame<'_>,
    area: Rect,
    workspace: WorkspaceEmptyState,
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
    render_with_state_at(frame, app, state, icons, sequence, theme, Instant::now());
}

fn render_with_state_at(
    frame: &mut Frame<'_>,
    app: &App,
    state: &mut UiState,
    icons: icons::IconSet,
    sequence: Option<&crate::input::keymap::KeySequenceState>,
    theme: Theme,
    now: Instant,
) {
    let area = frame.area();
    state.activity_icons = icons;
    state.observe_animations(app, now);
    frame.render_widget(Block::new().style(theme.base()), area);
    let is_relation = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Relation(_))
    );
    let is_dashboard = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::Dashboard(_))
    );
    let is_redis_browser = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::RedisBrowser(_))
    );
    let is_mongodb_browser = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::MongoBrowser(_))
    );
    let is_mongodb_console = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::MongoConsole(_))
    );
    let is_mongodb_dashboard = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::MongoDashboard(_))
    );
    let is_principal = matches!(
        app.tabs.get(app.active_tab),
        Some(WorkspaceTab::PrincipalDdl(_))
    );
    let layout = AppLayout::calculate(
        area,
        app.focus,
        is_relation
            || is_dashboard
            || is_redis_browser
            || is_mongodb_browser
            || is_mongodb_console
            || is_mongodb_dashboard
            || is_principal,
        app.pane_sizes,
        app.pane_maximized,
    );
    let empty_workspace = WorkspaceEmptyState::for_app(app);
    let editor_rendered = !is_relation
        && !is_dashboard
        && !is_redis_browser
        && !is_mongodb_browser
        && !is_mongodb_console
        && !is_mongodb_dashboard
        && !is_principal
        && empty_workspace.is_none()
        && layout.editor.is_some();
    let redis_layout = is_redis_browser
        .then(|| {
            layout.relation.map(|area| {
                layout::RedisBrowserLayout::calculate(area, app.pane_sizes.redis_keys_width)
            })
        })
        .flatten();
    let pane_drag_invalid = app.overlay.is_some()
        || state
            .pane_resize_drag
            .borrow()
            .is_some_and(|drag| match drag.split {
                PaneSplit::RedisKeysWidth => redis_layout
                    .as_ref()
                    .and_then(layout::RedisBrowserLayout::resize_region)
                    .is_none(),
                PaneSplit::EditorHeight => {
                    !editor_rendered || layout.pane_resize_region(drag.split).is_none()
                }
                split => layout.pane_resize_region(split).is_none(),
            });
    if pane_drag_invalid {
        state.pane_resize_drag.borrow_mut().take();
        if *state.mouse_gesture.borrow()
            == Some(crate::ui::text_selection::GestureOwner::PaneResize)
        {
            state.mouse_gesture.borrow_mut().take();
        }
    }
    state.pane_layout = layout.pane_metrics;
    state.pane_layout.redis_keys_width = redis_layout.as_ref().and_then(|value| value.keys_width);
    state.hit_regions.clear();
    state.editor_viewport = None;
    state.output_viewport = None;
    state.transaction_review_viewport = None;
    state.completion_popup = None;
    state.grid_viewport = None;
    state.grid_horizontal_scroll = None;
    state.record_view_fields = None;
    state.explorer_viewport_rows = None;
    state.principal_access_viewport = None;
    state.help_viewport_rows = None;
    state.omni_viewport_rows = None;
    state.redis_keys_viewport_rows = None;
    state.redis_preview_viewport_rows = None;
    state.redis_editor_viewport = None;
    state.ddl_viewport = None;
    state.cursor = None;
    state.text_selection_targets.clear();
    state.profile_input_targets.clear();
    state.catalog_input_targets.clear();
    state.input_selection_targets.clear();
    state.result_area = None;

    if layout.mode == LayoutMode::TooSmall {
        render_too_small(frame, area, theme);
        if app.omni.is_some() {
            state.hit_regions.clear();
            state.cursor = None;
            dim_background(frame, area, theme);
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Omni,
            });
            omni::render(frame, app, state, theme, icons);
        }
        return;
    }

    if let Some(area) = layout.tabs {
        render_tabs(frame, area, app, theme, state, icons);
    }
    if is_redis_browser {
        if let Some(area) = layout.explorer {
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Explorer),
            });
            render_explorer(frame, area, app, theme, state, icons);
        }
        if let Some(area) = layout.relation {
            redis_browser::render(frame, area, app, state, theme, icons);
        }
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
        footer::render(frame, layout.footer, app, theme, sequence, state);
    } else if is_mongodb_console {
        if let Some(area) = layout.relation {
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Editor),
            });
            let viewport = EditorViewport {
                width: area.width.saturating_sub(2) as usize,
                height: area.height.saturating_sub(2) as usize,
            };
            mongodb_console::render(frame, area, app, viewport);
        }
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
        footer::render(frame, layout.footer, app, theme, sequence, state);
    } else if is_mongodb_dashboard {
        if let Some(area) = layout.relation
            && let Some(WorkspaceTab::MongoDashboard(tab)) = app.tabs.get(app.active_tab)
        {
            mongodb_dashboard::render(frame, area, tab);
        }
        footer::render(frame, layout.footer, app, theme, sequence, state);
    } else if is_mongodb_browser {
        if let Some(area) = layout.explorer {
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Explorer),
            });
            render_explorer(frame, area, app, theme, state, icons);
        }
        if let Some(area) = layout.relation {
            mongodb_browser::render(frame, area, app, theme, state);
        }
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
        footer::render(frame, layout.footer, app, theme, sequence, state);
    } else if is_dashboard {
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
            let redis_dashboard = app
                .tabs
                .get(app.active_tab)
                .and_then(|tab| match tab {
                    WorkspaceTab::Dashboard(tab) => tab
                        .connection
                        .map(|connection| connection.profile_id)
                        .or(tab.profile_id),
                    WorkspaceTab::MongoDashboard(tab) => Some(tab.profile_id),
                    _ => None,
                })
                .and_then(|profile_id| app.profiles.iter().find(|profile| profile.id == profile_id))
                .is_some_and(|profile| profile.kind.is_redis_compatible());
            if redis_dashboard {
                redis_dashboard::render(frame, area, app, theme, state);
            } else {
                dashboard::render(frame, area, app, theme, state);
            }
        }
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
        footer::render(frame, layout.footer, app, theme, sequence, state);
    } else if is_relation || is_principal {
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
            if is_principal {
                principal::render(frame, area, app, theme, state);
            } else {
                relation::render(frame, area, app, theme, state);
            }
        }
        state.hit_regions.push(HitRegion {
            area: layout.footer,
            target: HitTarget::Help,
        });
        footer::render(frame, layout.footer, app, theme, sequence, state);
    } else {
        if let Some(area) = layout.explorer {
            state.hit_regions.push(HitRegion {
                area,
                target: HitTarget::Focus(Focus::Explorer),
            });
            render_explorer(frame, area, app, theme, state, icons);
        }
        if let Some(workspace) = empty_workspace {
            if let Some(area) = workspace_empty_area(layout) {
                render_empty_workspace(frame, area, workspace, theme);
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
            state.hit_regions.push(HitRegion {
                area: layout.footer,
                target: HitTarget::Help,
            });
            footer::render(frame, layout.footer, app, theme, sequence, state);
        }
    }

    if app.overlay.is_none() {
        for split in [
            PaneSplit::ExplorerWidth,
            PaneSplit::EditorHeight,
            PaneSplit::RedisKeysWidth,
        ] {
            if split == PaneSplit::EditorHeight && !editor_rendered {
                continue;
            }
            let region = if split == PaneSplit::RedisKeysWidth {
                redis_layout
                    .as_ref()
                    .and_then(layout::RedisBrowserLayout::resize_region)
            } else {
                layout.pane_resize_region(split)
            };
            if let Some(area) = region {
                state.hit_regions.push(HitRegion {
                    area,
                    target: HitTarget::PaneResize(split),
                });
            }
        }
    }

    if app.overlay.is_none()
        && let Some(split) = state.pane_resize_drag.borrow().map(|drag| drag.split)
        && let Some(area) = if split == PaneSplit::RedisKeysWidth {
            redis_layout
                .as_ref()
                .and_then(layout::RedisBrowserLayout::resize_region)
        } else {
            layout.pane_resize_region(split)
        }
    {
        let buffer = frame.buffer_mut();
        for x in area.x..area.right() {
            for y in area.y..area.bottom() {
                let cell = &mut buffer[(x, y)];
                cell.set_fg(theme.accent);
                cell.set_bg(theme.surface_raised);
                cell.set_style(Style::new().add_modifier(Modifier::BOLD));
            }
        }
    }

    if let Some(overlay) = &app.overlay {
        // Only the visible overlay may own the terminal cursor.
        state.cursor = None;
        // Omni is rendered above this overlay and owns the modal layer while
        // open, so the underlying overlay must not start or restart an effect.
        if app.omni.is_none() {
            state
                .animations
                .prepare_overlay(overlay_key(overlay), centered(area, 80, 20));
        }
        dim_background(frame, area, theme);
        render_overlay(frame, area, overlay, app, state, theme, icons);
        if app.omni.is_none() {
            state.animations.render_effect(frame, now);
        }
    } else {
        if app.omni.is_none() {
            state.animations.clear_overlay();
        }
        if state.animations.take_result_ready().is_some()
            && app.omni.is_none()
            && let Some(result_area) = state.result_area
        {
            state
                .animations
                .start_effect(animation::EffectKind::Result, result_area);
        }
        if app.omni.is_none() {
            state.animations.render_effect(frame, now);
        }
    }
    if app.omni.is_some() {
        state.cursor = None;
        // Omni content is rendered at its final foreground colors. A black
        // foreground fade makes lower rows unreadable on a dark theme.
        state.animations.clear_overlay();
        // The result transition belongs to the workspace underneath Omni.
        // Consume it while the modal is open so it cannot replay after Omni
        // closes and animate stale content over the newly visible workspace.
        state.animations.take_result_ready();
        dim_background(frame, area, theme);
        state.hit_regions.push(HitRegion {
            area,
            target: HitTarget::Omni,
        });
        omni::render(frame, app, state, theme, icons);
    }
    if let Some(sequence) = sequence {
        render_key_sequence_popup(frame, area, app, theme, sequence);
    }
    if !matches!(
        app.overlay,
        Some(Overlay::NotificationHistory(_) | Overlay::NotificationDetail(_))
    ) {
        notifications::render(frame, area, app, theme, state, icons);
    }
    if let Some(cursor) = state.cursor {
        frame.set_cursor_position(cursor.position);
    }
}

#[cfg(test)]
mod theme_render_tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use super::{UiState, icons::IconSet, render_with_state_using_icons_sequence_and_theme};
    use crate::{app::App, ui::theme::Theme};

    #[test]
    fn custom_theme_reaches_the_render_surface() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new(Vec::new());
        let mut state = UiState::new();
        let mut theme = Theme::deep_space();
        theme.background = ratatui::style::Color::Rgb(1, 2, 3);

        terminal
            .draw(|frame| {
                render_with_state_using_icons_sequence_and_theme(
                    frame,
                    &app,
                    &mut state,
                    IconSet::default(),
                    None,
                    theme,
                );
            })
            .unwrap();

        let buffer: &Buffer = terminal.backend().buffer();
        assert!(
            (0..buffer.area().height).any(|y| {
                (0..buffer.area().width)
                    .any(|x| buffer.cell((x, y)).unwrap().bg == theme.background)
            }),
            "custom theme background was not rendered"
        );
    }

    #[test]
    fn sequence_navigation_highlight_matches_the_visual_grid_column() {
        let mut app = App::new(Vec::new());
        app.focus = crate::model::workspace::Focus::Results;
        let shortcuts = crate::help::prefix_shortcuts_with_bindings(
            crate::help::shortcut_context(&app),
            crate::help::shortcut_capabilities(&app),
            crate::help::ShortcutPrefix::Leader,
            Some(&app.key_bindings),
        );
        let count = shortcuts.len();
        assert!(count > 1);
        for (width, height) in [(59, 36), (60, 36), (99, 36), (100, 36), (120, 36)] {
            let shortcuts = crate::help::prefix_shortcuts_with_bindings(
                crate::help::shortcut_context(&app),
                crate::help::shortcut_capabilities(&app),
                crate::help::ShortcutPrefix::Leader,
                Some(&app.key_bindings),
            );
            let layout = crate::input::sequence_navigation::Layout::new(width, shortcuts.len());
            let selected = layout.index((layout.columns() - 1).min(1), 0).unwrap();
            let sequence = crate::input::keymap::KeySequenceState {
                prefix: crate::help::ShortcutPrefix::Leader,
                display: "Space".into(),
                selected,
                candidates: Vec::new(),
            };
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut state = UiState::new();
            terminal
                .draw(|frame| {
                    super::render_with_state_using_icons_and_sequence(
                        frame,
                        &app,
                        &mut state,
                        IconSet::default(),
                        Some(&sequence),
                    )
                })
                .unwrap();
            let buffer: &Buffer = terminal.backend().buffer();
            let theme = Theme::default();
            let popup_height = (layout.rows() as u16 + 2).min(height.saturating_sub(4));
            let popup_y = height.saturating_sub(2).saturating_sub(popup_height);
            let inner_height = popup_height.saturating_sub(2) as usize;
            let row = selected % layout.rows();
            let scroll = if inner_height == 0 || row < inner_height {
                0
            } else {
                row + 1 - inner_height
            };
            let visible_row = row.saturating_sub(scroll) as u16;
            let y = popup_y + 1 + visible_row;
            let column = selected / layout.rows();
            let column_width = (width - 4) / layout.columns() as u16;
            let x = 2 + column as u16 * column_width;
            assert_eq!(
                buffer[(x, y)].bg,
                theme.selection,
                "width={width}, height={height}"
            );
            if column > 0 {
                assert_ne!(
                    buffer[(2, y)].bg,
                    theme.selection,
                    "width={width}, height={height}"
                );
            }
        }
    }
}

fn render_key_sequence_popup(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    sequence: &crate::input::keymap::KeySequenceState,
) {
    if !sequence.candidates.is_empty() {
        let rows = sequence.candidates.len();
        let height = (rows as u16 + 2).min(area.height.saturating_sub(2));
        let popup = Rect::new(
            area.x.saturating_add(1),
            area.bottom().saturating_sub(2).saturating_sub(height),
            area.width.saturating_sub(2),
            height,
        );
        let lines = sequence
            .candidates
            .iter()
            .enumerate()
            .map(|(index, (command, suffix))| {
                let label = match command.as_str() {
                    "focus-pane-left" => "focus left",
                    "focus-pane-down" => "focus down",
                    "focus-pane-up" => "focus up",
                    "focus-pane-right" => "focus right",
                    "toggle-pane-maximized" => "maximize/restore",
                    "reset-pane-sizes" => "reset sizes",
                    _ => command.as_str(),
                };
                let style = Style::new()
                    .fg(theme.text)
                    .bg(if index == sequence.selected {
                        theme.selection
                    } else {
                        theme.surface_raised
                    });
                Line::from(Span::styled(format!(" {suffix:<12} {label}"), style))
            })
            .collect::<Vec<_>>();
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(theme.accent))
                    .title(format!(
                        " {}  Arrows select  Enter run  Esc cancel ",
                        sequence.display
                    )),
            ),
            popup,
        );
        return;
    }
    let shortcuts = crate::help::prefix_shortcuts_with_bindings(
        crate::help::shortcut_context(app),
        crate::help::shortcut_capabilities(app),
        sequence.prefix,
        Some(&app.key_bindings),
    );
    if shortcuts.is_empty() || area.width < 16 || area.height < 5 {
        return;
    }

    let layout = crate::input::sequence_navigation::Layout::new(area.width, shortcuts.len());
    let columns = layout.columns();
    let rows = layout.rows();
    let height = (rows as u16)
        .saturating_add(2)
        .min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x.saturating_add(1),
        area.bottom().saturating_sub(2).saturating_sub(height),
        area.width.saturating_sub(2),
        height,
    );
    // Keep the transient sequence chooser from forwarding clicks to controls
    // underneath it.
    // The caller maps this target to no action.
    let inner_width = popup.width.saturating_sub(2);
    let column_width = usize::from(inner_width) / columns;
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut spans = Vec::new();
        for column in 0..columns {
            let Some(index) = layout.index(column, row) else {
                continue;
            };
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
                        " {}  Arrows select  Enter run  Esc/Ctrl-C cancel ",
                        sequence.display
                    )),
            ),
        popup,
    );
}

fn overlay_key(overlay: &Overlay) -> animation::OverlayKey {
    match overlay {
        Overlay::Help(_) => animation::OverlayKey::Help,
        Overlay::Update(_) => animation::OverlayKey::Update,
        Overlay::NotificationHistory(_) => animation::OverlayKey::NotificationHistory,
        Overlay::NotificationDetail(_) => animation::OverlayKey::NotificationDetail,
        Overlay::RecordView(_) => animation::OverlayKey::RecordView,
        Overlay::TextDetail(_) => animation::OverlayKey::TextDetail,
        Overlay::SqlHistory(_) => animation::OverlayKey::TextDetail,
        Overlay::ProfileManager => animation::OverlayKey::ProfileManager,
        Overlay::CatalogEditor => animation::OverlayKey::CatalogEditor,
        Overlay::RedisObjectEditor(_) => animation::OverlayKey::CatalogEditor,
        Overlay::RedisTableEditor(_) => animation::OverlayKey::CatalogEditor,
        Overlay::RedisTableDeleteConfirm(_) => animation::OverlayKey::DeleteConsole,
        Overlay::MongoManagement(_) => animation::OverlayKey::CatalogEditor,
        Overlay::MongoPrincipalMutation(_) | Overlay::MongoPrincipalConfirmation { .. } => {
            animation::OverlayKey::ExecutionConfirm
        }
        Overlay::RedisValueSaveConfirm { .. } => animation::OverlayKey::Message,
        Overlay::RedisUnsavedValueConfirm { .. } => animation::OverlayKey::Message,
        Overlay::ProfileAccess { .. } => animation::OverlayKey::ProfileAccess,
        Overlay::ProfileGroup(_) => animation::OverlayKey::ProfileGroup,
        Overlay::ExplorerAdd(_) => animation::OverlayKey::ExplorerAdd,
        Overlay::Message { .. } => animation::OverlayKey::Message,
        Overlay::WorkspaceSaveFailed { .. } => animation::OverlayKey::WorkspaceSaveFailed,
        Overlay::SubstituteConfirm { .. } => animation::OverlayKey::SubstituteConfirm,
        Overlay::ExecutionConfirm { .. } => animation::OverlayKey::ExecutionConfirm,
        Overlay::PrincipalMutationConfirm { .. } => animation::OverlayKey::ExecutionConfirm,
        Overlay::PrincipalMutationForm(_) => animation::OverlayKey::ExecutionConfirm,
        Overlay::ManualCancelConfirm { .. } => animation::OverlayKey::ManualCancelConfirm,
        Overlay::TransactionExitConfirm { .. } => animation::OverlayKey::TransactionExitConfirm,
        Overlay::RelationTransactionConfirm(_) => animation::OverlayKey::RelationTransactionConfirm,
        Overlay::ClearTransactionOutcome { .. } => animation::OverlayKey::ClearTransactionOutcome,
        Overlay::TransactionMenu { .. } => animation::OverlayKey::TransactionMenu,
        Overlay::TargetSelector { .. } => animation::OverlayKey::TargetSelector,
        Overlay::DatabaseSelector(_) => animation::OverlayKey::DatabaseSelector,
        Overlay::DeleteConsole { .. } => animation::OverlayKey::DeleteConsole,
        Overlay::RedisDeleteConfirm { .. } => animation::OverlayKey::DeleteConsole,
        Overlay::RedisDeletePreparing { .. } => animation::OverlayKey::DeleteConsole,
        Overlay::SqlEditorList(_) => animation::OverlayKey::SqlEditorList,
        Overlay::PageSizeSelector { .. } => animation::OverlayKey::PageSizeSelector,
        Overlay::RedisPreviewFormat { .. } => animation::OverlayKey::PageSizeSelector,
        Overlay::CatalogDropConfirm { .. } => animation::OverlayKey::CatalogDropConfirm,
        Overlay::PrincipalDropConfirm { .. } => animation::OverlayKey::CatalogDropConfirm,
        Overlay::CatalogEditorDestructiveConfirm { .. } => {
            animation::OverlayKey::CatalogEditorDestructiveConfirm
        }
        Overlay::CatalogEditorDiscardConfirm { .. } => {
            animation::OverlayKey::CatalogEditorDiscardConfirm
        }
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
    if let crate::model::update::UpdateState::Installing { request_id, .. } = app.update_state {
        observation
            .active_loads
            .insert(animation::LoadIdentity::Update { request_id });
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
                    .insert(animation::LoadIdentity::Relation(Box::new(request.clone())));
            }
            if let RelationLoad::Loading { request, .. } = &tab.ddl {
                observation
                    .active_loads
                    .insert(animation::LoadIdentity::Relation(Box::new(request.clone())));
            }
        }
        WorkspaceTab::Dashboard(_) | WorkspaceTab::MongoDashboard(_) => {}
        WorkspaceTab::RedisBrowser(_) => {}
        WorkspaceTab::MongoBrowser(_) => {}
        WorkspaceTab::MongoConsole(tab) => {
            if let crate::model::mongodb_console::MongoConsoleStatus::Running { generation } =
                tab.status
            {
                observation
                    .active_loads
                    .insert(animation::LoadIdentity::Query {
                        tab_id: tab.id,
                        generation,
                    });
            }
        }
        WorkspaceTab::PrincipalDdl(_) => {}
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
    let offset = text_input_horizontal_offset(area, prefix, input);
    let selection = input.selection_range();
    let mut visible = Vec::new();
    let mut cells = 0;
    for character in projection.text.chars() {
        let width = character.width().unwrap_or(0);
        let end = cells + width;
        if end > offset && cells < offset + available {
            let source = projection
                .source_to_display_cells
                .partition_point(|&boundary| boundary <= cells)
                .saturating_sub(1);
            let character_style = if selection
                .as_ref()
                .is_some_and(|range| range.contains(&source))
            {
                style.add_modifier(Modifier::REVERSED)
            } else {
                style
            };
            visible.push(Span::styled(character.to_string(), character_style));
        }
        cells = end;
        if cells >= offset + available {
            break;
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(
            std::iter::once(Span::styled(prefix, style))
                .chain(visible)
                .collect::<Vec<_>>(),
        ))
        .style(style),
        area,
    );
    let cursor_x = area
        .x
        .saturating_add(prefix_width as u16)
        .saturating_add(cursor_cells.saturating_sub(offset) as u16)
        .min(area.right().saturating_sub(1));
    let cursor = Position::new(cursor_x, area.y);
    state.cursor = Some(CursorSpec {
        position: cursor,
        style: CursorStyle::Bar,
    });
    Some(cursor)
}

pub(crate) fn register_input_selection_target(
    state: &mut UiState,
    target: text_selection::InputSelectionTarget,
    area: Rect,
    prefix: &str,
    value: &crate::model::text_input::TextInput,
    offset: usize,
) {
    state.input_selection_targets.push((
        target,
        text_selection::InputHitMap {
            area,
            source_to_display_cells: crate::security::project_editor_line(value.value())
                .source_to_display_cells,
            horizontal_offset: offset,
            prefix_width: prefix.width(),
            source_start: 0,
        },
    ));
}

pub(crate) fn text_input_horizontal_offset(
    area: Rect,
    prefix: &str,
    input: &crate::model::text_input::TextInput,
) -> usize {
    let projection = crate::security::project_editor_line(input.value());
    let available = usize::from(area.width).saturating_sub(prefix.width());
    let cursor_cells = projection
        .source_to_display_cells
        .get(input.cursor())
        .copied()
        .unwrap_or_else(|| projection.text.width());
    cursor_cells
        .saturating_sub(available.saturating_sub(1))
        .min(projection.text.width())
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
    let capacity = usize::from(area.height.saturating_sub(POPUP_BORDER_HEIGHT)).min(10);
    let selected = popup.selected.min(popup.candidates.len().saturating_sub(1));
    let start = selected.saturating_add(1).saturating_sub(capacity);
    let items = popup
        .candidates
        .iter()
        .enumerate()
        .skip(start)
        .take(capacity)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TabViewport {
    start: usize,
    end: usize,
    overflowed: bool,
}

struct RenderedTab {
    index: usize,
    id: Uuid,
    icon: String,
    icon_color: Option<Color>,
    title: String,
    marker: String,
    can_close: bool,
    width: u16,
}

fn tab_database_kind(app: &App, tab: &WorkspaceTab) -> Option<DatabaseKind> {
    let profile_id = match tab {
        WorkspaceTab::Sql(tab) => tab.execution_target.as_ref()?.profile_id,
        WorkspaceTab::Relation(tab) => tab.descriptor.key.profile_id,
        WorkspaceTab::Dashboard(tab) => tab
            .connection
            .map(|connection| connection.profile_id)
            .or(tab.profile_id)?,
        WorkspaceTab::RedisBrowser(tab) => {
            return app
                .profiles
                .iter()
                .find(|profile| profile.id == tab.target.profile_id)
                .map(|profile| profile.kind);
        }
        WorkspaceTab::MongoBrowser(_) => return Some(DatabaseKind::MongoDb),
        WorkspaceTab::MongoConsole(_) => return Some(DatabaseKind::MongoDb),
        WorkspaceTab::MongoDashboard(_) => return Some(DatabaseKind::MongoDb),
        WorkspaceTab::PrincipalDdl(tab) => tab.entry.id.profile_id,
    };

    app.profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| profile.kind)
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
            let title = if let Some(console) = tab.as_console() {
                let connection_name = console.execution_target.as_ref().map_or_else(
                    || "Unbound".to_owned(),
                    |target| {
                        app.profiles
                            .iter()
                            .find(|profile| profile.id == target.profile_id)
                            .map_or_else(
                                || "Invalid target".to_owned(),
                                |profile| {
                                    if target.is_valid(profile) {
                                        profile.name.clone()
                                    } else {
                                        format!("Invalid:{}", profile.name)
                                    }
                                },
                            )
                    },
                );
                format!("{} @{connection_name}", tab.title())
            } else {
                let connection_name = match tab {
                    WorkspaceTab::Relation(relation) => app
                        .profiles
                        .iter()
                        .find(|profile| profile.id == relation.descriptor.key.profile_id)
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Invalid target".to_owned()),
                    WorkspaceTab::Dashboard(dashboard) => dashboard
                        .connection
                        .or_else(|| {
                            dashboard.profile_id.map(|profile_id| {
                                crate::identity::ConnectionIdentity {
                                    profile_id,
                                    generation: 0,
                                }
                            })
                        })
                        .and_then(|connection| {
                            app.profiles
                                .iter()
                                .find(|profile| profile.id == connection.profile_id)
                        })
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Unbound".to_owned()),
                    WorkspaceTab::Sql(_) => unreachable!(),
                    WorkspaceTab::RedisBrowser(redis) => app
                        .profiles
                        .iter()
                        .find(|profile| profile.id == redis.target.profile_id)
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Invalid target".to_owned()),
                    WorkspaceTab::MongoDashboard(dashboard) => app
                        .profiles
                        .iter()
                        .find(|profile| profile.id == dashboard.profile_id)
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Invalid target".to_owned()),
                    WorkspaceTab::MongoBrowser(mongo) => app
                        .profiles
                        .iter()
                        .find(|profile| profile.id == mongo.profile_id)
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Invalid target".to_owned()),
                    WorkspaceTab::MongoConsole(mongo) => app
                        .profiles
                        .iter()
                        .find(|profile| profile.id == mongo.profile_id)
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Invalid target".to_owned()),
                    WorkspaceTab::PrincipalDdl(principal) => app
                        .profiles
                        .iter()
                        .find(|profile| profile.id == principal.entry.id.profile_id)
                        .map(|profile| profile.name.clone())
                        .unwrap_or_else(|| "Invalid target".to_owned()),
                };
                match tab {
                    WorkspaceTab::RedisBrowser(redis) => {
                        format!("db{}@{connection_name}", redis.target.database)
                    }
                    WorkspaceTab::PrincipalDdl(principal) => {
                        format!("{}@{connection_name}", principal.entry.name)
                    }
                    _ => format!("{} @{connection_name}", tab.title()),
                }
            };
            let title = sanitize_terminal_text(&title)
                .chars()
                .take(48)
                .collect::<String>();
            let icon = match tab {
                WorkspaceTab::Relation(tab) => icons.catalog(tab.descriptor.kind),
                _ => tab_database_kind(app, tab)
                    .map(|kind| icons.database(kind))
                    .unwrap_or_else(|| icons.catalog(CatalogKind::Database)),
            };
            let icon_color = tab_database_kind(app, tab).map(|kind| icons.database_color(kind));
            let label = format!(" {icon} {title} ");
            let can_close = index != 0 || tab.as_console().is_some();
            let marker = if can_close {
                format!("{} ", icons.close())
            } else {
                String::new()
            };
            let label_width = label.cell_width();
            let marker_width = marker.cell_width();
            let width = label_width + marker_width;
            RenderedTab {
                index,
                id: tab.id(),
                icon: icon.to_owned(),
                icon_color,
                title,
                marker,
                can_close,
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
        let marker_width = tab.marker.cell_width();
        let remaining_width = tabs_area.right().saturating_sub(x);
        let max_label_width = tab
            .width
            .saturating_sub(marker_width)
            .min(remaining_width.saturating_sub(marker_width));
        let prefix = format!(" {} ", tab.icon);
        let full_label_width = prefix
            .cell_width()
            .saturating_add(tab.title.cell_width())
            .saturating_add(1);
        let (prefix, title, suffix) = if full_label_width <= max_label_width {
            (prefix, tab.title.clone(), " ".to_owned())
        } else if prefix.cell_width() < max_label_width {
            let title_width = max_label_width
                .saturating_sub(prefix.cell_width())
                .saturating_sub(1)
                .max(1);
            (
                prefix,
                truncate_to_cell_width(&tab.title, title_width),
                String::new(),
            )
        } else {
            (
                String::new(),
                truncate_to_cell_width(&format!("{} {} ", tab.icon, tab.title), max_label_width),
                String::new(),
            )
        };
        let label_width = prefix
            .cell_width()
            .saturating_add(title.cell_width())
            .saturating_add(suffix.cell_width());
        if !prefix.is_empty() {
            let icon_style = tab.icon_color.map_or(style, |color| style.fg(color));
            spans.push(Span::styled(prefix, icon_style));
        }
        if !title.is_empty() {
            spans.push(Span::styled(title, style));
        }
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, style));
        }
        if !tab.marker.is_empty() {
            spans.push(Span::styled(tab.marker.clone(), style));
        }
        if x < tabs_area.right() {
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    x,
                    tabs_area.y,
                    label_width
                        .max(1)
                        .min(tabs_area.right().saturating_sub(x).max(1)),
                    1,
                ),
                target: HitTarget::Tab(tab.index),
            });
        }
        let marker_x = x.saturating_add(label_width);
        if tab.can_close && marker_width > 0 && marker_x < tabs_area.right() {
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    marker_x,
                    tabs_area.y,
                    marker_width.min(tabs_area.right().saturating_sub(marker_x)),
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
        render_explorer_search(frame, inner, app, search, theme, state, icons);
        return;
    }
    let viewport = app.explorer.viewport(inner.height as usize);
    if viewport.pinned.is_empty() && viewport.rows.is_empty() {
        let message = if app.connection.status == ConnectionStatus::Connecting {
            "Synchronizing catalog..."
        } else if app.connection.status == ConnectionStatus::Connected {
            "No visible objects"
        } else {
            "No active connection\n\nPress Enter for a new connection or a for the add menu"
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
        register_explorer_row_hits(
            state,
            Rect::new(inner.x, inner.y.saturating_add(row as u16), inner.width, 1),
            visible,
        );
    }
    for (row, visible) in displayed.iter().enumerate() {
        register_explorer_row_hits(
            state,
            Rect::new(
                inner.x,
                inner
                    .y
                    .saturating_add((pinned_rows + indicator_rows + row) as u16),
                inner.width,
                1,
            ),
            visible,
        );
    }
    let mut items = viewport
        .pinned
        .iter()
        .map(|visible| explorer_list_item(visible, app, theme, icons, None))
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
            .map(|visible| explorer_list_item(visible, app, theme, icons, None)),
    );
    frame.render_widget(
        List::new(items).style(Style::new().bg(theme.surface)),
        inner,
    );
    render_explorer_scrollbar(
        frame,
        inner,
        viewport.total_rows,
        app.explorer.normalized.scroll,
        theme,
        state,
    );
}

fn render_explorer_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    rows: usize,
    offset: usize,
    theme: Theme,
    state: &mut UiState,
) {
    let visible = area.height as usize;
    let track = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
    let Some(geometry) = crate::ui::scrollbar::geometry(track, visible, rows, offset) else {
        return;
    };
    let thumb_y = geometry.thumb_area().y;
    let before = geometry.thumb_start;
    let after = geometry
        .rail
        .height
        .saturating_sub(before)
        .saturating_sub(geometry.thumb_length);
    let mut lines = Vec::with_capacity(track.height as usize);
    lines.push(Line::from(Span::styled("▲", Style::new().fg(theme.muted))));
    lines.extend((0..before).map(|_| Line::from(Span::styled("│", Style::new().fg(theme.muted)))));
    lines.extend(
        (0..geometry.thumb_length)
            .map(|_| Line::from(Span::styled("┃", Style::new().fg(theme.accent)))),
    );
    lines.extend((0..after).map(|_| Line::from(Span::styled("│", Style::new().fg(theme.muted)))));
    lines.push(Line::from(Span::styled("▼", Style::new().fg(theme.muted))));
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme.surface)),
        track,
    );
    state.hit_regions.push(HitRegion {
        area: Rect::new(track.x, track.y.saturating_add(1), 1, before),
        target: HitTarget::ExplorerScrollbarPage {
            offset: offset.saturating_sub(visible),
        },
    });
    state.hit_regions.push(HitRegion {
        area: geometry.thumb_area(),
        target: HitTarget::ExplorerScrollbarThumb {
            track_start: geometry.rail.y,
            track_length: geometry.rail.height,
            thumb_start: thumb_y,
            thumb_length: geometry.thumb_length,
            max_offset: geometry.max_offset,
        },
    });
    state.hit_regions.push(HitRegion {
        area: Rect::new(
            track.x,
            thumb_y.saturating_add(geometry.thumb_length),
            1,
            after,
        ),
        target: HitTarget::ExplorerScrollbarPage {
            offset: offset.saturating_add(visible).min(geometry.max_offset),
        },
    });
}

fn register_explorer_row_hits(state: &mut UiState, area: Rect, visible: &VisibleCatalogNode) {
    if area.is_empty() {
        return;
    }
    state.hit_regions.push(HitRegion {
        area,
        target: HitTarget::ExplorerRow(visible.id.clone()),
    });
    if !visible.expandable {
        return;
    }
    let offset = visible.depth.saturating_mul(2) as u16;
    if offset >= area.width {
        return;
    }
    state.hit_regions.push(HitRegion {
        area: Rect::new(
            area.x.saturating_add(offset),
            area.y,
            (area.width - offset).min(2),
            1,
        ),
        target: HitTarget::ExplorerToggle(visible.id.clone()),
    });
}

fn explorer_list_item(
    visible: &VisibleCatalogNode,
    app: &App,
    theme: Theme,
    icons: icons::IconSet,
    query: Option<&str>,
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
        crate::model::explorer::ExplorerNodeId::RedisDatabase { .. } => {
            icons.catalog(CatalogKind::Schema)
        }
        crate::model::explorer::ExplorerNodeId::Group { group, .. } => {
            icons.group(*group, expanded)
        }
        crate::model::explorer::ExplorerNodeId::ProjectionGroup(_) => {
            icons.group(crate::db::catalog::ObjectGroup::Tables, expanded)
        }
        crate::model::explorer::ExplorerNodeId::PrincipalGroup { .. } => {
            icons.principal(crate::db::principal::PrincipalDisplayKind::Group)
        }
        crate::model::explorer::ExplorerNodeId::Principal { .. } => {
            visible.principal.map_or("·", |kind| icons.principal(kind))
        }
        _ => visible.kind.map_or("·", |kind| icons.catalog(kind)),
    };
    let label = sanitize_terminal_text(&visible.label);
    let selected = app.explorer.selected_id() == Some(&visible.id);
    let label_style = if visible.unavailable_reason.is_some() && !selected {
        Style::new()
            .fg(theme.muted)
            .bg(theme.surface)
            .add_modifier(Modifier::DIM)
    } else if selected {
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
    let secondary_style = Style::new().fg(theme.muted).bg(if selected {
        theme.selection
    } else {
        theme.surface
    });
    if let Some(kind) = visible.profile_kind {
        spans.push(Span::styled(
            format!(
                "{} ",
                if visible.unavailable_reason.is_some() {
                    "?"
                } else {
                    icons.database(kind)
                }
            ),
            Style::new()
                .fg(if visible.unavailable_reason.is_some() {
                    theme.muted
                } else {
                    icons.database_color(kind)
                })
                .bg(if selected {
                    theme.selection
                } else {
                    theme.surface
                }),
        ));
    } else if !is_others {
        let icon_color = if let Some(principal) = visible.principal {
            principal_node_color(principal, theme)
        } else if let crate::model::explorer::ExplorerNodeId::RedisDatabase { profile_id, .. } =
            &visible.id
        {
            app.profiles
                .iter()
                .find(|profile| profile.id == *profile_id)
                .map_or(theme.muted, |profile| icons.database_color(profile.kind))
        } else {
            visible
                .kind
                .map_or(theme.muted, |kind| kind_color(kind, theme))
        };
        spans.push(Span::styled(
            format!("{} ", icon),
            Style::new().fg(icon_color).bg(if selected {
                theme.selection
            } else {
                theme.surface
            }),
        ));
    }
    if let Some(reason) = visible.unavailable_reason.as_deref() {
        spans.push(Span::styled(
            format!("  UNSUPPORTED: {}", sanitize_terminal_text(reason)),
            secondary_style,
        ));
    }
    if let Some(query) = query.filter(|query| !query.trim().is_empty()) {
        spans.extend(match_spans(
            label,
            query,
            label_style,
            label_style.fg(theme.action).add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(label, label_style));
    }
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
        state.cursor = Some(CursorSpec {
            position: Position::new(
                area.x
                    .saturating_add(explorer_search_cursor_column(&query, area.width)),
                area.y,
            ),
            style: CursorStyle::Bar,
        });
    }
    let tree_area = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(2),
    );
    let viewport = app.explorer.viewport(tree_area.height as usize);
    let pinned_rows = viewport.pinned.len();
    let indicator_rows = usize::from(viewport.show_ancestor_indicator);
    let rows = viewport.pinned.iter().chain(viewport.rows.iter());
    for (row, visible) in rows.clone().enumerate() {
        // Find rows use the same stable IDs as normal Explorer rows.
        register_explorer_row_hits(
            state,
            Rect::new(
                tree_area.x,
                tree_area.y.saturating_add(
                    (row + usize::from(row >= pinned_rows) * indicator_rows) as u16,
                ),
                tree_area.width,
                1,
            ),
            visible,
        );
    }
    let mut items = viewport
        .pinned
        .iter()
        .map(|visible| explorer_list_item(visible, app, theme, icons, Some(find.query.value())))
        .collect::<Vec<_>>();
    if viewport.show_ancestor_indicator {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  ⋮ {} ancestors", viewport.hidden_ancestor_count),
            Style::new().fg(theme.muted).bg(theme.surface),
        ))));
    }
    items.extend(
        rows.skip(pinned_rows)
            .map(|visible| explorer_list_item(visible, app, theme, icons, Some(find.query.value())))
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        List::new(items).style(Style::new().bg(theme.surface)),
        tree_area,
    );
    render_explorer_scrollbar(
        frame,
        tree_area,
        viewport.total_rows,
        app.explorer.normalized.scroll,
        theme,
        state,
    );
    if area.height > 1 {
        let status = if find.phase == ExplorerSearchPhase::Editing {
            "Enter confirm  Esc cancel"
        } else {
            "n/N next/prev  Esc close"
        };
        frame.render_widget(
            Paragraph::new(status).style(Style::new().fg(theme.muted).bg(theme.surface)),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
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
    state: &mut UiState,
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
        state.cursor = Some(CursorSpec {
            position: Position::new(
                area.x
                    .saturating_add(explorer_search_cursor_column(&input, area.width)),
                area.y,
            ),
            style: CursorStyle::Bar,
        });
    }

    let result_height = area.height.saturating_sub(2) as usize;
    let row_count = app.explorer.search_row_count();
    let start = search
        .scroll
        .max(
            search
                .selected
                .saturating_sub(result_height.saturating_sub(1)),
        )
        .min(row_count.saturating_sub(1));
    let items = app
        .explorer
        .search_rows_range(start, result_height)
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            let index = start + offset;
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
                crate::model::explorer::ExplorerNodeId::ProjectionGroup(_) => {
                    icons.group(crate::db::catalog::ObjectGroup::Tables, expanded)
                }
                crate::model::explorer::ExplorerNodeId::PrincipalGroup { .. } => {
                    icons.principal(crate::db::principal::PrincipalDisplayKind::Group)
                }
                crate::model::explorer::ExplorerNodeId::Principal { .. } => {
                    row.principal.map_or("·", |kind| icons.principal(kind))
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
                    Style::new().fg(icons.database_color(kind)).bg(background),
                ));
            } else {
                let icon_color = if let Some(principal) = row.principal {
                    principal_node_color(principal, theme)
                } else {
                    row.kind.map_or(theme.muted, |kind| kind_color(kind, theme))
                };
                spans.push(Span::styled(
                    format!("{} ", icon),
                    Style::new().fg(icon_color).bg(background),
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
        render_explorer_search_scrollbar(frame, area, search, theme, state, result_height);
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

fn render_explorer_search_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    search: &crate::model::workspace::ExplorerSearchState,
    theme: Theme,
    state: &mut UiState,
    visible: usize,
) {
    let rows = search.frontend_rows.len();
    let track = Rect::new(
        area.right().saturating_sub(1),
        area.y.saturating_add(1),
        1,
        visible.min(u16::MAX as usize) as u16,
    );
    let Some(geometry) = crate::ui::scrollbar::geometry(track, visible, rows, search.scroll) else {
        return;
    };
    render_explorer_scrollbar_parts(frame, track, geometry, theme, state, search.scroll, visible);
}

fn render_explorer_scrollbar_parts(
    frame: &mut Frame<'_>,
    track: Rect,
    geometry: crate::ui::scrollbar::ScrollbarGeometry,
    theme: Theme,
    state: &mut UiState,
    offset: usize,
    page: usize,
) {
    let thumb_y = geometry.thumb_area().y;
    let before = geometry.thumb_start;
    let after = geometry
        .rail
        .height
        .saturating_sub(before)
        .saturating_sub(geometry.thumb_length);
    let mut lines = Vec::with_capacity(track.height as usize);
    lines.push(Line::from(Span::styled("▲", Style::new().fg(theme.muted))));
    lines.extend((0..before).map(|_| Line::from(Span::styled("│", Style::new().fg(theme.muted)))));
    lines.extend(
        (0..geometry.thumb_length)
            .map(|_| Line::from(Span::styled("┃", Style::new().fg(theme.accent)))),
    );
    lines.extend((0..after).map(|_| Line::from(Span::styled("│", Style::new().fg(theme.muted)))));
    lines.push(Line::from(Span::styled("▼", Style::new().fg(theme.muted))));
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme.surface)),
        track,
    );
    state.hit_regions.push(HitRegion {
        area: Rect::new(track.x, track.y.saturating_add(1), 1, before),
        target: HitTarget::ExplorerScrollbarPage {
            offset: offset.saturating_sub(page),
        },
    });
    state.hit_regions.push(HitRegion {
        area: geometry.thumb_area(),
        target: HitTarget::ExplorerScrollbarThumb {
            track_start: geometry.rail.y,
            track_length: geometry.rail.height,
            thumb_start: thumb_y,
            thumb_length: geometry.thumb_length,
            max_offset: geometry.max_offset,
        },
    });
    state.hit_regions.push(HitRegion {
        area: Rect::new(
            track.x,
            thumb_y.saturating_add(geometry.thumb_length),
            1,
            after,
        ),
        target: HitTarget::ExplorerScrollbarPage {
            offset: offset.saturating_add(page).min(geometry.max_offset),
        },
    });
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
    state
        .text_selection_targets
        .push(text_selection::TextSelectionTarget {
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
                    horizontal_offset: snapshot.horizontal_offset + line.wrap_offset,
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
    let text_width = inner.width.saturating_sub(gutter as u16);
    let preliminary_viewport = EditorViewport {
        width: text_width as usize,
        height: inner.height as usize,
    };
    let Ok(preliminary_snapshot) = app.active_editor_render_snapshot(preliminary_viewport) else {
        return None;
    };
    let has_horizontal_scrollbar =
        preliminary_snapshot.max_line_width > preliminary_viewport.width.max(1);
    let footer_rows =
        usize::from(preliminary_snapshot.prompt.is_some()) + usize::from(has_horizontal_scrollbar);
    let viewport = EditorViewport {
        width: preliminary_viewport.width,
        height: (inner.height as usize).saturating_sub(footer_rows).max(1),
    };
    state.editor_viewport = Some(viewport);
    let Ok(snapshot) = (if viewport == preliminary_viewport {
        Ok(preliminary_snapshot)
    } else {
        app.active_editor_render_snapshot(viewport)
    }) else {
        return None;
    };
    let text_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        viewport.height.min(u16::MAX as usize) as u16,
    );
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
            viewport: text_area,
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
                        && app.sessions.get(target).is_some_and(|session| {
                            session.status == crate::model::session::SessionStatus::Connected
                        })
                    {
                        format!("{target_label} READY")
                    } else if app.connection.pending_target.as_ref() == Some(target) {
                        format!("{target_label} CONNECTING")
                    } else if app
                        .active_console_opt()
                        .and_then(|tab| tab.target_error.as_ref())
                        .is_some()
                    {
                        format!("{target_label} TARGET ERROR")
                    } else if app.connection.active_identity().is_some() {
                        format!("{target_label} TARGET INACTIVE")
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
    let diagnostic_count = snapshot.semantic_diagnostics.len();
    let transaction_segment = format!(" {transaction} ");
    let diagnostic_segment = if diagnostic_count > 0 {
        format!(" DIAGNOSTICS {diagnostic_count} ")
    } else {
        String::new()
    };
    let query_segment_width =
        query_status.map_or(0, |(label, _)| format!(" {label} ").cell_width());
    let available_width = area.width.saturating_sub(2);
    let required_context_width = query_segment_width
        .saturating_add(diagnostic_segment.cell_width())
        .saturating_add(transaction_segment.cell_width());
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
    if !diagnostic_segment.is_empty() {
        context.push(Span::styled(
            diagnostic_segment,
            Style::new().fg(theme.error).add_modifier(Modifier::BOLD),
        ));
    }
    if show_target {
        context.push(Span::raw(&target_segment));
    }
    context.push(Span::raw(&transaction_segment));
    let context_right = area.right().saturating_sub(1);
    let transaction_x = context_right.saturating_sub(transaction_segment.cell_width());
    state.hit_regions.push(HitRegion {
        area: Rect::new(transaction_x, area.y, transaction_segment.cell_width(), 1),
        target: HitTarget::EditorTransactionMenu,
    });
    if show_target {
        let target_x = transaction_x.saturating_sub(target_segment.cell_width());
        state.hit_regions.push(HitRegion {
            area: Rect::new(target_x, area.y, target_segment.cell_width(), 1),
            target: HitTarget::EditorExecutionTarget,
        });
    }
    let block = base_block
        .title_top(Line::raw(left_title).left_aligned())
        .title_top(Line::from(context).right_aligned());
    let cursor_style = if snapshot.prompt.is_some() {
        CursorStyle::Bar
    } else {
        match snapshot.mode {
            EditorMode::Insert => CursorStyle::Bar,
            EditorMode::Replace => CursorStyle::Underline,
            _ => CursorStyle::Block,
        }
    };
    frame.render_widget(block, area);
    for (row, line) in snapshot.lines.iter().take(viewport.height).enumerate() {
        let y = inner.y.saturating_add(row as u16);
        let line_selected = snapshot.selections.iter().any(|selection| {
            selection.shape == crate::model::editor::EditorSelectionShape::Line
                && line.line >= selection.start.line.min(selection.end.line)
                && line.line <= selection.start.line.max(selection.end.line)
        });
        let line_style = Style::new().fg(theme.border).bg(theme.surface);
        let line_number_style = if line.line == snapshot.cursor.line
            && app.focus == Focus::Editor
            && app.overlay.is_none()
        {
            Style::new()
                .fg(theme.accent)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD)
        } else if line_has_diagnostic(line, &snapshot.semantic_diagnostics) {
            Style::new()
                .fg(theme.error)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD)
        } else {
            line_style
        };
        let content_background = if line_selected {
            theme.selection
        } else {
            theme.surface
        };
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
                Span::styled(
                    format!(" {:>number_width$} ", line.line + 1),
                    line_number_style,
                ),
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
            (snapshot.selections.is_empty()
                && matches!(
                    snapshot.mode,
                    EditorMode::Normal | EditorMode::Insert | EditorMode::Replace
                )
                && app.focus == Focus::Editor
                && app.overlay.is_none())
            .then_some(line.statement_background_cells)
            .flatten(),
            &mouse_selection_cells(
                state,
                app.active_console_opt()
                    .map(|tab| tab.id)
                    .unwrap_or_default(),
                &snapshot,
                line,
            ),
            None,
        );
        let content = if line.selection_newline {
            let mut content = content;
            content.push(Span::styled(
                " ",
                Style::new().fg(theme.text).bg(theme.selection),
            ));
            content
        } else {
            content
        };
        frame.render_widget(
            Paragraph::new(Line::from(content))
                .style(Style::new().bg(content_background))
                .scroll((0, snapshot.horizontal_offset.min(u16::MAX as usize) as u16)),
            Rect::new(
                inner.x.saturating_add(gutter as u16),
                y,
                viewport.width as u16,
                1,
            ),
        );
    }
    let horizontal_track = has_horizontal_scrollbar.then(|| {
        Rect::new(
            inner.x.saturating_add(1),
            inner.bottom().saturating_sub(1),
            inner.width.saturating_sub(2),
            1,
        )
    });
    render_editor_scrollbars(
        frame,
        area,
        session_id_for_editor(app),
        &snapshot,
        theme,
        state,
        horizontal_track,
    );

    if app.overlay.is_none()
        && app.focus == Focus::Editor
        && let Some(prompt) = snapshot.prompt.as_ref()
    {
        let prompt_text = match prompt.error.as_deref() {
            Some(error) => format!("{}{}  [{}]", prompt.prefix, prompt.text, error),
            None => format!("{}{}", prompt.prefix, prompt.text),
        };
        let prompt_y = inner
            .bottom()
            .saturating_sub(u16::from(has_horizontal_scrollbar))
            .saturating_sub(1);
        let prompt_area = Rect::new(inner.x, prompt_y, inner.width, 1);
        frame.render_widget(
            Paragraph::new(prompt_text).style(Style::new().fg(theme.accent).bg(theme.surface)),
            prompt_area,
        );
        let cursor_x = inner
            .x
            .saturating_add(prompt.prefix.chars().count() as u16)
            .saturating_add(prompt.cursor as u16)
            .min(prompt_area.right().saturating_sub(1));
        state.cursor = Some(CursorSpec {
            position: Position::new(cursor_x, prompt_area.y),
            style: cursor_style,
        });
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
            state.cursor = Some(CursorSpec {
                position: Position::new(x, y),
                style: cursor_style,
            });
        }
    }
    completion_anchor
}

fn session_id_for_editor(app: &App) -> Option<Uuid> {
    app.active_console_opt().map(|tab| tab.id)
}

pub(crate) fn render_editor_scrollbars(
    frame: &mut Frame<'_>,
    area: Rect,
    session_id: Option<Uuid>,
    snapshot: &crate::model::editor::EditorRenderSnapshot,
    theme: Theme,
    state: &mut UiState,
    horizontal_track: Option<Rect>,
) {
    let Some(session_id) = session_id else { return };
    let vertical_max = snapshot
        .total_lines
        .saturating_sub(snapshot.viewport.height.max(1));
    let horizontal_max = snapshot
        .max_line_width
        .saturating_sub(snapshot.viewport.width.max(1));

    if vertical_max > 0 && area.height > 3 {
        let track = Rect::new(
            area.right().saturating_sub(1),
            area.y.saturating_add(1),
            1,
            area.height.saturating_sub(2),
        );
        let rail = crate::ui::scrollbar::geometry(
            track,
            snapshot.viewport.height,
            snapshot.total_lines,
            snapshot.first_line,
        );
        if let Some(geometry) = rail {
            let thumb_area = geometry.thumb_area();
            crate::ui::scrollbar::render_vertical(frame, track, geometry, theme);
            let after = geometry
                .rail
                .height
                .saturating_sub(geometry.thumb_start)
                .saturating_sub(geometry.thumb_length);
            let offset = geometry.thumb_start;
            state.hit_regions.push(HitRegion {
                area: Rect::new(track.x, track.y.saturating_add(1), 1, offset),
                target: HitTarget::EditorScrollbarPage {
                    session_id,
                    rows: -(snapshot.viewport.height.max(1) as isize),
                    columns: 0,
                },
            });
            state.hit_regions.push(HitRegion {
                area: thumb_area,
                target: HitTarget::EditorScrollbarThumb {
                    session_id,
                    vertical: true,
                    track_start: geometry.rail.y,
                    track_length: geometry.rail.height,
                    thumb_start: thumb_area.y,
                    thumb_length: geometry.thumb_length,
                    offset: snapshot.first_line,
                    max_offset: geometry.max_offset,
                },
            });
            state.hit_regions.push(HitRegion {
                area: Rect::new(
                    track.x,
                    track.y.saturating_add(1 + offset + geometry.thumb_length),
                    1,
                    after,
                ),
                target: HitTarget::EditorScrollbarPage {
                    session_id,
                    rows: snapshot.viewport.height.max(1) as isize,
                    columns: 0,
                },
            });
        }
    }

    if horizontal_max > 0
        && let Some(track) = horizontal_track.filter(|track| track.width > 3)
    {
        let Some(geometry) = crate::ui::scrollbar::geometry(
            track,
            snapshot.viewport.width,
            snapshot.max_line_width,
            snapshot.horizontal_offset,
        ) else {
            return;
        };
        let thumb = geometry.thumb_length;
        let offset = geometry.thumb_start;
        crate::ui::scrollbar::render_horizontal(frame, track, geometry, theme);
        state.hit_regions.push(HitRegion {
            area: Rect::new(track.x, track.y, offset, 1),
            target: HitTarget::EditorScrollbarPage {
                session_id,
                rows: 0,
                columns: -(snapshot.viewport.width.max(1) as isize),
            },
        });
        state.hit_regions.push(HitRegion {
            area: Rect::new(track.x.saturating_add(offset), track.y, thumb, 1),
            target: HitTarget::EditorScrollbarThumb {
                session_id,
                vertical: false,
                track_start: geometry.rail.x,
                track_length: geometry.rail.width,
                thumb_start: geometry.thumb_area().x,
                thumb_length: thumb,
                offset: snapshot.horizontal_offset,
                max_offset: horizontal_max,
            },
        });
        state.hit_regions.push(HitRegion {
            area: Rect::new(
                track.x.saturating_add(offset + thumb),
                track.y,
                geometry.rail.width.saturating_sub(offset + thumb),
                1,
            ),
            target: HitTarget::EditorScrollbarPage {
                session_id,
                rows: 0,
                columns: snapshot.viewport.width.max(1) as isize,
            },
        });
    }
}

pub(crate) fn editor_syntax_color(kind: EditorHighlightKind) -> theme::SyntaxColor {
    match kind {
        EditorHighlightKind::Keyword => theme::SyntaxColor::Keyword,
        EditorHighlightKind::Identifier => theme::SyntaxColor::Identifier,
        EditorHighlightKind::Relation => theme::SyntaxColor::Relation,
        EditorHighlightKind::RelationAlias => theme::SyntaxColor::RelationAlias,
        EditorHighlightKind::Column => theme::SyntaxColor::Column,
        EditorHighlightKind::Type => theme::SyntaxColor::Type,
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
    mouse_selection_cells: &[(usize, usize)],
    output_style: Option<(crate::model::tab::OutputKind, usize)>,
) -> Vec<Span<'static>> {
    let selected = snapshot
        .selection_cells
        .iter()
        .filter(|(selected_line, _, _)| *selected_line == line.line)
        .map(|(_, start, end)| (*start, *end))
        .collect::<Vec<_>>();
    let mut display_cell = 0usize;
    let mut source_span_index = 0usize;
    let mut result: Vec<Span<'static>> = Vec::new();
    let source_boundaries = &line.source_byte_boundaries;
    for boundary in 0..source_boundaries.len().saturating_sub(1) {
        let source_start = line.source_start + source_boundaries[boundary];
        let source_end = line.source_start + source_boundaries[boundary + 1];
        while source_span_index + 1 < line.spans.len()
            && line.spans[source_span_index].source_end <= source_start
        {
            source_span_index += 1;
        }
        let kind = if syntax {
            line.spans
                .get(source_span_index)
                .filter(|span| span.source_start <= source_start && span.source_end >= source_end)
                .map_or(EditorHighlightKind::Plain, |span| span.kind)
        } else {
            EditorHighlightKind::Plain
        };
        let has_semantic_error =
            diagnostic_covers_source(&snapshot.semantic_diagnostics, source_start, source_end);
        let default_foreground = if has_semantic_error {
            theme.error
        } else if syntax {
            theme.syntax_color(editor_syntax_color(kind))
        } else {
            theme.text
        };
        let display_start = line.source_to_display_bytes[boundary];
        let display_end = line.source_to_display_bytes[boundary + 1];
        let projected = line
            .display_text
            .get(display_start..display_end)
            .unwrap_or_default();
        for character in projected.chars() {
            let width = character.width().unwrap_or(0);
            let source_offset = source_boundaries[boundary];
            let foreground = if let Some((kind, timestamp_end)) = output_style {
                if source_offset < timestamp_end {
                    theme.muted
                } else if kind == crate::model::tab::OutputKind::Error {
                    theme.error
                } else {
                    default_foreground
                }
            } else {
                default_foreground
            };
            let highlighted = selected.iter().any(|(start, end)| {
                display_cell < *end && display_cell.saturating_add(width) > *start
            });
            let mouse_highlighted = mouse_selection_cells.iter().any(|(start, end)| {
                display_cell < *end && display_cell.saturating_add(width) > *start
            });
            let style = Style::new()
                .fg(foreground)
                .bg(if mouse_highlighted {
                    theme.mouse_selection
                } else if highlighted {
                    theme.selection
                } else if statement_background_cells.is_some_and(|(start, end)| {
                    display_cell < end && display_cell.saturating_add(width) > start
                }) {
                    theme.surface_raised
                } else {
                    theme.surface
                })
                .add_modifier(if has_semantic_error {
                    Modifier::UNDERLINED
                } else {
                    Modifier::empty()
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

fn diagnostic_covers_source(
    diagnostics: &[crate::sql::SqlDiagnostic],
    source_start: usize,
    source_end: usize,
) -> bool {
    diagnostics.iter().any(|diagnostic| {
        diagnostic.range.start < source_end && diagnostic.range.end > source_start
    })
}

fn line_has_diagnostic(
    line: &crate::model::editor::EditorRenderLine,
    diagnostics: &[crate::sql::SqlDiagnostic],
) -> bool {
    diagnostics.iter().any(|diagnostic| {
        (diagnostic.range.start < line.source_end && diagnostic.range.end > line.source_start)
            || (diagnostic.range.start == diagnostic.range.end
                && diagnostic.range.start >= line.source_start
                && diagnostic.range.start <= line.source_end)
    })
}

pub(crate) fn mouse_selection_cells(
    state: &UiState,
    session_id: Uuid,
    snapshot: &crate::model::editor::EditorRenderSnapshot,
    line: &crate::model::editor::EditorRenderLine,
) -> Vec<(usize, usize)> {
    let Some(gesture) = state.text_gesture.borrow().as_ref().copied() else {
        return Vec::new();
    };
    if !gesture.has_dragged
        || gesture.session_id != session_id
        || gesture.revision != snapshot.revision
    {
        return Vec::new();
    }
    let first = (gesture.start.line, gesture.start.column);
    let last = (gesture.end.line, gesture.end.column);
    let ((first_line, first_column), (last_line, last_column)) = if first <= last {
        (first, last)
    } else {
        (last, first)
    };
    if line.line < first_line || line.line > last_line {
        return Vec::new();
    }
    let last_source_column = line.source_to_display_cells.len().saturating_sub(1);
    let start = if line.line == first_line {
        first_column.min(last_source_column)
    } else {
        0
    };
    let end = if line.line == last_line {
        last_column.saturating_add(1).min(last_source_column)
    } else {
        last_source_column
    };
    if end <= start {
        return Vec::new();
    }
    let start_cell = *line.source_to_display_cells.get(start).unwrap_or(&0);
    let end_cell = *line
        .source_to_display_cells
        .get(end)
        .unwrap_or_else(|| line.source_to_display_cells.last().unwrap_or(&0));
    (end_cell > start_cell)
        .then_some((start_cell, end_cell))
        .into_iter()
        .collect()
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
        let width = text.cell_width();
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
    let mut query = tab.query.clone();
    let derived_error = tab
        .derived
        .as_ref()
        .and_then(|derived| derived.error.as_ref());
    if derived_error.is_some() && query.error.as_ref() == derived_error {
        query.error = None;
    }
    let block = panel_block(" RESULT SET ", app.focus == Focus::Results, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let query_height = query_bar::height(&query, inner.width, state.activity_icons);
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
        &query,
        theme,
        state,
        state.activity_icons,
        app.sql_dialect(),
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
    let (displayed_outcome, pagination) = tab
        .displayed_result_page()
        .map_or((None, tab.pagination), |(outcome, pagination)| {
            (Some(outcome), pagination)
        });
    let result = displayed_outcome.and_then(|outcome| outcome.result_sets.last());
    let status = if loading_identity.is_none() {
        tab.derived
            .as_ref()
            .and_then(|derived| derived.error.as_deref())
            .map(|_| "Query failed - showing previous result")
            .or(match tab.query_status {
                QueryStatus::Cancelled => Some("Query cancelled - showing previous result"),
                QueryStatus::Failed => Some("Query failed - showing previous result"),
                QueryStatus::Idle | QueryStatus::Running => None,
            })
    } else {
        None
    };
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
                    cancellable: tab.query_status == QueryStatus::Running,
                    style: Style::new().fg(theme.action).bg(theme.surface_raised),
                },
                body[0],
            );
            render_result_table(
                frame,
                body[1],
                tab.id,
                result,
                tab.derived
                    .as_ref()
                    .map_or(tab.generation, |derived| derived.generation),
                tab.grid.clone(),
                theme,
                Block::default().style(Style::new().bg(theme.surface)),
                state,
                tab.query.submitted.order_by_clause.as_deref().unwrap_or(""),
                tab.last_execution
                    .as_ref()
                    .map_or(crate::sql::SqlDialect::Sqlite, |last| last.draft.dialect),
                tab.query_status != QueryStatus::Running
                    && !tab.derived.as_ref().is_some_and(|derived| derived.running),
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
                    cancellable: tab.query_status == QueryStatus::Running,
                    theme,
                    block: Block::default().style(Style::new().bg(theme.surface)),
                },
                result_area,
            );
        }
        let _ = identity;
    } else if let Some(result) = result {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(u16::from(status.is_some())),
                Constraint::Min(1),
            ])
            .split(result_area);
        if let Some(status) = status {
            frame.render_widget(
                Paragraph::new(status)
                    .style(Style::new().fg(theme.warning).bg(theme.surface_raised)),
                body[0],
            );
        }
        render_result_table(
            frame,
            body[1],
            tab.id,
            result,
            tab.derived
                .as_ref()
                .map_or(tab.generation, |derived| derived.generation),
            tab.grid.clone(),
            theme,
            Block::default().style(Style::new().bg(theme.surface)),
            state,
            tab.query.submitted.order_by_clause.as_deref().unwrap_or(""),
            tab.last_execution
                .as_ref()
                .map_or(crate::sql::SqlDialect::Sqlite, |last| last.draft.dialect),
            tab.query_status != QueryStatus::Running
                && !tab.derived.as_ref().is_some_and(|derived| derived.running),
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
        pagination,
        pagination::PaginationKind::Result,
        theme,
        state,
        tab.query_status != QueryStatus::Running
            && !tab.derived.as_ref().is_some_and(|derived| derived.running)
            && displayed_outcome.is_some(),
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
    data_revision: u64,
    grid: crate::model::tab::GridState,
    theme: Theme,
    block: Block<'_>,
    state: &mut UiState,
    order_by_clause: &str,
    dialect: crate::sql::SqlDialect,
    sort_interactive: bool,
) {
    state.result_area = Some(area);
    let overrides = grid.column_widths.clone();
    let icons = state.activity_icons;
    let column_names = result
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    let sort_projection =
        crate::sql::relation_column_sort_projection(order_by_clause, &column_names, dialect)
            .unwrap_or_else(|_| vec![None; column_names.len()]);
    let sort_interactive = sort_interactive && !tabular_column_names_ambiguous(&column_names);
    data_grid::render(
        frame,
        area,
        tab_id,
        result,
        data_revision,
        grid,
        &overrides,
        theme,
        block,
        state,
        None,
        icons,
        Some(sort_projection.as_slice()),
        sort_interactive,
    );
}

fn tabular_column_names_ambiguous(columns: &[&str]) -> bool {
    let mut names = HashSet::new();
    columns
        .iter()
        .any(|name| !names.insert(name.to_lowercase()))
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
    if let Some(session_id) = app.active_console_opt().map(|tab| tab.output_editor_id) {
        state.output_viewport = Some((session_id, viewport));
    }
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
        let content = editor_line_spans(
            line,
            &snapshot,
            theme,
            true,
            None,
            &mouse_selection_cells(
                state,
                app.active_console_opt()
                    .map(|tab| tab.output_editor_id)
                    .unwrap_or_default(),
                &snapshot,
                line,
            ),
            app.active_console_opt()
                .and_then(|tab| crate::app::output_line_style(tab, line.line)),
        );
        frame.render_widget(
            Paragraph::new(Line::from(content))
                .style(Style::new().bg(theme.surface))
                .scroll((0, snapshot.horizontal_offset.min(u16::MAX as usize) as u16)),
            Rect::new(inner.x.saturating_add(3), y, viewport.width as u16, 1),
        );
    }
    if let Some(session_id) = app.active_console_opt().map(|tab| tab.output_editor_id) {
        render_editor_scrollbars(
            frame,
            inner,
            Some(session_id),
            &snapshot,
            theme,
            state,
            Some(Rect::new(
                inner.x.saturating_add(3),
                inner.bottom().saturating_sub(1),
                viewport.width as u16,
                1,
            )),
        );
    }
    if app.focus == Focus::Results
        && app.overlay.is_none()
        && let Some((x, y)) = snapshot.cursor_screen_cell
    {
        state.cursor = Some(CursorSpec {
            position: Position::new(
                inner.x.saturating_add(3).saturating_add(x),
                inner.y.saturating_add(y),
            ),
            style: CursorStyle::Block,
        });
    }
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
        Overlay::Update(_) => update::render(frame, area, app, state, theme),
        Overlay::NotificationHistory(history) => {
            notifications::render_history(frame, area, app, history, theme, state, icons)
        }
        Overlay::NotificationDetail(detail) => {
            notifications::render_detail(frame, area, app, detail, theme, icons)
        }
        Overlay::RecordView(view) => record_view::render(frame, area, app, view, theme, state),
        Overlay::TextDetail(view) => text_detail::render(frame, area, app, view, theme, state),
        Overlay::SqlHistory(view) => {
            sql_history_modal::render(frame, area, app, view, theme, state)
        }
        Overlay::ProfileManager => {
            profiles::render_profile_manager(frame, area, app, state, theme, icons)
        }
        Overlay::CatalogEditor => catalog_editor::render(frame, area, app, state, theme, icons),
        Overlay::RedisObjectEditor(editor) => {
            redis_object_editor::render(frame, area, editor, app, state, theme)
        }
        Overlay::RedisTableEditor(editor) => {
            redis_table_editor::render(frame, area, editor, state, theme)
        }
        Overlay::RedisTableDeleteConfirm(confirm) => {
            redis_table_editor::render_delete_confirm(frame, area, confirm, state, theme)
        }
        Overlay::MongoManagement(draft) => {
            mongodb_management::render(frame, area, draft, app, state, theme)
        }
        Overlay::MongoPrincipalMutation(draft) => {
            mongodb_principal::render_mutation(frame, area, draft, theme, state)
        }
        Overlay::MongoPrincipalConfirmation {
            mutation,
            expected,
            input,
        } => mongodb_principal::render_confirmation(
            frame,
            area,
            mutation,
            expected,
            input.value(),
            theme,
            state,
        ),
        Overlay::RedisValueSaveConfirm {
            tab_id,
            revision,
            invalid,
            validation_error,
            focus,
            ..
        } => {
            let popup = centered(area, 76, 12.min(area.height));
            let inner = dialog::render_frame(
                frame,
                popup,
                if *invalid {
                    " SAVE INVALID REDIS VALUE? "
                } else {
                    " SAVE REDIS VALUE? "
                },
                theme,
            );
            let mut lines = vec![Line::from(Span::styled(
                format!("Revision {revision}"),
                theme.title(true),
            ))];
            if let Some(error) = validation_error {
                lines.push(Line::from(Span::styled(
                    error.clone(),
                    Style::new().fg(theme.warning),
                )));
                lines.push(Line::raw("The current text will be saved as-is."));
            } else {
                lines.push(Line::raw("Save the edited Redis value?"));
            }
            dialog::render_body(
                frame,
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(3),
                ),
                lines,
                theme,
            );
            let buttons = if *invalid {
                [
                    dialog::DialogButton {
                        label: "Save anyway",
                        tone: dialog::DialogTone::Danger,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                    dialog::DialogButton {
                        label: "Back to edit",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                ]
            } else {
                [
                    dialog::DialogButton {
                        label: "Save",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                    dialog::DialogButton {
                        label: "Cancel",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                ]
            };
            let actions = dialog::render_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                &buttons,
                *focus,
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: HitTarget::RedisValueSaveAction(action.index),
                });
            }
            dialog::render_interactive_hints(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                &[
                    ShortcutHint::with_keys(
                        "Tab",
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
                state,
            );
            let _ = tab_id;
        }
        Overlay::RedisUnsavedValueConfirm {
            next_key, focus, ..
        } => {
            let popup = centered(area, 82, 12.min(area.height));
            let inner = dialog::render_frame(frame, popup, " UNSAVED REDIS VALUE ", theme);
            dialog::render_body(
                frame,
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(3),
                ),
                vec![
                    Line::from(Span::styled(
                        "Local edits have not been saved.",
                        theme.title(true),
                    )),
                    Line::from(format!(
                        "Target: {}",
                        crate::ui::redis_value::display_bytes_lossless(&next_key.key)
                    )),
                    Line::raw("Choose how to continue."),
                ],
                theme,
            );
            let buttons = [
                dialog::DialogButton {
                    label: "Save",
                    tone: dialog::DialogTone::Normal,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
                dialog::DialogButton {
                    label: "Discard",
                    tone: dialog::DialogTone::Danger,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
                dialog::DialogButton {
                    label: "Cancel",
                    tone: dialog::DialogTone::Normal,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
            ];
            let actions = dialog::render_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                &buttons,
                *focus,
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: HitTarget::RedisUnsavedValueAction(action.index),
                });
            }
            dialog::render_interactive_hints(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                &[
                    ShortcutHint::with_keys(
                        "Tab",
                        "switch",
                        [KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)],
                    ),
                    ShortcutHint::with_keys(
                        "Enter",
                        "activate",
                        [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                    ),
                    ShortcutHint::with_keys(
                        "s",
                        "save",
                        [KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)],
                    ),
                    ShortcutHint::with_keys(
                        "d",
                        "discard",
                        [KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)],
                    ),
                    ShortcutHint::with_keys(
                        "Esc",
                        "cancel",
                        [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
                    ),
                ],
                theme,
                state,
            );
        }
        Overlay::ProfileAccess {
            profile_id,
            selected,
            options,
        } => render_profile_access(frame, area, app, *profile_id, *selected, options, theme),
        Overlay::Message { title, body } => render_message(frame, area, title, body, theme),
        Overlay::WorkspaceSaveFailed {
            revision,
            message,
            retryable,
            focus,
            detail_scroll,
        } => render_workspace_save_failed(
            frame,
            area,
            *revision,
            message,
            *retryable,
            *focus,
            *detail_scroll,
            theme,
            state,
        ),
        Overlay::SubstituteConfirm { remaining } => {
            render_substitute_confirm(frame, area, *remaining, theme)
        }
        Overlay::ExecutionConfirm {
            draft,
            focus,
            preview_offset,
        } => execution_confirm::render(
            frame,
            area,
            draft,
            *focus,
            *preview_offset,
            app,
            state,
            theme,
        ),
        Overlay::PrincipalMutationConfirm { plan, focus } => {
            principal_mutation_confirm::render(frame, area, plan, *focus, state, theme)
        }
        Overlay::PrincipalMutationForm(form) => {
            principal_mutation_form::render(frame, area, form, state, theme)
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
            ];
            let inner = dialog::render_frame(frame, popup, " CANCELLATION CONFIRMATION ", theme);
            dialog::render_body(
                frame,
                Rect::new(inner.x, inner.y, inner.width, 3),
                lines,
                theme,
            );
            let actions = dialog::render_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                &[
                    dialog::DialogButton {
                        label: "Keep running",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                    dialog::DialogButton {
                        label: "Cancel query + roll back",
                        tone: dialog::DialogTone::Danger,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                ],
                usize::from(cancel),
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: if action.index == 0 {
                        HitTarget::ManualCancellationKeepRunning
                    } else {
                        HitTarget::ManualCancellationConfirm
                    },
                });
            }
            dialog::render_interactive_hint(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                "Tab / Left / Right switch   Enter activate   Esc keep running",
                theme,
                state,
            );
        }
        Overlay::TransactionExitConfirm { prompt, choice } => {
            render_transaction_exit_overlay(frame, area, app, prompt, *choice, theme, state);
        }
        Overlay::RelationTransactionConfirm(review) => {
            let tab_id = &review.tab_id;
            let choice = review
                .focus
                .choice()
                .unwrap_or(crate::model::transaction::TransactionExitChoice::Cancel);
            use crate::model::transaction::TransactionExitChoice;
            let popup = centered(
                area,
                area.width.saturating_sub(4).min(110),
                18.min(area.height),
            );
            frame.render_widget(Clear, popup);
            let title = app
                .tabs
                .iter()
                .find(|tab| tab.id() == *tab_id)
                .map(|tab| tab.title())
                .unwrap_or("unknown");
            let transaction_state =
                app.tabs
                    .iter()
                    .find(|tab| tab.id() == *tab_id)
                    .and_then(|tab| match tab {
                        WorkspaceTab::Relation(tab) => Some(tab.transaction_state),
                        _ => None,
                    });
            let (status, status_color, commit_help, rollback_help) = match transaction_state {
                Some(crate::model::transaction::TransactionState::Active) => (
                    "ACTIVE TRANSACTION",
                    theme.warning,
                    "Save transaction",
                    "Undo transaction",
                ),
                Some(crate::model::transaction::TransactionState::Aborted) => (
                    "ABORTED - rollback required",
                    theme.error,
                    "Unavailable",
                    "Undo transaction",
                ),
                _ => (
                    "LOCAL CHANGES",
                    theme.warning,
                    "Apply changes",
                    "Discard local edits",
                ),
            };
            let inner = dialog::render_frame(frame, popup, " TRANSACTION REVIEW ", theme);
            let inner = inner.inner(ratatui::layout::Margin::new(
                u16::from(inner.width > 6) * 2,
                u16::from(inner.height >= 12),
            ));
            let action_height = if inner.width < 42 { 3 } else { 1 };
            let sections = Layout::vertical([
                Constraint::Length(2),
                Constraint::Min(0),
                Constraint::Length(action_height + 4),
            ])
            .split(inner);
            let muted = Style::new().fg(theme.muted);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled("TABLE  ", muted),
                        Span::styled(
                            sanitize_terminal_text(title),
                            Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(Span::styled(status, Style::new().fg(status_color))),
                ]),
                sections[0],
            );
            let preview_block = Block::default()
                .borders(Borders::TOP)
                .border_style(Style::new().fg(theme.border))
                .title(" SQL preview ")
                .title_style(muted)
                .style(Style::new().bg(theme.surface));
            let preview_inner = preview_block.inner(sections[1]);
            let viewport = crate::model::editor::EditorViewport {
                width: preview_inner.width as usize,
                height: preview_inner.height as usize,
            };
            state.transaction_review_viewport = Some((review.editor_session_id, viewport));
            if let Ok(snapshot) = app.transaction_review_snapshot(viewport) {
                crate::ui::read_only_sql::ReadOnlySqlEditor {
                    session_id: review.editor_session_id,
                    snapshot: &snapshot,
                    block: preview_block.clone(),
                    focused: review.focus
                        == crate::model::transaction_review::TransactionReviewFocus::SqlPreview,
                    show_line_numbers: true,
                }
                .render(frame, sections[1], theme, state);
            } else {
                frame.render_widget(preview_block, sections[1]);
            }
            if review.sql.trim().is_empty() {
                frame.render_widget(
                    Paragraph::new(
                        "SQL preview unavailable: no review SQL could be generated or recovered.",
                    )
                    .style(Style::new().fg(theme.warning).bg(theme.surface))
                    .wrap(Wrap { trim: true }),
                    preview_inner,
                );
            }
            let footer = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(action_height),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(sections[2]);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("Commit", Style::new().fg(theme.accent)),
                    Span::styled(format!("  {commit_help}   /   "), muted),
                    Span::styled("Rollback", Style::new().fg(theme.error)),
                    Span::styled(format!("  {rollback_help}"), muted),
                ]))
                .wrap(Wrap { trim: true })
                .alignment(Alignment::Center),
                footer[0],
            );
            let selected = match choice {
                TransactionExitChoice::Commit => 0,
                TransactionExitChoice::Rollback => 1,
                _ => 2,
            };
            let buttons = [
                dialog::DialogButton {
                    label: "Commit",
                    tone: dialog::DialogTone::Normal,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
                dialog::DialogButton {
                    label: "Rollback",
                    tone: dialog::DialogTone::Danger,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
                dialog::DialogButton {
                    label: "Cancel",
                    tone: dialog::DialogTone::Normal,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
            ];
            let actions = if review.focus
                == crate::model::transaction_review::TransactionReviewFocus::SqlPreview
            {
                dialog::render_actions_without_focus(frame, footer[1], &buttons, theme)
            } else {
                dialog::render_actions(frame, footer[1], &buttons, selected, theme)
            };
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: match action.index {
                        0 => HitTarget::TransactionExitChoice(TransactionExitChoice::Commit),
                        1 => HitTarget::TransactionExitChoice(TransactionExitChoice::Rollback),
                        _ => HitTarget::TransactionExitCancel,
                    },
                });
            }
            shortcut_hints::render(
                frame,
                footer[3],
                &[
                    shortcut_hints::ShortcutHint::new("Enter", "confirm"),
                    shortcut_hints::ShortcutHint::new("Esc", "cancel"),
                    shortcut_hints::ShortcutHint::new("Tab/Shift-Tab", "focus"),
                    shortcut_hints::ShortcutHint::new("hjkl/v/y", "preview"),
                ],
                theme,
                theme.surface_raised,
                Alignment::Center,
            );
        }
        Overlay::ClearTransactionOutcome { focus, .. } => {
            let popup = centered(area, 78, 12);
            let inner = dialog::render_frame(frame, popup, " TRANSACTION OUTCOME UNKNOWN ", theme);
            dialog::render_body(
                frame,
                Rect::new(inner.x, inner.y, inner.width, 4),
                vec![
                    Line::from(Span::styled(" VERIFY UNKNOWN OUTCOME ", theme.title(true))),
                    Line::raw("LazyDB cannot know whether commit/rollback reached the server."),
                    Line::raw("Verify externally before clearing this transaction state."),
                ],
                theme,
            );
            let actions = dialog::render_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                &[
                    dialog::DialogButton {
                        label: "Cancel",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                    dialog::DialogButton {
                        label: "Clear after verification",
                        tone: dialog::DialogTone::Danger,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                ],
                usize::from(*focus == crate::model::workspace::ClearTransactionOutcomeFocus::Clear),
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: if action.index == 0 {
                        HitTarget::ClearTransactionCancel
                    } else {
                        HitTarget::ClearTransactionConfirm
                    },
                });
            }
            dialog::render_interactive_hint(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                "Tab / Left / Right switch   Enter activate   Esc cancel",
                theme,
                state,
            );
        }
        Overlay::TransactionMenu { selected } => {
            render_transaction_menu(frame, area, app, *selected, theme, state);
        }
        Overlay::TargetSelector {
            candidates,
            selected,
            console_id,
            ..
        } => {
            const MAX_VISIBLE_ROWS: usize = 16;
            let height = (candidates.len().min(MAX_VISIBLE_ROWS) as u16)
                .saturating_add(5)
                .clamp(8, 24);
            let popup = centered(area, 68, height);
            frame.render_widget(Clear, popup);
            let block = panel_block(" TARGET SELECTOR ", true, theme);
            let inner = block.inner(popup);
            frame.render_widget(block, popup);
            let visible_count = candidates
                .len()
                .min(MAX_VISIBLE_ROWS)
                .min(usize::from(inner.height.saturating_sub(3)));
            let current = app
                .tabs
                .iter()
                .find(|tab| console_id.is_none_or(|id| tab.id() == id))
                .and_then(WorkspaceTab::as_console)
                .and_then(|tab| tab.execution_target.as_ref());
            let start = selected
                .saturating_sub(visible_count.saturating_sub(1))
                .min(candidates.len().saturating_sub(visible_count));
            let end = start.saturating_add(visible_count);
            let mut lines = Vec::with_capacity(visible_count.saturating_add(2));
            lines.extend(
                candidates[start..end]
                    .iter()
                    .enumerate()
                    .map(|(offset, candidate)| {
                        let index = start + offset;
                        let marker = if index == *selected { ">" } else { " " };
                        let (profile, profile_label, target_label, current_candidate) =
                            match candidate {
                                TargetSelectorCandidate::None => (
                                    None,
                                    String::new(),
                                    "No connection".to_owned(),
                                    current.is_none(),
                                ),
                                TargetSelectorCandidate::Target(target) => {
                                    let profile = app
                                        .profiles
                                        .iter()
                                        .find(|profile| profile.id == target.profile_id);
                                    let profile_label = profile
                                        .map(|profile| {
                                            format!("{}: ", sanitize_terminal_text(&profile.name))
                                        })
                                        .unwrap_or_default();
                                    let target_label = format!(
                                        "{}{}",
                                        sanitize_terminal_text(&target.database),
                                        target
                                            .schema
                                            .as_deref()
                                            .map(|schema| {
                                                format!(".{}", sanitize_terminal_text(schema))
                                            })
                                            .unwrap_or_default(),
                                    );
                                    (
                                        profile,
                                        profile_label,
                                        target_label,
                                        current == Some(target),
                                    )
                                }
                            };
                        let current_marker = if current_candidate { " current" } else { "" };
                        let target_label = format!("{target_label}{current_marker}");
                        let background = if index == *selected {
                            theme.selection
                        } else {
                            theme.surface_raised
                        };
                        let text_style = if index == *selected {
                            Style::new()
                                .fg(theme.text)
                                .bg(background)
                                .add_modifier(Modifier::BOLD)
                        } else if current_candidate {
                            Style::new().fg(theme.accent).bg(background)
                        } else {
                            Style::new().fg(theme.text).bg(background)
                        };
                        let icon = profile.map(|profile| icons.database(profile.kind));
                        let icon_width = icon.map_or(0, |icon| usize::from(icon.cell_width()));
                        let prefix = format!("{marker} ");
                        let text_width = usize::from(inner.width)
                            .saturating_sub(usize::from(prefix.cell_width()))
                            .saturating_sub(usize::from(profile_label.cell_width()))
                            .saturating_sub(icon_width.saturating_add(1));
                        let mut spans = vec![Span::styled(prefix, text_style)];
                        if let Some(profile) = profile {
                            spans.push(Span::styled(
                                format!("{} ", icons.database(profile.kind)),
                                Style::new()
                                    .fg(icons.database_color(profile.kind))
                                    .bg(background),
                            ));
                        }
                        spans.push(Span::styled(profile_label, text_style));
                        spans.push(Span::styled(
                            truncate_to_cells(&target_label, text_width),
                            text_style,
                        ));
                        Line::from(spans)
                    }),
            );
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                " Cancel ",
                Style::new().fg(theme.text).bg(theme.surface_raised),
            )));
            let cancel_y = inner.y.saturating_add(visible_count as u16 + 2);
            if inner.width > 0 && cancel_y < inner.bottom() {
                state.hit_regions.push(HitRegion {
                    area: Rect::new(inner.x, cancel_y, inner.width, 1),
                    target: HitTarget::TargetSelectorCancel,
                });
            }
            for (offset, _) in candidates[start..end].iter().enumerate() {
                let index = start + offset;
                let row = inner.y.saturating_add(offset as u16);
                if inner.width > 0 && row < inner.bottom() {
                    state.hit_regions.push(HitRegion {
                        area: Rect::new(inner.x, row, inner.width, 1),
                        target: HitTarget::TargetSelectorRow(index),
                    });
                }
            }
            shortcut_hints::render_interactive(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                &[
                    ShortcutHint::with_keys(
                        "j/k or Up/Down",
                        "select",
                        [KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)],
                    ),
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
                ],
                theme,
                theme.surface_raised,
                Alignment::Center,
                state,
            );
            frame.render_widget(
                Paragraph::new(lines).style(Style::new().fg(theme.text).bg(theme.surface_raised)),
                inner,
            );
        }
        Overlay::DatabaseSelector(selector) => {
            let visible_count = selector.candidates.len().clamp(1, 8);
            let height = (visible_count as u16 + 4).clamp(6, 12);
            let popup = centered(area, 52.min(area.width.saturating_sub(2)), height);
            frame.render_widget(Clear, popup);
            let start = selector
                .selected
                .saturating_add(1)
                .saturating_sub(visible_count)
                .min(selector.candidates.len().saturating_sub(visible_count));
            let end = (start + visible_count).min(selector.candidates.len());
            let mut lines = selector.candidates[start..end]
                .iter()
                .enumerate()
                .map(|(offset, target)| {
                    let index = start + offset;
                    let selected = index == selector.selected;
                    let marker = if selected { ">" } else { " " };
                    let current = if selector.is_current(&target.database) {
                        " (current)"
                    } else {
                        ""
                    };
                    Line::from(Span::styled(
                        truncate_to_cells(
                            &format!(
                                "{marker} {}{current}",
                                sanitize_terminal_text(&target.database)
                            ),
                            popup.width.saturating_sub(2) as usize,
                        ),
                        if selected {
                            Style::new().fg(theme.text).bg(theme.selection)
                        } else {
                            Style::new().fg(theme.text)
                        },
                    ))
                })
                .collect::<Vec<_>>();
            if selector.candidates.is_empty() {
                lines.push(Line::from(Span::styled(
                    "No databases available",
                    Style::new().fg(theme.muted),
                )));
            }
            lines.push(Line::raw(""));
            lines.push(Line::raw("j/k or Up/Down select  Enter switch  Esc cancel"));
            for (offset, index) in (start..end).enumerate() {
                state.hit_regions.push(HitRegion {
                    area: Rect::new(
                        popup.x + 1,
                        popup.y + 1 + offset as u16,
                        popup.width.saturating_sub(2),
                        1,
                    ),
                    target: HitTarget::DatabaseSelectorRow(index),
                });
            }
            frame.render_widget(
                Paragraph::new(lines)
                    .block(panel_block(" SWITCH DATABASE ", true, theme))
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
        Overlay::RedisPreviewFormat { selected } => {
            let popup = centered(area, 42, 13);
            frame.render_widget(Clear, popup);
            let mut lines = vec![];
            let labels = [
                "Auto",
                "RAW",
                "JSON",
                "YAML",
                "Table",
                "Hex",
                "Java Serialization",
                "PHP Serialization",
                "Python Pickle",
            ];
            for (index, label) in labels.iter().enumerate() {
                lines.push(Line::styled(
                    format!(" {} {label}", if index == *selected { ">" } else { " " }),
                    if index == *selected {
                        theme.base().bg(theme.selection)
                    } else {
                        theme.base()
                    },
                ));
            }
            lines.push(Line::raw(""));
            lines.push(Line::raw(" j/k select  Enter apply  Esc cancel"));
            frame.render_widget(
                Paragraph::new(lines).block(panel_block(" VALUE FORMAT ", true, theme)),
                popup,
            );
        }
        Overlay::DeleteConsole { console_id, focus } => {
            let popup = centered(area, 76, 8);
            let inner = dialog::render_frame(frame, popup, " DELETE CONFIRMATION ", theme);
            let name = app
                .sql_editors
                .iter()
                .find(|record| record.id == *console_id)
                .map_or("unknown", |record| record.name.as_str());
            dialog::render_body(
                frame,
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(3),
                ),
                vec![
                    Line::from(Span::styled(" DELETE SQL EDITOR? ", theme.title(true))),
                    Line::raw(format!(
                        "Permanently delete '{name}' and its saved SQL file?"
                    )),
                ],
                theme,
            );
            let actions = dialog::render_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                &[
                    dialog::DialogButton {
                        label: "Cancel",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                    dialog::DialogButton {
                        label: "Delete console",
                        tone: dialog::DialogTone::Danger,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                ],
                usize::from(*focus == crate::model::workspace::DeleteConsoleFocus::Delete),
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: if action.index == 0 {
                        HitTarget::DeleteConsoleCancel
                    } else {
                        HitTarget::DeleteConsoleConfirm
                    },
                });
            }
            dialog::render_interactive_hint(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                "Tab / Left / Right switch   Enter activate   Esc cancel",
                theme,
                state,
            );
        }
        Overlay::RedisDeleteConfirm {
            target,
            count,
            focus,
            ..
        } => {
            let popup = centered(area, 76, 8);
            let inner = dialog::render_frame(frame, popup, " DELETE REDIS TARGET ", theme);
            dialog::render_body(
                frame,
                Rect::new(
                    inner.x,
                    inner.y,
                    inner.width,
                    inner.height.saturating_sub(3),
                ),
                vec![
                    Line::from(Span::styled(
                        match target {
                            crate::model::workspace::RedisDeleteTarget::Key(_) => {
                                " DELETE REDIS KEY? "
                            }
                            crate::model::workspace::RedisDeleteTarget::Prefix { .. } => {
                                " DELETE REDIS GROUP? "
                            }
                        },
                        theme.title(true),
                    )),
                    Line::raw(format!(
                        "Permanently delete '{}' from DB {}?",
                        match target {
                            crate::model::workspace::RedisDeleteTarget::Key(key) => {
                                crate::model::redis_key_text::display_bytes(&key.key)
                            }
                            crate::model::workspace::RedisDeleteTarget::Prefix {
                                prefix, ..
                            } => {
                                format!(
                                    "all keys beginning with '{}' ({} keys found)",
                                    crate::model::redis_key_text::display_bytes(prefix),
                                    count
                                )
                            }
                        },
                        match target {
                            crate::model::workspace::RedisDeleteTarget::Key(key) =>
                                key.target.database,
                            crate::model::workspace::RedisDeleteTarget::Prefix {
                                target, ..
                            } => target.database,
                        }
                    )),
                ],
                theme,
            );
            let actions = dialog::render_actions(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
                &[
                    dialog::DialogButton {
                        label: "Cancel",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                    dialog::DialogButton {
                        label: "Delete key",
                        tone: dialog::DialogTone::Danger,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                ],
                usize::from(*focus == crate::model::workspace::DeleteConsoleFocus::Delete),
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: if action.index == 0 {
                        HitTarget::RedisDeleteCancel
                    } else {
                        HitTarget::RedisDeleteConfirm
                    },
                });
            }
            dialog::render_interactive_hint(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                "Tab / Left / Right switch   Enter activate   Esc cancel",
                theme,
                state,
            );
        }
        Overlay::RedisDeletePreparing { target, .. } => {
            let popup = centered(area, 76, 6);
            let inner = dialog::render_frame(frame, popup, " PREPARING REDIS DELETE ", theme);
            dialog::render_body(
                frame,
                inner,
                vec![Line::raw(match target {
                    crate::model::workspace::RedisDeleteTarget::Key(_) => "Preparing key deletion…",
                    crate::model::workspace::RedisDeleteTarget::Prefix { .. } => {
                        "Scanning matching keys…"
                    }
                })],
                theme,
            );
        }
        Overlay::SqlEditorList(list) => {
            render_console_manager(frame, area, app, list, state, theme)
        }
        Overlay::CatalogDropConfirm { .. } => {
            render_catalog_drop_confirm(frame, area, app, state, theme);
        }
        Overlay::PrincipalDropConfirm {
            plan,
            delete_selected,
            busy,
            error,
        } => {
            let popup = centered(area, 72, 12);
            let title = match plan.request.entry.kind {
                crate::db::principal::PrincipalKind::User => " DROP USER ",
                crate::db::principal::PrincipalKind::Role => " DROP ROLE ",
            };
            let inner = dialog::render_frame(frame, popup, title, theme);
            let chunks = Layout::vertical([
                Constraint::Min(0),
                Constraint::Length(2),
                Constraint::Length(1),
            ])
            .split(inner);
            let mut lines = vec![
                Line::raw("This operation will execute:"),
                Line::raw(""),
                Line::raw(plan.sql().to_owned()),
                Line::raw(""),
                Line::raw("This action cannot be undone."),
            ];
            if let Some(error) = error {
                lines.push(Line::styled(
                    crate::security::sanitize_terminal_text(error).to_owned(),
                    Style::new().fg(theme.error),
                ));
            }
            dialog::render_body(frame, chunks[0], lines, theme);
            let actions = dialog::render_actions(
                frame,
                chunks[1],
                &[
                    dialog::DialogButton {
                        label: "Cancel",
                        tone: dialog::DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: !*busy,
                    },
                    dialog::DialogButton {
                        label: if *busy { "Dropping..." } else { "Drop" },
                        tone: dialog::DialogTone::Danger,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: !*busy,
                    },
                ],
                usize::from(*delete_selected),
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: if action.index == 0 {
                        HitTarget::PrincipalDropCancel
                    } else {
                        HitTarget::PrincipalDropConfirm
                    },
                });
            }
            dialog::render_interactive_hint(
                frame,
                chunks[2],
                "Tab / Left / Right switch   Enter activate   Esc cancel",
                theme,
                state,
            );
        }
        Overlay::CatalogEditorDestructiveConfirm { plan, input } => {
            render_catalog_mutation_confirm(frame, area, plan, input, theme);
        }
        Overlay::CatalogEditorDiscardConfirm { focus } => {
            use crate::model::workspace::CatalogEditorDiscardFocus;
            let popup_width = area.width.saturating_sub(4).min(64);
            let compact = popup_width < 54;
            let popup_height = if compact { 12 } else { 11 };
            let popup = centered(area, popup_width, popup_height);
            let inner = dialog::render_frame(frame, popup, " DISCARD TABLE CHANGES ", theme);
            let action_height = u16::from(compact).saturating_add(1);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Min(0),
                    Constraint::Length(action_height),
                    Constraint::Length(1),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new("Discard unsaved table changes?")
                    .style(
                        Style::new()
                            .fg(theme.text)
                            .bg(theme.surface_raised)
                            .add_modifier(Modifier::BOLD),
                    )
                    .alignment(Alignment::Center),
                chunks[0],
            );
            frame.render_widget(
                Paragraph::new(vec![
                    Line::raw("Your draft changes will be lost."),
                    Line::raw("The database will remain unchanged."),
                ])
                .style(Style::new().fg(theme.muted).bg(theme.surface_raised))
                .alignment(Alignment::Center),
                chunks[2],
            );
            let action_area = if compact {
                let width = chunks[4].width.min(39);
                Rect::new(
                    chunks[4]
                        .x
                        .saturating_add(chunks[4].width.saturating_sub(width) / 2),
                    chunks[4].y,
                    width,
                    chunks[4].height,
                )
            } else {
                chunks[4]
            };
            let actions = dialog::render_actions(
                frame,
                action_area,
                &[
                    DialogButton {
                        label: "Keep Editing",
                        tone: DialogTone::Normal,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                    DialogButton {
                        label: "Discard Changes",
                        tone: DialogTone::Danger,
                        emphasis: dialog::DialogEmphasis::Secondary,
                        enabled: true,
                    },
                ],
                usize::from(*focus == CatalogEditorDiscardFocus::DiscardChanges),
                theme,
            );
            for action in actions {
                state.hit_regions.push(HitRegion {
                    area: action.area,
                    target: if action.index == 0 {
                        HitTarget::CatalogEditorDiscardKeepEditing
                    } else {
                        HitTarget::CatalogEditorDiscardChanges
                    },
                });
            }
            dialog::render_interactive_hint(
                frame,
                chunks[5],
                if compact {
                    "Tab move  Enter confirm  Esc back"
                } else {
                    "Tab / Left / Right switch   Enter confirm   Esc back"
                },
                theme,
                state,
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
    let profile = menu
        .profile_id
        .and_then(|profile_id| app.profiles.iter().find(|profile| profile.id == profile_id));
    let target = profile.map_or_else(
        || "TARGET  No connection selected".to_owned(),
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
    shortcut_hints::render_interactive(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
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
        theme.surface,
        Alignment::Center,
        state,
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

    let pending = std::iter::once(prompt.target)
        .chain(
            app.deferred_transaction_prompts()
                .filter(|queued| queued.intent == prompt.intent)
                .map(|queued| queued.target),
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
    for (index, target) in pending.iter().enumerate() {
        let y = row_start.saturating_add(index as u16);
        if y >= inner.bottom().saturating_sub(2) {
            break;
        }
        let id = match target {
            crate::model::transaction::DeferredTransactionTarget::Console(id)
            | crate::model::transaction::DeferredTransactionTarget::MongoConsole(id)
            | crate::model::transaction::DeferredTransactionTarget::Relation(id) => *id,
        };
        let tab = app.tabs.iter().find(|tab| tab.id() == id);
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

    if let crate::model::transaction::DeferredTransactionTarget::MongoConsole(id) = prompt.target {
        let unknown = app.tabs.iter().find(|tab| tab.id() == id)
            .is_some_and(|tab| matches!(tab, WorkspaceTab::MongoConsole(console) if console.transaction == crate::model::mongodb_console::MongoConsoleTransactionState::OutcomeUnknown));
        render_mongo_transaction_exit(frame, inner, choice, unknown, theme);
        return;
    }
    let current_console = app
        .tabs
        .iter()
        .find(|tab| {
            tab.id()
                == match prompt.target {
                    crate::model::transaction::DeferredTransactionTarget::Console(id)
                    | crate::model::transaction::DeferredTransactionTarget::MongoConsole(id)
                    | crate::model::transaction::DeferredTransactionTarget::Relation(id) => id,
                }
        })
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

fn render_mongo_transaction_exit(
    frame: &mut Frame<'_>,
    inner: Rect,
    choice: crate::model::transaction::TransactionExitChoice,
    unknown: bool,
    theme: Theme,
) {
    use crate::model::transaction::TransactionExitChoice;
    let actions: &[(&str, TransactionExitChoice)] = if unknown {
        &[
            ("[ Abandon ]", TransactionExitChoice::Abandon),
            ("Cancel", TransactionExitChoice::Cancel),
        ]
    } else {
        &[
            ("[ Commit ]", TransactionExitChoice::Commit),
            ("[ Rollback ]", TransactionExitChoice::Rollback),
            ("Cancel", TransactionExitChoice::Cancel),
        ]
    };
    let area = Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1);
    let spans = actions
        .iter()
        .flat_map(|(label, action)| {
            let style = if *action == choice {
                Style::new()
                    .fg(theme.background)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text).bg(theme.surface)
            };
            [
                Span::styled(format!("[ {label} ]"), style),
                Span::raw("   "),
            ]
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
    frame.render_widget(
        Paragraph::new("Tab/←/→ select   Enter confirm   Esc cancel")
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .alignment(Alignment::Center),
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
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
            let footer = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
            if *busy {
                frame.render_widget(
                    Paragraph::new("Updating group...")
                        .style(Style::new().fg(theme.muted).bg(theme.surface))
                        .alignment(Alignment::Center),
                    footer,
                );
            } else {
                shortcut_hints::render_interactive(
                    frame,
                    footer,
                    &[
                        ShortcutHint::with_keys(
                            "↑/↓",
                            "select",
                            [KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)],
                        ),
                        ShortcutHint::with_keys(
                            "Enter",
                            "apply",
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
                state.hit_regions.push(HitRegion {
                    area: input_area,
                    target: HitTarget::ProfileGroupName,
                });
                register_input_selection_target(
                    state,
                    text_selection::InputSelectionTarget::ProfileGroupName,
                    input_area,
                    "",
                    name,
                    text_input_horizontal_offset(input_area, "", name),
                );
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
                false,
                state,
                theme,
            );
            shortcut_hints::render_interactive(
                frame,
                Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
                &[
                    ShortcutHint::with_keys(
                        "Enter",
                        "save",
                        [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                    ),
                    ShortcutHint::with_keys(
                        "Esc",
                        "cancel",
                        [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
                    ),
                    ShortcutHint::new("Ctrl-W/U/A/E", "edit"),
                ],
                theme,
                theme.surface,
                Alignment::Center,
                state,
            );
        }
        ProfileGroupOverlay::DeleteConfirm {
            member_count,
            cancel_selected,
            busy,
            ..
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
                *cancel_selected,
                state,
                theme,
            );
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
                        "confirm",
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
    }
}

fn render_profile_group_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    confirm_label: &str,
    enabled: bool,
    cancel_selected: bool,
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
        Paragraph::new(confirm).style(if enabled && !cancel_selected {
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
        Paragraph::new(cancel).style(if enabled && cancel_selected {
            Style::new()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD)
        }),
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
    let popup = centered(area, 80, desired_height.clamp(8, 24));
    frame.render_widget(Clear, popup);

    let title = match &list.mode {
        SqlEditorListMode::Browse => " CONSOLES ",
        SqlEditorListMode::Search => " CONSOLES // SEARCH ",
        SqlEditorListMode::Rename { .. } => " CONSOLES // RENAME ",
        SqlEditorListMode::DeleteConfirm { .. } => " CONSOLES // DELETE ",
    };
    let block = panel_block(title, true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.is_empty() {
        return;
    }
    let deleting = matches!(&list.mode, SqlEditorListMode::DeleteConfirm { .. });
    let reserved_rows = if deleting { 2 } else { 1 };
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(reserved_rows),
    );
    let footer_area = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
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
                let icons = state.activity_icons;
                let rows = records
                    .iter()
                    .map(|record| {
                        let target = record.execution_target.as_ref();
                        let profile = target.and_then(|target| {
                            app.profiles
                                .iter()
                                .find(|profile| profile.id == target.profile_id)
                        });
                        let target_valid = target
                            .zip(profile)
                            .is_some_and(|(target, profile)| target.is_valid(profile));
                        let connection = profile.map_or_else(
                            || {
                                if target.is_none() {
                                    "Unbound"
                                } else {
                                    "Invalid target"
                                }
                                .to_owned()
                            },
                            |profile| {
                                if target_valid {
                                    profile.name.clone()
                                } else {
                                    "Invalid target".to_owned()
                                }
                            },
                        );
                        let location = target.map_or_else(String::new, |target| {
                            target.schema.as_deref().map_or_else(
                                || target.database.clone(),
                                |schema| format!("{}/{}", target.database, schema),
                            )
                        });
                        let icon = profile.map(|profile| {
                            (
                                icons.database(profile.kind).to_owned(),
                                icons.database_color(profile.kind),
                            )
                        });
                        (record, connection, location, icon)
                    })
                    .collect::<Vec<_>>();
                let available = usize::from(inner.width);
                let location_width = rows
                    .iter()
                    .map(|(_, _, location, _)| usize::from(location.cell_width()))
                    .max()
                    .unwrap_or(0)
                    .min(28usize);
                let connection_natural = rows
                    .iter()
                    .map(|(_, connection, _, icon)| {
                        usize::from(connection.cell_width())
                            + icon
                                .as_ref()
                                .map_or(0, |(icon, _)| usize::from(icon.cell_width()) + 1)
                    })
                    .max()
                    .unwrap_or(0);
                let connection_width = connection_natural.min(
                    available
                        .saturating_sub(location_width + 8)
                        .min(available * 3 / 5),
                );
                let right_width = connection_width + location_width + 2usize;
                let name_width = available.saturating_sub(right_width + 6);
                lines.extend(rows.iter().map(|(record, connection, location, icon)| {
                    let selected = list.selected_id == Some(record.id);
                    let name = truncate_to_cells(&record.name, name_width);
                    let status = match (icons.mode(), record.open) {
                        (icons::IconMode::Ascii, true) => "*",
                        (icons::IconMode::Ascii, false) => "o",
                        (_, true) => "●",
                        (_, false) => "○",
                    };
                    let connection = truncate_to_cells(connection, connection_width);
                    let location = truncate_to_cells(location, location_width);
                    let connection_used = usize::from(connection.cell_width())
                        + icon
                            .as_ref()
                            .map_or(0, |(icon, _)| usize::from(icon.cell_width()) + 1);
                    let connection_padding = connection_width.saturating_sub(connection_used);
                    let location_padding =
                        location_width.saturating_sub(usize::from(location.cell_width()));
                    let middle_padding = available.saturating_sub(
                        2 + usize::from(name.cell_width())
                            + 1
                            + usize::from(status.cell_width())
                            + right_width,
                    );
                    let background = if selected {
                        theme.selection
                    } else {
                        theme.surface
                    };
                    let prefix = if selected { "> " } else { "  " };
                    Line::from(vec![
                        Span::styled(
                            format!("{prefix}{name} "),
                            theme.base().bg(background).add_modifier(if selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                        ),
                        Span::styled(
                            status,
                            theme
                                .base()
                                .fg(if record.open {
                                    theme.accent
                                } else {
                                    theme.muted
                                })
                                .bg(background),
                        ),
                        Span::styled(" ".repeat(middle_padding), theme.base().bg(background)),
                        Span::styled(" ".repeat(connection_padding), theme.base().bg(background)),
                        icon.as_ref().map_or_else(
                            || Span::styled("", theme.base().bg(background)),
                            |(icon, color)| {
                                Span::styled(
                                    format!("{icon} "),
                                    theme.base().fg(*color).bg(background),
                                )
                            },
                        ),
                        Span::styled(connection, theme.base().fg(theme.text).bg(background)),
                        Span::styled("  ", theme.base().bg(background)),
                        Span::styled(
                            format!("{}{}", " ".repeat(location_padding), location),
                            theme.base().fg(theme.muted).bg(background),
                        ),
                        Span::styled(" ", theme.base().bg(background)),
                    ])
                }));
            }
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
            lines.push(Line::raw(""));
        }
    }
    if !body.is_empty() {
        frame.render_widget(Paragraph::new(lines).style(theme.base()), body);
    }
    match &list.mode {
        SqlEditorListMode::Search => {
            let input_area = Rect::new(body.x, body.y, body.width, 1);
            if !input_area.is_empty() {
                state.hit_regions.push(HitRegion {
                    area: input_area,
                    target: HitTarget::SqlEditorListSearch,
                });
                render_text_input(frame, input_area, "/", &list.query, theme.base(), state);
                register_input_selection_target(
                    state,
                    text_selection::InputSelectionTarget::ConsoleManagerSearch,
                    input_area,
                    "/",
                    &list.query,
                    text_input_horizontal_offset(input_area, "/", &list.query),
                );
            }
        }
        SqlEditorListMode::Rename { input, .. } => {
            let input_area = Rect::new(body.x, body.y.saturating_add(2), body.width, 1);
            if input_area.y < body.bottom() && !input_area.is_empty() {
                state.hit_regions.push(HitRegion {
                    area: input_area,
                    target: HitTarget::SqlEditorListRename,
                });
                render_text_input(frame, input_area, "Name: ", input, theme.base(), state);
                register_input_selection_target(
                    state,
                    text_selection::InputSelectionTarget::ConsoleManagerRename,
                    input_area,
                    "Name: ",
                    input,
                    text_input_horizontal_offset(input_area, "Name: ", input),
                );
            }
        }
        _ => {}
    }
    match &list.mode {
        SqlEditorListMode::Browse => {
            let hints = vec![
                ShortcutHint::with_keys(
                    "j/k",
                    "move",
                    [KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "Enter",
                    "open",
                    [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "a",
                    "new",
                    [KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "d",
                    "delete",
                    [KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "r",
                    "rename",
                    [KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "/",
                    "search",
                    [KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "Esc",
                    "close",
                    [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
                ),
            ];
            shortcut_hints::render_interactive(
                frame,
                footer_area,
                &hints,
                theme,
                theme.surface,
                Alignment::Center,
                state,
            );
        }
        SqlEditorListMode::Search => {
            let hints = vec![
                ShortcutHint::with_keys(
                    "Enter",
                    "open",
                    [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "Esc",
                    "cancel",
                    [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
                ),
            ];
            shortcut_hints::render_interactive(
                frame,
                footer_area,
                &hints,
                theme,
                theme.surface,
                Alignment::Center,
                state,
            );
        }
        SqlEditorListMode::Rename { .. } => {
            let hints = vec![
                ShortcutHint::with_keys(
                    "Enter",
                    "save",
                    [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
                ),
                ShortcutHint::with_keys(
                    "Esc",
                    "cancel",
                    [KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
                ),
            ];
            shortcut_hints::render_interactive(
                frame,
                footer_area,
                &hints,
                theme,
                theme.surface,
                Alignment::Center,
                state,
            );
        }
        SqlEditorListMode::DeleteConfirm { .. } => {}
    }
    if let SqlEditorListMode::DeleteConfirm { .. } = &list.mode {
        let actions = dialog::render_actions(
            frame,
            Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
            &[
                dialog::DialogButton {
                    label: "Cancel",
                    tone: dialog::DialogTone::Normal,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
                dialog::DialogButton {
                    label: "Delete console",
                    tone: dialog::DialogTone::Danger,
                    emphasis: dialog::DialogEmphasis::Secondary,
                    enabled: true,
                },
            ],
            usize::from(list.delete_focus == crate::model::sql_editor_list::DeleteFocus::Delete),
            theme,
        );
        for action in actions {
            state.hit_regions.push(HitRegion {
                area: action.area,
                target: if action.index == 0 {
                    HitTarget::SqlEditorListDeleteCancel
                } else {
                    HitTarget::SqlEditorListDeleteConfirm
                },
            });
        }
        dialog::render_interactive_hint(
            frame,
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            "Tab / Left / Right switch   Enter activate   Esc cancel",
            theme,
            state,
        );
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
    app: &App,
    state: &mut UiState,
    theme: Theme,
) {
    let Some(Overlay::CatalogDropConfirm {
        plan,
        maintenance_database,
        delete_selected,
        busy,
        error,
    }) = &app.overlay
    else {
        return;
    };
    let popup = centered(
        area,
        82,
        if plan.behavior() == crate::db::catalog_drop::CatalogDropBehavior::Cascade {
            20
        } else if maintenance_database.is_some() {
            19
        } else {
            16
        },
    );
    let title = match plan.kind {
        CatalogKind::Schema
            if plan.behavior() == crate::db::catalog_drop::CatalogDropBehavior::Cascade =>
        {
            " DROP SCHEMA CASCADE ".to_owned()
        }
        CatalogKind::MaterializedView => " DROP MATERIALIZED VIEW ".to_owned(),
        CatalogKind::PrimaryKey
        | CatalogKind::UniqueConstraint
        | CatalogKind::ForeignKey
        | CatalogKind::CheckConstraint => " DROP CONSTRAINT ".to_owned(),
        _ => format!(" DROP {:?} ", plan.kind).to_uppercase(),
    };
    let inner = dialog::render_frame(frame, popup, &title, theme);
    let chunks = Layout::vertical([
        Constraint::Min(0),
        if maintenance_database.is_some() {
            Constraint::Length(2)
        } else {
            Constraint::Length(0)
        },
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(inner);
    let mut lines = vec![Line::raw("This operation will execute:"), Line::raw("")];
    if plan.behavior() == crate::db::catalog_drop::CatalogDropBehavior::Cascade {
        lines = vec![
            Line::raw("This schema contains other objects."),
            Line::raw("Continuing will delete all objects and data in this schema."),
            Line::raw("Dependent objects in other schemas may also be deleted."),
            Line::raw("This action cannot be undone."),
            Line::raw(""),
            Line::raw("This operation will execute:"),
            Line::raw(""),
        ];
    }
    lines.extend(sql_preview::lines(
        plan.sql(),
        app.sql_dialect(),
        usize::from(inner.width.saturating_sub(4)),
        theme,
    ));
    if plan.behavior() != crate::db::catalog_drop::CatalogDropBehavior::Cascade {
        lines.extend([Line::raw(""), Line::raw("This action cannot be undone.")]);
    }
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            crate::security::sanitize_terminal_text(error),
            Style::new().fg(theme.error),
        )));
    }
    dialog::render_body(frame, chunks[0], lines, theme);
    if let Some(input) = maintenance_database {
        let label = format!("Maintenance database: {}", input.value());
        frame.render_widget(Paragraph::new(label), chunks[1]);
    }
    for action in dialog::render_actions(
        frame,
        chunks[2],
        &[
            dialog::DialogButton {
                label: "Cancel",
                tone: dialog::DialogTone::Normal,
                emphasis: dialog::DialogEmphasis::Secondary,
                enabled: !busy,
            },
            dialog::DialogButton {
                label: if *busy { "Dropping..." } else { "Drop" },
                tone: dialog::DialogTone::Danger,
                emphasis: dialog::DialogEmphasis::Secondary,
                enabled: !busy,
            },
        ],
        usize::from(*delete_selected),
        theme,
    ) {
        state.hit_regions.push(HitRegion {
            area: action.area,
            target: if action.index == 0 {
                HitTarget::CatalogDropCancel
            } else {
                HitTarget::CatalogDropConfirm
            },
        });
    }
    dialog::render_interactive_hint(
        frame,
        chunks[3],
        if *busy {
            "Execution in progress"
        } else {
            "Tab / Left / Right switch   Enter activate   Esc cancel"
        },
        theme,
        state,
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
    // Keep the contextual list tall enough to expose the Explorer jump
    // shortcuts on a normal 36-row terminal. Smaller terminals still use the
    // compact clamp and retain the existing scroll behavior.
    let popup = centered(area, 74, area.height.saturating_sub(2).clamp(12, 34));
    frame.render_widget(Clear, popup);
    let title = format!(" KEYMAP // {} ", crate::help::context_name(help.context));
    let block = Block::default()
        .title(title)
        .title_top(
            Line::from(Span::styled(" Tab -> Omni ", Style::new().fg(theme.muted)))
                .alignment(Alignment::Right),
        )
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
    state.hit_regions.push(HitRegion {
        area: chunks[0],
        target: HitTarget::HelpSearch,
    });
    let toggle_label = " Tab -> Omni ";
    let toggle_width = toggle_label.chars().count() as u16;
    state.hit_regions.push(HitRegion {
        area: Rect::new(
            popup.right().saturating_sub(1).saturating_sub(toggle_width),
            popup.y,
            toggle_width.min(popup.width.saturating_sub(2)),
            1,
        ),
        target: HitTarget::HelpTogglePanel,
    });
    register_input_selection_target(
        state,
        text_selection::InputSelectionTarget::HelpSearch,
        chunks[0],
        "Search ",
        &help.query,
        text_input_horizontal_offset(chunks[0], "Search ", &help.query),
    );
    let visible_height = chunks[2].height as usize;
    state.help_viewport_rows = Some(visible_height);
    let scrollbar_track = Rect::new(
        chunks[2].right().saturating_sub(1),
        chunks[2].y,
        1,
        chunks[2].height,
    );
    let scrollbar =
        crate::ui::scrollbar::geometry(scrollbar_track, visible_height, entries.len(), help.scroll);
    let list_area = scrollbar.map_or(chunks[2], |_| {
        Rect::new(
            chunks[2].x,
            chunks[2].y,
            chunks[2].width.saturating_sub(1),
            chunks[2].height,
        )
    });
    let start = help
        .scroll
        .min(entries.len().saturating_sub(visible_height));
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
                Span::styled(
                    truncate_to_cells(
                        shortcut.description,
                        usize::from(list_area.width).saturating_sub(20),
                    ),
                    Style::new().fg(theme.text),
                ),
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
            .wrap(Wrap { trim: true }),
        list_area,
    );
    for (offset, _shortcut) in entries.iter().enumerate().skip(start).take(visible_height) {
        let row = Rect::new(
            list_area.x,
            list_area.y.saturating_add(offset as u16 - start as u16),
            list_area.width,
            1,
        );
        state.hit_regions.push(HitRegion {
            area: row,
            target: HitTarget::HelpItem(offset),
        });
    }
    if let Some(geometry) = scrollbar {
        let track = scrollbar_track;
        let before = geometry.thumb_start;
        let after = track
            .height
            .saturating_sub(before)
            .saturating_sub(geometry.thumb_length);
        let mut lines = Vec::with_capacity(track.height as usize);
        lines.push(Line::from(Span::styled("▲", Style::new().fg(theme.muted))));
        lines.extend(
            (0..before).map(|_| Line::from(Span::styled("│", Style::new().fg(theme.muted)))),
        );
        lines.extend(
            (0..geometry.thumb_length)
                .map(|_| Line::from(Span::styled("┃", Style::new().fg(theme.accent)))),
        );
        lines.extend(
            (0..after).map(|_| Line::from(Span::styled("│", Style::new().fg(theme.muted)))),
        );
        lines.push(Line::from(Span::styled("▼", Style::new().fg(theme.muted))));
        frame.render_widget(
            Paragraph::new(lines).style(Style::new().bg(theme.surface_raised)),
            track,
        );
        state.hit_regions.push(HitRegion {
            area: Rect::new(track.x, track.y + 1, 1, before),
            target: HitTarget::HelpScrollbarPage {
                offset: start.saturating_sub(visible_height),
            },
        });
        state.hit_regions.push(HitRegion {
            area: geometry.thumb_area(),
            target: HitTarget::HelpScrollbarThumb {
                track_start: geometry.rail.y,
                track_length: geometry.rail.height,
                thumb_start: geometry.thumb_start + geometry.rail.y,
                thumb_length: geometry.thumb_length,
                max_offset: geometry.max_offset,
            },
        });
        state.hit_regions.push(HitRegion {
            area: Rect::new(track.x, geometry.thumb_area().bottom(), 1, after),
            target: HitTarget::HelpScrollbarPage {
                offset: start
                    .saturating_add(visible_height)
                    .min(geometry.max_offset),
            },
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

#[allow(clippy::too_many_arguments)]
fn render_workspace_save_failed(
    frame: &mut Frame<'_>,
    area: Rect,
    revision: u64,
    message: &str,
    retryable: bool,
    focus: crate::model::workspace_save::WorkspaceSaveFocus,
    detail_scroll: usize,
    theme: Theme,
    state: &mut UiState,
) {
    let width = area.width.saturating_sub(4).min(76);
    let height = area.height.saturating_sub(2).min(16);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(" WORKSPACE NOT SAVED ")
        .title_style(theme.title(true).fg(theme.error))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.border))
        .style(Style::new().fg(theme.text).bg(theme.surface_raised));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let buttons = if retryable {
        vec![
            DialogButton {
                label: "Stay in LazyDB",
                tone: DialogTone::Normal,
                emphasis: crate::ui::dialog::DialogEmphasis::Primary,
                enabled: true,
            },
            DialogButton {
                label: "Retry save",
                tone: DialogTone::Normal,
                emphasis: crate::ui::dialog::DialogEmphasis::Secondary,
                enabled: true,
            },
            DialogButton {
                label: "Quit without saving",
                tone: DialogTone::Danger,
                emphasis: crate::ui::dialog::DialogEmphasis::Secondary,
                enabled: true,
            },
        ]
    } else {
        vec![
            DialogButton {
                label: "Stay in LazyDB",
                tone: DialogTone::Normal,
                emphasis: crate::ui::dialog::DialogEmphasis::Primary,
                enabled: true,
            },
            DialogButton {
                label: "Quit without saving",
                tone: DialogTone::Danger,
                emphasis: crate::ui::dialog::DialogEmphasis::Secondary,
                enabled: true,
            },
        ]
    };
    let action_index = focus.index(retryable);
    let action_height =
        if buttons.len() * 18 + buttons.len().saturating_sub(1) * 2 > usize::from(inner.width) {
            buttons.len() as u16
        } else {
            1
        };
    let sections = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(action_height),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Your latest workspace changes could not be saved.",
                Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
            )),
            Line::raw("Quitting may lose the latest tab layout and SQL editor changes."),
        ])
        .wrap(Wrap { trim: true }),
        sections[0],
    );
    let detail = format!("Details · Revision {revision}\n{message}");
    frame.render_widget(
        Paragraph::new(detail)
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .scroll((detail_scroll as u16, 0))
            .wrap(Wrap { trim: true }),
        sections[1],
    );
    let actions = dialog::render_actions(frame, sections[2], &buttons, action_index, theme);
    for action in actions {
        state.hit_regions.push(HitRegion {
            area: action.area,
            target: HitTarget::WorkspaceSaveAction(action.index),
        });
    }
    let hint = if retryable {
        "Tab switch   Enter activate   r retry   d quit   Esc stay"
    } else {
        "Tab switch   Enter activate   d quit   Esc stay"
    };
    frame.render_widget(
        Paragraph::new(hint).style(Style::new().fg(theme.muted)),
        sections[3],
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

/// Semantic colour for the `Users & Roles` group and its user/role children.
///
/// Uses theme tokens so custom themes and `--color=never` (all `Reset`) are
/// honoured automatically.
fn principal_node_color(kind: crate::db::principal::PrincipalDisplayKind, theme: Theme) -> Color {
    match kind {
        crate::db::principal::PrincipalDisplayKind::Group => theme.accent,
        crate::db::principal::PrincipalDisplayKind::User => theme.action,
        crate::db::principal::PrincipalDisplayKind::Role => theme.syntax_column,
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

#[cfg(test)]
mod editor_diagnostic_tests {
    use super::*;
    use crate::model::editor::{
        EditorMode, EditorPosition, EditorRenderLine, EditorRenderSnapshot, EditorRenderSpan,
        EditorViewport,
    };

    fn diagnostic(start: usize, end: usize) -> crate::sql::SqlDiagnostic {
        crate::sql::SqlDiagnostic {
            range: crate::sql::TextRange::new(start, end),
            message: "test".to_owned(),
            code: "test",
        }
    }

    #[test]
    fn diagnostic_overlap_is_based_on_full_source_ranges() {
        let diagnostics = [diagnostic(36, 46)];

        assert!(!diagnostic_covers_source(&diagnostics, 29, 36));
        assert!(!diagnostic_covers_source(&diagnostics, 46, 55));
        assert!(diagnostic_covers_source(&diagnostics, 36, 46));
        assert!(diagnostic_covers_source(&diagnostics, 40, 42));
    }

    #[test]
    fn diagnostic_overlap_does_not_mark_adjacent_ranges() {
        let diagnostics = [diagnostic(14, 26)];

        assert!(!diagnostic_covers_source(&diagnostics, 29, 36));
        assert!(diagnostic_covers_source(&diagnostics, 14, 26));
    }

    #[test]
    fn editor_render_marks_only_the_diagnosed_source_character() {
        let projection = crate::security::project_editor_line("SELECT ignore_col from sys_user");
        let line = EditorRenderLine {
            wrap_offset: 0,
            line: 2,
            display_text: projection.text.clone(),
            spans: vec![EditorRenderSpan {
                text: projection.text.clone(),
                source_start: 29,
                source_end: 29 + "SELECT ignore_col from sys_user".len(),
                kind: EditorHighlightKind::Plain,
                current_statement: false,
            }],
            source_start: 29,
            source_end: 29 + "SELECT ignore_col from sys_user".len(),
            source_byte_boundaries: projection.source_byte_boundaries,
            source_to_display_bytes: projection.source_to_display_bytes,
            source_to_display_cells: projection.source_to_display_cells,
            current_statement: false,
            statement_background_cells: None,
            selection_newline: false,
        };
        let snapshot = EditorRenderSnapshot {
            revision: 0,
            mode: EditorMode::Insert,
            first_line: 0,
            logical_line_count: 3,
            total_lines: 3,
            viewport: EditorViewport {
                width: 80,
                height: 3,
            },
            horizontal_offset: 0,
            max_line_width: line.display_text.len(),
            lines: vec![line.clone()],
            cursor: EditorPosition { line: 2, column: 0 },
            cursor_screen_cell: None,
            selections: Vec::new(),
            selection_cells: Vec::new(),
            prompt: None,
            semantic_diagnostics: vec![diagnostic(36, 46)],
        };
        let spans = editor_line_spans(&line, &snapshot, Theme::default(), false, None, &[], None);
        let error_cells = spans
            .iter()
            .flat_map(|span| {
                std::iter::repeat_n(
                    span.style.fg == Some(Theme::default().error)
                        && span.style.add_modifier.contains(Modifier::UNDERLINED),
                    span.content.as_ref().chars().count(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(error_cells, {
            let mut expected = vec![false; "SELECT ignore_col from sys_user".len()];
            expected[7..17].fill(true);
            expected
        });
    }
}
