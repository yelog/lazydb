use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::model::profile_group::ProfileGroupOverlay;
use lazydb::{
    action::Action,
    app::App,
    cli::ConfirmationPolicy,
    db::{
        ServerInfo,
        catalog::{
            CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry, CatalogId, CatalogKind,
            CatalogMetadata, CatalogPage, CatalogRequestKey, CatalogTarget, ColumnMetadata,
            DdlProvenance, ObjectGroup, OptionalMetadata, QualifiedName, RelationDdl,
        },
        catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationExecutionMode, CatalogMutationMode,
            CatalogMutationRequest, CatalogObjectType, CatalogSelectionHint,
        },
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{
        catalog_editor::{
            CatalogEditorOperation, CatalogEditorPage, CatalogEditorState, CatalogMutationOption,
        },
        data_query::{DataQueryCandidate, DataQueryCompletion, DataQueryInput},
        execution_target::ExecutionTarget,
        explorer::{
            CatalogGroupState, ExplorerLoadState, ExplorerNodeId, ExplorerOwnerId, ProfilePlacement,
        },
        profile_manager::{ProfileField, ProfileManagerPage, ProfileOperation},
        relation::RelationTab,
        tab::WorkspaceTab,
        tab::{CompletionPopup, ResultView},
        transaction::TransactionMode,
        workspace::{ConnectionStatus, Focus, Overlay, PaneSplit, QueryStatus},
    },
    persistence::secrets::keyring_ref,
    profile::{DatabaseKind, Environment, import_connection_url},
    sql::{CompletionCandidate, CompletionKind, CompletionScore, TextRange},
    ui::{
        self, HitTarget, PaneResizeDrag, ProfileButton, UiState,
        icons::{IconMode, IconSet},
    },
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};

fn fixture() -> App {
    let profile = import_connection_url("sqlite::memory:", Some("orbital-lab"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Sqlite,
            version: "3.50.0".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ReplaceEditor(
        "SELECT id, name, active\nFROM users\nWHERE active = true;".into(),
    ));
    let tab_id = app.active_console().id;
    let generation = app.active_console().generation;
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection: app.connection.active_identity().unwrap(),
        outcome: QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![
                    ColumnMeta {
                        name: "id".into(),
                        type_name: "INTEGER".into(),
                    },
                    ColumnMeta {
                        name: "name".into(),
                        type_name: "TEXT".into(),
                    },
                    ColumnMeta {
                        name: "active".into(),
                        type_name: "BOOLEAN".into(),
                    },
                ],
                rows: vec![vec![
                    CellValue::Integer(42),
                    CellValue::Text("Ada".into()),
                    CellValue::Boolean(true),
                ]],
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::from_millis(24), Duration::from_millis(352), 1),
        },
    });
    app
}

#[test]
fn transaction_menu_renders_state_aware_disabled_reasons() {
    let mut app = fixture();
    app.update(Action::OpenTransactionMenu);
    let output = render(&app, 100, 30);
    assert!(output.contains("TRANSACTION MODE"));
    assert!(output.contains("no active transaction"));

    app.update(Action::CancelTransactionMenu);
    app.active_console_mut().query_status = QueryStatus::Running;
    app.update(Action::OpenTransactionMenu);
    let output = render(&app, 100, 30);
    assert!(output.contains("query running"));
}

#[test]
fn transaction_menu_selected_row_uses_readable_selection_style() {
    let mut app = fixture();
    app.update(Action::OpenTransactionMenu);
    let (buffer, _) = render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let x = find_ascii_cells(&buffer, 12, "Auto").expect("selected transaction mode");
    let cell = &buffer[(x, 12)];
    assert_eq!(cell.bg, Color::Rgb(26, 55, 70));
    assert!(cell.modifier.contains(Modifier::BOLD));
    assert_ne!(cell.fg, Color::Rgb(105, 126, 146));
}

#[test]
fn record_view_renders_the_selected_row_as_ordered_fields() {
    let mut app = fixture();
    app.focus = Focus::Results;
    app.update(Action::OpenRecordView);

    let output = render(&app, 100, 30);

    assert!(output.contains("RECORD VIEW"));
    assert!(output.contains("ROW 1 / 1"));
    assert!(output.contains("id"));
    assert!(output.contains("INTEGER"));
    assert!(output.contains("42"));
    assert!(output.contains("name"));
    assert!(output.contains("Ada"));
    assert!(output.contains("active"));
    assert!(output.contains("BOOLEAN"));
}

#[test]
fn dashboard_renders_all_pages_without_database_io() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.tabs.push(WorkspaceTab::Dashboard(
        lazydb::model::dashboard::DashboardTab::new(),
    ));
    app.active_tab = 0;
    app.focus = Focus::Results;

    assert!(render(&app, 120, 36).contains("Transactions"));
    app.update(Action::DashboardSetPage(
        lazydb::model::dashboard::DashboardPage::Processes,
    ));
    assert!(render(&app, 120, 36).contains("ProcessList"));
    app.update(Action::DashboardSetPage(
        lazydb::model::dashboard::DashboardPage::Charts,
    ));
    assert!(render(&app, 120, 36).contains("Charts will appear"));
}

#[test]
fn sql_server_dashboard_entry_is_disabled_without_opening_a_tab() {
    let profile = import_connection_url("sqlserver://db.example.test/app", Some("mssql"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.connection.profile_id = Some(profile.id);
    app.connection.server = Some(ServerInfo {
        kind: DatabaseKind::SqlServer,
        version: "16.0".into(),
        database: "app".into(),
        current_user: None,
    });
    app.active_workspace_profile = Some(profile.id);

    assert!(!app.dashboard_supported());
    assert!(app.update(Action::OpenDashboard).is_empty());
    assert!(
        !app.tabs
            .iter()
            .any(|tab| matches!(tab, WorkspaceTab::Dashboard(_)))
    );
    assert!(render(&app, 120, 36).contains("Dashboard"));
}

#[test]
fn dashboard_renders_real_chart_series_after_two_samples() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    let mut dashboard = lazydb::model::dashboard::DashboardTab::new();
    dashboard.page = lazydb::model::dashboard::DashboardPage::Charts;
    dashboard.history.push(
        lazydb::model::dashboard::RawSample::new(1_000, 1)
            .with(lazydb::model::dashboard::MetricKey::Commits, 10.0)
            .with(lazydb::model::dashboard::MetricKey::Selects, 100.0)
            .with(lazydb::model::dashboard::MetricKey::Inserts, 5.0),
    );
    dashboard.history.push(
        lazydb::model::dashboard::RawSample::new(3_000, 1)
            .with(lazydb::model::dashboard::MetricKey::Commits, 16.0)
            .with(lazydb::model::dashboard::MetricKey::Selects, 140.0)
            .with(lazydb::model::dashboard::MetricKey::Inserts, 9.0),
    );
    app.tabs.push(WorkspaceTab::Dashboard(dashboard));
    app.active_tab = 0;
    app.focus = Focus::Results;

    let output = render(&app, 140, 40);
    assert!(output.contains("3 commits/s"), "{output}");
    assert!(output.contains("select 20 activity/s"), "{output}");
    assert!(output.contains("insert 2 activity/s"), "{output}");
    assert!(output.contains("0 ─"), "{output}");
    assert!(output.contains("┬ 00:00:01"), "{output}");
    assert!(output.contains("Transactions and connections"), "{output}");
    assert!(output.contains("Statement activity"), "{output}");
}

#[test]
fn dashboard_overview_uses_remaining_space_for_history() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    let mut dashboard = lazydb::model::dashboard::DashboardTab::new();
    dashboard.history.push(
        lazydb::model::dashboard::RawSample::new(1_000, 1)
            .with(lazydb::model::dashboard::MetricKey::Commits, 10.0),
    );
    dashboard.history.push(
        lazydb::model::dashboard::RawSample::new(3_000, 1)
            .with(lazydb::model::dashboard::MetricKey::Commits, 16.0),
    );
    app.tabs.push(WorkspaceTab::Dashboard(dashboard));
    app.active_tab = 0;
    app.focus = Focus::Results;

    let output = render(&app, 140, 40);
    assert!(output.contains("Transactions"), "{output}");
    assert!(output.contains("commits/s"), "{output}");
    assert!(output.contains("Transactions and connections"), "{output}");
    assert!(output.contains("Statement activity"), "{output}");
}

#[test]
fn dashboard_overview_uses_rates_capacity_percentages_and_metric_icons() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    let mut dashboard = lazydb::model::dashboard::DashboardTab::new();
    dashboard.metadata.max_connections = Some(100);
    dashboard.history.push(
        lazydb::model::dashboard::RawSample::new(1_000, 1)
            .with(lazydb::model::dashboard::MetricKey::Commits, 10.0)
            .with(lazydb::model::dashboard::MetricKey::Rollbacks, 2.0)
            .with(lazydb::model::dashboard::MetricKey::Transactions, 12.0)
            .with(lazydb::model::dashboard::MetricKey::Connections, 40.0)
            .with(lazydb::model::dashboard::MetricKey::BlockHits, 90.0)
            .with(lazydb::model::dashboard::MetricKey::BlockReads, 10.0)
            .with(lazydb::model::dashboard::MetricKey::ServerUptime, 86_401.0),
    );
    dashboard.history.push(
        lazydb::model::dashboard::RawSample::new(3_000, 1)
            .with(lazydb::model::dashboard::MetricKey::Commits, 16.0)
            .with(lazydb::model::dashboard::MetricKey::Rollbacks, 4.0)
            .with(lazydb::model::dashboard::MetricKey::Transactions, 20.0)
            .with(lazydb::model::dashboard::MetricKey::Connections, 42.0)
            .with(lazydb::model::dashboard::MetricKey::BlockHits, 190.0)
            .with(lazydb::model::dashboard::MetricKey::BlockReads, 10.0)
            .with(lazydb::model::dashboard::MetricKey::ServerUptime, 86_403.0),
    );
    dashboard.latest = Some(dashboard.history.samples().last().unwrap().clone());
    app.tabs.push(WorkspaceTab::Dashboard(dashboard));
    app.active_tab = 0;
    app.focus = Focus::Results;

    let output = render(&app, 140, 40);
    assert!(output.contains("Transactions/s"), "{output}");
    assert!(output.contains("4"), "{output}");
    assert!(output.contains("Connections"), "{output}");
    assert!(output.contains("42/100"), "{output}");
    assert!(output.contains("Cache hit rate"), "{output}");
    assert!(output.contains("95.00%"), "{output}");
    assert!(output.contains("Uptime"), "{output}");
    assert!(output.contains("1d 00:00:03"), "{output}");
    assert!(output.contains("󰓡"), "{output}");
}

#[test]
#[allow(dead_code)]
fn catalog_editor_overlay_renders_picker_shell_and_context() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        0,
        [
            lazydb::db::catalog::CatalogKind::Table,
            lazydb::db::catalog::CatalogKind::View,
            lazydb::db::catalog::CatalogKind::MaterializedView,
            lazydb::db::catalog::CatalogKind::Sequence,
        ]
        .into_iter()
        .map(
            |kind| lazydb::model::catalog_editor::CatalogMutationOption {
                object_type: lazydb::db::catalog_mutation::CatalogObjectType::Catalog(kind),
                label: lazydb::db::catalog_mutation::CatalogObjectType::Catalog(kind)
                    .display_label()
                    .into(),
            },
        )
        .collect(),
    ));
    app.overlay = Some(Overlay::CatalogEditor);

    let output = render_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii)).0;

    assert!(output.contains("NEW CATALOG OBJECT"), "{output}");
    assert!(output.contains("TARGET"), "{output}");
    assert!(output.contains("TB Table"), "{output}");
    assert!(output.contains("VW View"), "{output}");
    assert!(output.contains("MV Materialized View"), "{output}");
    assert!(output.contains("SQ Sequence"), "{output}");
    assert!(output.contains("Table"), "{output}");
    assert!(output.contains("View"), "{output}");
    assert!(output.contains("Materialized View"), "{output}");
    assert!(output.contains("Sequence"), "{output}");
}

#[test]
fn catalog_preview_keeps_sql_and_footer_visible_around_sanitized_failure() {
    let connection = ConnectionIdentity {
        profile_id: uuid::Uuid::nil(),
        generation: 1,
    };
    let anchor = CatalogMutationAnchor::Catalog(CatalogId::new(
        uuid::Uuid::nil(),
        CatalogKind::Database,
        ["app\nsecret"],
    ));
    let request = CatalogMutationRequest::new(
        connection,
        9,
        0,
        CatalogMutationMode::Create,
        anchor.clone(),
        CatalogObjectType::Catalog(CatalogKind::Schema),
    )
    .unwrap();
    let plan = lazydb::db::catalog_mutation::CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(CatalogKind::Schema),
        CatalogMutationExecutionMode::Transactional,
        lazydb::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
            profile_id: connection.profile_id,
            database: "app".into(),
            schema: Some("public".into()),
        })
        .unwrap(),
        vec![CatalogTarget::Databases],
        CatalogSelectionHint::Parent(CatalogTarget::Databases),
        None,
        Vec::new(),
        vec!["CREATE SCHEMA events".into()],
    )
    .unwrap();
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor,
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Schema)),
        page: CatalogEditorPage::SqlPreview,
        operation: None,
        catalog_epoch: 0,
        options: vec![CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Schema),
            label: "Schema".into(),
        }],
        selected_option: 0,
        draft: None,
        baseline: None,
        plan: Some(plan),
        error: Some("database denied\nsecret".into()),
        owner_picker: Default::default(),
        preview_scroll: 0,
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let output = render(&app, 100, 20);
    assert!(output.contains("CREATE SCHEMA events"), "{output}");
    assert!(output.contains("SQL PREVIEW"), "{output}");
    assert!(output.contains("target: appsecret"), "{output}");
    assert!(!output.contains("target: app\nsecret"), "{output}");
    assert!(!output.contains("Applying changes..."), "{output}");
    assert!(output.contains("database deniedsecret"), "{output}");
    assert!(output.contains("Enter apply"), "{output}");
    assert!(output.contains("Esc return to form"), "{output}");
    assert!(!output.contains("\nsecret"), "{output}");
}

#[test]
fn explorer_add_overlay_renders_all_connection_actions() {
    let profile = import_connection_url(":memory:", Some("local"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    app.update(Action::OpenExplorerAdd);

    let output = render(&app, 80, 24);
    assert!(output.contains("ADD TO CONNECTION"), "{output}");
    assert!(output.contains("local"), "{output}");
    assert!(output.contains("Connection"), "{output}");
    assert!(output.contains("Connection Group"), "{output}");
    assert!(output.contains("Database"), "{output}");
    assert!(output.contains("User"), "{output}");
    assert!(output.contains("Role"), "{output}");
    assert!(output.contains("j/k"), "{output}");
    assert!(output.contains("Enter"), "{output}");
    assert!(output.contains("Esc"), "{output}");
    assert!(output.contains("PostgreSQL only"), "{output}");
}

#[test]
fn explorer_add_overlay_uses_ascii_icons() {
    let profile = import_connection_url(":memory:", Some("local"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    app.update(Action::OpenExplorerAdd);
    let mut state = UiState::new();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            ui::render_with_state_using_icons(
                frame,
                &app,
                &mut state,
                IconSet::new(IconMode::Ascii),
            )
        })
        .unwrap();
    let output = terminal.backend().to_string();
    assert!(output.contains("CN"), "{output}");
    assert!(output.contains("GR"), "{output}");
    assert!(output.contains("DB"), "{output}");
    assert!(output.contains("US"), "{output}");
    assert!(output.contains("RL"), "{output}");
}

#[test]
fn catalog_editor_busy_renders_real_cancel_control() {
    let profile = import_connection_url("postgresql://localhost/db", Some("busy"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile]);
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Edit,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Catalog(CatalogId::new(
            uuid::Uuid::nil(),
            CatalogKind::Schema,
            ["db", "public"],
        )),
        0,
        vec![],
    ));
    app.catalog_editor.as_mut().unwrap().begin_loading(1);
    app.overlay = Some(Overlay::CatalogEditor);
    let output = render(&app, 100, 30);
    assert!(output.contains("Please wait..."), "{output}");
    assert!(output.contains("Esc cancel"), "{output}");
    app.update(Action::CatalogEditorCancel);
    assert!(app.catalog_editor.is_none());
    assert!(app.overlay.is_none());

    let mut app = App::new(vec![]);
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        0,
        vec![],
    ));
    app.catalog_editor.as_mut().unwrap().operation =
        Some(CatalogEditorOperation::Applying { request_id: 1 });
    app.catalog_editor.as_mut().unwrap().page = CatalogEditorPage::Loading;
    app.overlay = Some(Overlay::CatalogEditor);
    let output = render(&app, 100, 30);
    assert!(output.contains("schema mutation is running"), "{output}");
    assert!(output.contains("Esc wait for completion"), "{output}");
}

#[test]
fn role_editor_renders_secret_as_status_only() {
    let mut draft = lazydb::model::catalog_editor::RoleDraft::new(true);
    draft.name = "alice".into();
    draft.set_password("render-secret");
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::LoginRole),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Role(draft)),
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let output = render(&app, 100, 30);
    assert!(output.contains("Password: <set>"));
    assert!(!output.contains("render-secret"));
}

#[test]
fn table_editor_renders_general_and_columns_sections() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Edit,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::Table,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(
            lazydb::model::catalog_editor::TableDraft {
                name: "events".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "".into(),
                columns: vec![lazydb::model::catalog_editor::ColumnDraft::new_added()],
                selected_column: 0,
                focus: lazydb::model::catalog_editor::TableEditorFocus::Columns,
                column_editor: None,
                indexes: vec![],
                constraints: vec![],
            },
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let output = render(&app, 100, 30);
    assert!(output.contains("GENERAL"), "{output}");
    assert!(output.contains("COLUMNS"), "{output}");
    assert!(!output.contains("COLUMN DETAILS"), "{output}");
    assert!(output.contains("Review SQL"), "{output}");
    assert!(!output.contains("Indexes"), "{output}");
    assert!(!output.contains("Constraints"), "{output}");

    let compact_output = render(&app, 56, 16);
    assert!(
        !compact_output.contains("COLUMN DETAILS"),
        "{compact_output}"
    );
}

#[test]
fn table_editor_baseline_hit_regions_stay_inside_the_rendered_window() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Table)),
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(
            lazydb::model::catalog_editor::TableDraft::new("public"),
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    for (width, height) in [(56, 16), (80, 24), (100, 30)] {
        let (buffer, state) = render_buffer_with_icons(&app, width, height, IconSet::default());
        let window = buffer.area;
        assert!(state.hit_regions.iter().all(|region| {
            region.area.x >= window.x
                && region.area.y >= window.y
                && region.area.right() <= window.right()
                && region.area.bottom() <= window.bottom()
        }));
    }
}

#[test]
fn table_editor_baseline_column_rows_are_disjoint_and_selectable() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.columns = (0..4)
        .map(|index| {
            let mut column = lazydb::model::catalog_editor::ColumnDraft::new_added();
            column.name = format!("column_{index}").into();
            column
        })
        .collect();
    draft.selected_column = 2;
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Table)),
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (buffer, state) = render_buffer_with_icons(&app, 100, 30, IconSet::default());
    let rows = state
        .hit_regions
        .iter()
        .filter_map(|region| match region.target {
            HitTarget::CatalogEditorTableColumn(_) => Some(region.area),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 4);
    assert!(rows.windows(2).all(|pair| pair[0].bottom() <= pair[1].y));
    assert!(rows.iter().all(|row| {
        row.x >= buffer.area.x
            && row.y >= buffer.area.y
            && row.right() <= buffer.area.right()
            && row.bottom() <= buffer.area.bottom()
    }));
}

#[test]
fn table_editor_discard_confirmation_renders_both_choices_and_hit_regions() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::CatalogEditorDiscardConfirm {
        focus: lazydb::model::workspace::CatalogEditorDiscardFocus::KeepEditing,
    });

    let (buffer, state) = render_buffer_with_icons(&app, 80, 24, IconSet::default());
    let mut output = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            output.push_str(buffer[(x, y)].symbol());
        }
    }
    assert!(output.contains("Discard unsaved table changes?"));
    assert!(output.contains("Keep Editing"));
    assert!(output.contains("Discard Changes"));
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::CatalogEditorDiscardKeepEditing })
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::CatalogEditorDiscardChanges })
    );
}

#[test]
fn table_editor_marks_removed_columns_and_exposes_restore_action() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.columns[0].name = "id".into();
    draft.columns[0].state = lazydb::model::catalog_editor::DraftRowState::Removed {
        id: CatalogId::new(uuid::Uuid::nil(), CatalogKind::Column, ["id"]),
    };
    draft.columns.push({
        let mut column = lazydb::model::catalog_editor::ColumnDraft::new_added();
        column.name = "name".into();
        column
    });
    draft.selected_column = 0;
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Table)),
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (buffer, state) = render_buffer_with_icons(&app, 100, 30, IconSet::default());
    let mut output = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            output.push_str(buffer[(x, y)].symbol());
        }
    }
    assert!(output.contains("REMOVED"), "{output}");
    assert!(output.contains("Restore Column"), "{output}");
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::CatalogEditorRestoreTableColumn })
    );
}

#[test]
fn table_editor_column_list_has_headers_and_bounds_wide_values() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.name = "events".into();
    draft.columns[0].name = "用户活动记录名称非常长".into();
    draft.columns[0].native_type = "character varying(4096) with custom metadata".into();
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Table)),
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (buffer, _) = render_buffer_with_icons(&app, 100, 30, IconSet::default());
    let mut output = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            output.push_str(buffer[(x, y)].symbol());
        }
    }
    assert!(output.contains("NAME"), "{output}");
    assert!(output.contains("TYPE"), "{output}");
    assert!(output.contains("NULLABLE"), "{output}");
    assert!(output.contains("…"), "{output}");
}

#[test]
fn table_editor_focus_drives_sections_details_actions_and_context_hints() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::Table,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(
            lazydb::model::catalog_editor::TableDraft {
                name: "events".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "audit".into(),
                columns: vec![lazydb::model::catalog_editor::ColumnDraft::new_added()],
                selected_column: 0,
                focus: lazydb::model::catalog_editor::TableEditorFocus::General(
                    lazydb::model::catalog_editor::TableGeneralField::Name,
                ),
                column_editor: None,
                indexes: vec![],
                constraints: vec![],
            },
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (general, general_state) = render_with_state(&app, 100, 30);
    assert!(general.contains("GENERAL"), "{general}");
    assert!(general.contains("COLUMNS"), "{general}");
    assert!(general.contains("events"), "{general}");
    assert!(
        general.contains("Changes  table properties changed"),
        "{general}"
    );
    assert!(general.contains("[ Review SQL ]"), "{general}");
    assert!(general.contains("[ Cancel ]"), "{general}");
    assert!(
        general.contains("Tab/Shift-Tab/Up/Down move focus"),
        "{general}"
    );
    assert!(general_state.hit_regions.iter().any(|region| {
        region.target
            == HitTarget::CatalogEditorTableField(
                lazydb::model::catalog_editor::TableEditorFocus::General(
                    lazydb::model::catalog_editor::TableGeneralField::Name,
                ),
            )
    }));

    let (compact, compact_state) = render_with_state(&app, 56, 16);
    assert!(compact.contains("events"), "{compact}");
    assert!(compact.contains("[ SQL ]"), "{compact}");
    assert!(compact.contains("[ Cancel ]"), "{compact}");
    assert!(compact_state.hit_regions.iter().any(|region| {
        region.target
            == HitTarget::CatalogEditorTableField(
                lazydb::model::catalog_editor::TableEditorFocus::General(
                    lazydb::model::catalog_editor::TableGeneralField::Name,
                ),
            )
    }));

    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    }
    let (columns, _) = render_with_state(&app, 100, 30);
    assert!(columns.contains("a add below"), "{columns}");
    assert!(columns.contains("e edit column"), "{columns}");

    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.begin_edit_selected_column();
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Nullable,
        );
    }
    let (details, _) = render_with_state(&app, 100, 30);
    assert!(details.contains("COLUMN DETAILS"), "{details}");
    assert!(details.contains("Nullable"), "{details}");
    assert!(details.contains("Enter confirm"), "{details}");

    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name,
        );
    }
    let text_details = render(&app, 100, 30);
    assert!(text_details.contains("COLUMN DETAILS"), "{text_details}");
    assert!(
        text_details.contains("Tab/Shift-Tab/Up/Down move field"),
        "{text_details}"
    );
    assert!(text_details.contains("Enter confirm"), "{text_details}");

    for field in [
        lazydb::model::catalog_editor::TableColumnField::Default,
        lazydb::model::catalog_editor::TableColumnField::Comment,
    ] {
        if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
            .catalog_editor
            .as_mut()
            .and_then(|editor| editor.draft.as_mut())
        {
            draft.focus = lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(field);
        }
        let output = render(&app, 100, 30);
        assert!(output.contains("COLUMN DETAILS"), "{output}");
        assert!(output.contains("Enter confirm"), "{output}");
    }

    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.cancel_column_details();
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Action(
            lazydb::model::catalog_editor::TableActionField::Review,
        );
    }
    let (actions, _) = render_with_state(&app, 100, 30);
    assert!(actions.contains("Review SQL"), "{actions}");
    assert!(actions.contains("Enter/Space activate"), "{actions}");
}

#[test]
fn compact_table_editor_hides_inline_details_and_keeps_actions_reachable() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Group {
            schema: CatalogId::new(uuid::Uuid::nil(), CatalogKind::Schema, ["app", "public"]),
            group: ObjectGroup::Tables,
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::Table,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(
            lazydb::model::catalog_editor::TableDraft::new("public"),
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (_, state) = render_with_state(&app, 56, 16);
    let output = render(&app, 56, 16);
    assert!(output.contains("app / public / Tables"), "{output}");
    assert!(output.contains("Esc"), "{output}");
    let action_rows: Vec<_> = state
        .hit_regions
        .iter()
        .filter(|region| {
            matches!(
                region.target,
                HitTarget::CatalogEditorAddTableColumn
                    | HitTarget::CatalogEditorRemoveTableColumn
                    | HitTarget::CatalogEditorReview
                    | HitTarget::CatalogEditorCancel
            )
        })
        .map(|region| region.area.y)
        .collect();
    assert!(!action_rows.is_empty());
    assert!(action_rows.iter().all(|row| *row < 16));
    assert!(!state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::CatalogEditorTableField(
            lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(_)
        )
    )));
}

#[test]
fn very_small_table_editor_keeps_selected_column_reachable() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.columns[0].name = "selected_column".into();
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::Table,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (output, state) = render_with_state(&app, 56, 16);

    assert!(output.contains("selected_column"), "{output}");
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::CatalogEditorTableColumn(0) })
    );
}

#[test]
fn table_column_details_modal_renders_all_fields_and_controls_at_compact_size() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.begin_edit_selected_column();
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::Table,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (output, state) = render_with_state(&app, 56, 16);
    assert!(output.contains("COLUMN DETAILS"), "{output}");
    for label in ["Name", "Type", "Default", "Comment", "Nullable", "Identity"] {
        assert!(output.contains(label), "missing {label}: {output}");
    }
    assert!(output.contains("[ Confirm ]"), "{output}");
    assert!(output.contains("[ Cancel ]"), "{output}");
    for field in [
        lazydb::model::catalog_editor::TableColumnField::Name,
        lazydb::model::catalog_editor::TableColumnField::Type,
        lazydb::model::catalog_editor::TableColumnField::Default,
        lazydb::model::catalog_editor::TableColumnField::Comment,
        lazydb::model::catalog_editor::TableColumnField::Nullable,
        lazydb::model::catalog_editor::TableColumnField::Identity,
    ] {
        assert!(state.hit_regions.iter().any(|region| {
            region.target
                == HitTarget::CatalogEditorTableField(
                    lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(field),
                )
        }));
    }
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::CatalogEditorColumnDetailsConfirm)
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::CatalogEditorColumnDetailsCancel)
    );
}

#[test]
fn table_column_details_modal_keeps_validation_error_above_controls() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.begin_edit_selected_column();
    draft.column_editor.as_mut().unwrap().error = Some("column type is required".into());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Table)),
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (buffer, _) = render_buffer_with_icons(&app, 80, 24, IconSet::default());
    let mut rows = Vec::new();
    for y in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        if line.contains("column type is required") || line.contains("Confirm") {
            rows.push((y, line));
        }
    }
    assert_eq!(rows.len(), 2);
    assert!(rows[0].0 < rows[1].0);
}

#[test]
fn table_editor_scrolls_selected_column_into_the_rendered_window() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.columns = (0..8)
        .map(|index| {
            let mut column = lazydb::model::catalog_editor::ColumnDraft::new_added();
            column.name = format!("column_{index}").into();
            column
        })
        .collect();
    draft.selected_column = 7;
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::Table,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (output, state) = render_with_state(&app, 100, 30);
    assert!(output.contains("column_7"), "{output}");
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::CatalogEditorTableColumn(0))
    );
}

#[test]
fn table_editor_hides_details_for_last_column_at_list_capacity() {
    let mut app = App::new(Vec::new());
    let mut draft = lazydb::model::catalog_editor::TableDraft::new("public");
    draft.columns = (0..18)
        .map(|index| {
            let mut column = lazydb::model::catalog_editor::ColumnDraft::new_added();
            column.name = format!("column_{index}").into();
            column
        })
        .collect();
    draft.selected_column = draft.columns.len() - 1;
    draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Table)),
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)),
    });
    app.overlay = Some(Overlay::CatalogEditor);

    let (output, state) = render_with_state(&app, 100, 36);

    assert!(output.contains("column_17"), "{output}");
    assert!(!output.contains("COLUMN DETAILS"), "{output}");
    assert!(!state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::CatalogEditorTableField(
            lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(_)
        )
    )));
}

#[test]
fn constraint_editor_renders_typed_fields() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::CheckConstraint,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Constraint(
            lazydb::model::catalog_editor::ConstraintDraft::new(
                lazydb::db::catalog_mutation::ConstraintDefinitionKind::Check {
                    expression: "price > 0".into(),
                    no_inherit: false,
                },
                "public",
                "items",
            ),
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let output = render(&app, 100, 30);
    assert!(output.contains("Expression: price > 0"), "{output}");
    assert!(output.contains("Check"), "{output}");
}

#[test]
fn view_editor_renders_query_and_output_columns() {
    let mut app = view_editor_fixture();
    app.overlay = Some(Overlay::CatalogEditor);
    let (output, state) = render_with_state(&app, 100, 30);
    for label in [
        "GENERAL",
        "DEFINITION",
        "OPTIONS",
        "Name",
        "Schema",
        "Owner",
        "Comment",
        "Output columns",
        "Query",
        "Security barrier",
        "Security invoker",
        "Check option",
    ] {
        assert!(output.contains(label), "missing {label}: {output}");
    }
    assert!(output.contains("Query"), "{output}");
    assert!(output.contains("SELECT id FROM items"), "{output}");
    assert!(output.contains("Output columns"), "{output}");
    assert!(output.contains("On"), "{output}");
    assert!(output.contains("Off"), "{output}");
    assert!(output.contains("Cascaded"), "{output}");
    assert!(output.contains("[ Review SQL ]"), "{output}");
    assert!(output.contains("[ Cancel ]"), "{output}");
    for field in [
        lazydb::model::catalog_editor::CatalogFormFocus::Name,
        lazydb::model::catalog_editor::CatalogFormFocus::Schema,
        lazydb::model::catalog_editor::CatalogFormFocus::Owner,
        lazydb::model::catalog_editor::CatalogFormFocus::Comment,
        lazydb::model::catalog_editor::CatalogFormFocus::OutputColumns,
        lazydb::model::catalog_editor::CatalogFormFocus::Query,
        lazydb::model::catalog_editor::CatalogFormFocus::SecurityBarrier,
        lazydb::model::catalog_editor::CatalogFormFocus::SecurityInvoker,
        lazydb::model::catalog_editor::CatalogFormFocus::CheckOption,
    ] {
        assert!(
            state
                .hit_regions
                .iter()
                .any(|region| { region.target == HitTarget::CatalogEditorFormField(field) }),
            "missing target for {field:?}"
        );
    }

    let compact = render(&app, 56, 16);
    assert!(compact.contains("GENERAL"), "{compact}");
    assert!(compact.contains("DEFINITION"), "{compact}");
    assert!(compact.contains("Query"), "{compact}");
    assert!(compact.contains("OPTIONS"), "{compact}");
    assert!(compact.contains("[ SQL ]"), "{compact}");
    assert!(compact.contains("[ Cancel ]"), "{compact}");

    if let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.security_barrier = lazydb::db::catalog_mutation::ViewOption::available(None);
        draft.security_invoker =
            lazydb::db::catalog_mutation::ViewOption::unavailable("not tested");
        draft.check_option =
            lazydb::db::catalog_mutation::ViewOption::available(Some("LOCAL".into()));
    }
    let states = render(&app, 100, 30);
    assert!(states.contains("Default"), "{states}");
    assert!(states.contains("Local"), "{states}");
    assert!(states.contains("DISABLED"), "{states}");
}

#[test]
fn compact_view_editor_keeps_focused_output_columns_visible() {
    let mut app = view_editor_fixture();
    if let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::CatalogFormFocus::OutputColumns;
    }

    let output = render(&app, 56, 16);
    assert!(output.contains("Output columns"), "{output}");
    assert!(output.contains("id"), "{output}");
    assert!(output.contains("[ SQL ]"), "{output}");
    assert!(output.contains("[ Cancel ]"), "{output}");
}

#[test]
fn compact_view_editor_keeps_focused_security_invoker_visible() {
    let mut app = view_editor_fixture();
    if let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::CatalogFormFocus::SecurityInvoker;
    }

    let output = render(&app, 56, 16);
    assert!(output.contains("Security invoker"), "{output}");
    assert!(output.contains("On"), "{output}");
    assert!(output.contains("[ SQL ]"), "{output}");
    assert!(output.contains("[ Cancel ]"), "{output}");
}

#[test]
fn catalog_form_footer_hints_follow_focus_kind_without_moving_actions() {
    let mut app = view_editor_fixture();
    for (focus, hint) in [
        (
            lazydb::model::catalog_editor::CatalogFormFocus::Name,
            "Type edit",
        ),
        (
            lazydb::model::catalog_editor::CatalogFormFocus::SecurityBarrier,
            "Space cycle",
        ),
        (
            lazydb::model::catalog_editor::CatalogFormFocus::Review,
            "Enter/Space activate",
        ),
    ] {
        if let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
            .catalog_editor
            .as_mut()
            .and_then(|editor| editor.draft.as_mut())
        {
            draft.focus = focus;
        }
        let (output, state) = render_with_state(&app, 100, 30);
        assert!(output.contains(hint), "{output}");
        let action_rows: Vec<_> = state
            .hit_regions
            .iter()
            .filter(|region| {
                matches!(
                    region.target,
                    HitTarget::CatalogEditorReview | HitTarget::CatalogEditorCancel
                )
            })
            .map(|region| region.area.y)
            .collect();
        assert_eq!(action_rows.len(), 2, "{output}");
        assert!(
            action_rows.windows(2).all(|rows| rows[0] == rows[1]),
            "{output}"
        );
    }
}

#[test]
fn materialized_view_footer_does_not_advertise_space_globally() {
    let mut app = view_editor_fixture();
    app.catalog_editor.as_mut().unwrap().object_type =
        Some(CatalogObjectType::Catalog(CatalogKind::MaterializedView));
    app.catalog_editor.as_mut().unwrap().draft = Some(
        lazydb::model::catalog_editor::CatalogDraft::MaterializedView(
            lazydb::model::catalog_editor::MaterializedViewDraft {
                name: "mv".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "".into(),
                query: "SELECT 1".into(),
                tablespace: "fast".into(),
                with_data: true,
                focus: lazydb::model::catalog_editor::CatalogFormFocus::Tablespace,
                query_editable: true,
            },
        ),
    );
    let output = render(&app, 100, 30);
    assert!(!output.contains("Space toggle data"), "{output}");
    assert!(output.contains("Type edit"), "{output}");
}

#[test]
fn redesigned_object_forms_have_no_debug_output_at_any_supported_size() {
    for (name, app) in [
        ("view", view_editor_fixture()),
        ("materialized view", materialized_view_fixture()),
        ("sequence", sequence_editor_fixture()),
    ] {
        for (width, height) in [(120, 36), (100, 30), (80, 24), (56, 16)] {
            let output = render(&app, width, height);
            for forbidden in [
                "ViewOption {",
                "availability:",
                "value:",
                "Unset",
                "NoLimit",
                "Value(\"",
            ] {
                assert!(
                    !output.contains(forbidden),
                    "{name} at {width}x{height} leaked {forbidden}: {output}"
                );
            }
        }
    }
}

#[test]
fn redesigned_object_forms_keep_active_field_actions_and_footer_in_render_matrix() {
    let cases = [
        (
            "view",
            view_editor_fixture(),
            [
                lazydb::model::catalog_editor::CatalogFormFocus::Name,
                lazydb::model::catalog_editor::CatalogFormFocus::Query,
                lazydb::model::catalog_editor::CatalogFormFocus::CheckOption,
            ],
        ),
        (
            "materialized view",
            materialized_view_fixture(),
            [
                lazydb::model::catalog_editor::CatalogFormFocus::Name,
                lazydb::model::catalog_editor::CatalogFormFocus::Query,
                lazydb::model::catalog_editor::CatalogFormFocus::WithData,
            ],
        ),
        (
            "sequence",
            sequence_editor_fixture(),
            [
                lazydb::model::catalog_editor::CatalogFormFocus::Name,
                lazydb::model::catalog_editor::CatalogFormFocus::MinValue,
                lazydb::model::catalog_editor::CatalogFormFocus::OwnedBy,
            ],
        ),
    ];

    for (name, mut app, focuses) in cases {
        for (width, height) in [(120, 36), (100, 30), (80, 24), (56, 16)] {
            for focus in focuses {
                set_catalog_form_focus(&mut app, focus);
                let (output, state) = render_with_state(&app, width, height);
                assert!(
                    state.hit_regions.iter().any(|region| {
                        region.target == HitTarget::CatalogEditorFormField(focus)
                    }),
                    "{name} at {width}x{height} lost active field {focus:?}: {output}"
                );
                assert!(
                    state
                        .hit_regions
                        .iter()
                        .any(|region| region.target == HitTarget::CatalogEditorReview),
                    "{name} at {width}x{height} lost Review: {output}"
                );
                assert!(
                    state
                        .hit_regions
                        .iter()
                        .any(|region| region.target == HitTarget::CatalogEditorCancel),
                    "{name} at {width}x{height} lost Cancel: {output}"
                );
                assert!(output.contains("Tab/Shift-Tab"), "{name}: {output}");
            }
        }
    }
}

fn materialized_view_fixture() -> App {
    let mut app = view_editor_fixture();
    app.catalog_editor.as_mut().unwrap().object_type =
        Some(CatalogObjectType::Catalog(CatalogKind::MaterializedView));
    app.catalog_editor.as_mut().unwrap().draft = Some(
        lazydb::model::catalog_editor::CatalogDraft::MaterializedView(
            lazydb::model::catalog_editor::MaterializedViewDraft {
                name: "mv".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "note".into(),
                query: "SELECT id FROM items".into(),
                tablespace: "fast".into(),
                with_data: true,
                focus: lazydb::model::catalog_editor::CatalogFormFocus::Name,
                query_editable: true,
            },
        ),
    );
    app
}

fn sequence_editor_fixture() -> App {
    let mut app = view_editor_fixture();
    app.catalog_editor.as_mut().unwrap().object_type =
        Some(CatalogObjectType::Catalog(CatalogKind::Sequence));
    app.catalog_editor.as_mut().unwrap().draft =
        Some(lazydb::model::catalog_editor::CatalogDraft::Sequence(
            lazydb::model::catalog_editor::SequenceDraft {
                name: "orders".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "ids".into(),
                data_type: "bigint".into(),
                increment: "1".into(),
                min_value: lazydb::model::catalog_editor::SequenceBound::Unset.into(),
                max_value: lazydb::model::catalog_editor::SequenceBound::Unset.into(),
                start_value: "1".into(),
                restart_value: "".into(),
                cache: "1".into(),
                cycle: false,
                owned_by: "NONE".into(),
                focus: lazydb::model::catalog_editor::CatalogFormFocus::Name,
            },
        ));
    app
}

fn set_catalog_form_focus(app: &mut App, focus: lazydb::model::catalog_editor::CatalogFormFocus) {
    match app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap() {
        lazydb::model::catalog_editor::CatalogDraft::View(draft) => draft.focus = focus,
        lazydb::model::catalog_editor::CatalogDraft::MaterializedView(draft) => draft.focus = focus,
        lazydb::model::catalog_editor::CatalogDraft::Sequence(draft) => draft.focus = focus,
        _ => panic!("unexpected catalog form fixture"),
    }
}

fn view_editor_fixture() -> App {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::View,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::View(
            lazydb::model::catalog_editor::ViewDraft {
                name: "v".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "note".into(),
                query: "SELECT id FROM items".into(),
                output_columns: "id".into(),
                security_barrier: lazydb::db::catalog_mutation::ViewOption::available(Some(false)),
                security_invoker: lazydb::db::catalog_mutation::ViewOption::available(Some(true)),
                check_option: lazydb::db::catalog_mutation::ViewOption::available(Some(
                    "CASCADED".into(),
                )),
                focus: lazydb::model::catalog_editor::CatalogFormFocus::Name,
            },
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    app
}

#[test]
fn index_editor_renders_typed_fields() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::Index,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Index(
            lazydb::model::catalog_editor::IndexDraft {
                name: "idx".into(),
                schema: "public".into(),
                relation: "events".into(),
                unique: true,
                access_method: "btree".into(),
                columns: vec![lazydb::model::catalog_editor::IndexColumnDraft {
                    expression: "name".into(),
                    descending: false,
                    nulls_first: false,
                    is_expression: false,
                }],
                include_columns: "id".into(),
                predicate: "active".into(),
                tablespace: "fast".into(),
            },
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let output = render(&app, 100, 30);
    assert!(output.contains("INCLUDE: id"));
    assert!(output.contains("Predicate: active"));
}

#[test]
fn materialized_view_editor_renders_data_state_and_read_only_query() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Edit,
        anchor: lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            CatalogKind::MaterializedView,
        )),
        page: lazydb::model::catalog_editor::CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(
            lazydb::model::catalog_editor::CatalogDraft::MaterializedView(
                lazydb::model::catalog_editor::MaterializedViewDraft {
                    name: "mv".into(),
                    schema: "public".into(),
                    owner: "postgres".into(),
                    comment: "note".into(),
                    query: "SELECT id FROM items".into(),
                    tablespace: "fast".into(),
                    with_data: false,
                    focus: lazydb::model::catalog_editor::CatalogFormFocus::Name,
                    query_editable: false,
                },
            ),
        ),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let output = render(&app, 100, 30);
    for label in [
        "GENERAL",
        "DEFINITION",
        "STORAGE",
        "Name",
        "Schema",
        "Owner",
        "Comment",
        "Query",
        "Tablespace",
        "With data",
    ] {
        assert!(output.contains(label), "missing {label}: {output}");
    }
    assert!(output.contains("WITH NO DATA"), "{output}");
    assert!(output.contains("READ ONLY"), "{output}");
    assert!(output.contains("[ Review SQL ]"), "{output}");
    assert!(output.contains("[ Cancel ]"), "{output}");
    let (_, state) = render_with_state(&app, 100, 30);
    for field in [
        lazydb::model::catalog_editor::CatalogFormFocus::Name,
        lazydb::model::catalog_editor::CatalogFormFocus::Schema,
        lazydb::model::catalog_editor::CatalogFormFocus::Owner,
        lazydb::model::catalog_editor::CatalogFormFocus::Comment,
        lazydb::model::catalog_editor::CatalogFormFocus::Tablespace,
    ] {
        assert!(
            state
                .hit_regions
                .iter()
                .any(|region| { region.target == HitTarget::CatalogEditorFormField(field) }),
            "missing target for {field:?}"
        );
    }
    if let Some(lazydb::model::catalog_editor::CatalogDraft::MaterializedView(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::CatalogFormFocus::Query;
    }
    let compact = render(&app, 56, 16);
    assert!(compact.contains("Query"), "{compact}");
    assert!(compact.contains("[ SQL ]"), "{compact}");
    assert!(compact.contains("[ Cancel ]"), "{compact}");
}

#[test]
fn sequence_editor_renders_sections_bounds_and_all_fields() {
    let mut app = App::new(Vec::new());
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Profile {
            profile_id: uuid::Uuid::nil(),
        },
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Sequence)),
        page: CatalogEditorPage::Form,
        operation: None,
        catalog_epoch: 0,
        options: vec![],
        selected_option: 0,
        baseline: None,
        plan: None,
        error: None,
        owner_picker: Default::default(),
        preview_scroll: 0,
        draft: Some(lazydb::model::catalog_editor::CatalogDraft::Sequence(
            lazydb::model::catalog_editor::SequenceDraft {
                name: "orders".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "ids".into(),
                data_type: "bigint".into(),
                increment: "1".into(),
                min_value: lazydb::model::catalog_editor::SequenceBoundDraft {
                    kind: lazydb::model::catalog_editor::SequenceBoundKind::Custom,
                    value: "10".into(),
                },
                max_value: lazydb::model::catalog_editor::SequenceBoundDraft {
                    kind: lazydb::model::catalog_editor::SequenceBoundKind::Default,
                    value: "".into(),
                },
                start_value: "10".into(),
                restart_value: "".into(),
                cache: "1".into(),
                cycle: true,
                owned_by: "public.orders.id".into(),
                focus: lazydb::model::catalog_editor::CatalogFormFocus::MinValue,
            },
        )),
    });
    app.overlay = Some(Overlay::CatalogEditor);
    let (output, state) = render_with_state(&app, 100, 30);
    for label in [
        "GENERAL",
        "VALUES",
        "OWNERSHIP",
        "Name",
        "Schema",
        "Owner",
        "Comment",
        "Data type",
        "Increment",
        "Minimum",
        "Maximum",
        "Start",
        "Restart",
        "Cache",
        "Cycle",
        "Owned by",
        "Default",
        "Custom",
        "10",
    ] {
        assert!(output.contains(label), "missing {label}: {output}");
    }
    assert!(output.contains("[x] On"), "{output}");
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Sequence(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.min_value.kind = lazydb::model::catalog_editor::SequenceBoundKind::NoLimit;
        draft.max_value.kind = lazydb::model::catalog_editor::SequenceBoundKind::NoLimit;
    }
    let bounds = render(&app, 100, 30);
    assert!(bounds.contains("No min value"), "{bounds}");
    assert!(bounds.contains("No max value"), "{bounds}");
    for field in [
        lazydb::model::catalog_editor::CatalogFormFocus::Name,
        lazydb::model::catalog_editor::CatalogFormFocus::Schema,
        lazydb::model::catalog_editor::CatalogFormFocus::Owner,
        lazydb::model::catalog_editor::CatalogFormFocus::Comment,
        lazydb::model::catalog_editor::CatalogFormFocus::DataType,
        lazydb::model::catalog_editor::CatalogFormFocus::Increment,
        lazydb::model::catalog_editor::CatalogFormFocus::MinValue,
        lazydb::model::catalog_editor::CatalogFormFocus::MaxValue,
        lazydb::model::catalog_editor::CatalogFormFocus::StartValue,
        lazydb::model::catalog_editor::CatalogFormFocus::RestartValue,
        lazydb::model::catalog_editor::CatalogFormFocus::Cache,
        lazydb::model::catalog_editor::CatalogFormFocus::Cycle,
        lazydb::model::catalog_editor::CatalogFormFocus::OwnedBy,
    ] {
        assert!(
            state
                .hit_regions
                .iter()
                .any(|region| { region.target == HitTarget::CatalogEditorFormField(field) }),
            "missing target for {field:?}"
        );
    }
    let compact = render(&app, 56, 16);
    assert!(compact.contains("Minimum"), "{compact}");
    assert!(compact.contains("[ SQL ]"), "{compact}");
    assert!(compact.contains("[ Cancel ]"), "{compact}");
}

#[test]
fn overflowing_workspace_tabs_keep_the_active_tab_visible() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    for name in ["alpha-tab", "bravo-tab", "charlie-tab", "delta-active"] {
        app.tabs
            .push(WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(name)));
    }
    app.active_tab = 3;

    let (output, state) = render_with_icons(&app, 56, 20, IconSet::new(IconMode::Unicode));

    assert!(output.contains("delta-active"), "{output}");
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Tab(app.active_tab))
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| matches!(region.target, HitTarget::TabScrollLeft(_)))
    );
}

#[test]
fn workspace_tab_arrows_are_hidden_when_all_tabs_fit() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.tabs
        .push(WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(
            "alpha",
        )));
    app.tabs
        .push(WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(
            "bravo",
        )));

    let (output, state) = render_with_icons(&app, 120, 20, IconSet::new(IconMode::Ascii));

    assert!(!output.contains('<'));
    assert!(!output.contains('>'));
    assert!(!state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::TabScrollLeft(_) | HitTarget::TabScrollRight(_)
    )));
}

#[test]
fn workspace_tab_viewport_follows_tab_changes_and_wraparound() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    for name in ["alpha-tab", "bravo-tab", "charlie-tab", "delta-active"] {
        app.tabs
            .push(WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(name)));
    }
    let mut state = UiState::new();
    let mut terminal = Terminal::new(TestBackend::new(56, 20)).unwrap();

    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    app.update(Action::PreviousTab);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();

    assert_eq!(app.active_tab, 3);
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Tab(app.active_tab))
    );
    app.update(Action::NextTab);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();

    assert_eq!(app.active_tab, 0);
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Tab(app.active_tab))
    );
}

#[test]
fn workspace_tab_viewport_recalculates_after_resize() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    for name in ["alpha-tab", "bravo-tab", "charlie-tab", "delta-active"] {
        app.tabs
            .push(WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(name)));
    }
    app.active_tab = 3;
    let mut state = UiState::new();
    let mut narrow = Terminal::new(TestBackend::new(56, 20)).unwrap();
    narrow
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Tab(app.active_tab))
    );

    let mut wide = Terminal::new(TestBackend::new(120, 20)).unwrap();
    wide.draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Tab(app.active_tab))
    );
    assert!(!state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::TabScrollLeft(_) | HitTarget::TabScrollRight(_)
    )));
}

#[test]
fn pane_maximize_hides_other_pane_hit_targets_and_restores_them() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    let mut state = UiState::new();

    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Explorer))
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Editor))
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Results))
    );

    app.update(Action::TogglePaneMaximized);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(
        !state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Explorer))
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Editor))
    );
    assert!(
        !state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Results))
    );

    app.update(Action::TogglePaneMaximized);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Explorer))
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Results))
    );
}

#[test]
fn dashboard_maximize_hides_explorer_when_results_are_focused() {
    let mut app = App::new(Vec::new());
    app.tabs.clear();
    app.tabs.push(WorkspaceTab::Dashboard(
        lazydb::model::dashboard::DashboardTab::new(),
    ));
    app.active_tab = 0;
    app.focus = Focus::Results;

    let (_, normal) = render_with_state(&app, 160, 40);
    assert!(
        normal
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Explorer))
    );

    app.update(Action::TogglePaneMaximized);
    let (_, maximized) = render_with_state(&app, 160, 40);
    assert!(
        !maximized
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Explorer))
    );
    assert!(
        maximized
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::Focus(Focus::Results))
    );
}

#[test]
fn offline_profiles_render_as_collapsed_even_when_expansion_is_pending() {
    let profile = import_connection_url(":memory:", Some("offline-profile"))
        .unwrap()
        .profile;
    let app = App::new(vec![profile]);
    let output = render(&app, 120, 36);

    assert!(output.contains("offline-profile"), "{output}");
    assert!(!output.contains("▾"), "{output}");
    assert!(output.contains("▸"), "{output}");
}

#[test]
fn record_view_highlight_follows_the_selected_field() {
    let mut app = fixture();
    app.focus = Focus::Results;
    app.update(Action::OpenRecordView);

    let selected = record_view_field_background(&app, "id");
    let unselected = record_view_field_background(&app, "name");
    assert_ne!(selected, unselected);

    app.update(Action::RecordViewViewportChanged {
        tab_id: app.active_console().id,
        visible_fields: 18,
    });
    app.update(Action::RecordViewMoveFields(1));

    assert_eq!(record_view_field_background(&app, "id"), unselected);
    assert_eq!(record_view_field_background(&app, "name"), selected);
}

#[test]
fn record_view_navigation_changes_record_and_closes_without_database_io() {
    let mut app = fixture();
    let tab_id = app.active_console().id;
    let generation = app.active_console().generation;
    let connection = app.connection.active_identity().unwrap();
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection,
        outcome: QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![ColumnMeta {
                    name: "id".into(),
                    type_name: "INTEGER".into(),
                }],
                rows: vec![vec![CellValue::Integer(1)], vec![CellValue::Integer(2)]],
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::ZERO, Duration::ZERO, 2),
        },
    });
    app.focus = Focus::Results;
    app.update(Action::OpenRecordView);
    app.update(Action::RecordViewMoveRow(1));

    assert!(matches!(app.overlay, Some(Overlay::RecordView(_))));
    assert_eq!(app.active_console().grid.selected_row, 1);
    assert!(render(&app, 100, 30).contains("ROW 2 / 2"));
    assert!(app.update(Action::CloseRecordView).is_empty());
    assert!(app.overlay.is_none());
}

#[test]
fn sql_editor_marks_only_the_statement_at_the_cursor() {
    let mut app = fixture();
    app.update(Action::ReplaceEditor("SELECT 1;\nSELECT 2;".into()));
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 120,
            height: 10,
        })
        .unwrap();
    assert!(
        snapshot.lines[0]
            .spans
            .iter()
            .any(|span| span.current_statement)
    );
    assert!(
        snapshot.lines[1]
            .spans
            .iter()
            .all(|span| !span.current_statement)
    );
}

#[test]
fn explorer_search_renders_inline_empty_and_loading_states_at_compact_size() {
    let mut app = fixture();
    app.focus = Focus::Explorer;
    app.update(Action::ExplorerSearchOpen);
    let empty = render(&app, 80, 24);
    assert!(empty.contains("/ "));
    assert!(empty.contains("Type to search all objects"));

    app.update(Action::ExplorerSearchInsert('u'));
    let loading = render(&app, 80, 24);
    assert!(loading.contains("/ u"));
    assert!(loading.contains("No objects match \"u\""));
}

#[test]
fn frontend_search_shows_no_match_state_without_server_loading() {
    let mut app = fixture();
    app.focus = Focus::Explorer;
    app.update(Action::ExplorerSearchOpen);
    assert!(app.update(Action::ExplorerSearchInsert('u')).is_empty());

    let output = render(&app, 100, 24);
    assert!(output.contains("No objects match \"u\""));
}

#[test]
fn explorer_search_highlights_matches_across_identifier_separators() {
    let profile = import_connection_url(":memory:", Some("search-highlight"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.focus = Focus::Explorer;
    app.connection.profile_id = Some(profile.id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;

    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["app"]),
        QualifiedName {
            database: Some("app".into()),
            schema: None,
            object: "app".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        CatalogId::new(profile.id, CatalogKind::Schema, ["app", "main"]),
        database.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("main".into()),
            object: "main".into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let table = CatalogEntry::relation(
        CatalogId::new(profile.id, CatalogKind::Table, ["app", "main", "sys_user"]),
        schema.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("main".into()),
            object: "sys_user".into(),
        },
        "table",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    let profile_state = app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap();
    profile_state
        .catalog
        .insert_subtree(vec![database, schema.clone(), table])
        .unwrap();
    profile_state
        .catalog
        .set_group_state(
            &schema.id,
            ObjectGroup::Tables,
            CatalogGroupState {
                count: CatalogCount::Exact(1),
                completeness: CatalogCompleteness::Complete,
            },
        )
        .unwrap();

    app.update(Action::ExplorerSearchOpen);
    for character in "sysuser".chars() {
        app.update(Action::ExplorerSearchInsert(character));
    }
    let (buffer, _) = render_buffer_with_icons(&app, 100, 24, IconSet::new(IconMode::Ascii));
    let (x, y) = find_text_cell(&buffer, "sys_user").expect("search result");

    assert_eq!(buffer[(x, y)].fg, buffer[(x + 4, y)].fg);
    assert_ne!(buffer[(x, y)].fg, buffer[(x + 3, y)].fg);
}

#[test]
fn sql_editor_marks_statement_when_cursor_is_on_internal_space() {
    let mut app = fixture();
    app.update(Action::ReplaceEditor("SELECT 1;\nSELECT 2;".into()));
    for _ in 0.."SELECT".len() {
        app.update(Action::EditorKey(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )));
    }

    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 120,
            height: 10,
        })
        .unwrap();

    assert!(
        snapshot.lines[0]
            .spans
            .iter()
            .any(|span| span.current_statement)
    );
    assert!(
        snapshot.lines[1]
            .spans
            .iter()
            .all(|span| !span.current_statement)
    );
}

#[test]
fn sql_editor_statement_indicator_preserves_text_style() {
    let mut app = fixture();
    app.update(Action::ReplaceEditor(
        "SELECT user_name;\nSELECT other_name;".into(),
    ));
    let (buffer, _) = render_buffer_with_icons(&app, 100, 24, IconSet::new(IconMode::Ascii));
    let (select_x, select_y) = find_text_cell(&buffer, "SELECT").expect("first statement");
    let (other_x, other_y) = find_text_cell(&buffer, "other_name").expect("second statement");

    assert!(
        !buffer[(select_x, select_y)]
            .modifier
            .contains(Modifier::UNDERLINED)
    );
    assert!(
        !buffer[(select_x, select_y)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert_eq!(buffer[(select_x, select_y)].bg, Color::Rgb(12, 19, 30));
    assert!(
        !buffer[(other_x, other_y)]
            .modifier
            .contains(Modifier::UNDERLINED)
    );
    assert_eq!(buffer[(other_x, other_y)].bg, Color::Rgb(12, 19, 30));

    let gutter_separator_x = (0..select_x)
        .find(|x| buffer[(*x, select_y)].symbol() == "┃")
        .expect("current statement gutter marker");
    assert_eq!(buffer[(gutter_separator_x, select_y)].symbol(), "┃");
    assert_eq!(buffer[(gutter_separator_x, other_y)].symbol(), "│");
}

#[test]
fn sql_editor_highlights_only_the_current_statement_on_shared_line() {
    let mut app = fixture();
    app.update(Action::ReplaceEditor("SELECT 1;SELECT 2;".into()));
    let (buffer, _) = render_buffer_with_icons(&app, 100, 24, IconSet::new(IconMode::Ascii));
    let (first_x, y) = find_text_cell(&buffer, "SELECT").expect("first statement");
    let second_x = first_x + "SELECT 1;".len() as u16;

    assert_eq!(buffer[(first_x, y)].bg, Color::Rgb(17, 28, 43));
    assert_eq!(buffer[(second_x, y)].symbol(), "S");
    assert_eq!(buffer[(second_x, y)].bg, Color::Rgb(12, 19, 30));
}

fn render(app: &App, width: u16, height: u16) -> String {
    render_with_state(app, width, height).0
}

fn console_manager_fixture() -> App {
    let mut app = App::new(Vec::new());
    app.sql_editors = vec![
        lazydb::model::tab::ConsoleRecord {
            id: uuid::Uuid::from_u128(1),
            name: "charlie".into(),
            execution_target: None,
            transaction_mode: TransactionMode::Auto,
            open: false,
        },
        lazydb::model::tab::ConsoleRecord {
            id: uuid::Uuid::from_u128(2),
            name: "Beta".into(),
            execution_target: None,
            transaction_mode: TransactionMode::Auto,
            open: true,
        },
        lazydb::model::tab::ConsoleRecord {
            id: uuid::Uuid::from_u128(3),
            name: "console".into(),
            execution_target: None,
            transaction_mode: TransactionMode::Auto,
            open: true,
        },
        lazydb::model::tab::ConsoleRecord {
            id: uuid::Uuid::from_u128(4),
            name: "alpha".into(),
            execution_target: None,
            transaction_mode: TransactionMode::Auto,
            open: true,
        },
    ];
    app.sql_editor_list.selected_id = Some(uuid::Uuid::from_u128(3));
    app.overlay = Some(Overlay::SqlEditorList(app.sql_editor_list.clone()));
    app
}

fn assert_order(output: &str, names: &[&str]) {
    let mut previous = 0;
    for name in names {
        let position = output[previous..]
            .find(name)
            .unwrap_or_else(|| panic!("missing {name:?} in {output}"));
        previous += position;
    }
}

#[test]
fn console_manager_renders_sorted_open_and_closed_consoles() {
    let app = console_manager_fixture();
    let output = render(&app, 100, 30);

    assert_order(&output, &["console", "alpha", "Beta", "charlie"]);
    assert!(output.contains("OPEN"), "{output}");
    assert!(output.contains("CLOSED"), "{output}");
    assert!(output.contains("a new"), "{output}");
    assert!(output.contains("d delete"), "{output}");
    assert!(output.contains("r rename"), "{output}");
    assert!(output.contains("/ search"), "{output}");
}

#[test]
fn console_manager_renders_empty_search_rename_and_delete_modes() {
    let mut app = console_manager_fixture();
    app.sql_editor_list.query.set("missing");
    app.overlay = Some(Overlay::SqlEditorList(app.sql_editor_list.clone()));
    assert!(render(&app, 80, 24).contains("No matching consoles"));

    app.sql_editor_list.mode = lazydb::model::sql_editor_list::SqlEditorListMode::Search;
    app.sql_editor_list.query.set("alp");
    app.overlay = Some(Overlay::SqlEditorList(app.sql_editor_list.clone()));
    let (search, search_state) = render_with_state(&app, 80, 24);
    assert!(search.contains("/alp"), "{search}");
    assert!(search_state.cursor_style.is_some(), "{search}");

    app.sql_editor_list.mode = lazydb::model::sql_editor_list::SqlEditorListMode::Rename {
        console_id: uuid::Uuid::from_u128(4),
        input: lazydb::model::text_input::TextInput::default(),
        error: Some("Name is already in use".into()),
    };
    app.overlay = Some(Overlay::SqlEditorList(app.sql_editor_list.clone()));
    let (rename, rename_state) = render_with_state(&app, 80, 24);
    assert!(rename.contains("Rename alpha"), "{rename}");
    assert!(rename.contains("Name is already in use"), "{rename}");
    assert!(rename_state.cursor_style.is_some(), "{rename}");

    app.sql_editor_list.mode = lazydb::model::sql_editor_list::SqlEditorListMode::DeleteConfirm {
        console_id: uuid::Uuid::from_u128(4),
    };
    app.overlay = Some(Overlay::SqlEditorList(app.sql_editor_list.clone()));
    let (delete, delete_state) = render_with_state(&app, 80, 24);
    assert!(delete.contains("Permanently delete 'alpha'"), "{delete}");
    assert!(delete.contains("Enter delete  Esc cancel"), "{delete}");
    assert!(delete_state.cursor_style.is_none(), "{delete}");
}

#[test]
fn console_manager_compact_popup_keeps_border_and_footer() {
    let app = console_manager_fixture();
    let output = render(&app, 80, 16);
    assert!(output.contains("CONSOLES"), "{output}");
    assert!(output.contains("Esc close"), "{output}");
    assert!(output.lines().any(|line| line.contains("╭")), "{output}");
}

#[test]
fn profile_group_overlay_renders_options_and_editor_content() {
    let mut app = App::new(Vec::new());
    app.connection_groups.push(
        lazydb::profile::ConnectionGroup::new(uuid::Uuid::from_u128(1), "Production").unwrap(),
    );
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Picker {
        profile_id: uuid::Uuid::from_u128(2),
        selected: 0,
        busy: false,
    }));
    let (picker, picker_state) = render_with_state(&app, 80, 30);
    assert!(picker.contains("Ungrouped"));
    assert!(picker.contains("Production"));
    assert!(picker.contains("Create group"));
    assert!(
        picker_state
            .hit_regions
            .iter()
            .any(|region| matches!(region.target, HitTarget::ProfileGroupOption(_)))
    );
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
        group_id: None,
        name: "Production".into(),
        error: Some("duplicate".into()),
        busy: false,
    }));
    let (editor, editor_state) = render_with_state(&app, 80, 30);
    assert!(editor.contains("NEW CONNECTION GROUP"), "{editor}");
    assert!(editor.contains("GROUP DETAILS"), "{editor}");
    assert!(editor.contains("Production"));
    assert!(editor.contains("duplicate"));
    assert!(editor.contains("[ Save group ]"), "{editor}");
    assert!(editor.contains("[ Cancel ]"), "{editor}");
    assert!(editor_state.cursor_style.is_some(), "{editor}");
    assert!(
        editor_state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::ProfileGroupConfirm)
    );
    assert!(
        editor_state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::ProfileGroupCancel)
    );
}

#[test]
fn busy_profile_group_editor_disables_actions_and_cursor() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
        group_id: Some(uuid::Uuid::from_u128(1)),
        name: "Production".into(),
        error: None,
        busy: true,
    }));

    let (editor, state) = render_with_state(&app, 80, 24);

    assert!(editor.contains("EDIT CONNECTION GROUP"), "{editor}");
    assert!(editor.contains("BUSY // SAVING GROUP"), "{editor}");
    assert!(editor.contains("[ Saving... ]"), "{editor}");
    assert!(state.cursor_style.is_none(), "{editor}");
    assert!(!state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ProfileGroupConfirm | HitTarget::ProfileGroupCancel
    )));
}

fn record_view_field_background(app: &App, field: &str) -> Color {
    let width = 100;
    let height = 30;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state(frame, app, &mut state))
        .unwrap();
    let buffer = terminal.backend().buffer();
    for y in 0..height {
        let line = (0..width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        if let Some(x) = line.find(field) {
            return buffer[(x as u16, y)].bg;
        }
    }
    panic!("field {field:?} was not rendered");
}

fn render_with_state(app: &App, width: u16, height: u16) -> (String, UiState) {
    render_with_icons(app, width, height, IconSet::default())
}

fn render_with_icons(app: &App, width: u16, height: u16, icons: IconSet) -> (String, UiState) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state_using_icons(frame, app, &mut state, icons))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut output = String::new();
    for y in 0..height {
        for x in 0..width {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    (output, state)
}

#[test]
fn other_profiles_group_is_rendered_as_muted_secondary_content_without_an_icon() {
    let current = import_connection_url("sqlite::memory:", Some("current"))
        .unwrap()
        .profile;
    let other = import_connection_url("sqlite::memory:", Some("other"))
        .unwrap()
        .profile;
    let other_id = other.id;
    let mut app = App::new(vec![current, other]);
    app.explorer
        .normalized
        .profiles
        .get_mut(&other_id)
        .unwrap()
        .placement = ProfilePlacement::OtherProject;
    let (buffer, _) = render_buffer_with_icons(&app, 100, 24, IconSet::new(IconMode::Ascii));
    let (x, y) = find_text_cell(&buffer, "others").expect("others group");
    let line = (0..100)
        .map(|column| buffer[(column, y)].symbol())
        .collect::<String>();

    assert!(!line.contains("OTHERS"));
    assert!(!line.contains("· others"));
    assert!(buffer[(x, y)].modifier.contains(Modifier::DIM));
    assert!(!buffer[(x, y)].modifier.contains(Modifier::BOLD));
}

#[test]
fn pending_prefix_opens_floating_shortcut_window() {
    let mut app = fixture();
    app.focus = Focus::Explorer;
    let mut keymap = lazydb::input::keymap::Keymap::default();
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), &app),
        None
    );
    let now = std::time::Instant::now();
    let sequence = keymap.sequence_state(&app, now).unwrap();
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| {
            ui::render_with_state_using_icons_and_sequence(
                frame,
                &app,
                &mut state,
                IconSet::default(),
                Some(&sequence),
            )
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let output = (0..36)
        .flat_map(|y| (0..120).map(move |x| buffer[(x, y)].symbol()))
        .collect::<String>();
    assert!(output.contains("Space  Up/Down select  Enter run  Esc/Ctrl-C cancel"));
    assert!(output.contains("focus Explorer"));
}

#[test]
fn counted_pending_prefix_keeps_count_in_footer_label() {
    let mut app = fixture();
    app.focus = Focus::Results;
    let mut keymap = lazydb::input::keymap::Keymap::default();
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE), &app),
        None
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE), &app),
        None
    );
    assert_eq!(
        keymap.map(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
            &app
        ),
        None
    );
    let sequence = keymap
        .sequence_state(&app, std::time::Instant::now())
        .unwrap();
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| {
            ui::render_with_state_using_icons_and_sequence(
                frame,
                &app,
                &mut state,
                IconSet::default(),
                Some(&sequence),
            )
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let output = (0..36)
        .flat_map(|y| (0..120).map(move |x| buffer[(x, y)].symbol()))
        .collect::<String>();
    assert!(output.contains("10 Ctrl-w  Up/Down select  Enter run  Esc/Ctrl-C cancel"));
    assert!(output.contains("restore default pane sizes"));
}

fn render_buffer_with_icons(
    app: &App,
    width: u16,
    height: u16,
    icons: IconSet,
) -> (ratatui::buffer::Buffer, UiState) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state_using_icons(frame, app, &mut state, icons))
        .unwrap();
    (terminal.backend().buffer().clone(), state)
}

fn render_buffer_with_state(
    app: &App,
    width: u16,
    height: u16,
    mut state: UiState,
) -> (ratatui::buffer::Buffer, UiState) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| ui::render_with_state_using_icons(frame, app, &mut state, IconSet::default()))
        .unwrap();
    (terminal.backend().buffer().clone(), state)
}

fn find_ascii_cells(buffer: &ratatui::buffer::Buffer, y: u16, text: &str) -> Option<u16> {
    let width = buffer.area.width;
    (0..width).find(|start| {
        text.chars().enumerate().all(|(offset, character)| {
            let x = start.saturating_add(offset as u16);
            x < width && buffer[(x, y)].symbol() == character.to_string()
        })
    })
}

fn find_text_cell(buffer: &ratatui::buffer::Buffer, text: &str) -> Option<(u16, u16)> {
    (0..buffer.area.height).find_map(|y| find_ascii_cells(buffer, y, text).map(|x| (x, y)))
}

fn find_text_cell_on_line(
    buffer: &ratatui::buffer::Buffer,
    text: &str,
    line_marker: &str,
) -> Option<(u16, u16)> {
    (0..buffer.area.height).find_map(|y| {
        let line = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>();
        line.contains(line_marker)
            .then(|| find_ascii_cells(buffer, y, text).map(|x| (x, y)))
            .flatten()
    })
}

fn completion_app(sql: &str, replace: TextRange, label: &str) -> App {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.update(Action::ReplaceEditor(String::new()));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste(sql.into()));
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: vec![CompletionCandidate {
            label: label.into(),
            insert_text: label.into(),
            kind: CompletionKind::Table,
            detail: Some("(main)".into()),
            replace,
            score: CompletionScore {
                context: 3,
                name_match: 2,
                schema: 1,
            },
        }],
        selected: 0,
    });
    app
}

fn completion_app_with_details(sql: &str, replace: TextRange, rows: &[(&str, &str)]) -> App {
    let mut app = completion_app(sql, replace, rows[0].0);
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: rows
            .iter()
            .map(|(label, detail)| CompletionCandidate {
                label: (*label).into(),
                insert_text: (*label).into(),
                kind: CompletionKind::Column,
                detail: (!detail.is_empty()).then(|| (*detail).to_owned()),
                replace,
                score: CompletionScore {
                    context: 3,
                    name_match: 2,
                    schema: 1,
                },
            })
            .collect(),
        selected: 0,
    });
    app
}

#[test]
fn workspace_tabs_use_content_icons_instead_of_sequence_numbers() {
    let mut app = fixture();
    app.active_console_mut().name = "console_1".into();
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));

    let output = render_with_icons(&app, 120, 30, IconSet::new(IconMode::Ascii)).0;

    assert!(output.contains("SQ console_1"), "{output}");
    assert!(output.contains("TB users"), "{output}");
    assert!(!output.contains("01 console_1"), "{output}");
    assert!(!output.contains("02 users"), "{output}");
}

#[test]
fn sql_tab_icon_prefers_its_bound_profile_over_the_active_connection() {
    let postgres = import_connection_url("postgres://localhost/app", Some("postgres"))
        .unwrap()
        .profile;
    let mut app = fixture();
    app.profiles.push(postgres.clone());
    app.active_console_mut().name = "console_1".into();
    app.active_console_mut().execution_target = Some(ExecutionTarget::from_profile(&postgres));

    let output = render_with_icons(&app, 120, 30, IconSet::new(IconMode::Ascii)).0;

    assert!(output.contains("PG console_1"), "{output}");
    assert!(!output.contains("SQ console_1"), "{output}");
}

#[test]
fn explorer_uses_selected_icon_mode() {
    let app = fixture();

    let nerd = render_with_icons(&app, 120, 36, IconSet::new(IconMode::NerdFont)).0;
    assert!(nerd.contains(nerd_font_symbols::dev::DEV_SQLITE), "{nerd}");

    let unicode = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Unicode)).0;
    assert!(unicode.contains("SQ "), "{unicode}");

    let ascii = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii)).0;
    assert!(ascii.contains("SQ "), "{ascii}");
}

#[test]
fn active_profile_explorer_renders_normalized_group_permission_and_load_more_rows() {
    let profile = import_connection_url(":memory:", Some("catalog-ui"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.connection.profile_id = Some(profile.id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["app"]),
        QualifiedName {
            database: Some("app".into()),
            schema: None,
            object: "app".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        CatalogId::new(profile.id, CatalogKind::Schema, ["app", "public"]),
        database.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "public".into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let owner = ExplorerOwnerId::Group {
        parent: schema.id.clone(),
        group: ObjectGroup::Tables,
    };
    let state = app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap();
    state
        .catalog
        .insert_subtree(vec![database.clone(), schema.clone()])
        .unwrap();
    state
        .catalog
        .set_group_state(
            &schema.id,
            ObjectGroup::Tables,
            CatalogGroupState {
                count: CatalogCount::Exact(2),
                completeness: lazydb::db::catalog::CatalogCompleteness::Partial,
            },
        )
        .unwrap();
    state.load_states.insert(
        owner.clone(),
        ExplorerLoadState::PermissionDenied { request_id: 4 },
    );
    state
        .load_errors
        .insert(owner.clone(), "permission denied".into());
    app.explorer.normalized.expanded.extend([
        ExplorerNodeId::Catalog(database.id),
        ExplorerNodeId::Catalog(schema.id.clone()),
        ExplorerNodeId::Group {
            parent: schema.id.clone(),
            group: ObjectGroup::Tables,
        },
    ]);
    app.explorer.rebuild_projection(profile.id);
    let permission = render(&app, 120, 36);
    assert!(permission.contains("Tables"));
    assert!(permission.contains("Permission"), "{permission}");

    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .load_states
        .insert(
            owner,
            ExplorerLoadState::Loaded {
                next_cursor: Some(CatalogCursor::from_keyset("users", "users").unwrap()),
            },
        );
    app.explorer.rebuild_projection(profile.id);
    let load_more = render(&app, 120, 36);
    assert!(load_more.contains("Load more..."));
}

#[test]
fn explorer_find_keeps_group_counts_and_column_metadata_visible() {
    let profile = import_connection_url(":memory:", Some("catalog-search-ui"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.focus = Focus::Explorer;
    app.connection.profile_id = Some(profile.id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;

    let database_id = CatalogId::new(profile.id, CatalogKind::Database, ["app"]);
    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["app"]),
        QualifiedName {
            database: Some("app".into()),
            schema: None,
            object: "app".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        CatalogId::new(profile.id, CatalogKind::Schema, ["app", "public"]),
        database.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "public".into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let table = CatalogEntry::relation(
        CatalogId::new(profile.id, CatalogKind::Table, ["users"]),
        schema.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "users".into(),
        },
        "table",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let column = CatalogEntry::relation_child(
        CatalogId::new(profile.id, CatalogKind::Column, ["users", "id"]),
        table.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "id".into(),
        },
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
    )
    .unwrap();
    let profile_state = app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap();
    profile_state
        .catalog
        .insert_subtree(vec![database, schema.clone(), table, column])
        .unwrap();
    profile_state
        .catalog
        .set_group_state(
            &schema.id,
            ObjectGroup::Tables,
            CatalogGroupState {
                count: CatalogCount::Exact(2),
                completeness: CatalogCompleteness::Complete,
            },
        )
        .unwrap();
    app.explorer.normalized.expanded.extend([
        ExplorerNodeId::Profile(profile.id),
        ExplorerNodeId::Catalog(database_id.clone()),
        ExplorerNodeId::Catalog(schema.id.clone()),
        ExplorerNodeId::Group {
            parent: schema.id,
            group: ObjectGroup::Tables,
        },
        ExplorerNodeId::Catalog(CatalogId::new(profile.id, CatalogKind::Table, ["users"])),
    ]);
    app.explorer.rebuild_projection(profile.id);
    app.update(Action::ExplorerFindOpen);
    app.update(Action::ExplorerFindInsert('u'));
    let output = render(&app, 120, 36);

    assert!(output.contains("Tables  2"), "{output}");
    assert!(output.contains("users"), "{output}");
    assert!(output.contains("id  bigint"), "{output}");
}

#[test]
fn explorer_width_is_adaptive_and_clamped_in_split_layouts() {
    let app = fixture();
    for (width, expected) in [(120, 40), (180, 56), (300, 56)] {
        let (_, state) = render_with_state(&app, width, 36);
        let explorer = state
            .hit_regions
            .iter()
            .find(|region| region.target == HitTarget::Focus(Focus::Explorer))
            .unwrap();
        assert_eq!(explorer.area.width, expected);
    }
}

#[test]
fn explorer_hostile_metadata_is_sanitized_and_name_type_stay_first() {
    let profile = import_connection_url("sqlite::memory:", Some("safe\x1b[31m-name"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.focus = Focus::Explorer;
    app.connection.profile_id = Some(profile.id);
    app.connection.status = ConnectionStatus::Connected;
    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["db\x1b[31m"]),
        QualifiedName {
            database: Some("db\x1b[31m".into()),
            schema: None,
            object: "db\x1b[31m".into(),
        },
        "db\x1b[31m",
        OptionalMetadata::Supported(Some("comment\x1b[2J".into())),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog
        .insert(database)
        .unwrap();
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(profile.id));
    app.explorer.rebuild_projection(profile.id);
    for (width, height) in [(80, 24), (120, 36), (180, 50)] {
        let output = render(&app, width, height);
        assert!(!output.contains('\x1b'));
        assert!(output.contains("safe<ESC>[31m-name"));
        assert!(output.contains("db<ESC>[31m"));
    }
}

#[test]
fn relation_hostile_title_is_sanitized_in_placeholder() {
    let mut app = App::new(Vec::new());
    let raw_title = "users\x1b[31m\x07";
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new(raw_title)));
    app.active_tab = 1;
    app.focus = Focus::Results;
    assert_eq!(app.tabs[1].title(), raw_title);

    let output = render(&app, 80, 24);

    assert!(!output.contains('\x1b'));
    assert!(output.contains("users<ESC>[31m<0x07>"));
}

#[test]
fn relation_title_is_bounded_in_workspace_tab_bar() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new(format!(
            "{}END",
            "x".repeat(200)
        ))));
    app.active_tab = 1;
    let output = render(&app, 120, 36);
    assert!(!output.contains(&"x".repeat(200)));
    assert!(output.contains(&"x".repeat(48)));
}

#[test]
fn relation_loading_with_previous_snapshot_keeps_data_visible_and_exposes_cancel() {
    let mut app = fixture();
    let mut relation = RelationTab::new("users");
    relation.data = lazydb::model::relation::RelationLoad::Loading {
        request: lazydb::model::relation::RelationRequest {
            tab_id: relation.id,
            tab_generation: relation.generation,
            request_id: 1,
            connection: lazydb::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 0,
            },
            relation: relation.descriptor.key.clone(),
            kind: lazydb::model::relation::RelationRequestKind::Preview,
            scope: lazydb::profile::CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
            options: lazydb::model::relation::RelationPreviewOptions::default(),
            page: lazydb::model::pagination::PageRequest::first(
                lazydb::model::pagination::PageSize::default(),
            ),
        },
        previous: Some(lazydb::model::relation::OwnedSnapshot::new(
            lazydb::db::RelationPreview {
                sql: "SELECT previous".into(),
                result: app.active_console().outcome.clone().unwrap(),
                pagination: lazydb::model::pagination::ResultPagination::from_page(
                    lazydb::model::pagination::PageRequest::first(
                        lazydb::model::pagination::PageSize::default(),
                    ),
                    0,
                ),
            },
            lazydb::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 0,
            },
            lazydb::profile::CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
        )),
    };
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;
    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("RELATION DATA"), "{output}");
    assert!(output.contains("Refreshing relation data"), "{output}");
    assert!(output.contains("showing previous snapshot"), "{output}");
    assert!(output.contains("Ada"), "{output}");
    assert_eq!(state.grid_viewport.unwrap().tab_id, app.tabs[1].id());
}

#[test]
fn relation_first_load_uses_quiet_status_without_dense_skeleton() {
    let mut app = fixture();
    let mut relation = RelationTab::new("users");
    relation.data = lazydb::model::relation::RelationLoad::Loading {
        request: lazydb::model::relation::RelationRequest {
            tab_id: relation.id,
            tab_generation: relation.generation,
            request_id: 1,
            connection: lazydb::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 0,
            },
            relation: relation.descriptor.key.clone(),
            kind: lazydb::model::relation::RelationRequestKind::Preview,
            scope: lazydb::profile::CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
            options: lazydb::model::relation::RelationPreviewOptions::default(),
            page: lazydb::model::pagination::PageRequest::first(
                lazydb::model::pagination::PageSize::default(),
            ),
        },
        previous: None,
    };
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;
    app.focus = Focus::Results;

    let (output, state) = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));

    assert!(output.contains("Loading relation data"), "{output}");
    assert!(!output.chars().any(|c| matches!(c, '░' | '▒' | '▓' | '█')));
    assert!(state.grid_viewport.is_none());
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::RelationCancel)
    );
}

#[test]
fn empty_relation_preview_renders_clean_empty_state() {
    let mut app = fixture();
    let connection = app.connection.active_identity().unwrap();
    let mut outcome = app.active_console().outcome.clone().unwrap();
    outcome.result_sets[0].rows.clear();
    let mut relation = RelationTab::new("users");
    relation.data =
        lazydb::model::relation::RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot::new(
            lazydb::db::RelationPreview {
                sql: "SELECT id, name, active FROM users".into(),
                result: outcome,
                pagination: lazydb::model::pagination::ResultPagination::from_page(
                    lazydb::model::pagination::PageRequest::first(
                        lazydb::model::pagination::PageSize::default(),
                    ),
                    0,
                ),
            },
            connection,
            lazydb::profile::CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
        ));
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;
    app.focus = Focus::Results;

    let output = render(&app, 120, 36);
    let lines = output.lines().collect::<Vec<_>>();
    let header_y = lines
        .iter()
        .position(|line| line.contains("id") && line.contains("#"))
        .expect("grid header line");
    let header_line = lines[header_y];
    let empty_line = lines
        .iter()
        .skip(header_y + 1)
        .find(|line| line.contains("No rows"))
        .expect("empty state after the grid header");
    let header_chars = header_line.chars().collect::<Vec<_>>();
    let row_number_x = header_chars
        .iter()
        .position(|character| *character == '#')
        .expect("row-number header");
    let grid_left = header_chars[..row_number_x]
        .iter()
        .rposition(|character| *character == '│')
        .expect("grid left border");
    let grid_right = header_chars
        .iter()
        .rposition(|character| *character == '│')
        .expect("grid right border");
    let empty_chars = empty_line.chars().collect::<Vec<_>>();

    assert!(output.contains('─'), "{output}");
    assert!(empty_line.contains("No rows"), "{output}");
    assert!(
        empty_chars[grid_left + 1..grid_right]
            .iter()
            .all(|character| !matches!(character, '│' | '▌')),
        "{output}"
    );
}

#[test]
fn relation_query_completion_is_anchored_to_active_input() {
    let mut app = fixture();
    let mut relation = RelationTab::new("users");
    relation.data =
        lazydb::model::relation::RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot::new(
            lazydb::db::RelationPreview {
                sql: "SELECT * FROM users".into(),
                result: app.active_console().outcome.clone().unwrap(),
                pagination: lazydb::model::pagination::ResultPagination::from_page(
                    lazydb::model::pagination::PageRequest::first(
                        lazydb::model::pagination::PageSize::default(),
                    ),
                    0,
                ),
            },
            lazydb::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 0,
            },
            lazydb::profile::CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
        ));
    relation.query.focus = Some(DataQueryInput::Where);
    relation.query.where_input.set("userid");
    relation.query.completion = Some(DataQueryCompletion {
        candidates: vec![DataQueryCandidate {
            name: "user_id".into(),
            type_name: Some("bigint\x1b[31m".into()),
        }],
        selected: 0,
        replace: TextRange::new(0, 6),
    });
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;
    app.focus = Focus::Results;

    let (output, state) = render_with_state(&app, 120, 36);
    let popup = state.completion_popup.unwrap();

    assert!(output.contains("user_id"), "{output}");
    assert!(output.contains("bigint<ESC>[31m"), "{output}");
    assert!(!output.contains('\x1b'));
    assert!(popup.right() <= 120);
    assert!(popup.bottom() <= 36);
}

#[test]
fn relation_page_renders_data_ddl_selectors_and_relation_layout() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;

    for (width, height) in [(80, 24), (120, 36), (180, 50)] {
        let (output, state) = render_with_state(&app, width, height);
        assert!(output.contains("DATA"), "{width}x{height}: {output}");
        assert!(output.contains("DDL"), "{width}x{height}: {output}");
        assert!(!output.contains("STRUCTURE"), "{width}x{height}: {output}");
        if width >= 100 {
            assert!(output.contains("EXPLORER"), "{width}x{height}: {output}");
        }
        assert!(state.hit_regions.iter().any(|region| region.target
            == HitTarget::RelationView(lazydb::model::relation::RelationView::Data)));
        assert!(state.hit_regions.iter().any(|region| region.target
            == HitTarget::RelationView(lazydb::model::relation::RelationView::Ddl)));
        let pane = state
            .hit_regions
            .iter()
            .find(|region| region.target == HitTarget::Focus(Focus::Results))
            .expect("relation pane focus region");
        assert!(pane.area.width > 0);
        assert!(pane.area.height > 0);

        let selector = state
            .hit_regions
            .iter()
            .find(|region| {
                region.target
                    == HitTarget::RelationView(lazydb::model::relation::RelationView::Data)
            })
            .expect("data selector");
        assert_eq!(
            state.target_at(selector.area.x, selector.area.y),
            Some(&selector.target)
        );
    }
}

#[test]
fn empty_relation_ddl_uses_the_ddl_panel_empty_state() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.update(Action::SetRelationView(
        lazydb::model::relation::RelationView::Ddl,
    ));
    let output = render(&app, 120, 36);
    assert!(output.contains("RELATION DDL"), "{output}");
    assert!(output.contains("No DDL available"), "{output}");
}

fn ready_relation_ddl_fixture() -> App {
    let mut app = fixture();
    let connection = app.connection.active_identity().unwrap();
    let profile_id = connection.profile_id;
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, ["db", "main"]);
    let relation_id = CatalogId::new(profile_id, CatalogKind::Table, ["db", "main", "users"]);
    let relation_entry = CatalogEntry::relation(
        relation_id.clone(),
        schema_id,
        QualifiedName {
            database: Some("db".into()),
            schema: Some("main".into()),
            object: "users".into(),
        },
        "table",
        OptionalMetadata::Unsupported,
        false,
    )
    .unwrap();
    let children = CatalogPage {
        key: CatalogRequestKey {
            connection,
            catalog_epoch: 0,
            request_id: 1,
            target: CatalogTarget::RelationChildren {
                relation: relation_id,
            },
            cursor: None,
        },
        entries: Vec::new(),
        group_summaries: Vec::new(),
        total_count: CatalogCount::Exact(0),
        next_cursor: None,
        completeness: CatalogCompleteness::Complete,
    };
    let mut relation = RelationTab::new("users");
    relation.view = lazydb::model::relation::RelationView::Ddl;
    relation.ddl =
        lazydb::model::relation::RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot::new(
            RelationDdl {
                relation: relation_entry,
                children,
                sql: "CREATE TABLE users (\n  id INTEGER PRIMARY KEY\n);".into(),
                provenance: DdlProvenance::NativeCatalog,
            },
            connection,
            app.active_profile().unwrap().catalog_scope.clone(),
        ));
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    app
}

#[test]
fn relation_ddl_context_is_rendered_on_the_panel_border() {
    let app = ready_relation_ddl_fixture();
    let (buffer, _) = render_buffer_with_icons(&app, 120, 36, IconSet::default());

    let (_, title_y) = find_text_cell(&buffer, "RELATION DDL").expect("DDL title");
    let (_, source_y) = find_text_cell(&buffer, "NATIVE CATALOG").expect("DDL source");
    let (_, snapshot_y) = find_text_cell(&buffer, "LIVE").expect("snapshot provenance");

    assert_eq!(source_y, title_y);
    assert_eq!(snapshot_y, title_y);
    let output = render(&app, 120, 36);
    let footer = output.lines().last().unwrap();
    assert!(!footer.contains("DDL:"), "{output}");
    assert!(!footer.contains("Snapshot:"), "{output}");
    assert!(output.contains("ROW 1"), "{output}");
    assert!(output.contains("COL 1"), "{output}");
}

#[test]
fn relation_ddl_offline_snapshot_remains_visible() {
    let mut app = ready_relation_ddl_fixture();
    let tab = match &mut app.tabs[app.active_tab] {
        WorkspaceTab::Relation(tab) => tab,
        _ => unreachable!(),
    };
    let snapshot = match &mut tab.ddl {
        lazydb::model::relation::RelationLoad::Ready(snapshot) => snapshot,
        _ => unreachable!(),
    };
    snapshot.attribution.connection.generation += 1;

    let (buffer, _) = render_buffer_with_icons(&app, 120, 36, IconSet::default());
    let (_, title_y) = find_text_cell(&buffer, "RELATION DDL").expect("DDL title");
    let (_, snapshot_y) =
        find_text_cell(&buffer, "OFFLINE SNAPSHOT").expect("offline snapshot provenance");

    assert_eq!(snapshot_y, title_y);
}

#[test]
fn relation_ddl_long_provenance_does_not_replace_the_title_when_narrow() {
    let mut app = ready_relation_ddl_fixture();
    app.profiles.clear();

    let output = render(&app, 56, 24);

    assert!(output.contains("RELATION DDL"), "{output}");
    assert!(output.contains("PROFILE DELETED SNAPSHOT"), "{output}");
}

#[test]
fn relation_page_renders_contextual_help_overlay() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;
    app.overlay = Some(Overlay::Help(lazydb::help::HelpState::new(
        lazydb::help::ShortcutContext::RelationDataBrowse,
        lazydb::help::ShortcutCapabilities::relation_data(),
    )));

    let (output, state) = render_with_state(&app, 120, 36);

    assert!(output.contains("KEYMAP // RESULTS"), "{output}");
    assert!(!state.hit_regions.is_empty());
}

#[test]
fn explorer_metadata_keeps_name_type_before_flags_and_comments() {
    let profile = import_connection_url(":memory:", Some("metadata-order"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["db"]),
        QualifiedName {
            database: Some("db".into()),
            schema: None,
            object: "db".into(),
        },
        "DATABASE",
        OptionalMetadata::Supported(Some("database comment".into())),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog
        .insert(database)
        .unwrap();
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(profile.id));
    app.explorer.rebuild_projection(profile.id);
    let row = app.explorer.visible().into_iter().find(
        |row| matches!(row.id, ExplorerNodeId::Catalog(ref id) if id.kind == CatalogKind::Database),
    );
    let row = row.unwrap();
    assert_eq!(row.label, "db");
    assert_eq!(row.metadata, None);
    assert_eq!(row.comment.as_deref(), Some("database comment"));
}

#[test]
fn explorer_local_status_rows_render_at_supported_sizes() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    for (width, height) in [(80, 24), (120, 36), (180, 50)] {
        let output = render(&app, width, height);
        assert!(output.contains("No profiles"), "{width}x{height}: {output}");
    }
}

#[test]
fn editor_prompt_is_rendered_as_inert_display_text() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste("run\x1b[31m".into()));
    let rendered = render(&app, 120, 30);
    assert!(rendered.contains(":run<ESC>[31m"));
    assert!(!rendered.contains('\x1b'));
}

#[test]
fn execution_confirmation_preview_is_sanitized_and_shows_scope() {
    let profile = import_connection_url("sqlite::memory:", Some("preview-db"))
        .unwrap()
        .profile;
    let mut app = App::with_confirmation_policy(vec![profile.clone()], ConfirmationPolicy::Always);
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Sqlite,
            version: "3.50.0".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ReplaceEditor("SELECT 1;\x1b]8;;bad\x07".into()));
    app.update(Action::RunAllSql);

    let output = render(&app, 120, 36);
    assert!(output.contains("EXECUTION CONFIRMATION"));
    assert!(output.contains("FullBuffer"));
    assert!(!output.contains('\x1b'));
}

#[test]
fn completion_candidate_label_aligns_with_identifier_start() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.update(Action::ReplaceEditor(String::new()));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste("SELECT * FROM sys_u".into()));
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: vec![CompletionCandidate {
            label: "sys_user".into(),
            insert_text: "sys_user".into(),
            kind: CompletionKind::Table,
            detail: Some("(main)".into()),
            replace: TextRange::new(14, 19),
            score: CompletionScore {
                context: 3,
                name_match: 2,
                schema: 1,
            },
        }],
        selected: 0,
    });

    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let editor = state
        .hit_regions
        .iter()
        .find_map(|region| {
            (region.target == HitTarget::Focus(Focus::Editor)).then_some(region.area)
        })
        .unwrap();
    let popup = state.completion_popup.unwrap();
    let identifier_x = (editor.y..popup.y)
        .find_map(|y| find_ascii_cells(&buffer, y, "sys_u"))
        .expect("SQL identifier");
    let candidate_x =
        find_ascii_cells(&buffer, popup.y + 1, "sys_user").expect("completion candidate");

    assert_eq!(popup.y, editor.y + 2);
    assert_eq!(candidate_x, identifier_x);
    assert_eq!(buffer[(popup.x, popup.y)].symbol(), "╭");
    assert_eq!(buffer[(popup.right() - 1, popup.y)].symbol(), "╮");
    assert_eq!(buffer[(popup.x, popup.bottom() - 1)].symbol(), "╰");
    assert_eq!(
        buffer[(popup.right() - 1, popup.bottom() - 1)].symbol(),
        "╯"
    );
    assert_eq!(buffer[(popup.x, popup.y)].fg, Color::Rgb(43, 66, 86));
    assert!(popup.right() <= editor.right());
    assert!(popup.bottom() <= editor.bottom());
}

#[test]
fn completion_candidate_labels_share_a_fixed_icon_column() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.update(Action::ReplaceEditor(String::new()));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste("SELECT * FROM sy".into()));
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: vec![
            CompletionCandidate {
                label: "sys_user".into(),
                insert_text: "sys_user".into(),
                kind: CompletionKind::Table,
                detail: None,
                replace: TextRange::new(14, 16),
                score: CompletionScore {
                    context: 3,
                    name_match: 2,
                    schema: 1,
                },
            },
            CompletionCandidate {
                label: "RETURNING".into(),
                insert_text: "RETURNING".into(),
                kind: CompletionKind::Keyword,
                detail: None,
                replace: TextRange::new(14, 16),
                score: CompletionScore {
                    context: 1,
                    name_match: 2,
                    schema: 0,
                },
            },
        ],
        selected: 0,
    });

    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();

    assert_eq!(
        find_ascii_cells(&buffer, popup.y + 1, "sys_user"),
        find_ascii_cells(&buffer, popup.y + 2, "RETURNING")
    );
}

#[test]
fn completion_candidate_label_highlights_an_ordinary_prefix() {
    let app = completion_app("SELECT * FROM sys_u", TextRange::new(14, 19), "sys_user");
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let x = find_ascii_cells(&buffer, popup.y + 1, "sys_user").unwrap();

    assert!(buffer[(x, popup.y + 1)].modifier.contains(Modifier::BOLD));
    assert!(
        buffer[(x + 4, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert!(
        !buffer[(x + 5, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn completion_candidate_label_highlights_a_compact_prefix_without_the_separator() {
    let app = completion_app("SELECT * FROM sysuser", TextRange::new(14, 20), "sys_user");
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let x = find_ascii_cells(&buffer, popup.y + 1, "sys_user").unwrap();

    assert!(buffer[(x, popup.y + 1)].modifier.contains(Modifier::BOLD));
    assert!(
        buffer[(x + 2, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert!(
        !buffer[(x + 3, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert!(
        buffer[(x + 4, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn completion_candidate_label_highlighting_keeps_unicode_boundaries_intact() {
    let app = completion_app("SELECT * FROM 界🙂", TextRange::new(14, 21), "界🙂table");
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let x = (0..buffer.area.width)
        .find(|x| buffer[(*x, popup.y + 1)].symbol() == "界")
        .unwrap();

    assert!(buffer[(x, popup.y + 1)].modifier.contains(Modifier::BOLD));
    assert!(
        buffer[(x + 2, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert!(
        !buffer[(x + 4, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn completion_candidate_label_highlight_preserves_selected_row_contrast() {
    let mut app = completion_app("SELECT * FROM sys_u", TextRange::new(14, 19), "sys_user");
    app.active_console_mut()
        .completion
        .as_mut()
        .unwrap()
        .candidates
        .push(CompletionCandidate {
            label: "other".into(),
            insert_text: "other".into(),
            kind: CompletionKind::Table,
            detail: None,
            replace: TextRange::new(14, 19),
            score: CompletionScore {
                context: 1,
                name_match: 1,
                schema: 0,
            },
        });
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let selected_x = find_ascii_cells(&buffer, popup.y + 1, "sys_user").unwrap();
    let other_x = find_ascii_cells(&buffer, popup.y + 2, "other").unwrap();

    assert_eq!(buffer[(selected_x, popup.y + 1)].fg, Color::Rgb(7, 11, 18));
    assert_eq!(
        buffer[(selected_x, popup.y + 1)].bg,
        Color::Rgb(99, 230, 216)
    );
    assert_ne!(
        buffer[(selected_x, popup.y + 1)].fg,
        buffer[(other_x, popup.y + 2)].fg
    );
    assert!(
        buffer[(selected_x, popup.y + 1)]
            .modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn completion_detail_column_is_right_aligned() {
    // The rows deliberately differ in `label + detail` width: a shared right
    // edge can only come from a real detail column, not from ragged text.
    let app = completion_app_with_details(
        "SELECT * FROM sys_u",
        TextRange::new(14, 19),
        &[("id", "bigint"), ("sys_user", "varchar(200)")],
    );
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let short = find_ascii_cells(&buffer, popup.y + 1, "bigint").expect("short detail");
    let long = find_ascii_cells(&buffer, popup.y + 2, "varchar(200)").expect("long detail");
    let label = find_ascii_cells(&buffer, popup.y + 2, "sys_user").expect("long label");

    let short_end = short + "bigint".len() as u16;
    let long_end = long + "varchar(200)".len() as u16;
    assert_eq!(short_end, long_end, "detail column must be right aligned");
    // 右边框 1 格 + 行尾留白 1 格。
    assert_eq!(long_end, popup.right() - 2);
    // 最长 label 与类型列之间保留最小间距。
    assert!(long >= label + "sys_user".len() as u16 + 2);
}

#[test]
fn completion_selected_row_highlight_spans_the_popup_width() {
    let app = completion_app_with_details(
        "SELECT * FROM sys_u",
        TextRange::new(14, 19),
        // The selected row is the narrow one, so a text-width highlight leaves
        // an obvious gap.
        &[("id", "bigint"), ("sys_user", "varchar(200)")],
    );
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();

    for x in popup.x + 1..popup.right() - 1 {
        assert_eq!(
            buffer[(x, popup.y + 1)].bg,
            Color::Rgb(99, 230, 216),
            "selected row must be a full-width bar at x={x}"
        );
    }
}

#[test]
fn completion_detail_is_dropped_when_the_popup_cannot_fit_it() {
    let app = completion_app_with_details(
        "SELECT * FROM sys_u",
        TextRange::new(14, 19),
        &[("sys_user_created_at_index_name", "varchar(200)")],
    );
    // 56 格是弹框仍会渲染的最窄视口；此时内宽只够 icon 列与 label。
    let (buffer, state) = render_buffer_with_icons(&app, 56, 24, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let label = find_ascii_cells(&buffer, popup.y + 1, "sys_user_created_at_index_name")
        .expect("full label");

    assert!(popup.right() <= 56);
    assert!(find_ascii_cells(&buffer, popup.y + 1, "varchar").is_none());
    // 类型列整列消失，而不是被边框裁成半截。
    for x in label + "sys_user_created_at_index_name".len() as u16..popup.right() - 1 {
        assert_eq!(
            buffer[(x, popup.y + 1)].symbol(),
            " ",
            "detail column must disappear entirely at x={x}"
        );
    }
}

#[test]
fn completion_popup_stays_fixed_while_typing() {
    let cases = [
        ("SELECT * FROM s", TextRange::new(14, 15)),
        ("SELECT * FROM sy", TextRange::new(14, 16)),
        ("SELECT * FROM sys", TextRange::new(14, 17)),
        ("SELECT * FROM sys_", TextRange::new(14, 18)),
        ("SELECT * FROM sys_u", TextRange::new(14, 19)),
    ];
    let mut popup_xs = Vec::new();
    let mut label_xs = Vec::new();
    let mut identifier_xs = Vec::new();

    for (sql, replace) in cases {
        let app = completion_app(sql, replace, "sys_user");
        let (buffer, state) =
            render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
        let popup = state.completion_popup.unwrap();
        let identifier = &sql[replace.start..replace.end];
        let source_needle = format!("FROM {identifier}");
        popup_xs.push(popup.x);
        label_xs.push(find_ascii_cells(&buffer, popup.y + 1, "sys_user").unwrap());
        identifier_xs.push(
            (0..popup.y)
                .find_map(|y| find_ascii_cells(&buffer, y, &source_needle))
                .map(|x| x + "FROM ".len() as u16)
                .unwrap(),
        );
    }

    assert!(popup_xs.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(label_xs.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(label_xs, identifier_xs);
}

#[test]
fn completion_popup_keeps_origin_when_candidate_width_changes() {
    let sql = "SELECT * FROM                         sys_u";
    let start = sql.find("sys_u").unwrap();
    let replace = TextRange::new(start, start + "sys_u".len());
    let short = completion_app(sql, replace, "sys_user");
    let long = completion_app(
        sql,
        replace,
        "sys_user_with_a_candidate_name_that_exceeds_the_viewport",
    );

    let (_, short_state) = render_with_state(&short, 80, 24);
    let (_, long_state) = render_with_state(&long, 80, 24);
    let short_popup = short_state.completion_popup.unwrap();
    let long_popup = long_state.completion_popup.unwrap();

    assert_eq!(short_popup.x, long_popup.x);
    assert!(short_popup.right() <= 80);
    assert!(long_popup.right() <= 80);
    assert!(long_popup.width >= short_popup.width);
}

#[test]
fn completion_label_alignment_handles_multiline_tabs_and_wide_characters() {
    let sql = "SELECT '界🙂';\n\tFROM sys_u";
    let start = sql.find("sys_u").unwrap();
    let app = completion_app(
        sql,
        TextRange::new(start, start + "sys_u".len()),
        "sys_user",
    );
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let identifier_x = (0..popup.y)
        .find_map(|y| find_ascii_cells(&buffer, y, "FROM sys_u"))
        .map(|x| x + "FROM ".len() as u16)
        .unwrap();
    let candidate_x = find_ascii_cells(&buffer, popup.y + 1, "sys_user").unwrap();

    assert_eq!(candidate_x, identifier_x);
}

#[test]
fn completion_popup_stays_in_editor_when_identifier_starts_at_column_zero() {
    let app = completion_app("sys_u", TextRange::new(0, 5), "sys_user");
    let (buffer, state) = render_buffer_with_icons(&app, 80, 24, IconSet::new(IconMode::Ascii));
    let editor = state
        .hit_regions
        .iter()
        .find_map(|region| {
            (region.target == HitTarget::Focus(Focus::Editor)).then_some(region.area)
        })
        .unwrap();
    let popup = state.completion_popup.unwrap();
    let identifier_x = (editor.y..popup.y)
        .find_map(|y| find_ascii_cells(&buffer, y, "sys_u"))
        .unwrap();
    let candidate_x = find_ascii_cells(&buffer, popup.y + 1, "sys_user").unwrap();

    assert_eq!(candidate_x, identifier_x);
    assert!(popup.x >= editor.x);
    assert!(popup.right() <= editor.right());
}

#[test]
fn completion_label_alignment_supports_every_icon_mode() {
    let sql = "SELECT * FROM sys_u";
    let start = sql.find("sys_u").unwrap();
    let app = completion_app(
        sql,
        TextRange::new(start, start + "sys_u".len()),
        "sys_user",
    );

    for mode in [IconMode::NerdFont, IconMode::Unicode, IconMode::Ascii] {
        let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(mode));
        let popup = state.completion_popup.unwrap();
        let identifier_x = (0..popup.y)
            .find_map(|y| find_ascii_cells(&buffer, y, "FROM sys_u"))
            .map(|x| x + "FROM ".len() as u16)
            .unwrap();
        let candidate_x = find_ascii_cells(&buffer, popup.y + 1, "sys_user").unwrap();

        assert_eq!(candidate_x, identifier_x, "icon mode: {mode:?}");
    }
}

#[test]
fn completion_popup_is_not_rendered_in_normal_mode() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: vec![CompletionCandidate {
            label: "app.public.users".into(),
            insert_text: "users".into(),
            kind: CompletionKind::Table,
            detail: None,
            replace: TextRange::new(0, 0),
            score: CompletionScore {
                context: 3,
                name_match: 2,
                schema: 1,
            },
        }],
        selected: 0,
    });

    let (_, state) = render_with_state(&app, 120, 36);

    assert!(state.completion_popup.is_none());
}

#[test]
fn cursor_style_follows_editor_mode() {
    let mut app = fixture();
    let (_, normal_state) = render_with_state(&app, 120, 36);
    assert_eq!(
        normal_state.cursor_style,
        Some(lazydb::ui::CursorStyle::Block)
    );

    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    let (_, insert_state) = render_with_state(&app, 120, 36);
    assert_eq!(
        insert_state.cursor_style,
        Some(lazydb::ui::CursorStyle::Bar)
    );

    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    let (_, returned_normal_state) = render_with_state(&app, 120, 36);
    assert_eq!(
        returned_normal_state.cursor_style,
        Some(lazydb::ui::CursorStyle::Block)
    );
}

#[test]
fn editor_prompt_uses_bar_cursor() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::NONE,
    )));

    let (_, state) = render_with_state(&app, 120, 36);
    assert_eq!(state.cursor_style, Some(lazydb::ui::CursorStyle::Bar));
}

#[test]
fn replace_mode_uses_underline_cursor() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('R'),
        KeyModifiers::NONE,
    )));

    let (_, state) = render_with_state(&app, 120, 36);
    assert_eq!(state.cursor_style, Some(lazydb::ui::CursorStyle::Underline));
}

#[test]
fn footer_and_header_show_transaction_state_and_controls() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    let (output, _) = render_with_state(&app, 120, 36);
    assert!(output.contains("TX MANUAL:ACTIVE"));
}

#[test]
fn editor_title_owns_target_and_transaction_context() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    let (output, _) = render_with_state(&app, 120, 36);
    assert!(output.contains("orbital-lab"));
    assert!(output.contains("TX MANUAL:ACTIVE"));
    assert!(!output.contains("TX AUTO"));
    assert!(!output.lines().take(2).any(|line| line.contains("TX ")));
}

#[test]
fn editor_help_documents_target_context_controls() {
    let mut app = fixture();
    app.update(Action::Focus(Focus::Editor));
    app.update(Action::ShowHelp);
    let (output, _) = render_with_state(&app, 120, 40);
    for text in ["Space d", "Space f", "Space tt", "Space tc"] {
        assert!(output.contains(text), "missing {text}");
    }
    assert!(!output.contains("Space tr"));
    assert!(output.contains("Search"));
    assert!(!output.contains(":connection"));
    assert!(!output.contains(":database"));
    assert!(!output.contains(":schema"));
}

#[test]
fn relation_help_documents_transaction_control_panel() {
    let mut app = fixture();
    let mut relation = RelationTab::new("users");
    relation.edit =
        Some(lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![]]));
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;
    app.focus = Focus::Results;
    app.update(Action::ShowHelp);

    let output = render(&app, 120, 40);
    assert!(output.contains("Space tc"));
    assert!(output.contains("commit or roll back transaction"));
}

#[test]
fn quit_panel_uses_compact_transaction_summary_layout() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.update(Action::NewConsole);
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Aborted;

    assert!(app.update(Action::Quit).is_empty());
    let output = render(&app, 100, 30);

    assert_eq!(
        output.matches("PENDING TRANSACTIONS").count(),
        1,
        "{output}"
    );
    assert!(output.contains("TRANSACTION SUMMARY"), "{output}");
    assert!(output.contains("ACTIVE"), "{output}");
    assert!(output.contains("ABORTED"), "{output}");
    assert!(!output.contains("Active"), "{output}");
    assert!(!output.contains("Aborted"), "{output}");
    assert!(output.contains("Commit"), "{output}");
    assert!(output.contains("Rollback"), "{output}");
    assert!(output.contains("Esc cancel"), "{output}");
    assert!(!output.contains("Rollback is the default"), "{output}");
}

#[test]
fn quit_panel_highlights_rollback_as_the_default_action() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    assert!(app.update(Action::Quit).is_empty());

    let (buffer, _) = render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (rollback_x, rollback_y) = find_text_cell(&buffer, "Rollback").expect("rollback action");
    let (commit_x, commit_y) = find_text_cell(&buffer, "Commit").expect("commit action");

    assert_eq!(
        buffer[(rollback_x, rollback_y)].bg,
        Color::Rgb(99, 230, 216)
    );
    assert!(
        buffer[(rollback_x, rollback_y)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert_ne!(
        buffer[(commit_x, commit_y)].bg,
        buffer[(rollback_x, rollback_y)].bg
    );
}

#[test]
fn transaction_panel_keeps_the_title_out_of_the_body() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.update(Action::OpenTransactionControl);

    let output = render(&app, 100, 30);
    let title_line = output
        .lines()
        .find(|line| line.contains(" TRANSACTION "))
        .expect("transaction border title");

    assert!(title_line.contains('─'), "{output}");
    assert!(output.contains("TRANSACTION SUMMARY"), "{output}");
    assert_eq!(
        output
            .lines()
            .filter(|line| line.trim() == "TRANSACTION")
            .count(),
        0,
        "{output}"
    );
}

#[test]
fn quit_panel_marks_the_current_transaction_and_colors_states() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.update(Action::NewConsole);
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Aborted;
    assert!(app.update(Action::Quit).is_empty());

    let (buffer, _) = render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (active_x, active_y) =
        find_text_cell_on_line(&buffer, "ACTIVE", "›").expect("active state");
    let (aborted_x, aborted_y) =
        find_text_cell_on_line(&buffer, "ABORTED", "  console").expect("aborted state");
    let marker_x = (0..active_x)
        .rev()
        .find(|x| buffer[(*x, active_y)].symbol() == "›")
        .expect("current transaction marker");

    assert_ne!(
        buffer[(active_x, active_y)].fg,
        buffer[(aborted_x, aborted_y)].fg
    );
    assert_ne!(
        buffer[(marker_x, active_y)].fg,
        buffer[(active_x, active_y)].fg
    );
}

#[test]
fn quit_panel_disables_commit_for_an_aborted_transaction() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Aborted;
    assert!(app.update(Action::Quit).is_empty());

    let (buffer, _) = render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (commit_x, commit_y) =
        find_text_cell_on_line(&buffer, "Commit", "[ Commit ]").expect("commit action");
    let (rollback_x, rollback_y) =
        find_text_cell_on_line(&buffer, "Rollback", "[ Rollback ]").expect("rollback action");

    assert_ne!(buffer[(commit_x, commit_y)].bg, Color::Rgb(99, 230, 216));
    assert_eq!(
        buffer[(rollback_x, rollback_y)].bg,
        Color::Rgb(99, 230, 216)
    );
}

#[test]
fn quit_panel_moves_selection_style_to_commit() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    assert!(app.update(Action::Quit).is_empty());
    app.update(Action::ToggleTransactionExitChoice);

    let (buffer, _) = render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (commit_x, commit_y) =
        find_text_cell_on_line(&buffer, "Commit", "[ Commit ]").expect("commit action");
    let (rollback_x, rollback_y) =
        find_text_cell_on_line(&buffer, "Rollback", "[ Rollback ]").expect("rollback action");

    assert_eq!(buffer[(commit_x, commit_y)].bg, Color::Rgb(99, 230, 216));
    assert_ne!(
        buffer[(rollback_x, rollback_y)].bg,
        Color::Rgb(99, 230, 216)
    );
}

#[test]
fn quit_panel_replaces_transaction_actions_while_query_is_running() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.active_console_mut().query_status = QueryStatus::Running;
    let (console_id, transaction_generation) = {
        let console = app.active_console();
        (console.id, console.transaction_generation)
    };
    app.overlay = Some(Overlay::TransactionExitConfirm {
        prompt: lazydb::model::transaction::DeferredTransactionPrompt {
            console_id,
            transaction_generation,
            intent: lazydb::model::transaction::DeferredIntent::Quit,
        },
        choice: lazydb::model::transaction::TransactionExitChoice::Rollback,
    });

    let output = render(&app, 100, 30);

    assert!(output.contains("QUERY IN PROGRESS"), "{output}");
    assert!(output.contains("wait or Ctrl-C to cancel"), "{output}");
    assert!(!output.contains("[ Commit ]"), "{output}");
    assert!(!output.contains("[ Rollback ]"), "{output}");
    assert!(output.contains("Esc return"), "{output}");
}

#[test]
fn quit_panel_isolates_unknown_outcome_actions() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::OutcomeUnknown;
    let (console_id, transaction_generation) = {
        let console = app.active_console();
        (console.id, console.transaction_generation)
    };
    app.overlay = Some(Overlay::TransactionExitConfirm {
        prompt: lazydb::model::transaction::DeferredTransactionPrompt {
            console_id,
            transaction_generation,
            intent: lazydb::model::transaction::DeferredIntent::Quit,
        },
        choice: lazydb::model::transaction::TransactionExitChoice::Abandon,
    });

    let output = render(&app, 100, 30);

    assert!(output.contains("OUTCOME UNKNOWN"), "{output}");
    assert!(output.contains("Abandon local state"), "{output}");
    assert!(!output.contains("[ Commit ]"), "{output}");
    assert!(!output.contains("[ Rollback ]"), "{output}");
    assert!(output.contains("A abandon"), "{output}");
    assert!(output.contains("Esc cancel"), "{output}");
}

#[test]
fn quit_panel_remains_readable_at_minimum_terminal_size() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.active_console_mut().name =
        "a-very-long-console-name-that-must-not-overwrite-the-state".into();
    assert!(app.update(Action::Quit).is_empty());

    let output = render(&app, 56, 16);

    assert_eq!(
        output.matches("PENDING TRANSACTIONS").count(),
        1,
        "{output}"
    );
    assert!(output.contains("TRANSACTION SUMMARY"), "{output}");
    assert!(output.contains("ACTIVE"), "{output}");
    assert!(output.contains("Rollback"), "{output}");
    assert!(output.contains("Esc cancel"), "{output}");
    assert!(output.contains('╭'), "{output}");
    assert!(output.contains('╮'), "{output}");
    assert!(output.contains('╰'), "{output}");
    assert!(output.contains('╯'), "{output}");
}

#[test]
fn editor_context_keeps_transaction_visible_when_narrow() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.active_console_mut().query_status = QueryStatus::Running;
    for width in [120, 80, 56] {
        let output = render(&app, width, 24);
        assert!(
            output.contains("TX MANUAL:ACTIVE"),
            "width={width}: {output}"
        );
        assert!(output.contains("QUERY RUNNING"), "width={width}: {output}");
    }
}

#[test]
fn sql_editor_border_only_shows_non_idle_query_status() {
    let cases = [
        (QueryStatus::Idle, None),
        (QueryStatus::Running, Some("QUERY RUNNING")),
        (QueryStatus::Cancelled, Some("QUERY CANCELLED")),
        (QueryStatus::Failed, Some("QUERY ERROR")),
    ];

    for (status, expected) in cases {
        let mut app = fixture();
        app.focus = Focus::Editor;
        app.active_console_mut().query_status = status;
        let output = render(&app, 120, 36);

        if let Some(expected) = expected {
            assert!(output.contains(expected), "status={status:?}: {output}");
            assert_eq!(output.matches(expected).count(), 1, "{output}");
        } else {
            assert!(!output.contains("QUERY IDLE"), "{output}");
            assert!(!output.contains("QUERY RUNNING"), "{output}");
            assert!(!output.contains("QUERY CANCELLED"), "{output}");
            assert!(!output.contains("QUERY ERROR"), "{output}");
        }
    }
}

#[test]
fn running_query_status_is_rendered_on_the_editor_top_border() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.active_console_mut().query_status = QueryStatus::Running;

    let (buffer, _) = render_buffer_with_icons(&app, 120, 36, IconSet::default());
    let (_, editor_y) = find_text_cell(&buffer, "SQL EDITOR").expect("editor title");
    let (status_x, status_y) = find_text_cell(&buffer, "QUERY RUNNING").expect("query status");

    assert_eq!(status_y, editor_y);
    assert_eq!(buffer[(status_x, status_y)].fg, Color::Rgb(101, 167, 255));
}

#[test]
fn narrow_editor_preserves_title_cancelled_query_and_transaction() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.active_console_mut().query_status = QueryStatus::Cancelled;

    let output = render(&app, 56, 24);

    assert!(output.contains("SQL EDITOR"), "{output}");
    assert!(output.contains("QUERY CANCELLED"), "{output}");
    assert!(output.contains("TX MANUAL:ACTIVE"), "{output}");
}

#[test]
fn standard_layout_shows_stable_workspace_regions() {
    let output = render(&fixture(), 120, 36);

    assert!(output.contains("LAZYDB"));
    assert!(output.contains("orbital-lab"));
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("console"));
    assert!(output.contains("SELECT"));
    assert!(output.contains("DATA"));
    assert!(output.contains("OUTPUT"));
    assert!(output.contains("Ada"));
    assert!(!output.contains("Ready"), "{output}");
    assert!(!output.contains("QUERY IDLE"), "{output}");
}

#[test]
fn pane_resize_drag_highlights_the_rendered_explorer_border() {
    let app = fixture();
    let (before, state) = render_buffer_with_icons(&app, 120, 36, IconSet::default());
    let border = state
        .hit_regions
        .iter()
        .find(|region| region.target == HitTarget::PaneResize(PaneSplit::ExplorerWidth))
        .unwrap()
        .area;
    assert_ne!(before[(border.x, border.y)].fg, Color::Rgb(101, 167, 255));

    state.pane_resize_drag.replace(Some(PaneResizeDrag {
        split: PaneSplit::ExplorerWidth,
        start_pointer: border.x,
        start_size: state.pane_layout.explorer_width.unwrap(),
    }));
    let (during, state) = render_buffer_with_state(&app, 120, 36, state);

    assert_ne!(
        during[(border.x, border.y)].fg,
        before[(border.x, border.y)].fg
    );
    assert!(
        during[(border.x, border.y)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert!(state.pane_resize_drag.borrow().is_some());
}

#[test]
fn workspace_header_and_footer_render_without_redundant_status_rows() {
    let output = render(&fixture(), 120, 36);
    let lines = output.lines().collect::<Vec<_>>();

    assert!(lines[0].contains("LAZYDB"), "{output}");
    assert!(lines[0].contains("orbital-lab"), "{output}");
    assert!(!output.contains("ONLINE"), "{output}");
    assert!(!output.contains("QUERY IDLE"), "{output}");
    assert!(!output.contains("Ready"), "{output}");
    assert!(
        lines.last().unwrap().contains("NORMAL")
            || lines.last().unwrap().contains("EXPLORE")
            || lines.last().unwrap().contains("DATA"),
        "{output}"
    );
}

#[test]
fn one_row_header_retains_only_transitional_and_failed_connection_status() {
    let mut app = fixture();
    app.connection.status = ConnectionStatus::Connecting;
    let linking = render(&app, 80, 24);
    assert!(
        linking.lines().next().unwrap().contains("LINKING"),
        "{linking}"
    );

    app.connection.status = ConnectionStatus::Failed;
    let failed = render(&app, 80, 24);
    assert!(
        failed.lines().next().unwrap().contains("FAILED"),
        "{failed}"
    );

    app.connection.status = ConnectionStatus::Connected;
    assert!(!render(&app, 80, 24).contains("ONLINE"));
}

#[test]
fn one_row_header_keeps_failure_status_after_long_context() {
    let mut app = fixture();
    app.profiles[0].name = "profile-name-that-is-much-wider-than-the-compact-header".into();
    app.connection.server.as_mut().unwrap().database =
        "database-name-that-would-otherwise-push-the-status-offscreen".into();
    app.connection.status = ConnectionStatus::Failed;

    let output = render(&app, 56, 24);
    let header = output.lines().next().unwrap();

    assert!(header.contains("LAZYDB"), "{output}");
    assert!(header.ends_with(" FAILED "), "{output}");
}

#[test]
fn workspace_tabs_publish_close_targets_for_each_tab() {
    let mut app = fixture();
    app.update(Action::NewConsole);
    let ids = app.tabs.iter().map(|tab| tab.id()).collect::<Vec<_>>();
    let (_, state) = render_with_state(&app, 120, 36);

    let visible_tab_ids = state
        .hit_regions
        .iter()
        .filter_map(|region| match region.target {
            HitTarget::Tab(index) => app.tabs.get(index).map(|tab| tab.id()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!visible_tab_ids.is_empty());
    assert!(visible_tab_ids.iter().all(|id| ids.contains(id)));
    assert!(visible_tab_ids.iter().all(|id| {
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::CloseTab(*id))
    }));
}

#[test]
fn default_console_tab_has_no_close_target() {
    let app = App::new(Vec::new());
    let id = app.active_console().id;
    let (_, state) = render_with_state(&app, 120, 36);

    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::CloseTab(id))
    );
}

#[test]
fn result_pagination_bar_renders_range_and_page_size_target() {
    let (output, state) = render_with_state(&fixture(), 120, 36);
    assert!(output.contains("0-0 of 0"), "{output}");
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::ResultPageSize)
    );
}

#[test]
fn data_grid_places_rows_immediately_below_the_header() {
    let output = render(&fixture(), 120, 36);
    let lines = output.lines().collect::<Vec<_>>();
    let header_y = lines
        .iter()
        .position(|line| line.contains("id") && line.contains("#"))
        .expect("grid header line");

    assert!(output.contains('│'), "{output}");
    assert!(
        lines
            .iter()
            .skip(header_y + 1)
            .any(|line| line.contains("Ada")),
        "{output}"
    );
}

#[test]
fn data_grid_renders_and_navigates_a_trailing_partial_column() {
    let mut app = fixture();
    app.focus = Focus::Results;
    let result = app
        .active_console_mut()
        .outcome
        .as_mut()
        .unwrap()
        .result_sets
        .last_mut()
        .unwrap();
    result.columns = vec![
        ColumnMeta {
            name: "first_column".into(),
            type_name: "TEXT".into(),
        },
        ColumnMeta {
            name: "second_column".into(),
            type_name: "TEXT".into(),
        },
        ColumnMeta {
            name: "TRAILING_COLUMN_CONTENT".into(),
            type_name: "TEXT".into(),
        },
    ];
    result.rows = vec![vec![
        CellValue::Text("first".into()),
        CellValue::Text("second".into()),
        CellValue::Text("TRAILING_VALUE_CONTENT".into()),
    ]];
    app.active_console_mut().grid.column_widths = vec![Some(20), Some(20), Some(40)];
    app.active_console_mut().grid.selected_column = 1;

    let (output, initial) = render_with_state(&app, 80, 24);
    let partial = initial
        .hit_regions
        .iter()
        .find(|region| matches!(region.target, HitTarget::ResultCell { row: 0, column: 2 }))
        .expect("trailing column fragment should be interactive");
    assert!(partial.area.width > 0 && partial.area.width < 40);
    assert!(output.contains("TRAILING_"), "{output}");
    assert!(!initial.hit_regions.iter().any(|region| {
        matches!(
            region.target,
            HitTarget::RelationColumnResize { column: 2, .. }
        )
    }));

    app.update(Action::GridMove {
        rows: 0,
        columns: 1,
    });
    let (_, revealed) = render_with_state(&app, 80, 24);
    assert_eq!(revealed.grid_viewport.unwrap().column_offset, 1);
    let selected = revealed
        .hit_regions
        .iter()
        .find(|region| matches!(region.target, HitTarget::ResultCell { row: 0, column: 2 }))
        .expect("selected column should remain interactive");
    assert_eq!(selected.area.width, 40);
}

#[test]
fn data_grid_publishes_and_clears_its_rendered_viewport() {
    let mut app = fixture();
    let (_, state) = render_with_state(&app, 120, 36);
    let viewport = state
        .grid_viewport
        .expect("SQL DATA should publish a viewport");
    assert_eq!(viewport.tab_id, app.active_console().id);
    assert_eq!(viewport.column_offset, 0);
    assert_eq!(viewport.row_offset, 0);
    assert!(viewport.visible_rows > 0);

    app.active_console_mut().result_view = lazydb::model::tab::ResultView::Output;
    let (_, state) = render_with_state(&app, 120, 36);
    assert_eq!(state.grid_viewport, None);
}

#[test]
fn data_grid_updates_visible_row_capacity_after_terminal_resize() {
    let app = fixture();
    let (_, compact) = render_with_state(&app, 120, 24);
    let (_, tall) = render_with_state(&app, 120, 40);

    let compact = compact.grid_viewport.unwrap();
    let tall = tall.grid_viewport.unwrap();
    assert_eq!(compact.tab_id, tall.tab_id);
    assert!(tall.visible_rows > compact.visible_rows);
}

#[test]
fn data_grid_renders_scrolled_rows_with_absolute_hit_targets() {
    let mut app = fixture();
    app.focus = Focus::Results;
    let result = app
        .active_console_mut()
        .outcome
        .as_mut()
        .unwrap()
        .result_sets
        .last_mut()
        .unwrap();
    result.rows = (0..30)
        .map(|row| {
            vec![
                CellValue::Integer(row),
                CellValue::Text(format!("row-{row}")),
                CellValue::Boolean(true),
            ]
        })
        .collect();
    app.active_console_mut().grid.selected_row = 15;
    app.active_console_mut().grid.row_offset = 15;

    let (output, state) = render_with_state(&app, 80, 20);
    let viewport = state.grid_viewport.unwrap();
    assert!(viewport.row_offset > 0);
    assert!(output.contains('#'), "{output}");
    assert!(
        output.contains(&format!("{}│", viewport.row_offset + 1)),
        "{output}"
    );
    let first_row = state
        .hit_regions
        .iter()
        .filter_map(|region| match region.target {
            HitTarget::ResultCell { row, .. } => Some(row),
            _ => None,
        })
        .min()
        .unwrap();
    assert_eq!(first_row, viewport.row_offset);
}

#[test]
fn selected_row_scrolls_before_the_horizontal_scrollbar_would_cover_it() {
    let mut app = fixture();
    app.focus = Focus::Results;
    let result = app
        .active_console_mut()
        .outcome
        .as_mut()
        .unwrap()
        .result_sets
        .last_mut()
        .unwrap();
    result.columns = (0..12)
        .map(|column| ColumnMeta {
            name: format!("column_{column}"),
            type_name: "INTEGER".into(),
        })
        .collect();
    result.rows = (0..30)
        .map(|row| (0..12).map(|_| CellValue::Integer(row)).collect())
        .collect();

    let (_, initial) = render_with_state(&app, 80, 20);
    let visible_rows = initial.grid_viewport.unwrap().visible_rows;
    assert!(visible_rows > 0);

    app.active_console_mut().grid.selected_row = visible_rows;
    let (_, state) = render_with_state(&app, 80, 20);
    let viewport = state.grid_viewport.unwrap();
    assert_eq!(viewport.row_offset, 1);
    let selected_y = state
        .hit_regions
        .iter()
        .find_map(|region| match region.target {
            HitTarget::ResultCell { row, .. } if row == visible_rows => Some(region.area.y),
            _ => None,
        })
        .unwrap();
    let scrollbar_y = state
        .hit_regions
        .iter()
        .find_map(|region| match region.target {
            HitTarget::GridScrollbarThumb { .. } => Some(region.area.y),
            _ => None,
        })
        .unwrap();
    assert!(selected_y < scrollbar_y);
}

#[test]
fn data_grid_renders_temporal_values_without_confusing_them_with_bytes() {
    use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};

    let mut app = fixture();
    let date = NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();
    let datetime = NaiveDateTime::new(date, NaiveTime::from_hms_opt(10, 20, 31).unwrap());
    let timestamp = DateTime::<FixedOffset>::from_naive_utc_and_offset(
        datetime,
        FixedOffset::east_opt(8 * 60 * 60).unwrap(),
    );
    app.active_console_mut()
        .outcome
        .as_mut()
        .unwrap()
        .result_sets
        .last_mut()
        .unwrap()
        .columns
        .extend([
            ColumnMeta {
                name: "date".into(),
                type_name: "DATE".into(),
            },
            ColumnMeta {
                name: "timestamp".into(),
                type_name: "TIMESTAMPTZ".into(),
            },
            ColumnMeta {
                name: "payload".into(),
                type_name: "BYTEA".into(),
            },
        ]);
    app.active_console_mut()
        .outcome
        .as_mut()
        .unwrap()
        .result_sets
        .last_mut()
        .unwrap()
        .rows[0]
        .extend([
            CellValue::Date(date),
            CellValue::Timestamp(timestamp),
            CellValue::Bytes(vec![0, 1, 2, 255]),
        ]);

    let output = render(&app, 180, 36);
    assert!(output.contains("2026-08-28"), "{output}");
    assert!(output.contains("2026-08-28 18:20:31+08:00"), "{output}");
    assert!(output.contains("0x000102FF"), "{output}");
}

#[test]
fn data_grid_keeps_null_muted_on_the_selected_row() {
    let mut app = fixture();
    let result = app
        .active_console_mut()
        .outcome
        .as_mut()
        .unwrap()
        .result_sets
        .last_mut()
        .unwrap();
    result.rows[0][1] = CellValue::Null;
    result.rows[0][2] = CellValue::Text("NULL".into());

    let (buffer, _) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let null = find_text_cell(&buffer, "<null>").expect("null preview");
    let null_text = find_text_cell(&buffer, "NULL").expect("NULL text value");

    assert_ne!(buffer[null].fg, buffer[null_text].fg);
}

#[test]
fn sql_data_renders_shared_query_bar_above_the_grid() {
    let app = fixture();
    let (output, state) = render_with_state(&app, 120, 36);

    assert!(output.contains("WHERE"), "{output}");
    assert!(output.contains("ORDER BY"), "{output}");
    assert!(!output.contains("Run a read-only query first"), "{output}");
    let lines = output.lines().collect::<Vec<_>>();
    let panel_y = lines
        .iter()
        .position(|line| line.contains("RESULT SET"))
        .unwrap();
    let query_y = lines
        .iter()
        .enumerate()
        .skip(panel_y + 1)
        .find_map(|(index, line)| line.contains("WHERE").then_some(index))
        .unwrap();
    let header_y = lines
        .iter()
        .enumerate()
        .skip(query_y + 1)
        .find_map(|(index, line)| (line.contains('#') && line.contains("id")).then_some(index))
        .unwrap();
    assert!(panel_y < query_y, "{output}");
    assert!(query_y < header_y, "{output}");
    let cell = state
        .hit_regions
        .iter()
        .find(|region| matches!(region.target, HitTarget::ResultCell { .. }))
        .unwrap();
    assert!(cell.area.y > 0);
}

#[test]
fn compact_query_bar_places_fields_side_by_side_when_the_inputs_remain_usable() {
    let mut app = fixture();
    app.focus = Focus::Results;
    let output = render(&app, 80, 36);
    let lines = output.lines().collect::<Vec<_>>();
    let where_y = lines
        .iter()
        .position(|line| line.contains("WHERE"))
        .unwrap_or_else(|| panic!("{output}"));
    let order_by_y = lines
        .iter()
        .position(|line| line.contains("ORDER BY"))
        .unwrap_or_else(|| panic!("{output}"));

    assert_eq!(order_by_y, where_y, "{output}");
    assert!(lines[where_y + 1].contains('─'), "{output}");
}

#[test]
fn sql_query_bar_is_inert_until_derived_execution_exists() {
    let mut app = fixture();
    app.focus = Focus::Results;
    let before = app.active_console().query.clone();

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::Where,
    ));
    app.update(Action::DataQueryInsert('x'));
    assert_eq!(app.active_console().query, before);
    assert!(app.update(Action::SubmitDataQuery).is_empty());
}

#[test]
fn sql_data_before_first_execution_has_a_quiet_disabled_query_bar() {
    let mut app = fixture();
    app.active_console_mut().outcome = None;
    let (output, state) = render_with_state(&app, 120, 36);

    assert!(output.contains("WHERE"), "{output}");
    assert!(output.contains("ORDER BY"), "{output}");
    assert!(
        output.contains("Run a query to populate the data viewport"),
        "{output}"
    );
    assert!(!output.contains("not implemented yet"), "{output}");
    assert!(!output.contains("Run a read-only query first"), "{output}");
    assert!(
        !state
            .hit_regions
            .iter()
            .any(|region| matches!(region.target, HitTarget::DataQueryInput(_)))
    );
}

#[test]
fn sql_result_query_completion_is_rendered_above_the_grid() {
    let mut app = fixture();
    app.active_console_mut().query.capability = lazydb::model::data_query::DataQueryCapability::Sql;
    app.active_console_mut().result_view = ResultView::Data;
    app.update(Action::FocusDataQueryInput(DataQueryInput::Where));
    for character in "act".chars() {
        app.update(Action::DataQueryInsert(character));
    }

    let (output, state) = render_with_state(&app, 120, 36);

    assert!(output.contains("active"), "{output}");
    // The fixture column is `BOOLEAN`; the popup shows the compact spelling.
    assert!(output.contains("active  bool"), "{output}");
    assert!(!output.contains("BOOLEAN"), "{output}");
    let popup = state.completion_popup.unwrap();
    let (buffer, _) = render_buffer_with_icons(&app, 120, 36, IconSet::default());
    assert_eq!(buffer[(popup.x, popup.y)].symbol(), "╭");
    assert_eq!(buffer[(popup.right() - 1, popup.y)].symbol(), "╮");
    assert_eq!(buffer[(popup.x, popup.bottom() - 1)].symbol(), "╰");
    assert_eq!(
        buffer[(popup.right() - 1, popup.bottom() - 1)].symbol(),
        "╯"
    );
    assert_eq!(buffer[(popup.x, popup.y)].fg, Color::Rgb(43, 66, 86));
    assert!(popup.right() <= 120);
    assert!(popup.bottom() <= 36);
}

#[test]
fn target_selector_renders_real_target_and_navigation_hint() {
    let mut app = fixture();
    app.update(Action::OpenTargetSelector);
    let output = render(&app, 120, 36);

    assert!(output.contains("EXECUTION TARGET"));
    assert!(output.contains(":memory:.main"));
    assert!(output.contains("current"));
    assert!(output.contains("Enter confirm"));
    assert!(output.contains("Cancel"));
    assert!(!output.contains("Target selector is available"));
}

#[test]
fn target_selector_renders_visible_rows_and_mouse_regions() {
    let mut app = fixture();
    let profile_id = app.active_profile().unwrap().id;
    let candidates = (0..24)
        .map(|index| lazydb::model::execution_target::ExecutionTarget {
            profile_id,
            database: format!("db-{index}\nunsafe"),
            schema: Some(format!("schema-{index}")),
        })
        .collect();
    app.overlay = Some(Overlay::TargetSelector {
        candidates,
        selected: 23,
    });

    let (output, state) = render_with_state(&app, 80, 24);

    assert!(output.contains("db-23unsafe.schema-23"), "{output}");
    assert!(!output.contains("db-0unsafe.schema-0"), "{output}");
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::TargetSelectorRow(23))
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| region.target == HitTarget::TargetSelectorCancel)
    );
    assert!(
        state
            .hit_regions
            .iter()
            .all(|region| region.area.bottom() <= 24)
    );
}

#[test]
fn compact_layout_uses_the_focused_panel() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    let output = render(&app, 80, 24);

    assert!(output.contains("LAZYDB"));
    assert!(output.contains("SQL EDITOR"));
    assert!(output.contains("WHERE"));
    assert!(!output.contains("EXPLORER"));
}

#[test]
fn wide_layout_remains_readable() {
    let mut app = fixture();
    app.update(Action::ToggleResultView);
    let output = render(&app, 180, 50);

    assert!(output.contains("EXPLORER"));
    assert!(output.contains("SQL EDITOR"));
    assert!(output.contains("1 row(s) retrieved in 376 ms"));
}

#[test]
fn help_overlay_is_contextual() {
    let mut app = fixture();
    app.focus = Focus::Explorer;
    app.update(Action::ShowHelp);
    let output = render(&app, 120, 36);

    assert!(output.contains("KEYMAP // EXPLORER"));
    assert!(output.contains("select first node"));
    assert!(output.contains("select last node"));
    assert!(output.contains("Esc"));
}

#[test]
fn tiny_terminal_gets_an_actionable_message() {
    let output = render(&fixture(), 40, 10);

    assert!(output.contains("TERMINAL TOO SMALL"));
    assert!(output.contains("Resize"));
}

#[test]
fn explorer_roots_show_connection_metadata_and_semantic_hit_regions() {
    let mut primary = import_connection_url(
        "postgres://alice@db.example.com:5432/app",
        Some("production"),
    )
    .unwrap()
    .profile;
    primary.environment = Environment::Production;
    primary.read_only = true;
    let replica =
        import_connection_url("mysql://report@mysql.example.com/metrics", Some("reports"))
            .unwrap()
            .profile;
    let primary_id = primary.id;
    let mut app = App::new(vec![primary, replica]);
    app.focus = Focus::Explorer;
    app.connection.profile_id = Some(primary_id);
    app.connection.status = ConnectionStatus::Connected;
    app.explorer
        .normalized
        .profiles
        .get_mut(&primary_id)
        .unwrap()
        .status = lazydb::model::explorer::ExplorerConnectionStatus::Online;
    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("production"));
    assert!(!output.contains("SAVED"));
    assert!(output.contains("●"));
    assert!(output.contains("production"));
    assert!(output.contains("reports"));
    assert!(state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ExplorerRow(ExplorerNodeId::Profile(_))
    )));
}

#[test]
fn server_profile_form_shows_all_fields_and_never_reveals_passwords() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let manager = app.profile_manager.as_mut().unwrap();
    let draft = manager.draft.as_mut().unwrap();
    draft.name.set("primary");
    draft.user.set("operator");
    draft.database.set("warehouse");
    draft.set_password("super-secret");

    let (output, state) = render_with_state(&app, 120, 36);
    for label in [
        "Driver",
        "URL",
        "Name",
        "Host",
        "Port",
        "User",
        "Password",
        "Database",
        "Default schema",
        "Visible objects",
        "SSL mode",
        "Environment",
        "Read only",
        "Password storage",
        "Test",
        "Save",
        "Save & Connect",
        "Cancel",
    ] {
        assert!(output.contains(label), "missing {label}");
    }
    assert!(!output.contains("URL FORMAT"));
    assert!(output.contains("postgresql://"));
    assert!(!output.contains("postgres://user:password@host:5432/database"));
    assert!(!output.contains("super-secret"));
    assert!(output.contains("••••••••••••"));
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileField(ProfileField::Password) })
    );
    assert!(state.hit_regions.iter().any(|region| {
        region.target == HitTarget::ProfileButton(ProfileButton::SaveAndConnect)
    }));
    assert!(
        !state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileField(ProfileField::UrlFormat) })
    );
}

#[test]
fn profile_url_help_follows_the_selected_driver_when_focused() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let cases = [
        (
            0,
            "postgres://user:password@host:5432/database",
            "mysql://user:password@host:3306/database",
        ),
        (
            1,
            "mysql://user:password@host:3306/database",
            "sqlite:///path/to/database.db",
        ),
        (
            2,
            "sqlserver://user:password@host:1433/database",
            "sqlite:///path/to/database.db",
        ),
        (
            3,
            "sqlite:///path/to/database.db",
            "postgres://user:password@host:5432/database",
        ),
    ];
    for (cycle, expected, absent) in cases {
        if cycle != 0 {
            app.update(Action::ProfileFocusField(ProfileField::Kind));
            app.update(Action::ProfileCycle(1));
        }
        let output = render(&app, 120, 36);
        assert!(
            !output.contains(expected),
            "unexpected example {expected}: {output}"
        );
        assert!(
            !output.contains(absent),
            "unexpected example {absent}: {output}"
        );
        app.update(Action::ProfileFocusField(ProfileField::Url));
        let focused = render(&app, 120, 36);
        let help = match cycle {
            0 => "Accepts postgres://",
            1 => "Accepts mysql://",
            2 => "Accepts sqlserver://",
            _ => "Accepts sqlite://",
        };
        assert!(focused.contains(help), "missing URL help: {focused}");
    }
}

#[test]
fn profile_url_preview_is_display_width_safe_when_unfocused() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let draft = app
        .profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap();
    draft.name.set("connection-with-a-long-name");
    draft.host.set("db.example.internal.with-a-long-hostname");
    draft.database.set("warehouse_with_a_long_database_name");

    let output = render(&app, 80, 24);
    assert!(output.contains("URL"));
    assert!(output.contains('…') || output.contains("postgresql://"));
    assert!(!output.contains("EXAMPLES"));
}

#[test]
fn visible_objects_scope_shows_discovery_loading_and_refresh_hint() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    {
        let draft = app
            .profile_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap();
        draft.name.set("primary");
        draft.database.set("warehouse");
    }
    assert!(matches!(
        app.update(Action::ProfileOpenScope).as_slice(),
        [lazydb::action::Command::DiscoverProfileCatalog { .. }]
    ));

    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("Loading visible objects"));
    assert!(output.contains("discovering databases and schemas"));
    assert!(output.contains("Loading..."));
    assert!(!output.contains("r refresh"));
    assert!(output.contains("warehouse"));
    assert!(
        !state
            .hit_regions
            .iter()
            .any(|region| matches!(region.target, HitTarget::ProfileScopeRow(_)))
    );
}

#[test]
fn visible_objects_scope_renders_partial_database_without_all_schemas_row() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    {
        let draft = app
            .profile_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap();
        draft.name.set("primary");
        draft.database.set("warehouse");
    }
    let (request_id, fingerprint) = match app.update(Action::ProfileOpenScope).as_slice() {
        [
            lazydb::action::Command::DiscoverProfileCatalog {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ProfileCatalogDiscoverySucceeded {
        request_id,
        fingerprint,
        server: ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16".into(),
            database: "warehouse".into(),
            current_user: None,
        },
        capabilities: lazydb::db::catalog::CatalogCapabilities {
            namespace_model: lazydb::db::catalog::NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![],
            column_metadata: Default::default(),
            supports_lazy_children: false,
        },
        discovery: lazydb::db::catalog::CatalogDiscovery {
            databases: vec![lazydb::db::catalog::DiscoveredDatabase {
                name: "warehouse".into(),
                schemas: vec!["analytics".into(), "public".into()],
            }],
            warnings: Vec::new(),
        },
    });

    let output = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii)).0;
    assert!(output.contains("[-] warehouse"), "{output}");
    assert!(output.contains("[x] public"), "{output}");
    assert!(output.contains("[ ] analytics"), "{output}");
    assert!(!output.contains("All schemas"), "{output}");
}

#[test]
fn pending_url_redacts_an_embedded_password_before_commit() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::Url));
    let draft = app
        .profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap();
    draft.move_home(ProfileField::Url);
    draft.paste(
        ProfileField::Url,
        "postgresql://alice:never-render-this@db.example/app?sslmode=require",
    );

    let output = render(&app, 120, 36);
    assert!(!output.contains("never-render-this"));
    assert!(output.contains("[REDACTED]"));
}

#[test]
fn stored_password_is_described_without_rendering_a_secret() {
    let mut profile =
        import_connection_url("postgres://alice@db.example.com/app", Some("remembered"))
            .unwrap()
            .profile;
    profile.credential_policy = lazydb::profile::CredentialPolicy::Keyring(keyring_ref(profile.id));
    let mut app = App::new(vec![profile]);
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileStartEdit {
        profile_id: app.profiles[0].id,
    });

    let output = render(&app, 120, 36);
    assert!(output.contains("Stored in system credential store"));
    assert!(!output.contains("keyring:dev.lazydb"));
}

#[test]
fn server_and_sqlite_forms_only_show_relevant_fields() {
    let mut mysql = App::new(Vec::new());
    mysql.update(Action::OpenProfileManager);
    mysql.update(Action::ProfileCycle(1));
    let mysql_output = render(&mysql, 120, 36);
    assert!(mysql_output.contains("MySQL"));
    assert!(mysql_output.contains("Host"));
    assert!(!mysql_output.contains("Default schema"));
    assert!(!mysql_output.contains("Memory database"));

    let mut sql_server = App::new(Vec::new());
    sql_server.update(Action::OpenProfileManager);
    sql_server.update(Action::ProfileCycle(2));
    let sql_server_output = render(&sql_server, 120, 36);
    assert!(sql_server_output.contains("SQL Server"));
    assert!(sql_server_output.contains("Default schema"));
    assert!(sql_server_output.contains("dbo"));
    assert!(sql_server_output.contains("sqlserver://"));

    let mut sqlite_file = App::new(Vec::new());
    sqlite_file.update(Action::OpenProfileManager);
    sqlite_file.update(Action::ProfileCycle(3));
    let sqlite_file_output = render(&sqlite_file, 120, 36);
    assert!(sqlite_file_output.contains("SQLite"));
    assert!(sqlite_file_output.contains("Path"));
    assert!(sqlite_file_output.contains("Memory database"));
    assert!(!sqlite_file_output.contains("Host"));
    assert!(!sqlite_file_output.contains("Password"));

    sqlite_file.update(Action::ProfileFocusField(ProfileField::SqliteMemory));
    sqlite_file.update(Action::ProfileToggle);
    let sqlite_memory_output = render(&sqlite_file, 120, 36);
    assert!(sqlite_memory_output.contains("Memory database"));
    assert!(!sqlite_memory_output.contains("Path"));
}

#[test]
fn profile_form_remains_actionable_in_compact_layout() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let output = render(&app, 80, 24);

    assert!(output.contains("NEW CONNECTION"));
    assert!(output.contains("PostgreSQL"), "{output}");
    assert!(output.contains("MySQL"), "{output}");
    assert!(output.contains("SQL Server"), "{output}");
    assert!(output.contains("SQLite"), "{output}");
    assert!(output.contains("Host"));
    assert!(output.contains("Password"));
    assert!(output.contains("URL") || output.contains("CONNECTION URL"));
    assert!(output.contains("Save & Connect"));
    assert!(output.contains("Esc cancel") || output.contains("Esc Close"));
}

#[test]
fn driver_options_have_individual_targets_and_selected_style_survives_field_blur() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::Name));

    let (buffer, state) = render_buffer_with_icons(&app, 80, 24, IconSet::new(IconMode::Ascii));

    let options = [
        DatabaseKind::Postgres,
        DatabaseKind::MySql,
        DatabaseKind::SqlServer,
        DatabaseKind::Sqlite,
    ]
    .map(|kind| {
        state
            .hit_regions
            .iter()
            .find(|region| region.target == HitTarget::ProfileDriver(kind))
            .unwrap()
    });
    assert!(
        options
            .windows(2)
            .all(|pair| pair[0].area.right() < pair[1].area.x)
    );
    let selected = options[0].area;
    let unselected = options[1].area;
    let theme = ui::theme::Theme::default();
    assert_eq!(buffer[(selected.x, selected.y)].bg, theme.accent);
    assert_ne!(buffer[(unselected.x, unselected.y)].bg, theme.accent);
}

#[test]
fn driver_options_use_database_icons_in_each_icon_mode() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let kinds = [
        DatabaseKind::Postgres,
        DatabaseKind::MySql,
        DatabaseKind::SqlServer,
        DatabaseKind::Sqlite,
    ];

    for mode in [IconMode::NerdFont, IconMode::Unicode, IconMode::Ascii] {
        let (output, _) = render_with_icons(&app, 120, 36, IconSet::new(mode));
        for kind in kinds {
            let display_name = match kind {
                DatabaseKind::Postgres => "PostgreSQL",
                DatabaseKind::MySql => "MySQL",
                DatabaseKind::SqlServer => "SQL Server",
                DatabaseKind::Sqlite => "SQLite",
            };
            let label = format!("{} {display_name}", IconSet::new(mode).database(kind));
            assert!(
                output.contains(&label),
                "missing {label:?} in {mode:?}: {output}"
            );
        }
    }
}

#[test]
fn profile_manager_renders_confirmation_busy_errors_and_warnings() {
    let profile = import_connection_url(":memory:", Some("throwaway"))
        .unwrap()
        .profile;
    let mut deleting = App::new(vec![profile]);
    deleting.update(Action::OpenProfileManager);
    deleting.update(Action::ProfileRequestDelete {
        profile_id: deleting.profiles[0].id,
    });
    let confirmation = render(&deleting, 100, 30);
    assert!(confirmation.contains("DELETE CONNECTION"));
    assert!(confirmation.contains("throwaway"));
    assert!(confirmation.contains("DELETE PERMANENTLY"));

    let mut busy = App::new(Vec::new());
    busy.update(Action::OpenProfileManager);
    {
        let manager = busy.profile_manager.as_mut().unwrap();
        let draft = manager.draft.as_mut().unwrap();
        draft.name.set("busy");
        draft.database.set("app");
    }
    assert!(!busy.update(Action::ProfileTest).is_empty());
    assert_eq!(
        busy.profile_manager.as_ref().unwrap().operation,
        Some(ProfileOperation::Testing)
    );
    let (busy_output, busy_state) = render_with_state(&busy, 100, 30);
    assert!(busy_output.contains("TESTING CONNECTION"));
    assert!(busy_output.contains("BUSY"));
    assert!(!busy_state.hit_regions.iter().any(|region| {
        matches!(
            region.target,
            HitTarget::ProfileField(_)
                | HitTarget::ProfileDriver(_)
                | HitTarget::ProfileToggle(_)
                | HitTarget::ProfileButton(_)
        )
    }));

    let mut connecting = App::new(vec![
        import_connection_url(":memory:", Some("connecting"))
            .unwrap()
            .profile,
    ]);
    connecting.update(Action::OpenProfileManager);
    connecting.profile_manager.as_mut().unwrap().operation = Some(ProfileOperation::Connecting);
    let (_, connecting_state) = render_with_state(&connecting, 100, 30);
    assert!(!connecting_state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ProfileField(_) | HitTarget::ProfileButton(_)
    )));

    let mut invalid = App::new(Vec::new());
    invalid.update(Action::OpenProfileManager);
    invalid.update(Action::ProfileSave { connect: false });
    let invalid_output = render(&invalid, 100, 30);
    assert!(invalid_output.contains("profile name is required"));

    invalid.profile_manager.as_mut().unwrap().message =
        Some("Native password store is unavailable; the password is session-only".into());
    let warning_output = render(&invalid, 100, 30);
    assert!(warning_output.contains("Native password store is unavailable"));
    assert!(warning_output.contains("session-only"));
}

#[test]
fn tiny_terminal_wins_over_profile_overlay() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::Form
    );
    let output = render(&app, 40, 10);
    assert!(output.contains("TERMINAL TOO SMALL"));
    assert!(!output.contains("NEW CONNECTION"));
}

#[test]
fn disconnected_explorer_points_to_the_profile_manager() {
    let output = render(&App::new(Vec::new()), 120, 36);
    assert!(output.contains("No profiles"));
    assert!(output.contains("NEW"));
}

#[test]
fn disconnected_workspace_without_profiles_renders_first_run_empty_state() {
    let output = render(&App::new(Vec::new()), 120, 36);

    assert!(output.contains("NO CONNECTIONS YET"), "{output}");
    assert!(output.contains("Select NEW in Explorer"), "{output}");
    assert!(output.contains("Enter"), "{output}");
    assert!(!output.contains("no result"), "{output}");
    assert!(!output.contains("DATA"), "{output}");
    assert!(!output.contains("OUTPUT"), "{output}");
}

#[test]
fn disconnected_workspace_with_profiles_prompts_for_connection() {
    let profile = import_connection_url(":memory:", Some("local"))
        .unwrap()
        .profile;
    let output = render(&App::new(vec![profile]), 120, 36);

    assert!(output.contains("NO ACTIVE CONNECTION"), "{output}");
    assert!(
        output.contains("Select a connection in Explorer"),
        "{output}"
    );
    assert!(output.contains("Enter"), "{output}");
    assert!(!output.contains("NO CONNECTIONS YET"), "{output}");
    assert!(!output.contains("no result"), "{output}");
}

#[test]
fn disconnected_workspace_keeps_actionable_copy_at_compact_sizes() {
    let no_profiles = App::new(Vec::new());
    let no_profiles_output = render(&no_profiles, 80, 24);
    assert!(
        no_profiles_output.contains("NO CONNECTIONS YET"),
        "{no_profiles_output}"
    );
    assert!(
        no_profiles_output.contains("Select NEW in Explorer"),
        "{no_profiles_output}"
    );
    assert!(
        !no_profiles_output.contains("no result"),
        "{no_profiles_output}"
    );

    let profile = import_connection_url(":memory:", Some("local"))
        .unwrap()
        .profile;
    let with_profile_output = render(&App::new(vec![profile]), 80, 24);
    assert!(
        with_profile_output.contains("NO ACTIVE CONNECTION"),
        "{with_profile_output}"
    );
    assert!(
        with_profile_output.contains("Select a connection"),
        "{with_profile_output}"
    );
    assert!(
        !with_profile_output.contains("no result"),
        "{with_profile_output}"
    );
}

#[test]
fn tiny_disconnected_terminal_keeps_the_global_size_fallback() {
    let output = render(&App::new(Vec::new()), 40, 10);

    assert!(output.contains("TERMINAL TOO SMALL"), "{output}");
    assert!(!output.contains("NO CONNECTIONS YET"), "{output}");
    assert!(!output.contains("NO ACTIVE CONNECTION"), "{output}");
}

#[test]
fn explorer_root_projection_keeps_ordered_roots_visible() {
    let profiles = (0..7)
        .map(|index| {
            import_connection_url(":memory:", Some(&format!("profile-{index:02}")))
                .unwrap()
                .profile
        })
        .collect();
    let mut app = App::new(profiles);
    app.focus = Focus::Explorer;
    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("profile-00"));
    assert!(state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ExplorerRow(ExplorerNodeId::Profile(_))
    )));
}

#[test]
fn minimum_supported_form_scrolls_to_the_selected_field() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::PasswordStorage));

    let (output, state) = render_with_state(&app, 56, 16);
    assert!(output.contains("Password storage"));
    assert!(output.contains("Esc cancel") || output.contains("Esc Close"));
    assert!(
        state.hit_regions.iter().any(|region| {
            region.target == HitTarget::ProfileField(ProfileField::PasswordStorage)
        })
    );
}

#[test]
fn hostile_and_long_form_values_render_safely_at_the_cursor() {
    let mut hostile = App::new(Vec::new());
    hostile.update(Action::OpenProfileManager);
    hostile.update(Action::ProfileFocusField(ProfileField::Name));
    hostile.update(Action::ProfilePaste("\n\u{1b}".into()));
    let hostile_output = render(&hostile, 80, 24);
    assert!(hostile_output.contains("<LF><ESC>"));

    let mut long = App::new(Vec::new());
    long.update(Action::OpenProfileManager);
    long.update(Action::ProfileFocusField(ProfileField::Name));
    long.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .name
        .set(format!("{}VISIBLE-END", "prefix-".repeat(20)));
    let long_output = render(&long, 80, 24);
    assert!(long_output.contains("VISIBLE-END"));
}

#[test]
fn profile_modal_hides_the_workspace_cursor_unless_editing_text() {
    let mut list = fixture();
    list.update(lazydb::action::Action::EditorKey(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ),
    ));
    list.update(Action::OpenProfileManager);
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state(frame, &list, &mut state))
        .unwrap();
    assert!(!terminal.backend().cursor_visible());

    list.update(Action::ProfileStartEdit {
        profile_id: list.profiles[0].id,
    });
    list.update(Action::ProfileFocusField(ProfileField::Name));
    terminal
        .draw(|frame| ui::render_with_state(frame, &list, &mut state))
        .unwrap();
    assert!(terminal.backend().cursor_visible());
}

#[test]
fn empty_profile_list_only_exposes_actionable_buttons() {
    let app = App::new(Vec::new());
    let (_, state) = render_with_state(&app, 120, 36);
    assert!(state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ExplorerRow(ExplorerNodeId::EmptyProfiles)
    )));
}

#[test]
fn editor_snapshot_projects_hostile_controls_to_inert_display_text() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor(
        "safe\u{1b}]52;c;secret\u{7}\u{1b}[2J\u{00}\tend".into(),
    ));
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 80,
            height: 4,
        })
        .unwrap();
    let display = snapshot.lines[0].spans[0].text.as_str();
    assert!(!display.contains('\u{1b}'));
    assert!(display.contains("<ESC>"));
    assert!(display.contains("<0x07>"));
    assert!(display.contains("<0x00>"));
    assert!(display.ends_with("end"));
    let output = render(&app, 80, 24);
    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\u{7}'));
    assert!(output.contains("NO CONNECTIONS YET"));
}

#[test]
fn editor_snapshot_maps_cjk_emoji_and_tabs_to_display_cells() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("数据🙂\tX".into()));
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 40,
            height: 4,
        })
        .unwrap();
    assert_eq!(
        snapshot.lines[0].source_to_display_cells,
        vec![0, 2, 4, 6, 8, 9]
    );
    assert_eq!(snapshot.cursor_screen_cell, Some((0, 0)));

    for _ in 0..3 {
        app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('l'),
            crossterm::event::KeyModifiers::NONE,
        )));
    }
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 40,
            height: 4,
        })
        .unwrap();
    assert_eq!(snapshot.cursor_screen_cell, Some((6, 0)));
}

#[test]
fn editor_snapshot_scrolls_without_projecting_offscreen_lines() {
    let mut app = App::new(Vec::new());
    let text = (0..10_000)
        .map(|line| format!("line-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.update(Action::ReplaceEditor(text));
    app.update(Action::EditorScroll {
        rows: 5_000,
        columns: 0,
    });
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 40,
            height: 3,
        })
        .unwrap();
    assert_eq!(snapshot.first_line, 5_000);
    assert_eq!(snapshot.lines.len(), 5);
    assert_eq!(snapshot.lines[0].line, 5_000);
    assert!(!snapshot.lines[0].spans[0].text.contains("line-4999"));
}
