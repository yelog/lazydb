use lazydb::{
    action::Action,
    app::App,
    db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, ColumnMetadata, OptionalMetadata,
        QualifiedName,
    },
    model::workspace::ConnectionStatus,
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
    sql::{
        CompletionContext, CompletionIndex, CompletionKind, SqlDialect, complete, quote_identifier,
    },
};
use uuid::Uuid;

fn fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let database = CatalogId::new(connection, CatalogKind::Database, ["app"]);
    let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let table = CatalogId::new(connection, CatalogKind::Table, ["app", "public", "users"]);
    vec![
        CatalogEntry::database(
            database.clone(),
            qualified("app", None, "app"),
            "database",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::schema(
            schema.clone(),
            database,
            qualified("app", Some("public"), "public"),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::relation(
            table.clone(),
            schema,
            qualified("app", Some("public"), "users"),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::relation_child(
            CatalogId::new(
                connection,
                CatalogKind::Column,
                ["app", "public", "users", "odd name"],
            ),
            table,
            qualified("app", Some("public"), "odd name"),
            "column",
            OptionalMetadata::Unsupported,
            CatalogMetadata::Column(ColumnMetadata::new(1, "text\x1b[31m", true)),
        )
        .unwrap(),
    ]
}

fn compact_match_fixture() -> Vec<CatalogEntry> {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let schema = entries[1].id.clone();
    for name in ["sys_user", "sysuser_archive"] {
        entries.push(
            CatalogEntry::relation(
                CatalogId::new(connection, CatalogKind::Table, ["app", "public", name]),
                schema.clone(),
                qualified("app", Some("public"), name),
                "table",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
    }
    let users = entries[2].id.clone();
    entries.push(
        CatalogEntry::relation_child(
            CatalogId::new(
                connection,
                CatalogKind::Column,
                ["app", "public", "users", "user_id"],
            ),
            users,
            qualified("app", Some("public"), "user_id"),
            "column",
            OptionalMetadata::Unsupported,
            CatalogMetadata::Column(ColumnMetadata::new(2, "bigint", false)),
        )
        .unwrap(),
    );
    entries
}

#[test]
fn relation_completion_ignores_identifier_separators() {
    let index = CompletionIndex::new(&compact_match_fixture());
    let sql = "select * from sysuser";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Table
            && candidate.label == "sys_user"
            && candidate.insert_text == "sys_user"
    }));
    assert_eq!(
        candidates
            .iter()
            .filter(|candidate| candidate.label == "sys_user")
            .count(),
        1
    );
}

#[test]
fn ordinary_prefix_ranks_above_compact_prefix() {
    let index = CompletionIndex::new(&compact_match_fixture());
    let sql = "select * from sysuser";
    let labels = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    )
    .into_iter()
    .filter(|candidate| candidate.kind == CompletionKind::Table)
    .map(|candidate| candidate.label)
    .collect::<Vec<_>>();

    assert_eq!(labels[..2], ["sysuser_archive", "sys_user"]);
}

#[test]
fn alias_column_completion_ignores_identifier_separators() {
    let index = CompletionIndex::new(&compact_match_fixture());
    let sql = "select u.userid from users u";
    let candidates = complete(
        sql,
        "select u.userid".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Column && candidate.label == "user_id"
    }));
}

#[test]
fn completion_is_contextual_and_quotes_raw_names() {
    let index = CompletionIndex::new(&fixture());
    let candidates = complete(
        "select * from us",
        16,
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );
    assert!(candidates.iter().any(
        |candidate| candidate.kind == CompletionKind::Table && candidate.insert_text == "users"
    ));
    assert_eq!(
        quote_identifier("odd\" name", SqlDialect::Postgres),
        "\"odd\"\" name\""
    );
    assert_eq!(
        quote_identifier("odd`name", SqlDialect::MySql),
        "`odd``name`"
    );
}

#[test]
fn completion_includes_databases_and_qualified_children() {
    let index = CompletionIndex::new(&fixture());

    let databases = complete(
        "select * from ",
        14,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(databases.iter().any(|candidate| {
        candidate.kind == CompletionKind::Database && candidate.label == "app"
    }));

    let schema_text = "select * from app.";
    let schemas = complete(
        schema_text,
        schema_text.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(schemas.iter().any(|candidate| {
        candidate.kind == CompletionKind::Schema && candidate.label == "public"
    }));

    let table_text = "select * from app.public.";
    let tables = complete(
        table_text,
        table_text.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(tables.iter().any(|candidate| {
        candidate.kind == CompletionKind::Table
            && candidate.label == "users"
            && candidate.detail.as_deref() == Some("(app.public)")
    }));
}

#[test]
fn alias_column_completion_uses_relation_columns_and_native_type() {
    let index = CompletionIndex::new(&fixture());
    let candidates = complete(
        "select u. from users u",
        9,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let column = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Column)
        .expect("alias should resolve to table columns");
    assert_eq!(column.label, "odd name");
    assert_eq!(column.detail.as_deref(), Some("text<ESC>[31m"));
}

#[test]
fn unqualified_columns_are_limited_to_relations_in_current_statement() {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let other_schema = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let other_table = CatalogId::new(connection, CatalogKind::Table, ["app", "public", "roles"]);
    entries.push(
        CatalogEntry::relation(
            other_table.clone(),
            other_schema,
            qualified("app", Some("public"), "roles"),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
    );
    entries.push(
        CatalogEntry::relation_child(
            CatalogId::new(
                connection,
                CatalogKind::Column,
                ["app", "public", "roles", "user_id"],
            ),
            other_table,
            qualified("app", Some("public"), "user_id"),
            "column",
            OptionalMetadata::Unsupported,
            CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
        )
        .unwrap(),
    );
    let index = CompletionIndex::new(&entries);
    let candidates = complete(
        "select odd from users u",
        10,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "odd name")
    );
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.label == "user_id")
    );
}

#[test]
fn hostile_display_text_does_not_change_insertion() {
    let mut nodes = fixture();
    let hostile_profile = Uuid::new_v4();
    let hostile_schema = CatalogId::new(hostile_profile, CatalogKind::Schema, ["x", "public"]);
    nodes.push(
        CatalogEntry::relation(
            CatalogId::new(
                hostile_profile,
                CatalogKind::Table,
                ["x", "public", "\x1b[2J"],
            ),
            hostile_schema,
            qualified("x", Some("public"), "\x1b[2J"),
            "table",
            OptionalMetadata::Supported(Some("\x00detail".into())),
            false,
        )
        .unwrap(),
    );
    let index = CompletionIndex::new(&nodes);
    let candidates = complete(
        "from ",
        5,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.insert_text.contains("2J"))
        .unwrap();
    assert!(!candidate.label.contains('\x1b'));
    assert!(candidate.insert_text.contains("\x1b[2J"));
}

#[test]
fn general_sql_keywords_rank_before_matching_catalog_names() {
    let connection = Uuid::new_v4();
    let nodes = [("sales", "s"), ("data", "d"), ("facts", "f")]
        .into_iter()
        .map(|(name, _)| {
            CatalogEntry::relation(
                CatalogId::new(connection, CatalogKind::Table, ["app", "public", name]),
                CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]),
                qualified("app", Some("public"), name),
                "table",
                OptionalMetadata::Supported(None),
                false,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let index = CompletionIndex::new(&nodes);

    for (prefix, keyword) in [("s", "SELECT"), ("d", "DELETE"), ("f", "FROM")] {
        let candidates = complete(
            prefix,
            prefix.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(
            candidates.first().map(|candidate| candidate.label.as_str()),
            Some(keyword)
        );
        assert_eq!(
            candidates.first().map(|candidate| candidate.kind),
            Some(CompletionKind::Keyword)
        );
    }
}

#[test]
fn index_retains_only_completion_relevant_entries() {
    let mut entries = fixture();
    let profile = entries[0].id.profile_id();
    entries.push(
        CatalogEntry::object(
            CatalogId::new(profile, CatalogKind::Sequence, ["app", "public", "seq"]),
            CatalogId::new(profile, CatalogKind::Schema, ["app", "public"]),
            qualified("app", Some("public"), "seq"),
            "sequence",
            OptionalMetadata::Supported(None),
            false,
        )
        .unwrap(),
    );
    let index = CompletionIndex::new(&entries);
    assert!(index.entries().iter().all(|entry| matches!(
        entry.kind,
        CatalogKind::Database
            | CatalogKind::Schema
            | CatalogKind::Table
            | CatalogKind::View
            | CatalogKind::MaterializedView
            | CatalogKind::Column
            | CatalogKind::Function
            | CatalogKind::Procedure
    )));
}

#[test]
fn scoped_index_deduplicates_replaces_removed_entries_and_rejects_out_of_scope_entries() {
    let entries = fixture();
    let profile = entries[0].id.profile_id();
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".into(),
            schemas: CatalogSelection::Selected(vec!["public".into()]),
        }]),
    };
    let mut index = CompletionIndex::default();
    index.replace_scoped(&entries, &scope);
    index.append_scoped(&[entries[2].clone(), entries[3].clone()], &scope);

    assert_eq!(
        index
            .entries()
            .iter()
            .filter(|entry| entry.id == entries[2].id)
            .count(),
        1
    );
    assert!(index.entries().iter().all(|entry| {
        entry.kind == CatalogKind::Database
            || (entry.qualified_name.database.as_deref() == Some("app")
                && entry.qualified_name.schema.as_deref() == Some("public"))
    }));

    let other_database = CatalogEntry::relation(
        CatalogId::new(profile, CatalogKind::Table, ["other", "public", "orders"]),
        CatalogId::new(profile, CatalogKind::Schema, ["other", "public"]),
        qualified("other", Some("public"), "orders"),
        "table",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    index.append_scoped(&[other_database], &scope);
    assert!(
        !index
            .entries()
            .iter()
            .any(|entry| entry.qualified_name.database.as_deref() == Some("other"))
    );

    index.replace_scoped(&[entries[0].clone()], &scope);
    assert_eq!(index.entries().len(), 1);
    assert_eq!(index.entries()[0].kind, CatalogKind::Database);
}

#[test]
fn app_completion_prefers_the_active_console_target_schema() {
    let mut profile = import_connection_url("postgres://localhost/app", Some("app"))
        .unwrap()
        .profile;
    profile.default_schema = Some("audit".into());
    profile.catalog_scope = CatalogScope::for_profile(DatabaseKind::Postgres, "app", Some("audit"));
    let profile_id = profile.id;
    let entries = ["public", "audit"]
        .map(|schema| {
            CatalogEntry::relation(
                CatalogId::new(profile_id, CatalogKind::Table, ["app", schema, "orders"]),
                CatalogId::new(profile_id, CatalogKind::Schema, ["app", schema]),
                qualified("app", Some(schema), "orders"),
                "table",
                OptionalMetadata::Supported(None),
                false,
            )
            .unwrap()
        })
        .to_vec();
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.status = ConnectionStatus::Connected;
    app.active_console_mut()
        .execution_target
        .as_mut()
        .unwrap()
        .schema = Some("public".into());
    app.explorer.completion_index = CompletionIndex::new(&entries);
    app.update(Action::ReplaceEditor("select * from or".into()));
    app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('i'),
        crossterm::event::KeyModifiers::NONE,
    )));

    app.update(Action::CompletionExplicit);

    let popup = app.active_console().completion.as_ref().unwrap();
    let scores = popup
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.label == "orders" && candidate.detail.as_deref() == Some("(app.public)")
        })
        .map(|candidate| candidate.score.schema)
        .collect::<Vec<_>>();
    assert_eq!(scores, [1]);
}

#[test]
fn relation_completion_uses_shortest_target_relative_reference() {
    let connection = Uuid::new_v4();
    let database = CatalogId::new(connection, CatalogKind::Database, ["app"]);
    let public = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let mut entries = vec![
        CatalogEntry::database(
            database.clone(),
            qualified("app", None, "app"),
            "database",
            OptionalMetadata::Supported(None),
            false,
        )
        .unwrap(),
        CatalogEntry::schema(
            public.clone(),
            database,
            qualified("app", Some("public"), "public"),
            "schema",
            OptionalMetadata::Supported(None),
            false,
        )
        .unwrap(),
    ];
    entries.extend(
        [
            ("app", "public", "users"),
            ("app", "audit", "users"),
            ("analytics", "bi", "users"),
        ]
        .into_iter()
        .map(|(database, schema, object)| {
            CatalogEntry::relation(
                CatalogId::new(connection, CatalogKind::Table, [database, schema, object]),
                CatalogId::new(connection, CatalogKind::Schema, [database, schema]),
                qualified(database, Some(schema), object),
                "table",
                OptionalMetadata::Supported(None),
                false,
            )
            .unwrap()
        })
        .collect::<Vec<_>>(),
    );
    let index = CompletionIndex::new(&entries);
    let context = CompletionContext {
        database: Some("app"),
        schema: Some("public"),
    };
    let candidates = complete(
        "select * from us",
        16,
        SqlDialect::Postgres,
        &index,
        context,
    );
    let by_detail = |detail: &str| {
        candidates
            .iter()
            .find(|candidate| {
                candidate.label == "users" && candidate.detail.as_deref() == Some(detail)
            })
            .unwrap()
    };
    assert_eq!(by_detail("(app.public)").insert_text, "users");
    assert_eq!(by_detail("(app.audit)").insert_text, "audit.users");
    assert_eq!(
        by_detail("(analytics.bi)").insert_text,
        "analytics.bi.users"
    );

    let qualified_text = "select * from app.public.us";
    let qualified_candidates = complete(
        qualified_text,
        qualified_text.len(),
        SqlDialect::Postgres,
        &index,
        context,
    );
    assert_eq!(
        qualified_candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>(),
        ["users"]
    );
    assert_eq!(
        qualified_candidates[0].detail.as_deref(),
        Some("(app.public)")
    );
}

#[test]
fn relation_completion_deduplicates_mirrored_database_and_schema_detail() {
    let connection = Uuid::new_v4();
    let entry = CatalogEntry::relation(
        CatalogId::new(connection, CatalogKind::Table, ["app", "app", "users"]),
        CatalogId::new(connection, CatalogKind::Schema, ["app", "app"]),
        qualified("app", Some("app"), "users"),
        "table",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    let candidates = complete(
        "select * from us",
        16,
        SqlDialect::MySql,
        &CompletionIndex::new(&[entry]),
        CompletionContext::default(),
    );

    assert_eq!(candidates[0].label, "users");
    assert_eq!(candidates[0].detail.as_deref(), Some("(app)"));
}

fn qualified(database: &str, schema: Option<&str>, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.into()),
        schema: schema.map(str::to_owned),
        object: object.into(),
    }
}
