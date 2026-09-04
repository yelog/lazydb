use lazydb::{
    db::{
        catalog::CatalogTarget,
        catalog::{
            CatalogEntry, CatalogId, CatalogKind, ObjectGroup, OptionalMetadata, QualifiedName,
        },
        catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationAvailability, CatalogMutationCapabilities,
            CatalogMutationError, CatalogMutationExecutionMode, CatalogMutationMode,
            CatalogMutationOption, CatalogMutationPlan, CatalogMutationRequest,
            CatalogMutationTarget, CatalogObjectDefinitionRequest, CatalogObjectType,
            CatalogSelectionHint,
        },
    },
    identity::ConnectionIdentity,
    model::catalog_editor::{
        CatalogDraft, ColumnDraft, ConstraintDraft, DatabaseDraft, DraftRowState,
        MaterializedViewDraft, RoleDraft, SchemaDraft, TableDraft, ViewDraft,
    },
    model::text_input::TextInput,
};
use uuid::Uuid;

fn id(profile: Uuid, kind: CatalogKind, path: &[&str]) -> CatalogId {
    CatalogId::new(profile, kind, path.iter().copied())
}

#[test]
fn postgres_role_create_redacts_password_and_plans_attributes_memberships_and_comment() {
    let profile = Uuid::new_v4();
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        7,
        3,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Profile {
            profile_id: profile,
        },
        CatalogObjectType::LoginRole,
    )
    .unwrap()
    .with_current_database("app");
    let mut draft = RoleDraft::new(true);
    draft.name = "alice".into();
    draft.superuser = true;
    draft.connection_limit = "12".into();
    draft.memberships = "reporting, analysts".into();
    draft.comment = "service account".into();
    draft.set_password("never-display-this");
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::Role(draft),
        None,
    )
    .unwrap();
    assert!(plan.sql().contains("CREATE ROLE \"alice\" LOGIN SUPERUSER"));
    assert!(plan.sql().contains("PASSWORD '<REDACTED>'"));
    assert!(!plan.sql().contains("never-display-this"));
    assert!(format!("{plan:?}").contains("<redacted>"));
    assert!(!format!("{plan:?}").contains("never-display-this"));
    assert!(plan.sql().contains("GRANT \"alice\" TO \"reporting\""));
    assert_eq!(plan.execution_target.database(), "app");
    assert_eq!(plan.execution_target.execution_target(profile).schema, None);
}

#[test]
fn postgres_role_edit_blank_password_is_unchanged_and_membership_diff_is_planned() {
    use lazydb::db::catalog_mutation::{CatalogObjectDefinition, RoleDefinition};
    let profile = Uuid::new_v4();
    let object = id(profile, CatalogKind::Database, &["__role__", "alice"]);
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        8,
        3,
        CatalogMutationMode::Edit,
        CatalogMutationAnchor::Catalog(object),
        CatalogObjectType::LoginRole,
    )
    .unwrap();
    let baseline = RoleDefinition {
        name: "alice".into(),
        login: true,
        superuser: false,
        createdb: false,
        createrole: false,
        inherit: true,
        replication: false,
        bypass_rls: false,
        connection_limit: -1,
        valid_until: OptionalMetadata::Supported(Some("infinity".into())),
        memberships: vec!["old_group".into()],
        comment: OptionalMetadata::Supported(None),
        baseline_fingerprint: "sha256:role".into(),
    };
    let mut draft = RoleDraft::from_definition(&baseline);
    draft.memberships = "new_group".into();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::Role(draft),
        Some(CatalogObjectDefinition::Role(baseline)),
    )
    .unwrap();
    assert!(!plan.sql().contains("PASSWORD"));
    assert!(plan.sql().contains("GRANT \"alice\" TO \"new_group\""));
    assert!(plan.sql().contains("REVOKE \"alice\" FROM \"old_group\""));
}

#[test]
fn postgres_database_create_is_autocommit_and_refreshes_databases() {
    let profile = Uuid::new_v4();
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        1,
        2,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Profile {
            profile_id: profile,
        },
        CatalogObjectType::Catalog(CatalogKind::Database),
    )
    .unwrap();
    let mut draft = DatabaseDraft::new("");
    draft.name = "app".into();
    draft.owner = "postgres".into();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::Database(draft),
        None,
    )
    .unwrap();
    assert_eq!(
        plan.execution_mode,
        CatalogMutationExecutionMode::Autocommit
    );
    assert_eq!(plan.refresh, vec![CatalogTarget::Databases]);
    assert!(plan.sql().starts_with("CREATE DATABASE \"app\""));
}

#[test]
fn postgres_database_rename_rejects_current_database() {
    let profile = Uuid::new_v4();
    let id = id(profile, CatalogKind::Database, &["app"]);
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        1,
        2,
        CatalogMutationMode::Edit,
        CatalogMutationAnchor::Catalog(id),
        CatalogObjectType::Catalog(CatalogKind::Database),
    )
    .unwrap()
    .with_current_database("app");
    let definition = lazydb::db::catalog_mutation::DatabaseDefinition {
        name: "app".into(),
        owner: "postgres".into(),
        template: "template0".into(),
        encoding: "UTF8".into(),
        locale_provider: "libc".into(),
        locale: "C".into(),
        collation: "C".into(),
        ctype: "C".into(),
        tablespace: "pg_default".into(),
        connection_limit: -1,
        allow_connections: true,
        is_template: false,
        comment: OptionalMetadata::Supported(None),
        baseline_fingerprint: "sha256:app".into(),
    };
    let mut draft = DatabaseDraft::from_definition(&definition);
    draft.name = "new_app".into();
    assert!(
        lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
            request,
            CatalogDraft::Database(draft),
            Some(lazydb::db::catalog_mutation::CatalogObjectDefinition::Database(definition))
        )
        .is_err()
    );
}

#[test]
fn catalog_mutation_target_rejects_empty_databases_and_preserves_target_kind() {
    let profile = Uuid::new_v4();
    let target = lazydb::model::execution_target::ExecutionTarget {
        profile_id: profile,
        database: "app".into(),
        schema: Some("public".into()),
    };
    assert_eq!(
        CatalogMutationTarget::database_target(target.clone())
            .unwrap()
            .database(),
        "app"
    );
    assert_eq!(
        CatalogMutationTarget::maintenance("postgres")
            .unwrap()
            .database(),
        "postgres"
    );
    assert!(CatalogMutationTarget::maintenance("").is_err());
    assert!(
        CatalogMutationTarget::database_target(lazydb::model::execution_target::ExecutionTarget {
            profile_id: profile,
            database: String::new(),
            schema: None,
        })
        .is_err()
    );
}

#[test]
fn postgres_materialized_view_capability_hides_incomplete_index_creation() {
    let profile = Uuid::new_v4();
    let object = id(
        profile,
        CatalogKind::MaterializedView,
        &["app", "public", "mv", "42"],
    );
    let options = lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities()
        .create_options(&CatalogMutationAnchor::Catalog(object), None)
        .unwrap();
    assert!(options.is_empty());
}

#[test]
fn postgres_materialized_view_draft_disables_query_edits_after_loading() {
    use lazydb::db::catalog_mutation::MaterializedViewDefinition;
    let draft = MaterializedViewDraft::from_definition(&MaterializedViewDefinition {
        database: "app".into(),
        schema: "public".into(),
        name: "mv".into(),
        owner: "alice".into(),
        comment: OptionalMetadata::Supported(Some("note".into())),
        query: "SELECT 1".into(),
        tablespace: OptionalMetadata::Supported(None),
        populated: false,
        baseline_fingerprint: "sha256:x".into(),
    });
    assert!(!draft.query_editable);
    assert!(!draft.with_data);
}

#[test]
fn postgres_materialized_view_create_plan_refreshes_materialized_view_group_and_reports_impact() {
    use lazydb::db::catalog_mutation::{
        CatalogMutationAnchor, CatalogMutationMode, CatalogMutationRequest,
        CatalogObjectDefinition, MaterializedViewDefinition,
    };
    let profile = Uuid::new_v4();
    let schema = id(profile, CatalogKind::Schema, &["app", "reporting"]);
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        17,
        3,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(schema),
        CatalogObjectType::Catalog(CatalogKind::MaterializedView),
    )
    .unwrap();
    let draft = MaterializedViewDraft {
        name: "daily".into(),
        schema: "reporting".into(),
        owner: "alice".into(),
        comment: "daily report".into(),
        query: "SELECT 1".into(),
        tablespace: "fast".into(),
        with_data: false,
        selected_field: 0,
        query_editable: true,
    };
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::MaterializedView(draft),
        None,
    )
    .unwrap();
    assert!(plan.refresh.contains(&CatalogTarget::Objects {
        schema: id(profile, CatalogKind::Schema, &["app", "reporting"]),
        group: ObjectGroup::MaterializedViews,
    }));
    assert_eq!(
        plan.impact.namespace.schema.as_ref().unwrap().native_path,
        vec!["app".to_owned(), "reporting".to_owned()]
    );
    assert!(!plan.impact.native_identity_changed);
    assert!(plan.sql().contains("WITH NO DATA"));
    let _ = CatalogObjectDefinition::MaterializedView(MaterializedViewDefinition {
        database: "app".into(),
        schema: "reporting".into(),
        name: "daily".into(),
        owner: "alice".into(),
        comment: OptionalMetadata::Supported(None),
        query: "SELECT 1".into(),
        tablespace: OptionalMetadata::Supported(None),
        populated: false,
        baseline_fingerprint: "sha256:x".into(),
    });
}

#[test]
fn postgres_constraint_draft_rejects_fk_count_and_duplicate_columns() {
    use lazydb::db::catalog_mutation::ConstraintDefinitionKind;
    use lazydb::model::catalog_editor::ConstraintDraft;
    let mut draft = ConstraintDraft::new(
        ConstraintDefinitionKind::ForeignKey {
            columns: vec![],
            referenced_schema: "public".into(),
            referenced_relation: "accounts".into(),
            referenced_columns: vec![],
            match_type: "SIMPLE".into(),
            on_update: "NO ACTION".into(),
            on_delete: "NO ACTION".into(),
        },
        "public",
        "events",
    );
    draft.columns = "account_id, account_id".into();
    draft.referenced_columns = "id".into();
    assert!(draft.validate().is_err());
}

fn constraint_request(
    profile: Uuid,
    kind: CatalogKind,
    mode: CatalogMutationMode,
) -> CatalogMutationRequest {
    CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        99,
        7,
        mode,
        CatalogMutationAnchor::Catalog(id(profile, kind, &["app", "public", "events", "42", "7"])),
        CatalogObjectType::Catalog(kind),
    )
    .unwrap()
}

#[test]
fn postgres_constraint_planner_quotes_create_and_marks_structural_edits_destructive() {
    use lazydb::db::catalog_mutation::{ConstraintDefinition, ConstraintDefinitionKind};
    let profile = Uuid::new_v4();
    let mut draft = ConstraintDraft::new(
        ConstraintDefinitionKind::Unique { columns: vec![] },
        "public",
        "events",
    );
    draft.database = "app".into();
    draft.name = "odd\"constraint".into();
    draft.columns = "user_id, created_at".into();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        constraint_request(
            profile,
            CatalogKind::UniqueConstraint,
            CatalogMutationMode::Create,
        ),
        CatalogDraft::Constraint(draft.clone()),
        None,
    )
    .unwrap();
    assert!(
        plan.sql()
            .contains("ADD CONSTRAINT \"odd\"\"constraint\" UNIQUE (\"user_id\", \"created_at\")")
    );
    assert!(!plan.destructive);
    let mut unnamed = draft.clone();
    unnamed.name = "".into();
    let unnamed_plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        constraint_request(
            profile,
            CatalogKind::UniqueConstraint,
            CatalogMutationMode::Create,
        ),
        CatalogDraft::Constraint(unnamed),
        None,
    )
    .unwrap();
    assert!(
        unnamed_plan
            .sql()
            .contains("ALTER TABLE \"public\".\"events\" ADD UNIQUE")
    );

    let baseline =
        lazydb::db::catalog_mutation::CatalogObjectDefinition::Constraint(ConstraintDefinition {
            database: "app".into(),
            schema: "public".into(),
            relation: "events".into(),
            relation_kind: CatalogKind::Table,
            name: "old_uq".into(),
            kind: ConstraintDefinitionKind::Unique {
                columns: vec!["user_id".into()],
            },
            deferrable: false,
            initially_deferred: false,
            validated: true,
            comment: OptionalMetadata::Supported(None),
            baseline_fingerprint: "sha256:c".into(),
        });
    draft.columns = "email".into();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        constraint_request(
            profile,
            CatalogKind::UniqueConstraint,
            CatalogMutationMode::Edit,
        ),
        CatalogDraft::Constraint(draft),
        Some(baseline),
    )
    .unwrap();
    assert!(plan.destructive);
    assert!(plan.sql().contains("DROP CONSTRAINT \"old_uq\""));
}

#[test]
fn postgres_constraint_planner_emits_validate_without_drop_add() {
    use lazydb::db::catalog_mutation::{ConstraintDefinition, ConstraintDefinitionKind};
    let profile = Uuid::new_v4();
    let mut draft = ConstraintDraft::new(
        ConstraintDefinitionKind::Check {
            expression: "price > 0".into(),
            no_inherit: false,
        },
        "public",
        "events",
    );
    draft.database = "app".into();
    draft.name = "price_ck".into();
    let baseline =
        lazydb::db::catalog_mutation::CatalogObjectDefinition::Constraint(ConstraintDefinition {
            database: "app".into(),
            schema: "public".into(),
            relation: "events".into(),
            relation_kind: CatalogKind::Table,
            name: "price_ck".into(),
            kind: ConstraintDefinitionKind::Check {
                expression: "price > 0".into(),
                no_inherit: false,
            },
            deferrable: false,
            initially_deferred: false,
            validated: false,
            comment: OptionalMetadata::Supported(None),
            baseline_fingerprint: "sha256:c".into(),
        });
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        constraint_request(
            profile,
            CatalogKind::CheckConstraint,
            CatalogMutationMode::Edit,
        ),
        CatalogDraft::Constraint(draft),
        Some(baseline),
    )
    .unwrap();
    assert_eq!(
        plan.sql(),
        "ALTER TABLE \"public\".\"events\" VALIDATE CONSTRAINT \"price_ck\""
    );
    assert!(!plan.destructive);
}

#[test]
fn postgres_constraint_create_emits_not_valid_only_when_drafted() {
    use lazydb::db::catalog_mutation::ConstraintDefinitionKind;
    let profile = Uuid::new_v4();
    let mut draft = ConstraintDraft::new(
        ConstraintDefinitionKind::Check {
            expression: "price > 0".into(),
            no_inherit: false,
        },
        "public",
        "events",
    );
    draft.database = "app".into();
    draft.name = "price_ck".into();
    draft.not_valid = true;
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        constraint_request(
            profile,
            CatalogKind::CheckConstraint,
            CatalogMutationMode::Create,
        ),
        CatalogDraft::Constraint(draft),
        None,
    )
    .unwrap();
    assert!(plan.sql().ends_with("NOT VALID"));
}

#[test]
fn postgres_index_capabilities_cover_table_and_materialized_view() {
    let profile = Uuid::new_v4();
    let caps = lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities();
    for kind in [CatalogKind::Table, CatalogKind::MaterializedView] {
        let object = id(profile, kind, &["app", "public", "items", "1"]);
        let options = caps
            .create_options(&CatalogMutationAnchor::Catalog(object), None)
            .unwrap();
        assert!(!options.contains(&CatalogObjectType::Catalog(CatalogKind::Index)));
    }
}

#[test]
fn postgres_schema_create_options_include_every_implemented_schema_object() {
    let profile = Uuid::new_v4();
    let schema = id(profile, CatalogKind::Schema, &["app", "public"]);
    let capabilities = lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities();
    let options = capabilities
        .create_options(&CatalogMutationAnchor::Catalog(schema.clone()), None)
        .unwrap();

    assert_eq!(
        options,
        vec![
            CatalogObjectType::Catalog(CatalogKind::Table),
            CatalogObjectType::Catalog(CatalogKind::View),
            CatalogObjectType::Catalog(CatalogKind::MaterializedView),
            CatalogObjectType::Catalog(CatalogKind::Sequence),
        ]
    );

    for (group, kind) in [
        (ObjectGroup::Tables, CatalogKind::Table),
        (ObjectGroup::Views, CatalogKind::View),
        (
            ObjectGroup::MaterializedViews,
            CatalogKind::MaterializedView,
        ),
        (ObjectGroup::Sequences, CatalogKind::Sequence),
    ] {
        assert_eq!(
            capabilities
                .create_options(
                    &CatalogMutationAnchor::Group {
                        schema: schema.clone(),
                        group,
                    },
                    None,
                )
                .unwrap(),
            vec![CatalogObjectType::Catalog(kind)]
        );
    }
}

#[test]
fn postgres_create_capabilities_hide_incomplete_column_and_index_flows() {
    let capabilities = lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities();
    for kind in [CatalogKind::Column, CatalogKind::Index] {
        assert!(matches!(
            capabilities.create_availability(CatalogObjectType::Catalog(kind)),
            Some(CatalogMutationAvailability::Unavailable { .. })
        ));
        assert_eq!(
            capabilities.edit_availability(CatalogObjectType::Catalog(kind)),
            Some(CatalogMutationAvailability::Available)
        );
    }
}

fn index_draft(name: &str, expression: &str) -> CatalogDraft {
    CatalogDraft::Index(lazydb::model::catalog_editor::IndexDraft {
        name: name.into(),
        schema: "public".into(),
        relation: "events".into(),
        unique: false,
        access_method: "btree".into(),
        columns: vec![lazydb::model::catalog_editor::IndexColumnDraft {
            expression: expression.into(),
            descending: false,
            nulls_first: false,
            is_expression: false,
        }],
        include_columns: "".into(),
        predicate: "".into(),
        tablespace: "".into(),
    })
}

#[test]
fn postgres_index_mutation_uses_safe_create_and_rename_plans() {
    let profile = Uuid::new_v4();
    let relation = id(
        profile,
        CatalogKind::Table,
        &["app", "public", "events", "42"],
    );
    let create_request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        1,
        1,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(relation.clone()),
        CatalogObjectType::Catalog(CatalogKind::Index),
    )
    .unwrap();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        create_request,
        index_draft("odd\"idx", "name"),
        None,
    )
    .unwrap();
    assert_eq!(
        plan.sql(),
        "CREATE INDEX \"odd\"\"idx\" ON \"public\".\"events\" USING \"btree\" (\"name\" ASC NULLS LAST)"
    );
    assert!(!plan.destructive);

    let index = id(
        profile,
        CatalogKind::Index,
        &["app", "public", "events", "42", "1"],
    );
    let edit_request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        2,
        1,
        CatalogMutationMode::Edit,
        CatalogMutationAnchor::Catalog(index),
        CatalogObjectType::Catalog(CatalogKind::Index),
    )
    .unwrap();
    let baseline = lazydb::db::catalog_mutation::CatalogObjectDefinition::Index(
        lazydb::db::catalog_mutation::IndexDefinition {
            database: "app".into(),
            schema: "public".into(),
            relation: "events".into(),
            relation_kind: CatalogKind::Table,
            name: "old_idx".into(),
            unique: false,
            access_method: "btree".into(),
            columns: vec![lazydb::db::catalog_mutation::IndexColumnDefinition {
                expression: "name".into(),
                descending: false,
                nulls_first: false,
                is_expression: false,
            }],
            include_columns: vec![],
            predicate: OptionalMetadata::Supported(None),
            tablespace: OptionalMetadata::Supported(None),
            baseline_fingerprint: "sha256:x".into(),
        },
    );
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        edit_request,
        index_draft("new_idx", "name"),
        Some(baseline),
    )
    .unwrap();
    assert_eq!(
        plan.sql(),
        "ALTER INDEX \"public\".\"old_idx\" RENAME TO \"new_idx\""
    );
    assert!(!plan.destructive);
}

#[test]
fn mutation_protocol_validates_definition_requests_and_plans() {
    let profile = Uuid::new_v4();
    let connection = ConnectionIdentity {
        profile_id: profile,
        generation: 1,
    };
    let schema = id(profile, CatalogKind::Schema, &["app", "public"]);
    let request = CatalogMutationRequest::new(
        connection,
        3,
        9,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Profile {
            profile_id: profile,
        },
        CatalogObjectType::Catalog(CatalogKind::Database),
    )
    .unwrap();
    let definition_request = CatalogObjectDefinitionRequest {
        connection,
        request_id: 4,
        catalog_epoch: 9,
        object: schema.clone(),
        target: lazydb::model::execution_target::ExecutionTarget {
            profile_id: profile,
            database: "app".into(),
            schema: Some("public".into()),
        },
    };
    assert!(definition_request.validate().is_ok());

    let plan = CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(CatalogKind::Database),
        CatalogMutationExecutionMode::Transactional,
        CatalogMutationTarget::maintenance("postgres").unwrap(),
        vec![CatalogTarget::Databases],
        CatalogSelectionHint::Object(id(profile, CatalogKind::Database, &["app"])),
        Some("baseline".into()),
        Vec::new(),
        vec!["CREATE DATABASE app".into()],
    )
    .unwrap();
    assert_eq!(plan.statements(), &["CREATE DATABASE app"]);
    assert_eq!(plan.sql(), "CREATE DATABASE app");
    assert_eq!(plan.baseline_fingerprint.as_deref(), Some("baseline"));

    let empty = CatalogMutationPlan::new(
        plan.request.clone(),
        plan.object_type,
        plan.execution_mode,
        CatalogMutationTarget::maintenance("postgres").unwrap(),
        plan.refresh.clone(),
        plan.selection.clone(),
        None,
        Vec::new(),
        Vec::new(),
    );
    assert!(matches!(
        empty,
        Err(CatalogMutationError::InvalidPlan { .. })
    ));
}

#[test]
fn mutation_plan_validates_execution_target_invariants() {
    let profile = Uuid::new_v4();
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        1,
        1,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Profile {
            profile_id: profile,
        },
        CatalogObjectType::Catalog(CatalogKind::Database),
    )
    .unwrap();
    let target = |profile_id: Uuid, database: &str| {
        CatalogMutationTarget::database_target(lazydb::model::execution_target::ExecutionTarget {
            profile_id,
            database: database.into(),
            schema: None,
        })
    };
    let make_plan = |target, refresh| {
        CatalogMutationPlan::new(
            request.clone(),
            CatalogObjectType::Catalog(CatalogKind::Database),
            CatalogMutationExecutionMode::Transactional,
            target,
            refresh,
            CatalogSelectionHint::Parent(CatalogTarget::Databases),
            None,
            Vec::new(),
            vec!["CREATE DATABASE app".into()],
        )
    };

    assert!(matches!(
        make_plan(
            target(Uuid::new_v4(), "app").unwrap(),
            vec![CatalogTarget::Databases]
        ),
        Err(CatalogMutationError::ProfileMismatch { .. })
    ));
    assert!(target(profile, "").is_err());
    assert!(
        make_plan(
            target(profile, "app").unwrap(),
            vec![CatalogTarget::Objects {
                schema: id(profile, CatalogKind::Schema, &["app", "public"]),
                group: ObjectGroup::Tables,
            }]
        )
        .is_ok()
    );
}

fn entry(id: CatalogId, parent_id: Option<CatalogId>) -> CatalogEntry {
    CatalogEntry {
        kind: id.kind,
        id,
        parent_id,
        native_kind: "test".into(),
        qualified_name: QualifiedName {
            database: None,
            schema: None,
            object: "object".into(),
        },
        comment: OptionalMetadata::Unsupported,
        metadata: Default::default(),
        expandable: false,
        relation_id: None,
    }
}

#[test]
fn mutation_model_anchors_and_request_validation_preserve_identity() {
    let profile = Uuid::new_v4();
    let connection = ConnectionIdentity {
        profile_id: profile,
        generation: 1,
    };
    let catalog = id(profile, CatalogKind::Database, &["app"]);
    let schema = id(profile, CatalogKind::Schema, &["app", "public"]);

    assert_eq!(
        CatalogMutationAnchor::Profile {
            profile_id: profile
        },
        CatalogMutationAnchor::Profile {
            profile_id: profile
        }
    );
    assert_eq!(
        CatalogMutationAnchor::Catalog(catalog.clone()),
        CatalogMutationAnchor::Catalog(catalog.clone())
    );
    assert_eq!(
        CatalogMutationAnchor::Group {
            schema: schema.clone(),
            group: ObjectGroup::Tables
        },
        CatalogMutationAnchor::Group {
            schema,
            group: ObjectGroup::Tables
        }
    );
    assert!(
        CatalogMutationRequest::new(
            connection,
            1,
            2,
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Catalog(catalog),
            CatalogObjectType::Catalog(CatalogKind::Schema)
        )
        .is_ok()
    );
}

#[test]
fn mutation_model_request_rejects_profile_and_group_mismatches() {
    let profile = Uuid::new_v4();
    let other = Uuid::new_v4();
    let connection = ConnectionIdentity {
        profile_id: profile,
        generation: 1,
    };
    assert!(matches!(
        CatalogMutationRequest::new(
            connection,
            1,
            0,
            CatalogMutationMode::Edit,
            CatalogMutationAnchor::Catalog(id(other, CatalogKind::Table, &["x"])),
            CatalogObjectType::Catalog(CatalogKind::Table)
        ),
        Err(CatalogMutationError::ProfileMismatch { .. })
    ));
    assert!(matches!(
        CatalogMutationRequest::new(
            connection,
            1,
            0,
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Group {
                schema: id(profile, CatalogKind::Table, &["x"]),
                group: ObjectGroup::Tables
            },
            CatalogObjectType::Catalog(CatalogKind::Table)
        ),
        Err(CatalogMutationError::InvalidAnchor { .. })
    ));
}

#[test]
fn mutation_model_capabilities_have_labels_and_validate_selection() {
    let profile = Uuid::new_v4();
    let schema = id(profile, CatalogKind::Schema, &["app", "public"]);
    let table = entry(
        id(profile, CatalogKind::Table, &["app", "public", "users"]),
        Some(schema.clone()),
    );
    let capabilities = CatalogMutationCapabilities {
        profile_create: vec![],
        create: vec![CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Table),
            availability: CatalogMutationAvailability::Available,
        }],
        edit: vec![CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(CatalogKind::Table),
            availability: CatalogMutationAvailability::Available,
        }],
        view_options: Default::default(),
    };
    assert_eq!(
        CatalogObjectType::Catalog(CatalogKind::MaterializedView).display_label(),
        "Materialized View"
    );
    assert_eq!(
        capabilities
            .create_options(
                &CatalogMutationAnchor::Group {
                    schema: schema.clone(),
                    group: ObjectGroup::Tables
                },
                Some(&table)
            )
            .unwrap(),
        vec![CatalogObjectType::Catalog(CatalogKind::Table)]
    );
    assert!(
        capabilities
            .can_edit(
                &CatalogMutationAnchor::Catalog(table.id.clone()),
                Some(&table)
            )
            .unwrap()
    );
    assert!(
        capabilities
            .create_options(
                &CatalogMutationAnchor::Group {
                    schema,
                    group: ObjectGroup::Views
                },
                Some(&table)
            )
            .is_err()
    );
    assert!(
        CatalogMutationCapabilities::default()
            .create_options(
                &CatalogMutationAnchor::Profile {
                    profile_id: profile
                },
                None
            )
            .unwrap()
            .is_empty()
    );
}

fn schema_request(
    profile: Uuid,
    mode: CatalogMutationMode,
    anchor: CatalogMutationAnchor,
) -> CatalogMutationRequest {
    CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        9,
        4,
        mode,
        anchor,
        CatalogObjectType::Catalog(CatalogKind::Schema),
    )
    .unwrap()
}

fn schema_draft(name: &str, owner: &str, comment: &str) -> CatalogDraft {
    CatalogDraft::Schema(SchemaDraft {
        name: TextInput::from(name),
        owner: TextInput::from(owner),
        comment: TextInput::from(comment),
        selected_field: 0,
    })
}

#[test]
fn postgres_schema_create_quotes_identifiers_and_literals() {
    let profile = Uuid::new_v4();
    let request = schema_request(
        profile,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(id(profile, CatalogKind::Database, &["app"])),
    );
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        schema_draft(
            "odd\"schema",
            "role\"owner",
            "Robert's; DROP SCHEMA public;",
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        plan.statements(),
        &[
            "CREATE SCHEMA \"odd\"\"schema\" AUTHORIZATION \"role\"\"owner\"",
            "COMMENT ON SCHEMA \"odd\"\"schema\" IS 'Robert''s; DROP SCHEMA public;'",
        ]
    );
    assert_eq!(
        plan.execution_mode,
        CatalogMutationExecutionMode::Transactional
    );
    assert_eq!(
        plan.refresh,
        vec![CatalogTarget::Schemas {
            database: id(profile, CatalogKind::Database, &["app"])
        }]
    );
    assert_eq!(
        plan.selection,
        CatalogSelectionHint::Object(id(profile, CatalogKind::Schema, &["app", "odd\"schema"]))
    );
    assert_eq!(plan.execution_target.execution_target(profile).schema, None);
}

#[test]
fn postgres_schema_edit_plans_rename_owner_and_comment_changes() {
    let profile = Uuid::new_v4();
    let schema_id = id(profile, CatalogKind::Schema, &["app", "old"]);
    let request = schema_request(
        profile,
        CatalogMutationMode::Edit,
        CatalogMutationAnchor::Catalog(schema_id),
    );
    let baseline = lazydb::db::catalog_mutation::CatalogObjectDefinition::Schema(
        lazydb::db::catalog_mutation::SchemaDefinition {
            database: "app".into(),
            name: "old".into(),
            owner: "old_owner".into(),
            comment: OptionalMetadata::Supported(Some("old comment".into())),
            baseline_fingerprint: "sha256:test".into(),
        },
    );
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        schema_draft("new", "new_owner", "new 'comment'"),
        Some(baseline),
    )
    .unwrap();
    assert_eq!(
        plan.sql(),
        "ALTER SCHEMA \"old\" RENAME TO \"new\"\nALTER SCHEMA \"new\" OWNER TO \"new_owner\"\nCOMMENT ON SCHEMA \"new\" IS 'new ''comment'''"
    );
    assert_eq!(plan.execution_target.execution_target(profile).schema, None);
}

#[test]
fn postgres_schema_edit_rejects_noop_and_can_remove_comment() {
    let profile = Uuid::new_v4();
    let request = schema_request(
        profile,
        CatalogMutationMode::Edit,
        CatalogMutationAnchor::Catalog(id(profile, CatalogKind::Schema, &["app", "same"])),
    );
    let baseline = lazydb::db::catalog_mutation::CatalogObjectDefinition::Schema(
        lazydb::db::catalog_mutation::SchemaDefinition {
            database: "app".into(),
            name: "same".into(),
            owner: "owner".into(),
            comment: OptionalMetadata::Supported(None),
            baseline_fingerprint: "sha256:test".into(),
        },
    );
    assert!(matches!(
        lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
            request.clone(),
            schema_draft("same", "owner", ""),
            Some(baseline.clone())
        ),
        Err(CatalogMutationError::NoChanges)
    ));
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        schema_draft("same", "owner", ""),
        Some(
            lazydb::db::catalog_mutation::CatalogObjectDefinition::Schema(
                lazydb::db::catalog_mutation::SchemaDefinition {
                    database: "app".into(),
                    name: "same".into(),
                    owner: "owner".into(),
                    comment: OptionalMetadata::Supported(Some("remove".into())),
                    baseline_fingerprint: "sha256:test".into(),
                },
            ),
        ),
    )
    .unwrap();
    assert_eq!(plan.sql(), "COMMENT ON SCHEMA \"same\" IS NULL");
}

#[test]
fn postgres_quoting_helpers_keep_identifiers_and_literals_distinct() {
    assert_eq!(lazydb::db::postgres::quote_identifier("a\"b"), "\"a\"\"b\"");
    assert_eq!(lazydb::db::postgres::quote_literal("a'b"), "'a''b'");
    assert_ne!(
        lazydb::db::postgres::quote_identifier("comment' OR 1=1 --"),
        lazydb::db::postgres::quote_literal("comment' OR 1=1 --")
    );
}

#[test]
fn postgres_view_draft_rejects_trailing_statements_and_plans_safe_replace() {
    let profile = Uuid::new_v4();
    let schema = id(profile, CatalogKind::Schema, &["app", "public"]);
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        4,
        2,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(schema),
        CatalogObjectType::Catalog(CatalogKind::View),
    )
    .unwrap();
    let mut draft = ViewDraft {
        name: "odd\"view".into(),
        schema: "public".into(),
        owner: "role\"owner".into(),
        comment: "owner's note".into(),
        query: "SELECT 1; SELECT 2".into(),
        output_columns: "one, two".into(),
        security_barrier: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        security_invoker: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        check_option: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        selected_field: 0,
    };
    assert!(draft.validate().is_err());
    draft.query = "SELECT 1".into();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::View(draft),
        None,
    )
    .unwrap();
    assert!(
        plan.sql()
            .contains("CREATE VIEW \"public\".\"odd\"\"view\" (\"one\", \"two\") AS SELECT 1")
    );
    assert!(
        plan.sql()
            .contains("COMMENT ON VIEW \"public\".\"odd\"\"view\" IS 'owner''s note'")
    );
}

#[test]
fn postgres_view_options_are_version_gated_and_render_exact_syntax() {
    let caps =
        lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(140_000);
    assert!(!caps.view_options.security_invoker.is_available());
    assert!(caps.view_options.security_barrier.is_available());
    let mut draft = ViewDraft {
        name: "v".into(),
        schema: "public".into(),
        owner: "owner".into(),
        comment: "".into(),
        query: "SELECT 1".into(),
        output_columns: "".into(),
        security_barrier: lazydb::db::catalog_mutation::ViewOption::available(Some(true)),
        security_invoker: lazydb::db::catalog_mutation::ViewOption::unavailable("not supported"),
        check_option: lazydb::db::catalog_mutation::ViewOption::available(Some("LOCAL".into())),
        selected_field: 0,
    };
    let profile = Uuid::new_v4();
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        1,
        1,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(id(profile, CatalogKind::Schema, &["app", "public"])),
        CatalogObjectType::Catalog(CatalogKind::View),
    )
    .unwrap();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation_for_version(
        request,
        CatalogDraft::View(draft.clone()),
        None,
        140_000,
    )
    .unwrap();
    assert!(
        plan.sql()
            .contains("WITH (security_barrier=true) AS SELECT 1 LOCAL CHECK OPTION")
    );
    draft.security_invoker = lazydb::db::catalog_mutation::ViewOption::available(Some(true));
    assert!(
        lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation_for_version(
            CatalogMutationRequest::new(
                ConnectionIdentity {
                    profile_id: profile,
                    generation: 1
                },
                2,
                1,
                CatalogMutationMode::Create,
                CatalogMutationAnchor::Catalog(id(
                    profile,
                    CatalogKind::Schema,
                    &["app", "public"]
                )),
                CatalogObjectType::Catalog(CatalogKind::View)
            )
            .unwrap(),
            CatalogDraft::View(draft),
            None,
            140_000
        )
        .is_err()
    );
}

fn table_draft(name: &str, columns: Vec<ColumnDraft>) -> CatalogDraft {
    CatalogDraft::Table(TableDraft {
        name: name.into(),
        schema: "public".into(),
        owner: "owner".into(),
        comment: "table comment".into(),
        columns,
        selected_column: 0,
        focus: lazydb::model::catalog_editor::TableEditorFocus::Columns,
        column_editor: None,
        indexes: vec![],
        constraints: vec![],
    })
}

fn column(name: &str, existing_name: Option<&str>) -> ColumnDraft {
    ColumnDraft {
        row_id: Uuid::new_v4(),
        ordinal_position: 1,
        existing_name: existing_name.map(str::to_owned),
        name: name.into(),
        native_type: "integer".into(),
        nullable: false,
        default_expression: "nextval('seq')".into(),
        identity: false,
        generated_expression: "".into(),
        collation: "".into(),
        comment: "".into(),
        state: existing_name.map_or(DraftRowState::Added, |_| DraftRowState::Existing {
            id: id(Uuid::nil(), CatalogKind::Column, &[name]),
        }),
    }
}

#[test]
fn postgres_table_create_and_edit_plan_is_quoted_ordered_and_destructive_when_needed() {
    let profile = Uuid::new_v4();
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        1,
        1,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Group {
            schema: id(profile, CatalogKind::Schema, &["app", "public"]),
            group: ObjectGroup::Tables,
        },
        CatalogObjectType::Catalog(CatalogKind::Table),
    )
    .unwrap();
    let plan = lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(
        request,
        table_draft("odd\"table", vec![column("id", None), column("name", None)]),
        None,
    )
    .unwrap();
    assert!(plan.refresh.contains(&CatalogTarget::Groups {
        schema: id(profile, CatalogKind::Schema, &["app", "public"]),
    }));
    assert_eq!(
        plan.statements()[0],
        "CREATE TABLE \"public\".\"odd\"\"table\" (\"id\" integer DEFAULT nextval('seq') NOT NULL, \"name\" integer DEFAULT nextval('seq') NOT NULL)"
    );
    assert!(!plan.destructive);
}

#[test]
fn sequence_bounds_keep_unset_distinct_from_no_limit_and_validate_numbers() {
    use lazydb::db::catalog_mutation::SequenceBound;
    let draft = lazydb::model::catalog_editor::SequenceDraft {
        name: "events_id_seq".into(),
        schema: "public".into(),
        owner: "postgres".into(),
        comment: "".into(),
        data_type: "bigint".into(),
        increment: "1".into(),
        min_value: SequenceBound::Unset,
        max_value: SequenceBound::NoLimit,
        start_value: "1".into(),
        restart_value: "".into(),
        cache: "1".into(),
        cycle: false,
        owned_by: "NONE".into(),
        selected_field: 0,
    };
    assert!(draft.validate().is_ok());
    assert!(matches!(draft.min_value, SequenceBound::Unset));
    let mut invalid = draft;
    invalid.cache = "not-a-number".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn sequence_create_plan_quotes_owned_by_and_has_no_child_options() {
    use lazydb::db::catalog_mutation::{SequenceBound, SequenceDefinition};
    let profile = Uuid::new_v4();
    let schema = id(profile, CatalogKind::Schema, &["app", "public"]);
    let request = CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id: profile,
            generation: 1,
        },
        17,
        1,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(schema.clone()),
        CatalogObjectType::Catalog(CatalogKind::Sequence),
    )
    .unwrap();
    let draft = CatalogDraft::Sequence(lazydb::model::catalog_editor::SequenceDraft {
        name: "seq".into(),
        schema: "public".into(),
        owner: "postgres".into(),
        comment: "note".into(),
        data_type: "bigint".into(),
        increment: "2".into(),
        min_value: SequenceBound::NoLimit,
        max_value: SequenceBound::Unset,
        start_value: "5".into(),
        restart_value: "".into(),
        cache: "10".into(),
        cycle: true,
        owned_by: "public.events.id".into(),
        selected_field: 0,
    });
    let plan =
        lazydb::db::postgres::PostgresAdapter::plan_catalog_mutation(request, draft, None).unwrap();
    assert!(plan.sql().contains("OWNED BY \"public\".\"events\".\"id\""));
    assert!(
        lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities()
            .create_options(
                &CatalogMutationAnchor::Catalog(id(
                    profile,
                    CatalogKind::Sequence,
                    &["app", "public", "seq", "1"]
                )),
                None
            )
            .unwrap()
            .is_empty()
    );
    let _ = SequenceDefinition {
        database: "app".into(),
        schema: "public".into(),
        name: "seq".into(),
        owner: "postgres".into(),
        comment: OptionalMetadata::Supported(None),
        data_type: "bigint".into(),
        increment: "1".into(),
        min_value: SequenceBound::Unset,
        max_value: SequenceBound::Unset,
        start_value: "1".into(),
        cache: "1".into(),
        cycle: false,
        owned_by: None,
        baseline_fingerprint: "x".into(),
    };
}
