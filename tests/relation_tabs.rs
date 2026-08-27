use lazydb::{
    action::Action,
    db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, ColumnMetadata, ConstraintMetadata,
        IndexMetadata, OptionalMetadata, QualifiedName,
    },
    db::query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
    db::value::CellValue,
    model::{
        explorer::CatalogTree,
        relation::{
            RelationDescriptor, RelationKey, RelationSnapshotProvenance, RelationTab, RelationView,
        },
        tab::WorkspaceTab,
    },
};
use uuid::Uuid;

#[test]
fn relation_and_supported_descendants_resolve_to_the_relation() {
    let profile = Uuid::new_v4();
    let database = entry(profile, CatalogKind::Database, "db", None);
    let schema = entry(profile, CatalogKind::Schema, "schema", None);
    let table = entry(
        profile,
        CatalogKind::Table,
        "table",
        Some(schema.id.clone()),
    );
    let mut tree = CatalogTree::new(profile);
    tree.insert_subtree(vec![database, schema.clone(), table.clone()])
        .unwrap();

    for kind in [
        CatalogKind::Column,
        CatalogKind::Index,
        CatalogKind::PrimaryKey,
        CatalogKind::UniqueConstraint,
        CatalogKind::ForeignKey,
        CatalogKind::CheckConstraint,
        CatalogKind::Trigger,
    ] {
        let id = CatalogId::new(profile, kind, [format!("table.{kind:?}")]);
        let child = if kind == CatalogKind::Trigger {
            CatalogEntry::relation_object(
                id,
                schema.id.clone(),
                table.id.clone(),
                qualified("trigger"),
                "trigger",
                OptionalMetadata::Unsupported,
            )
            .unwrap()
        } else {
            CatalogEntry::relation_child(
                id,
                table.id.clone(),
                qualified("child"),
                "child",
                OptionalMetadata::Unsupported,
                child_metadata(kind),
            )
            .unwrap()
        };
        tree.insert(child.clone()).unwrap();
        assert_eq!(tree.owning_relation_id(&child.id), Some(&table.id));
    }
    assert_eq!(tree.owning_relation_id(&table.id), Some(&table.id));
}

#[test]
fn missing_parent_and_cycle_return_none() {
    let profile = Uuid::new_v4();
    let tree = CatalogTree::new(profile);
    let missing = CatalogId::new(profile, CatalogKind::Column, ["missing"]);
    assert!(tree.owning_relation_id(&missing).is_none());
}

#[test]
fn opening_relation_is_a_semantic_action() {
    let profile_id = Uuid::new_v4();
    let object_id = CatalogId::new(profile_id, CatalogKind::Table, ["main", "users"]);
    let key = RelationKey {
        profile_id,
        object_id,
    };
    assert_eq!(
        Action::OpenSelectedRelation {
            view: RelationView::Data
        },
        Action::OpenSelectedRelation {
            view: RelationView::Data
        }
    );
    assert_eq!(key.profile_id, profile_id);
    let _ = WorkspaceTab::Relation;
}

#[test]
fn relation_view_reducer_switches_between_data_and_structure() {
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;

    app.update(Action::SetRelationView(RelationView::Structure));
    assert_eq!(relation_view(&app), RelationView::Structure);
    app.update(Action::SetRelationView(RelationView::Data));
    assert_eq!(relation_view(&app), RelationView::Data);
}

#[test]
fn relation_grid_actions_update_relation_grid_using_preview_dimensions() {
    let mut app = lazydb::app::App::new(Vec::new());
    let mut tab = RelationTab::new("users");
    tab.data =
        lazydb::model::relation::RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot::new(
            lazydb::db::RelationPreview {
                sql: "select".into(),
                result: QueryOutcome {
                    result_sets: vec![ResultSet {
                        columns: vec![
                            ColumnMeta {
                                name: "id".into(),
                                type_name: "int".into(),
                            },
                            ColumnMeta {
                                name: "name".into(),
                                type_name: "text".into(),
                            },
                        ],
                        rows: vec![vec![CellValue::Integer(1), CellValue::Text("a".into())]],
                        affected_rows: 0,
                    }],
                    stats: QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 1),
                },
            },
            lazydb::identity::ConnectionIdentity {
                profile_id: Uuid::nil(),
                generation: 0,
            },
            lazydb::profile::CatalogScope::for_profile(
                lazydb::profile::DatabaseKind::Sqlite,
                "db",
                None,
            ),
        ));
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = 1;
    app.update(Action::GridSelect { row: 0, column: 1 });

    let WorkspaceTab::Relation(tab) = &app.tabs[1] else {
        panic!()
    };
    assert_eq!(tab.grid.selected_column, 1);
    app.update(Action::GridMove {
        rows: 1,
        columns: -1,
    });
    let WorkspaceTab::Relation(tab) = &app.tabs[1] else {
        panic!()
    };
    assert_eq!(tab.grid.selected_row, 0);
    assert_eq!(tab.grid.selected_column, 0);
}

#[test]
fn shared_query_actions_preserve_relation_editing_and_submission() {
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::Where,
    ));
    app.update(Action::DataQueryInsert('i'));
    assert_eq!(relation_query(&app).where_input.value(), "i");
    assert!(app.update(Action::SubmitDataQuery).is_empty());
    assert_eq!(
        relation_query(&app).submitted.where_clause.as_deref(),
        Some("i")
    );
}

#[test]
fn relation_focus_cycles_only_explorer_and_results() {
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = lazydb::model::workspace::Focus::Results;
    app.update(Action::FocusNext);
    assert_eq!(app.focus, lazydb::model::workspace::Focus::Explorer);
    app.update(Action::FocusNext);
    assert_eq!(app.focus, lazydb::model::workspace::Focus::Results);
    app.update(Action::FocusPrevious);
    assert_eq!(app.focus, lazydb::model::workspace::Focus::Explorer);
}

#[test]
fn relation_snapshot_provenance_is_derived_from_current_connection_and_profile() {
    let profile = Uuid::new_v4();
    let connection = lazydb::identity::ConnectionIdentity {
        profile_id: profile,
        generation: 1,
    };
    let descriptor = RelationDescriptor {
        key: RelationKey {
            profile_id: profile,
            object_id: CatalogId::new(profile, CatalogKind::Table, ["db", "main", "users"]),
        },
        qualified_name: QualifiedName {
            database: Some("db".into()),
            schema: Some("main".into()),
            object: "users".into(),
        },
        kind: CatalogKind::Table,
        title: "users".into(),
    };
    let mut tab = RelationTab::with_descriptor(descriptor, RelationView::Data);
    tab.data =
        lazydb::model::relation::RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot::new(
            lazydb::db::RelationPreview {
                sql: "select".into(),
                result: lazydb::db::query::QueryOutcome {
                    result_sets: Vec::new(),
                    stats: lazydb::db::query::QueryStats::new(
                        std::time::Duration::ZERO,
                        std::time::Duration::ZERO,
                        0,
                    ),
                },
            },
            connection,
            lazydb::profile::CatalogScope::for_profile(
                lazydb::profile::DatabaseKind::Sqlite,
                "db",
                None,
            ),
        ));
    let profile_data = import_profile(profile);
    assert_eq!(
        tab.provenance(RelationView::Data, Some(connection), Some(&profile_data)),
        Some(RelationSnapshotProvenance::Live)
    );
    assert_eq!(
        tab.provenance(RelationView::Data, None, Some(&profile_data)),
        Some(RelationSnapshotProvenance::OfflineSnapshot)
    );
    assert_eq!(
        tab.provenance(RelationView::Data, Some(connection), None),
        Some(RelationSnapshotProvenance::ProfileDeletedSnapshot)
    );
}

fn import_profile(id: Uuid) -> lazydb::profile::ConnectionProfile {
    let mut profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("test"))
        .unwrap()
        .profile;
    profile.id = id;
    profile.catalog_scope.databases = lazydb::profile::CatalogSelection::All;
    profile
}

fn relation_view(app: &lazydb::app::App) -> RelationView {
    match &app.tabs[app.active_tab] {
        WorkspaceTab::Relation(tab) => tab.view,
        WorkspaceTab::Sql(_) => panic!("expected relation tab"),
    }
}

fn relation_query(app: &lazydb::app::App) -> &lazydb::model::data_query::DataQueryState {
    match &app.tabs[app.active_tab] {
        WorkspaceTab::Relation(tab) => &tab.query,
        WorkspaceTab::Sql(_) => panic!("expected relation tab"),
    }
}

fn entry(profile: Uuid, kind: CatalogKind, name: &str, parent: Option<CatalogId>) -> CatalogEntry {
    let id = CatalogId::new(profile, kind, [name]);
    match kind {
        CatalogKind::Database => CatalogEntry::database(
            id,
            qualified(name),
            "database",
            OptionalMetadata::Unsupported,
            true,
        )
        .unwrap(),
        CatalogKind::Schema => CatalogEntry::schema(
            id,
            parent.unwrap_or_else(|| CatalogId::new(profile, CatalogKind::Database, ["db"])),
            qualified(name),
            "schema",
            OptionalMetadata::Unsupported,
            true,
        )
        .unwrap(),
        CatalogKind::Table => CatalogEntry::relation(
            id,
            parent.unwrap(),
            qualified(name),
            "table",
            OptionalMetadata::Unsupported,
            true,
        )
        .unwrap(),
        _ => CatalogEntry::relation_child(
            id,
            parent.unwrap(),
            qualified(name),
            "child",
            OptionalMetadata::Unsupported,
            CatalogMetadata::default(),
        )
        .unwrap(),
    }
}

fn qualified(name: &str) -> QualifiedName {
    QualifiedName {
        database: Some("db".into()),
        schema: Some("schema".into()),
        object: name.into(),
    }
}

fn child_metadata(kind: CatalogKind) -> CatalogMetadata {
    match kind {
        CatalogKind::Column => CatalogMetadata::Column(ColumnMetadata::new(1, "text", true)),
        CatalogKind::Index => CatalogMetadata::Index(IndexMetadata {
            columns: vec!["id".into()],
            unique: false,
        }),
        CatalogKind::PrimaryKey => CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
            columns: vec!["id".into()],
        }),
        CatalogKind::UniqueConstraint => CatalogMetadata::Constraint(ConstraintMetadata::Unique {
            columns: vec!["id".into()],
        }),
        CatalogKind::ForeignKey => CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
            columns: vec!["id".into()],
            referenced_relation: qualified("other"),
            referenced_columns: vec!["id".into()],
        }),
        CatalogKind::CheckConstraint => CatalogMetadata::Constraint(ConstraintMetadata::Check {
            expression: "id > 0".into(),
        }),
        _ => CatalogMetadata::None,
    }
}
