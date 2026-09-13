use chrono::Datelike;
use lazydb::{
    action::Action,
    db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, ColumnMetadata, ConstraintMetadata,
        IndexMetadata, OptionalMetadata, QualifiedName,
    },
    db::catalog_mutation::{CatalogMutationImpact, CatalogMutationNamespace},
    db::query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
    db::value::CellValue,
    model::{
        data_query::DataQueryCapability,
        editor::EditorViewport,
        explorer::CatalogTree,
        relation::{
            RelationDescriptor, RelationKey, RelationSnapshotProvenance, RelationTab, RelationView,
        },
        tab::{ConsoleTab, ResultView, WorkspaceTab},
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
fn explicit_catalog_id_opens_relation_without_changing_explorer_selection() {
    let profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("local"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = lazydb::app::App::new(vec![profile.clone()]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });

    let database_id = CatalogId::new(profile_id, CatalogKind::Database, [":memory:"]);
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "main"]);
    let table_id = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "users"],
    );
    let database = CatalogEntry::database(
        database_id.clone(),
        QualifiedName {
            database: Some(":memory:".into()),
            schema: None,
            object: ":memory:".into(),
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
            database: Some(":memory:".into()),
            schema: Some("main".into()),
            object: "main".into(),
        },
        "schema",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let table = CatalogEntry::relation(
        table_id.clone(),
        schema_id,
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("main".into()),
            object: "users".into(),
        },
        "table",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let tree = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog;
    tree.insert_subtree(vec![database, schema, table]).unwrap();
    let other_id = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "other"],
    );
    app.explorer.normalized.selected = Some(lazydb::model::explorer::ExplorerNodeId::Catalog(
        other_id.clone(),
    ));

    app.update(Action::OpenCatalogRelation {
        id: table_id.clone(),
        view: RelationView::Data,
    });

    assert_eq!(
        app.explorer.selected_id(),
        Some(&lazydb::model::explorer::ExplorerNodeId::Catalog(other_id))
    );
    assert!(app.tabs.iter().any(|tab| matches!(
        tab,
        WorkspaceTab::Relation(tab) if tab.descriptor.key.object_id == table_id
    )));
}

#[test]
fn relation_view_reducer_switches_between_data_and_ddl() {
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;

    app.update(Action::SetRelationView(RelationView::Ddl));
    assert_eq!(relation_view(&app), RelationView::Ddl);
    app.update(Action::SetRelationView(RelationView::Data));
    assert_eq!(relation_view(&app), RelationView::Data);
}

#[test]
fn ddl_viewport_defaults_to_zero() {
    let tab = RelationTab::new("users");

    assert_eq!(tab.ddl_viewport.row_offset, 0);
    assert_eq!(tab.ddl_viewport.column_offset, 0);
    assert_eq!(tab.ddl_viewport.visible_rows, 0);
    assert_eq!(tab.ddl_viewport.visible_columns, 0);
    assert_eq!(tab.ddl_viewport.total_rows, 0);
    assert_eq!(tab.ddl_viewport.max_line_width, 0);
}

#[test]
fn catalog_mutation_invalidates_matching_relation_snapshots_only() {
    let profile = Uuid::new_v4();
    let schema = CatalogId::new(profile, CatalogKind::Schema, ["db", "public"]);
    let users = CatalogId::new(profile, CatalogKind::Table, ["db", "public", "users"]);
    let orders = CatalogId::new(profile, CatalogKind::Table, ["db", "public", "orders"]);
    let impact = CatalogMutationImpact {
        old_object_id: CatalogId::new(
            profile,
            CatalogKind::Column,
            ["db", "public", "users", "id"],
        ),
        owning_relation_id: Some(users.clone()),
        namespace: CatalogMutationNamespace {
            database: None,
            schema: None,
        },
        native_identity_changed: false,
    };
    let matching = RelationTab::with_descriptor(descriptor(users, "users"), RelationView::Data);
    let unrelated = RelationTab::with_descriptor(descriptor(orders, "orders"), RelationView::Data);
    assert!(matching.invalidated_by_catalog_mutation(&impact));
    assert!(!unrelated.invalidated_by_catalog_mutation(&impact));

    let schema_impact = CatalogMutationImpact {
        old_object_id: schema.clone(),
        owning_relation_id: None,
        namespace: CatalogMutationNamespace {
            database: None,
            schema: Some(schema),
        },
        native_identity_changed: true,
    };
    assert!(matching.invalidated_by_catalog_mutation(&schema_impact));
}

#[test]
fn renamed_relation_is_marked_as_native_identity_stale() {
    let profile = Uuid::new_v4();
    let id = CatalogId::new(profile, CatalogKind::Table, ["db", "public", "users"]);
    let impact = CatalogMutationImpact {
        old_object_id: id.clone(),
        owning_relation_id: None,
        namespace: CatalogMutationNamespace {
            database: None,
            schema: None,
        },
        native_identity_changed: true,
    };
    let mut tab = RelationTab::with_descriptor(descriptor(id, "users"), RelationView::Data);
    assert!(tab.invalidated_by_catalog_mutation(&impact));
    tab.invalidate_catalog_mutation(true);
    assert!(tab.stale_native_identity);
}

#[test]
fn rebinding_relation_keeps_tab_state_and_invalidates_requests() {
    let profile = Uuid::new_v4();
    let old = CatalogId::new(profile, CatalogKind::Table, ["db", "public", "users"]);
    let new = CatalogId::new(profile, CatalogKind::Table, ["db", "public", "accounts"]);
    let mut tab = RelationTab::with_descriptor(descriptor(old, "users"), RelationView::Ddl);
    let id = tab.id;
    tab.generation = 7;
    tab.next_request_id = 12;
    tab.pagination.offset = 40;
    tab.grid.selected_row = 3;
    tab.stale_native_identity = false;

    tab.rebind_descriptor(descriptor(new.clone(), "accounts"), false);

    assert_eq!(tab.id, id);
    assert_eq!(tab.view, RelationView::Ddl);
    assert_eq!(tab.generation, 8);
    assert_eq!(tab.next_request_id, 12);
    assert_eq!(tab.pagination.offset, 40);
    assert_eq!(tab.grid.selected_row, 3);
    assert_eq!(tab.descriptor.key.object_id, new);
    assert_eq!(tab.title(), "accounts");
    assert!(!tab.stale_native_identity);
    assert!(matches!(
        tab.data,
        lazydb::model::relation::RelationLoad::Failed { .. }
    ));
    assert!(matches!(
        tab.ddl,
        lazydb::model::relation::RelationLoad::Failed { .. }
    ));
}

#[test]
fn rebinding_dirty_relation_preserves_draft_and_blocks_writes() {
    let profile = Uuid::new_v4();
    let old = CatalogId::new(profile, CatalogKind::Table, ["users"]);
    let new = CatalogId::new(profile, CatalogKind::Table, ["accounts"]);
    let mut tab = RelationTab::with_descriptor(descriptor(old, "users"), RelationView::Data);
    let mut edit =
        lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![CellValue::Text(
            "draft".into(),
        )]]);
    edit.rows[0].state = lazydb::model::relation_edit::EditableRowState::Updated {
        changed_columns: [0].into_iter().collect(),
    };
    tab.edit = Some(edit.clone());
    tab.transaction_generation = 4;

    tab.rebind_descriptor(descriptor(new, "accounts"), true);

    assert_eq!(tab.edit, Some(edit));
    assert!(tab.stale_native_identity);
    assert_eq!(tab.transaction_generation, 5);
}

#[test]
fn schema_and_database_impacts_match_open_relation_namespace() {
    let profile = Uuid::new_v4();
    let id = CatalogId::new(profile, CatalogKind::Table, ["db", "public", "users"]);
    let tab = RelationTab::with_descriptor(descriptor(id, "users"), RelationView::Data);
    let database = CatalogMutationImpact {
        old_object_id: CatalogId::new(profile, CatalogKind::Database, ["db"]),
        owning_relation_id: None,
        namespace: CatalogMutationNamespace {
            database: Some(CatalogId::new(profile, CatalogKind::Database, ["db"])),
            schema: None,
        },
        native_identity_changed: true,
    };
    assert!(tab.invalidated_by_catalog_mutation(&database));
}

#[test]
fn restored_relation_is_an_empty_transient_shell() {
    let id = Uuid::new_v4();
    let profile = Uuid::new_v4();
    let tab = RelationTab::restored(
        id,
        RelationDescriptor {
            key: RelationKey {
                profile_id: profile,
                object_id: CatalogId::new(profile, CatalogKind::Table, ["users"]),
            },
            qualified_name: qualified("users"),
            kind: CatalogKind::Table,
            title: "users".into(),
        },
        RelationView::Ddl,
    );

    assert_eq!(tab.id, id);
    assert_eq!(tab.view, RelationView::Ddl);
    assert!(matches!(
        tab.data,
        lazydb::model::relation::RelationLoad::Empty
    ));
    assert!(matches!(
        tab.ddl,
        lazydb::model::relation::RelationLoad::Empty
    ));
    assert!(tab.edit.is_none());
    assert_eq!(
        tab.transaction_state,
        lazydb::model::transaction::TransactionState::Idle
    );
}

#[test]
fn ddl_scroll_saturates_and_clamps_to_viewport_bounds() {
    let mut app = app_with_relation(RelationView::Ddl);
    app.update(Action::SetDdlViewportMetrics {
        visible_rows: 10,
        visible_columns: 20,
        total_rows: 25,
        max_line_width: 50,
    });

    app.update(Action::DdlScroll {
        rows: 100,
        columns: 100,
    });
    assert_eq!(ddl_offsets(&app), (15, 30));

    app.update(Action::DdlScroll {
        rows: -100,
        columns: -100,
    });
    assert_eq!(ddl_offsets(&app), (0, 0));
}

#[test]
fn ddl_editor_viewport_sync_uses_explicit_session_without_results_focus() {
    let mut app = app_with_relation(RelationView::Ddl);
    app.focus = lazydb::model::workspace::Focus::Explorer;
    let session_id = relation_tab(&app).ddl_editor_id;
    let viewport = EditorViewport {
        width: 24,
        height: 8,
    };

    app.update(Action::DdlEditorViewportChanged {
        session_id,
        viewport,
    });

    assert_eq!(app.active_ddl_editor_viewport().unwrap(), viewport);
}

#[test]
fn ddl_scroll_start_end_and_metric_changes_clamp_offsets() {
    let mut app = app_with_relation(RelationView::Ddl);
    app.update(Action::SetDdlViewportMetrics {
        visible_rows: 0,
        visible_columns: 0,
        total_rows: 5,
        max_line_width: 8,
    });
    app.update(Action::DdlScrollToEnd);
    assert_eq!(ddl_offsets(&app), (5, 8));

    app.update(Action::SetDdlViewportMetrics {
        visible_rows: 4,
        visible_columns: 7,
        total_rows: 5,
        max_line_width: 8,
    });
    assert_eq!(ddl_offsets(&app), (1, 1));

    app.update(Action::DdlScrollToStart);
    assert_eq!(ddl_offsets(&app), (0, 0));
}

#[test]
fn ddl_viewport_actions_only_affect_the_active_relation_ddl_view() {
    let mut app = app_with_relation(RelationView::Data);
    app.update(Action::SetDdlViewportMetrics {
        visible_rows: 1,
        visible_columns: 1,
        total_rows: 10,
        max_line_width: 10,
    });
    app.update(Action::DdlScroll {
        rows: 5,
        columns: 5,
    });
    assert_eq!(ddl_offsets(&app), (0, 0));
    assert_eq!(relation_tab(&app).ddl_viewport.total_rows, 0);

    app.active_tab = 1;
    app.update(Action::SetDdlViewportMetrics {
        visible_rows: 1,
        visible_columns: 1,
        total_rows: 10,
        max_line_width: 10,
    });
    assert_eq!(relation_tab_at(&app, 1).ddl_viewport.total_rows, 0);
}

#[test]
fn ddl_viewport_survives_workspace_switches_and_refresh() {
    let mut app = app_with_relation(RelationView::Ddl);
    app.update(Action::SetDdlViewportMetrics {
        visible_rows: 2,
        visible_columns: 3,
        total_rows: 10,
        max_line_width: 12,
    });
    app.update(Action::DdlScroll {
        rows: 4,
        columns: 5,
    });

    app.update(Action::PreviousTab);
    app.update(Action::NextTab);
    assert_eq!(ddl_offsets(&app), (4, 5));

    app.update(Action::RefreshActiveRelation);
    assert_eq!(ddl_offsets(&app), (4, 5));
}

#[test]
fn relation_refresh_returns_to_first_page_and_forgets_exact_total() {
    let mut app = app_with_relation(RelationView::Data);
    let mut profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("test"))
        .unwrap()
        .profile;
    profile.id = uuid::Uuid::nil();
    app.profiles.push(profile);
    app.connection.profile_id = Some(uuid::Uuid::nil());
    app.connection.generation = 1;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.target = Some(lazydb::model::execution_target::ExecutionTarget {
        profile_id: uuid::Uuid::nil(),
        database: ":memory:".into(),
        schema: None,
    });
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.pagination = lazydb::model::pagination::ResultPagination {
            page_size: lazydb::model::pagination::PageSize::Ten,
            offset: 20,
            visible_rows: 10,
            has_next: false,
            total: lazydb::model::pagination::TotalRows::Exact(30),
        };
    }
    let commands = app.update(Action::RefreshActiveRelation);
    assert!(commands.iter().any(|command| matches!(
        command,
        lazydb::action::Command::LoadRelationPreview(request)
            if request.page.offset == 0 && !request.page.resolve_total
    )));
    assert_eq!(relation_tab(&app).pagination.offset, 0);
    assert_eq!(
        relation_tab(&app).pagination.total,
        lazydb::model::pagination::TotalRows::LowerBound(0)
    );
}

#[test]
fn relation_sort_resets_page_without_changing_grid_layout() {
    let mut app = app_with_relation_columns(&["id", "name"]);
    let mut profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("test"))
        .unwrap()
        .profile;
    profile.id = uuid::Uuid::nil();
    app.profiles.push(profile);
    app.connection.profile_id = Some(uuid::Uuid::nil());
    app.connection.generation = 1;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.target = Some(lazydb::model::execution_target::ExecutionTarget {
        profile_id: uuid::Uuid::nil(),
        database: ":memory:".into(),
        schema: None,
    });
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.pagination.page_size = lazydb::model::pagination::PageSize::Ten;
        tab.pagination.offset = 20;
        tab.grid.column_widths = vec![Some(17), Some(23)];
        tab.grid.column_offset = 1;
    }

    let commands = app.update(Action::CycleDataColumnSort(0));
    assert!(commands.iter().any(|command| matches!(
        command,
        lazydb::action::Command::LoadRelationPreview(request)
            if request.page.size == lazydb::model::pagination::PageSize::Ten
                && request.page.offset == 0
    )));
    let tab = relation_tab(&app);
    assert_eq!(
        tab.pagination.page_size,
        lazydb::model::pagination::PageSize::Ten
    );
    assert_eq!(tab.pagination.offset, 0);
    assert_eq!(tab.grid.column_widths, vec![Some(17), Some(23)]);
    assert_eq!(tab.grid.column_offset, 1);
}

#[test]
fn relation_dirty_edits_block_refresh_and_all_navigation() {
    let mut app = app_with_relation(RelationView::Data);
    let WorkspaceTab::Relation(tab) = &mut app.tabs[1] else {
        panic!()
    };
    tab.edit = Some(
        lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![CellValue::Text(
            "x".into(),
        )]]),
    );
    assert!(
        tab.edit
            .as_mut()
            .unwrap()
            .update_cell(0, 0, CellValue::Text("changed".into()))
    );
    for action in [
        Action::RefreshActiveRelation,
        Action::RelationFirstPage,
        Action::RelationPreviousPage,
        Action::RelationNextPage,
        Action::RelationLastPage,
    ] {
        assert!(app.update(action).is_empty());
    }
}

#[test]
fn relation_visual_selection_does_not_block_refresh() {
    let mut app = app_with_relation_columns(&["value"]);
    let mut profile = lazydb::profile::import_connection_url("sqlite::memory:", Some("test"))
        .unwrap()
        .profile;
    profile.id = Uuid::nil();
    app.profiles.push(profile);
    app.connection.profile_id = Some(Uuid::nil());
    app.connection.generation = 1;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.target = Some(lazydb::model::execution_target::ExecutionTarget {
        profile_id: Uuid::nil(),
        database: ":memory:".into(),
        schema: None,
    });
    let WorkspaceTab::Relation(tab) = &mut app.tabs[1] else {
        panic!()
    };
    tab.edit = Some(
        lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![CellValue::Text(
            "x".into(),
        )]]),
    );
    tab.edit.as_mut().unwrap().mode =
        lazydb::model::relation_edit::RelationGridMode::VisualLine { anchor: 0 };

    let commands = app.update(Action::RefreshActiveRelation);
    assert!(matches!(
        commands.as_slice(),
        [lazydb::action::Command::LoadRelationPreview(_)]
    ));
}

#[test]
fn relation_cell_edit_open_and_cancel_do_not_emit_write_commands() {
    let mut app = app_with_relation_columns(&["id", "name"]);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(
            lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
                CellValue::Integer(1),
                CellValue::Text("old".into()),
            ]]),
        );
        tab.grid.selected_column = 1;
    }

    assert!(app.update(Action::RelationEditCell).is_empty());
    assert!(matches!(
        relation_tab(&app).edit.as_ref().unwrap().mode,
        lazydb::model::relation_edit::RelationGridMode::EditCell(_)
    ));

    assert!(app.update(Action::RelationEditCancel).is_empty());
    let edit = relation_tab(&app).edit.as_ref().unwrap();
    assert_eq!(
        edit.mode,
        lazydb::model::relation_edit::RelationGridMode::Browse
    );
    assert_eq!(edit.rows[0].current[1], CellValue::Text("old".into()));
    assert!(edit.pending_save.is_empty());
}

#[test]
fn relation_cell_edit_confirm_changes_only_the_local_draft() {
    let mut app = app_with_relation_columns(&["id", "name"]);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(
            lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
                CellValue::Integer(1),
                CellValue::Text("old".into()),
            ]]),
        );
        tab.grid.selected_column = 1;
    }

    app.update(Action::RelationEditCell);
    app.update(Action::RelationEditDeleteToStart);
    for character in "new".chars() {
        app.update(Action::RelationEditInsert(character));
    }
    assert!(app.update(Action::RelationEditConfirm).is_empty());

    let edit = relation_tab(&app).edit.as_ref().unwrap();
    assert_eq!(
        edit.mode,
        lazydb::model::relation_edit::RelationGridMode::Browse
    );
    assert_eq!(edit.rows[0].original[1], CellValue::Text("old".into()));
    assert_eq!(edit.rows[0].current[1], CellValue::Text("new".into()));
    assert!(matches!(
        edit.rows[0].state,
        lazydb::model::relation_edit::EditableRowState::Updated { .. }
    ));
    assert!(edit.pending_save.is_empty());
    assert_eq!(
        relation_tab(&app).transaction_state,
        lazydb::model::transaction::TransactionState::Idle
    );
}

#[test]
fn temporal_relation_editor_uses_segmented_input_and_preserves_fraction() {
    let mut app = app_with_relation_columns_with_types(&[("created_at", "timestamp")]);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(
            lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
                CellValue::Timestamp(
                    chrono::DateTime::parse_from_rfc3339("2026-08-28T10:20:31.1204+05:30").unwrap(),
                ),
            ]]),
        );
    }
    app.update(Action::RelationEditCell);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit.as_mut().unwrap().mode = lazydb::model::relation_edit::RelationGridMode::EditCell(
            Box::new(lazydb::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: lazydb::model::cell_editor::CellEditorBuffer {
                    presence: lazydb::model::cell_editor::CellEditorPresence::Value,
                    content: lazydb::model::cell_editor::CellEditorContent::Typed {
                        kind: lazydb::model::cell_editor::CellEditorKind::Timestamp,
                        draft: lazydb::model::cell_editor::TypedDraft::Temporal(
                            lazydb::model::cell_editor::TemporalDraft::from_timestamp(
                                chrono::DateTime::parse_from_rfc3339(
                                    "2026-08-28T10:20:31.1204+05:30",
                                )
                                .unwrap(),
                            ),
                        ),
                    },
                    ..lazydb::model::cell_editor::CellEditorBuffer::default()
                },
                error: None,
            }),
        );
    }
    assert!(matches!(
        relation_tab(&app).edit.as_ref().unwrap().mode,
        lazydb::model::relation_edit::RelationGridMode::EditCell(_)
    ));
    app.update(Action::RelationEditTemporalMonth(1));
    app.update(Action::RelationEditConfirm);
    assert!(matches!(
        relation_tab(&app).edit.as_ref().unwrap().rows[0].current[0],
        CellValue::Timestamp(value) if value.month() == 9
    ));
}

#[test]
fn boolean_relation_editor_actions_stay_local_until_confirmed() {
    let mut app = app_with_relation_columns_with_types(&[("active", "boolean")]);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(
            lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
                CellValue::Boolean(false),
            ]]),
        );
        tab.grid.selected_column = 0;
        tab.edit.as_mut().unwrap().mode = lazydb::model::relation_edit::RelationGridMode::EditCell(
            Box::new(lazydb::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: lazydb::model::cell_editor::CellEditorBuffer {
                    presence: lazydb::model::cell_editor::CellEditorPresence::Value,
                    content: lazydb::model::cell_editor::CellEditorContent::Typed {
                        kind: lazydb::model::cell_editor::CellEditorKind::Boolean,
                        draft: lazydb::model::cell_editor::TypedDraft::Boolean(
                            lazydb::model::text_input::TextInput::from("false"),
                        ),
                    },
                    ..lazydb::model::cell_editor::CellEditorBuffer::default()
                },
                error: None,
            }),
        );
    }

    app.update(Action::RelationEditBooleanSet(true));
    assert!(app.update(Action::RelationEditBooleanToggle).is_empty());
    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("boolean action should keep the relation editor open");
    };
    assert_eq!(state.input.value(), Some("false"));
    assert_eq!(edit.rows[0].current[0], CellValue::Boolean(false));
    assert!(app.update(Action::RelationEditBooleanSet(true)).is_empty());
    assert!(app.update(Action::RelationEditConfirm).is_empty());
    assert_eq!(
        relation_tab(&app).edit.as_ref().unwrap().rows[0].current[0],
        CellValue::Boolean(true)
    );
}

#[test]
fn inserted_boolean_cell_uses_the_typed_editor_before_a_value_is_supplied() {
    let mut app = app_with_relation_columns_with_types(&[("active", "boolean")]);
    configure_postgres_profile(&mut app);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(lazydb::model::relation_edit::RelationEditSession::from_rows(Vec::new()));
    }

    app.update(Action::RelationInsertRow);

    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("insert should open the cell editor");
    };
    assert!(matches!(
        state.input,
        lazydb::model::cell_editor::CellEditorBuffer {
            presence: lazydb::model::cell_editor::CellEditorPresence::Unprovided,
            content: lazydb::model::cell_editor::CellEditorContent::Typed {
                kind: lazydb::model::cell_editor::CellEditorKind::Boolean,
                draft: lazydb::model::cell_editor::TypedDraft::Boolean(_),
                ..
            },
            ..
        }
    ));
    assert!(edit.rows[0].supplied_columns.is_empty());
    assert_eq!(edit.rows[0].current[0], CellValue::Null);
}

#[test]
fn presence_actions_map_null_value_and_default_without_emitting_commands() {
    let mut app = app_with_relation_columns_with_types(&[("active", "boolean")]);
    configure_postgres_profile(&mut app);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(lazydb::model::relation_edit::RelationEditSession::from_rows(Vec::new()));
    }

    app.update(Action::RelationInsertRow);
    app.update(Action::RelationEditSetNull);
    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("presence action should keep editor open");
    };
    assert_eq!(
        state.input.presence,
        lazydb::model::cell_editor::CellEditorPresence::Null
    );
    app.update(Action::RelationEditUseValue);
    app.update(Action::RelationEditRestoreDefault);
    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("default action should keep editor open");
    };
    assert_eq!(
        state.input.presence,
        lazydb::model::cell_editor::CellEditorPresence::Unprovided
    );

    app.update(Action::RelationEditConfirm);
    assert!(matches!(
        relation_tab(&app).edit.as_ref().unwrap().mode,
        lazydb::model::relation_edit::RelationGridMode::Browse
    ));
    assert!(
        relation_tab(&app).edit.as_ref().unwrap().rows[0]
            .supplied_columns
            .is_empty()
    );
}

#[test]
fn default_action_is_gated_for_existing_rows() {
    let mut app = app_with_relation_columns_with_types(&[("active", "boolean")]);
    configure_postgres_profile(&mut app);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(
            lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
                CellValue::Boolean(true),
            ]]),
        );
    }
    app.update(Action::RelationEditCell);
    app.update(Action::RelationEditRestoreDefault);
    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("existing row should remain editable");
    };
    assert_eq!(
        state.input.presence,
        lazydb::model::cell_editor::CellEditorPresence::Value
    );
}

#[test]
fn inserted_timestamp_cell_uses_the_typed_editor_when_reopened() {
    let mut app =
        app_with_relation_columns_with_types(&[("active", "boolean"), ("created_at", "timestamp")]);
    configure_postgres_profile(&mut app);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(lazydb::model::relation_edit::RelationEditSession::from_rows(Vec::new()));
    }

    app.update(Action::RelationInsertRow);
    app.update(Action::RelationEditCancel);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.grid.selected_column = 1;
    }
    app.update(Action::RelationEditCell);

    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("reopening an inserted cell should open the editor");
    };
    assert!(matches!(
        state.input,
        lazydb::model::cell_editor::CellEditorBuffer {
            presence: lazydb::model::cell_editor::CellEditorPresence::Unprovided,
            content: lazydb::model::cell_editor::CellEditorContent::Typed {
                kind: lazydb::model::cell_editor::CellEditorKind::DateTime,
                draft: lazydb::model::cell_editor::TypedDraft::Temporal(_),
                ..
            },
            ..
        }
    ));
    assert!(edit.rows[0].supplied_columns.is_empty());
}

#[test]
fn existing_null_boolean_cell_uses_the_typed_editor() {
    let mut app = app_with_relation_columns_with_types(&[("active", "boolean")]);
    configure_postgres_profile(&mut app);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(
            lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
                CellValue::Null,
            ]]),
        );
    }

    app.update(Action::RelationEditCell);

    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("NULL cell should open the editor");
    };
    assert!(matches!(
        state.input,
        lazydb::model::cell_editor::CellEditorBuffer {
            presence: lazydb::model::cell_editor::CellEditorPresence::Null,
            content: lazydb::model::cell_editor::CellEditorContent::Typed {
                kind: lazydb::model::cell_editor::CellEditorKind::Boolean,
                draft: lazydb::model::cell_editor::TypedDraft::Boolean(_),
                ..
            },
            ..
        }
    ));
}

#[test]
fn invalid_relation_cell_input_stays_in_editor_state() {
    let mut app = app_with_relation_columns(&["id"]);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.edit = Some(
            lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
                CellValue::Integer(1),
            ]]),
        );
    }

    app.update(Action::RelationEditCell);
    app.update(Action::RelationEditDeleteToStart);
    for character in "not-an-integer".chars() {
        app.update(Action::RelationEditInsert(character));
    }
    assert!(app.update(Action::RelationEditConfirm).is_empty());

    let edit = relation_tab(&app).edit.as_ref().unwrap();
    let lazydb::model::relation_edit::RelationGridMode::EditCell(state) = &edit.mode else {
        panic!("invalid input should keep the cell editor open");
    };
    assert_eq!(state.input.value(), Some("not-an-integer"));
    assert_eq!(state.error.as_deref(), Some("invalid integer"));
    assert_eq!(relation_tab(&app).query.error, None);
    assert_eq!(edit.rows[0].current[0], CellValue::Integer(1));
}

#[test]
fn relation_edit_session_preserves_typed_cell_values() {
    let values = vec![
        CellValue::Null,
        CellValue::Boolean(true),
        CellValue::Integer(-1),
        CellValue::Unsigned(2),
        CellValue::Float(3.5),
        CellValue::Bytes(vec![1, 2]),
    ];
    let session =
        lazydb::model::relation_edit::RelationEditSession::from_rows(vec![values.clone()]);

    assert_eq!(session.rows[0].original, values);
    assert_eq!(session.rows[0].current, session.rows[0].original);
}

#[test]
fn relation_preview_columns_keep_native_type_metadata_separate_from_values() {
    let columns = [
        ColumnMeta {
            name: "id".into(),
            type_name: "INTEGER".into(),
        },
        ColumnMeta {
            name: "payload".into(),
            type_name: "JSON".into(),
        },
    ];
    let values = [CellValue::Integer(1), CellValue::Text("NULL".into())];

    assert_eq!(columns[0].type_name, "INTEGER");
    assert_eq!(columns[1].type_name, "JSON");
    assert_eq!(values[1], CellValue::Text("NULL".into()));
    assert_ne!(values[1], CellValue::Null);
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
                pagination: default_pagination(1),
                row_versions: None,
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
    assert_eq!(relation_query(&app).submitted, Default::default());
}

#[test]
fn rejected_relation_query_refresh_preserves_submitted_state() {
    let mut app = app_with_relation(RelationView::Data);
    if let WorkspaceTab::Relation(tab) = &mut app.tabs[1] {
        tab.query.submitted.where_clause = Some("old".into());
        tab.query.where_input.set("new");
    }
    let before = relation_query(&app).submitted.clone();

    assert!(app.update(Action::SubmitDataQuery).is_empty());
    assert_eq!(relation_query(&app).submitted, before);
}

#[test]
fn data_query_undo_and_redo_are_scoped_to_the_focused_input() {
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::Where,
    ));
    app.update(Action::DataQueryInsert('a'));
    app.update(Action::DataQueryInsert('b'));
    app.update(Action::DataQueryUndo);
    assert_eq!(relation_query(&app).where_input.value(), "");
    app.update(Action::DataQueryRedo);
    assert_eq!(relation_query(&app).where_input.value(), "ab");

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::OrderBy,
    ));
    app.update(Action::DataQueryInsert('c'));
    app.update(Action::DataQueryUndo);
    assert_eq!(relation_query(&app).where_input.value(), "ab");
    assert_eq!(relation_query(&app).order_by_input.value(), "");
}

#[test]
fn relation_query_suggests_only_current_relation_columns() {
    let profile = Uuid::new_v4();
    let current_relation = CatalogId::new(profile, CatalogKind::Table, ["db", "main", "users"]);
    let other_relation = CatalogId::new(profile, CatalogKind::Table, ["db", "main", "roles"]);
    let descriptor = RelationDescriptor {
        key: RelationKey {
            profile_id: profile,
            object_id: current_relation.clone(),
        },
        qualified_name: QualifiedName {
            database: Some("db".into()),
            schema: Some("main".into()),
            object: "users".into(),
        },
        kind: CatalogKind::Table,
        title: "users".into(),
    };
    let columns = [
        (current_relation, "user_id", "bigint"),
        (other_relation, "role_id", "bigint"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (relation, name, native_type))| {
        CatalogEntry::relation_child(
            CatalogId::new(
                profile,
                CatalogKind::Column,
                ["db", "main", relation.native_path.last().unwrap(), name],
            ),
            relation,
            QualifiedName {
                database: Some("db".into()),
                schema: Some("main".into()),
                object: name.into(),
            },
            "column",
            OptionalMetadata::Unsupported,
            CatalogMetadata::Column(ColumnMetadata::new(index as u32 + 1, native_type, false)),
        )
        .unwrap()
    })
    .collect::<Vec<_>>();
    let mut app = lazydb::app::App::new(Vec::new());
    app.explorer.completion_index.append(&columns);
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::with_descriptor(
            descriptor,
            RelationView::Data,
        )));
    app.active_tab = 1;

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::Where,
    ));
    for character in "userid".chars() {
        app.update(Action::DataQueryInsert(character));
    }

    let completion = relation_query(&app).completion.as_ref().unwrap();
    assert_eq!(completion.candidates.len(), 1);
    assert_eq!(completion.candidates[0].name, "user_id");
    assert_eq!(
        completion.candidates[0].type_name.as_deref(),
        Some("bigint")
    );
    assert_eq!(completion.replace, lazydb::sql::TextRange::new(0, 6));
    app.update(Action::DataQueryCompletionAccept);
    assert_eq!(relation_query(&app).where_input.value(), "\"user_id\" ");
    assert!(relation_query(&app).completion.is_none());

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::OrderBy,
    ));
    for character in "userid".chars() {
        app.update(Action::DataQueryInsert(character));
    }
    assert_eq!(
        relation_query(&app).completion.as_ref().unwrap().candidates[0].name,
        "user_id"
    );
    app.update(Action::DataQueryCompletionAccept);
    assert_eq!(relation_query(&app).order_by_input.value(), "\"user_id\" ");
}

#[test]
fn sql_result_query_suggests_output_columns_in_both_inputs() {
    let mut app = lazydb::app::App::new(Vec::new());
    let mut tab = ConsoleTab::new("search");
    tab.result_view = ResultView::Data;
    tab.query.capability = DataQueryCapability::Sql;
    tab.outcome = Some(QueryOutcome {
        result_sets: vec![ResultSet {
            columns: vec![
                ColumnMeta {
                    name: "uid".into(),
                    type_name: "bigint".into(),
                },
                ColumnMeta {
                    name: "display_name".into(),
                    type_name: "text".into(),
                },
            ],
            rows: Vec::new(),
            affected_rows: 0,
        }],
        stats: QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 0),
    });
    app.tabs.push(WorkspaceTab::Sql(tab));
    app.active_tab = 1;

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::Where,
    ));
    for character in "display".chars() {
        app.update(Action::DataQueryInsert(character));
    }
    let WorkspaceTab::Sql(tab) = &app.tabs[1] else {
        panic!("expected SQL tab")
    };
    assert_eq!(
        tab.query.completion.as_ref().unwrap().candidates[0].name,
        "display_name"
    );
    assert_eq!(
        tab.query.completion.as_ref().unwrap().candidates[0]
            .type_name
            .as_deref(),
        Some("text")
    );

    app.update(Action::DataQueryCompletionAccept);
    let WorkspaceTab::Sql(tab) = &app.tabs[1] else {
        panic!("expected SQL tab")
    };
    assert_eq!(tab.query.where_input.value(), "\"display_name\" ");

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::OrderBy,
    ));
    for character in "uid".chars() {
        app.update(Action::DataQueryInsert(character));
    }
    let WorkspaceTab::Sql(tab) = &app.tabs[1] else {
        panic!("expected SQL tab")
    };
    assert_eq!(
        tab.query.completion.as_ref().unwrap().candidates[0].name,
        "uid"
    );
}

#[test]
fn relation_query_falls_back_to_preview_columns() {
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
                sql: "select * from users".into(),
                result: QueryOutcome {
                    result_sets: vec![ResultSet {
                        columns: vec![ColumnMeta {
                            name: "user_id".into(),
                            type_name: "bigint".into(),
                        }],
                        rows: Vec::new(),
                        affected_rows: 0,
                    }],
                    stats: QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 0),
                },
                pagination: default_pagination(0),
                row_versions: None,
            },
            connection,
            lazydb::profile::CatalogScope::for_profile(
                lazydb::profile::DatabaseKind::Sqlite,
                "db",
                None,
            ),
        ));
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = 1;

    app.update(Action::FocusDataQueryInput(
        lazydb::model::data_query::DataQueryInput::Where,
    ));
    for character in "userid".chars() {
        app.update(Action::DataQueryInsert(character));
    }

    let candidate = &relation_query(&app).completion.as_ref().unwrap().candidates[0];
    assert_eq!(candidate.name, "user_id");
    assert_eq!(candidate.type_name.as_deref(), Some("bigint"));
}

#[test]
fn relation_column_sort_cycles_and_submits() {
    let mut app = app_with_relation_columns(&["id", "name"]);

    app.update(Action::CycleDataColumnSort(0));
    assert_eq!(relation_query(&app).order_by_input.value(), "\"id\" DESC");
    app.update(Action::CycleDataColumnSort(0));
    assert_eq!(relation_query(&app).order_by_input.value(), "\"id\" ASC");
    app.update(Action::CycleDataColumnSort(0));
    assert_eq!(relation_query(&app).order_by_input.value(), "");
}

#[test]
fn relation_column_sort_appends_and_preserves_where() {
    let mut app = app_with_relation_columns(&["id", "name"]);
    relation_query_mut(&mut app).where_input.set("\"id\" > 10");

    app.update(Action::CycleDataColumnSort(0));
    app.update(Action::CycleDataColumnSort(1));

    assert_eq!(relation_query(&app).where_input.value(), "\"id\" > 10");
    assert_eq!(
        relation_query(&app).order_by_input.value(),
        "\"id\" DESC, \"name\" DESC"
    );
}

#[test]
fn relation_column_sort_invalid_draft_is_preserved() {
    let mut app = app_with_relation_columns(&["id"]);
    relation_query_mut(&mut app).order_by_input.set("id DESC,");

    assert!(app.update(Action::CycleDataColumnSort(0)).is_empty());
    assert_eq!(relation_query(&app).order_by_input.value(), "id DESC,");
}

#[test]
fn relation_column_sort_wrong_tab_view_and_bounds_are_no_ops() {
    let mut app = app_with_relation_columns(&["id"]);
    let original = relation_query(&app).order_by_input.value().to_owned();

    app.update(Action::SetRelationView(RelationView::Ddl));
    assert!(app.update(Action::CycleDataColumnSort(0)).is_empty());
    assert_eq!(relation_query(&app).order_by_input.value(), original);

    app.update(Action::SetRelationView(RelationView::Data));
    assert!(app.update(Action::CycleDataColumnSort(1)).is_empty());
    assert_eq!(relation_query(&app).order_by_input.value(), original);

    app.tabs.push(WorkspaceTab::Sql(ConsoleTab::new("sql")));
    app.active_tab = 2;
    assert!(app.update(Action::CycleDataColumnSort(0)).is_empty());
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
fn relation_tab_normalizes_direct_editor_focus_to_results() {
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = lazydb::model::workspace::Focus::Explorer;

    app.update(Action::Focus(lazydb::model::workspace::Focus::Editor));

    assert_eq!(app.focus, lazydb::model::workspace::Focus::Results);
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
                pagination: default_pagination(0),
                row_versions: None,
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

fn configure_postgres_profile(app: &mut lazydb::app::App) {
    let mut profile =
        lazydb::profile::import_connection_url("postgres://localhost/app", Some("test"))
            .unwrap()
            .profile;
    profile.id = Uuid::nil();
    app.profiles.push(profile);
    app.connection.profile_id = Some(Uuid::nil());
}

fn relation_view(app: &lazydb::app::App) -> RelationView {
    match &app.tabs[app.active_tab] {
        WorkspaceTab::Relation(tab) => tab.view,
        WorkspaceTab::Sql(_) => panic!("expected relation tab"),
        WorkspaceTab::Dashboard(_) => panic!("expected relation tab"),
    }
}

fn relation_query(app: &lazydb::app::App) -> &lazydb::model::data_query::DataQueryState {
    match &app.tabs[app.active_tab] {
        WorkspaceTab::Relation(tab) => &tab.query,
        WorkspaceTab::Sql(_) => panic!("expected relation tab"),
        WorkspaceTab::Dashboard(_) => panic!("expected relation tab"),
    }
}

fn relation_query_mut(
    app: &mut lazydb::app::App,
) -> &mut lazydb::model::data_query::DataQueryState {
    match &mut app.tabs[app.active_tab] {
        WorkspaceTab::Relation(tab) => &mut tab.query,
        WorkspaceTab::Sql(_) => panic!("expected relation tab"),
        WorkspaceTab::Dashboard(_) => panic!("expected relation tab"),
    }
}

fn app_with_relation_columns(columns: &[&str]) -> lazydb::app::App {
    app_with_relation_columns_with_types(
        &columns
            .iter()
            .map(|name| (*name, "text"))
            .collect::<Vec<_>>(),
    )
}

fn app_with_relation_columns_with_types(columns: &[(&str, &str)]) -> lazydb::app::App {
    let mut app = app_with_relation(RelationView::Data);
    let mut tab = match app.tabs.pop().unwrap() {
        WorkspaceTab::Relation(tab) => tab,
        _ => unreachable!(),
    };
    tab.data =
        lazydb::model::relation::RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot::new(
            lazydb::db::RelationPreview {
                sql: "select".into(),
                result: QueryOutcome {
                    result_sets: vec![ResultSet {
                        columns: columns
                            .iter()
                            .map(|(name, type_name)| ColumnMeta {
                                name: (*name).into(),
                                type_name: (*type_name).into(),
                            })
                            .collect(),
                        rows: Vec::new(),
                        affected_rows: 0,
                    }],
                    stats: QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 0),
                },
                pagination: default_pagination(0),
                row_versions: None,
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
    app
}

fn app_with_relation(view: RelationView) -> lazydb::app::App {
    let mut app = lazydb::app::App::new(Vec::new());
    let mut tab = RelationTab::new("users");
    tab.view = view;
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = 1;
    app
}

fn relation_tab(app: &lazydb::app::App) -> &RelationTab {
    relation_tab_at(app, app.active_tab)
}

fn relation_tab_at(app: &lazydb::app::App, index: usize) -> &RelationTab {
    match &app.tabs[index] {
        WorkspaceTab::Relation(tab) => tab,
        WorkspaceTab::Sql(_) => panic!("expected relation tab"),
        WorkspaceTab::Dashboard(_) => panic!("expected relation tab"),
    }
}

fn descriptor(id: CatalogId, title: &str) -> RelationDescriptor {
    RelationDescriptor {
        key: RelationKey {
            profile_id: id.profile_id(),
            object_id: id,
        },
        qualified_name: QualifiedName {
            database: Some("db".into()),
            schema: Some("public".into()),
            object: title.into(),
        },
        kind: CatalogKind::Table,
        title: title.into(),
    }
}

fn ddl_offsets(app: &lazydb::app::App) -> (usize, usize) {
    let viewport = &relation_tab(app).ddl_viewport;
    (viewport.row_offset, viewport.column_offset)
}

fn default_pagination(fetched_rows: usize) -> lazydb::model::pagination::ResultPagination {
    lazydb::model::pagination::ResultPagination::from_page(
        lazydb::model::pagination::PageRequest::first(
            lazydb::model::pagination::PageSize::default(),
        ),
        fetched_rows,
    )
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
