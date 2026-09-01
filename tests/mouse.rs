use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        catalog::{CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName},
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        value::CellValue,
    },
    input::mouse::map_mouse,
    model::{
        explorer::{ExplorerNodeId, ExplorerScrollAmount},
        profile_manager::ProfileField,
        tab::GridScrollAmount,
        workspace::{Focus, Overlay},
    },
    profile::{DatabaseKind, import_connection_url},
    ui::{self, HitRegion, HitTarget, ProfileButton, UiState},
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use uuid::Uuid;

#[test]
fn maps_tabs_tree_rows_and_result_cells_from_rendered_hit_regions() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewConsole);
    let connection_id = Uuid::new_v4();
    app.connection.profile_id = Some(connection_id);
    app.connection.generation = 1;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    let database = CatalogEntry::database(
        CatalogId::new(connection_id, CatalogKind::Database, ["demo"]),
        QualifiedName {
            database: Some("demo".into()),
            schema: None,
            object: "demo".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    app.explorer.normalized.add_profile(connection_id);
    app.explorer
        .normalized
        .profiles
        .get_mut(&connection_id)
        .unwrap()
        .catalog
        .insert(database)
        .unwrap();
    app.explorer.rebuild_projection(connection_id);
    let tab_id = app.active_console().id;
    let generation = app.active_console().generation;
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection: app.connection.active_identity().unwrap(),
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
    let mut ui_state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))
        .unwrap();

    assert_click_maps(&ui_state, &app, &HitTarget::Tab(0), Action::ActivateTab(0));
    assert_click_maps(
        &ui_state,
        &app,
        &HitTarget::HeaderProfile,
        Action::ExplorerSelect(ExplorerNodeId::Profile(connection_id)),
    );
    let row_id = lazydb::model::explorer::ExplorerNodeId::Catalog(CatalogId::new(
        connection_id,
        CatalogKind::Database,
        ["demo"],
    ));
    assert_click_maps(
        &ui_state,
        &app,
        &HitTarget::ExplorerRow(row_id.clone()),
        Action::ExplorerSelect(row_id.clone()),
    );
    let action = click_action(&ui_state, &app, &HitTarget::ExplorerRow(row_id));
    app.update(action);
    assert_eq!(
        app.explorer.selected_id(),
        Some(&lazydb::model::explorer::ExplorerNodeId::Catalog(
            CatalogId::new(connection_id, CatalogKind::Database, ["demo"]),
        ))
    );
    assert_click_maps(
        &ui_state,
        &app,
        &HitTarget::ResultCell { row: 0, column: 0 },
        Action::GridSelect { row: 0, column: 0 },
    );

    assert!(matches!(
        app.update(Action::NewConsole).as_slice(),
        [Command::PersistWorkspace(_)]
    ));
}

#[test]
fn relation_view_and_retry_hit_targets_emit_semantic_actions() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Explorer;
    let mut ui = UiState::new();
    ui.hit_regions.extend([
        HitRegion {
            area: Rect::new(1, 1, 6, 1),
            target: HitTarget::RelationView(lazydb::model::relation::RelationView::Data),
        },
        HitRegion {
            area: Rect::new(1, 2, 12, 1),
            target: HitTarget::RelationRetry,
        },
    ]);

    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            &ui,
            &app
        ),
        Some(Action::SetRelationView(
            lazydb::model::relation::RelationView::Data
        ))
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 2),
            &ui,
            &app
        ),
        Some(Action::RefreshActiveRelation)
    );
}

#[test]
fn result_view_tabs_emit_explicit_view_actions() {
    let app = App::new(Vec::new());
    let mut ui = UiState::new();
    ui.hit_regions.extend([
        HitRegion {
            area: Rect::new(1, 1, 6, 1),
            target: HitTarget::ResultView(lazydb::model::tab::ResultView::Data),
        },
        HitRegion {
            area: Rect::new(8, 1, 8, 1),
            target: HitTarget::ResultView(lazydb::model::tab::ResultView::Output),
        },
    ]);

    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            &ui,
            &app
        ),
        Some(Action::SetResultView(lazydb::model::tab::ResultView::Data))
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 8, 1),
            &ui,
            &app
        ),
        Some(Action::SetResultView(
            lazydb::model::tab::ResultView::Output
        ))
    );
}

#[test]
fn relation_result_cell_mouse_action_updates_relation_grid() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    let mut ui = UiState::new();
    ui.hit_regions.push(HitRegion {
        area: Rect::new(2, 2, 8, 1),
        target: HitTarget::ResultCell { row: 2, column: 3 },
    });
    let action = map_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 2, 2),
        &ui,
        &app,
    );
    assert_eq!(action, Some(Action::GridSelect { row: 2, column: 3 }));
    app.update(action.unwrap());
    assert_eq!(app.focus, Focus::Results);
    let lazydb::model::tab::WorkspaceTab::Relation(tab) = &app.tabs[1] else {
        panic!()
    };
    assert_eq!((tab.grid.selected_row, tab.grid.selected_column), (0, 0));
}

#[test]
fn relation_pane_background_click_focuses_results() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Explorer;
    let mut ui = UiState::new();
    ui.hit_regions.push(HitRegion {
        area: Rect::new(10, 2, 40, 20),
        target: HitTarget::Focus(Focus::Results),
    });

    let action = map_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 20, 10),
        &ui,
        &app,
    );

    assert_eq!(action, Some(Action::Focus(Focus::Results)));
    app.update(action.unwrap());
    assert_eq!(app.focus, Focus::Results);
}

#[test]
fn secondary_click_on_profile_root_edits_that_stable_id() {
    let profile = import_connection_url(":memory:", Some("root"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let app = App::new(vec![profile]);
    let mut ui = UiState::new();
    ui.hit_regions.push(HitRegion {
        area: Rect::new(1, 1, 20, 1),
        target: HitTarget::ExplorerRow(ExplorerNodeId::Profile(profile_id)),
    });

    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Right), 1, 1),
            &ui,
            &app
        ),
        Some(Action::ProfileStartEdit { profile_id })
    );
}

#[test]
fn double_click_same_explorer_node_uses_primary_action_without_sleeping() {
    let id = ExplorerNodeId::Profile(Uuid::new_v4());
    let ui = UiState::new();
    let first = Instant::now();
    assert!(!ui.track_explorer_click(&id, first));
    assert!(ui.track_explorer_click(&id, first + Duration::from_millis(499)));
    assert!(!ui.track_explorer_click(&id, first + Duration::from_millis(1000)));
    assert!(!ui.track_explorer_click(
        &ExplorerNodeId::EmptyProfiles,
        first + Duration::from_millis(1001)
    ));
}

#[test]
fn non_explorer_mouse_down_clears_explorer_double_click_tracker() {
    let id = ExplorerNodeId::Profile(Uuid::new_v4());
    let mut ui = UiState::new();
    ui.hit_regions.extend([
        HitRegion {
            area: Rect::new(1, 1, 10, 1),
            target: HitTarget::ExplorerRow(id.clone()),
        },
        HitRegion {
            area: Rect::new(1, 2, 10, 1),
            target: HitTarget::Tab(0),
        },
    ]);
    let app = App::new(Vec::new());
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            &ui,
            &app
        ),
        Some(Action::ExplorerSelect(id))
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 2),
            &ui,
            &app
        ),
        Some(Action::ActivateTab(0))
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            &ui,
            &app
        ),
        Some(Action::ExplorerSelect(ExplorerNodeId::Profile(
            ui.hit_regions
                .first()
                .and_then(|region| match &region.target {
                    HitTarget::ExplorerRow(ExplorerNodeId::Profile(id)) => Some(*id),
                    _ => None,
                })
                .unwrap(),
        )))
    );
}

#[test]
fn maps_profile_fields_toggles_and_buttons() {
    let profiles = ["first", "second", "third"]
        .into_iter()
        .map(|name| {
            import_connection_url(":memory:", Some(name))
                .unwrap()
                .profile
        })
        .collect();
    let mut app = App::new(profiles);
    app.update(Action::OpenProfileManager);
    let mut ui = UiState::new();
    let targets = [
        HitTarget::ProfileField(ProfileField::Name),
        HitTarget::ProfileDriver(DatabaseKind::MySql),
        HitTarget::ProfileToggle(ProfileField::ReadOnly),
        HitTarget::ProfileButton(ProfileButton::Save),
        HitTarget::ProfileButton(ProfileButton::ConfirmDelete),
    ];
    for (row, target) in targets.into_iter().enumerate() {
        ui.hit_regions.push(HitRegion {
            area: Rect::new(2, row as u16 + 2, 12, 1),
            target,
        });
    }
    ui.hit_regions.push(HitRegion {
        area: Rect::new(2, 9, 12, 1),
        target: HitTarget::HeaderProfile,
    });

    assert_click_maps(
        &ui,
        &app,
        &HitTarget::ProfileDriver(DatabaseKind::MySql),
        Action::ProfileSelectDriver(DatabaseKind::MySql),
    );
    assert_click_maps(
        &ui,
        &app,
        &HitTarget::ProfileField(ProfileField::Name),
        Action::ProfileFocusField(ProfileField::Name),
    );
    assert_click_maps(
        &ui,
        &app,
        &HitTarget::ProfileToggle(ProfileField::ReadOnly),
        Action::ProfileToggleField(ProfileField::ReadOnly),
    );
    assert_click_maps(
        &ui,
        &app,
        &HitTarget::ProfileButton(ProfileButton::Save),
        Action::ProfileSave { connect: false },
    );
    assert_click_maps(
        &ui,
        &app,
        &HitTarget::ProfileButton(ProfileButton::ConfirmDelete),
        Action::ProfileConfirmDelete,
    );

    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 2, 9),
            &ui,
            &app,
        ),
        None
    );
}

#[test]
fn rendered_driver_options_select_exact_kinds_and_are_disabled_while_busy() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();

    for kind in [
        DatabaseKind::Postgres,
        DatabaseKind::MySql,
        DatabaseKind::Sqlite,
    ] {
        assert_click_maps(
            &state,
            &app,
            &HitTarget::ProfileDriver(kind),
            Action::ProfileSelectDriver(kind),
        );
    }

    app.profile_manager.as_mut().unwrap().operation =
        Some(lazydb::model::profile_manager::ProfileOperation::Testing);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(
        !state
            .hit_regions
            .iter()
            .any(|region| matches!(region.target, HitTarget::ProfileDriver(_)))
    );
}

#[test]
fn editor_mouse_scroll_is_a_viewport_action() {
    let app = App::new(Vec::new());
    let mut state = UiState::new();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    let action = map_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 40,
            row: 8,
            modifiers: KeyModifiers::NONE,
        },
        &state,
        &app,
    );
    assert_eq!(
        action,
        Some(Action::EditorScroll {
            rows: 3,
            columns: 0
        })
    );
}

#[test]
fn grid_and_explorer_mouse_wheels_scroll_the_viewport_immediately() {
    let app = App::new(Vec::new());
    let mut ui = UiState::new();
    ui.hit_regions.extend([
        HitRegion {
            area: Rect::new(0, 2, 20, 20),
            target: HitTarget::Focus(Focus::Explorer),
        },
        HitRegion {
            area: Rect::new(20, 2, 60, 20),
            target: HitTarget::Focus(Focus::Results),
        },
    ]);

    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollDown, 10, 10), &ui, &app),
        Some(Action::ExplorerScrollNodes {
            direction: 1,
            amount: ExplorerScrollAmount::Lines(3),
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollUp, 10, 10), &ui, &app),
        Some(Action::ExplorerScrollNodes {
            direction: -1,
            amount: ExplorerScrollAmount::Lines(3),
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollDown, 40, 10), &ui, &app),
        Some(Action::GridScrollRows {
            direction: 1,
            amount: GridScrollAmount::Lines(3),
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollUp, 40, 10), &ui, &app),
        Some(Action::GridScrollRows {
            direction: -1,
            amount: GridScrollAmount::Lines(3),
        })
    );
}

#[test]
fn editor_horizontal_mouse_scroll_is_a_viewport_action() {
    let app = App::new(Vec::new());
    let mut state = UiState::new();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();

    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollLeft, 40, 8), &state, &app),
        Some(Action::EditorScroll {
            rows: 0,
            columns: -3
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollRight, 40, 8), &state, &app),
        Some(Action::EditorScroll {
            rows: 0,
            columns: 3
        })
    );
}

#[test]
fn result_horizontal_mouse_scroll_moves_the_grid_viewport() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    let mut ui = UiState::new();
    ui.hit_regions.push(HitRegion {
        area: Rect::new(10, 2, 40, 20),
        target: HitTarget::Focus(Focus::Results),
    });
    ui.grid_horizontal_scroll = Some(lazydb::ui::GridHorizontalScrollTargets {
        left: lazydb::ui::GridHorizontalScrollTarget {
            offset: 0,
            first_visible: 0,
            last_visible: 2,
        },
        right: lazydb::ui::GridHorizontalScrollTarget {
            offset: 1,
            first_visible: 1,
            last_visible: 3,
        },
    });

    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollLeft, 20, 10), &ui, &app),
        Some(Action::GridScrollColumns {
            offset: 0,
            first_visible: 0,
            last_visible: 2,
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollRight, 20, 10), &ui, &app),
        Some(Action::GridScrollColumns {
            offset: 1,
            first_visible: 1,
            last_visible: 3,
        })
    );
}

#[test]
fn ddl_view_mouse_scroll_is_vertical_ddl_scroll() {
    let mut app = App::new(Vec::new());
    app.tabs.push(lazydb::model::tab::WorkspaceTab::Relation(
        lazydb::model::relation::RelationTab::new("users"),
    ));
    app.active_tab = 1;
    app.focus = Focus::Results;
    app.update(Action::SetRelationView(
        lazydb::model::relation::RelationView::Ddl,
    ));
    let ui = UiState::new();
    let session_id = match app.tabs.get(app.active_tab) {
        Some(lazydb::model::tab::WorkspaceTab::Relation(tab)) => tab.ddl_editor_id,
        _ => panic!("expected relation tab"),
    };

    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollDown, 40, 8), &ui, &app),
        Some(Action::ReadOnlyEditorScroll {
            session_id,
            rows: 3,
            columns: 0
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollUp, 40, 8), &ui, &app),
        Some(Action::ReadOnlyEditorScroll {
            session_id,
            rows: -3,
            columns: 0
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollLeft, 40, 8), &ui, &app),
        Some(Action::ReadOnlyEditorScroll {
            session_id,
            rows: 0,
            columns: -3
        })
    );
    assert_eq!(
        map_mouse(mouse(MouseEventKind::ScrollRight, 40, 8), &ui, &app),
        Some(Action::ReadOnlyEditorScroll {
            session_id,
            rows: 0,
            columns: 3
        })
    );
}

#[test]
fn help_and_message_overlays_block_background_mouse_input() {
    let mut app = App::new(Vec::new());
    let mut ui = UiState::new();
    ui.hit_regions.extend([
        HitRegion {
            area: Rect::new(0, 0, 10, 1),
            target: HitTarget::HeaderProfile,
        },
        HitRegion {
            area: Rect::new(0, 1, 10, 4),
            target: HitTarget::ResultCell { row: 0, column: 0 },
        },
    ]);

    for overlay in [
        Overlay::Help(lazydb::help::HelpState::new(
            lazydb::help::ShortcutContext::EditorNormal,
            lazydb::help::ShortcutCapabilities::default(),
        )),
        Overlay::Message {
            title: "Notice".into(),
            body: "Body".into(),
        },
    ] {
        app.overlay = Some(overlay);
        assert_eq!(
            map_mouse(
                mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
                &ui,
                &app,
            ),
            None
        );
        assert_eq!(
            map_mouse(mouse(MouseEventKind::ScrollDown, 0, 1), &ui, &app),
            None
        );
        assert_eq!(
            map_mouse(mouse(MouseEventKind::ScrollRight, 0, 1), &ui, &app),
            None
        );
    }
}

#[test]
fn header_profile_hit_region_uses_terminal_display_width() {
    for (name, expected_width) in [("数据库", 6), ("ｶﾞ", 2), ("\u{7}", 6)] {
        let profile = import_connection_url(":memory:", Some(name))
            .unwrap()
            .profile;
        let profile_id = profile.id;
        let mut app = App::new(vec![profile]);
        app.connection.profile_id = Some(profile_id);
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut ui_state = UiState::new();
        terminal
            .draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))
            .unwrap();

        let region = ui_state
            .hit_regions
            .iter()
            .find(|region| region.target == HitTarget::HeaderProfile)
            .unwrap();
        assert_eq!(region.area.width, expected_width, "{name}");
    }
}

#[test]
fn horizontal_scrollbar_track_click_sets_page_offset() {
    let app = App::new(Vec::new());
    let mut ui = UiState::new();
    ui.hit_regions.push(HitRegion {
        area: Rect::new(20, 10, 8, 1),
        target: HitTarget::GridScrollbarPage { offset: 6 },
    });

    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 22, 10),
            &ui,
            &app,
        ),
        Some(Action::GridSetColumnOffset { offset: 6 })
    );
}

#[test]
fn pagination_mouse_targets_emit_page_actions_and_open_size_selector() {
    let app = App::new(Vec::new());
    let mut ui = UiState::new();
    ui.hit_regions.extend([
        HitRegion {
            area: Rect::new(1, 1, 2, 1),
            target: HitTarget::ResultFirstPage,
        },
        HitRegion {
            area: Rect::new(4, 1, 1, 1),
            target: HitTarget::ResultPageSize,
        },
        HitRegion {
            area: Rect::new(6, 1, 1, 1),
            target: HitTarget::ResultNextPage,
        },
    ]);
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            &ui,
            &app
        ),
        Some(Action::ResultFirstPage)
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 4, 1),
            &ui,
            &app
        ),
        Some(Action::OpenPageSizeSelector { relation: false })
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 6, 1),
            &ui,
            &app
        ),
        Some(Action::ResultNextPage)
    );
}

#[test]
fn horizontal_scrollbar_thumb_drag_maps_to_column_offsets() {
    let app = App::new(Vec::new());
    let mut ui = UiState::new();
    ui.hit_regions.push(HitRegion {
        area: Rect::new(12, 10, 4, 1),
        target: HitTarget::GridScrollbarThumb {
            track_x: 10,
            track_width: 20,
            thumb_x: 12,
            thumb_width: 4,
            offset: 2,
            max_offset: 16,
        },
    });

    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 13, 10),
            &ui,
            &app,
        ),
        Some(Action::GridSetColumnOffset { offset: 2 })
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Drag(MouseButton::Left), 29, 10),
            &ui,
            &app,
        ),
        Some(Action::GridSetColumnOffset { offset: 16 })
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Drag(MouseButton::Left), 10, 10),
            &ui,
            &app,
        ),
        Some(Action::GridSetColumnOffset { offset: 0 })
    );
    assert_eq!(
        map_mouse(
            mouse(MouseEventKind::Up(MouseButton::Left), 10, 10),
            &ui,
            &app,
        ),
        Some(Action::GridEndColumnResize)
    );
    assert!(ui.grid_scrollbar_drag.borrow().is_none());
}

fn assert_click_maps(ui: &UiState, app: &App, target: &HitTarget, expected: Action) {
    let region = ui
        .hit_regions
        .iter()
        .find(|region| &region.target == target)
        .unwrap();
    let event = mouse(
        MouseEventKind::Down(MouseButton::Left),
        region.area.x,
        region.area.y,
    );
    assert_eq!(map_mouse(event, ui, app), Some(expected));
}

fn click_action(ui: &UiState, app: &App, target: &HitTarget) -> Action {
    ui.clear_click_tracker();
    let region = ui
        .hit_regions
        .iter()
        .find(|region| &region.target == target)
        .unwrap();
    map_mouse(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            region.area.x,
            region.area.y,
        ),
        ui,
        app,
    )
    .unwrap()
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}
