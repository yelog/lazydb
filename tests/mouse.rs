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
    model::{
        profile_manager::ProfileField,
        workspace::{Focus, Overlay},
    },
    profile::import_connection_url,
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
    let mut ui_state = UiState::new(true);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))
        .unwrap();

    assert_click_maps(&ui_state, &app, &HitTarget::Tab(0), Action::ActivateTab(0));
    assert_click_maps(
        &ui_state,
        &app,
        &HitTarget::HeaderProfile,
        Action::OpenProfileManager,
    );
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

#[test]
fn maps_profile_rows_fields_toggles_buttons_and_scroll() {
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
    app.profile_manager.as_mut().unwrap().selected = 1;
    let mut ui = UiState::new(true);
    let targets = [
        HitTarget::ProfileRow(0),
        HitTarget::ProfileField(ProfileField::Name),
        HitTarget::ProfileToggle(ProfileField::ReadOnly),
        HitTarget::ProfileButton(ProfileButton::New),
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
        &HitTarget::ProfileRow(0),
        Action::ProfileMove(-1),
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
        &HitTarget::ProfileButton(ProfileButton::New),
        Action::ProfileStartNew,
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

    let list_region = ui
        .hit_regions
        .iter()
        .find(|region| region.target == HitTarget::ProfileRow(0))
        .unwrap();
    assert_eq!(
        map_mouse(
            mouse(
                MouseEventKind::ScrollDown,
                list_region.area.x,
                list_region.area.y,
            ),
            &ui,
            &app,
        ),
        Some(Action::ProfileMove(3))
    );
    assert_eq!(
        map_mouse(
            mouse(
                MouseEventKind::ScrollUp,
                list_region.area.x,
                list_region.area.y,
            ),
            &ui,
            &app,
        ),
        Some(Action::ProfileMove(-3))
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
fn editor_mouse_scroll_is_a_viewport_action() {
    let app = App::new(Vec::new());
    let mut state = UiState::new(true);
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
fn help_and_message_overlays_block_background_mouse_input() {
    let mut app = App::new(Vec::new());
    let mut ui = UiState::new(true);
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
        Overlay::Help(Focus::Editor),
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
        let mut ui_state = UiState::new(true);
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

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}
