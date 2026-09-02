use lazydb::{
    action::Action,
    app::App,
    db::{catalog::CatalogId, catalog_mutation::CatalogMutationAnchor},
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
