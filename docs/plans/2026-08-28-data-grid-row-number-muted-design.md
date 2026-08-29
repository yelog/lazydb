# Data Grid Row Number Muted Design

**Goal:** Keep the data grid's left row-number column visually secondary by rendering its text in the theme's muted color, including when a row is selected.

**Approach:** Update `body_cells` in `src/ui/data_grid.rs` so the row-number cell gets its own foreground style instead of inheriting the optional selected-row style. Preserve the selected-row background by taking the background from the row style when present, while forcing the foreground to `theme.muted`. Data cells remain unchanged.

**Verification:** Add a focused unit test for the row-number style behavior and run the data-grid tests plus the full Rust test suite if practical.
