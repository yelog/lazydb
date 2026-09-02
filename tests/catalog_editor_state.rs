use lazydb::{
    db::catalog_mutation::{
        CatalogMutationAnchor, CatalogMutationExecutionMode, CatalogMutationMode,
        CatalogMutationRequest, CatalogObjectType, CatalogSelectionHint,
    },
    identity::ConnectionIdentity,
    model::catalog_editor::{
        CatalogDraft, CatalogEditorOperation, CatalogEditorPage, CatalogEditorSection,
        CatalogEditorState, CatalogMutationOption, CatalogMutationPlan, DatabaseDraft,
        MaterializedViewDraft, SchemaDraft, TableDraft,
    },
    model::text_input::TextInput,
};
use uuid::Uuid;

fn state() -> CatalogEditorState {
    CatalogEditorState::new(
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(lazydb::db::catalog::CatalogId::new(
            profile(),
            lazydb::db::catalog::CatalogKind::Database,
            ["app"],
        )),
        7,
        vec![CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(lazydb::db::catalog::CatalogKind::Schema),
            label: "Schema".into(),
        }],
    )
}

#[test]
fn database_draft_keeps_creation_options_display_only_after_loading() {
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
        connection_limit: 10,
        allow_connections: true,
        is_template: false,
        comment: lazydb::db::catalog::OptionalMetadata::Supported(None),
        baseline_fingerprint: "sha256:db".into(),
    };
    let mut draft = DatabaseDraft::from_definition(&definition);
    draft.selected_field = 2;
    draft.insert('x');
    assert_eq!(draft.template.value(), "template0");
    draft.selected_field = 9;
    draft.insert('0');
    assert_eq!(draft.connection_limit.value(), "100");
}

fn profile() -> Uuid {
    Uuid::from_u128(1)
}

#[test]
fn constraint_create_selection_initializes_typed_draft_and_field_focus() {
    let profile = profile();
    let mut editor = CatalogEditorState::new(
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(lazydb::db::catalog::CatalogId::new(
            profile,
            lazydb::db::catalog::CatalogKind::Table,
            ["app", "public", "events", "1"],
        )),
        1,
        vec![CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(lazydb::db::catalog::CatalogKind::ForeignKey),
            label: "Foreign Key".into(),
        }],
    );
    assert!(editor.select_option(0));
    let Some(CatalogDraft::Constraint(mut draft)) = editor.draft else {
        panic!("constraint draft expected")
    };
    draft.move_field(1);
    draft.insert('x');
    assert_eq!(draft.columns.value(), "x");
}

#[test]
fn picker_loading_and_form_transitions() {
    let mut editor = state();
    assert_eq!(editor.page, CatalogEditorPage::ObjectPicker);
    assert!(editor.select_option(0));
    assert_eq!(editor.page, CatalogEditorPage::Form);
    assert!(!editor.begin_loading(1));

    editor.page = CatalogEditorPage::Loading;
    assert!(editor.begin_loading(1));
    assert_eq!(
        editor.operation,
        Some(CatalogEditorOperation::LoadingDefinition { request_id: 1 })
    );
    assert!(editor.finish_loading(1, None));
    assert_eq!(editor.page, CatalogEditorPage::Form);
}

#[test]
fn profile_role_picker_initializes_login_and_non_login_drafts() {
    let profile = profile();
    for (object_type, login) in [
        (CatalogObjectType::LoginRole, true),
        (CatalogObjectType::Role, false),
    ] {
        let mut editor = CatalogEditorState::new(
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Profile {
                profile_id: profile,
            },
            1,
            vec![CatalogMutationOption {
                object_type,
                label: object_type.display_label().into(),
            }],
        );
        assert!(editor.select_option(0));
        assert!(
            matches!(editor.draft, Some(CatalogDraft::Role(ref draft)) if draft.login == login)
        );
    }
}

#[test]
fn schema_draft_rejects_blank_name_and_owner() {
    let draft = SchemaDraft {
        name: TextInput::from("  "),
        owner: TextInput::from("postgres"),
        comment: TextInput::default(),
    };
    assert!(draft.validate().is_err());

    let draft = SchemaDraft {
        name: TextInput::from("events"),
        owner: TextInput::from("\t"),
        comment: TextInput::default(),
    };
    assert!(draft.validate().is_err());
}

#[test]
fn materialized_view_edit_draft_keeps_query_display_only() {
    let definition = lazydb::db::catalog_mutation::MaterializedViewDefinition {
        database: "app".into(),
        schema: "public".into(),
        name: "mv".into(),
        owner: "postgres".into(),
        comment: lazydb::db::catalog::OptionalMetadata::Supported(Some("note".into())),
        query: "SELECT 1".into(),
        tablespace: lazydb::db::catalog::OptionalMetadata::Supported(Some("fast".into())),
        populated: true,
        baseline_fingerprint: "sha256:mv".into(),
    };
    let mut draft = MaterializedViewDraft::from_definition(&definition);
    draft.selected_field = 4;
    draft.insert('x');
    assert_eq!(draft.query.value(), "SELECT 1");
    draft.selected_field = 5;
    draft.insert('x');
    assert_eq!(draft.tablespace.value(), "fastx");
}

#[test]
fn form_preview_apply_and_cancel_transitions() {
    let mut editor = state();
    editor.select_option(0);
    editor.draft = Some(CatalogDraft::Schema(SchemaDraft {
        name: TextInput::from("events"),
        owner: TextInput::from("postgres"),
        comment: TextInput::default(),
    }));
    assert!(editor.begin_planning(2));
    let connection = ConnectionIdentity {
        profile_id: profile(),
        generation: 3,
    };
    let request = CatalogMutationRequest::new(
        connection,
        2,
        7,
        CatalogMutationMode::Create,
        editor.anchor.clone(),
        CatalogObjectType::Catalog(lazydb::db::catalog::CatalogKind::Schema),
    )
    .unwrap();
    let plan = CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(lazydb::db::catalog::CatalogKind::Schema),
        CatalogMutationExecutionMode::Transactional,
        vec![lazydb::db::catalog::CatalogTarget::Databases],
        CatalogSelectionHint::Parent(lazydb::db::catalog::CatalogTarget::Databases),
        None,
        Vec::new(),
        vec!["CREATE SCHEMA events".into()],
    )
    .unwrap();
    assert!(editor.plan_ready(2, plan));
    assert_eq!(editor.page, CatalogEditorPage::SqlPreview);
    assert!(editor.begin_apply(2));
    assert!(editor.cancel());
    assert!(editor.apply_succeeded(2, connection, 7));
    assert!(!editor.is_busy());

    let editor = state();
    assert!(editor.cancel());
}

#[test]
fn stale_responses_and_validation_preserve_form() {
    let mut editor = state();
    editor.select_option(0);
    editor.draft = Some(CatalogDraft::Schema(SchemaDraft {
        name: TextInput::from("kept"),
        owner: TextInput::from("postgres"),
        comment: TextInput::default(),
    }));
    assert!(editor.begin_planning(4));
    let connection = ConnectionIdentity {
        profile_id: profile(),
        generation: 1,
    };
    let request = CatalogMutationRequest::new(
        connection,
        4,
        8,
        CatalogMutationMode::Create,
        editor.anchor.clone(),
        CatalogObjectType::Catalog(lazydb::db::catalog::CatalogKind::Schema),
    )
    .unwrap();
    let plan = CatalogMutationPlan::new(
        request,
        CatalogObjectType::Catalog(lazydb::db::catalog::CatalogKind::Schema),
        CatalogMutationExecutionMode::Transactional,
        vec![lazydb::db::catalog::CatalogTarget::Databases],
        CatalogSelectionHint::Parent(lazydb::db::catalog::CatalogTarget::Databases),
        None,
        Vec::new(),
        vec!["stale".into()],
    )
    .unwrap();
    assert!(!editor.plan_ready(4, plan));
    assert_eq!(
        editor.operation,
        Some(CatalogEditorOperation::Planning { request_id: 4 })
    );
    assert!(editor.planning_failed(4, "name is required"));
    assert_eq!(editor.page, CatalogEditorPage::Form);
    assert_eq!(editor.error.as_deref(), Some("name is required"));
    let Some(CatalogDraft::Schema(draft)) = editor.draft else {
        panic!("expected schema draft");
    };
    assert_eq!(draft.name.value(), "kept");
}

#[test]
fn table_draft_keeps_stable_row_ids_and_focuses_columns() {
    let definition = lazydb::db::catalog_mutation::TableDefinition {
        database: "app".into(),
        schema: "public".into(),
        name: "events".into(),
        owner: "postgres".into(),
        comment: lazydb::db::catalog::OptionalMetadata::Supported(None),
        columns: vec![lazydb::db::catalog_mutation::ColumnDefinition {
            name: "id".into(),
            ordinal_position: 1,
            native_type: "integer".into(),
            nullable: false,
            default_expression: lazydb::db::catalog::OptionalMetadata::Supported(None),
            identity: lazydb::db::catalog::OptionalMetadata::Supported(Some(true)),
            generated_expression: lazydb::db::catalog::OptionalMetadata::Supported(None),
            collation: lazydb::db::catalog::OptionalMetadata::Supported(None),
            comment: lazydb::db::catalog::OptionalMetadata::Supported(None),
        }],
        indexes: vec![],
        constraints: vec![],
        baseline_fingerprint: "sha256:x".into(),
    };
    let mut draft = TableDraft::from_definition(&definition);
    let row_id = draft.columns[0].row_id;
    assert_eq!(draft.selected_section, CatalogEditorSection::General);
    draft.select_section(1);
    draft.selected_column = 0;
    assert_eq!(draft.selected_section, CatalogEditorSection::Columns);
    assert_eq!(draft.columns[0].row_id, row_id);
}

#[test]
fn index_draft_preserves_definition_fields() {
    let definition = lazydb::db::catalog_mutation::IndexDefinition {
        database: "app".into(),
        schema: "public".into(),
        relation: "events".into(),
        relation_kind: lazydb::db::catalog::CatalogKind::Table,
        name: "events_name_idx".into(),
        unique: true,
        access_method: "btree".into(),
        columns: vec![lazydb::db::catalog_mutation::IndexColumnDefinition {
            expression: "name".into(),
            descending: true,
            nulls_first: true,
            is_expression: false,
        }],
        include_columns: vec!["id".into()],
        predicate: lazydb::db::catalog::OptionalMetadata::Supported(Some("active".into())),
        tablespace: lazydb::db::catalog::OptionalMetadata::Supported(Some("fast".into())),
        baseline_fingerprint: "sha256:x".into(),
    };
    let draft = lazydb::model::catalog_editor::IndexDraft::from_definition(&definition);
    assert!(draft.unique);
    assert_eq!(draft.columns[0].expression.value(), "name");
    assert_eq!(draft.include_columns.value(), "id");
    assert!(draft.validate().is_ok());
}

#[test]
fn index_section_is_available_in_editor_state() {
    let definition = lazydb::db::catalog_mutation::IndexDefinition {
        database: "app".into(),
        schema: "public".into(),
        relation: "events".into(),
        relation_kind: lazydb::db::catalog::CatalogKind::Table,
        name: "events_idx".into(),
        unique: false,
        access_method: "btree".into(),
        columns: vec![],
        include_columns: vec![],
        predicate: lazydb::db::catalog::OptionalMetadata::Supported(None),
        tablespace: lazydb::db::catalog::OptionalMetadata::Supported(None),
        baseline_fingerprint: "x".into(),
    };
    let draft = lazydb::model::catalog_editor::IndexDraft::from_definition(&definition);
    assert_eq!(draft.schema.value(), "public");
}

#[test]
fn view_draft_preserves_query_and_cycles_editable_fields() {
    use lazydb::model::catalog_editor::ViewDraft;
    let mut draft = ViewDraft {
        name: "v".into(),
        schema: "public".into(),
        owner: "postgres".into(),
        comment: "note".into(),
        query: "SELECT 1".into(),
        output_columns: "value".into(),
        security_barrier: lazydb::db::catalog_mutation::ViewOption::available(Some(false)),
        security_invoker: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        check_option: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        selected_field: 0,
    };
    draft.move_field(4);
    draft.insert(' ');
    assert_eq!(draft.query.value(), "SELECT 1 ");
    draft.move_field(1);
    assert_eq!(draft.selected_field, 5);
}

#[test]
fn view_query_validation_ignores_semicolons_in_literals_and_comments() {
    use lazydb::model::catalog_editor::ViewDraft;
    let mut draft = ViewDraft {
        name: "v".into(),
        schema: "public".into(),
        owner: "postgres".into(),
        comment: "".into(),
        query: "SELECT 'a;b' AS value -- ;\n".into(),
        output_columns: "value".into(),
        security_barrier: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        security_invoker: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        check_option: lazydb::db::catalog_mutation::ViewOption::unavailable("not tested"),
        selected_field: 0,
    };
    assert!(draft.validate().is_ok());
    draft.query = "SELECT 1; SELECT 2".into();
    assert!(draft.validate().is_err());
}

#[test]
fn sequence_draft_cycles_all_fields_and_preserves_no_limit_state() {
    let mut draft = lazydb::model::catalog_editor::SequenceDraft {
        name: "seq".into(),
        schema: "public".into(),
        owner: "owner".into(),
        comment: "".into(),
        data_type: "bigint".into(),
        increment: "1".into(),
        min_value: lazydb::db::catalog_mutation::SequenceBound::NoLimit,
        max_value: lazydb::db::catalog_mutation::SequenceBound::Unset,
        start_value: "1".into(),
        restart_value: "".into(),
        cache: "1".into(),
        cycle: false,
        owned_by: "NONE".into(),
        selected_field: 0,
    };
    draft.move_field(-1);
    draft.insert('x');
    assert_eq!(draft.owned_by.value(), "NONEx");
    assert!(matches!(
        draft.min_value,
        lazydb::db::catalog_mutation::SequenceBound::NoLimit
    ));
}
