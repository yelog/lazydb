use ratatui::layout::Rect;

/// A source position in a rendered text line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextPosition {
    pub line: usize,
    pub column: usize,
}

/// Geometry for one visible text source. Coordinates are terminal cells.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextHitMap {
    pub area: Rect,
    pub line: usize,
    pub source_to_display_cells: Vec<usize>,
    pub horizontal_offset: usize,
}

impl TextHitMap {
    pub fn source_at(&self, column: u16, row: u16) -> Option<TextPosition> {
        if !contains(self.area, column, row) {
            return None;
        }

        let display_cell = self
            .horizontal_offset
            .saturating_add(usize::from(column.saturating_sub(self.area.x)));
        let source_column = self
            .source_to_display_cells
            .partition_point(|&boundary| boundary <= display_cell)
            .saturating_sub(1)
            .min(self.source_to_display_cells.len().saturating_sub(1));
        Some(TextPosition {
            line: self.line,
            column: source_column,
        })
    }

    pub fn source_at_horizontal_clamped(&self, column: u16) -> TextPosition {
        let display_cell = self
            .horizontal_offset
            .saturating_add(usize::from(column.saturating_sub(self.area.x)));
        let source_column = self
            .source_to_display_cells
            .partition_point(|&boundary| boundary <= display_cell)
            .saturating_sub(1)
            .min(self.source_to_display_cells.len().saturating_sub(1));
        TextPosition {
            line: self.line,
            column: source_column,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSelectionTarget {
    pub session_id: uuid::Uuid,
    pub hit_maps: Vec<TextHitMap>,
}

/// Geometry for a single-line editable input. Boundaries are source character
/// boundaries expressed in terminal cells before the visible horizontal offset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputHitMap {
    pub area: Rect,
    pub source_to_display_cells: Vec<usize>,
    pub horizontal_offset: usize,
    pub prefix_width: usize,
    pub source_start: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputSelectionTarget {
    DataQuery(crate::model::data_query::DataQueryInput),
    RedisValueFilter(uuid::Uuid),
    Profile(crate::ui::ProfileField),
    ProfileUrl,
    Catalog(crate::action::CatalogEditorCursorTarget),
    RelationTemporal {
        tab_id: uuid::Uuid,
        row_id: crate::model::relation_edit::EditableRowId,
        column: usize,
    },
    RelationJson {
        tab_id: uuid::Uuid,
        row_id: crate::model::relation_edit::EditableRowId,
        column: usize,
    },
    RelationText {
        tab_id: uuid::Uuid,
        row_id: crate::model::relation_edit::EditableRowId,
        column: usize,
    },
    ConsoleManagerSearch,
    ConsoleManagerRename,
    HelpSearch,
    ProfileGroupName,
    ExplorerFind,
    ExplorerSearch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputGesture {
    pub target: InputSelectionTarget,
    pub hit_map: InputHitMap,
    pub start: usize,
    pub end: usize,
    pub has_dragged: bool,
}

impl InputHitMap {
    pub fn source_at(&self, column: u16, row: u16) -> Option<usize> {
        if !contains(self.area, column, row) {
            return None;
        }
        if usize::from(column.saturating_sub(self.area.x)) < self.prefix_width {
            return None;
        }
        let display_cell = self.horizontal_offset.saturating_add(
            usize::from(column.saturating_sub(self.area.x)).saturating_sub(self.prefix_width),
        );
        let boundary = self
            .source_to_display_cells
            .partition_point(|&boundary| boundary <= display_cell)
            .saturating_sub(1);
        Some(self.source_start + boundary.min(self.source_to_display_cells.len().saturating_sub(1)))
    }

    pub fn source_at_clamped(&self, column: u16, row: u16) -> Option<usize> {
        if row < self.area.y || row >= self.area.bottom() || self.source_to_display_cells.is_empty()
        {
            return None;
        }
        let relative = usize::from(column.saturating_sub(self.area.x));
        if relative < self.prefix_width {
            return Some(self.source_start);
        }
        let display = self
            .horizontal_offset
            .saturating_add(relative - self.prefix_width);
        Some(
            self.source_start
                + self
                    .source_to_display_cells
                    .partition_point(|&boundary| boundary <= display)
                    .saturating_sub(1)
                    .min(self.source_to_display_cells.len().saturating_sub(1)),
        )
    }

    /// Map a pointer that may be outside this line's horizontal bounds.
    /// Vertical bounds are handled by the caller, which can select the first
    /// or last visible line for multiline inputs.
    pub fn source_at_horizontal_clamped(&self, column: u16) -> Option<usize> {
        if self.source_to_display_cells.is_empty() {
            return None;
        }
        let relative = usize::from(column.saturating_sub(self.area.x));
        if relative < self.prefix_width {
            return Some(self.source_start);
        }
        let display = self
            .horizontal_offset
            .saturating_add(relative - self.prefix_width);
        Some(
            self.source_start
                + self
                    .source_to_display_cells
                    .partition_point(|&boundary| boundary <= display)
                    .saturating_sub(1)
                    .min(self.source_to_display_cells.len().saturating_sub(1)),
        )
    }
}

impl TextSelectionTarget {
    pub fn source_at(&self, column: u16, row: u16) -> Option<TextPosition> {
        self.hit_maps
            .iter()
            .find_map(|hit_map| hit_map.source_at(column, row))
    }

    /// Resolve a pointer that may sit outside the visible lines.
    ///
    /// A drag that leaves the text keeps selecting the nearest visible line
    /// instead of cancelling.
    pub fn position_clamped(&self, column: u16, row: u16) -> Option<TextPosition> {
        let first = self.hit_maps.iter().min_by_key(|map| map.area.y)?;
        let last = self.hit_maps.iter().max_by_key(|map| map.area.bottom())?;
        let line = self
            .hit_maps
            .iter()
            .find(|map| row >= map.area.y && row < map.area.bottom())
            .unwrap_or(if row < first.area.y { first } else { last });
        Some(line.source_at_horizontal_clamped(column))
    }

    /// Rows an editor must scroll so a drag that left the visible text keeps
    /// extending the selection.
    ///
    /// Zero while the pointer is inside the text. The further the pointer is
    /// from the edge it left, the faster the editor scrolls, so a drag that
    /// only just leaves the pane stays precise.
    pub fn drag_scroll_rows(&self, row: u16) -> isize {
        let Some(first) = self.hit_maps.iter().map(|map| map.area.y).min() else {
            return 0;
        };
        let Some(last) = self.hit_maps.iter().map(|map| map.area.bottom()).max() else {
            return 0;
        };
        let (distance, direction) = if row < first {
            (first.saturating_sub(row), -1)
        } else if row >= last {
            (row.saturating_sub(last).saturating_add(1), 1)
        } else {
            return 0;
        };
        let rows = 1 + isize::try_from(distance.saturating_sub(1).min(3)).unwrap_or(3);
        direction * rows
    }
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.right() && row >= area.y && row < area.bottom()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureOwner {
    Text,
    Input,
    PaneResize,
    GridScrollbar,
    RelationColumnResize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextGestureSource {
    Editor,
    TextDetail,
    SqlHistory,
    TransactionReview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextGesture {
    pub session_id: uuid::Uuid,
    pub source: TextGestureSource,
    pub start: TextPosition,
    pub end: TextPosition,
    pub revision: u64,
    pub has_dragged: bool,
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::{InputHitMap, TextHitMap, TextPosition};

    fn map(source_to_display_cells: &[usize], width: u16, offset: usize) -> TextHitMap {
        TextHitMap {
            area: Rect::new(10, 4, width, 1),
            line: 7,
            source_to_display_cells: source_to_display_cells.to_vec(),
            horizontal_offset: offset,
        }
    }

    #[test]
    fn text_hit_maps_display_cells_to_source() {
        let ascii = map(&[0, 1, 2, 3], 3, 0);
        assert_eq!(
            ascii.source_at(10, 4),
            Some(TextPosition { line: 7, column: 0 })
        );
        assert_eq!(
            ascii.source_at(12, 4),
            Some(TextPosition { line: 7, column: 2 })
        );

        let wide = map(&[0, 2, 3], 3, 0);
        assert_eq!(
            wide.source_at(10, 4),
            Some(TextPosition { line: 7, column: 0 })
        );
        assert_eq!(
            wide.source_at(11, 4),
            Some(TextPosition { line: 7, column: 0 })
        );
        assert_eq!(
            wide.source_at(12, 4),
            Some(TextPosition { line: 7, column: 1 })
        );

        let offset = map(&[0, 1, 3, 4], 3, 2);
        assert_eq!(
            offset.source_at(10, 4),
            Some(TextPosition { line: 7, column: 1 })
        );
        assert_eq!(
            offset.source_at(12, 4),
            Some(TextPosition { line: 7, column: 3 })
        );
    }

    #[test]
    fn text_hit_maps_empty_lines_line_end_and_outside() {
        let empty = map(&[0], 4, 0);
        assert_eq!(
            empty.source_at(10, 4),
            Some(TextPosition { line: 7, column: 0 })
        );

        let line_end = map(&[0, 1, 2], 4, 0);
        assert_eq!(
            line_end.source_at(13, 4),
            Some(TextPosition { line: 7, column: 2 })
        );
        assert_eq!(line_end.source_at(9, 4), None);
        assert_eq!(line_end.source_at(10, 5), None);
    }

    #[test]
    fn large_hit_map_uses_binary_search_without_copying_source_text() {
        let boundaries: Vec<_> = (0..100_001).collect();
        let hit_map = TextHitMap {
            area: Rect::new(0, 0, 80, 1),
            line: 0,
            source_to_display_cells: boundaries,
            horizontal_offset: 50_000,
        };

        assert_eq!(
            hit_map.source_at(0, 0),
            Some(TextPosition {
                line: 0,
                column: 50_000
            })
        );
        assert_eq!(
            hit_map.source_at(79, 0),
            Some(TextPosition {
                line: 0,
                column: 50_079
            })
        );
    }

    #[test]
    fn input_hit_map_excludes_prefix_and_maps_visible_boundaries() {
        let input = InputHitMap {
            area: Rect::new(10, 4, 12, 1),
            source_to_display_cells: vec![0, 1, 2, 3, 4],
            horizontal_offset: 0,
            prefix_width: 3,
            source_start: 0,
        };

        assert_eq!(input.source_at(12, 4), None);
        assert_eq!(input.source_at(13, 4), Some(0));
        assert_eq!(input.source_at(16, 4), Some(3));
    }

    #[test]
    fn input_hit_map_clamps_horizontal_pointer_to_source_boundaries() {
        let input = InputHitMap {
            area: Rect::new(10, 4, 4, 1),
            source_to_display_cells: vec![0, 1, 2, 3],
            horizontal_offset: 0,
            prefix_width: 0,
            source_start: 7,
        };

        assert_eq!(input.source_at_horizontal_clamped(0), Some(7));
        assert_eq!(input.source_at_horizontal_clamped(99), Some(10));
    }
}
