use uuid::Uuid;

use crate::db::query::QueryOutcome;
use crate::sql::{CompletionCandidate, TextRange};

use super::execution_target::ExecutionTarget;
use super::workspace::QueryStatus;
use super::{transaction::TransactionMode, transaction::TransactionState};
use crate::sql::ExecutionDraft;

use super::dashboard::DashboardTab;
use super::data_query::DataQueryOptions;
use super::data_query::DataQueryState;
use super::pagination::{PageRequest, PageSize, ResultPagination};
use super::relation::RelationTab;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabKind {
    Sql,
    Relation,
    Dashboard,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DataGridState {
    pub selected_row: usize,
    pub selected_column: usize,
    pub column_offset: usize,
    pub row_offset: usize,
    pub viewport_rows: usize,
    pub column_widths: Vec<Option<u16>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridRowTarget {
    First,
    Last,
    ViewTop,
    ViewMiddle,
    ViewBottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridColumnTarget {
    First,
    Last,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridScrollAmount {
    Lines(usize),
    HalfPage,
    Page,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridRowAlignment {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataGridViewport {
    pub tab_id: Uuid,
    pub column_offset: usize,
    pub row_offset: usize,
    pub visible_rows: usize,
}

/// Compatibility name for the shared grid state used by the current renderer.
pub type GridState = DataGridState;

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WorkspaceTab {
    Sql(ConsoleTab),
    Relation(RelationTab),
    Dashboard(DashboardTab),
}

impl WorkspaceTab {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Sql(tab) => tab.id,
            Self::Relation(tab) => tab.id,
            Self::Dashboard(tab) => tab.id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Sql(tab) => &tab.name,
            Self::Relation(tab) => tab.title(),
            Self::Dashboard(_) => "Dashboard",
        }
    }

    pub fn kind(&self) -> TabKind {
        match self {
            Self::Sql(_) => TabKind::Sql,
            Self::Relation(_) => TabKind::Relation,
            Self::Dashboard(_) => TabKind::Dashboard,
        }
    }

    pub fn as_console(&self) -> Option<&ConsoleTab> {
        match self {
            Self::Sql(tab) => Some(tab),
            Self::Relation(_) => None,
            Self::Dashboard(_) => None,
        }
    }

    pub fn as_console_mut(&mut self) -> Option<&mut ConsoleTab> {
        match self {
            Self::Sql(tab) => Some(tab),
            Self::Relation(_) => None,
            Self::Dashboard(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResultView {
    #[default]
    Data,
    Output,
    Plan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputKind {
    Info,
    Success,
    Error,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputEntry {
    pub kind: OutputKind,
    pub message: String,
    pub sql_range: Option<TextRange>,
}

impl OutputEntry {
    pub fn plain(kind: OutputKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            sql_range: None,
        }
    }

    pub fn sql(kind: OutputKind, prefix: impl Into<String>, sql: impl AsRef<str>) -> Self {
        let mut message = prefix.into();
        let start = message.len();
        message.push_str(sql.as_ref());
        let end = message.len();
        Self {
            kind,
            message,
            sql_range: Some(TextRange::new(start, end)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleTab {
    pub id: Uuid,
    pub output_editor_id: Uuid,
    pub name: String,
    pub generation: u64,
    pub query_status: QueryStatus,
    pub outcome: Option<QueryOutcome>,
    pub output: Vec<OutputEntry>,
    pub result_view: ResultView,
    pub grid: DataGridState,
    pub pagination: ResultPagination,
    pub completion: Option<CompletionPopup>,
    pub transaction_generation: u64,
    pub transaction_mode: TransactionMode,
    pub transaction_state: TransactionState,
    pub last_execution: Option<LastExecution>,
    pub execution_target: Option<ExecutionTarget>,
    pub query: DataQueryState,
    pub derived: Option<DerivedResultState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleRecord {
    pub id: Uuid,
    pub name: String,
    pub execution_target: Option<ExecutionTarget>,
    pub transaction_mode: TransactionMode,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DerivedResultState {
    pub source: LastExecution,
    pub query: DataQueryOptions,
    pub generation: u64,
    pub outcome: Option<QueryOutcome>,
    pub error: Option<String>,
    pub running: bool,
    pub pagination: ResultPagination,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionResult {
    Dispatched,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastExecution {
    pub draft: ExecutionDraft,
    pub result: ExecutionResult,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionPopup {
    pub candidates: Vec<CompletionCandidate>,
    pub selected: usize,
}

impl ConsoleTab {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            output_editor_id: Uuid::new_v4(),
            name: name.into(),
            generation: 0,
            query_status: QueryStatus::Idle,
            outcome: None,
            output: Vec::new(),
            result_view: ResultView::Data,
            grid: DataGridState::default(),
            pagination: default_pagination(),
            completion: None,
            transaction_generation: 0,
            transaction_mode: TransactionMode::Auto,
            transaction_state: TransactionState::Idle,
            last_execution: None,
            execution_target: None,
            query: DataQueryState::default(),
            derived: None,
        }
    }
}

fn default_pagination() -> ResultPagination {
    ResultPagination::from_page(PageRequest::first(PageSize::default()), 0)
}

impl DataGridState {
    pub fn select_column_target(&mut self, target: GridColumnTarget, column_count: usize) {
        if column_count == 0 {
            self.selected_column = 0;
            return;
        }

        self.selected_column = match target {
            GridColumnTarget::First => 0,
            GridColumnTarget::Last => column_count - 1,
        };
    }

    pub fn clamp(&mut self, row_count: usize, column_count: usize) {
        self.selected_row = self.selected_row.min(row_count.saturating_sub(1));
        self.selected_column = self.selected_column.min(column_count.saturating_sub(1));
        self.column_offset = self.column_offset.min(column_count.saturating_sub(1));
        self.row_offset = self.row_offset.min(row_count.saturating_sub(1));
        if self.selected_row < self.row_offset {
            self.row_offset = self.selected_row;
        }
        self.column_widths.truncate(column_count);
    }

    pub fn ensure_row_visible(&mut self, row_count: usize) {
        self.selected_row = self.selected_row.min(row_count.saturating_sub(1));
        self.row_offset = self.row_offset.min(row_count.saturating_sub(1));
        if row_count == 0 {
            self.row_offset = 0;
            return;
        }
        let visible_rows = self.viewport_rows;
        if visible_rows == 0 {
            return;
        }
        if self.selected_row < self.row_offset {
            self.row_offset = self.selected_row;
        } else if self.selected_row >= self.row_offset.saturating_add(visible_rows) {
            self.row_offset = self.selected_row + 1 - visible_rows;
        }
        self.row_offset = self
            .row_offset
            .min(row_count.saturating_sub(visible_rows.min(row_count)));
    }

    pub fn select_row_target(&mut self, target: GridRowTarget, row_count: usize) {
        if row_count == 0 {
            self.selected_row = 0;
            self.row_offset = 0;
            return;
        }

        match target {
            GridRowTarget::First => {
                self.selected_row = 0;
                self.row_offset = 0;
            }
            GridRowTarget::Last => {
                self.selected_row = row_count - 1;
                self.ensure_row_visible(row_count);
            }
            GridRowTarget::ViewTop | GridRowTarget::ViewMiddle | GridRowTarget::ViewBottom => {
                if self.viewport_rows == 0 {
                    return;
                }
                let first = self.row_offset.min(row_count - 1);
                let last = first
                    .saturating_add(self.viewport_rows.saturating_sub(1))
                    .min(row_count - 1);
                self.selected_row = match target {
                    GridRowTarget::ViewTop => first,
                    GridRowTarget::ViewMiddle => first + (last - first) / 2,
                    GridRowTarget::ViewBottom => last,
                    GridRowTarget::First | GridRowTarget::Last => unreachable!(),
                };
            }
        }
    }

    pub fn scroll_rows(&mut self, direction: isize, amount: GridScrollAmount, row_count: usize) {
        if row_count == 0 {
            self.selected_row = 0;
            self.row_offset = 0;
            return;
        }
        if self.viewport_rows == 0 || direction == 0 {
            return;
        }

        let step = match amount {
            GridScrollAmount::Lines(lines) => {
                let delta = scroll_delta(direction, lines);
                self.row_offset = move_bounded(
                    self.row_offset,
                    delta,
                    self.max_row_offset(row_count).saturating_add(1),
                );
                let last_visible = self
                    .row_offset
                    .saturating_add(self.viewport_rows.saturating_sub(1))
                    .min(row_count - 1);
                self.selected_row = self.selected_row.clamp(self.row_offset, last_visible);
                return;
            }
            GridScrollAmount::HalfPage => (self.viewport_rows / 2).max(1),
            GridScrollAmount::Page => self.viewport_rows,
        };
        let delta = scroll_delta(direction, step);
        self.selected_row = move_bounded(self.selected_row, delta, row_count);
        self.row_offset = move_bounded(
            self.row_offset,
            delta,
            self.max_row_offset(row_count).saturating_add(1),
        );
        self.ensure_row_visible(row_count);
    }

    pub fn align_selected_row(&mut self, alignment: GridRowAlignment, row_count: usize) {
        if row_count == 0 {
            self.selected_row = 0;
            self.row_offset = 0;
            return;
        }
        if self.viewport_rows == 0 {
            return;
        }

        self.selected_row = self.selected_row.min(row_count - 1);
        let screen_row = match alignment {
            GridRowAlignment::Top => 0,
            GridRowAlignment::Middle => self.viewport_rows.saturating_sub(1) / 2,
            GridRowAlignment::Bottom => self.viewport_rows.saturating_sub(1),
        };
        self.row_offset = self
            .selected_row
            .saturating_sub(screen_row)
            .min(self.max_row_offset(row_count));
    }

    fn max_row_offset(&self, row_count: usize) -> usize {
        row_count.saturating_sub(self.viewport_rows.min(row_count))
    }
}

fn scroll_delta(direction: isize, step: usize) -> isize {
    if direction.is_negative() {
        -(step.min(isize::MAX as usize) as isize)
    } else {
        step.min(isize::MAX as usize) as isize
    }
}

fn move_bounded(current: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        current
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DataGridState, GridColumnTarget, GridRowAlignment, GridRowTarget, GridScrollAmount,
        OutputEntry, OutputKind,
    };

    #[test]
    fn output_sql_entry_records_only_the_sql_suffix() {
        let entry = OutputEntry::sql(
            OutputKind::Success,
            "[2026-08-31] database> ",
            "SELECT 'Ada'",
        );
        let range = entry.sql_range.unwrap();

        assert_eq!(&entry.message[range.start..range.end], "SELECT 'Ada'");
    }

    #[test]
    fn clamping_keeps_selection_and_widths_inside_result_dimensions() {
        let mut state = DataGridState {
            selected_row: 9,
            selected_column: 8,
            column_offset: 7,
            row_offset: 8,
            viewport_rows: 5,
            column_widths: vec![Some(10), Some(11), Some(12), Some(13)],
        };

        state.clamp(2, 3);

        assert_eq!(state.selected_row, 1);
        assert_eq!(state.selected_column, 2);
        assert_eq!(state.column_offset, 2);
        assert_eq!(state.row_offset, 1);
        assert_eq!(state.column_widths, vec![Some(10), Some(11), Some(12)]);
    }

    #[test]
    fn column_targets_preserve_row_and_viewport_state() {
        let mut state = DataGridState {
            selected_row: 4,
            selected_column: 2,
            column_offset: 1,
            row_offset: 3,
            viewport_rows: 5,
            column_widths: vec![Some(10), None, Some(20)],
        };

        state.select_column_target(GridColumnTarget::First, 5);
        assert_eq!(state.selected_column, 0);
        assert_eq!(
            (state.selected_row, state.column_offset, state.row_offset),
            (4, 1, 3)
        );

        state.select_column_target(GridColumnTarget::Last, 5);
        assert_eq!(state.selected_column, 4);
        assert_eq!(
            (state.selected_row, state.column_offset, state.row_offset),
            (4, 1, 3)
        );
    }

    #[test]
    fn column_targets_are_safe_without_columns() {
        let mut state = DataGridState {
            selected_column: 3,
            ..DataGridState::default()
        };

        state.select_column_target(GridColumnTarget::Last, 0);

        assert_eq!(state.selected_column, 0);
    }

    #[test]
    fn clamping_empty_dimensions_resets_selection() {
        let mut state = DataGridState {
            selected_row: 3,
            selected_column: 4,
            column_offset: 2,
            row_offset: 3,
            viewport_rows: 5,
            column_widths: vec![Some(10)],
        };

        state.clamp(0, 0);

        assert_eq!(state.selected_row, 0);
        assert_eq!(state.selected_column, 0);
        assert_eq!(state.column_offset, 0);
        assert_eq!(state.row_offset, 0);
        assert!(state.column_widths.is_empty());
    }

    #[test]
    fn ensure_row_visible_scrolls_in_both_directions() {
        let mut state = DataGridState {
            selected_row: 0,
            selected_column: 0,
            column_offset: 0,
            row_offset: 0,
            viewport_rows: 5,
            column_widths: vec![None],
        };

        state.selected_row = 6;
        state.ensure_row_visible(10);
        assert_eq!(state.row_offset, 2);

        state.selected_row = 1;
        state.ensure_row_visible(10);
        assert_eq!(state.row_offset, 1);
    }

    #[test]
    fn row_viewport_scrolls_only_after_selection_crosses_an_edge() {
        let mut state = DataGridState {
            selected_row: 1,
            row_offset: 0,
            viewport_rows: 3,
            ..DataGridState::default()
        };

        state.selected_row = 2;
        state.ensure_row_visible(10);
        assert_eq!(state.row_offset, 0);

        state.selected_row = 3;
        state.ensure_row_visible(10);
        assert_eq!(state.row_offset, 1);

        state.selected_row = 1;
        state.ensure_row_visible(10);
        assert_eq!(state.row_offset, 1);

        state.selected_row = 0;
        state.ensure_row_visible(10);
        assert_eq!(state.row_offset, 0);
    }

    #[test]
    fn ensuring_row_visibility_does_not_clamp_columns_from_width_overrides() {
        let mut state = DataGridState {
            selected_column: 5,
            column_offset: 4,
            selected_row: 6,
            row_offset: 0,
            viewport_rows: 5,
            column_widths: Vec::new(),
        };

        state.ensure_row_visible(10);

        assert_eq!(state.selected_column, 5);
        assert_eq!(state.column_offset, 4);
        assert_eq!(state.row_offset, 2);
    }

    #[test]
    fn semantic_row_targets_use_absolute_and_visible_bounds() {
        let base = DataGridState {
            selected_row: 6,
            row_offset: 4,
            viewport_rows: 5,
            ..DataGridState::default()
        };
        let cases = [
            (GridRowTarget::First, 0, 0),
            (GridRowTarget::Last, 11, 7),
            (GridRowTarget::ViewTop, 4, 4),
            (GridRowTarget::ViewMiddle, 6, 4),
            (GridRowTarget::ViewBottom, 8, 4),
        ];

        for (target, selected, offset) in cases {
            let mut state = base.clone();
            state.select_row_target(target, 12);
            assert_eq!((state.selected_row, state.row_offset), (selected, offset));
        }

        let mut partial = DataGridState {
            row_offset: 7,
            viewport_rows: 5,
            ..DataGridState::default()
        };
        partial.select_row_target(GridRowTarget::ViewMiddle, 10);
        assert_eq!(partial.selected_row, 8);
        partial.select_row_target(GridRowTarget::ViewBottom, 10);
        assert_eq!(partial.selected_row, 9);
    }

    #[test]
    fn page_scroll_moves_selection_and_viewport_together() {
        let mut state = DataGridState {
            selected_row: 6,
            row_offset: 4,
            viewport_rows: 5,
            ..DataGridState::default()
        };

        state.scroll_rows(1, GridScrollAmount::HalfPage, 20);
        assert_eq!((state.selected_row, state.row_offset), (8, 6));
        state.scroll_rows(-1, GridScrollAmount::Page, 20);
        assert_eq!((state.selected_row, state.row_offset), (3, 1));

        state.scroll_rows(-1, GridScrollAmount::Page, 20);
        assert_eq!((state.selected_row, state.row_offset), (0, 0));
        state.scroll_rows(-1, GridScrollAmount::Page, 20);
        assert_eq!((state.selected_row, state.row_offset), (0, 0));

        state.selected_row = 18;
        state.row_offset = 14;
        state.scroll_rows(1, GridScrollAmount::Page, 20);
        assert_eq!((state.selected_row, state.row_offset), (19, 15));
        state.scroll_rows(1, GridScrollAmount::Page, 20);
        assert_eq!((state.selected_row, state.row_offset), (19, 15));
    }

    #[test]
    fn line_scroll_moves_selection_and_viewport_immediately_with_bounds() {
        let mut state = DataGridState {
            selected_row: 2,
            row_offset: 0,
            viewport_rows: 5,
            ..DataGridState::default()
        };

        state.scroll_rows(1, GridScrollAmount::Lines(1), 10);
        assert_eq!((state.selected_row, state.row_offset), (2, 1));

        state.scroll_rows(1, GridScrollAmount::Lines(1), 10);
        assert_eq!((state.selected_row, state.row_offset), (2, 2));

        state.scroll_rows(1, GridScrollAmount::Lines(1), 10);
        assert_eq!((state.selected_row, state.row_offset), (3, 3));

        state.scroll_rows(1, GridScrollAmount::Lines(20), 10);
        assert_eq!((state.selected_row, state.row_offset), (5, 5));

        state.scroll_rows(1, GridScrollAmount::Lines(1), 10);
        assert_eq!((state.selected_row, state.row_offset), (5, 5));

        state.scroll_rows(-1, GridScrollAmount::Lines(20), 10);
        assert_eq!((state.selected_row, state.row_offset), (4, 0));
    }

    #[test]
    fn half_page_has_a_minimum_step_of_one() {
        let mut state = DataGridState {
            viewport_rows: 1,
            ..DataGridState::default()
        };

        state.scroll_rows(1, GridScrollAmount::HalfPage, 3);

        assert_eq!((state.selected_row, state.row_offset), (1, 1));
    }

    #[test]
    fn selected_row_can_be_aligned_within_the_viewport() {
        for (alignment, offset) in [
            (GridRowAlignment::Top, 10),
            (GridRowAlignment::Middle, 8),
            (GridRowAlignment::Bottom, 6),
        ] {
            let mut state = DataGridState {
                selected_row: 10,
                viewport_rows: 5,
                ..DataGridState::default()
            };
            state.align_selected_row(alignment, 30);
            assert_eq!((state.selected_row, state.row_offset), (10, offset));
        }

        let mut at_end = DataGridState {
            selected_row: 29,
            viewport_rows: 5,
            ..DataGridState::default()
        };
        at_end.align_selected_row(GridRowAlignment::Top, 30);
        assert_eq!((at_end.selected_row, at_end.row_offset), (29, 25));
    }

    #[test]
    fn empty_and_unknown_viewports_are_safe() {
        let mut empty = DataGridState {
            selected_row: 3,
            row_offset: 2,
            viewport_rows: 5,
            ..DataGridState::default()
        };
        empty.select_row_target(GridRowTarget::Last, 0);
        assert_eq!((empty.selected_row, empty.row_offset), (0, 0));

        let mut unknown = DataGridState {
            selected_row: 3,
            row_offset: 2,
            viewport_rows: 0,
            ..DataGridState::default()
        };
        unknown.scroll_rows(1, GridScrollAmount::Page, 10);
        unknown.align_selected_row(GridRowAlignment::Middle, 10);
        unknown.select_row_target(GridRowTarget::ViewBottom, 10);
        assert_eq!((unknown.selected_row, unknown.row_offset), (3, 2));
    }
}
