use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCapabilities, CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry,
            CatalogGroupSummary, CatalogId, CatalogKind, CatalogMetadata, CatalogPage,
            CatalogRequest, CatalogRequestKey, CatalogTarget, ColumnMetadata,
            ColumnMetadataCapabilities, ConstraintMembership, ConstraintMetadata, IndexMetadata,
            MAX_CATALOG_PAGE_SIZE, NamespaceModel, ObjectGroup, OptionalMetadata, QualifiedName,
            finalize_keyset_page,
        },
    },
    identity::ConnectionIdentity,
    model::workspace::ConnectionIdentity as WorkspaceConnectionIdentity,
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
};
use uuid::Uuid;

#[test]
fn object_identity_is_stable_across_reconnect_generations() {
    let profile_id = Uuid::new_v4();
    let first = ConnectionIdentity {
        profile_id,
        generation: 1,
    };
    let reconnected = ConnectionIdentity {
        profile_id,
        generation: 2,
    };

    let before = CatalogId::new(
        first.profile_id,
        CatalogKind::Table,
        ["app", "public", "users"],
    );
    let after = CatalogId::new(
        reconnected.profile_id,
        CatalogKind::Table,
        ["app", "public", "users"],
    );

    assert_ne!(first, reconnected);
    assert_eq!(before, after);
    let legacy_import: WorkspaceConnectionIdentity = first;
    assert_eq!(legacy_import, first);
}

#[test]
fn identical_native_paths_are_profile_scoped() {
    let path = ["app", "public", "users"];
    let first = CatalogId::new(Uuid::new_v4(), CatalogKind::Table, path);
    let second = CatalogId::new(Uuid::new_v4(), CatalogKind::Table, path);

    assert_ne!(first, second);
}

#[test]
fn materialized_views_are_relations() {
    assert!(CatalogKind::Table.is_relation());
    assert!(CatalogKind::View.is_relation());
    assert!(CatalogKind::MaterializedView.is_relation());
    assert!(!CatalogKind::Schema.is_relation());
    assert!(!CatalogKind::Sequence.is_relation());
}

#[test]
fn presentation_groups_are_not_catalog_kinds() {
    assert_eq!(
        serde_json::to_value(ObjectGroup::Tables).unwrap(),
        serde_json::json!("tables")
    );
    assert!(serde_json::from_value::<CatalogKind>(serde_json::json!("tables")).is_err());
    assert_eq!(
        serde_json::to_value(CatalogKind::MaterializedView).unwrap(),
        serde_json::json!("materialized_view")
    );
}

#[test]
fn optional_metadata_distinguishes_unsupported_from_supported_absence() {
    let unsupported: OptionalMetadata<String> = OptionalMetadata::Unsupported;
    let absent = OptionalMetadata::Supported(None);
    let present = OptionalMetadata::Supported(Some("owner comment".to_owned()));

    assert_ne!(unsupported, absent);
    assert_ne!(absent, present);
    assert!(!unsupported.is_supported());
    assert!(absent.is_supported());
}

#[test]
fn catalog_counts_distinguish_exact_lower_bound_and_unknown() {
    assert_ne!(CatalogCount::Exact(12), CatalogCount::AtLeast(12));
    assert_ne!(CatalogCount::AtLeast(12), CatalogCount::Unknown);
    assert_ne!(CatalogCount::Exact(12), CatalogCount::Unknown);
}

#[test]
fn catalog_targets_validate_their_hierarchy_level() {
    let profile_id = Uuid::new_v4();
    let database = id(profile_id, CatalogKind::Database, "database-oid");
    let schema = id(profile_id, CatalogKind::Schema, "schema-oid");
    let table = id(profile_id, CatalogKind::Table, "relation-oid");
    let column = id(profile_id, CatalogKind::Column, "column-attnum");

    assert_eq!(CatalogTarget::Databases, CatalogTarget::Databases);
    assert!(CatalogTarget::schemas(database.clone()).is_ok());
    assert!(CatalogTarget::groups(schema.clone()).is_ok());
    assert!(CatalogTarget::objects(schema.clone(), ObjectGroup::Tables).is_ok());
    assert!(CatalogTarget::relation_children(table.clone()).is_ok());

    assert!(CatalogTarget::schemas(schema.clone()).is_err());
    assert!(CatalogTarget::groups(database.clone()).is_err());
    assert!(CatalogTarget::objects(table.clone(), ObjectGroup::Tables).is_err());
    assert!(CatalogTarget::relation_children(column).is_err());
}

#[test]
fn relation_ancestry_and_qualification_do_not_depend_on_path_offsets() {
    let profile_id = Uuid::new_v4();
    let database = CatalogEntry::database(
        id(profile_id, CatalogKind::Database, "opaque-db-id"),
        QualifiedName {
            database: Some("app".to_owned()),
            schema: None,
            object: "app".to_owned(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        id(profile_id, CatalogKind::Schema, "opaque-schema-id"),
        database.id.clone(),
        QualifiedName {
            database: Some("app".to_owned()),
            schema: Some("audit".to_owned()),
            object: "audit".to_owned(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let relation_name = QualifiedName {
        database: Some("app".to_owned()),
        schema: Some("audit".to_owned()),
        object: "event_log".to_owned(),
    };
    let relation = CatalogEntry::relation(
        id(
            profile_id,
            CatalogKind::MaterializedView,
            "opaque-relation-id",
        ),
        schema.id.clone(),
        relation_name.clone(),
        "materialized view",
        OptionalMetadata::Supported(Some("audit projection".to_owned())),
        true,
    )
    .unwrap();
    let column = CatalogEntry::relation_child(
        id(profile_id, CatalogKind::Column, "opaque-column-id"),
        relation.id.clone(),
        QualifiedName {
            database: Some("app".to_owned()),
            schema: Some("audit".to_owned()),
            object: "event_id".to_owned(),
        },
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
    )
    .unwrap();

    assert_eq!(relation.qualified_name, relation_name);
    assert_eq!(relation.owning_relation_id(), Some(&relation.id));
    assert_eq!(column.owning_relation_id(), Some(&relation.id));
    assert_eq!(column.parent_id.as_ref(), Some(&relation.id));
    assert_eq!(column.id.native_path, vec!["opaque-column-id"]);
}

#[test]
fn entry_constructors_reject_wrong_profiles_and_parent_kinds() {
    let profile_id = Uuid::new_v4();
    let other_profile_id = Uuid::new_v4();
    let database = id(profile_id, CatalogKind::Database, "db");
    let schema = id(profile_id, CatalogKind::Schema, "schema");
    let other_schema = id(other_profile_id, CatalogKind::Schema, "schema");
    let relation_name = QualifiedName {
        database: Some("app".to_owned()),
        schema: Some("public".to_owned()),
        object: "users".to_owned(),
    };

    assert!(
        CatalogEntry::schema(
            id(other_profile_id, CatalogKind::Schema, "schema"),
            database.clone(),
            QualifiedName {
                database: Some("app".to_owned()),
                schema: Some("public".to_owned()),
                object: "public".to_owned(),
            },
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .is_err()
    );
    assert!(
        CatalogEntry::relation(
            id(profile_id, CatalogKind::Table, "users"),
            database,
            relation_name.clone(),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .is_err()
    );
    assert!(
        CatalogEntry::relation(
            id(profile_id, CatalogKind::Table, "users"),
            other_schema,
            relation_name,
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .is_err()
    );
    assert!(CatalogTarget::objects(schema, ObjectGroup::Views).is_ok());
}

#[test]
fn composite_index_and_constraint_parts_remain_grouped() {
    let index = IndexMetadata {
        columns: vec!["tenant_id".to_owned(), "email".to_owned()],
        unique: true,
    };
    let foreign_key = ConstraintMetadata::ForeignKey {
        columns: vec!["tenant_id".to_owned(), "owner_id".to_owned()],
        referenced_relation: QualifiedName {
            database: Some("app".to_owned()),
            schema: Some("public".to_owned()),
            object: "owners".to_owned(),
        },
        referenced_columns: vec!["tenant_id".to_owned(), "id".to_owned()],
    };

    assert_eq!(index.columns.len(), 2);
    let ConstraintMetadata::ForeignKey {
        columns,
        referenced_columns,
        ..
    } = foreign_key
    else {
        panic!("expected grouped foreign-key metadata")
    };
    assert_eq!(columns.len(), 2);
    assert_eq!(referenced_columns.len(), 2);
}

#[test]
fn catalog_transport_contracts_retain_request_identity() {
    let profile_id = Uuid::new_v4();
    let connection = ConnectionIdentity {
        profile_id,
        generation: 7,
    };
    let target = CatalogTarget::groups(CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        ["app", "public"],
    ))
    .unwrap();
    let cursor = CatalogCursor::from_keyset("next-schema", "schema-42").unwrap();
    let key = CatalogRequestKey {
        connection,
        catalog_epoch: 3,
        request_id: 11,
        target,
        cursor: None,
    };
    let request = CatalogRequest {
        key: key.clone(),
        scope: CatalogScope::for_profile(DatabaseKind::Postgres, "app", Some("public")),
        page_size: 100,
    };
    let page = CatalogPage {
        key: key.clone(),
        entries: Vec::new(),
        group_summaries: Vec::new(),
        total_count: CatalogCount::AtLeast(100),
        next_cursor: Some(cursor.clone()),
        completeness: CatalogCompleteness::Partial,
    };
    let capabilities = CatalogCapabilities {
        namespace_model: NamespaceModel::DatabaseAndSchema,
        top_level_groups: vec![ObjectGroup::Tables, ObjectGroup::Views],
        column_metadata: ColumnMetadataCapabilities::default(),
        supports_lazy_children: true,
    };

    assert_eq!(request.key, key);
    assert_eq!(page.key, request.key);
    assert_eq!(page.next_cursor.as_ref(), Some(&cursor));
    assert_eq!(page.completeness, CatalogCompleteness::Partial);
    assert_eq!(
        capabilities.namespace_model,
        NamespaceModel::DatabaseAndSchema
    );
}

#[test]
fn page_request_validation_enforces_size_profile_and_target_shape() {
    let profile_id = Uuid::new_v4();
    let mut request = page_request(profile_id, CatalogTarget::Databases, None, 1);

    for page_size in 1..=MAX_CATALOG_PAGE_SIZE {
        request.page_size = page_size;
        assert!(
            request.validate().is_ok(),
            "page size {page_size} should be valid"
        );
    }

    request.page_size = 0;
    assert!(request.validate().is_err());
    request.page_size = MAX_CATALOG_PAGE_SIZE + 1;
    assert!(request.validate().is_err());

    request.page_size = 1;
    request.key.target = CatalogTarget::Schemas {
        database: id(profile_id, CatalogKind::Schema, "not-a-database"),
    };
    assert!(request.validate().is_err());

    request.key.target = CatalogTarget::Schemas {
        database: id(Uuid::new_v4(), CatalogKind::Database, "other-profile"),
    };
    assert!(request.validate().is_err());

    request.key.target = CatalogTarget::Groups {
        schema: CatalogId::new(
            profile_id,
            CatalogKind::Schema,
            std::iter::empty::<String>(),
        ),
    };
    assert!(request.validate().is_err());
}

#[test]
fn page_constructor_echoes_the_complete_key_and_derives_completeness() {
    let profile_id = Uuid::new_v4();
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let request = page_request(
        profile_id,
        CatalogTarget::objects(schema.clone(), ObjectGroup::Tables).unwrap(),
        Some(CatalogCursor::from_keyset("request", "request-1").unwrap()),
        1,
    );
    let entry = relation_entry(profile_id, schema, "app", "public", "users");
    let next_cursor = CatalogCursor::from_keyset("subsequent", "native-2").unwrap();

    let first = CatalogPage::new(
        &request,
        vec![entry.clone()],
        CatalogCount::AtLeast(1),
        Some(next_cursor.clone()),
    )
    .unwrap();
    let final_page = CatalogPage::new(&request, vec![entry], CatalogCount::Exact(1), None).unwrap();

    assert_eq!(first.key, request.key);
    assert_eq!(first.key.connection, request.key.connection);
    assert_eq!(first.key.catalog_epoch, request.key.catalog_epoch);
    assert_eq!(first.key.request_id, request.key.request_id);
    assert_eq!(first.key.target, request.key.target);
    assert_eq!(first.key.cursor, request.key.cursor);
    assert_eq!(first.next_cursor, Some(next_cursor));
    assert_eq!(first.completeness, CatalogCompleteness::Partial);
    assert_eq!(final_page.completeness, CatalogCompleteness::Complete);
    assert!(first.validate_for(&request).is_ok());
    assert!(final_page.validate_for(&request).is_ok());
}

#[test]
fn page_validation_rejects_wrong_profile_and_case_sensitive_out_of_scope_entries() {
    let profile_id = Uuid::new_v4();
    let request = page_request(profile_id, CatalogTarget::Databases, None, 10);
    let wrong_profile = database_entry(Uuid::new_v4(), "app");
    let wrong_database_case = database_entry(profile_id, "App");

    assert!(
        CatalogPage::new(&request, vec![wrong_profile], CatalogCount::Exact(1), None,).is_err()
    );
    assert!(
        CatalogPage::new(
            &request,
            vec![wrong_database_case],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );

    let database = CatalogId::new(profile_id, CatalogKind::Database, ["app"]);
    let schema_request = page_request(
        profile_id,
        CatalogTarget::schemas(database.clone()).unwrap(),
        None,
        10,
    );
    let exact_schema = schema_entry(profile_id, database.clone(), "app", "public");
    let wrong_schema_case = schema_entry(profile_id, database, "app", "Public");

    assert!(
        CatalogPage::new(
            &schema_request,
            vec![exact_schema],
            CatalogCount::Exact(1),
            None,
        )
        .is_ok()
    );
    assert!(
        CatalogPage::new(
            &schema_request,
            vec![wrong_schema_case],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );

    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let object_request = page_request(
        profile_id,
        CatalogTarget::objects(schema.clone(), ObjectGroup::Tables).unwrap(),
        None,
        10,
    );
    let mut missing_native_name = relation_entry(profile_id, schema, "app", "public", "users");
    missing_native_name.qualified_name.object.clear();
    assert!(
        CatalogPage::new(
            &object_request,
            vec![missing_native_name],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );
}

#[test]
fn page_validation_rejects_wrong_parent_object_group_and_relation_parent() {
    let profile_id = Uuid::new_v4();
    let database = CatalogId::new(profile_id, CatalogKind::Database, ["app"]);
    let other_database = CatalogId::new(profile_id, CatalogKind::Database, ["other"]);
    let schema_request = page_request(
        profile_id,
        CatalogTarget::schemas(database.clone()).unwrap(),
        None,
        10,
    );
    let wrong_parent_schema = schema_entry(profile_id, other_database, "app", "public");
    assert!(
        CatalogPage::new(
            &schema_request,
            vec![wrong_parent_schema],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );

    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let views_request = page_request(
        profile_id,
        CatalogTarget::objects(schema.clone(), ObjectGroup::Views).unwrap(),
        None,
        10,
    );
    assert!(
        CatalogPage::new(
            &views_request,
            vec![relation_entry(
                profile_id,
                schema.clone(),
                "app",
                "public",
                "users",
            )],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );

    let tables_request = page_request(
        profile_id,
        CatalogTarget::objects(schema.clone(), ObjectGroup::Tables).unwrap(),
        None,
        10,
    );
    let mut malformed_relation =
        relation_entry(profile_id, schema.clone(), "app", "public", "users");
    malformed_relation.relation_id = Some(CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["app", "public", "accounts"],
    ));
    assert!(
        CatalogPage::new(
            &tables_request,
            vec![malformed_relation],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );

    let requested_relation =
        CatalogId::new(profile_id, CatalogKind::Table, ["app", "public", "users"]);
    let other_relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["app", "public", "accounts"],
    );
    let children_request = page_request(
        profile_id,
        CatalogTarget::relation_children(requested_relation).unwrap(),
        None,
        10,
    );
    let column = CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            ["app", "public", "accounts", "id"],
        ),
        other_relation,
        QualifiedName {
            database: Some("app".to_owned()),
            schema: Some("public".to_owned()),
            object: "id".to_owned(),
        },
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
    )
    .unwrap();
    assert!(
        CatalogPage::new(
            &children_request,
            vec![column],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );
}

#[test]
fn page_validation_rejects_mismatched_target_cursor_and_request_id() {
    let profile_id = Uuid::new_v4();
    let request = page_request(profile_id, CatalogTarget::Databases, None, 10);
    let page = CatalogPage::new(
        &request,
        vec![database_entry(profile_id, "app")],
        CatalogCount::Exact(1),
        None,
    )
    .unwrap();

    let mut wrong_target = request.clone();
    wrong_target.key.target =
        CatalogTarget::schemas(CatalogId::new(profile_id, CatalogKind::Database, ["app"])).unwrap();
    assert!(page.validate_for(&wrong_target).is_err());

    let mut wrong_cursor = request.clone();
    wrong_cursor.key.cursor = Some(CatalogCursor::from_keyset("different", "native-9").unwrap());
    assert!(page.validate_for(&wrong_cursor).is_err());

    let mut wrong_request_id = request;
    wrong_request_id.key.request_id += 1;
    assert!(page.validate_for(&wrong_request_id).is_err());
}

#[test]
fn page_validation_rejects_inconsistent_completeness() {
    let profile_id = Uuid::new_v4();
    let request = page_request(profile_id, CatalogTarget::Databases, None, 10);
    let mut page = CatalogPage::new(
        &request,
        vec![database_entry(profile_id, "app")],
        CatalogCount::Exact(1),
        None,
    )
    .unwrap();

    page.completeness = CatalogCompleteness::Partial;

    assert!(page.validate_for(&request).is_err());
}

#[test]
fn page_keyset_finalization_retains_limit_and_uses_last_retained_stable_key() {
    let mut rows = vec![
        ("Alpha".to_owned(), "native-3".to_owned()),
        ("beta".to_owned(), "native-1".to_owned()),
        ("gamma".to_owned(), "native-2".to_owned()),
    ];

    let next_cursor = finalize_keyset_page(
        &mut rows,
        2,
        |row| row.0.to_lowercase(),
        |row| row.1.clone(),
    )
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], ("beta".to_owned(), "native-1".to_owned()));
    assert_eq!(
        next_cursor,
        Some(CatalogCursor::from_keyset("beta", "native-1").unwrap())
    );

    let mut final_rows = rows.clone();
    assert_eq!(
        finalize_keyset_page(
            &mut final_rows,
            2,
            |row| row.0.to_lowercase(),
            |row| row.1.clone(),
        )
        .unwrap(),
        None
    );
    assert_eq!(final_rows, rows);
}

#[test]
fn page_group_summaries_support_non_empty_and_empty_pages() {
    let profile_id = Uuid::new_v4();
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let request = page_request(profile_id, CatalogTarget::groups(schema).unwrap(), None, 2);
    let summaries = vec![
        CatalogGroupSummary {
            group: ObjectGroup::Tables,
            object_count: CatalogCount::Exact(42),
        },
        CatalogGroupSummary {
            group: ObjectGroup::Views,
            object_count: CatalogCount::Unknown,
        },
    ];

    let page =
        CatalogPage::groups(&request, summaries.clone(), CatalogCount::Exact(2), None).unwrap();
    let empty = CatalogPage::groups(&request, Vec::new(), CatalogCount::Exact(0), None).unwrap();

    assert!(page.entries.is_empty());
    assert_eq!(page.group_summaries, summaries);
    assert_eq!(page.completeness, CatalogCompleteness::Complete);
    assert!(empty.entries.is_empty());
    assert!(empty.group_summaries.is_empty());
    assert!(empty.validate_for(&request).is_ok());
}

#[test]
fn page_group_validation_rejects_duplicates_mixed_payloads_and_wrong_targets() {
    let profile_id = Uuid::new_v4();
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let groups_request = page_request(
        profile_id,
        CatalogTarget::groups(schema.clone()).unwrap(),
        None,
        2,
    );
    let duplicate = CatalogGroupSummary {
        group: ObjectGroup::Tables,
        object_count: CatalogCount::Exact(1),
    };
    assert!(
        CatalogPage::groups(
            &groups_request,
            vec![duplicate.clone(), duplicate],
            CatalogCount::Exact(2),
            None,
        )
        .is_err()
    );

    let mut mixed = CatalogPage::groups(
        &groups_request,
        vec![CatalogGroupSummary {
            group: ObjectGroup::Tables,
            object_count: CatalogCount::Exact(1),
        }],
        CatalogCount::Exact(1),
        None,
    )
    .unwrap();
    mixed.entries.push(relation_entry(
        profile_id,
        schema.clone(),
        "app",
        "public",
        "users",
    ));
    assert!(mixed.validate_for(&groups_request).is_err());

    let objects_request = page_request(
        profile_id,
        CatalogTarget::objects(schema.clone(), ObjectGroup::Tables).unwrap(),
        None,
        2,
    );
    let mut native = CatalogPage::new(
        &objects_request,
        vec![relation_entry(profile_id, schema, "app", "public", "users")],
        CatalogCount::Exact(1),
        None,
    )
    .unwrap();
    native.group_summaries.push(CatalogGroupSummary {
        group: ObjectGroup::Tables,
        object_count: CatalogCount::Exact(1),
    });
    assert!(native.validate_for(&objects_request).is_err());
    assert!(
        CatalogPage::groups(&objects_request, Vec::new(), CatalogCount::Exact(0), None,).is_err()
    );

    let one_group_request = page_request(
        profile_id,
        CatalogTarget::groups(CatalogId::new(
            profile_id,
            CatalogKind::Schema,
            ["app", "public"],
        ))
        .unwrap(),
        None,
        1,
    );
    assert!(
        CatalogPage::groups(
            &one_group_request,
            vec![
                CatalogGroupSummary {
                    group: ObjectGroup::Tables,
                    object_count: CatalogCount::Unknown,
                },
                CatalogGroupSummary {
                    group: ObjectGroup::Views,
                    object_count: CatalogCount::Unknown,
                },
            ],
            CatalogCount::Exact(2),
            None,
        )
        .is_err()
    );
}

#[test]
fn page_request_validation_rejects_out_of_scope_and_noncanonical_target_paths() {
    let profile_id = Uuid::new_v4();
    let mut request = page_request(profile_id, CatalogTarget::Databases, None, 10);

    request.key.target =
        CatalogTarget::schemas(CatalogId::new(profile_id, CatalogKind::Database, ["other"]))
            .unwrap();
    assert!(request.validate().is_err());

    request.key.target = CatalogTarget::groups(CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        ["app", "private"],
    ))
    .unwrap();
    assert!(request.validate().is_err());
    request.key.target = CatalogTarget::objects(
        CatalogId::new(profile_id, CatalogKind::Schema, ["app", "Public"]),
        ObjectGroup::Tables,
    )
    .unwrap();
    assert!(request.validate().is_err());

    request.key.target = CatalogTarget::schemas(CatalogId::new(
        profile_id,
        CatalogKind::Database,
        ["app", "extra"],
    ))
    .unwrap();
    assert!(request.validate().is_err());
    for path in [vec!["app"], vec!["app", "public", "extra"]] {
        request.key.target =
            CatalogTarget::groups(CatalogId::new(profile_id, CatalogKind::Schema, path)).unwrap();
        assert!(request.validate().is_err());
    }
    for path in [
        vec!["app", "public"],
        vec!["app", "", "users"],
        vec!["app", "public", ""],
    ] {
        request.key.target =
            CatalogTarget::relation_children(CatalogId::new(profile_id, CatalogKind::Table, path))
                .unwrap();
        assert!(request.validate().is_err());
    }

    request.key.target = CatalogTarget::relation_children(CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["app", "public", "users", "stable-native-suffix"],
    ))
    .unwrap();
    assert!(request.validate().is_ok());
}

#[test]
fn page_validation_rejects_forged_qualified_names_and_native_namespaces() {
    let profile_id = Uuid::new_v4();
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let request = page_request(
        profile_id,
        CatalogTarget::objects(schema.clone(), ObjectGroup::Tables).unwrap(),
        None,
        10,
    );
    let mut forged = relation_entry(profile_id, schema.clone(), "app", "public", "users");
    forged.id = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["outside", "private", "users"],
    );
    forged.relation_id = Some(forged.id.clone());
    assert!(CatalogPage::new(&request, vec![forged], CatalogCount::Exact(1), None).is_err());

    let mut missing_native_object =
        relation_entry(profile_id, schema.clone(), "app", "public", "users");
    missing_native_object.id = CatalogId::new(profile_id, CatalogKind::Table, ["app", "public"]);
    missing_native_object.relation_id = Some(missing_native_object.id.clone());
    assert!(
        CatalogPage::new(
            &request,
            vec![missing_native_object],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );

    let mut two_schema_request = request.clone();
    two_schema_request.scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".to_owned(),
            schemas: CatalogSelection::Selected(vec!["public".to_owned(), "audit".to_owned()]),
        }]),
    };
    let mismatched_parent = relation_entry(profile_id, schema, "app", "audit", "events");
    assert!(
        CatalogPage::new(
            &two_schema_request,
            vec![mismatched_parent],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );

    let relation = CatalogId::new(profile_id, CatalogKind::Table, ["app", "public", "users"]);
    let children_request = page_request(
        profile_id,
        CatalogTarget::relation_children(relation.clone()).unwrap(),
        None,
        10,
    );
    let forged_child = CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            ["outside", "private", "users", "id"],
        ),
        relation,
        QualifiedName {
            database: Some("app".to_owned()),
            schema: Some("public".to_owned()),
            object: "id".to_owned(),
        },
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
    )
    .unwrap();
    assert!(
        CatalogPage::new(
            &children_request,
            vec![forged_child],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );
}

#[test]
fn page_validation_rejects_same_schema_schema_object_identity_forgery() {
    let profile_id = Uuid::new_v4();
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let qualified_name = QualifiedName {
        database: Some("app".to_owned()),
        schema: Some("public".to_owned()),
        object: "expected_object".to_owned(),
    };

    for (kind, group) in [
        (CatalogKind::Table, ObjectGroup::Tables),
        (CatalogKind::View, ObjectGroup::Views),
        (
            CatalogKind::MaterializedView,
            ObjectGroup::MaterializedViews,
        ),
        (CatalogKind::Function, ObjectGroup::Functions),
        (CatalogKind::Procedure, ObjectGroup::Procedures),
        (CatalogKind::Sequence, ObjectGroup::Sequences),
        (CatalogKind::Type, ObjectGroup::Types),
    ] {
        let forged_id = CatalogId::new(
            profile_id,
            kind,
            ["app", "public", "different_object", "stable-suffix"],
        );
        let entry = if kind.is_relation() {
            CatalogEntry::relation(
                forged_id,
                schema.clone(),
                qualified_name.clone(),
                "object",
                OptionalMetadata::Unsupported,
                true,
            )
        } else {
            CatalogEntry::object(
                forged_id,
                schema.clone(),
                qualified_name.clone(),
                "object",
                OptionalMetadata::Unsupported,
                false,
            )
        }
        .unwrap();
        let request = page_request(
            profile_id,
            CatalogTarget::objects(schema.clone(), group).unwrap(),
            None,
            10,
        );

        assert!(
            CatalogPage::new(&request, vec![entry], CatalogCount::Exact(1), None).is_err(),
            "{kind:?} must use its qualified object in the canonical native path component"
        );
    }

    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["app", "public", "users", "relation-42"],
    );
    let trigger = CatalogEntry::relation_object(
        CatalogId::new(
            profile_id,
            CatalogKind::Trigger,
            ["app", "public", "different_trigger", "trigger-7"],
        ),
        schema.clone(),
        relation,
        qualified_name,
        "trigger",
        OptionalMetadata::Unsupported,
    )
    .unwrap();
    let request = page_request(
        profile_id,
        CatalogTarget::objects(schema, ObjectGroup::Triggers).unwrap(),
        None,
        10,
    );
    assert!(CatalogPage::new(&request, vec![trigger], CatalogCount::Exact(1), None).is_err());
}

#[test]
fn page_validation_binds_children_and_memberships_to_the_complete_relation_path() {
    let profile_id = Uuid::new_v4();
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["app", "public", "users", "relation-42"],
    );
    let request = page_request(
        profile_id,
        CatalogTarget::relation_children(relation.clone()).unwrap(),
        None,
        10,
    );
    let qualified_name = QualifiedName {
        database: Some("app".to_owned()),
        schema: Some("public".to_owned()),
        object: "display_column_name".to_owned(),
    };
    let mut valid_metadata = ColumnMetadata::new(1, "bigint", false);
    valid_metadata
        .constraint_memberships
        .push(ConstraintMembership {
            constraint_id: CatalogId::new(
                profile_id,
                CatalogKind::PrimaryKey,
                ["app", "public", "users", "relation-42", "constraint-9"],
            ),
            ordinal_position: 1,
        });
    let valid = CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            ["app", "public", "users", "relation-42", "column-attnum-1"],
        ),
        relation.clone(),
        qualified_name.clone(),
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(valid_metadata),
    )
    .unwrap();
    assert!(
        CatalogPage::new(&request, vec![valid], CatalogCount::Exact(1), None,).is_ok(),
        "synthetic child identities do not have to equal their display names"
    );

    let forged_child = CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            ["app", "public", "users", "relation-99", "column-attnum-1"],
        ),
        relation.clone(),
        qualified_name.clone(),
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
    )
    .unwrap();
    assert!(CatalogPage::new(&request, vec![forged_child], CatalogCount::Exact(1), None,).is_err());

    let mut forged_metadata = ColumnMetadata::new(1, "bigint", false);
    forged_metadata
        .constraint_memberships
        .push(ConstraintMembership {
            constraint_id: CatalogId::new(
                profile_id,
                CatalogKind::UniqueConstraint,
                ["app", "public", "users", "relation-99", "constraint-9"],
            ),
            ordinal_position: 1,
        });
    let forged_membership = CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            ["app", "public", "users", "relation-42", "column-attnum-1"],
        ),
        relation,
        qualified_name,
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(forged_metadata),
    )
    .unwrap();
    assert!(
        CatalogPage::new(
            &request,
            vec![forged_membership],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );
}

#[test]
fn page_validation_distinguishes_schema_and_relation_parent_triggers() {
    let profile_id = Uuid::new_v4();
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["app", "public", "users", "relation-42"],
    );
    let qualified_name = QualifiedName {
        database: Some("app".to_owned()),
        schema: Some("public".to_owned()),
        object: "audit_users".to_owned(),
    };

    let schema_trigger = CatalogEntry::relation_object(
        CatalogId::new(
            profile_id,
            CatalogKind::Trigger,
            ["app", "public", "audit_users", "trigger-7"],
        ),
        schema.clone(),
        relation.clone(),
        qualified_name.clone(),
        "trigger",
        OptionalMetadata::Unsupported,
    )
    .unwrap();
    let schema_request = page_request(
        profile_id,
        CatalogTarget::objects(schema, ObjectGroup::Triggers).unwrap(),
        None,
        10,
    );
    assert!(
        CatalogPage::new(
            &schema_request,
            vec![schema_trigger],
            CatalogCount::Exact(1),
            None,
        )
        .is_ok()
    );

    let relation_request = page_request(
        profile_id,
        CatalogTarget::relation_children(relation.clone()).unwrap(),
        None,
        10,
    );
    let relation_trigger = CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Trigger,
            ["app", "public", "users", "relation-42", "trigger-7"],
        ),
        relation.clone(),
        qualified_name.clone(),
        "trigger",
        OptionalMetadata::Unsupported,
        CatalogMetadata::None,
    )
    .unwrap();
    assert!(
        CatalogPage::new(
            &relation_request,
            vec![relation_trigger],
            CatalogCount::Exact(1),
            None,
        )
        .is_ok()
    );

    let forged_relation_trigger = CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Trigger,
            ["app", "public", "users", "relation-99", "trigger-7"],
        ),
        relation,
        qualified_name,
        "trigger",
        OptionalMetadata::Unsupported,
        CatalogMetadata::None,
    )
    .unwrap();
    assert!(
        CatalogPage::new(
            &relation_request,
            vec![forged_relation_trigger],
            CatalogCount::Exact(1),
            None,
        )
        .is_err()
    );
}

#[test]
fn page_keyset_cursors_round_trip_and_reject_malformed_encodings() {
    let cursor = CatalogCursor::from_keyset("Béta:1", "native:δ").unwrap();
    assert!(cursor.as_str().starts_with("v1:"));
    assert_eq!(cursor.keyset_parts().unwrap(), ("Béta:1", "native:δ"));
    assert!(CatalogCursor::from_keyset("users", "").is_err());

    let profile_id = Uuid::new_v4();
    for malformed in [
        "",
        "request-cursor",
        "v2:5:8:usersnative-1",
        "v1:x:8:usersnative-1",
        "v1:5:8:short",
        "v1:5:0:users",
        "v1:1:2:éx",
    ] {
        let request = page_request(
            profile_id,
            CatalogTarget::Databases,
            Some(CatalogCursor::new(malformed)),
            10,
        );
        assert!(
            request.validate().is_err(),
            "cursor should fail: {malformed}"
        );
    }

    let request = page_request(profile_id, CatalogTarget::Databases, None, 10);
    let mut page = CatalogPage::new(
        &request,
        vec![database_entry(profile_id, "app")],
        CatalogCount::Exact(1),
        None,
    )
    .unwrap();
    page.next_cursor = Some(CatalogCursor::new("not-keyset"));
    page.completeness = CatalogCompleteness::Partial;
    assert!(page.validate_for(&request).is_err());

    let mut rows = vec![
        ("users".to_owned(), "native-1".to_owned()),
        ("views".to_owned(), String::new()),
    ];
    assert!(finalize_keyset_page(&mut rows, 1, |row| row.0.clone(), |_| String::new(),).is_err());
    assert_eq!(
        rows.len(),
        2,
        "failed cursor creation must not truncate rows"
    );
}

#[test]
fn page_total_count_cannot_be_below_the_active_payload_length() {
    let profile_id = Uuid::new_v4();
    let entry = database_entry(profile_id, "app");
    let first = page_request(profile_id, CatalogTarget::Databases, None, 10);
    assert!(CatalogPage::new(&first, vec![entry.clone()], CatalogCount::Exact(0), None).is_err());
    assert!(CatalogPage::new(&first, vec![entry.clone()], CatalogCount::AtLeast(0), None).is_err());

    let continuation = page_request(
        profile_id,
        CatalogTarget::Databases,
        Some(CatalogCursor::from_keyset("app", "database-1").unwrap()),
        10,
    );
    assert!(CatalogPage::new(&continuation, vec![entry], CatalogCount::Exact(1), None,).is_ok());

    let groups_request = page_request(
        profile_id,
        CatalogTarget::groups(CatalogId::new(
            profile_id,
            CatalogKind::Schema,
            ["app", "public"],
        ))
        .unwrap(),
        None,
        10,
    );
    assert!(
        CatalogPage::groups(
            &groups_request,
            vec![CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Unknown,
            }],
            CatalogCount::Exact(0),
            None,
        )
        .is_err()
    );
}

#[test]
fn page_validation_rejects_non_advancing_and_short_partial_pages() {
    let profile_id = Uuid::new_v4();
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let groups_request = page_request(profile_id, CatalogTarget::groups(schema).unwrap(), None, 2);
    let next_cursor = CatalogCursor::from_keyset("views", "group-2").unwrap();
    assert!(
        CatalogPage::groups(
            &groups_request,
            vec![CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Unknown,
            }],
            CatalogCount::Unknown,
            Some(next_cursor.clone()),
        )
        .is_err()
    );
    assert!(
        CatalogPage::groups(
            &groups_request,
            Vec::new(),
            CatalogCount::Unknown,
            Some(next_cursor),
        )
        .is_err()
    );

    let request_cursor = CatalogCursor::from_keyset("app", "database-1").unwrap();
    let continuation = page_request(
        profile_id,
        CatalogTarget::Databases,
        Some(request_cursor.clone()),
        1,
    );
    assert!(
        CatalogPage::new(
            &continuation,
            vec![database_entry(profile_id, "app")],
            CatalogCount::Unknown,
            Some(request_cursor),
        )
        .is_err()
    );

    let unicode_cursor = CatalogCursor::from_keyset("Béta", "native-2").unwrap();
    let unicode_continuation = page_request(
        profile_id,
        CatalogTarget::Databases,
        Some(unicode_cursor),
        1,
    );
    let backward_cursor = CatalogCursor::from_keyset("Béta", "native-1").unwrap();
    assert!(
        CatalogPage::new(
            &unicode_continuation,
            vec![database_entry(profile_id, "app")],
            CatalogCount::Unknown,
            Some(backward_cursor),
        )
        .is_err()
    );

    let forward_cursor = CatalogCursor::from_keyset("Béta", "native-3").unwrap();
    assert!(
        CatalogPage::new(
            &unicode_continuation,
            vec![database_entry(profile_id, "app")],
            CatalogCount::Unknown,
            Some(forward_cursor),
        )
        .is_ok()
    );
}

#[test]
fn page_validation_uses_exact_counts_to_check_only_initial_page_completeness() {
    let profile_id = Uuid::new_v4();
    let initial = page_request(profile_id, CatalogTarget::Databases, None, 10);
    let entry = database_entry(profile_id, "app");
    assert!(
        CatalogPage::new(&initial, vec![entry.clone()], CatalogCount::Exact(2), None,).is_err()
    );
    assert!(CatalogPage::new(&initial, vec![entry.clone()], CatalogCount::Exact(1), None,).is_ok());

    let initial_partial = page_request(profile_id, CatalogTarget::Databases, None, 1);
    let next_cursor = CatalogCursor::from_keyset("app", "database-1").unwrap();
    assert!(
        CatalogPage::new(
            &initial_partial,
            vec![entry.clone()],
            CatalogCount::Exact(1),
            Some(next_cursor.clone()),
        )
        .is_err()
    );
    assert!(
        CatalogPage::new(
            &initial_partial,
            vec![entry.clone()],
            CatalogCount::Exact(2),
            Some(next_cursor),
        )
        .is_ok()
    );

    let continuation = page_request(
        profile_id,
        CatalogTarget::Databases,
        Some(CatalogCursor::from_keyset("before", "database-0").unwrap()),
        10,
    );
    for count in [
        CatalogCount::Exact(2),
        CatalogCount::AtLeast(1),
        CatalogCount::Unknown,
    ] {
        assert!(
            CatalogPage::new(&continuation, vec![entry.clone()], count, None).is_ok(),
            "continuation count {count:?} must not be compared for equality with this page"
        );
    }

    let continuation_partial = page_request(
        profile_id,
        CatalogTarget::Databases,
        Some(CatalogCursor::from_keyset("before", "database-0").unwrap()),
        1,
    );
    let after = CatalogCursor::from_keyset("later", "database-2").unwrap();
    for count in [
        CatalogCount::Exact(1),
        CatalogCount::AtLeast(1),
        CatalogCount::Unknown,
    ] {
        assert!(
            CatalogPage::new(
                &continuation_partial,
                vec![entry.clone()],
                count,
                Some(after.clone()),
            )
            .is_ok(),
            "continuation count {count:?} must remain valid on a full partial page"
        );
    }
}

#[tokio::test]
async fn page_dispatch_rejects_invalid_requests_and_unsupported_groups() {
    let imported = import_connection_url("sqlite://:memory:", Some("paged-contract")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "main"]);
    let valid = CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 9,
            },
            catalog_epoch: 4,
            request_id: 1,
            target: CatalogTarget::Databases,
            cursor: None,
        },
        scope: imported.profile.catalog_scope.clone(),
        page_size: 25,
    };
    assert_eq!(
        database
            .load_catalog_page(&valid)
            .await
            .unwrap()
            .total_count,
        CatalogCount::Exact(1)
    );

    let unsupported = CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 9,
            },
            catalog_epoch: 4,
            request_id: 2,
            target: CatalogTarget::objects(schema_id, ObjectGroup::Functions).unwrap(),
            cursor: None,
        },
        scope: imported.profile.catalog_scope.clone(),
        page_size: 25,
    };
    let error = database.load_catalog_page(&unsupported).await.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Unsupported);
    assert_eq!(error.code.as_deref(), Some("catalog_target_unsupported"));
    assert!(error.message.contains("objects"));

    let mut invalid = CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 9,
            },
            catalog_epoch: 4,
            request_id: 10,
            target: CatalogTarget::Databases,
            cursor: None,
        },
        scope: imported.profile.catalog_scope.clone(),
        page_size: 0,
    };
    let error = database.load_catalog_page(&invalid).await.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("invalid_catalog_request"));

    invalid.page_size = 1;
    invalid.key.connection.profile_id = Uuid::new_v4();
    let error = database.load_catalog_page(&invalid).await.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("invalid_catalog_request"));

    database.close().await;
}

fn page_request(
    profile_id: Uuid,
    target: CatalogTarget,
    cursor: Option<CatalogCursor>,
    page_size: usize,
) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 7,
            },
            catalog_epoch: 3,
            request_id: 11,
            target,
            cursor,
        },
        scope: CatalogScope::for_profile(DatabaseKind::Postgres, "app", Some("public")),
        page_size,
    }
}

fn database_entry(profile_id: Uuid, database: &str) -> CatalogEntry {
    CatalogEntry::database(
        CatalogId::new(profile_id, CatalogKind::Database, [database]),
        QualifiedName {
            database: Some(database.to_owned()),
            schema: None,
            object: database.to_owned(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn schema_entry(
    profile_id: Uuid,
    database_id: CatalogId,
    database: &str,
    schema: &str,
) -> CatalogEntry {
    CatalogEntry::schema(
        CatalogId::new(profile_id, CatalogKind::Schema, [database, schema]),
        database_id,
        QualifiedName {
            database: Some(database.to_owned()),
            schema: Some(schema.to_owned()),
            object: schema.to_owned(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn relation_entry(
    profile_id: Uuid,
    schema_id: CatalogId,
    database: &str,
    schema: &str,
    relation: &str,
) -> CatalogEntry {
    CatalogEntry::relation(
        CatalogId::new(profile_id, CatalogKind::Table, [database, schema, relation]),
        schema_id,
        QualifiedName {
            database: Some(database.to_owned()),
            schema: Some(schema.to_owned()),
            object: relation.to_owned(),
        },
        "table",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap()
}

fn id(profile_id: Uuid, kind: CatalogKind, native_id: &str) -> CatalogId {
    CatalogId::new(profile_id, kind, [native_id])
}
