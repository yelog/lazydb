use lazydb::{
    action::Action,
    app::App,
    db::ServerInfo,
    db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, ColumnMetadata, OptionalMetadata,
        QualifiedName,
    },
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
    sql::{
        CompletionCandidate, CompletionContext, CompletionIndex, CompletionKind, SqlDialect,
        complete, quote_identifier, relation_ids_for_completion, should_offer_completion,
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

fn contextual_fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let mut entries = Vec::new();
    for (table_name, columns) in [
        (
            "sys_user",
            &[
                "update_time",
                "update_user",
                "update_user_phone",
                "user_type",
                "username",
            ][..],
        ),
        ("user_agreement_accept", &["agreement_id"][..]),
        ("unit_mtmm_capacity", &["capacity_id"][..]),
    ] {
        let table = CatalogId::new(
            connection,
            CatalogKind::Table,
            ["app", "public", table_name],
        );
        entries.push(
            CatalogEntry::relation(
                table.clone(),
                schema.clone(),
                qualified("app", Some("public"), table_name),
                "table",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
        entries.extend(columns.iter().enumerate().map(|(position, column)| {
            CatalogEntry::relation_child(
                CatalogId::new(
                    connection,
                    CatalogKind::Column,
                    ["app", "public", table_name, column],
                ),
                table.clone(),
                qualified("app", Some("public"), column),
                "column",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(position as u32 + 1, "text", true)),
            )
            .unwrap()
        }));
    }
    entries
}

fn labels(candidates: &[CompletionCandidate]) -> Vec<&str> {
    candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect()
}

fn multi_relation_fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let mut entries = Vec::new();
    for (table_name, columns) in [
        ("users", &["id", "user_name"][..]),
        ("roles", &["id", "role_name"][..]),
        ("audit_log", &["audit_message"][..]),
    ] {
        let table = CatalogId::new(
            connection,
            CatalogKind::Table,
            ["app", "public", table_name],
        );
        entries.push(
            CatalogEntry::relation(
                table.clone(),
                schema.clone(),
                qualified("app", Some("public"), table_name),
                "table",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
        entries.extend(columns.iter().enumerate().map(|(position, column)| {
            CatalogEntry::relation_child(
                CatalogId::new(
                    connection,
                    CatalogKind::Column,
                    ["app", "public", table_name, column],
                ),
                table.clone(),
                qualified("app", Some("public"), column),
                "column",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(position as u32 + 1, "text", true)),
            )
            .unwrap()
        }));
    }
    entries
}

#[test]
fn select_expression_excludes_relation_candidates() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "select u from sys_user";
    let candidates = complete(
        sql,
        "select u".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Column && candidate.label == "username"
    }));
    assert!(candidates.iter().all(|candidate| !matches!(
        candidate.kind,
        CompletionKind::Database
            | CompletionKind::Schema
            | CompletionKind::Table
            | CompletionKind::View
    )));
}

#[test]
fn where_expression_excludes_relation_candidates() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "select * from sys_user\nwhere ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Column && candidate.label == "update_time"
    }));
    assert!(candidates.iter().all(|candidate| !matches!(
        candidate.kind,
        CompletionKind::Database
            | CompletionKind::Schema
            | CompletionKind::Table
            | CompletionKind::View
    )));
}

#[test]
fn missing_columns_do_not_fall_back_to_global_relations() {
    let mut entries = contextual_fixture();
    entries.retain(|entry| entry.kind != CatalogKind::Column);
    let index = CompletionIndex::new(&entries);
    let sql = "select * from sys_user where ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().all(|candidate| !matches!(
        candidate.kind,
        CompletionKind::Database
            | CompletionKind::Schema
            | CompletionKind::Table
            | CompletionKind::View
    )));
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Keyword)
    );
}

#[test]
fn statement_and_expression_keywords_are_contextual() {
    let index = CompletionIndex::new(&contextual_fixture());
    let statement = complete(
        "u",
        1,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        statement
            .iter()
            .any(|candidate| candidate.label == "UPDATE")
    );

    let projection_sql = "select u from sys_user";
    let projection = complete(
        projection_sql,
        "select u".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        !projection
            .iter()
            .any(|candidate| candidate.label == "UPDATE")
    );
}

#[test]
fn insert_completion_offers_only_into_keyword() {
    let index = CompletionIndex::new(&contextual_fixture());

    for sql in ["insert ", "insert i"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );

        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.kind == CompletionKind::Keyword)
                .map(|candidate| candidate.label.as_str())
                .collect::<Vec<_>>(),
            vec!["INTO"],
            "unexpected keyword candidates for {sql}: {candidates:?}"
        );
    }
}

#[test]
fn insert_context_does_not_leak_into_statement_or_relation_completion() {
    let index = CompletionIndex::new(&fixture());

    let statement = complete(
        "i",
        1,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(
        statement.first().map(|candidate| candidate.label.as_str()),
        Some("INSERT")
    );
    assert!(statement.iter().all(|candidate| candidate.label != "INTO"));

    let relation_sql = "insert into u";
    let relation = complete(
        relation_sql,
        relation_sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(relation.iter().any(|candidate| {
        candidate.kind == CompletionKind::Table && candidate.label == "users"
    }));
}

#[test]
fn insert_column_list_only_offers_target_columns() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "insert into sys_user(";
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

    assert_eq!(
        labels(&candidates),
        [
            "update_time",
            "update_user",
            "update_user_phone",
            "user_type",
            "username",
        ]
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Column)
    );
}

#[test]
fn insert_column_list_filters_prefix_and_continues_after_comma() {
    let index = CompletionIndex::new(&contextual_fixture());
    let context = CompletionContext {
        database: Some("app"),
        schema: Some("public"),
    };

    let prefix_sql = "insert into sys_user(update_u";
    let prefix = complete(
        prefix_sql,
        prefix_sql.len(),
        SqlDialect::Postgres,
        &index,
        context,
    );
    assert_eq!(labels(&prefix), ["update_user", "update_user_phone"]);

    let comma_sql = "insert into sys_user(update_time, user_";
    let after_comma = complete(
        comma_sql,
        comma_sql.len(),
        SqlDialect::Postgres,
        &index,
        context,
    );
    assert_eq!(labels(&after_comma), ["user_type", "username"]);
}

#[test]
fn insert_column_list_discovers_target_relation_for_lazy_loading() {
    let entries = contextual_fixture();
    let expected = entries
        .iter()
        .find(|entry| entry.kind == CatalogKind::Table && entry.qualified_name.object == "sys_user")
        .unwrap()
        .id
        .clone();
    let index = CompletionIndex::new(&entries);
    let sql = "insert into sys_user(";

    let relations = relation_ids_for_completion(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );

    assert_eq!(relations.len(), 1);
    assert!(relations.contains(&expected));
}

#[test]
fn unrelated_parentheses_are_not_insert_column_lists() {
    let index = CompletionIndex::new(&contextual_fixture());
    let target_columns = [
        "update_time",
        "update_user",
        "update_user_phone",
        "user_type",
        "username",
    ];

    for sql in [
        "select count(",
        "select * from (",
        "select * from sys_user where username in (",
        "insert into sys_user (username) values (",
        "insert into sys_user values (",
        "insert into sys_user default values",
        "insert into sys_user select (",
        "insert into sys_user set username = (",
    ] {
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
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.label == "DELETE")
                || candidates.is_empty(),
            "unexpected target-column context for {sql}: {candidates:?}"
        );
        assert!(!target_columns.iter().all(|column| {
            candidates
                .iter()
                .any(|candidate| candidate.label == *column)
        }));
    }
}

#[test]
fn insert_column_list_uses_active_schema_for_duplicate_targets() {
    let connection = Uuid::new_v4();
    let mut entries = Vec::new();
    for (schema_name, column) in [("public", "public_value"), ("audit", "audit_value")] {
        let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", schema_name]);
        let table = CatalogId::new(
            connection,
            CatalogKind::Table,
            ["app", schema_name, "events"],
        );
        entries.push(
            CatalogEntry::relation(
                table.clone(),
                schema,
                qualified("app", Some(schema_name), "events"),
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
                    ["app", schema_name, "events", column],
                ),
                table,
                qualified("app", Some(schema_name), column),
                "column",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(1, "text", true)),
            )
            .unwrap(),
        );
    }
    let sql = "insert into events(";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::new(&entries),
        CompletionContext {
            database: Some("app"),
            schema: Some("audit"),
        },
    );

    assert_eq!(labels(&candidates), ["audit_value"]);
}

#[test]
fn insert_column_list_supports_qualified_and_quoted_targets() {
    let index = CompletionIndex::new(&contextual_fixture());
    for (sql, dialect) in [
        ("insert into public.sys_user(", SqlDialect::Postgres),
        ("insert into \"public\".\"sys_user\"(", SqlDialect::Postgres),
        ("insert into `app`.`sys_user`(", SqlDialect::MySql),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext {
                database: Some("app"),
                schema: Some("public"),
            },
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.label == "username")
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind == CompletionKind::Column),
            "unexpected candidates for {sql}: {candidates:?}"
        );
    }
}

#[test]
fn insert_space_triggers_completion() {
    assert!(should_offer_completion("insert ", "insert ".len()));
    assert!(should_offer_completion("INSERT ", "INSERT ".len()));
}

#[test]
fn completed_projection_prefers_from_keyword() {
    let index = CompletionIndex::new(&contextual_fixture());

    for sql in ["select * f", "select \"username\" f"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );

        assert_eq!(
            candidates.first().map(|candidate| candidate.label.as_str()),
            Some("FROM"),
            "unexpected candidates for {sql}: {candidates:?}"
        );
    }
}

#[test]
fn incomplete_projection_prefers_expression_keyword() {
    let index = CompletionIndex::new(&contextual_fixture());

    for sql in ["select f", "select username, f", "select username + f"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );

        assert_eq!(
            candidates.first().map(|candidate| candidate.label.as_str()),
            Some("FALSE"),
            "unexpected candidates for {sql}: {candidates:?}"
        );
        assert!(
            candidates.iter().all(|candidate| candidate.label != "FROM"),
            "FROM should not be offered for {sql}: {candidates:?}"
        );
    }
}

#[test]
fn predicate_completion_offers_predicate_keywords() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "select * from sys_user where n";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Keyword && candidate.label == "NOT"
    }));
}

#[test]
fn strings_and_comments_do_not_create_relation_bindings() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in [
        "select 'from user_agreement_accept' as note from sys_user where a",
        "select 1 /* join user_agreement_accept */ from sys_user where a",
        "select 1 -- join user_agreement_accept\nfrom sys_user where a",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(!candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Column && candidate.label == "agreement_id"
        }));
    }
}

#[test]
fn quoted_relation_identifiers_are_tokenized_as_single_words() {
    let index = CompletionIndex::new(&contextual_fixture());
    for (sql, dialect) in [
        (
            "select update_ from \"sys_user\" where update_",
            SqlDialect::Postgres,
        ),
        (
            "select update_ from `sys_user` where update_",
            SqlDialect::MySql,
        ),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext::default(),
        );
        assert!(candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Column && candidate.label == "update_time"
        }));
    }
}

#[test]
fn join_predicate_sees_both_relation_bindings() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select * from users u join roles r on ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"user_name"));
    assert!(labels.contains(&"role_name"));
    assert!(!labels.contains(&"audit_message"));
}

#[test]
fn comma_from_list_sees_each_relation_binding() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select  from users u, roles r";
    let candidates = complete(
        sql,
        "select ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"user_name"));
    assert!(labels.contains(&"role_name"));
    assert!(!labels.contains(&"audit_message"));
}

#[test]
fn alias_qualified_completion_only_uses_that_binding() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select r. from users u join roles r on u.id = r.id";
    let candidates = complete(
        sql,
        "select r.".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"role_name"));
    assert!(!labels.contains(&"user_name"));
}

#[test]
fn active_schema_resolves_duplicate_unqualified_relations() {
    let connection = Uuid::new_v4();
    let mut entries = Vec::new();
    for (schema_name, column) in [("public", "public_value"), ("audit", "audit_value")] {
        let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", schema_name]);
        let table = CatalogId::new(
            connection,
            CatalogKind::Table,
            ["app", schema_name, "events"],
        );
        entries.push(
            CatalogEntry::relation(
                table.clone(),
                schema,
                qualified("app", Some(schema_name), "events"),
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
                    ["app", schema_name, "events", column],
                ),
                table,
                qualified("app", Some(schema_name), column),
                "column",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(1, "text", true)),
            )
            .unwrap(),
        );
    }
    let index = CompletionIndex::new(&entries);
    let sql = "select  from events";
    let candidates = complete(
        sql,
        "select ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("audit"),
        },
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"audit_value"));
    assert!(!labels.contains(&"public_value"));
}

#[test]
fn subquery_does_not_leak_its_relations_to_the_outer_query() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select * from users u where exists (select 1 from roles r) and ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"user_name"));
    assert!(!labels.contains(&"role_name"));
}

#[test]
fn correlated_subquery_can_see_enclosing_relations() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select * from users u where exists (select 1 from roles r where )";
    let cursor = sql.len() - 1;
    let candidates = complete(
        sql,
        cursor,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"user_name"));
    assert!(labels.contains(&"role_name"));
}

#[test]
fn sibling_subquery_relations_are_not_visible() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select * from users u where exists (select 1 from roles r) and exists (select 1 from audit_log a where )";
    let cursor = sql.len() - 1;
    let candidates = complete(
        sql,
        cursor,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"user_name"));
    assert!(labels.contains(&"audit_message"));
    assert!(!labels.contains(&"role_name"));
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
    assert_eq!(
        quote_identifier("odd]name", SqlDialect::SqlServer),
        "[odd]]name]"
    );
}

#[test]
fn sql_server_completion_understands_bracketed_relations() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let text = "SELECT [users]. FROM [public].[users]";
    let candidates = complete(
        text,
        "SELECT [users].".len(),
        SqlDialect::SqlServer,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );
    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Column && candidate.insert_text == "[id]"
    }));

    let variable = "SELECT @user";
    assert!(
        complete(
            variable,
            variable.len(),
            SqlDialect::SqlServer,
            &index,
            CompletionContext::default(),
        )
        .is_empty()
    );
}

#[test]
fn sql_server_completion_uses_case_insensitive_active_database_and_schema() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let text = "SELECT * FROM ";
    let candidates = complete(
        text,
        text.len(),
        SqlDialect::SqlServer,
        &index,
        CompletionContext {
            database: Some("APP"),
            schema: Some("PUBLIC"),
        },
    );
    let users = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "users")
        .expect("active SQL Server target should resolve users");
    assert_eq!(users.insert_text, "users");
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
fn column_completion_detail_uses_short_type_spelling() {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let table = entries[2].id.clone();
    for (name, native_type) in [
        ("code", "character varying(30)"),
        ("created_at", "timestamp without time zone"),
    ] {
        entries.push(
            CatalogEntry::relation_child(
                CatalogId::new(
                    connection,
                    CatalogKind::Column,
                    ["app", "public", "users", name],
                ),
                table.clone(),
                qualified("app", Some("public"), name),
                "column",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(2, native_type, true)),
            )
            .unwrap(),
        );
    }
    let index = CompletionIndex::new(&entries);
    let candidates = complete(
        "select c from users",
        8,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    let detail = |label: &str| {
        candidates
            .iter()
            .find(|candidate| candidate.label == label)
            .and_then(|candidate| candidate.detail.clone())
    };
    assert_eq!(detail("code").as_deref(), Some("varchar(30)"));
    assert_eq!(detail("created_at").as_deref(), Some("timestamp"));
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
fn statement_keywords_rank_before_matching_catalog_names() {
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

    for (prefix, keyword) in [("s", "SELECT"), ("d", "DELETE"), ("u", "UPDATE")] {
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
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16.4".into(),
            database: "app".into(),
        },
    });
    app.active_console_mut()
        .execution_target
        .as_mut()
        .unwrap()
        .schema = Some("public".into());
    app.explorer.completion_index = CompletionIndex::new(&entries);
    app.update(Action::ReplaceEditor("select * from or".into()));
    app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('A'),
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
