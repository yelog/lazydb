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

#[test]
fn table_navigation_reaches_and_leaves_every_action() {
    let mut app = App::new(Vec::new());
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        lazydb::db::catalog_mutation::CatalogMutationAnchor::Group {
            schema: CatalogId::new(Uuid::nil(), CatalogKind::Schema, ["app", "public"]),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Table),
            label: "Table".into(),
        }],
    );
    assert!(editor.select_object_type(CatalogObjectType::Catalog(CatalogKind::Table)));
    app.catalog_editor = Some(editor);
    app.catalog_editor_compact.set(true);

    app.update(Action::CatalogEditorFieldNext);
    assert!(matches!(
        app.catalog_editor.as_ref().and_then(|editor| editor.draft.as_ref()),
        Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft))
            if draft.focus
                == lazydb::model::catalog_editor::TableEditorFocus::General(
                    lazydb::model::catalog_editor::TableGeneralField::Schema
                )
    ));
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.focus = lazydb::model::catalog_editor::TableEditorFocus::Columns;
    }
    for expected in [
        lazydb::model::catalog_editor::TableActionField::AddColumn,
        lazydb::model::catalog_editor::TableActionField::RemoveColumn,
        lazydb::model::catalog_editor::TableActionField::Review,
        lazydb::model::catalog_editor::TableActionField::Cancel,
    ] {
        app.update(Action::CatalogEditorFieldNext);
        assert!(matches!(
            app.catalog_editor.as_ref().and_then(|editor| editor.draft.as_ref()),
            Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft))
                if draft.focus == lazydb::model::catalog_editor::TableEditorFocus::Action(expected)
        ));
    }
    for expected in [
        lazydb::model::catalog_editor::TableActionField::Review,
        lazydb::model::catalog_editor::TableActionField::RemoveColumn,
        lazydb::model::catalog_editor::TableActionField::AddColumn,
        lazydb::model::catalog_editor::TableActionField::AddColumn,
    ] {
        app.update(Action::CatalogEditorFieldPrevious);
        assert!(matches!(
            app.catalog_editor.as_ref().and_then(|editor| editor.draft.as_ref()),
            Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft))
                if draft.focus == lazydb::model::catalog_editor::TableEditorFocus::Action(expected)
                    || expected == lazydb::model::catalog_editor::TableActionField::AddColumn
                        && draft.focus == lazydb::model::catalog_editor::TableEditorFocus::Columns
        ));
    }
}

fn table_editor_for_paste() -> App {
    let mut app = App::new(Vec::new());
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        CatalogMutationAnchor::Group {
            schema: CatalogId::new(Uuid::nil(), CatalogKind::Schema, ["app", "public"]),
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        0,
        vec![lazydb::model::catalog_editor::CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Table),
            label: "Table".into(),
        }],
    );
    assert!(editor.select_object_type(CatalogObjectType::Catalog(CatalogKind::Table)));
    app.catalog_editor = Some(editor);
    app
}

#[test]
fn catalog_editor_paste_writes_multicharacter_table_name_at_general_name_focus() {
    let mut app = table_editor_for_paste();

    app.update(Action::CatalogEditorPaste("events\n数据🙂".into()));

    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(draft.name.value(), "events\n数据🙂");
}

#[test]
fn catalog_editor_paste_writes_multicharacter_column_name_at_column_name_focus() {
    let mut app = table_editor_for_paste();
    draft_mut(&mut app).focus = lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
        lazydb::model::catalog_editor::TableColumnField::Name,
    );

    app.update(Action::CatalogEditorPaste("user\n名前🙂".into()));

    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(draft.columns[0].name.value(), "user\n名前🙂");
}

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
            current_user: Some("effective_role".into()),
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
    let owner_request = app
        .update(Action::OpenCatalogCreate)
        .into_iter()
        .find_map(|command| match command {
            lazydb::action::Command::LoadCatalogOwnerContext(request) => Some(request),
            _ => None,
        });
    assert!(
        owner_request.is_none(),
        "owner request should be deduplicated"
    );
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
    app.update(Action::CatalogEditorEditTableColumn);
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name,
        )
    );
    app.update(Action::CatalogEditorLeaveTableColumnDetails);
    let Some(lazydb::model::catalog_editor::CatalogDraft::Table(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("table draft expected");
    };
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::Columns
    );

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
    let Some(lazydb::model::catalog_editor::CatalogDraft::Schema(draft)) = editor.draft.as_ref()
    else {
        panic!("schema draft expected");
    };
    assert_eq!(draft.owner.value(), "effective_role");
    if let Some(lazydb::model::catalog_editor::CatalogDraft::Schema(draft)) = app
        .catalog_editor
        .as_mut()
        .and_then(|editor| editor.draft.as_mut())
    {
        draft.owner.set("");
    }

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
            current_user: Some("postgres".into()),
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
fn table_column_selection_and_add_actions_sync_focus_and_details() {
    let profile = profile();
    let schema = CatalogId::new(profile.id, CatalogKind::Schema, ["app", "public"]);
    let mut editor = lazydb::model::catalog_editor::CatalogEditorState::new(
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        CatalogMutationAnchor::Group {
            schema,
            group: lazydb::db::catalog::ObjectGroup::Tables,
        },
        1,
        Vec::new(),
    );
    assert!(editor.select_object_type(CatalogObjectType::Catalog(CatalogKind::Table)));
    let mut app = App::new(vec![profile]);
    app.catalog_editor = Some(editor);

    app.update(Action::CatalogEditorSelectTableColumn(0));
    let draft = app.catalog_editor.as_ref().unwrap().draft.as_ref().unwrap();
    let lazydb::model::catalog_editor::CatalogDraft::Table(draft) = draft else {
        panic!("table draft expected");
    };
    assert_eq!(draft.selected_column, 0);
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::Columns
    );

    draft_mut(&mut app).columns[0].name = "id".into();
    app.update(Action::CatalogEditorAddTableColumn);
    let draft = app.catalog_editor.as_ref().unwrap().draft.as_ref().unwrap();
    let lazydb::model::catalog_editor::CatalogDraft::Table(draft) = draft else {
        panic!("table draft expected");
    };
    assert_eq!(draft.columns[0].name.value(), "id");
    assert_eq!(draft.selected_column, 1);
    assert_eq!(
        draft.focus,
        lazydb::model::catalog_editor::TableEditorFocus::ColumnDetails(
            lazydb::model::catalog_editor::TableColumnField::Name
        )
    );
}

#[test]
fn catalog_mutation_failure_from_an_old_connection_is_ignored() {
    let old_profile = profile();
    let old_profile_id = old_profile.id;
    let new_profile = import_connection_url(":memory:", Some("new"))
        .unwrap()
        .profile;
    let old_connection = lazydb::identity::ConnectionIdentity {
        profile_id: old_profile.id,
        generation: 1,
    };
    let mut app = App::new(vec![old_profile, new_profile.clone()]);
    app.update(Action::ConnectionSucceeded {
        profile_id: new_profile.id,
        generation: 1,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3.50.0".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });

    let anchor = CatalogMutationAnchor::Catalog(CatalogId::new(
        old_profile_id,
        CatalogKind::Database,
        ["app"],
    ));
    let request = lazydb::db::catalog_mutation::CatalogMutationRequest::new(
        old_connection,
        2,
        0,
        lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor.clone(),
        CatalogObjectType::Catalog(CatalogKind::Schema),
    )
    .unwrap();
    let plan = lazydb::db::catalog_mutation::CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(CatalogKind::Schema),
        lazydb::db::catalog_mutation::CatalogMutationExecutionMode::Transactional,
        lazydb::db::catalog_mutation::CatalogMutationTarget::maintenance("postgres").unwrap(),
        vec![lazydb::db::catalog::CatalogTarget::Databases],
        lazydb::db::catalog_mutation::CatalogSelectionHint::Parent(
            lazydb::db::catalog::CatalogTarget::Databases,
        ),
        None,
        Vec::new(),
        vec!["CREATE SCHEMA events".into()],
    )
    .unwrap();
    app.catalog_editor = Some(lazydb::model::catalog_editor::CatalogEditorState {
        mode: lazydb::db::catalog_mutation::CatalogMutationMode::Create,
        anchor,
        object_type: Some(CatalogObjectType::Catalog(CatalogKind::Schema)),
        page: lazydb::model::catalog_editor::CatalogEditorPage::SqlPreview,
        operation: Some(
            lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 2 },
        ),
        catalog_epoch: 0,
        options: Vec::new(),
        selected_option: 0,
        draft: None,
        baseline: None,
        plan: Some(plan.clone()),
        error: None,
        owner_picker: Default::default(),
    });

    app.update(Action::CatalogMutationFailed {
        plan,
        message: "old connection failed".into(),
    });

    let editor = app.catalog_editor.as_ref().unwrap();
    assert!(matches!(
        editor.operation,
        Some(lazydb::model::catalog_editor::CatalogEditorOperation::Applying { request_id: 2 })
    ));
    assert_eq!(editor.error, None);
}

fn draft_mut(app: &mut App) -> &mut lazydb::model::catalog_editor::TableDraft {
    let draft = app.catalog_editor.as_mut().unwrap().draft.as_mut().unwrap();
    let lazydb::model::catalog_editor::CatalogDraft::Table(draft) = draft else {
        panic!("table draft expected");
    };
    draft
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
            current_user: Some("postgres".into()),
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
