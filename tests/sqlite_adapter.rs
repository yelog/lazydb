use std::collections::HashMap;

use futures_util::future::join_all;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCapabilities, CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry,
            CatalogId, CatalogKind, CatalogMetadata, CatalogRequest, CatalogRequestKey,
            CatalogSearchObjectScope, CatalogSearchRequest, CatalogTarget, ColumnMetadata,
            ColumnMetadataCapabilities, ConstraintMembership, ConstraintMetadata, DdlProvenance,
            IndexMetadata, NamespaceModel, ObjectGroup, OptionalMetadata,
        },
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::pagination::{PageRequest, PageSize},
    profile::{CatalogScope, CatalogSelection, DatabaseScope, import_connection_url},
};
use tempfile::TempDir;
use uuid::Uuid;

use lazydb::db::catalog_mutation::{
    CatalogMutationAnchor, CatalogMutationError, CatalogMutationMode, CatalogMutationPlan,
    CatalogMutationRequest, CatalogObjectType, ConstraintDefinitionKind,
};
use lazydb::model::catalog_editor::{CatalogDraft, ConstraintDraft, TableDraft};

const ATTACHED_ALIAS: &str = "ArchiveCase";

fn sqlite_create_request(
    profile_id: Uuid,
    database: &str,
    schema_name: &str,
) -> CatalogMutationRequest {
    CatalogMutationRequest {
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        request_id: 1,
        catalog_epoch: 1,
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Group {
            schema: CatalogId::new(
                profile_id,
                CatalogKind::Schema,
                [database.to_owned(), schema_name.to_owned()],
            ),
            group: ObjectGroup::Tables,
        },
        object_type: CatalogObjectType::Catalog(CatalogKind::Table),
        current_database: Some(database.to_owned()),
    }
}

fn sqlite_table_create_plan(
    profile_id: Uuid,
    database: &str,
    schema_name: &str,
    draft: TableDraft,
) -> Result<CatalogMutationPlan, CatalogMutationError> {
    lazydb::db::sqlite::SqliteAdapter::plan_catalog_mutation(
        sqlite_create_request(profile_id, database, schema_name),
        CatalogDraft::Table(draft),
        None,
    )
}

fn sqlite_table_draft(
    schema_name: &str,
    table_name: &str,
    columns: &[(&str, &str, bool)],
) -> TableDraft {
    let mut draft =
        TableDraft::new_for_database(schema_name, lazydb::profile::DatabaseKind::Sqlite);
    draft.name.set(table_name);
    draft.columns.clear();
    for (column_name, native_type, nullable) in columns {
        let mut column = lazydb::model::catalog_editor::ColumnDraft::new_added_for_database(
            lazydb::profile::DatabaseKind::Sqlite,
        );
        column.name.set(*column_name);
        column.native_type.set(*native_type);
        column.nullable = *nullable;
        draft.columns.push(column);
    }
    draft
}

fn sqlite_constrained_child_draft() -> TableDraft {
    let mut draft = sqlite_table_draft(
        "main",
        "child",
        &[
            ("tenant_id", "INTEGER", false),
            ("child_id", "INTEGER", false),
            ("parent_id", "INTEGER", true),
            ("code", "TEXT", false),
            ("amount", "INTEGER", false),
        ],
    );

    let mut primary_key = ConstraintDraft::new(
        ConstraintDefinitionKind::PrimaryKey { columns: vec![] },
        "main",
        "child",
    );
    primary_key.name.set("child\"pk");
    primary_key.columns.set("tenant_id, child_id");
    draft.add_constraint(primary_key);

    let mut unique = ConstraintDraft::new(
        ConstraintDefinitionKind::Unique { columns: vec![] },
        "main",
        "child",
    );
    unique.name.set("child_unique");
    unique.columns.set("tenant_id, code");
    draft.add_constraint(unique);

    let mut check = ConstraintDraft::new(
        ConstraintDefinitionKind::Check {
            expression: String::new(),
            no_inherit: false,
        },
        "main",
        "child",
    );
    check.name.set("child_check");
    check.expression.set("amount > 0 AND length(code) > 0");
    draft.add_constraint(check);

    let mut foreign_key = ConstraintDraft::new(
        ConstraintDefinitionKind::ForeignKey {
            columns: vec![],
            referenced_schema: "main".into(),
            referenced_relation: "parent".into(),
            referenced_columns: vec![],
            match_type: "SIMPLE".into(),
            on_update: "NO ACTION".into(),
            on_delete: "NO ACTION".into(),
        },
        "main",
        "child",
    );
    foreign_key.name.set("child\"fk");
    foreign_key.columns.set("tenant_id, parent_id");
    foreign_key.referenced_schema.set("main");
    foreign_key.referenced_relation.set("parent");
    foreign_key.referenced_columns.set("tenant_id, parent_id");
    foreign_key.on_update.set("cascade");
    foreign_key.on_delete.set("restrict");
    draft.add_constraint(foreign_key);

    draft
}

fn sqlite_create_error(
    profile_id: Uuid,
    database: &str,
    schema_name: &str,
    draft: TableDraft,
) -> String {
    match sqlite_table_create_plan(profile_id, database, schema_name, draft) {
        Ok(plan) => panic!("expected SQLite create plan to fail, got {}", plan.sql()),
        Err(CatalogMutationError::InvalidDraft { reason }) => reason,
        Err(error) => panic!("expected invalid SQLite draft, got {error:?}"),
    }
}

#[tokio::test]
async fn sqlite_executes_a_table_create_plan_against_a_temporary_database() {
    let imported = import_connection_url("sqlite://:memory:", Some("mutation-plan")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    let schema =
        lazydb::db::catalog::CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "main"]);
    let request = lazydb::db::catalog_mutation::CatalogMutationRequest {
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        request_id: 1,
        catalog_epoch: 1,
        mode: CatalogMutationMode::Create,
        anchor: CatalogMutationAnchor::Group {
            schema,
            group: ObjectGroup::Tables,
        },
        object_type: CatalogObjectType::Catalog(CatalogKind::Table),
        current_database: Some(":memory:".to_owned()),
    };
    let mut table = TableDraft::new("main");
    table.name.set("created_by_plan");
    table.columns[0].name.set("id");
    table.columns[0].native_type.set("INTEGER");
    let plan = lazydb::db::sqlite::SqliteAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::Table(table),
        None,
    )
    .unwrap();
    database.execute_catalog_mutation(&plan).await.unwrap();
    let result = database
        .execute("SELECT name FROM sqlite_master WHERE name = 'created_by_plan'")
        .await
        .unwrap();
    assert_eq!(result.result_sets[0].rows.len(), 1);
    database.close().await;
}

#[tokio::test]
async fn sqlite_create_plan_qualifies_main_and_targets_the_authorized_schema() {
    let imported = import_connection_url("sqlite://:memory:", Some("create-target")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    let draft = sqlite_table_draft(
        "main",
        "created\"table",
        &[("id", "INTEGER", false), ("label", "TEXT", true)],
    );

    let plan = sqlite_table_create_plan(profile_id, ":memory:", "main", draft).unwrap();
    assert_eq!(
        plan.sql(),
        "CREATE TABLE \"main\".\"created\"\"table\" (\"id\" INTEGER NOT NULL, \"label\" TEXT)"
    );
    assert!(!plan.sql().contains("CONSTRAINT"));
    let target = plan.execution_target.execution_target(profile_id);
    assert_eq!(target.database, ":memory:");
    assert_eq!(target.schema.as_deref(), Some("main"));
    assert!(target.is_valid(&imported.profile));

    database.execute_catalog_mutation(&plan).await.unwrap();
    let result = database
        .execute(
            "SELECT name FROM main.sqlite_master WHERE type = 'table' AND name = 'created\"table'",
        )
        .await
        .unwrap();
    assert_eq!(result.result_sets[0].rows.len(), 1);
    database.close().await;
}

#[tokio::test]
async fn sqlite_create_plan_applies_quoted_multi_column_constraints_and_enforces_them() {
    let imported = import_connection_url("sqlite://:memory:", Some("create-constraints")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            "CREATE TABLE parent (tenant_id INTEGER NOT NULL, parent_id INTEGER NOT NULL, PRIMARY KEY (tenant_id, parent_id)); INSERT INTO parent VALUES (1, 10);",
        )
        .await
        .unwrap();

    let plan = sqlite_table_create_plan(
        profile_id,
        ":memory:",
        "main",
        sqlite_constrained_child_draft(),
    )
    .unwrap();
    let sql = plan.sql();
    assert!(sql.starts_with("CREATE TABLE \"main\".\"child\""));
    assert!(sql.contains("CONSTRAINT \"child\"\"pk\" PRIMARY KEY (\"tenant_id\", \"child_id\")"));
    assert!(sql.contains("CONSTRAINT \"child_unique\" UNIQUE (\"tenant_id\", \"code\")"));
    assert!(sql.contains("CONSTRAINT \"child_check\" CHECK (amount > 0 AND length(code) > 0)"));
    assert!(sql.contains(
        "CONSTRAINT \"child\"\"fk\" FOREIGN KEY (\"tenant_id\", \"parent_id\") REFERENCES \"parent\" (\"tenant_id\", \"parent_id\") ON UPDATE CASCADE ON DELETE RESTRICT"
    ));
    assert_eq!(
        plan.execution_target
            .execution_target(profile_id)
            .schema
            .as_deref(),
        Some("main")
    );

    database.execute_catalog_mutation(&plan).await.unwrap();
    database
        .execute(
            "INSERT INTO main.child (tenant_id, child_id, parent_id, code, amount) VALUES (1, 100, 10, 'ok', 5)",
        )
        .await
        .unwrap();
    assert!(
        database
            .execute(
                "INSERT INTO main.child (tenant_id, child_id, parent_id, code, amount) VALUES (1, 100, 10, 'other', 6)",
            )
            .await
            .is_err(),
        "the composite primary key must reject duplicate keys"
    );
    assert!(
        database
            .execute(
                "INSERT INTO main.child (tenant_id, child_id, parent_id, code, amount) VALUES (1, 101, 10, 'ok', 6)",
            )
            .await
            .is_err(),
        "the unique constraint must reject duplicate tenant/code pairs"
    );
    assert!(
        database
            .execute(
                "INSERT INTO main.child (tenant_id, child_id, parent_id, code, amount) VALUES (1, 102, 10, 'ok', 0)",
            )
            .await
            .is_err(),
        "the check constraint must reject non-positive amounts"
    );
    assert!(
        database
            .execute(
                "INSERT INTO main.child (tenant_id, child_id, parent_id, code, amount) VALUES (1, 103, 99, 'ok', 5)",
            )
            .await
            .is_err(),
        "the foreign key must reject a missing parent"
    );

    database
        .execute("UPDATE main.parent SET parent_id = 11 WHERE tenant_id = 1 AND parent_id = 10")
        .await
        .unwrap();
    let cascaded = database
        .execute("SELECT parent_id FROM main.child WHERE tenant_id = 1 AND child_id = 100")
        .await
        .unwrap();
    assert!(matches!(
        cascaded.result_sets[0].rows[0][0],
        CellValue::Integer(11)
    ));
    assert!(
        database
            .execute("DELETE FROM main.parent WHERE tenant_id = 1 AND parent_id = 11")
            .await
            .is_err(),
        "the restrictive foreign key must reject deleting a referenced parent"
    );
    database.close().await;
}

#[test]
fn sqlite_create_plan_rejects_cross_schema_and_unsupported_constraint_options() {
    let profile_id = Uuid::new_v4();

    let mut cross_schema = sqlite_constrained_child_draft();
    cross_schema.table_constraints[3].referenced_schema = ATTACHED_ALIAS.into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", cross_schema);
    assert!(error.contains("cross-schema"), "{error}");

    let mut unsupported_match = sqlite_constrained_child_draft();
    unsupported_match.table_constraints[3].match_type = "FULL".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", unsupported_match);
    assert!(error.contains("MATCH SIMPLE"), "{error}");

    let mut unsupported_action = sqlite_constrained_child_draft();
    unsupported_action.table_constraints[3].on_delete = "SET NULL (tenant_id)".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", unsupported_action);
    assert!(error.contains("ON DELETE action"), "{error}");

    let mut not_valid = sqlite_constrained_child_draft();
    not_valid.table_constraints[3].not_valid = true;
    let error = sqlite_create_error(profile_id, ":memory:", "main", not_valid);
    assert!(error.contains("NOT VALID"), "{error}");

    let mut deferred_key = sqlite_constrained_child_draft();
    deferred_key.table_constraints[0].deferrable = true;
    let error = sqlite_create_error(profile_id, ":memory:", "main", deferred_key);
    assert!(error.contains("DEFERRABLE PRIMARY KEY"), "{error}");

    let mut unknown_column = sqlite_constrained_child_draft();
    unknown_column.table_constraints[1].columns = "tenant_id, missing".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", unknown_column);
    assert!(error.contains("unknown column"), "{error}");

    let mut duplicate_name = sqlite_constrained_child_draft();
    duplicate_name.table_constraints[1].name = duplicate_name.table_constraints[0].name.clone();
    let error = sqlite_create_error(profile_id, ":memory:", "main", duplicate_name);
    assert!(error.contains("names must be unique"), "{error}");

    let mut multiple_primary_keys = sqlite_constrained_child_draft();
    multiple_primary_keys.table_constraints[0].name = "second_pk".into();
    multiple_primary_keys.add_constraint(ConstraintDraft::new(
        ConstraintDefinitionKind::PrimaryKey { columns: vec![] },
        "main",
        "child",
    ));
    multiple_primary_keys.table_constraints[4].columns = "code".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", multiple_primary_keys);
    assert!(error.contains("at most one PRIMARY KEY"), "{error}");

    let mut unsafe_check = sqlite_constrained_child_draft();
    unsafe_check.table_constraints[2].expression = "amount > 0; DROP TABLE child".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", unsafe_check);
    assert!(error.contains("single expression"), "{error}");

    let mut malformed_check = sqlite_constrained_child_draft();
    malformed_check.table_constraints[2].expression = "amount >".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", malformed_check);
    assert!(error.contains("single expression"), "{error}");

    let mut terminated_check = sqlite_constrained_child_draft();
    terminated_check.table_constraints[2].expression = "amount > 0;".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", terminated_check);
    assert!(error.contains("single expression"), "{error}");

    let mut duplicate_source_columns = sqlite_constrained_child_draft();
    duplicate_source_columns.table_constraints[0].columns = "tenant_id, TENANT_ID".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", duplicate_source_columns);
    assert!(error.contains("must not repeat"), "{error}");

    let mut duplicate_referenced_columns = sqlite_constrained_child_draft();
    duplicate_referenced_columns.table_constraints[3].referenced_columns =
        "tenant_id, TENANT_ID".into();
    let error = sqlite_create_error(profile_id, ":memory:", "main", duplicate_referenced_columns);
    assert!(error.contains("must not repeat"), "{error}");

    let mut duplicate_columns = sqlite_table_draft(
        "main",
        "duplicate_columns",
        &[("id", "INTEGER", false), ("ID", "INTEGER", false)],
    );
    duplicate_columns.table_constraints.clear();
    let error = sqlite_create_error(profile_id, ":memory:", "main", duplicate_columns);
    assert!(error.contains("ASCII case"), "{error}");

    let mut mismatched_schema =
        sqlite_table_draft("other", "wrong_schema", &[("id", "INTEGER", false)]);
    mismatched_schema.table_constraints.clear();
    let error = sqlite_create_error(profile_id, ":memory:", "main", mismatched_schema);
    assert!(
        error.contains("does not match the selected schema"),
        "{error}"
    );

    let mut opaque_constraint = sqlite_constrained_child_draft();
    opaque_constraint.constraints = vec!["UNIQUE (code)".into()];
    let error = sqlite_create_error(profile_id, ":memory:", "main", opaque_constraint);
    assert!(error.contains("opaque constraint"), "{error}");
}

#[test]
fn sqlite_create_plan_rejects_malformed_schema_anchors() {
    let profile_id = Uuid::new_v4();
    let mut request = sqlite_create_request(profile_id, ":memory:", "main");
    let CatalogMutationAnchor::Group { schema, .. } = &mut request.anchor else {
        panic!("SQLite create request must use a group anchor");
    };
    *schema = CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        [":memory:", "main", "extra"],
    );
    let error = lazydb::db::sqlite::SqliteAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::Table(sqlite_table_draft(
            "main",
            "bad_anchor",
            &[("id", "INTEGER", false)],
        )),
        None,
    )
    .unwrap_err();
    match error {
        CatalogMutationError::InvalidAnchor { reason } => {
            assert!(reason.contains("incomplete"), "{reason}");
        }
        other => panic!("expected invalid SQLite anchor, got {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_create_plan_matches_source_columns_case_insensitively_and_accepts_check_literals() {
    let imported =
        import_connection_url("sqlite://:memory:", Some("create-case-and-check")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            "CREATE TABLE parent (tenant_id INTEGER NOT NULL, parent_id INTEGER NOT NULL, PRIMARY KEY (tenant_id, parent_id)); INSERT INTO parent VALUES (1, 10);",
        )
        .await
        .unwrap();

    let mut draft = sqlite_constrained_child_draft();
    draft.table_constraints[0].columns = "TENANT_ID, Child_Id".into();
    draft.table_constraints[1].columns = "Tenant_Id, CODE".into();
    draft.table_constraints[2].expression = "amount > 0 AND instr(code, ';') = 0".into();
    draft.table_constraints[3].columns = "TENANT_ID, Parent_Id".into();
    draft.table_constraints[3].referenced_columns = "TENANT_ID, PARENT_ID".into();

    let plan = sqlite_table_create_plan(profile_id, ":memory:", "main", draft).unwrap();
    let sql = plan.sql();
    assert!(
        sql.contains("PRIMARY KEY (\"tenant_id\", \"child_id\")"),
        "{sql}"
    );
    assert!(sql.contains("UNIQUE (\"tenant_id\", \"code\")"), "{sql}");
    assert!(
        sql.contains("FOREIGN KEY (\"tenant_id\", \"parent_id\")"),
        "{sql}"
    );
    assert!(
        sql.contains("REFERENCES \"parent\" (\"TENANT_ID\", \"PARENT_ID\")"),
        "{sql}"
    );
    assert!(
        sql.contains("CHECK (amount > 0 AND instr(code, ';') = 0)"),
        "{sql}"
    );

    database.execute_catalog_mutation(&plan).await.unwrap();
    database
        .execute(
            "INSERT INTO main.child (tenant_id, child_id, parent_id, code, amount) VALUES (1, 100, 10, 'ok', 5)",
        )
        .await
        .unwrap();
    assert!(
        database
            .execute(
                "INSERT INTO main.child (tenant_id, child_id, parent_id, code, amount) VALUES (1, 101, 10, 'bad;code', 5)",
            )
            .await
            .is_err(),
        "a semicolon inside a string literal must still be enforced by the check expression"
    );
    database.close().await;
}

#[tokio::test]
async fn sqlite_create_plan_targets_a_named_attached_schema_without_redirecting_to_main() {
    let temp = TempDir::new().unwrap();
    let main_path = temp.path().join("create-target.db");
    let attached_path = temp.path().join("create-attached.db");
    let imported = import_connection_url(
        &format!("sqlite://{}", main_path.display()),
        Some("create-attached"),
    )
    .unwrap();
    let mut profile = imported.profile;
    let database_name = profile.database.clone().unwrap();
    profile.catalog_scope = selected_scope(&database_name, &["main", ATTACHED_ALIAS]);
    let profile_id = profile.id;
    let database = DatabaseConnection::connect(&profile, None).await.unwrap();
    let attached_path = attached_path.to_string_lossy().replace('\'', "''");
    database
        .execute(&format!(
            "ATTACH DATABASE '{attached_path}' AS \"{ATTACHED_ALIAS}\""
        ))
        .await
        .unwrap();

    let plan = sqlite_table_create_plan(
        profile_id,
        &database_name,
        ATTACHED_ALIAS,
        sqlite_table_draft(
            ATTACHED_ALIAS,
            "attached_created",
            &[("id", "INTEGER", false)],
        ),
    )
    .unwrap();
    assert_eq!(
        plan.sql(),
        "CREATE TABLE \"ArchiveCase\".\"attached_created\" (\"id\" INTEGER NOT NULL)"
    );
    let target = plan.execution_target.execution_target(profile_id);
    assert_eq!(target.schema.as_deref(), Some(ATTACHED_ALIAS));
    assert!(target.is_valid(&profile));
    let mut restricted = profile.clone();
    restricted.catalog_scope = selected_scope(&database_name, &["main"]);
    assert!(!target.is_valid(&restricted));

    database.execute_catalog_mutation(&plan).await.unwrap();
    let attached = database
        .execute(&format!(
            "SELECT name FROM \"{ATTACHED_ALIAS}\".sqlite_master WHERE type = 'table' AND name = 'attached_created'"
        ))
        .await
        .unwrap();
    assert_eq!(attached.result_sets[0].rows.len(), 1);
    let main = database
        .execute(
            "SELECT name FROM main.sqlite_master WHERE type = 'table' AND name = 'attached_created'",
        )
        .await
        .unwrap();
    assert!(main.result_sets[0].rows.is_empty());
    database.close().await;
}

#[tokio::test]
async fn sqlite_loads_and_executes_a_table_edit_plan_against_a_temporary_database() {
    let imported = import_connection_url("sqlite://:memory:", Some("edit-plan")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute("CREATE TABLE records (id INTEGER NOT NULL, value TEXT)")
        .await
        .unwrap();
    let object = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let definition = database
        .load_catalog_object_definition(
            &lazydb::db::catalog_mutation::CatalogObjectDefinitionRequest {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                request_id: 1,
                catalog_epoch: 1,
                object: object.clone(),
                target: lazydb::model::execution_target::ExecutionTarget {
                    profile_id,
                    database: ":memory:".to_owned(),
                    schema: Some("main".to_owned()),
                },
                principal: None,
            },
        )
        .await
        .unwrap();
    let lazydb::db::catalog_mutation::CatalogObjectDefinition::Table(table_definition) = definition
    else {
        panic!("SQLite table definition loader returned the wrong object type");
    };
    assert_eq!(table_definition.columns.len(), 2);
    let mut draft = TableDraft::from_definition(&table_definition);
    draft.name.set("renamed_records");
    let plan = database
        .plan_catalog_mutation(
            lazydb::db::catalog_mutation::CatalogMutationRequest {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                request_id: 2,
                catalog_epoch: 1,
                mode: CatalogMutationMode::Edit,
                anchor: CatalogMutationAnchor::Catalog(object),
                object_type: CatalogObjectType::Catalog(CatalogKind::Table),
                current_database: Some(":memory:".to_owned()),
            },
            CatalogDraft::Table(draft),
            Some(lazydb::db::catalog_mutation::CatalogObjectDefinition::Table(table_definition)),
        )
        .unwrap();
    database.execute_catalog_mutation(&plan).await.unwrap();
    assert!(
        database
            .object_ddl(CatalogKind::Table, "main", "renamed_records")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        database
            .object_ddl(CatalogKind::Table, "main", "records")
            .await
            .unwrap()
            .is_none()
    );
    database.close().await;
}

#[tokio::test]
async fn sqlite_rebuilds_structural_table_edits_without_losing_data_indexes_or_triggers() {
    let imported = import_connection_url("sqlite://:memory:", Some("rebuild-plan")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            "CREATE TABLE audit (item_id INTEGER); CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT, normalized TEXT GENERATED ALWAYS AS (lower(value)) STORED); CREATE INDEX items_value_idx ON items(value); CREATE TRIGGER items_audit AFTER INSERT ON items BEGIN INSERT INTO audit VALUES (NEW.id); END; INSERT INTO items(id, value) VALUES (1, 'before');",
        )
        .await
        .unwrap();
    let object = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "items"],
    );
    let definition = database
        .load_catalog_object_definition(
            &lazydb::db::catalog_mutation::CatalogObjectDefinitionRequest {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                request_id: 1,
                catalog_epoch: 1,
                object: object.clone(),
                target: lazydb::model::execution_target::ExecutionTarget {
                    profile_id,
                    database: ":memory:".to_owned(),
                    schema: Some("main".to_owned()),
                },
                principal: None,
            },
        )
        .await
        .unwrap();
    let lazydb::db::catalog_mutation::CatalogObjectDefinition::Table(table_definition) = definition
    else {
        panic!("SQLite table definition loader returned the wrong object type");
    };
    let mut draft = TableDraft::from_definition(&table_definition);
    draft
        .columns
        .iter_mut()
        .find(|column| column.name.value() == "value")
        .expect("value column should be loaded")
        .native_type
        .set("BLOB");
    let plan = database
        .plan_catalog_mutation(
            lazydb::db::catalog_mutation::CatalogMutationRequest {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                request_id: 2,
                catalog_epoch: 1,
                mode: CatalogMutationMode::Edit,
                anchor: CatalogMutationAnchor::Catalog(object.clone()),
                object_type: CatalogObjectType::Catalog(CatalogKind::Table),
                current_database: Some(":memory:".to_owned()),
            },
            CatalogDraft::Table(draft),
            Some(lazydb::db::catalog_mutation::CatalogObjectDefinition::Table(table_definition)),
        )
        .unwrap();
    assert!(plan.rebuild.is_some());
    database.execute_catalog_mutation(&plan).await.unwrap();
    database
        .execute("INSERT INTO main.items(id, value) VALUES (2, 'after')")
        .await
        .unwrap();
    let audit = database
        .execute("SELECT COUNT(*) FROM main.audit")
        .await
        .unwrap();
    assert!(matches!(
        audit.result_sets[0].rows[0][0],
        CellValue::Integer(2)
    ));
    let ddl = database.relation_ddl(&object).await.unwrap();
    assert!(ddl.sql.contains("CREATE INDEX items_value_idx"));
    assert!(ddl.sql.contains("CREATE TRIGGER items_audit"));
    database.close().await;
}

#[tokio::test]
async fn sqlite_rebuild_restores_composite_keys_and_foreign_key_enforcement() {
    let imported = import_connection_url("sqlite://:memory:", Some("rebuild-foreign-key")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            "CREATE TABLE parent (parent_id INTEGER PRIMARY KEY); INSERT INTO parent VALUES (1); CREATE TABLE child (tenant_id INTEGER NOT NULL, parent_id INTEGER NOT NULL, value INTEGER, PRIMARY KEY (tenant_id, parent_id), FOREIGN KEY (parent_id) REFERENCES parent(parent_id)) WITHOUT ROWID; INSERT INTO child VALUES (7, 1, 10);",
        )
        .await
        .unwrap();
    let object = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "child"],
    );
    let definition = database
        .load_catalog_object_definition(
            &lazydb::db::catalog_mutation::CatalogObjectDefinitionRequest {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                request_id: 1,
                catalog_epoch: 1,
                object: object.clone(),
                target: lazydb::model::execution_target::ExecutionTarget {
                    profile_id,
                    database: ":memory:".to_owned(),
                    schema: Some("main".to_owned()),
                },
                principal: None,
            },
        )
        .await
        .unwrap();
    let lazydb::db::catalog_mutation::CatalogObjectDefinition::Table(table_definition) = definition
    else {
        panic!("SQLite child definition loader returned the wrong object type");
    };
    let mut draft = TableDraft::from_definition(&table_definition);
    draft
        .columns
        .iter_mut()
        .find(|column| column.name.value() == "value")
        .expect("value column should be loaded")
        .native_type
        .set("TEXT");
    let plan = database
        .plan_catalog_mutation(
            lazydb::db::catalog_mutation::CatalogMutationRequest {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                request_id: 2,
                catalog_epoch: 1,
                mode: CatalogMutationMode::Edit,
                anchor: CatalogMutationAnchor::Catalog(object.clone()),
                object_type: CatalogObjectType::Catalog(CatalogKind::Table),
                current_database: Some(":memory:".to_owned()),
            },
            CatalogDraft::Table(draft),
            Some(lazydb::db::catalog_mutation::CatalogObjectDefinition::Table(table_definition)),
        )
        .unwrap();
    database.execute_catalog_mutation(&plan).await.unwrap();
    assert!(
        database
            .execute("INSERT INTO main.child VALUES (8, 999, 'invalid')")
            .await
            .is_err()
    );
    let ddl = database.relation_ddl(&object).await.unwrap();
    assert!(ddl.sql.contains("PRIMARY KEY (tenant_id, parent_id)"));
    assert!(ddl.sql.contains("FOREIGN KEY (parent_id)"));
    database.close().await;
}

#[tokio::test]
async fn relation_reads_use_request_scope_without_reconnecting() {
    let mut imported = import_connection_url("sqlite://:memory:", Some("request-scope")).unwrap();
    imported.profile.catalog_scope = CatalogScope::for_profile(
        lazydb::profile::DatabaseKind::Sqlite,
        ":memory:",
        Some("other"),
    );
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute("CREATE TABLE main.request_scope (id INTEGER PRIMARY KEY, label TEXT);")
        .await
        .unwrap();

    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "request_scope"],
    );
    let scope = CatalogScope::for_profile(
        lazydb::profile::DatabaseKind::Sqlite,
        ":memory:",
        Some("main"),
    );
    let preview = database
        .preview_relation_with_scope(
            &relation,
            &scope,
            &Default::default(),
            PageRequest::first(PageSize::Ten),
        )
        .await
        .unwrap();
    assert!(preview.sql.contains("main"));

    let ddl = database
        .relation_ddl_with_scope(&relation, &scope)
        .await
        .unwrap();
    assert!(ddl.sql.contains("request_scope"));
    database.close().await;
}

#[tokio::test]
async fn relation_preview_preserves_metadata_limits_quotes_and_rejects_forged_ids() {
    let imported = import_connection_url("sqlite://:memory:", Some("relation-preview")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            r#"
            CREATE TABLE "odd""table" ("odd""column" INTEGER, "payload" TEXT);
            CREATE TABLE empty (id INTEGER, note TEXT);
            CREATE TABLE many (id INTEGER);
            CREATE VIEW "odd""view" AS SELECT "odd""column", "payload" FROM "odd""table";
            WITH RECURSIVE numbers(value) AS (
                SELECT 1 UNION ALL SELECT value + 1 FROM numbers WHERE value < 501
            ) INSERT INTO many SELECT value FROM numbers;
            "#,
        )
        .await
        .unwrap_or_else(|_| panic!("SQLite test setup must support the requested DDL"));

    let empty = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "empty"],
    );
    let preview = database
        .preview_relation(
            &empty,
            &Default::default(),
            PageRequest::first(PageSize::FiveHundred),
        )
        .await
        .unwrap();
    assert!(preview.sql.contains("LIMIT 501 OFFSET 0"));
    assert_eq!(preview.result.stats.row_count, 0);
    assert_eq!(
        preview.result.result_sets[0]
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.type_name.as_str()))
            .collect::<Vec<_>>(),
        [("id", "INTEGER"), ("note", "TEXT")]
    );

    let many = CatalogId::new(profile_id, CatalogKind::Table, [":memory:", "main", "many"]);
    let many_preview = database
        .preview_relation(
            &many,
            &Default::default(),
            PageRequest::first(PageSize::FiveHundred),
        )
        .await
        .unwrap();
    assert_eq!(many_preview.result.stats.row_count, 500);
    assert_eq!(many_preview.result.result_sets[0].rows.len(), 500);
    assert!(many_preview.pagination.has_next);

    let second_page = database
        .preview_relation(
            &many,
            &Default::default(),
            PageRequest::at(PageSize::Ten, 10),
        )
        .await
        .unwrap();
    assert!(second_page.sql.contains("LIMIT 11 OFFSET 10"));
    assert_eq!(second_page.result.stats.row_count, 10);
    assert_eq!(second_page.result.result_sets[0].rows.len(), 10);

    let last_page = database
        .preview_relation(
            &many,
            &Default::default(),
            PageRequest::last(PageSize::Ten, 0),
        )
        .await
        .unwrap();
    assert!(last_page.sql.contains("LIMIT 11 OFFSET 500"));
    assert_eq!(last_page.result.stats.row_count, 1);
    assert_eq!(
        last_page.pagination.total,
        lazydb::model::pagination::TotalRows::Exact(501)
    );

    let hostile = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "odd\"table"],
    );
    let hostile_preview = database
        .preview_relation(
            &hostile,
            &Default::default(),
            PageRequest::first(PageSize::FiveHundred),
        )
        .await
        .unwrap();
    assert!(hostile_preview.sql.contains("\"odd\"\"table\""));
    assert!(hostile_preview.sql.contains("LIMIT 501 OFFSET 0"));

    let view = CatalogId::new(
        profile_id,
        CatalogKind::View,
        [":memory:", "main", "odd\"view"],
    );
    let view_preview = database
        .preview_relation(
            &view,
            &Default::default(),
            PageRequest::first(PageSize::FiveHundred),
        )
        .await
        .unwrap();
    assert_eq!(view_preview.result.stats.row_count, 0);

    for id in [
        CatalogId::new(
            Uuid::new_v4(),
            CatalogKind::Table,
            [":memory:", "main", "empty"],
        ),
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            [":memory:", "main", "empty", "id"],
        ),
        CatalogId::new(
            profile_id,
            CatalogKind::Table,
            [":memory:", "main", "missing"],
        ),
    ] {
        let error = database
            .preview_relation(
                &id,
                &Default::default(),
                PageRequest::first(PageSize::FiveHundred),
            )
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
    }
    database.close().await;
}

#[tokio::test]
async fn relation_preview_orders_multiple_columns_across_pages() {
    let imported = import_connection_url("sqlite://:memory:", Some("relation-sort-pages")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            "CREATE TABLE records (id INTEGER PRIMARY KEY, team TEXT, score INTEGER); \
             INSERT INTO records VALUES (1, 'b', 2), (2, 'a', 3), (3, 'a', 1), (4, 'b', 1), (5, 'a', 2);",
        )
        .await
        .unwrap();
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let options = lazydb::model::relation::RelationPreviewOptions {
        order_by_clause: Some("team ASC, score DESC".into()),
        ..Default::default()
    };

    let first = database
        .preview_relation(&relation, &options, PageRequest::at(PageSize::Ten, 0))
        .await
        .unwrap();
    let second = database
        .preview_relation(&relation, &options, PageRequest::at(PageSize::Ten, 3))
        .await
        .unwrap();
    let rows = |preview: &lazydb::db::RelationPreview| {
        preview.result.result_sets[0]
            .rows
            .iter()
            .map(|row| format!("{:?}", row))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&first).len(), 5);
    assert!(first.sql.contains("ORDER BY team ASC, score DESC"));
    assert_eq!(rows(&second).len(), 2);
    assert!(rows(&first)[0].contains("a"));
    assert!(rows(&first)[1].contains("a"));
    assert!(rows(&first)[2].contains("a"));
    assert!(rows(&first)[3].contains("b"));
    assert!(rows(&second)[0].contains("b"));
    database.close().await;
}

#[tokio::test]
async fn relation_ddl_with_only_the_relation_uses_native_catalog_provenance() {
    let imported = import_connection_url("sqlite://:memory:", Some("relation-ddl")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute("CREATE TABLE records (id INTEGER PRIMARY KEY, label TEXT NOT NULL DEFAULT 'x');")
        .await
        .unwrap();
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let ddl = database.relation_ddl(&relation).await.unwrap();
    assert_eq!(ddl.relation.id, relation);
    assert!(ddl.sql.contains("CREATE TABLE records"));
    assert_eq!(ddl.provenance, DdlProvenance::NativeCatalog);
    assert_eq!(ddl.relation.native_kind, "table");
    database.close().await;
}

#[tokio::test]
async fn relation_ddl_includes_explicit_indexes_and_triggers_but_not_autoindexes() {
    let imported =
        import_connection_url("sqlite://:memory:", Some("relation-composite-ddl")).unwrap();
    let profile_id = imported.profile.id;
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            r#"
            CREATE TABLE records (
                id INTEGER PRIMARY KEY,
                label TEXT NOT NULL UNIQUE,
                updated_at TEXT
            );
            CREATE INDEX records_label_expression_idx ON records (lower(label));
            CREATE TRIGGER records_set_updated_at
            AFTER UPDATE ON records
            BEGIN
                UPDATE records SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
            END;
            "#,
        )
        .await
        .unwrap();

    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let ddl = database.relation_ddl(&relation).await.unwrap();

    assert!(ddl.sql.contains("-- Table\n\nCREATE TABLE records"));
    assert!(ddl.sql.contains(
        "-- Indexes\n\nCREATE INDEX records_label_expression_idx ON records (lower(label));"
    ));
    assert!(
        ddl.sql
            .contains("-- Triggers\n\nCREATE TRIGGER records_set_updated_at")
    );
    assert!(!ddl.sql.contains("sqlite_autoindex"));
    assert!(ddl.sql.trim_end().ends_with("END;"));
    assert_eq!(ddl.provenance, DdlProvenance::AdapterGenerated);
    assert!(ddl.children.entries.iter().any(|entry| {
        entry.kind == CatalogKind::Column && entry.qualified_name.object == "label"
    }));

    database.close().await;
}

#[tokio::test]
async fn relation_ddl_rejects_excluded_scope_before_loading_ddl() {
    let imported = import_connection_url("sqlite://:memory:", Some("relation-scope")).unwrap();
    let profile_id = imported.profile.id;
    let mut profile = imported.profile;
    profile.catalog_scope = selected_scope(":memory:", &["other"]);
    let database = DatabaseConnection::connect(&profile, None).await.unwrap();
    database
        .execute("CREATE TABLE records (id INTEGER);")
        .await
        .unwrap();

    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "records"],
    );
    let error = database.relation_ddl(&relation).await.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    database.close().await;
}

struct CatalogFixture {
    _temp: TempDir,
    database: DatabaseConnection,
    profile_id: Uuid,
    configured_database: String,
}

impl CatalogFixture {
    async fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let main_path = temp.path().join("catalog.db");
        let attached_path = temp.path().join("archive.db");
        let imported = import_connection_url(
            &format!("sqlite://{}", main_path.display()),
            Some("catalog-pages"),
        )
        .unwrap();
        let profile_id = imported.profile.id;
        let configured_database = imported.profile.database.clone().unwrap();
        let database = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap();
        let attached_path = attached_path.to_string_lossy().replace('\'', "''");

        database
            .execute(&format!(
                r#"
                PRAGMA foreign_keys = ON;
                ATTACH DATABASE '{attached_path}' AS "ArchiveCase";

                CREATE TABLE parent (
                    tenant_id INTEGER,
                    parent_id INTEGER,
                    code TEXT NOT NULL,
                    PRIMARY KEY (tenant_id, parent_id),
                    UNIQUE (tenant_id, code)
                ) WITHOUT ROWID;
                CREATE TABLE child (
                    tenant_id INTEGER NOT NULL,
                    child_id INTEGER NOT NULL,
                    owner_id INTEGER NOT NULL DEFAULT 7,
                    label TEXT NOT NULL DEFAULT 'new',
                    label_key TEXT GENERATED ALWAYS AS (lower(label)) STORED,
                    PRIMARY KEY (tenant_id, child_id),
                    UNIQUE (tenant_id, label),
                    FOREIGN KEY (tenant_id, owner_id)
                        REFERENCES parent(tenant_id, parent_id)
                ) WITHOUT ROWID;
                CREATE INDEX child_lookup_idx ON child(owner_id, label);
                CREATE TABLE Alpha (id INTEGER PRIMARY KEY);
                CREATE TABLE beta (id INTEGER PRIMARY KEY);
                CREATE TABLE shared_name (main_value TEXT);
                CREATE VIEW child_view AS
                    SELECT tenant_id, child_id, label_key FROM child;
                CREATE TRIGGER child_label_guard BEFORE INSERT ON child
                    WHEN NEW.label = ''
                    BEGIN
                        SELECT RAISE(ABORT, 'label required');
                    END;

                CREATE TABLE "ArchiveCase".shared_name (archive_value INTEGER);
                CREATE TABLE "ArchiveCase".excluded_only (id INTEGER PRIMARY KEY);
                "#
            ))
            .await
            .unwrap();

        Self {
            _temp: temp,
            database,
            profile_id,
            configured_database,
        }
    }

    fn scope(&self, schemas: &[&str]) -> CatalogScope {
        selected_scope(&self.configured_database, schemas)
    }

    fn database_id(&self) -> CatalogId {
        CatalogId::new(
            self.profile_id,
            CatalogKind::Database,
            [self.configured_database.clone()],
        )
    }

    fn schema_id(&self, schema: &str) -> CatalogId {
        CatalogId::new(
            self.profile_id,
            CatalogKind::Schema,
            [self.configured_database.clone(), schema.to_owned()],
        )
    }

    async fn close(self) {
        self.database.close().await;
    }

    async fn search(
        &self,
        query: &str,
        schemas: &[&str],
        limit: usize,
    ) -> lazydb::db::catalog::CatalogSearchPage {
        self.database
            .search_catalog(&CatalogSearchRequest {
                connection: ConnectionIdentity {
                    profile_id: self.profile_id,
                    generation: 9,
                },
                session_id: 11,
                generation: 12,
                query: query.into(),
                scope: self.scope(schemas),
                object_scope: CatalogSearchObjectScope::AllObjects,
                limit,
            })
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn catalog_search_finds_unloaded_objects_children_paths_scope_and_limit() {
    let fixture = CatalogFixture::new().await;
    let connection = ConnectionIdentity {
        profile_id: fixture.profile_id,
        generation: 9,
    };
    let page = fixture
        .database
        .search_catalog(&CatalogSearchRequest {
            connection,
            session_id: 1,
            generation: 4,
            query: "CHILD.LABEL".into(),
            scope: fixture.scope(&["main"]),
            object_scope: CatalogSearchObjectScope::AllObjects,
            limit: 100,
        })
        .await
        .unwrap();

    assert!(page.hits.iter().any(|hit| {
        hit.entry.kind == CatalogKind::Column
            && hit.entry.qualified_name.object == "label"
            && hit.qualified_path().to_lowercase().contains("child.label")
    }));
    assert!(page.hits.iter().all(|hit| {
        hit.entry
            .qualified_name
            .schema
            .as_deref()
            .is_none_or(|schema| schema == "main")
    }));

    let limited = fixture
        .database
        .search_catalog(&CatalogSearchRequest {
            connection,
            session_id: 1,
            generation: 5,
            query: "a".into(),
            scope: fixture.scope(&["main"]),
            object_scope: CatalogSearchObjectScope::AllObjects,
            limit: 1,
        })
        .await
        .unwrap();
    assert_eq!(limited.hits.len(), 1);
    assert!(limited.truncated);
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_ignores_identifier_separators() {
    let fixture = CatalogFixture::new().await;

    let page = fixture.search("childview", &["main"], 100).await;

    assert!(page.hits.iter().any(|hit| {
        hit.entry.kind == CatalogKind::View && hit.entry.qualified_name.object == "child_view"
    }));
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_preserves_all_sqlite_kinds_metadata_and_ancestors() {
    let fixture = CatalogFixture::new().await;

    let cases = [
        (&fixture.configured_database[..], CatalogKind::Database),
        ("MAIN", CatalogKind::Schema),
        ("child", CatalogKind::Table),
        ("child_view", CatalogKind::View),
        ("child_label_guard", CatalogKind::Trigger),
        ("label_key", CatalogKind::Column),
        ("child_lookup_idx", CatalogKind::Index),
        ("sqlite_autoindex_child_1", CatalogKind::PrimaryKey),
        ("sqlite_autoindex_child_2", CatalogKind::UniqueConstraint),
        ("fk_child_0", CatalogKind::ForeignKey),
    ];
    for (query, kind) in cases {
        let page = fixture.search(query, &["main"], 100).await;
        assert_eq!(page.total_count, None);
        let hit = page
            .hits
            .iter()
            .find(|hit| {
                hit.entry.kind == kind
                    && hit.entry.qualified_name.object.eq_ignore_ascii_case(query)
            })
            .unwrap_or_else(|| panic!("missing {kind:?} search hit for {query}"));
        assert_eq!(hit.entry.id.connection_id, fixture.profile_id);
        if kind.is_relation_child() {
            assert_eq!(hit.ancestors.len(), 3);
            assert!(hit.ancestors[2].kind.is_relation());
            assert_eq!(hit.entry.relation_id.as_ref(), Some(&hit.ancestors[2].id));
        }
    }

    let fallback = fixture.search("primary_key", &["main"], 100).await;
    let alpha_primary_key = fallback
        .hits
        .iter()
        .find(|hit| {
            hit.entry.kind == CatalogKind::PrimaryKey
                && hit
                    .ancestors
                    .last()
                    .is_some_and(|owner| owner.qualified_name.object == "Alpha")
        })
        .expect("INTEGER PRIMARY KEY must expose the fallback primary_key entry");
    assert_eq!(
        alpha_primary_key.entry.id.native_path.last().unwrap(),
        "primary_key"
    );
    assert_eq!(
        alpha_primary_key.entry.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
            columns: vec!["id".to_owned()]
        })
    );

    let trigger = fixture.search("child_label_guard", &["main"], 100).await;
    let trigger = trigger
        .hits
        .iter()
        .find(|hit| hit.entry.kind == CatalogKind::Trigger)
        .unwrap();
    assert_eq!(
        trigger
            .ancestors
            .iter()
            .map(|entry| entry.qualified_name.object.as_str())
            .collect::<Vec<_>>(),
        [fixture.configured_database.as_str(), "main", "child"]
    );
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_is_case_insensitive_scoped_literal_and_globally_ranked() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute(
            r#"
            CREATE TABLE "literal%name" ("literal_value" TEXT);
            CREATE TABLE "literal_name" (other TEXT);
            CREATE TABLE rankneedle (path_only TEXT);
            CREATE TABLE rankneedle_prefix (id INTEGER);
            CREATE TABLE xrankneedlex (id INTEGER);
            "#,
        )
        .await
        .unwrap();

    let percent = fixture.search("%", &["main"], 100).await;
    assert!(
        percent
            .hits
            .iter()
            .any(|hit| hit.entry.qualified_name.object == "literal%name")
    );
    assert!(
        percent
            .hits
            .iter()
            .all(|hit| hit.qualified_path().contains('%'))
    );

    let underscore = fixture.search("_", &["main"], 100).await;
    assert!(underscore.hits.iter().all(|hit| {
        hit.entry.qualified_name.object.contains('_') || hit.qualified_path().contains('_')
    }));
    let scoped = fixture.search("SHARED_NAME", &[ATTACHED_ALIAS], 100).await;
    assert!(scoped.hits.iter().any(|hit| {
        hit.entry.kind == CatalogKind::Table
            && hit.entry.qualified_name.schema.as_deref() == Some(ATTACHED_ALIAS)
    }));
    assert!(scoped.hits.iter().all(|hit| {
        hit.entry
            .qualified_name
            .schema
            .as_deref()
            .is_none_or(|schema| schema == ATTACHED_ALIAS)
    }));

    let ranked = fixture.search("RANKNEEDLE", &["main"], 100).await;
    let ranks = ranked
        .hits
        .iter()
        .map(|hit| {
            let name = hit.entry.qualified_name.object.to_lowercase();
            if name == "rankneedle" {
                0
            } else if name.starts_with("rankneedle") {
                1
            } else if name.contains("rankneedle") {
                2
            } else {
                3
            }
        })
        .collect::<Vec<_>>();
    assert!(ranks.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(ranks.first(), Some(&0));
    assert!(
        ranks.contains(&3),
        "qualified owner paths must participate in ranking"
    );

    let repeated = fixture.search("rankneedle", &["main"], 100).await;
    assert_eq!(
        ranked
            .hits
            .iter()
            .map(|hit| &hit.entry.id)
            .collect::<Vec<_>>(),
        repeated
            .hits
            .iter()
            .map(|hit| &hit.entry.id)
            .collect::<Vec<_>>()
    );
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_rejects_wrong_profile_and_skips_unrelated_relation_hydration() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute("CREATE TABLE poison (value INTEGER REFERENCES missing_parent);")
        .await
        .unwrap();

    let page = fixture.search("child.label", &["main"], 100).await;
    assert!(
        page.hits
            .iter()
            .any(|hit| hit.entry.qualified_name.object == "label")
    );

    let error = fixture
        .database
        .search_catalog(&CatalogSearchRequest {
            connection: ConnectionIdentity {
                profile_id: Uuid::new_v4(),
                generation: 9,
            },
            session_id: 1,
            generation: 1,
            query: "child".into(),
            scope: fixture.scope(&["main"]),
            object_scope: CatalogSearchObjectScope::AllObjects,
            limit: 100,
        })
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("invalid_catalog_request"));
    fixture.close().await;
}

#[tokio::test]
async fn catalog_search_trigger_path_uses_the_canonical_owner() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute(
            r#"
            CREATE TABLE MixedOwner (id INTEGER PRIMARY KEY);
            CREATE TRIGGER mixed_table_guard BEFORE INSERT ON mixedowner
                BEGIN SELECT 1; END;
            PRAGMA writable_schema = ON;
            INSERT INTO sqlite_schema (type, name, tbl_name, rootpage, sql)
                VALUES (
                    'view',
                    'mixedowner',
                    'mixedowner',
                    0,
                    'CREATE VIEW mixedowner AS SELECT 1 AS id'
                );
            PRAGMA writable_schema = OFF;
            "#,
        )
        .await
        .unwrap();

    let page = fixture
        .search("mixedowner.mixed_table_guard", &["main"], 100)
        .await;
    let trigger = page
        .hits
        .iter()
        .find(|hit| hit.entry.kind == CatalogKind::Trigger)
        .expect("trigger owner path should be searchable");
    let owner = trigger.ancestors.last().unwrap();
    assert_eq!(owner.kind, CatalogKind::Table);
    assert_eq!(owner.qualified_name.object, "MixedOwner");
    assert_eq!(trigger.entry.relation_id.as_ref(), Some(&owner.id));
    fixture.close().await;
}

fn selected_scope(database: &str, schemas: &[&str]) -> CatalogScope {
    CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database.to_owned(),
            schemas: CatalogSelection::Selected(
                schemas.iter().map(|schema| (*schema).to_owned()).collect(),
            ),
        }]),
    }
}

fn catalog_request(
    profile_id: Uuid,
    target: CatalogTarget,
    scope: CatalogScope,
    page_size: usize,
    cursor: Option<CatalogCursor>,
    request_id: u64,
) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 9,
            },
            catalog_epoch: 4,
            request_id,
            target,
            cursor,
        },
        scope,
        page_size,
    }
}

fn expected_catalog_capabilities() -> CatalogCapabilities {
    CatalogCapabilities {
        namespace_model: NamespaceModel::DatabaseAndSchema,
        top_level_groups: vec![
            ObjectGroup::Tables,
            ObjectGroup::Views,
            ObjectGroup::Triggers,
        ],
        column_metadata: ColumnMetadataCapabilities {
            default_expression: true,
            hidden: true,
            ..ColumnMetadataCapabilities::default()
        },
        supports_lazy_children: true,
    }
}

#[tokio::test]
async fn catalog_capabilities_match_implemented_sqlite_metadata() {
    let imported = import_connection_url("sqlite://:memory:", Some("capabilities")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    assert_eq!(
        database.catalog_capabilities(),
        expected_catalog_capabilities()
    );
    database.close().await;
}

#[test]
fn sqlite_mutation_capabilities_expose_native_table_and_view_creation() {
    let capabilities = lazydb::db::sqlite::SqliteAdapter::catalog_mutation_capabilities();
    assert!(capabilities.create.iter().any(|option| {
        option.object_type
            == lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::Table,
            )
    }));
    assert!(capabilities.create.iter().any(|option| {
        option.object_type
            == lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::View,
            )
    }));
    assert_eq!(capabilities.edit.len(), 2);
}

#[tokio::test]
async fn catalog_aliases_remain_connection_local_for_file_and_memory_profiles() {
    let fixture = CatalogFixture::new().await;
    assert_connection_local_alias(
        &fixture.database,
        fixture.profile_id,
        &fixture.configured_database,
        ATTACHED_ALIAS,
        "excluded_only",
    )
    .await;
    fixture.close().await;

    let imported = import_connection_url("sqlite://:memory:", Some("memory-alias")).unwrap();
    let profile_id = imported.profile.id;
    let configured_database = imported.profile.database.clone().unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute(
            r#"
            ATTACH DATABASE ':memory:' AS "MemoryAlias";
            CREATE TABLE "MemoryAlias".memory_object (id INTEGER PRIMARY KEY);
            "#,
        )
        .await
        .unwrap();
    assert_connection_local_alias(
        &database,
        profile_id,
        &configured_database,
        "MemoryAlias",
        "memory_object",
    )
    .await;
    database.close().await;
}

async fn assert_connection_local_alias(
    database: &DatabaseConnection,
    profile_id: Uuid,
    configured_database: &str,
    alias: &str,
    expected_object: &str,
) {
    let discoveries = join_all((0..6).map(|_| {
        let database = database.clone();
        async move { database.discover_catalog_scope().await }
    }))
    .await;
    for discovery in discoveries {
        let discovery = discovery.unwrap();
        assert_eq!(discovery.databases.len(), 1);
        assert!(
            discovery.databases[0]
                .schemas
                .iter()
                .any(|schema| schema == alias),
            "every pool acquisition must observe attached alias {alias}"
        );
    }

    let schema_id = CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        [configured_database.to_owned(), alias.to_owned()],
    );
    let target = CatalogTarget::objects(schema_id, ObjectGroup::Tables).unwrap();
    let scope = selected_scope(configured_database, &[alias]);
    let pages = join_all((0..6).map(|request_id| {
        let database = database.clone();
        let request = catalog_request(
            profile_id,
            target.clone(),
            scope.clone(),
            10,
            None,
            request_id,
        );
        async move { database.load_catalog_page(&request).await }
    }))
    .await;
    for page in pages {
        let page = page.unwrap();
        assert!(
            page.entries
                .iter()
                .any(|entry| entry.qualified_name.object == expected_object),
            "every paged request must observe objects in attached alias {alias}"
        );
    }
}

#[tokio::test]
async fn catalog_pages_apply_scope_and_return_exact_group_summaries() {
    let fixture = CatalogFixture::new().await;
    let scope = fixture.scope(&["main"]);

    let databases = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::Databases,
            scope.clone(),
            10,
            None,
            1,
        ))
        .await
        .unwrap();
    assert_eq!(databases.total_count, CatalogCount::Exact(1));
    assert_eq!(databases.entries.len(), 1);
    assert_eq!(databases.entries[0].id, fixture.database_id());
    assert_eq!(databases.entries[0].comment, OptionalMetadata::Unsupported);

    let schemas = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::schemas(fixture.database_id()).unwrap(),
            scope.clone(),
            10,
            None,
            2,
        ))
        .await
        .unwrap();
    assert_eq!(schemas.total_count, CatalogCount::Exact(1));
    assert_eq!(schemas.entries.len(), 1);
    assert_eq!(schemas.entries[0].qualified_name.object, "main");
    assert_eq!(schemas.entries[0].comment, OptionalMetadata::Unsupported);
    assert!(
        schemas
            .entries
            .iter()
            .all(|entry| entry.qualified_name.object != ATTACHED_ALIAS)
    );

    let groups = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::groups(fixture.schema_id("main")).unwrap(),
            scope.clone(),
            10,
            None,
            3,
        ))
        .await
        .unwrap();
    assert_eq!(groups.total_count, CatalogCount::Exact(3));
    assert_eq!(groups.completeness, CatalogCompleteness::Complete);
    assert!(groups.entries.is_empty());
    let counts = groups
        .group_summaries
        .iter()
        .map(|summary| (summary.group, summary.object_count))
        .collect::<HashMap<_, _>>();
    assert_eq!(
        counts.get(&ObjectGroup::Tables),
        Some(&CatalogCount::Exact(5))
    );
    assert_eq!(
        counts.get(&ObjectGroup::Views),
        Some(&CatalogCount::Exact(1))
    );
    assert_eq!(
        counts.get(&ObjectGroup::Triggers),
        Some(&CatalogCount::Exact(1))
    );

    let views = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Views).unwrap(),
            scope.clone(),
            10,
            None,
            4,
        ))
        .await
        .unwrap();
    assert_eq!(views.total_count, CatalogCount::Exact(1));
    assert_eq!(views.entries[0].kind, CatalogKind::View);
    assert_eq!(views.entries[0].qualified_name.object, "child_view");

    let triggers = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Triggers).unwrap(),
            scope,
            10,
            None,
            5,
        ))
        .await
        .unwrap();
    assert_eq!(triggers.total_count, CatalogCount::Exact(1));
    let trigger = &triggers.entries[0];
    assert_eq!(trigger.kind, CatalogKind::Trigger);
    assert_eq!(trigger.parent_id.as_ref(), Some(&fixture.schema_id("main")));
    assert_eq!(
        trigger.relation_id,
        Some(CatalogId::new(
            fixture.profile_id,
            CatalogKind::Table,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "child".to_owned(),
            ],
        ))
    );
    assert_eq!(trigger.comment, OptionalMetadata::Unsupported);

    let case_mismatched_scope = fixture.scope(&["archivecase"]);
    let case_mismatched_schema = fixture.schema_id("archivecase");
    let error = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(case_mismatched_schema, ObjectGroup::Tables).unwrap(),
            case_mismatched_scope,
            10,
            None,
            6,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));

    fixture.close().await;
}

#[tokio::test]
async fn catalog_object_pages_use_stable_binary_keyset_cursors() {
    let fixture = CatalogFixture::new().await;
    let (first_names, first_ids, completeness) = collect_table_pages(&fixture, 20).await;
    let (repeated_names, repeated_ids, _) = collect_table_pages(&fixture, 40).await;

    assert_eq!(
        first_names,
        ["Alpha", "beta", "child", "parent", "shared_name"]
    );
    assert_eq!(first_names, repeated_names);
    assert_eq!(first_ids, repeated_ids);
    assert_eq!(
        first_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        first_ids.len()
    );
    assert_eq!(
        completeness,
        [
            CatalogCompleteness::Partial,
            CatalogCompleteness::Partial,
            CatalogCompleteness::Partial,
            CatalogCompleteness::Partial,
            CatalogCompleteness::Complete,
        ]
    );

    fixture.close().await;
}

async fn collect_table_pages(
    fixture: &CatalogFixture,
    first_request_id: u64,
) -> (Vec<String>, Vec<CatalogId>, Vec<CatalogCompleteness>) {
    let target = CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Tables).unwrap();
    let mut cursor = None;
    let mut names = Vec::new();
    let mut ids = Vec::new();
    let mut completeness = Vec::new();

    for page_index in 0..10 {
        let request = catalog_request(
            fixture.profile_id,
            target.clone(),
            fixture.scope(&["main"]),
            1,
            cursor.clone(),
            first_request_id + page_index,
        );
        let page = fixture.database.load_catalog_page(&request).await.unwrap();
        assert_eq!(page.total_count, CatalogCount::Exact(5));
        assert_eq!(page.entries.len(), 1);
        let entry = &page.entries[0];
        assert_eq!(entry.comment, OptionalMetadata::Unsupported);
        assert_eq!(entry.qualified_name.schema.as_deref(), Some("main"));
        names.push(entry.qualified_name.object.clone());
        ids.push(entry.id.clone());
        completeness.push(page.completeness);

        cursor = page.next_cursor;
        if let Some(next) = cursor.as_ref() {
            let (sort_key, tie_breaker) = next.keyset_parts().unwrap();
            assert!(!sort_key.is_empty());
            assert!(!tie_breaker.is_empty());
        } else {
            break;
        }
    }

    (names, ids, completeness)
}

#[tokio::test]
async fn catalog_relation_children_are_structured_grouped_and_stable() {
    let fixture = CatalogFixture::new().await;
    let relation = CatalogId::new(
        fixture.profile_id,
        CatalogKind::Table,
        [
            fixture.configured_database.clone(),
            "main".to_owned(),
            "child".to_owned(),
        ],
    );
    let target = CatalogTarget::relation_children(relation.clone()).unwrap();
    let request = catalog_request(
        fixture.profile_id,
        target.clone(),
        fixture.scope(&["main"]),
        100,
        None,
        60,
    );
    let page = fixture.database.load_catalog_page(&request).await.unwrap();

    assert_eq!(page.total_count, CatalogCount::Exact(11));
    assert_eq!(page.entries.len(), 11);
    assert_eq!(page.completeness, CatalogCompleteness::Complete);
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.comment == OptionalMetadata::Unsupported)
    );
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.kind != CatalogKind::Trigger)
    );

    let columns = page
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Column(metadata) => {
                Some((entry.qualified_name.object.as_str(), metadata))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(columns.len(), 5);
    assert_column(
        columns["owner_id"],
        3,
        "INTEGER",
        false,
        OptionalMetadata::Supported(Some("7".to_owned())),
        false,
    );
    assert_column(
        columns["label"],
        4,
        "TEXT",
        false,
        OptionalMetadata::Supported(Some("'new'".to_owned())),
        false,
    );
    assert_column(
        columns["label_key"],
        5,
        "TEXT",
        true,
        OptionalMetadata::Supported(None),
        false,
    );
    assert_eq!(
        columns["label_key"].generated_expression,
        OptionalMetadata::Unsupported
    );

    let indexes = page
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Index(metadata) => Some((entry, metadata)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(indexes.len(), 3, "one entry is required per native index");
    let (_, lookup_index) = indexes
        .iter()
        .find(|(entry, _)| entry.qualified_name.object == "child_lookup_idx")
        .unwrap();
    assert_eq!(
        **lookup_index,
        IndexMetadata {
            columns: vec!["owner_id".to_owned(), "label".to_owned()],
            unique: false,
        }
    );

    let primary_key = constraint(&page.entries, CatalogKind::PrimaryKey);
    let unique = constraint(&page.entries, CatalogKind::UniqueConstraint);
    let foreign_key = constraint(&page.entries, CatalogKind::ForeignKey);
    assert_eq!(
        primary_key.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
            columns: vec!["tenant_id".to_owned(), "child_id".to_owned()],
        })
    );
    assert_eq!(
        unique.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::Unique {
            columns: vec!["tenant_id".to_owned(), "label".to_owned()],
        })
    );
    assert_eq!(
        foreign_key.metadata,
        CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
            columns: vec!["tenant_id".to_owned(), "owner_id".to_owned()],
            referenced_relation: lazydb::db::catalog::QualifiedName {
                database: Some(fixture.configured_database.clone()),
                schema: Some("main".to_owned()),
                object: "parent".to_owned(),
            },
            referenced_columns: vec!["tenant_id".to_owned(), "parent_id".to_owned()],
        })
    );

    assert_membership(columns["tenant_id"], &primary_key.id, 1);
    assert_membership(columns["tenant_id"], &unique.id, 1);
    assert_membership(columns["tenant_id"], &foreign_key.id, 1);
    assert_membership(columns["child_id"], &primary_key.id, 2);
    assert_membership(columns["label"], &unique.id, 2);
    assert_membership(columns["owner_id"], &foreign_key.id, 2);

    let repeated = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            target,
            fixture.scope(&["main"]),
            100,
            None,
            61,
        ))
        .await
        .unwrap();
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| &entry.id)
            .collect::<Vec<_>>(),
        repeated
            .entries
            .iter()
            .map(|entry| &entry.id)
            .collect::<Vec<_>>()
    );

    fixture.close().await;
}

#[tokio::test]
async fn catalog_primary_key_nullability_uses_sqlite_table_facts() {
    let fixture = CatalogFixture::new().await;

    let rowid_relation = CatalogId::new(
        fixture.profile_id,
        CatalogKind::Table,
        [
            fixture.configured_database.clone(),
            "main".to_owned(),
            "Alpha".to_owned(),
        ],
    );
    let rowid_page = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::relation_children(rowid_relation).unwrap(),
            fixture.scope(&["main"]),
            100,
            None,
            70,
        ))
        .await
        .unwrap();
    let rowid_key = column_metadata(&rowid_page.entries, "id");
    assert!(!rowid_key.nullable, "INTEGER PRIMARY KEY aliases rowid");
    assert_eq!(rowid_key.type_family, OptionalMetadata::Unsupported);

    let without_rowid_relation = CatalogId::new(
        fixture.profile_id,
        CatalogKind::Table,
        [
            fixture.configured_database.clone(),
            "main".to_owned(),
            "parent".to_owned(),
        ],
    );
    let without_rowid_page = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::relation_children(without_rowid_relation).unwrap(),
            fixture.scope(&["main"]),
            100,
            None,
            71,
        ))
        .await
        .unwrap();
    assert!(!column_metadata(&without_rowid_page.entries, "tenant_id").nullable);
    assert!(!column_metadata(&without_rowid_page.entries, "parent_id").nullable);

    fixture.close().await;
}

#[tokio::test]
async fn catalog_triggers_resolve_canonical_owners_and_exclude_cross_schema_temp() {
    let fixture = CatalogFixture::new().await;
    fixture
        .database
        .execute(
            r#"
            CREATE TABLE MixedOwner (id INTEGER PRIMARY KEY);
            CREATE TRIGGER mixed_table_guard BEFORE INSERT ON mixedowner
                BEGIN SELECT 1; END;
            CREATE VIEW MixedView AS SELECT id FROM MixedOwner;
            CREATE TRIGGER mixed_view_guard INSTEAD OF INSERT ON mixedview
                BEGIN SELECT 1; END;
            CREATE TEMP TRIGGER excluded_temp_guard BEFORE INSERT ON main.MixedOwner
                BEGIN SELECT 1; END;
            PRAGMA writable_schema = ON;
            INSERT INTO sqlite_schema (type, name, tbl_name, rootpage, sql)
                VALUES (
                    'view',
                    'mixedowner',
                    'mixedowner',
                    0,
                    'CREATE VIEW mixedowner AS SELECT 1 AS id'
                );
            PRAGMA writable_schema = OFF;
            "#,
        )
        .await
        .unwrap();

    let main_scope = fixture.scope(&["main"]);
    let main_groups = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::groups(fixture.schema_id("main")).unwrap(),
            main_scope.clone(),
            10,
            None,
            80,
        ))
        .await
        .unwrap();
    assert_eq!(
        group_count(&main_groups, ObjectGroup::Triggers),
        CatalogCount::Exact(3)
    );
    let triggers = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(fixture.schema_id("main"), ObjectGroup::Triggers).unwrap(),
            main_scope,
            10,
            None,
            81,
        ))
        .await
        .unwrap();
    assert_eq!(triggers.total_count, CatalogCount::Exact(3));
    assert_eq!(triggers.entries.len(), 3);
    let table_trigger = triggers
        .entries
        .iter()
        .find(|entry| entry.qualified_name.object == "mixed_table_guard")
        .unwrap();
    assert_eq!(
        table_trigger.relation_id,
        Some(CatalogId::new(
            fixture.profile_id,
            CatalogKind::Table,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "MixedOwner".to_owned(),
            ],
        ))
    );
    let view_trigger = triggers
        .entries
        .iter()
        .find(|entry| entry.qualified_name.object == "mixed_view_guard")
        .unwrap();
    assert_eq!(
        view_trigger.relation_id,
        Some(CatalogId::new(
            fixture.profile_id,
            CatalogKind::View,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "MixedView".to_owned(),
            ],
        ))
    );

    let temp_scope = fixture.scope(&["temp"]);
    let temp_schema = fixture.schema_id("temp");
    let temp_groups = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::groups(temp_schema.clone()).unwrap(),
            temp_scope.clone(),
            10,
            None,
            82,
        ))
        .await
        .unwrap();
    assert_eq!(
        group_count(&temp_groups, ObjectGroup::Triggers),
        CatalogCount::Exact(0)
    );
    let temp_triggers = fixture
        .database
        .load_catalog_page(&catalog_request(
            fixture.profile_id,
            CatalogTarget::objects(temp_schema, ObjectGroup::Triggers).unwrap(),
            temp_scope,
            10,
            None,
            83,
        ))
        .await
        .unwrap();
    assert_eq!(temp_triggers.total_count, CatalogCount::Exact(0));
    assert!(temp_triggers.entries.is_empty());

    fixture.close().await;
}

#[tokio::test]
async fn catalog_native_target_misses_use_not_found_errors() {
    let fixture = CatalogFixture::new().await;
    for (request_id, relation) in [
        CatalogId::new(
            fixture.profile_id,
            CatalogKind::Table,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "missing_relation".to_owned(),
            ],
        ),
        CatalogId::new(
            fixture.profile_id,
            CatalogKind::View,
            [
                fixture.configured_database.clone(),
                "main".to_owned(),
                "Alpha".to_owned(),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let error = fixture
            .database
            .load_catalog_page(&catalog_request(
                fixture.profile_id,
                CatalogTarget::relation_children(relation).unwrap(),
                fixture.scope(&["main"]),
                100,
                None,
                90 + request_id as u64,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));
    }

    fixture.close().await;
}

fn column_metadata<'a>(entries: &'a [CatalogEntry], name: &str) -> &'a ColumnMetadata {
    entries
        .iter()
        .find_map(|entry| match &entry.metadata {
            CatalogMetadata::Column(metadata) if entry.qualified_name.object == name => {
                Some(metadata)
            }
            _ => None,
        })
        .unwrap()
}

fn group_count(page: &lazydb::db::catalog::CatalogPage, group: ObjectGroup) -> CatalogCount {
    page.group_summaries
        .iter()
        .find(|summary| summary.group == group)
        .unwrap()
        .object_count
}

fn assert_column(
    column: &ColumnMetadata,
    ordinal_position: u32,
    native_type: &str,
    nullable: bool,
    default_expression: OptionalMetadata<String>,
    hidden: bool,
) {
    assert_eq!(column.ordinal_position, ordinal_position);
    assert_eq!(column.native_type, native_type);
    assert_eq!(column.type_family, OptionalMetadata::Unsupported);
    assert_eq!(column.nullable, nullable);
    assert_eq!(column.default_expression, default_expression);
    assert_eq!(column.identity, OptionalMetadata::Unsupported);
    assert_eq!(column.auto_increment, OptionalMetadata::Unsupported);
    assert_eq!(column.hidden, OptionalMetadata::Supported(Some(hidden)));
    assert_eq!(column.numeric_precision, OptionalMetadata::Unsupported);
    assert_eq!(column.numeric_scale, OptionalMetadata::Unsupported);
    assert_eq!(
        column.character_maximum_length,
        OptionalMetadata::Unsupported
    );
    assert_eq!(column.collation, OptionalMetadata::Unsupported);
    assert_eq!(column.character_set, OptionalMetadata::Unsupported);
}

fn constraint(entries: &[CatalogEntry], kind: CatalogKind) -> &CatalogEntry {
    let matches = entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one grouped {kind:?}");
    matches[0]
}

fn assert_membership(column: &ColumnMetadata, constraint_id: &CatalogId, ordinal_position: u32) {
    assert!(
        column
            .constraint_memberships
            .contains(&ConstraintMembership {
                constraint_id: constraint_id.clone(),
                ordinal_position,
            })
    );
}

#[tokio::test]
async fn catalog_object_ddl_uses_the_verified_schema_alias() {
    let fixture = CatalogFixture::new().await;

    let main = fixture
        .database
        .object_ddl(CatalogKind::Table, "main", "shared_name")
        .await
        .unwrap()
        .unwrap();
    let attached = fixture
        .database
        .object_ddl(CatalogKind::Table, ATTACHED_ALIAS, "shared_name")
        .await
        .unwrap()
        .unwrap();
    assert!(main.contains("main_value TEXT"));
    assert!(!main.contains("archive_value"));
    assert!(attached.contains("archive_value INTEGER"));
    assert!(!attached.contains("main_value"));

    let error = fixture
        .database
        .object_ddl(CatalogKind::Table, "archivecase", "shared_name")
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));

    fixture.close().await;
}

#[tokio::test]
async fn discovery_returns_the_configured_sqlite_database_and_ordered_aliases() {
    let imported = import_connection_url("sqlite://:memory:", Some("discovery")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    database
        .execute("ATTACH DATABASE ':memory:' AS analytics")
        .await
        .unwrap();

    assert_eq!(
        database.catalog_capabilities(),
        expected_catalog_capabilities()
    );
    let discovery = database.discover_catalog_scope().await.unwrap();
    assert_eq!(discovery.databases.len(), 1);
    assert_eq!(discovery.databases[0].name, ":memory:");
    assert_eq!(discovery.databases[0].schemas, ["main", "analytics"]);
    database.close().await;
}

#[tokio::test]
async fn probes_catalogs_queries_and_reads_ddl() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("catalog.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("catalog")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    database
        .execute(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE teams (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                team_id INTEGER REFERENCES teams(id),
                name TEXT NOT NULL,
                score REAL,
                payload BLOB
            );
            CREATE INDEX users_name_idx ON users(name);
            CREATE VIEW active_users AS SELECT id, name FROM users;
            CREATE TRIGGER users_name_guard BEFORE INSERT ON users
            WHEN NEW.name = '' BEGIN SELECT RAISE(ABORT, 'name required'); END;
            INSERT INTO teams VALUES (1, 'core');
            INSERT INTO users VALUES (1, 1, 'Ada', 9.5, X'0001FF');
            INSERT INTO users VALUES (2, NULL, 'Lin', NULL, NULL);
            "#,
        )
        .await
        .unwrap();

    let server = database.probe().await.unwrap();
    assert_eq!(
        server.database,
        ":memory:".replace(":memory:", &path.to_string_lossy())
    );
    assert!(server.version.chars().next().unwrap().is_ascii_digit());

    let outcome = database
        .execute("SELECT id, team_id, name, score, payload FROM users ORDER BY id")
        .await
        .unwrap();
    let result = outcome.result_sets.last().unwrap();
    assert_eq!(
        result
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "team_id", "name", "score", "payload"]
    );
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], CellValue::Integer(1));
    assert_eq!(result.rows[0][2], CellValue::Text("Ada".into()));
    assert_eq!(result.rows[0][3], CellValue::Float(9.5));
    assert_eq!(result.rows[0][4], CellValue::Bytes(vec![0, 1, 255]));
    assert_eq!(result.rows[1][1], CellValue::Null);
    assert_eq!(result.rows[1][3], CellValue::Null);

    let ddl = database
        .object_ddl(CatalogKind::Table, "main", "users")
        .await
        .unwrap()
        .unwrap();
    assert!(ddl.contains("CREATE TABLE users"));
    database.close().await;
}

#[tokio::test]
async fn enforces_sqlite_read_only_at_connection_level() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("readonly.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("readonly")).unwrap();
    let writable = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    writable
        .execute("CREATE TABLE records (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    writable.close().await;

    let mut read_only_profile = imported.profile;
    read_only_profile.read_only = true;
    let read_only = DatabaseConnection::connect(&read_only_profile, None)
        .await
        .unwrap();

    let error = read_only
        .execute("INSERT INTO records VALUES (1)")
        .await
        .unwrap_err();
    assert!(error.to_string().to_ascii_lowercase().contains("readonly"));
    read_only.close().await;
}

#[tokio::test]
async fn preserves_result_sets_counts_timing_empty_values_and_errors() {
    let imported = import_connection_url("sqlite://:memory:", Some("results")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    let outcome = database
        .execute("SELECT '' AS empty_value, NULL AS missing_value; SELECT 7 AS second_result")
        .await
        .unwrap();
    assert_eq!(outcome.result_sets.len(), 2);
    assert_eq!(outcome.stats.row_count, 2);
    assert!(outcome.stats.total() >= outcome.stats.execution);
    assert_eq!(
        outcome.result_sets[0].rows[0][0],
        CellValue::Text(String::new())
    );
    assert_eq!(outcome.result_sets[0].rows[0][1], CellValue::Null);
    assert_eq!(outcome.result_sets[1].rows[0][0], CellValue::Integer(7));

    database
        .execute("CREATE TABLE affected (value TEXT); INSERT INTO affected VALUES ('x'), ('y')")
        .await
        .unwrap();
    let affected = database
        .execute("UPDATE affected SET value = value")
        .await
        .unwrap();
    assert_eq!(affected.result_sets.last().unwrap().affected_rows, 2);

    let error = database
        .execute("SELECT * FROM missing_table")
        .await
        .unwrap_err();
    assert_eq!(error.category, lazydb::db::ErrorCategory::Sql);
    database.close().await;
}
