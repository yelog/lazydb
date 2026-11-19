use lazydb::{
    action::{Action, Command},
    app::App,
    db::catalog::{
        CatalogCompleteness, CatalogCount, CatalogEntry, CatalogId, CatalogKind, CatalogPage,
        CatalogTarget, ObjectGroup, OptionalMetadata, QualifiedName,
    },
    model::{
        execution_target::ExecutionTarget,
        explorer::{CatalogGroupState, ExplorerConnectionStatus, ExplorerNodeId},
        relation::{RelationDescriptor, RelationKey, RelationTab, RelationView},
        tab::{ConsoleTab, WorkspaceTab},
        workspace::{ConnectionStatus, Focus},
    },
    profile::{CatalogSelection, DatabaseKind, import_connection_url},
};
use uuid::Uuid;

fn app_with_tables() -> (App, CatalogId, Vec<CatalogId>) {
    let mut profile = import_connection_url("postgres://user@localhost/app", Some("local"))
        .unwrap()
        .profile;
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let database_id = CatalogId::new(profile_id, CatalogKind::Database, ["app"]);
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let database = CatalogEntry::database(
        database_id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: None,
            object: "app".into(),
        },
        "database",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        schema_id.clone(),
        database_id,
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "public".into(),
        },
        "schema",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let state = app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap();
    state.status = ExplorerConnectionStatus::Online;
    state
        .catalog
        .insert_subtree(vec![database, schema])
        .unwrap();
    state
        .catalog
        .set_group_state(
            &schema_id,
            ObjectGroup::Tables,
            CatalogGroupState {
                count: CatalogCount::Exact(80),
                completeness: CatalogCompleteness::Complete,
            },
        )
        .unwrap();
    let mut tables = Vec::new();
    for index in 0..80 {
        let object = format!("table_{index:02}");
        let table_id = CatalogId::new(profile_id, CatalogKind::Table, [index.to_string()]);
        let name = QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: object.clone(),
        };
        state
            .catalog
            .insert(
                CatalogEntry::relation(
                    table_id.clone(),
                    schema_id.clone(),
                    name.clone(),
                    "table",
                    OptionalMetadata::Unsupported,
                    true,
                )
                .unwrap(),
            )
            .unwrap();
        if index == 0 || index == 79 {
            app.tabs
                .push(WorkspaceTab::Relation(RelationTab::with_descriptor(
                    RelationDescriptor {
                        key: RelationKey {
                            profile_id,
                            object_id: table_id.clone(),
                        },
                        qualified_name: name,
                        kind: CatalogKind::Table,
                        title: object,
                    },
                    RelationView::Data,
                )));
        }
        tables.push(table_id);
    }
    app.focus = Focus::Results;
    app.explorer.normalized.expanded.clear();
    app.explorer.set_viewport_height(8);
    app.explorer.select_id(ExplorerNodeId::Profile(profile_id));
    (app, schema_id, tables)
}

fn assert_revealed(app: &App, node: ExplorerNodeId) {
    assert_eq!(app.explorer.selected_id(), Some(&node));
    let viewport = app.explorer.viewport(8);
    assert!(
        viewport
            .rows
            .iter()
            .chain(&viewport.pinned)
            .any(|row| row.id == node),
        "expected {node:?} in the viewport"
    );
    assert_eq!(app.explorer.scroll, app.explorer.normalized.scroll);
}

#[test]
fn tab_activation_expands_ancestors_and_scrolls_to_the_object() {
    for action in [Action::NextTab, Action::PreviousTab, Action::ActivateTab(1)] {
        let (mut app, schema_id, tables) = app_with_tables();

        app.update(action);

        assert_revealed(&app, ExplorerNodeId::Catalog(tables[79].clone()));
        assert!(
            app.explorer
                .normalized
                .expanded
                .contains(&ExplorerNodeId::Group {
                    parent: schema_id,
                    group: ObjectGroup::Tables,
                })
        );
        assert!(
            !app.explorer
                .normalized
                .expanded
                .contains(&ExplorerNodeId::Catalog(tables[79].clone()))
        );
        assert_eq!(app.focus, Focus::Results);

        app.update(Action::PreviousTab);

        assert_revealed(&app, ExplorerNodeId::Catalog(tables[0].clone()));
    }
}

#[test]
fn closing_the_active_tab_reveals_the_remaining_tab() {
    let (mut app, _, tables) = app_with_tables();
    app.update(Action::ActivateTab(1));
    let active_id = app.tabs[1].id();

    app.update(Action::CloseTab(active_id));

    assert_revealed(&app, ExplorerNodeId::Catalog(tables[0].clone()));
}

#[test]
fn disabled_auto_reveal_preserves_the_explorer_state() {
    let (mut app, _, _) = app_with_tables();
    app.set_explorer_auto_reveal_active_tab(false);
    let selection = app.explorer.selected_id().cloned();
    let expanded = app.explorer.normalized.expanded.clone();
    let scroll = app.explorer.scroll;

    app.update(Action::NextTab);

    assert_eq!(app.explorer.selected_id(), selection.as_ref());
    assert_eq!(app.explorer.normalized.expanded, expanded);
    assert_eq!(app.explorer.scroll, scroll);
}

#[test]
fn console_activation_reveals_its_schema_without_connecting() {
    let (mut app, schema_id, _) = app_with_tables();
    let profile_id = schema_id.profile_id();
    let mut console = ConsoleTab::new("console");
    console.execution_target = Some(ExecutionTarget {
        profile_id,
        database: "app".into(),
        schema: Some("public".into()),
    });
    app.tabs.push(WorkspaceTab::Sql(console));

    let commands = app.update(Action::ActivateTab(2));

    assert_revealed(&app, ExplorerNodeId::Catalog(schema_id));
    assert_eq!(app.focus, Focus::Editor);
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::Connect { .. }))
    );
}

fn add_namespace(app: &mut App, profile_id: Uuid, database: &str, schema: &str) -> CatalogId {
    let database_id = CatalogId::new(profile_id, CatalogKind::Database, [database]);
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [database, schema]);
    let database_entry = CatalogEntry::database(
        database_id.clone(),
        QualifiedName {
            database: Some(database.into()),
            schema: None,
            object: database.into(),
        },
        "database",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let schema_entry = CatalogEntry::schema(
        schema_id.clone(),
        database_id,
        QualifiedName {
            database: Some(database.into()),
            schema: Some(schema.into()),
            object: schema.into(),
        },
        "schema",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog
        .insert_subtree(vec![database_entry, schema_entry])
        .unwrap();
    schema_id
}

#[test]
fn same_named_console_namespaces_are_scoped_to_the_tabs_connection() {
    let (mut app, original_schema, _) = app_with_tables();
    let other = import_connection_url("postgres://user@localhost/app", Some("other"))
        .unwrap()
        .profile;
    let profile_id = other.id;
    app.profiles.push(other);
    app.explorer.normalized.add_profile(profile_id);
    let schema_id = add_namespace(&mut app, profile_id, "app", "public");
    let mut console = ConsoleTab::new("other");
    console.execution_target = Some(ExecutionTarget {
        profile_id,
        database: "app".into(),
        schema: Some("public".into()),
    });
    app.tabs.push(WorkspaceTab::Sql(console));

    app.update(Action::ActivateTab(2));

    assert_revealed(&app, ExplorerNodeId::Catalog(schema_id));
    assert!(
        !app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(original_schema.profile_id()))
    );
}

#[test]
fn bigquery_console_reveals_its_data_project_instead_of_its_billing_project() {
    let profile = import_connection_url(
        "bigquery://billing-project/dataset?data_project=data-project",
        Some("warehouse"),
    )
    .unwrap()
    .profile;
    let target = ExecutionTarget::from_profile(&profile);
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let schema_id = add_namespace(&mut app, profile_id, "data-project", "dataset");
    let mut console = ConsoleTab::new("warehouse");
    console.execution_target = Some(target);
    app.tabs.push(WorkspaceTab::Sql(console));
    app.explorer.set_viewport_height(8);

    app.update(Action::ActivateTab(0));

    assert_revealed(&app, ExplorerNodeId::Catalog(schema_id));
}

#[test]
fn an_unbound_console_keeps_the_explorer_selection() {
    let (mut app, _, tables) = app_with_tables();
    app.update(Action::ActivateTab(1));
    app.tabs.push(WorkspaceTab::Sql(ConsoleTab::new("unbound")));

    app.update(Action::ActivateTab(2));

    assert_revealed(&app, ExplorerNodeId::Catalog(tables[79].clone()));
}

#[test]
fn manual_explorer_navigation_is_preserved_until_the_next_tab_activation() {
    let (mut app, _, tables) = app_with_tables();
    app.update(Action::NextTab);
    let profile = ExplorerNodeId::Profile(tables[0].profile_id());

    app.update(Action::ExplorerSelect(profile.clone()));
    app.update(Action::ExplorerCollapse);
    app.update(Action::ExplorerViewportChanged(8));

    assert_eq!(app.explorer.selected_id(), Some(&profile));
    assert!(!app.explorer.normalized.expanded.contains(&profile));

    app.update(Action::ActivateTab(1));

    assert_revealed(&app, ExplorerNodeId::Catalog(tables[79].clone()));
}

#[test]
fn reordering_the_active_tab_preserves_manual_explorer_navigation() {
    let (mut app, _, tables) = app_with_tables();
    app.update(Action::ActivateTab(1));
    let active_id = app.tabs[1].id();
    let profile = ExplorerNodeId::Profile(tables[0].profile_id());
    app.update(Action::ExplorerSelect(profile.clone()));
    app.update(Action::ExplorerCollapse);

    app.update(Action::MoveTabLeft);

    assert_eq!(app.tabs[app.active_tab].id(), active_id);
    assert_eq!(app.explorer.selected_id(), Some(&profile));
    assert!(!app.explorer.normalized.expanded.contains(&profile));
}

#[test]
fn activating_a_tab_exits_explorer_search_and_find_to_show_the_object() {
    for search_action in [Action::ExplorerSearchOpen, Action::ExplorerFindOpen] {
        let (mut app, _, tables) = app_with_tables();
        app.focus = Focus::Explorer;
        app.update(search_action);
        assert!(app.explorer.search.is_some() || app.explorer.find.is_some());

        app.update(Action::ActivateTab(1));

        assert!(app.explorer.search.is_none());
        assert!(app.explorer.find.is_none());
        assert_revealed(&app, ExplorerNodeId::Catalog(tables[79].clone()));
    }
}

fn pending_schema_reveal() -> (App, CatalogId, CatalogPage) {
    let (mut app, schema_id, _) = app_with_tables();
    let profile_id = schema_id.profile_id();
    let target = ExecutionTarget {
        profile_id,
        database: "app".into(),
        schema: Some("analytics".into()),
    };
    let connection = app.sessions.start_attempt(target.clone()).unwrap();
    assert!(app.sessions.accept_success(
        connection,
        lazydb::db::ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "17".into(),
            database: "app".into(),
            current_user: None,
        },
        Default::default(),
    ));
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = connection.generation;
    app.connection.target = Some(target.clone());
    app.connection.status = ConnectionStatus::Connected;
    app.explorer.catalog_sessions.insert(profile_id, connection);
    let mut console = ConsoleTab::new("analytics");
    console.execution_target = Some(target);
    app.tabs.push(WorkspaceTab::Sql(console));

    app.update(Action::ActivateTab(2));

    let catalog = &app.explorer.normalized.profiles[&profile_id].catalog;
    let database_id = catalog.parent(&schema_id).unwrap().clone();
    assert_revealed(&app, ExplorerNodeId::Catalog(database_id.clone()));
    let schemas = CatalogTarget::schemas(database_id.clone()).unwrap();
    let commands = app.update(Action::ExplorerLoadTarget(schemas));
    let request = commands
        .into_iter()
        .find_map(|command| match command {
            Command::LoadCatalogPage(request) => Some(request),
            _ => None,
        })
        .expect("schema metadata request");
    let analytics_id = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "analytics"]);
    let schema = CatalogEntry::schema(
        analytics_id.clone(),
        database_id,
        QualifiedName {
            database: Some("app".into()),
            schema: Some("analytics".into()),
            object: "analytics".into(),
        },
        "schema",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let public_schema = app.explorer.normalized.profiles[&profile_id]
        .catalog
        .get(&schema_id)
        .unwrap()
        .clone();
    let page = CatalogPage::new(
        &request,
        vec![public_schema, schema],
        CatalogCount::Exact(2),
        None,
    )
    .unwrap();
    (app, analytics_id, page)
}

#[test]
fn missing_namespace_is_revealed_after_its_metadata_arrives() {
    let (mut app, schema_id, page) = pending_schema_reveal();

    app.update(Action::CatalogPageLoaded(page));

    assert_revealed(&app, ExplorerNodeId::Catalog(schema_id));
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn manual_navigation_cancels_a_pending_reveal() {
    let (mut app, schema_id, page) = pending_schema_reveal();
    let profile = ExplorerNodeId::Profile(schema_id.profile_id());
    app.update(Action::ExplorerSelect(profile.clone()));
    app.update(Action::ExplorerCollapse);

    app.update(Action::CatalogPageLoaded(page));

    assert_eq!(app.explorer.selected_id(), Some(&profile));
    assert!(!app.explorer.normalized.expanded.contains(&profile));
}

#[test]
fn metadata_for_a_previous_tab_does_not_override_the_current_tab() {
    let (mut app, _, page) = pending_schema_reveal();
    app.update(Action::ActivateTab(0));
    let current = app.explorer.selected_id().cloned();

    app.update(Action::CatalogPageLoaded(page));

    assert_eq!(app.explorer.selected_id(), current.as_ref());
}

#[test]
fn disabling_auto_reveal_cancels_a_pending_reveal() {
    let (mut app, _, page) = pending_schema_reveal();
    app.set_explorer_auto_reveal_active_tab(false);
    let current = app.explorer.selected_id().cloned();

    app.update(Action::CatalogPageLoaded(page));

    assert_eq!(app.explorer.selected_id(), current.as_ref());
}
