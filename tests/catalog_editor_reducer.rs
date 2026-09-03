use lazydb::{
    action::Action,
    app::App,
    db::{
        catalog::{CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName},
        catalog_mutation::{CatalogMutationAnchor, CatalogObjectType},
    },
    model::{
        explorer::{ExplorerMutationIntent, ExplorerNodeId, StatusRowKind},
        workspace::Overlay,
    },
    profile::{ConnectionProfile, import_connection_url},
};
use uuid::Uuid;

fn profile() -> ConnectionProfile {
    import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile
}

fn catalog_id(profile_id: Uuid) -> CatalogId {
    CatalogId::new(
        profile_id,
        lazydb::db::catalog::CatalogKind::Table,
        ["app", "public", "users"],
    )
}

#[test]
fn explorer_selection_resolves_direct_nodes_without_inheriting_owner_actions() {
    let profile = profile();
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);

    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    assert_eq!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::EditProfile(profile_id))
    );
    assert_eq!(
        app.resolve_explorer_mutation_intent(false),
        Some(ExplorerMutationIntent::Create(
            CatalogMutationAnchor::Profile { profile_id }
        ))
    );
    app.update(Action::OpenCatalogEdit);
    assert!(matches!(app.overlay, Some(Overlay::ProfileManager)));

    let id = catalog_id(profile_id);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id.clone()));
    assert_eq!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::Edit(
            CatalogMutationAnchor::Catalog(id.clone())
        ))
    );
    assert_eq!(
        app.resolve_explorer_mutation_intent(false),
        Some(ExplorerMutationIntent::Create(
            CatalogMutationAnchor::Catalog(id)
        ))
    );

    app.explorer.normalized.selected = Some(ExplorerNodeId::Group {
        parent: CatalogId::new(
            profile_id,
            lazydb::db::catalog::CatalogKind::Schema,
            ["app", "public"],
        ),
        group: lazydb::db::catalog::ObjectGroup::Tables,
    });
    assert!(matches!(
        app.resolve_explorer_mutation_intent(false),
        Some(ExplorerMutationIntent::Create(
            CatalogMutationAnchor::Group { .. }
        ))
    ));
    assert_eq!(app.resolve_explorer_mutation_intent(true), None);

    app.explorer.normalized.selected = Some(ExplorerNodeId::Status {
        owner: lazydb::model::explorer::ExplorerOwnerId::Profile(profile_id),
        kind: StatusRowKind::Loading,
    });
    assert_eq!(app.resolve_explorer_mutation_intent(false), None);
    assert_eq!(app.resolve_explorer_mutation_intent(true), None);
}

#[test]
fn help_edit_shortcut_uses_direct_selection_resolution() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = lazydb::model::workspace::Focus::Explorer;

    let id = catalog_id(profile_id);
    let entry = lazydb::db::catalog::CatalogEntry::relation(
        id.clone(),
        CatalogId::new(
            profile_id,
            lazydb::db::catalog::CatalogKind::Schema,
            ["app", "public"],
        ),
        lazydb::db::catalog::QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "users".into(),
        },
        "table",
        lazydb::db::catalog::OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog;
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::database(
                CatalogId::new(
                    profile_id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: None,
                    object: "app".into(),
                },
                "database",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::schema(
                CatalogId::new(
                    profile_id,
                    lazydb::db::catalog::CatalogKind::Schema,
                    ["app", "public"],
                ),
                CatalogId::new(
                    profile_id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: Some("public".into()),
                    object: "public".into(),
                },
                "schema",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog.insert(entry).unwrap();
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id));
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("edit selected object".into()));
    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::ExplorerEditCatalog)
    );
    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::ExplorerEditProfile,
    ));
    assert!(app.profile_manager.is_none());
    assert!(app.catalog_editor.is_none());
    assert_ne!(app.overlay, Some(Overlay::CatalogEditor));
}

#[test]
fn opening_create_on_schema_uses_capability_ordered_options() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });
    let database = CatalogId::new(profile_id, CatalogKind::Database, ["app"]);
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog;
    catalog
        .insert(
            CatalogEntry::database(
                database.clone(),
                QualifiedName {
                    database: Some("app".into()),
                    schema: None,
                    object: "app".into(),
                },
                "database",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog
        .insert(
            CatalogEntry::schema(
                schema.clone(),
                database,
                QualifiedName {
                    database: Some("app".into()),
                    schema: Some("public".into()),
                    object: "public".into(),
                },
                "schema",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    app.explorer.normalized.selected =
        Some(lazydb::model::explorer::ExplorerNodeId::Catalog(schema));
    let catalog_epoch = app
        .explorer
        .normalized
        .profiles
        .get(&profile_id)
        .unwrap()
        .catalog_epoch;

    app.update(Action::OpenCatalogCreate);
    let editor = app.catalog_editor.as_ref().expect("catalog editor");
    assert_eq!(
        editor
            .options
            .iter()
            .map(|option| option.object_type)
            .collect::<Vec<_>>(),
        vec![
            CatalogObjectType::Catalog(CatalogKind::Table),
            CatalogObjectType::Catalog(CatalogKind::View),
            CatalogObjectType::Catalog(CatalogKind::MaterializedView),
            CatalogObjectType::Catalog(CatalogKind::Sequence),
        ]
    );
    assert_eq!(editor.catalog_epoch, catalog_epoch);

    app.update(Action::CatalogEditorMove(1));
    app.update(Action::CatalogEditorSelect);
    let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert!(draft.security_invoker.availability.is_available());

    app.update(Action::CatalogEditorCancel);
    app.explorer.normalized.selected = Some(lazydb::model::explorer::ExplorerNodeId::Group {
        parent: CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]),
        group: lazydb::db::catalog::ObjectGroup::Tables,
    });
    app.update(Action::OpenCatalogCreate);
    let editor = app.catalog_editor.as_ref().expect("table editor");
    assert_eq!(
        editor.page,
        lazydb::model::catalog_editor::CatalogEditorPage::Form
    );
    assert_eq!(
        editor.object_type,
        Some(CatalogObjectType::Catalog(CatalogKind::Table))
    );
    assert!(matches!(
        editor.draft,
        Some(lazydb::model::catalog_editor::CatalogDraft::Table(_))
    ));

    app.update(Action::CatalogEditorCancel);
    app.explorer.normalized.selected = Some(lazydb::model::explorer::ExplorerNodeId::Catalog(
        CatalogId::new(profile_id, CatalogKind::Database, ["app"]),
    ));
    app.update(Action::OpenCatalogCreate);
    let editor = app.catalog_editor.as_ref().expect("schema editor");
    assert_eq!(
        editor.page,
        lazydb::model::catalog_editor::CatalogEditorPage::Form
    );
    assert_eq!(
        editor.object_type,
        Some(CatalogObjectType::Catalog(CatalogKind::Schema))
    );
    assert!(matches!(
        editor.draft,
        Some(lazydb::model::catalog_editor::CatalogDraft::Schema(_))
    ));

    app.update(Action::CatalogEditorInsert('n'));
    app.update(Action::CatalogEditorFieldNext);
    app.update(Action::CatalogEditorInsert('o'));
    app.update(Action::CatalogEditorFocusField(2));
    app.update(Action::CatalogEditorInsert('c'));
    app.update(Action::CatalogEditorFieldPrevious);
    app.update(Action::CatalogEditorInsert('w'));
    app.update(Action::CatalogEditorFocusField(2));
    let Some(lazydb::model::catalog_editor::CatalogDraft::Schema(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("schema draft expected");
    };
    assert_eq!(draft.name.value(), "n");
    assert_eq!(draft.owner.value(), "ow");
    assert_eq!(draft.comment.value(), "c");
    assert_eq!(draft.selected_field, 2);
}

#[test]
fn views_group_opens_view_form_with_connected_capabilities() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Group {
        parent: schema,
        group: lazydb::db::catalog::ObjectGroup::Views,
    });

    app.update(Action::OpenCatalogCreate);

    let Some(lazydb::model::catalog_editor::CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert!(draft.security_invoker.availability.is_available());
}

#[test]
fn mutation_refresh_api_accepts_unique_targets_and_leaves_selection_for_reload() {
    let profile = profile();
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let target = lazydb::db::catalog::CatalogTarget::Databases;
    let commands = app.commands_for_catalog_targets(profile_id, &[target.clone(), target.clone()]);
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, lazydb::action::Command::LoadCatalogPage(_)))
            .count(),
        0
    );
    assert_eq!(
        app.explorer.selected_id(),
        Some(&ExplorerNodeId::Profile(profile_id))
    );
}

#[test]
fn relation_invalidation_is_exposed_by_catalog_mutation_impact() {
    let profile = profile();
    let profile_id = profile.id;
    let relation = CatalogId::new(
        profile_id,
        lazydb::db::catalog::CatalogKind::Table,
        ["db", "public", "users"],
    );
    let tab = lazydb::model::relation::RelationTab::with_descriptor(
        lazydb::model::relation::RelationDescriptor {
            key: lazydb::model::relation::RelationKey {
                profile_id,
                object_id: relation.clone(),
            },
            qualified_name: lazydb::db::catalog::QualifiedName {
                database: Some("db".into()),
                schema: Some("public".into()),
                object: "users".into(),
            },
            kind: lazydb::db::catalog::CatalogKind::Table,
            title: "users".into(),
        },
        lazydb::model::relation::RelationView::Data,
    );
    let impact = lazydb::db::catalog_mutation::CatalogMutationImpact {
        old_object_id: CatalogId::new(
            profile_id,
            lazydb::db::catalog::CatalogKind::Column,
            ["db", "public", "users", "id"],
        ),
        owning_relation_id: Some(relation),
        namespace: lazydb::db::catalog_mutation::CatalogMutationNamespace {
            database: None,
            schema: None,
        },
        native_identity_changed: false,
    };
    assert!(tab.invalidated_by_catalog_mutation(&impact));
}

#[test]
fn constraint_edit_is_allowed_from_a_direct_catalog_selection() {
    let profile = profile();
    let mut app = App::new(vec![profile.clone()]);
    let id = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::ForeignKey,
        ["app", "public", "events", "42", "9"],
    );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id.clone()));
    assert_eq!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::Edit(
            CatalogMutationAnchor::Catalog(id)
        ))
    );
}

#[test]
fn sequence_edit_is_allowed_from_a_direct_catalog_selection() {
    let profile = profile();
    let mut app = App::new(vec![profile.clone()]);
    let id = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::Sequence,
        ["app", "public", "seq", "42"],
    );
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id));
    assert!(matches!(
        app.resolve_explorer_mutation_intent(true),
        Some(ExplorerMutationIntent::Edit(_))
    ));
}

#[test]
fn view_edit_dispatches_definition_load_and_accepts_matching_view_definition() {
    let profile = import_connection_url("postgres://localhost/app", Some("postgres-test"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    let generation = match app.update(Action::RequestConnect(profile.id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });
    let id = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::View,
        ["app", "public", "v", "42"],
    );
    let schema = CatalogId::new(
        profile.id,
        lazydb::db::catalog::CatalogKind::Schema,
        ["app", "public"],
    );
    let entry = lazydb::db::catalog::CatalogEntry::relation(
        id.clone(),
        schema,
        lazydb::db::catalog::QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "v".into(),
        },
        "view",
        lazydb::db::catalog::OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog;
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::database(
                CatalogId::new(
                    profile.id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: None,
                    object: "app".into(),
                },
                "database",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog
        .insert(
            lazydb::db::catalog::CatalogEntry::schema(
                CatalogId::new(
                    profile.id,
                    lazydb::db::catalog::CatalogKind::Schema,
                    ["app", "public"],
                ),
                CatalogId::new(
                    profile.id,
                    lazydb::db::catalog::CatalogKind::Database,
                    ["app"],
                ),
                lazydb::db::catalog::QualifiedName {
                    database: Some("app".into()),
                    schema: Some("public".into()),
                    object: "public".into(),
                },
                "schema",
                lazydb::db::catalog::OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog
        .insert(entry)
        .unwrap();
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(id.clone()));
    let commands = app.update(Action::OpenCatalogEdit);
    assert!(
        matches!(commands.as_slice(), [lazydb::action::Command::LoadCatalogObjectDefinition(request)] if request.object == id)
    );
}
