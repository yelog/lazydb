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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSelectionTarget {
    pub session_id: uuid::Uuid,
    pub hit_maps: Vec<TextHitMap>,
}

impl TextSelectionTarget {
    pub fn source_at(&self, column: u16, row: u16) -> Option<TextPosition> {
        self.hit_maps
            .iter()
            .find_map(|hit_map| hit_map.source_at(column, row))
    }
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.right() && row >= area.y && row < area.bottom()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureOwner {
    Text,
    PaneResize,
    GridScrollbar,
    RelationColumnResize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextGesture {
    pub start: TextPosition,
    pub end: TextPosition,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::{TextHitMap, TextPosition};

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
}
