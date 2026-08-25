use std::time::Duration;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        catalog::{CatalogId, CatalogKind, CatalogNode},
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        value::CellValue,
    },
    input::mouse::map_mouse,
    ui::{self, HitTarget, UiState},
};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

#[test]
fn maps_tabs_tree_rows_and_result_cells_from_rendered_hit_regions() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewConsole);
    let connection_id = Uuid::new_v4();
    app.explorer.set_nodes(vec![CatalogNode::new(
        CatalogId::new(connection_id, CatalogKind::Database, ["demo"]),
        None,
        "demo",
        "database",
        None,
        true,
    )]);
    let tab_id = app.active_console().id;
    let generation = app.active_console().generation;
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        outcome: QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![ColumnMeta {
                    name: "id".into(),
                    type_name: "INTEGER".into(),
                }],
                rows: vec![vec![CellValue::Integer(1)]],
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::ZERO, Duration::ZERO, 1),
        },
    });

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut ui_state = UiState::new(true);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))
        .unwrap();

    assert_click_maps(&ui_state, &app, &HitTarget::Tab(0), Action::ActivateTab(0));
    assert_click_maps(
        &ui_state,
        &app,
        &HitTarget::ExplorerRow(0),
        Action::ExplorerSelect(0),
    );
    assert_click_maps(
        &ui_state,
        &app,
        &HitTarget::ResultCell { row: 0, column: 0 },
        Action::GridSelect { row: 0, column: 0 },
    );

    assert!(matches!(
        app.update(Action::NewConsole).as_slice(),
        [Command::PersistWorkspace]
    ));
}

fn assert_click_maps(ui: &UiState, app: &App, target: &HitTarget, expected: Action) {
    let region = ui
        .hit_regions
        .iter()
        .find(|region| &region.target == target)
        .unwrap();
    let event = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: region.area.x,
        row: region.area.y,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(map_mouse(event, ui, app), Some(expected));
}
