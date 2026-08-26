use lazydb::{
    action::Action,
    app::App,
    db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, ColumnMetadata, OptionalMetadata,
        QualifiedName,
    },
    model::workspace::ConnectionStatus,
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
    sql::{CompletionIndex, CompletionKind, SqlDialect, complete, quote_identifier},
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

#[test]
fn completion_is_contextual_and_quotes_raw_names() {
    let index = CompletionIndex::new(&fixture());
    let candidates = complete(
        "select * from us",
        16,
        SqlDialect::Postgres,
        &index,
        Some("public"),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Table
                && candidate.insert_text == "\"users\"")
    );
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
    let candidates = complete("from ", 5, SqlDialect::Postgres, &index, None);
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
        let candidates = complete(prefix, prefix.len(), SqlDialect::Postgres, &index, None);
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
        CatalogKind::Schema
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
        entry.qualified_name.database.as_deref() == Some("app")
            && entry.qualified_name.schema.as_deref() == Some("public")
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
    assert_eq!(index.entries().len(), 0);
}

#[test]
fn app_completion_uses_the_active_profiles_default_schema() {
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
    app.explorer.completion_index = CompletionIndex::new(&entries);
    app.update(Action::ReplaceEditor("select * from or".into()));

    app.update(Action::CompletionExplicit);

    let popup = app.active_console().completion.as_ref().unwrap();
    assert!(
        popup
            .candidates
            .iter()
            .any(|candidate| candidate.score.schema == 1)
    );
}

fn qualified(database: &str, schema: Option<&str>, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.into()),
        schema: schema.map(str::to_owned),
        object: object.into(),
    }
}
