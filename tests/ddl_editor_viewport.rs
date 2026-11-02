//! Regression tests for the DDL scroll ceiling (yelog/lazydb#45).
//!
//! The DDL pane is drawn inside the workspace content area, so its editor
//! viewport is smaller than the terminal: the workspace tab bar, the footer,
//! the DATA/DDL (or OVERVIEW/DDL) selector and both panel borders are not part
//! of it, and a DDL status row takes two more. Only the render knows that rect,
//! so it publishes the viewport in `UiState` and the runtime records it as the
//! editor session viewport. Every scroll path (Ctrl-d, PageDown, the wheel,
//! `G`) clamps against it, so a viewport larger than the pane makes the tail of
//! the DDL unreachable.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::Action,
    app::App,
    db::{
        ServerInfo,
        catalog::{
            CatalogCompleteness, CatalogCount, CatalogEntry, CatalogId, CatalogKind, CatalogPage,
            CatalogRequestKey, CatalogTarget, DdlProvenance, OptionalMetadata, QualifiedName,
            RelationDdl,
        },
        principal::{PrincipalDdl, PrincipalEntry, PrincipalId, PrincipalKind, PrincipalScope},
    },
    identity::ConnectionIdentity,
    model::{
        editor::EditorViewport,
        pagination::PageRequest,
        principal::{PrincipalDdlRequest, PrincipalView},
        relation::{
            RelationDescriptor, RelationKey, RelationLoad, RelationRequest, RelationRequestKind,
            RelationSnapshot, RelationTab, RelationView,
        },
        tab::WorkspaceTab,
        workspace::Focus,
    },
    profile::{DatabaseKind, import_connection_url},
    ui::{self, HitTarget, UiState},
};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;
/// `HEIGHT` minus the workspace tab bar (2), the footer (1), the view selector
/// (2) and both panel borders (2).
const DDL_EDITOR_HEIGHT: usize = 33;
/// Rows reserved by a DDL status row, which the layout takes from the editor.
const DDL_STATUS_ROWS: usize = 2;
const DDL_LINES: usize = 100;
const LAST_LINE: &str = "-- LINE 100";

fn app_with_connection() -> (App, ConnectionIdentity) {
    let profile = import_connection_url("sqlite::memory:", Some("orbital-lab"))
        .expect("sqlite profile")
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Sqlite,
            version: "3.50.0".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    let connection = app
        .connection
        .active_identity()
        .expect("active connection identity");
    (app, connection)
}

fn ddl_text() -> String {
    (1..=DDL_LINES)
        .map(|line| format!("-- LINE {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn catalog_entry(
    connection: ConnectionIdentity,
    object: &str,
) -> (CatalogEntry, QualifiedName, CatalogId) {
    let profile_id = connection.profile_id;
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, ["db", "main"]);
    let relation_id = CatalogId::new(profile_id, CatalogKind::Table, ["db", "main", object]);
    let qualified_name = QualifiedName {
        database: None,
        schema: None,
        object: object.into(),
    };
    let entry = CatalogEntry::relation(
        relation_id.clone(),
        schema_id,
        qualified_name.clone(),
        "table",
        OptionalMetadata::Unsupported,
        false,
    )
    .expect("relation catalog entry");
    (entry, qualified_name, relation_id)
}

fn relation_ddl(
    connection: ConnectionIdentity,
    sql: &str,
) -> (RelationDescriptor, Box<RelationDdl>, RelationRequestKind) {
    let (entry, qualified_name, relation_id) = catalog_entry(connection, "users");
    let children = CatalogPage {
        key: CatalogRequestKey {
            connection,
            catalog_epoch: 0,
            request_id: 1,
            target: CatalogTarget::RelationChildren {
                relation: relation_id.clone(),
            },
            cursor: None,
        },
        entries: Vec::new(),
        group_summaries: Vec::new(),
        total_count: CatalogCount::Exact(0),
        next_cursor: None,
        completeness: CatalogCompleteness::Complete,
    };
    let descriptor = RelationDescriptor {
        key: RelationKey {
            profile_id: connection.profile_id,
            object_id: relation_id,
        },
        qualified_name,
        kind: CatalogKind::Table,
        title: "users".into(),
    };
    let ddl = RelationDdl {
        relation: entry,
        children,
        sql: sql.to_owned(),
        provenance: DdlProvenance::NativeCatalog,
    };
    (descriptor, Box::new(ddl), RelationRequestKind::Ddl)
}

/// Relation tab in the DDL view whose read-only editor holds the DDL text.
///
/// `with_status` keeps the tab in the `Refreshing` state while the previous DDL
/// stays on screen, which is the layout that reserves the status rows.
fn relation_app(with_status: bool) -> (App, Uuid, Uuid) {
    relation_app_with_sql(with_status, &ddl_text())
}

fn relation_app_with_sql(with_status: bool, sql: &str) -> (App, Uuid, Uuid) {
    let (mut app, connection) = app_with_connection();
    let (descriptor, ddl, kind) = relation_ddl(connection, sql);
    let mut tab = RelationTab::with_descriptor(descriptor, RelationView::Ddl);
    let request = RelationRequest {
        tab_id: tab.id,
        console_id: tab.id,
        tab_generation: tab.generation,
        request_id: tab.next_request_id,
        connection,
        relation: tab.descriptor.key.clone(),
        kind,
        scope: app.profiles.first().expect("profile").catalog_scope.clone(),
        options: Default::default(),
        page: PageRequest::first(Default::default()),
    };
    tab.ddl = RelationLoad::Loading {
        request: request.clone(),
        previous: None,
    };
    let tab_id = tab.id;
    let session_id = tab.ddl_editor_id;
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    app.update(Action::RelationSucceeded {
        request: request.clone(),
        snapshot: Box::new(RelationSnapshot::Ddl(ddl)),
    });
    // Relation editors are initialized lazily by the runtime viewport action.
    app.update(Action::DdlEditorViewportChanged {
        session_id,
        viewport: EditorViewport {
            width: 1,
            height: 1,
        },
    });
    if with_status {
        let previous = match &mut app.tabs[app.active_tab] {
            WorkspaceTab::Relation(tab) => {
                match std::mem::replace(&mut tab.ddl, RelationLoad::Empty) {
                    RelationLoad::Ready(snapshot) => Some(snapshot),
                    other => {
                        tab.ddl = other;
                        None
                    }
                }
            }
            _ => unreachable!(),
        };
        if let WorkspaceTab::Relation(tab) = &mut app.tabs[app.active_tab] {
            tab.ddl = RelationLoad::Loading { request, previous };
        }
    }
    (app, tab_id, session_id)
}

fn principal_entry(connection: ConnectionIdentity) -> PrincipalEntry {
    PrincipalEntry {
        id: PrincipalId {
            profile_id: connection.profile_id,
            scope: PrincipalScope::Cluster,
            native_id: "10".to_owned(),
            host: None,
        },
        kind: PrincipalKind::User,
        name: "alice".to_owned(),
        native_kind: "login_role".to_owned(),
        system: false,
    }
}

/// Principal (user/role) tab in the DDL view whose editor holds the DDL text.
fn principal_app(with_status: bool) -> (App, Uuid) {
    principal_app_with_sql(with_status, &ddl_text())
}

fn principal_app_with_sql(with_status: bool, sql: &str) -> (App, Uuid) {
    let (mut app, connection) = app_with_connection();
    let entry = principal_entry(connection);
    app.update(Action::OpenPrincipal {
        profile_id: connection.profile_id,
        entry: entry.clone(),
    });
    let index = app
        .tabs
        .iter()
        .position(|tab| matches!(tab, WorkspaceTab::PrincipalDdl(_)))
        .expect("principal tab");
    let (tab_id, tab_generation) = match &app.tabs[index] {
        WorkspaceTab::PrincipalDdl(tab) => (tab.id, tab.generation),
        _ => unreachable!(),
    };
    let request = PrincipalDdlRequest {
        tab_id,
        tab_generation,
        request_id: 1,
        connection,
        entry: entry.clone(),
    };
    if let WorkspaceTab::PrincipalDdl(tab) = &mut app.tabs[index] {
        tab.begin_load(request.clone());
        tab.view = PrincipalView::Ddl;
    }
    app.active_tab = index;
    app.focus = Focus::Results;
    app.update(Action::PrincipalDdlLoaded {
        request,
        ddl: PrincipalDdl {
            principal: entry,
            sql: sql.to_owned(),
        },
    });
    if with_status {
        let request = PrincipalDdlRequest {
            tab_id,
            tab_generation,
            request_id: 2,
            connection,
            entry: principal_entry(connection),
        };
        if let WorkspaceTab::PrincipalDdl(tab) = &mut app.tabs[index] {
            tab.begin_load(request);
        }
    }
    (app, tab_id)
}

fn render(app: &App) -> (ratatui::buffer::Buffer, UiState) {
    let backend = TestBackend::new(WIDTH, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state_using_icons(frame, app, &mut state, Default::default()))
        .expect("render");
    (terminal.backend().buffer().clone(), state)
}

fn screen_text(buffer: &ratatui::buffer::Buffer) -> String {
    let mut output = String::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    output
}

fn pane_width(state: &UiState) -> usize {
    state
        .hit_regions
        .iter()
        .filter(|region| region.target == HitTarget::Focus(Focus::Results))
        .map(|region| usize::from(region.area.width))
        .max()
        .expect("DDL pane focus region")
}

fn published_viewport(state: &UiState) -> (Uuid, EditorViewport) {
    state
        .ddl_editor_viewport
        .expect("the DDL render publishes the editor viewport it drew into")
}

fn record_viewport(app: &mut App, state: &UiState) {
    let (session_id, viewport) = published_viewport(state);
    app.update(Action::DdlEditorViewportChanged {
        session_id,
        viewport,
    });
}

/// Scrolls the DDL editor to the end and returns the screen and the state of
/// the frame drawn with the recorded viewport.
fn scroll_to_end(app: &mut App, session_id: Uuid) -> (String, UiState) {
    app.update(Action::EditorScrollBy {
        session_id,
        rows: DDL_LINES as isize * 2,
        columns: 0,
    });
    let (buffer, state) = render(app);
    (screen_text(&buffer), state)
}

fn assert_vertical_thumb_reaches_the_end(state: &UiState, session_id: Uuid) {
    let (offset, max_offset) = state
        .hit_regions
        .iter()
        .find_map(|region| match region.target {
            HitTarget::EditorScrollbarThumb {
                session_id: id,
                vertical: true,
                offset,
                max_offset,
                ..
            } if id == session_id => Some((offset, max_offset)),
            _ => None,
        })
        .expect("vertical DDL scrollbar thumb");
    assert_eq!(
        offset, max_offset,
        "the scrollbar thumb must reach the end of its track"
    );
}

fn ddl_app(principal: bool, with_status: bool, sql: &str) -> (App, Uuid) {
    if principal {
        let (app, _) = principal_app_with_sql(with_status, sql);
        let WorkspaceTab::PrincipalDdl(tab) = &app.tabs[app.active_tab] else {
            unreachable!()
        };
        let session_id = tab.editor_id;
        (app, session_id)
    } else {
        let (app, _, session_id) = relation_app_with_sql(with_status, sql);
        (app, session_id)
    }
}

#[test]
fn ddl_horizontal_scroll_reaches_the_final_column_with_line_numbers() {
    let sql = (1..=DDL_LINES)
        .map(|line| format!("-- LINE {line} {} LAST_COLUMN", "x".repeat(180)))
        .collect::<Vec<_>>()
        .join("\n");
    for principal in [false, true] {
        for with_status in [false, true] {
            let (mut app, session_id) = ddl_app(principal, with_status, &sql);
            let (_, state) = render(&app);
            let (_, viewport) = published_viewport(&state);
            let body = state.text_selection_targets.last().unwrap();
            assert_eq!(body.session_id, session_id);
            assert_eq!(viewport.width, usize::from(body.hit_maps[0].area.width));
            record_viewport(&mut app, &state);
            app.update(Action::EditorScrollBy {
                session_id,
                rows: 0,
                columns: 1_000,
            });
            let (buffer, state) = render(&app);
            let screen = screen_text(&buffer);
            assert!(screen.contains("LAST_COLUMN"), "{screen}");
            let (offset, max_offset) = state
                .hit_regions
                .iter()
                .find_map(|region| match region.target {
                    HitTarget::EditorScrollbarThumb {
                        session_id: id,
                        vertical: false,
                        offset,
                        max_offset,
                        ..
                    } if id == session_id => Some((offset, max_offset)),
                    _ => None,
                })
                .expect("horizontal DDL scrollbar");
            assert_eq!(offset, max_offset);
        }
    }
}

#[test]
fn ddl_search_prompt_reserves_a_row_and_keeps_the_last_line_visible() {
    for principal in [false, true] {
        for with_status in [false, true] {
            let (mut app, session_id) = ddl_app(principal, with_status, &ddl_text());
            let (_, state) = render(&app);
            let (_, without_prompt) = published_viewport(&state);
            record_viewport(&mut app, &state);
            app.update(Action::ReadOnlyEditorKey {
                session_id,
                event: KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            });
            let (_, state) = render(&app);
            let (_, with_prompt) = published_viewport(&state);
            assert_eq!(with_prompt.height + 1, without_prompt.height);
            assert_eq!(with_prompt.width, without_prompt.width);
            record_viewport(&mut app, &state);
            let (screen, state) = scroll_to_end(&mut app, session_id);
            assert!(screen.contains(LAST_LINE), "{screen}");
            assert_vertical_thumb_reaches_the_end(&state, session_id);
            let body = state.text_selection_targets.last().unwrap();
            let last_line = body.hit_maps.last().expect("visible DDL text");
            assert_eq!(last_line.line, DDL_LINES - 1);
            assert!(last_line.area.y < state.cursor.expect("search prompt cursor").position.y);

            app.update(Action::ReadOnlyEditorKey {
                session_id,
                event: KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            });
            let (_, state) = render(&app);
            assert_eq!(published_viewport(&state).1, without_prompt);
            record_viewport(&mut app, &state);
            assert!(scroll_to_end(&mut app, session_id).0.contains(LAST_LINE));
        }
    }
}

#[test]
fn relation_ddl_viewport_is_the_pane_and_reaches_the_last_line() {
    let (mut app, _, session_id) = relation_app(false);
    let (_, state) = render(&app);

    let (published_session, viewport) = published_viewport(&state);
    assert_eq!(published_session, session_id);
    assert_eq!(viewport.height, DDL_EDITOR_HEIGHT);
    assert_eq!(viewport.width, pane_width(&state) - 2 - 4);
    assert_ne!(
        viewport.height,
        usize::from(HEIGHT) - 2,
        "the recorded viewport must not be derived from the terminal size"
    );

    record_viewport(&mut app, &state);
    let (screen, state) = scroll_to_end(&mut app, session_id);

    assert!(screen.contains(LAST_LINE), "{screen}");
    assert_vertical_thumb_reaches_the_end(&state, session_id);
}

#[test]
fn relation_ddl_status_row_shrinks_the_viewport_and_still_reaches_the_last_line() {
    let (mut app, _, session_id) = relation_app(true);
    let (_, state) = render(&app);

    let (published_session, viewport) = published_viewport(&state);
    assert_eq!(published_session, session_id);
    assert_eq!(viewport.height, DDL_EDITOR_HEIGHT - DDL_STATUS_ROWS);

    record_viewport(&mut app, &state);
    let (screen, state) = scroll_to_end(&mut app, session_id);

    assert!(screen.contains("Refreshing"), "{screen}");
    assert!(screen.contains(LAST_LINE), "{screen}");
    assert_vertical_thumb_reaches_the_end(&state, session_id);
}

#[test]
fn principal_ddl_viewport_is_the_pane_and_reaches_the_last_line() {
    let (mut app, _) = principal_app(false);
    let (_, state) = render(&app);

    let (session_id, viewport) = published_viewport(&state);
    assert_eq!(viewport.height, DDL_EDITOR_HEIGHT);
    assert_eq!(viewport.width, pane_width(&state) - 2 - 4);

    record_viewport(&mut app, &state);
    let (screen, state) = scroll_to_end(&mut app, session_id);

    assert!(screen.contains(LAST_LINE), "{screen}");
    assert_vertical_thumb_reaches_the_end(&state, session_id);
}

#[test]
fn principal_ddl_status_row_shrinks_the_viewport_and_still_reaches_the_last_line() {
    let (mut app, _) = principal_app(true);
    let (_, state) = render(&app);

    let (session_id, viewport) = published_viewport(&state);
    assert_eq!(viewport.height, DDL_EDITOR_HEIGHT - DDL_STATUS_ROWS);

    record_viewport(&mut app, &state);
    let (screen, state) = scroll_to_end(&mut app, session_id);

    assert!(screen.contains(LAST_LINE), "{screen}");
    assert_vertical_thumb_reaches_the_end(&state, session_id);
}
