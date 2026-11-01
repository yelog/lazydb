use lazydb::{
    action::{Action, Command},
    app::App,
    db::ServerInfo,
    db::catalog::{
        CatalogCount, CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, CatalogPage,
        CatalogRequest, CatalogTarget, ColumnMetadata, OptionalMetadata, QualifiedName,
    },
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
    sql::{
        CompletionContext, CompletionIndex, CompletionInsertionMode, CompletionKind, SqlDialect,
        TextRange, complete, complete_with_mode, completion_dependencies, quote_identifier,
        should_offer_completion,
    },
};
use uuid::Uuid;

#[test]
fn unbound_console_automatically_offers_generic_keywords() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewUnboundConsoleNamed("scratch".into()));
    for character in ['s', 'e', 'l'] {
        app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(character),
            crossterm::event::KeyModifiers::NONE,
        )));
    }

    let key = app
        .active_console()
        .completion_request
        .as_ref()
        .map(|request| lazydb::sql::CompletionScheduleKey {
            console_id: app.active_console().id,
            document_revision: request.revision,
            cursor: request.cursor,
            connection: request.connection,
            target: request.target.clone(),
            catalog_generation: request.catalog_generation,
        })
        .expect("unbound completion should schedule automatically");
    app.update(Action::CompletionDue(key));

    let popup = app
        .active_console()
        .completion
        .as_ref()
        .expect("generic completion popup");
    assert!(
        popup
            .candidates
            .iter()
            .any(|candidate| candidate.insert_text == "SELECT")
    );
    assert!(popup.candidates.iter().all(|candidate| {
        !matches!(
            candidate.kind,
            CompletionKind::Table | CompletionKind::Column
        )
    }));
}

#[test]
fn unbound_console_explicit_completion_does_not_connect() {
    let mut app = App::new(Vec::new());
    app.update(Action::NewUnboundConsoleNamed("scratch".into()));
    for character in ['s', 'e', 'l'] {
        app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(character),
            crossterm::event::KeyModifiers::NONE,
        )));
    }
    let commands = app
        .update(Action::CompletionExplicit)
        .into_iter()
        .find_map(|command| match command {
            Command::ScheduleCompletion(key) => Some(key),
            _ => None,
        });
    assert!(commands.is_none());
    assert!(app.active_console().completion.is_some());
}

#[test]
fn builtin_default_current_timestamp_without_catalog() {
    let sql = "CREATE TABLE test1(\n id BIGINT NOT NULL,\n \"name\" text,\n create_time TIMESTAMP NOT NULL DEFAULT CURRENT_TIM\n)";
    let cursor = sql.find("CURRENT_TIM").unwrap() + "CURRENT_TIM".len();
    let candidates = complete(
        sql,
        cursor,
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.label == "CURRENT_TIMESTAMP")
        .expect("CURRENT_TIMESTAMP completion");
    assert_eq!(candidate.insert_text, "CURRENT_TIMESTAMP");
    assert_eq!(candidate.kind, CompletionKind::BuiltinExpression);
    assert_eq!(
        candidate.replace,
        TextRange::new(cursor - "CURRENT_TIM".len(), cursor)
    );
}

#[test]
fn builtin_current_timestamp_is_available_in_select_expression() {
    let sql = "SELECT CURRENT_TIM";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "CURRENT_TIMESTAMP"),
        "{candidates:?}"
    );
}

#[test]
fn clickhouse_builtin_functions_and_types_are_available_offline() {
    let sql = "SELECT arrayJ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::ClickHouse,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "arrayJoin"),
        "{candidates:?}"
    );
    let sql = "CREATE TABLE events (value Nullable";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::ClickHouse,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "Nullable"
                && candidate.kind == CompletionKind::DataType),
        "{candidates:?}"
    );
    let sql = "CREATE TABLE events (id UInt";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::ClickHouse,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "UInt64"
                && candidate.kind == CompletionKind::DataType),
        "{candidates:?}"
    );
}

#[test]
fn builtin_select_expression_match_is_case_insensitive() {
    let sql = "SELECT current_tim";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates.iter().any(|candidate| {
            candidate.label == "CURRENT_TIMESTAMP" && candidate.insert_text == "CURRENT_TIMESTAMP"
        }),
        "{candidates:?}"
    );
}

#[test]
fn builtin_select_dialect_specific_functions_are_isolated() {
    let cases = [
        (SqlDialect::Postgres, "GETDATE", 0),
        (SqlDialect::MySql, "NOW", 1),
        (SqlDialect::SqlServer, "GETDATE", 1),
        (SqlDialect::Sqlite, "NOW", 0),
    ];
    for (dialect, prefix, expected_any) in cases {
        let sql = format!("SELECT {prefix}");
        let candidates = complete(
            &sql,
            sql.len(),
            dialect,
            &CompletionIndex::default(),
            CompletionContext::default(),
        );
        let present = candidates
            .iter()
            .any(|candidate| candidate.label == dialect_bound_name(prefix));
        assert_eq!(present, expected_any == 1, "{dialect:?}: {candidates:?}");
    }
}

fn dialect_bound_name(prefix: &str) -> &'static str {
    match prefix {
        "NOW" => "NOW",
        "GETDATE" => "GETDATE",
        _ => unreachable!(),
    }
}

#[test]
fn builtin_select_empty_prefix_only_offers_special_expressions() {
    let sql = "SELECT ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "CURRENT_TIMESTAMP"),
        "{candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "COALESCE"),
        "{candidates:?}"
    );
}

#[test]
fn builtin_select_not_offered_in_relation_or_qualifier_positions() {
    let from = complete(
        "SELECT * FROM cur",
        "SELECT * FROM cur".len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        from.iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "{from:?}"
    );
    let qualified = complete(
        "SELECT t.cur FROM test t",
        "SELECT t.cur".len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        qualified
            .iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "{qualified:?}"
    );
}

#[test]
fn builtin_sqlite_function_is_not_excluded_by_catalog_dialect_filter() {
    let sql = "SELECT LENGTH";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Sqlite,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates.iter().any(|candidate| {
            candidate.label == "LENGTH" && candidate.kind == CompletionKind::Function
        }),
        "{candidates:?}"
    );
}

fn has_builtin(
    candidates: &[lazydb::sql::CompletionCandidate],
    label: &str,
    kind: lazydb::sql::CompletionKind,
) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.label == label && candidate.kind == kind)
}

#[test]
fn builtin_default_expression_scope_handles_parentheses_and_boundaries() {
    let dialect = SqlDialect::Postgres;
    let index = CompletionIndex::default();
    let cases: &[(&str, &str)] = &[
        (
            "CREATE TABLE t (ts TIMESTAMP DEFAULT CURRENT_TIM",
            "CURRENT_TIM",
        ),
        (
            "CREATE TABLE t (ts TIMESTAMP DEFAULT (CURRENT_TIM)",
            "CURRENT_TIM",
        ),
        (
            "CREATE TABLE t (n DECIMAL(10, 2), ts TIMESTAMP DEFAULT CURRENT_TIM)",
            "CURRENT_TIM",
        ),
        (
            "CREATE TABLE t (ts TIMESTAMP DEFAULT COALESCE(NULL, CURRENT_TIM))",
            "CURRENT_TIM",
        ),
    ];
    for (sql, needle) in cases {
        let cursor = sql.find(needle).unwrap() + needle.len();
        let candidates = complete(sql, cursor, dialect, &index, CompletionContext::default());
        assert!(
            has_builtin(
                &candidates,
                "CURRENT_TIMESTAMP",
                CompletionKind::BuiltinExpression,
            ),
            "{sql}: {candidates:?}"
        );
    }
}

#[test]
fn builtin_default_value_restores_type_and_constraint_context() {
    let dialect = SqlDialect::Postgres;
    let index = CompletionIndex::default();
    let type_case = "CREATE TABLE t (ts TIMESTAMP DEFAULT CURRENT_TIMESTAMP, n IN";
    let candidates = complete(
        type_case,
        type_case.len(),
        dialect,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::DataType),
        "expected type candidates: {candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "{candidates:?}"
    );

    let constraint_case = "CREATE TABLE t (ts TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT N";
    let candidates = complete(
        constraint_case,
        constraint_case.len(),
        dialect,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "NOT NULL"),
        "expected NOT NULL keyword: {candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "{candidates:?}"
    );
}

#[test]
fn builtin_default_null_value_does_not_break_column_boundary() {
    let sql = "CREATE TABLE t (ts TIMESTAMP DEFAULT NULL, n IN";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::DataType),
        "expected type candidates: {candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "{candidates:?}"
    );
}

#[test]
fn builtin_default_ignores_quoted_identifiers_and_string_literals() {
    let sql = "CREATE TABLE t (\"default\" TIMESTAMP NOT N";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "NOT NULL"),
        "expected NOT NULL keyword: {candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "{candidates:?}"
    );

    let literal = "CREATE TABLE t (a TEXT DEFAULT 'DEFAULT CURRENT_TIM";
    let candidates = complete(
        literal,
        literal.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "string literal must not offer builtins: {candidates:?}"
    );
}

#[test]
fn builtin_default_value_dialect_layout_match() {
    let index = CompletionIndex::default();
    let pg = complete(
        "CREATE TABLE t (ts TIMESTAMP DEFAULT NOW",
        "CREATE TABLE t (ts TIMESTAMP DEFAULT NOW".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let mysql = complete(
        "CREATE TABLE t (ts TIMESTAMP DEFAULT NOW",
        "CREATE TABLE t (ts TIMESTAMP DEFAULT NOW".len(),
        SqlDialect::MySql,
        &index,
        CompletionContext::default(),
    );
    assert!(
        pg.iter().any(|candidate| candidate.label == "NOW"),
        "{pg:?}"
    );
    assert!(
        mysql.iter().all(|candidate| candidate.label != "NOW"),
        "{mysql:?}"
    );
}

#[test]
fn builtin_alter_table_default_forms_offer_builtins() {
    let index = CompletionIndex::default();
    let cases = [
        (
            "ALTER TABLE t ADD COLUMN c TIMESTAMP DEFAULT CURRENT_TIM",
            SqlDialect::Postgres,
            "CURRENT_TIMESTAMP",
        ),
        (
            "ALTER TABLE t ALTER COLUMN c SET DEFAULT CURRENT_TIM",
            SqlDialect::Postgres,
            "CURRENT_TIMESTAMP",
        ),
        (
            "ALTER TABLE t ADD COLUMN c TIMESTAMP DEFAULT CURRENT_TIM",
            SqlDialect::MySql,
            "CURRENT_TIMESTAMP",
        ),
        (
            "ALTER TABLE t ALTER COLUMN c SET DEFAULT CURRENT_TIM",
            SqlDialect::MySql,
            "CURRENT_TIMESTAMP",
        ),
        (
            "ALTER TABLE t ADD c INT DEFAULT GETD",
            SqlDialect::SqlServer,
            "GETDATE",
        ),
        (
            "ALTER TABLE t ADD COLUMN c TIMESTAMP DEFAULT CURRENT_TIM",
            SqlDialect::Sqlite,
            "CURRENT_TIMESTAMP",
        ),
    ];
    for (sql, dialect, expected) in cases {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.label == expected),
            "{dialect:?} {sql}: {candidates:?}"
        );
    }
}

#[test]
fn builtin_alter_non_default_actions_offer_no_builtins() {
    let index = CompletionIndex::default();
    let cases = [
        (
            "ALTER TABLE t ALTER COLUMN c DROP DEFAULT",
            SqlDialect::Postgres,
        ),
        (
            "ALTER TABLE t RENAME COLUMN a TO default",
            SqlDialect::Postgres,
        ),
        ("ALTER TABLE t DROP COLUMN c", SqlDialect::Postgres),
        (
            "ALTER TABLE t ADD CONSTRAINT ck DEFAULT 0 FOR c",
            SqlDialect::SqlServer,
        ),
    ];
    for (sql, dialect) in cases {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"
                    && candidate.label != "GETDATE"),
            "{dialect:?} {sql}: {candidates:?}"
        );
    }
}

#[test]
fn builtin_filter_subqueries_and_ordering_positions() {
    let index = CompletionIndex::default();
    let subquery = "SELECT * FROM t WHERE x IN (SELECT CUR";
    let candidates = complete(
        subquery,
        subquery.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "CURRENT_TIMESTAMP"),
        "subquery expression should offer builtins: {candidates:?}"
    );

    let expression = "SELECT a FROM t ORDER BY CUR";
    let candidates = complete(
        expression,
        expression.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "CURRENT_TIMESTAMP"),
        "ordering expression should offer builtins: {candidates:?}"
    );

    let direction = "SELECT a FROM t ORDER BY a ASC";
    let candidates = complete(
        direction,
        direction.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "CURRENT_TIMESTAMP"),
        "ordering direction state must not offer builtins: {candidates:?}"
    );
}

#[test]
fn builtin_filter_comment_position_is_not_offered_by_trigger() {
    let sql = "SELECT /* CUR";
    assert!(!should_offer_completion(sql, sql.len()));
}

#[test]
fn builtin_identity_keeps_user_functions_and_avoid_internal_duplicates() {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let schema = entries[1].id.clone();
    let function = CatalogEntry::object(
        CatalogId::new(connection, CatalogKind::Function, ["app", "public", "NOW"]),
        schema.clone(),
        qualified("app", Some("public"), "NOW"),
        "user function",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    entries.push(function);
    let index = CompletionIndex::new(&entries);
    let sql = "SELECT NOW";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let now = candidates
        .iter()
        .filter(|candidate| candidate.label == "NOW")
        .count();
    assert!(
        now >= 2,
        "static builtin and catalog function must both survive: {candidates:?}"
    );
}

#[test]
fn builtin_internal_list_has_no_duplicate_labels() {
    let sql = "SELECT C";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    let mut labels: Vec<&str> = candidates
        .iter()
        .filter(|candidate| candidate.detail.is_some())
        .map(|candidate| candidate.label.as_str())
        .collect();
    labels.sort_unstable();
    let before = labels.len();
    labels.dedup();
    assert_eq!(
        before,
        labels.len(),
        "duplicate builtin labels: {candidates:?}"
    );
}

#[test]
fn builtin_default_completion_does_not_request_relation_children() {
    let sql = "CREATE TABLE t (ts TIMESTAMP DEFAULT CURRENT_TIM";
    let cursor = sql.find("CURRENT_TIM").unwrap();
    let dependencies = completion_dependencies(
        sql,
        cursor,
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        dependencies.relation_children.is_empty(),
        "default value completion must not add catalog requests: {dependencies:?}"
    );
}

#[test]
fn builtin_app_completion_popup_and_accept_work_end_to_end() {
    let profile = import_connection_url("postgres://localhost/app", Some("app"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.update(Action::NewConsole);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16.4".into(),
            database: "app".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    let sql = "CREATE TABLE test1(\n id BIGINT NOT NULL,\n \"name\" text,\n create_time TIMESTAMP NOT NULL DEFAULT CURRENT_TIM";
    app.update(Action::ReplaceEditor(sql[..sql.len() - 2].into()));
    for key in ['G', '$', 'a', 'I', 'M'] {
        app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(key),
            crossterm::event::KeyModifiers::NONE,
        )));
    }
    app.update(Action::CompletionExplicit);
    let popup = app
        .active_console_opt()
        .and_then(|tab| tab.completion.as_ref())
        .expect("completion popup");
    assert!(
        popup
            .candidates
            .iter()
            .any(|candidate| candidate.label == "CURRENT_TIMESTAMP"),
        "{:?}",
        popup
            .candidates
            .iter()
            .map(|c| &c.label)
            .collect::<Vec<_>>()
    );
    let mut steps = 0;
    while app
        .active_console_opt()
        .and_then(|tab| tab.completion.as_ref())
        .and_then(|popup| popup.candidates.get(popup.selected))
        .map(|candidate| candidate.label.as_str())
        != Some("CURRENT_TIMESTAMP")
    {
        app.update(Action::CompletionNext);
        steps += 1;
        assert!(steps < 20, "could not navigate to CURRENT_TIMESTAMP");
    }
    app.update(Action::CompletionAccept);
    let text = app.active_editor_text().unwrap();
    assert!(
        text.trim_end().ends_with("DEFAULT CURRENT_TIMESTAMP"),
        "accepted completion must replace prefix without quotes: {text:?}"
    );
    assert!(
        !text.contains("CURRENT_TIMESTAMP\""),
        "builtin expression must not be quoted: {text:?}"
    );
}

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

fn fixture_with_column_names(names: &[&str]) -> Vec<CatalogEntry> {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let table = entries[2].id.clone();
    for name in names {
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
                CatalogMetadata::Column(ColumnMetadata::new(2, "bigint", false)),
            )
            .unwrap(),
        );
    }
    entries
}

#[test]
fn alter_add_column_matches_create_definition() {
    let index = CompletionIndex::default();
    for (suffix, expected) in [("age b", "BIGINT"), ("age BIGINT n", "NOT NULL")] {
        let create = format!("CREATE TABLE draft ({suffix}");
        let alter = format!("ALTER TABLE users ADD COLUMN {suffix}");
        let project = |sql: &str| {
            complete(
                sql,
                sql.len(),
                SqlDialect::Postgres,
                &index,
                CompletionContext::default(),
            )
            .into_iter()
            .map(|candidate| (candidate.label, candidate.kind, candidate.insert_text))
            .collect::<Vec<_>>()
        };
        let create_candidates = project(&create);
        let alter_candidates = project(&alter);
        assert!(
            create_candidates
                .iter()
                .any(|candidate| candidate.0 == expected),
            "CREATE {create}: {create_candidates:?}"
        );
        assert!(
            alter_candidates
                .iter()
                .any(|candidate| candidate.0 == expected),
            "ALTER {alter}: {alter_candidates:?}"
        );
        assert_eq!(alter_candidates, create_candidates);
    }
}

#[test]
fn alter_add_column_offers_constraints_after_completed_type() {
    let index = CompletionIndex::default();
    let sql = "ALTER TABLE users ADD COLUMN age BIGINT n";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Keyword && candidate.label == "NOT NULL"
        }),
        "{candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Keyword)
    );
}

#[test]
fn alter_column_offers_definition_after_completed_column_name() {
    let index = CompletionIndex::default();
    let type_prefix = "ALTER TABLE users ALTER COLUMN age v";
    let type_candidates = complete(
        type_prefix,
        type_prefix.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        type_candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::DataType && candidate.label == "VARCHAR"
        }),
        "{type_prefix}: {type_candidates:?}"
    );

    let constraint_prefix = "ALTER TABLE users ALTER COLUMN age s";
    let constraint_candidates = complete(
        constraint_prefix,
        constraint_prefix.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        constraint_candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Keyword && candidate.label == "SET NOT NULL"
        }),
        "{constraint_prefix}: {constraint_candidates:?}"
    );

    let default_prefix = "ALTER TABLE users ALTER COLUMN age SET DEFAULT n";
    let default_candidates = complete(
        default_prefix,
        default_prefix.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        default_candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Function && candidate.label == "NOW"
        }),
        "{default_prefix}: {default_candidates:?}"
    );
}

#[test]
fn alter_column_postgres_type_and_attribute_insertions_are_valid() {
    let index = CompletionIndex::default();
    for (sql, label, insert_text) in [
        (
            "ALTER TABLE users ALTER COLUMN age v",
            "VARCHAR",
            "TYPE VARCHAR",
        ),
        (
            "ALTER TABLE users ALTER COLUMN age TYPE v",
            "VARCHAR",
            "VARCHAR",
        ),
        (
            "ALTER TABLE users ALTER COLUMN age SET n",
            "NOT NULL",
            "NOT NULL",
        ),
        (
            "ALTER TABLE users ALTER COLUMN age DROP d",
            "DEFAULT",
            "DEFAULT",
        ),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.label == label)
            .unwrap_or_else(|| panic!("{sql}: {candidates:?}"));
        assert_eq!(candidate.insert_text, insert_text, "{sql}");
    }
}

#[test]
fn alter_column_definition_follows_dialect_rules() {
    let index = CompletionIndex::default();
    let mysql = complete(
        "ALTER TABLE users MODIFY COLUMN age v",
        "ALTER TABLE users MODIFY COLUMN age v".len(),
        SqlDialect::MySql,
        &index,
        CompletionContext::default(),
    );
    assert!(mysql.iter().any(|candidate| {
        candidate.kind == CompletionKind::DataType
            && candidate.label == "VARCHAR"
            && candidate.insert_text == "VARCHAR"
    }));

    let sql_server = complete(
        "ALTER TABLE users ALTER COLUMN age n",
        "ALTER TABLE users ALTER COLUMN age n".len(),
        SqlDialect::SqlServer,
        &index,
        CompletionContext::default(),
    );
    assert!(sql_server.iter().any(|candidate| {
        candidate.kind == CompletionKind::Keyword && candidate.label == "NOT NULL"
    }));

    let sqlite = complete(
        "ALTER TABLE users ALTER COLUMN age v",
        "ALTER TABLE users ALTER COLUMN age v".len(),
        SqlDialect::Sqlite,
        &index,
        CompletionContext::default(),
    );
    assert!(sqlite.is_empty(), "{sqlite:?}");
}

#[test]
fn alter_column_keeps_completed_name_in_column_stage_until_space() {
    let index = CompletionIndex::new(&fixture_with_column_names(&["age", "archived"]));
    let sql = "ALTER TABLE users ALTER COLUMN age";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Column && candidate.label == "age"),
        "{candidates:?}"
    );
    assert!(candidates.iter().all(|candidate| {
        candidate.kind == CompletionKind::Column || candidate.kind == CompletionKind::Keyword
    }));
}

#[test]
fn alter_column_definition_allows_automatic_completion_after_space() {
    for sql in [
        "ALTER TABLE users ALTER COLUMN age ",
        "ALTER TABLE users ALTER COLUMN age TYPE ",
        "ALTER TABLE users ALTER COLUMN age SET ",
        "ALTER TABLE users ALTER COLUMN age SET DEFAULT ",
    ] {
        assert!(
            lazydb::sql::should_offer_completion_for_dialect(sql, sql.len(), SqlDialect::Postgres,),
            "{sql:?}"
        );
    }
}

#[test]
fn alter_column_definition_handles_quoted_table_and_column_names() {
    let sql = "ALTER TABLE \"users\" ALTER COLUMN \"age\" v";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    assert!(
        candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::DataType
                && candidate.label == "VARCHAR"
                && candidate.insert_text == "TYPE VARCHAR"
        }),
        "{candidates:?}"
    );
}

#[test]
fn alter_rename_column_suggests_target_columns() {
    let entries = fixture_with_column_names(&["age", "archived"]);
    let index = CompletionIndex::new(&entries);
    let sql = "ALTER TABLE users RENAME COLUMN a";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Column && candidate.label == "age"
        }),
        "{candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Column),
        "{candidates:?}"
    );
}

#[test]
fn alter_rename_column_transitions_to_and_new_name() {
    let index = CompletionIndex::new(&fixture_with_column_names(&["age", "archived"]));
    let to = complete(
        "ALTER TABLE users RENAME COLUMN age ",
        "ALTER TABLE users RENAME COLUMN age ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(to.iter().any(|candidate| candidate.label == "TO"), "{to:?}");

    let new_name = complete(
        "ALTER TABLE users RENAME COLUMN age TO ",
        "ALTER TABLE users RENAME COLUMN age TO ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        new_name
            .iter()
            .all(|candidate| candidate.kind != CompletionKind::Column),
        "{new_name:?}"
    );
}

#[test]
fn completion_trigger_rejects_blank_current_line() {
    for sql in ["ALTER TABLE users\n  ", "SELECT * FROM users ORDER BY\n\t"] {
        assert!(
            !lazydb::sql::should_offer_completion_for_dialect(sql, sql.len(), SqlDialect::Postgres,),
            "{sql:?}"
        );
    }
}

#[test]
fn column_insertion_keeps_plain_names() {
    let entries = fixture_with_column_names(&["small_num"]);
    let index = CompletionIndex::new(&entries);
    let sql = "SELECT small";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.label == "small_num")
        .expect("small_num column completion");
    assert_eq!(candidate.insert_text, "small_num");
}

#[test]
fn column_insertion_quotes_reserved_and_special_names() {
    let entries = fixture_with_column_names(&["select", "odd name"]);
    let index = CompletionIndex::new(&entries);
    for (prefix, expected) in [("sele", "\"select\""), ("odd", "\"odd name\"")] {
        let sql = format!("SELECT {prefix}");
        let candidates = complete(
            &sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        let candidate = candidates
            .iter()
            .find(|candidate| {
                candidate.label
                    == if prefix == "sele" {
                        "select"
                    } else {
                        "odd name"
                    }
            })
            .expect("identifier completion");
        assert_eq!(candidate.insert_text, expected);
    }
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
fn completion_keeps_all_matching_candidates() {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let schema = entries[1].id.clone();
    for index in 0..25 {
        let name = format!("candidate_{index:02}");
        entries.push(
            CatalogEntry::relation(
                CatalogId::new(connection, CatalogKind::Table, ["app", "public", &name]),
                schema.clone(),
                qualified("app", Some("public"), &name),
                "table",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
    }
    let index = CompletionIndex::new(&entries);
    let candidates = complete(
        "select * from candidate_",
        "select * from candidate_".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );
    let labels = candidates
        .iter()
        .filter(|candidate| candidate.kind == CompletionKind::Table)
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();

    assert_eq!(labels.len(), 25);
    assert!(labels.contains(&"candidate_10"));
    assert!(labels.contains(&"candidate_24"));
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

fn cross_database_fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let app = CatalogId::new(connection, CatalogKind::Database, ["app"]);
    let public = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let audit = CatalogId::new(connection, CatalogKind::Schema, ["app", "audit"]);
    let other = CatalogId::new(connection, CatalogKind::Database, ["other"]);
    let tools = CatalogId::new(connection, CatalogKind::Schema, ["other", "tools"]);
    let mut entries = vec![
        CatalogEntry::database(
            app.clone(),
            qualified("app", None, "app"),
            "database",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::schema(
            public.clone(),
            app.clone(),
            qualified("app", Some("public"), "public"),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::schema(
            audit.clone(),
            app,
            qualified("app", Some("audit"), "audit"),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::database(
            other.clone(),
            qualified("other", None, "other"),
            "database",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::schema(
            tools.clone(),
            other,
            qualified("other", Some("tools"), "tools"),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
    ];
    for (schema_id, schema_name, database_name) in [
        (public, "public", "app"),
        (audit, "audit", "app"),
        (tools, "tools", "other"),
    ] {
        let table = CatalogId::new(
            connection,
            CatalogKind::Table,
            [database_name, schema_name, "sys_user"],
        );
        entries.push(
            CatalogEntry::relation(
                table,
                schema_id,
                qualified(database_name, Some(schema_name), "sys_user"),
                "table",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
    }
    entries
}

#[test]
fn current_segment_mode_never_adds_namespace() {
    let index = CompletionIndex::new(&cross_database_fixture());
    for context in [
        CompletionContext::default(),
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    ] {
        let candidates = complete_with_mode(
            "select * from sys_",
            "select * from sys_".len(),
            SqlDialect::Generic,
            &index,
            context,
            CompletionInsertionMode::CurrentSegment,
        );
        let table = candidates
            .iter()
            .find(|candidate| candidate.kind == CompletionKind::Table)
            .expect("table completion in current segment mode");
        assert_eq!(table.insert_text, "sys_user");
    }
}

#[test]
fn contextual_mode_keeps_existing_namespace_policy() {
    let index = CompletionIndex::new(&cross_database_fixture());
    let without_context = complete(
        "select * from sys_",
        "select * from sys_".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    let table = without_context
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table)
        .expect("table completion in contextual mode");
    assert_eq!(table.insert_text, "app.public.sys_user");

    let with_context = complete(
        "select * from sys_",
        "select * from sys_".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );
    let table = with_context
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table)
        .expect("table completion with matching context");
    assert_eq!(table.insert_text, "sys_user");
}

#[test]
fn current_segment_mode_preserves_typed_qualifier_prefix() {
    let index = CompletionIndex::new(&cross_database_fixture());
    let sql = "select * from tools.sys_";
    let candidates = complete_with_mode(
        sql,
        sql.len(),
        SqlDialect::Generic,
        &index,
        CompletionContext::default(),
        CompletionInsertionMode::CurrentSegment,
    );
    let table = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table)
        .expect("table completion with qualifier");
    assert_eq!(table.insert_text, "sys_user");
    let mut applied = String::new();
    applied.push_str(&sql[..table.replace.start]);
    applied.push_str(&table.insert_text);
    applied.push_str(&sql[table.replace.end..]);
    assert_eq!(applied, "select * from tools.sys_user");
}

fn quoted_identifier_fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let app = CatalogId::new(connection, CatalogKind::Database, ["app"]);
    let public = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let weird = CatalogId::new(connection, CatalogKind::Schema, ["app", "weird.schema"]);
    let unicode_table = CatalogId::new(connection, CatalogKind::Table, ["app", "public", "用户"]);
    let dot_table = CatalogId::new(
        connection,
        CatalogKind::Table,
        ["app", "weird.schema", "sys_user"],
    );
    vec![
        CatalogEntry::database(
            app.clone(),
            qualified("app", None, "app"),
            "database",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::schema(
            public.clone(),
            app.clone(),
            qualified("app", Some("public"), "public"),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::schema(
            weird.clone(),
            app,
            qualified("app", Some("weird.schema"), "weird.schema"),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::relation(
            unicode_table,
            public,
            qualified("app", Some("public"), "用户"),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::relation(
            dot_table,
            weird,
            qualified("app", Some("weird.schema"), "sys_user"),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
    ]
}

#[test]
fn quoted_qualifier_with_dot_resolves_as_single_segment() {
    let index = CompletionIndex::new(&quoted_identifier_fixture());
    let sql = "select * from \"weird.schema\".";
    let candidates = complete_with_mode(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
        CompletionInsertionMode::CurrentSegment,
    );
    let table = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "sys_user")
        .expect("quoted schema with dot must resolve to its tables");
    assert_eq!(table.detail.as_deref(), Some("(app.weird.schema)"));
}

#[test]
fn current_segment_keeps_quoted_qualifier_path() {
    let index = CompletionIndex::new(&quoted_identifier_fixture());
    let sql = "select * from \"weird.schema\".sys_";
    let candidates = complete_with_mode(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
        CompletionInsertionMode::CurrentSegment,
    );
    let table = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table)
        .expect("table completion inside a quoted qualifier path");
    assert_eq!(table.insert_text, "sys_user");
    let mut applied = String::new();
    applied.push_str(&sql[..table.replace.start]);
    applied.push_str(&table.insert_text);
    applied.push_str(&sql[table.replace.end..]);
    assert_eq!(applied, "select * from \"weird.schema\".sys_user");
}

#[test]
fn mid_word_completion_replaces_whole_token() {
    let index = CompletionIndex::new(&quoted_identifier_fixture());
    let sql = "select * from sys_user_archive";
    let candidates = complete_with_mode(
        sql,
        "select * from sys_".len(),
        SqlDialect::Generic,
        &index,
        CompletionContext::default(),
        CompletionInsertionMode::CurrentSegment,
    );
    let table = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "sys_user")
        .expect("mid word completion should still match the prefix");
    assert_eq!(table.insert_text, "sys_user");
    let mut applied = String::new();
    applied.push_str(&sql[..table.replace.start]);
    applied.push_str(&table.insert_text);
    applied.push_str(&sql[table.replace.end..]);
    assert_eq!(applied, "select * from sys_user");
}

#[test]
fn unclosed_quoted_segment_inserts_quoted_object() {
    let index = CompletionIndex::new(&quoted_identifier_fixture());
    let sql = "select * from \"sys_";
    let candidates = complete_with_mode(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
        CompletionInsertionMode::CurrentSegment,
    );
    let table = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "sys_user")
        .expect("quoted prefix should match table names");
    assert_eq!(table.insert_text, "\"sys_user\"");
    let mut applied = String::new();
    applied.push_str(&sql[..table.replace.start]);
    applied.push_str(&table.insert_text);
    applied.push_str(&sql[table.replace.end..]);
    assert_eq!(applied, "select * from \"sys_user\"");
}

#[test]
fn unicode_prefix_matches_identifier_without_byte_splitting() {
    let index = CompletionIndex::new(&quoted_identifier_fixture());
    let sql = "select * from 用";
    let candidates = complete_with_mode(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
        CompletionInsertionMode::CurrentSegment,
    );
    let table = candidates
        .iter()
        .find(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "用户")
        .expect("unicode prefix should match");
    assert_eq!(table.insert_text, "\"用户\"");
    let mut applied = String::new();
    applied.push_str(&sql[..table.replace.start]);
    applied.push_str(&table.insert_text);
    applied.push_str(&sql[table.replace.end..]);
    assert_eq!(applied, "select * from \"用户\"");
}

fn schema_ambiguity_fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let mut entries = Vec::new();
    for database_name in ["app", "other"] {
        let database = CatalogId::new(connection, CatalogKind::Database, [database_name]);
        let tools = CatalogId::new(connection, CatalogKind::Schema, [database_name, "tools"]);
        let users = CatalogId::new(
            connection,
            CatalogKind::Table,
            [database_name, "tools", "sys_user"],
        );
        entries.push(
            CatalogEntry::database(
                database.clone(),
                qualified(database_name, None, database_name),
                "database",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
        entries.push(
            CatalogEntry::schema(
                tools.clone(),
                database,
                qualified(database_name, Some("tools"), "tools"),
                "schema",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
        entries.push(
            CatalogEntry::relation(
                users,
                tools,
                qualified(database_name, Some("tools"), "sys_user"),
                "table",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        );
    }
    entries
}

fn mirrored_nav_fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let database = CatalogId::new(connection, CatalogKind::Database, ["app"]);
    let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", "app"]);
    let orders = CatalogId::new(connection, CatalogKind::Table, ["app", "app", "orders"]);
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
            qualified("app", Some("app"), "app"),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
        CatalogEntry::relation(
            orders,
            schema,
            qualified("app", Some("app"), "orders"),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
    ]
}

#[test]
fn single_qualifier_schema_is_limited_to_active_database() {
    let index = CompletionIndex::new(&schema_ambiguity_fixture());
    let sql = "select * from tools.";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Generic,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );
    let tables = candidates
        .iter()
        .filter(|candidate| candidate.kind == CompletionKind::Table)
        .collect::<Vec<_>>();
    assert_eq!(
        tables.len(),
        1,
        "only the active database schema should match"
    );
    assert_eq!(tables[0].label, "sys_user");
    assert_eq!(tables[0].detail.as_deref(), Some("(app.tools)"));
}

#[test]
fn ambiguous_schemata_keep_distinct_details_when_context_unknown() {
    let index = CompletionIndex::new(&schema_ambiguity_fixture());
    let sql = "select * from tools.";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Generic,
        &index,
        CompletionContext::default(),
    );
    let details = candidates
        .iter()
        .filter(|candidate| candidate.kind == CompletionKind::Table)
        .map(|candidate| candidate.detail.as_deref().unwrap_or(""))
        .collect::<Vec<_>>();
    assert!(
        details.contains(&"(app.tools)"),
        "missing app schema detail"
    );
    assert!(
        details.contains(&"(other.tools)"),
        "missing other schema detail"
    );
    let unqualified = complete(
        "select * from ",
        "select * from ".len(),
        SqlDialect::Generic,
        &index,
        CompletionContext::default(),
    );
    let schema_details = unqualified
        .iter()
        .filter(|candidate| candidate.kind == CompletionKind::Schema)
        .filter_map(|candidate| candidate.detail.as_deref())
        .collect::<Vec<_>>();
    assert!(
        schema_details.contains(&"(app)"),
        "schema detail requires database"
    );
    assert!(
        schema_details.contains(&"(other)"),
        "schema detail requires database"
    );
}

#[test]
fn postgres_blocks_non_current_database_navigation() {
    let index = CompletionIndex::new(&schema_ambiguity_fixture());
    let sql = "select * from other.tools.";
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
        !candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Table),
        "postgres must not navigate into a non-current database"
    );
}

#[test]
fn alias_shortcut_ignores_extra_segments() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select u.x. from users u";
    let candidates = complete(
        sql,
        "select u.x.".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates.iter().all(|candidate| {
            candidate.kind != CompletionKind::Column
                || (candidate.label != "user_name" && candidate.label != "id")
        }),
        "a multi-segment path starting with an alias must not fall back to alias columns"
    );
}

#[test]
fn mysql_folds_mirrored_schema_level() {
    let index = CompletionIndex::new(&mirrored_nav_fixture());
    let sql = "select * from app.";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::MySql,
        &index,
        CompletionContext::default(),
    );
    assert!(candidates
        .iter()
        .any(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "orders"));
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Schema),
        "mysql must not expose the mirrored schema node after a database qualifier"
    );
}

#[test]
fn sqlite_main_qualifier_offers_tables_not_schema_node() {
    let index = CompletionIndex::new(&mirrored_nav_fixture());
    let sql = "select * from app.";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Sqlite,
        &index,
        CompletionContext::default(),
    );
    assert!(candidates
        .iter()
        .any(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "orders"));
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Schema),
        "sqlite must not expose the mirrored schema node after a qualifier"
    );
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
fn order_by_clause_completion_without_catalog() {
    let sql = "select 1 where 1 = 1 orde";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &CompletionIndex::default(),
        CompletionContext::default(),
    );
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.label == "ORDER BY")
        .expect("a completed predicate should offer ORDER BY");

    assert_eq!(candidate.insert_text, "ORDER BY");
    assert_eq!(candidate.kind, CompletionKind::Keyword);
    assert_eq!(candidate.replace.start, sql.len() - "orde".len());
    assert_eq!(candidate.replace.end, sql.len());
}

#[test]
fn order_by_clause_completion_respects_expression_boundaries() {
    let index = CompletionIndex::default();
    for sql in [
        "select 1 where 1 = 1 orde",
        "select 1 from users orde",
        "select 1 from users group by 1 orde",
        "select 1 from users group by 1 having 1 = 1 orde",
        "select 1 orde",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.label == "ORDER BY"),
            "expected ORDER BY for {sql}: {candidates:?}"
        );
    }

    for sql in [
        "select 1 where 1 = orde",
        "select 1 where 1 between 1 and orde",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.label == "ORDER BY")
        );
    }
}

#[test]
fn order_by_clause_completion_handles_waiting_for_by() {
    let index = CompletionIndex::default();
    for sql in ["select 1 from users order ", "select 1 from users order b"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates.iter().any(|candidate| candidate.label == "BY"),
            "expected BY for {sql}: {candidates:?}"
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.label == "ORDER BY")
        );
    }
}

#[test]
fn order_by_clause_completion_replaces_only_the_current_prefix() {
    for (sql, expected, label) in [
        ("select 1 from users ord", "ORDER BY", "ORDER BY"),
        ("select 1 from users order b", "BY", "BY"),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &CompletionIndex::default(),
            CompletionContext::default(),
        );
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.label == label)
            .expect("expected ORDER BY replacement candidate");
        let replaced = format!(
            "{}{}{}",
            &sql[..candidate.replace.start],
            expected,
            &sql[candidate.replace.end..]
        );
        assert_eq!(
            replaced,
            if label == "ORDER BY" {
                "select 1 from users ORDER BY"
            } else {
                "select 1 from users order BY"
            }
        );
    }
}

#[test]
fn order_by_completion_offers_columns_then_direction() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let columns = complete(
        "select * from users u join roles r on u.id = r.id order by ",
        "select * from users u join roles r on u.id = r.id order by ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        columns
            .iter()
            .any(|candidate| candidate.label == "user_name")
    );
    assert!(!columns.iter().any(|candidate| candidate.label == "ASC"));

    let directions = complete(
        "select * from users u join roles r on u.id = r.id order by u.user_name ",
        "select * from users u join roles r on u.id = r.id order by u.user_name ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(directions.iter().any(|candidate| candidate.label == "ASC"));
    assert!(directions.iter().any(|candidate| candidate.label == "DESC"));
}

#[test]
fn order_by_completion_filters_direction_by_expression_state() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    for sql in [
        "select * from users u join roles r on u.id = r.id order by u.user_name + ",
        "select * from users u join roles r on u.id = r.id order by coalesce(u.user_name, ",
        "select * from users u join roles r on u.id = r.id order by u.user_name, ",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(!candidates.iter().any(|candidate| candidate.label == "ASC"));
        assert!(!candidates.iter().any(|candidate| candidate.label == "DESC"));
    }
}

#[test]
fn order_by_completion_filters_null_placement_by_dialect() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select * from users order by user_name ";
    for (dialect, supports_nulls) in [
        (SqlDialect::Postgres, true),
        (SqlDialect::Sqlite, true),
        (SqlDialect::Generic, true),
        (SqlDialect::MySql, false),
        (SqlDialect::SqlServer, false),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(
            candidates
                .iter()
                .any(|candidate| candidate.label == "NULLS FIRST"),
            supports_nulls,
            "unexpected NULLS support for {dialect:?}: {candidates:?}"
        );
    }
}

#[test]
fn order_by_completion_advances_after_direction_and_null_placement() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let after_direction = complete(
        "select * from users order by user_name desc ",
        "select * from users order by user_name desc ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        !after_direction
            .iter()
            .any(|candidate| candidate.label == "ASC")
    );
    assert!(
        !after_direction
            .iter()
            .any(|candidate| candidate.label == "DESC")
    );
    assert!(
        after_direction
            .iter()
            .any(|candidate| candidate.label == "NULLS FIRST")
    );

    let after_null_placement = complete(
        "select * from users order by user_name desc nulls last ",
        "select * from users order by user_name desc nulls last ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        !after_null_placement
            .iter()
            .any(|candidate| candidate.label == "ASC" || candidate.label == "NULLS FIRST")
    );
}

#[test]
fn order_by_completion_respects_relation_qualifiers() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select * from users u join roles r on u.id = r.id order by r.";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "role_name")
    );
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.label == "user_name")
    );
    assert!(!candidates.iter().any(|candidate| candidate.label == "ASC"));
}

#[test]
fn order_by_completion_dependencies_include_visible_relations() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "select * from users u join roles r on u.id = r.id order by ";
    let dependencies = completion_dependencies(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(dependencies.relation_children.len(), 2);
}

#[test]
fn ordering_completion_trigger_is_limited_to_ordering_positions() {
    for (sql, expected) in [
        ("select 1 from users order by user_name ", true),
        ("select 1 from users order by user_name, ", true),
        ("select 1 from users where user_name = ", false),
        ("select 'order by ' ", false),
        ("select 1 -- order by \n", false),
    ] {
        assert_eq!(
            should_offer_completion(sql, sql.len()),
            expected,
            "unexpected trigger result for {sql:?}"
        );
    }
}

#[test]
fn order_by_completion_keeps_nested_query_scopes_separate() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let outer = "select * from users u where u.id in (select r.id from roles r order by ";
    let candidates = complete(
        outer,
        outer.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "role_name")
    );

    let inner = "select * from users u where u.id in (select r.id from roles r order by r.";
    let candidates = complete(
        inner,
        inner.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "role_name")
    );
}

#[test]
fn delete_from_keyword_completion() {
    let index = CompletionIndex::new(&[]);
    for sql in [
        "DELETE f",
        "DELETE fr",
        "DELETE fro",
        "dElEtE FrO",
        "DELETE ",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(candidates.len(), 1, "{sql}: {candidates:?}");
        let candidate = &candidates[0];
        assert_eq!(candidate.label, "FROM");
        assert_eq!(candidate.insert_text, "FROM");
        assert_eq!(candidate.kind, CompletionKind::Keyword);
        assert_eq!(candidate.replace, TextRange::new(7, sql.len()));
        assert_eq!(candidate.score.context, 4);
    }
}

#[test]
fn delete_where_keyword_completion() {
    let index = CompletionIndex::new(&[]);
    for preceding in [
        "",
        "SELECT * FROM all_types_test;\nUPDATE all_types_test SET id = 1 WHERE id = 2;\n",
    ] {
        for prefix in ["w", "wh"] {
            let target = format!("{preceding}DELETE FROM all_types_test ");
            let sql = format!("{target}{prefix}");
            let candidates = complete(
                &sql,
                sql.len(),
                SqlDialect::Postgres,
                &index,
                CompletionContext::default(),
            );
            assert_eq!(candidates.len(), 1, "{sql}: {candidates:?}");
            let candidate = &candidates[0];
            assert_eq!(candidate.label, "WHERE");
            assert_eq!(candidate.insert_text, "WHERE");
            assert_eq!(candidate.kind, CompletionKind::Keyword);
            assert_eq!(candidate.replace, TextRange::new(target.len(), sql.len()));
            assert_eq!(candidate.score.context, 4);
        }
    }
}

#[test]
fn delete_where_offered_after_target_variants() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in [
        "DELETE FROM sys_user w",
        "DELETE FROM public.sys_user w",
        "DELETE FROM sys_user t w",
        "DELETE FROM sys_user AS t w",
        "DELETE FROM missing_table w",
        "DELETE FROM \"where\" w",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(candidates.len(), 1, "{sql}: {candidates:?}");
        let candidate = &candidates[0];
        assert_eq!(candidate.label, "WHERE", "{sql}: {candidates:?}");
        assert_eq!(candidate.insert_text, "WHERE");
        assert_eq!(candidate.kind, CompletionKind::Keyword);
        assert_eq!(candidate.replace, TextRange::new(sql.len() - 1, sql.len()));
        assert_eq!(candidate.score.context, 4);
    }

    let sql = "DELETE FROM sys_user ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(candidates.len(), 1, "{sql}: {candidates:?}");
    let candidate = &candidates[0];
    assert_eq!(candidate.label, "WHERE");
    assert_eq!(candidate.insert_text, "WHERE");
    assert_eq!(candidate.kind, CompletionKind::Keyword);
    assert_eq!(candidate.replace, TextRange::new(sql.len(), sql.len()));
    assert_eq!(candidate.score.context, 4);
}

#[test]
fn delete_where_suppressed_during_alias_entry() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in [
        "DELETE FROM sys_user AS",
        "DELETE FROM sys_user AS ",
        "DELETE FROM sys_user AS w",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(candidates.is_empty(), "{sql}: {candidates:?}");
    }
}

#[test]
fn delete_target_completion_keeps_relation_suggestions() {
    let index = CompletionIndex::new(&contextual_fixture());

    for sql in ["DELETE FROM sys_u", "DELETE FROM sys_user"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(candidates.len(), 1, "{sql}: {candidates:?}");
        assert_eq!(candidates[0].label, "sys_user", "{sql}: {candidates:?}");
        assert_eq!(
            candidates[0].kind,
            CompletionKind::Table,
            "{sql}: {candidates:?}"
        );
    }

    let sql = "DELETE FROM public.";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(candidates.len(), 3, "{sql}: {candidates:?}");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "sys_user"),
        "{sql}: {candidates:?}"
    );
    assert!(
        candidates.iter().all(|candidate| {
            candidate.label != "WHERE" && matches!(candidate.kind, CompletionKind::Table)
        }),
        "{sql}: {candidates:?}"
    );
}

#[test]
fn delete_where_keeps_target_column_completion() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "DELETE FROM sys_user WHERE u";
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
            .filter(|candidate| candidate.kind == CompletionKind::Column)
            .count(),
        5,
        "{sql}: {candidates:?}"
    );
    for label in [
        "update_time",
        "update_user",
        "update_user_phone",
        "user_type",
        "username",
    ] {
        assert!(
            candidates.iter().any(|candidate| candidate.label == label),
            "{sql}: missing {label} in {candidates:?}"
        );
    }
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.label != "WHERE"),
        "{sql}: {candidates:?}"
    );
}

#[test]
fn delete_where_absent_with_qualified_predicate() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "DELETE FROM sys_user WHERE t.";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(candidates.is_empty(), "{sql}: {candidates:?}");
}

#[test]
fn delete_completion_preserves_suffix_and_uses_document_relative_range() {
    let index = CompletionIndex::new(&[]);
    let sql = "SELECT 1;\n-- keep this\nDELETE FROM users wh AND id = 1";
    let cursor = sql.find("wh AND").unwrap() + 2;
    let candidates = complete(
        sql,
        cursor,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let candidate = &candidates[0];
    assert_eq!(candidate.label, "WHERE");
    assert_eq!(candidate.replace, TextRange::new(cursor - 2, cursor));
    let mut completed = sql.to_owned();
    completed.replace_range(
        candidate.replace.start..candidate.replace.end,
        &candidate.insert_text,
    );
    assert_eq!(
        completed,
        "SELECT 1;\n-- keep this\nDELETE FROM users WHERE AND id = 1"
    );
}

#[test]
fn delete_completion_ignores_comments_and_uses_unicode_byte_ranges() {
    let mut entries = contextual_fixture();
    let schema = entries
        .iter()
        .find(|entry| entry.kind == CatalogKind::Table)
        .and_then(|entry| entry.parent_id.as_ref())
        .unwrap()
        .clone();
    entries.push(
        CatalogEntry::relation(
            CatalogId::new(
                schema.profile_id(),
                CatalogKind::Table,
                ["app", "public", "用户"],
            ),
            schema,
            qualified("app", Some("public"), "用户"),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap(),
    );
    let index = CompletionIndex::new(&entries);
    let sql = "-- 前置\nSELECT 1;\nDELETE FROM 用 AND id = 1";
    let cursor = sql.find("用 AND").unwrap() + "用".len();
    let candidates = complete(
        sql,
        cursor,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let candidate = &candidates[0];
    assert_eq!(candidate.label, "用户");
    assert_eq!(
        candidate.replace,
        TextRange::new(cursor - "用".len(), cursor)
    );
    assert_eq!(&sql[cursor..], " AND id = 1");
}

#[test]
fn delete_completion_keeps_cte_and_nested_select_scopes_isolated() {
    let index = CompletionIndex::new(&multi_relation_fixture());

    let cte_sql = "WITH recent AS (SELECT * FROM users u) SELECT * FROM roles r WHERE u.";
    let cte_candidates = complete(
        cte_sql,
        cte_sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(cte_candidates.is_empty(), "{cte_candidates:?}");

    let nested_sql = "SELECT * FROM users u WHERE u.id IN (SELECT r.id FROM roles r WHERE r.";
    let nested_candidates = complete(
        nested_sql,
        nested_sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(nested_candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Column && candidate.label == "role_name"
    }));
    assert!(
        !nested_candidates
            .iter()
            .any(|candidate| candidate.label == "user_name")
    );
}

#[test]
fn completion_with_empty_index_keeps_delete_from_and_where_independent() {
    let index = CompletionIndex::default();
    for (sql, expected) in [("DELETE f", "FROM"), ("DELETE FROM users w", "WHERE")] {
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
                .map(|candidate| candidate.label.as_str())
                .collect::<Vec<_>>(),
            [expected]
        );
    }
}

#[test]
fn delete_completion_dependencies_retain_target_relation_children() {
    let entries = contextual_fixture();
    let index = CompletionIndex::new(&entries);
    let target = entries
        .iter()
        .find(|entry| entry.kind == CatalogKind::Table && entry.qualified_name.object == "sys_user")
        .unwrap()
        .id
        .clone();
    let dependencies = completion_dependencies(
        "DELETE FROM sys_user WHERE ",
        "DELETE FROM sys_user WHERE ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(dependencies.relation_children, vec![target]);
}

#[test]
fn delete_from_keyword_completion_across_dialects() {
    let index = CompletionIndex::new(&[]);
    for dialect in [
        SqlDialect::Postgres,
        SqlDialect::MySql,
        SqlDialect::Sqlite,
        SqlDialect::SqlServer,
        SqlDialect::Generic,
    ] {
        for sql in [
            "DELETE f",
            "DELETE fr",
            "DELETE fro",
            "dElEtE FrO",
            "DELETE ",
        ] {
            let candidates = complete(
                sql,
                sql.len(),
                dialect,
                &index,
                CompletionContext::default(),
            );
            assert_eq!(candidates.len(), 1, "{sql} {dialect:?}: {candidates:?}");
            let candidate = &candidates[0];
            assert_eq!(candidate.label, "FROM");
            assert_eq!(candidate.insert_text, "FROM");
            assert_eq!(candidate.kind, CompletionKind::Keyword);
            assert_eq!(candidate.replace, TextRange::new(7, sql.len()));
            assert_eq!(candidate.score.context, 4);
        }
    }
}

#[test]
fn delete_where_after_plain_target_across_dialects() {
    let index = CompletionIndex::new(&[]);
    for dialect in [
        SqlDialect::Postgres,
        SqlDialect::MySql,
        SqlDialect::Sqlite,
        SqlDialect::SqlServer,
        SqlDialect::Generic,
    ] {
        for (sql, prefix_len) in [
            ("DELETE FROM sys_user w", 1),
            ("DELETE FROM sys_user wh", 2),
            ("DELETE FROM sys_user ", 0),
        ] {
            let candidates = complete(
                sql,
                sql.len(),
                dialect,
                &index,
                CompletionContext::default(),
            );
            assert_eq!(candidates.len(), 1, "{sql} {dialect:?}: {candidates:?}");
            let candidate = &candidates[0];
            assert_eq!(candidate.label, "WHERE");
            assert_eq!(candidate.insert_text, "WHERE");
            assert_eq!(candidate.kind, CompletionKind::Keyword);
            assert_eq!(
                candidate.replace,
                TextRange::new(sql.len() - prefix_len, sql.len())
            );
            assert_eq!(candidate.score.context, 4);
        }
    }
}

#[test]
fn delete_where_after_quoted_target_across_dialects() {
    let index = CompletionIndex::new(&[]);
    for dialect in [
        SqlDialect::Postgres,
        SqlDialect::MySql,
        SqlDialect::Sqlite,
        SqlDialect::SqlServer,
        SqlDialect::Generic,
    ] {
        let target = match dialect {
            SqlDialect::MySql => "`where`",
            SqlDialect::SqlServer => "[where]",
            _ => "\"where\"",
        };
        let sql = format!("DELETE FROM {target} w");
        let candidates = complete(
            &sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(candidates.len(), 1, "{sql} {dialect:?}: {candidates:?}");
        let candidate = &candidates[0];
        assert_eq!(candidate.label, "WHERE");
        assert_eq!(candidate.insert_text, "WHERE");
        assert_eq!(candidate.kind, CompletionKind::Keyword);
        assert_eq!(candidate.replace, TextRange::new(sql.len() - 1, sql.len()));
        assert_eq!(candidate.score.context, 4);
    }
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
fn automatic_completion_requires_an_identifier_prefix() {
    for sql in [
        "",
        " ",
        "\n",
        "select ",
        "select * from users;",
        "select * from users;\n",
    ] {
        assert!(!should_offer_completion(sql, sql.len()), "{sql:?}");
    }
    for sql in [
        "s",
        "select * from us",
        "select u.",
        "select u1.",
        "select * from public.",
    ] {
        assert!(should_offer_completion(sql, sql.len()), "{sql:?}");
    }
}

#[test]
fn automatic_completion_ignores_non_identifier_dots_and_literals() {
    for sql in ["1.", "select 'users'", "-- users", "select * from users; "] {
        assert!(!should_offer_completion(sql, sql.len()), "{sql:?}");
    }
}

#[test]
fn automatic_completion_dialect_trigger_boundaries_remain_stable() {
    for dialect in [
        SqlDialect::Postgres,
        SqlDialect::MySql,
        SqlDialect::Sqlite,
        SqlDialect::SqlServer,
        SqlDialect::Generic,
    ] {
        for (sql, expected) in [
            ("fro", true),
            ("w", true),
            ("-- fro", false),
            ("SELECT 'w'", false),
            ("   ", false),
            ("\n\t", false),
        ] {
            assert_eq!(
                lazydb::sql::should_offer_completion_for_dialect(sql, sql.len(), dialect),
                expected,
                "{dialect:?} {sql:?}"
            );
        }
    }
}

#[test]
fn ddl_completion_trigger_matches_structural_context() {
    for sql in [
        "CREATE ",
        "ALTER ",
        "DROP ",
        "TRUNCATE TABLE ",
        "CREATE INDEX ix ON ",
    ] {
        assert!(!should_offer_completion(sql, sql.len()), "{sql}");
    }
    for sql in ["ALTER TABLE users ", "ALTER TABLE users DROP COLUMN "] {
        assert!(should_offer_completion(sql, sql.len()), "{sql}");
    }
    for sql in [
        "SELECT 'CREATE '",
        "-- CREATE ",
        "CREATE TABLE \"drop\" (id INTEGER)",
    ] {
        assert!(!should_offer_completion(sql, sql.len()), "{sql}");
    }
}

#[test]
fn ddl_completion_handles_incomplete_input_without_global_leakage() {
    let index = CompletionIndex::new(&fixture());
    for (sql, dialect) in [
        ("CREATE", SqlDialect::Postgres),
        ("CREATE TABLE", SqlDialect::Postgres),
        ("CREATE TABLE users (", SqlDialect::Postgres),
        ("CREATE TABLE users (id VARCHAR(", SqlDialect::Postgres),
        ("ALTER TABLE users DROP", SqlDialect::Postgres),
        ("REFERENCES users (", SqlDialect::Postgres),
        ("DROP TABLE \"unterminated", SqlDialect::Postgres),
        (
            "CREATE TABLE users (label TEXT DEFAULT 'unterminated",
            SqlDialect::Postgres,
        ),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext::default(),
        );
        assert!(candidates.len() <= 10, "{sql}: {candidates:?}");
        assert!(
            !candidates.iter().any(|candidate| {
                matches!(
                    candidate.kind,
                    CompletionKind::Database | CompletionKind::Schema
                )
            }),
            "incomplete DDL leaked namespace candidates for {sql}: {candidates:?}"
        );
    }
}

#[test]
fn ddl_completion_only_uses_the_statement_at_cursor() {
    let index = CompletionIndex::new(&fixture());
    let first = "SELECT * FROM users; DROP TABLE us";
    let candidates = complete(
        first,
        first.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Table)
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "users")
    );

    let second = "CREATE TABLE draft (id INTEGER); SELECT * FROM us";
    let candidates = complete(
        second,
        second.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(candidates.iter().all(|candidate| {
        candidate.kind == CompletionKind::Table || candidate.kind == CompletionKind::View
    }));

    let third = "CREATE VIEW draft AS SELECT * FROM users; ALTER TABLE us";
    let candidates = complete(
        third,
        third.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Table)
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "users"),
        "{third}: {candidates:?}"
    );
}

#[test]
fn alter_table_target_completion_matches_drop_table() {
    let index = CompletionIndex::new(&fixture());
    for sql in ["ALTER TABLE us", "Alter table us", "DROP TABLE us"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates.iter().any(|candidate| {
                candidate.kind == CompletionKind::Table && candidate.label == "users"
            }),
            "{sql}: {candidates:?}"
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind == CompletionKind::Table)
        );
    }
}

#[test]
fn alter_table_operation_completion_is_dialect_specific() {
    let index = CompletionIndex::new(&fixture());

    let mysql = complete(
        "ALTER TABLE users M",
        "ALTER TABLE users M".len(),
        SqlDialect::MySql,
        &index,
        CompletionContext::default(),
    );
    assert!(
        mysql
            .iter()
            .any(|candidate| candidate.label == "MODIFY COLUMN"),
        "{mysql:?}"
    );

    let postgres = complete(
        "ALTER TABLE users M",
        "ALTER TABLE users M".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        !postgres
            .iter()
            .any(|candidate| candidate.label == "MODIFY COLUMN")
    );

    let add_column = complete(
        "ALTER TABLE users ADD C",
        "ALTER TABLE users ADD C".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        add_column
            .iter()
            .any(|candidate| { candidate.label == "COLUMN" && candidate.insert_text == "COLUMN" }),
        "{add_column:?}"
    );
}

#[test]
fn statement_completion_offers_ddl_commands() {
    let index = CompletionIndex::default();
    for (sql, expected) in [
        ("cre", "CREATE"),
        ("alt", "ALTER"),
        ("dro", "DROP"),
        ("tru", "TRUNCATE"),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(
            candidates.first().map(|candidate| candidate.label.as_str()),
            Some(expected)
        );
    }
}

#[test]
fn ddl_object_and_type_keywords_follow_dialect_matrix() {
    for (dialect, expected, rejected) in [
        (
            SqlDialect::Postgres,
            &[
                "TABLE",
                "VIEW",
                "MATERIALIZED VIEW",
                "SEQUENCE",
                "TYPE",
                "TRIGGER",
            ][..],
            &["AUTOINCREMENT"][..],
        ),
        (
            SqlDialect::MySql,
            &["TABLE", "VIEW", "TRIGGER"][..],
            &["MATERIALIZED VIEW", "SEQUENCE", "TYPE"][..],
        ),
        (
            SqlDialect::SqlServer,
            &["TABLE", "VIEW", "TRIGGER"][..],
            &["MATERIALIZED VIEW", "TYPE"][..],
        ),
        (
            SqlDialect::Sqlite,
            &["TABLE", "VIEW", "TRIGGER"][..],
            &["MATERIALIZED VIEW", "SEQUENCE", "TYPE"][..],
        ),
        (
            SqlDialect::DuckDb,
            &["TABLE", "VIEW", "SEQUENCE", "TYPE", "MACRO", "FUNCTION"][..],
            &["MATERIALIZED VIEW", "TRIGGER", "AUTOINCREMENT"][..],
        ),
    ] {
        for keyword in expected {
            let sql = format!(
                "CREATE {}",
                &keyword[..keyword
                    .char_indices()
                    .nth(1)
                    .map_or(keyword.len(), |(index, _)| index)]
            );
            let candidates = complete(
                &sql,
                sql.len(),
                dialect,
                &CompletionIndex::default(),
                CompletionContext::default(),
            );
            assert!(
                candidates
                    .iter()
                    .any(|candidate| candidate.label == *keyword),
                "{dialect:?}: {keyword}: {candidates:?}"
            );
        }
        for keyword in rejected {
            let prefix = &keyword[..keyword
                .char_indices()
                .nth(1)
                .map_or(keyword.len(), |(index, _)| index)];
            let sql = format!("CREATE {prefix}");
            let candidates = complete(
                &sql,
                sql.len(),
                dialect,
                &CompletionIndex::default(),
                CompletionContext::default(),
            );
            assert!(
                !candidates
                    .iter()
                    .any(|candidate| candidate.label == *keyword),
                "{dialect:?}: {keyword}: {candidates:?}"
            );
        }
    }

    for (dialect, sql, expected, rejected) in [
        (
            SqlDialect::Postgres,
            "CREATE TABLE t (id js",
            "JSONB",
            "JSON",
        ),
        (SqlDialect::MySql, "CREATE TABLE t (id js", "JSON", "JSONB"),
        (
            SqlDialect::SqlServer,
            "CREATE TABLE t (id nv",
            "NVARCHAR",
            "JSONB",
        ),
        (
            SqlDialect::Sqlite,
            "CREATE TABLE t (id in",
            "INTEGER",
            "JSONB",
        ),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &CompletionIndex::default(),
            CompletionContext::default(),
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.label == expected),
            "{dialect:?}: {candidates:?}"
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.label == rejected),
            "{dialect:?}: {candidates:?}"
        );
    }
}

#[test]
fn ddl_object_completion_filters_catalog_kind() {
    let index = CompletionIndex::new(&fixture());
    let candidates = complete(
        "DROP TABLE us",
        13,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "users"),
        "{candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Table)
    );
}

#[test]
fn ddl_keywords_ignore_quoted_identifiers_and_literals() {
    let index = CompletionIndex::default();
    let quoted_sql = "CREATE TABLE t (\"drop\" INTEGER, na";
    let quoted = complete(
        quoted_sql,
        quoted_sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(!quoted.iter().any(|candidate| candidate.label == "DROP"));
    let literal = complete(
        "SELECT 'create' WHERE cre",
        26,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(!literal.iter().any(|candidate| candidate.label == "CREATE"));
}

#[test]
fn create_table_completion_offers_dialect_data_types() {
    let index = CompletionIndex::default();
    for (dialect, sql, expected, rejected) in [
        (
            SqlDialect::Postgres,
            "CREATE TABLE t (id j",
            "JSONB",
            "JSON",
        ),
        (SqlDialect::MySql, "CREATE TABLE t (id j", "JSON", "JSONB"),
        (
            SqlDialect::SqlServer,
            "CREATE TABLE t (id unique",
            "UNIQUEIDENTIFIER",
            "JSONB",
        ),
        (SqlDialect::Sqlite, "CREATE TABLE t (id var", "", "VARCHAR"),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            dialect,
            &index,
            CompletionContext::default(),
        );
        if expected.is_empty() {
            assert!(
                candidates
                    .iter()
                    .all(|candidate| candidate.label != rejected)
            );
        } else {
            assert!(
                candidates.iter().any(|candidate| {
                    candidate.kind == CompletionKind::DataType && candidate.label == expected
                }),
                "{dialect:?} {sql}: {candidates:?}"
            );
            assert!(
                candidates
                    .iter()
                    .all(|candidate| candidate.label != rejected)
            );
        }
    }
}

#[test]
fn create_table_completion_offers_column_constraints_after_type() {
    let index = CompletionIndex::default();
    let sql = "CREATE TABLE t (id INTEGER n";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Keyword && candidate.label == "NOT NULL"
        }),
        "{candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Keyword)
    );
}

#[test]
fn ddl_completion_covers_table_elements_alter_actions_and_children() {
    let mut entries = fixture();
    let users = entries[2].id.clone();
    for (kind, name) in [
        (CatalogKind::Column, "id"),
        (CatalogKind::Column, "email"),
        (CatalogKind::Index, "ix_users_email"),
        (CatalogKind::PrimaryKey, "users_pkey"),
        (CatalogKind::UniqueConstraint, "users_email_key"),
        (CatalogKind::ForeignKey, "users_role_fkey"),
        (CatalogKind::CheckConstraint, "users_email_check"),
    ] {
        let metadata = match kind {
            CatalogKind::Column => CatalogMetadata::Column(ColumnMetadata::new(2, "text", true)),
            CatalogKind::Index => CatalogMetadata::Index(lazydb::db::catalog::IndexMetadata {
                columns: vec!["email".into()],
                unique: false,
            }),
            CatalogKind::PrimaryKey => {
                CatalogMetadata::Constraint(lazydb::db::catalog::ConstraintMetadata::PrimaryKey {
                    columns: vec!["id".into()],
                })
            }
            CatalogKind::UniqueConstraint => {
                CatalogMetadata::Constraint(lazydb::db::catalog::ConstraintMetadata::Unique {
                    columns: vec!["email".into()],
                })
            }
            CatalogKind::ForeignKey => {
                CatalogMetadata::Constraint(lazydb::db::catalog::ConstraintMetadata::ForeignKey {
                    columns: vec!["id".into()],
                    referenced_relation: qualified("app", Some("public"), "roles"),
                    referenced_columns: vec!["id".into()],
                })
            }
            CatalogKind::CheckConstraint => {
                CatalogMetadata::Constraint(lazydb::db::catalog::ConstraintMetadata::Check {
                    expression: "true".into(),
                })
            }
            _ => unreachable!(),
        };
        entries.push(
            CatalogEntry::relation_child(
                CatalogId::new(users.profile_id(), kind, ["app", "public", "users", name]),
                users.clone(),
                qualified("app", Some("public"), name),
                "child",
                OptionalMetadata::Supported(None),
                metadata,
            )
            .unwrap(),
        );
    }
    let index = CompletionIndex::new(&entries);
    let candidates = complete(
        "ALTER TABLE users ",
        "ALTER TABLE users ".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "ADD COLUMN")
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.label == "DROP COLUMN")
    );

    for (sql, kind, expected) in [
        (
            "ALTER TABLE users MODIFY COLUMN e",
            CompletionKind::Column,
            "email",
        ),
        (
            "ALTER TABLE users DROP COLUMN e",
            CompletionKind::Column,
            "email",
        ),
        (
            "ALTER TABLE users ALTER COLUMN i",
            CompletionKind::Column,
            "id",
        ),
        (
            "ALTER TABLE users DROP CONSTRAINT users_",
            CompletionKind::Constraint,
            "users_email_key",
        ),
        (
            "ALTER TABLE users DROP INDEX ix_",
            CompletionKind::Index,
            "ix_users_email",
        ),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == kind && candidate.label == expected),
            "{sql}: {candidates:?}"
        );
        assert!(candidates.iter().all(|candidate| candidate.kind == kind));
    }
}

#[test]
fn ddl_completion_reports_shared_relation_child_dependencies() {
    let index = CompletionIndex::new(&fixture());
    let users = index
        .entries()
        .iter()
        .find(|entry| entry.kind == CatalogKind::Table)
        .unwrap()
        .id
        .clone();
    for sql in [
        "SELECT u. FROM users u",
        "ALTER TABLE users DROP COLUMN ",
        "CREATE INDEX ix_users ON users (",
        "REFERENCES users (",
    ] {
        let dependencies = completion_dependencies(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert_eq!(dependencies.relation_children, vec![users.clone()], "{sql}");
    }
    assert!(
        completion_dependencies(
            "DROP TABLE users",
            "DROP TABLE users".len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        )
        .relation_children
        .is_empty()
    );
}

#[test]
fn references_and_create_index_complete_target_columns() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in [
        "CREATE TABLE orders (user_id BIGINT REFERENCES users (u",
        "CREATE INDEX ix_users ON users (u",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates.iter().any(|candidate| {
                candidate.kind == CompletionKind::Column && candidate.label == "username"
            }),
            "{sql}: {candidates:?}"
        );
        assert!(candidates.iter().all(|candidate| {
            candidate.kind == CompletionKind::Column || candidate.kind == CompletionKind::Keyword
        }));
    }
}

#[test]
fn ddl_existing_object_targets_are_strictly_filtered() {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let schema = entries[1].id.clone();
    for (kind, name) in [
        (CatalogKind::View, "report"),
        (CatalogKind::Index, "ix_users"),
        (CatalogKind::Sequence, "order_seq"),
        (CatalogKind::Type, "status_type"),
        (CatalogKind::Function, "format_user"),
        (CatalogKind::Procedure, "refresh_users"),
        (CatalogKind::Trigger, "users_trigger"),
    ] {
        let entry = if matches!(kind, CatalogKind::View | CatalogKind::MaterializedView) {
            CatalogEntry::relation(
                CatalogId::new(connection, kind, ["app", "public", name]),
                schema.clone(),
                qualified("app", Some("public"), name),
                "view",
                OptionalMetadata::Supported(None),
                false,
            )
        } else if kind == CatalogKind::Trigger {
            CatalogEntry::relation_object(
                CatalogId::new(connection, kind, ["app", "public", name]),
                schema.clone(),
                entries[2].id.clone(),
                qualified("app", Some("public"), name),
                "trigger",
                OptionalMetadata::Supported(None),
            )
        } else if kind == CatalogKind::Index {
            CatalogEntry::relation_child(
                CatalogId::new(connection, kind, ["app", "public", "users", name]),
                entries[2].id.clone(),
                qualified("app", Some("public"), name),
                "index",
                OptionalMetadata::Supported(None),
                CatalogMetadata::Index(lazydb::db::catalog::IndexMetadata {
                    columns: Vec::new(),
                    unique: false,
                }),
            )
        } else {
            CatalogEntry::object(
                CatalogId::new(connection, kind, ["app", "public", name]),
                schema.clone(),
                qualified("app", Some("public"), name),
                "object",
                OptionalMetadata::Supported(None),
                false,
            )
        };
        entries.push(entry.unwrap());
    }
    let index = CompletionIndex::new(&entries);
    for (sql, kind, name) in [
        ("DROP VIEW rep", CompletionKind::View, "report"),
        ("DROP INDEX ix_", CompletionKind::Index, "ix_users"),
        ("DROP SEQUENCE ord", CompletionKind::Sequence, "order_seq"),
        ("DROP TYPE sta", CompletionKind::Type, "status_type"),
        ("DROP FUNCTION for", CompletionKind::Function, "format_user"),
        (
            "DROP PROCEDURE ref",
            CompletionKind::Procedure,
            "refresh_users",
        ),
        ("DROP TRIGGER use", CompletionKind::Trigger, "users_trigger"),
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == kind && candidate.label == name),
            "{sql}: {candidates:?}"
        );
        assert!(
            candidates.iter().all(|candidate| candidate.kind == kind),
            "{sql}: {candidates:?}"
        );
    }
}

#[test]
fn ddl_existing_objects_support_qualified_names() {
    let mut entries = fixture();
    let index_id = CatalogId::new(
        entries[2].id.profile_id(),
        CatalogKind::Index,
        ["app", "public", "users", "ix_users"],
    );
    entries.push(
        CatalogEntry::relation_child(
            index_id,
            entries[2].id.clone(),
            qualified("app", Some("public"), "ix_users"),
            "index",
            OptionalMetadata::Supported(None),
            CatalogMetadata::Index(lazydb::db::catalog::IndexMetadata {
                columns: vec!["id".into()],
                unique: false,
            }),
        )
        .unwrap(),
    );
    let index = CompletionIndex::new(&entries);

    let table = complete(
        "DROP TABLE app.public.us",
        "DROP TABLE app.public.us".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        table.iter().any(|candidate| {
            candidate.kind == CompletionKind::Table
                && candidate.label == "users"
                && candidate.insert_text == "\"users\""
        }),
        "{table:?}"
    );
    assert!(
        table
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Table)
    );

    let object = complete(
        "DROP INDEX app.public.ix_",
        "DROP INDEX app.public.ix_".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(object.iter().any(|candidate| {
        candidate.kind == CompletionKind::Index && candidate.label == "ix_users"
    }));
    assert!(
        object
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Index)
    );
}

#[test]
fn create_index_and_view_handoff_complete_the_following_relation_or_query() {
    let index = CompletionIndex::new(&fixture());
    let create_index = "CREATE INDEX ix_users ON us";
    let candidates = complete(
        create_index,
        create_index.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "users")
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Table)
    );

    let index = CompletionIndex::new(&contextual_fixture());
    let view = "CREATE VIEW active_users AS sel";
    let candidates = complete(
        view,
        view.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(
        candidates.first().map(|candidate| candidate.label.as_str()),
        Some("SELECT")
    );

    let query = "CREATE VIEW active_users AS SELECT u FROM sys_user";
    let candidates = complete(
        query,
        "CREATE VIEW active_users AS SELECT u".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Column
                && candidate.label == "username"),
        "{candidates:?}"
    );
}

#[test]
fn sql_server_drop_index_on_completes_relation() {
    let index = CompletionIndex::new(&fixture());
    let sql = "DROP INDEX ix_users ON us";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::SqlServer,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "users")
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Table)
    );
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
            | CatalogKind::Sequence
            | CatalogKind::Type
            | CatalogKind::Index
            | CatalogKind::PrimaryKey
            | CatalogKind::UniqueConstraint
            | CatalogKind::ForeignKey
            | CatalogKind::CheckConstraint
            | CatalogKind::Trigger
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
    app.update(Action::NewConsole);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16.4".into(),
            database: "app".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    app.active_console_mut()
        .execution_target
        .as_mut()
        .unwrap()
        .schema = Some("public".into());
    app.explorer
        .completion_indexes
        .insert(profile_id, CompletionIndex::new(&entries));
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
fn offline_console_keeps_local_completion_without_catalog_objects_or_requests() {
    let profile = import_connection_url("postgres://localhost/app", Some("app"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.reveal_startup_profile(None);
    app.update(Action::NewConsole);
    app.update(Action::ReplaceEditor("SELECT CURRENT_TIM".into()));
    for key in ['G', '$', 'a'] {
        app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(key),
            crossterm::event::KeyModifiers::NONE,
        )));
    }

    let commands = app.update(Action::CompletionExplicit);
    let popup = app.active_console().completion.as_ref().unwrap();

    assert!(
        popup
            .candidates
            .iter()
            .any(|candidate| candidate.label == "CURRENT_TIMESTAMP")
    );
    assert!(popup.candidates.iter().all(|candidate| !matches!(
        candidate.kind,
        CompletionKind::Table | CompletionKind::Column
    )));
    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            Command::Connect { .. } | Command::LoadCatalogPage { .. }
        )
    }));
    assert!(!app.explorer.completion_indexes.contains_key(&profile_id));
}

fn completion_app_with_table() -> (App, Uuid, CatalogEntry) {
    let mut profile = import_connection_url("postgres://localhost/app", Some("app"))
        .unwrap()
        .profile;
    profile.catalog_scope =
        CatalogScope::for_profile(DatabaseKind::Postgres, "app", Some("public"));
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.update(Action::NewConsole);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16.4".into(),
            database: "app".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    let table = completion_table(profile_id);
    let database = CatalogEntry::database(
        CatalogId::new(profile_id, CatalogKind::Database, ["app"]),
        qualified("app", None, "app"),
        "database",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]),
        database.id.clone(),
        qualified("app", Some("public"), "public"),
        "schema",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog
        .insert_subtree(vec![database, schema, table.clone()])
        .unwrap();
    app.explorer.completion_indexes.insert(
        profile_id,
        CompletionIndex::new(std::slice::from_ref(&table)),
    );
    (app, profile_id, table)
}

fn completion_page(request: &CatalogRequest, entries: Vec<CatalogEntry>) -> CatalogPage {
    CatalogPage::new(request, entries, CatalogCount::Exact(1), None).unwrap()
}

fn completion_table(profile_id: Uuid) -> CatalogEntry {
    CatalogEntry::relation(
        CatalogId::new(profile_id, CatalogKind::Table, ["app", "public", "users"]),
        CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]),
        qualified("app", Some("public"), "users"),
        "table",
        OptionalMetadata::Supported(None),
        false,
    )
    .unwrap()
}

fn completion_column(profile_id: Uuid, table: &CatalogEntry) -> CatalogEntry {
    CatalogEntry::relation_child(
        CatalogId::new(
            profile_id,
            CatalogKind::Column,
            ["app", "public", "users", "user_id"],
        ),
        table.id.clone(),
        qualified("app", Some("public"), "user_id"),
        "column",
        OptionalMetadata::Unsupported,
        CatalogMetadata::Column(ColumnMetadata::new(1, "bigint", false)),
    )
    .unwrap()
}

#[test]
fn app_completion_requests_missing_relation_children() {
    let (mut app, profile_id, table) = completion_app_with_table();
    app.update(Action::ReplaceEditor(
        "select * from users u where u.".into(),
    ));
    app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('A'),
        crossterm::event::KeyModifiers::NONE,
    )));

    let commands = app.update(Action::CompletionExplicit);
    assert!(matches!(
        commands.as_slice(),
        [Command::LoadCatalogPage(request)]
            if request.key.target == CatalogTarget::relation_children(table.id.clone()).unwrap()
    ));
    assert!(matches!(
        &commands[0],
        Command::LoadCatalogPage(request) if request.key.connection.profile_id == profile_id
    ));
}

#[test]
fn app_completion_refreshes_popup_when_relation_children_load() {
    let (mut app, profile_id, table) = completion_app_with_table();
    let column = completion_column(profile_id, &table);
    app.update(Action::ReplaceEditor(
        "select * from users u where u.".into(),
    ));
    app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('A'),
        crossterm::event::KeyModifiers::NONE,
    )));
    let commands = app.update(Action::CompletionExplicit);
    let request = match &commands[..] {
        [Command::LoadCatalogPage(request)] => request.clone(),
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::CatalogPageLoaded(completion_page(
        &request,
        vec![column.clone()],
    )));

    let popup = app.active_console().completion.as_ref().unwrap();
    assert!(
        popup
            .candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Column
                && candidate.label == "user_id")
    );
}

#[test]
fn app_completion_does_not_request_already_loaded_relation_children() {
    let (mut app, profile_id, table) = completion_app_with_table();
    let column = completion_column(profile_id, &table);
    app.update(Action::ReplaceEditor(
        "select * from users u where u.".into(),
    ));
    app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('A'),
        crossterm::event::KeyModifiers::NONE,
    )));
    let request = match app.update(Action::CompletionExplicit).as_slice() {
        [Command::LoadCatalogPage(request)] => request.clone(),
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::CatalogPageLoaded(completion_page(
        &request,
        vec![column],
    )));

    let commands = app.update(Action::CompletionExplicit);
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadCatalogPage(_)))
    );
}

#[test]
fn stale_completion_catalog_response_does_not_restore_popup_after_edit() {
    let (mut app, profile_id, table) = completion_app_with_table();
    let column = completion_column(profile_id, &table);
    app.update(Action::ReplaceEditor(
        "select * from users u where u.".into(),
    ));
    app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('A'),
        crossterm::event::KeyModifiers::NONE,
    )));
    let request = match app.update(Action::CompletionExplicit).as_slice() {
        [Command::LoadCatalogPage(request)] => request.clone(),
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ReplaceEditor(
        "select * from users u where u.user".into(),
    ));

    app.update(Action::CatalogPageLoaded(completion_page(
        &request,
        vec![column],
    )));

    assert!(app.active_console().completion.is_none());
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

fn sqlserver_update_fixture() -> Vec<CatalogEntry> {
    let connection = Uuid::new_v4();
    let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let mut entries = Vec::new();
    for (table_name, columns) in [("users", &["id", "user_name"][..]), ("u", &["dumb"][..])] {
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

fn update_context() -> CompletionContext<'static> {
    CompletionContext {
        database: Some("app"),
        schema: Some("public"),
    }
}

#[test]
fn update_where_completion_after_complete_assignment() {
    let index = CompletionIndex::new(&[]);
    for dialect in [
        SqlDialect::Generic,
        SqlDialect::Postgres,
        SqlDialect::MySql,
        SqlDialect::Sqlite,
        SqlDialect::SqlServer,
    ] {
        for assignment in [
            "small_num = 1",
            "small_num = NULL",
            "small_num = other_num",
            "small_num = 1 + 2",
            "small_num = abs(-1)",
            "small_num = (select 1)",
            "small_num = CASE WHEN 1 = 1 THEN 2 ELSE 3 END",
            "small_num = 1, other_num = 2",
        ] {
            for prefix in ["", "w", "wh", "WhErE"] {
                let before = format!("update all_types_test SET {assignment} {prefix}");
                let sql = format!("{before} small_num > 0");
                let candidates = complete(&sql, before.len(), dialect, &index, update_context());
                let candidate = candidates
                    .iter()
                    .find(|candidate| candidate.label == "WHERE")
                    .unwrap_or_else(|| panic!("{dialect:?}: {sql}: {candidates:?}"));
                assert_eq!(candidate.kind, CompletionKind::Keyword);
                assert_eq!(candidate.insert_text, "WHERE");
                assert_eq!(
                    candidate.replace,
                    TextRange::new(before.len() - prefix.len(), before.len())
                );
            }
        }
    }
    for sql in [
        "update all_types_test SET \"small_num\" = 1 w",
        "select 1; update all_types_test set small_num = 1 w",
        "update all_types_test set small_num = 1 -- value\nw",
        "with t as (select 1) update all_types_test set small_num = 1 w",
    ] {
        assert!(
            has_builtin(
                &complete(
                    sql,
                    sql.len(),
                    SqlDialect::Postgres,
                    &index,
                    update_context()
                ),
                "WHERE",
                CompletionKind::Keyword
            ),
            "{sql}"
        );
    }
}

#[test]
fn update_where_completion_rejects_incomplete_assignments() {
    let index = CompletionIndex::new(&[]);
    for sql in [
        "update all_types_test w",
        "update all_types_test set w",
        "update all_types_test set small_num = w",
        "update all_types_test set small_num = 1 + w",
        "update all_types_test set small_num = (1 w",
        "update all_types_test set small_num = abs(1 w",
        "update all_types_test set small_num = CASE WHEN 1 = 1 THEN 2 w",
        "update all_types_test set small_num = 1, w",
        "update all_types_test set small_num = 1, other_num = w",
        "update all_types_test set small_num = (select 1 w",
        "update all_types_test set small_num = 1 where w",
        "update all_types_test set small_num = t.w",
    ] {
        assert!(
            !has_builtin(
                &complete(
                    sql,
                    sql.len(),
                    SqlDialect::Postgres,
                    &index,
                    update_context()
                ),
                "WHERE",
                CompletionKind::Keyword
            ),
            "{sql}"
        );
    }
}

#[test]
fn update_target_completion_offers_set_keyword() {
    let index = CompletionIndex::new(&contextual_fixture());
    for dialect in [
        SqlDialect::Generic,
        SqlDialect::Postgres,
        SqlDialect::MySql,
        SqlDialect::Sqlite,
        SqlDialect::SqlServer,
    ] {
        for before in [
            "update all_types_test ",
            "update all_types_test s",
            "update all_types_test se",
            "UPDATE all_types_test SeT",
            "update public.all_types_test s",
            "update \"all_types_test\" s",
            "update all_types_test t s",
            "update all_types_test as t s",
            "update all_types_test -- target\ns",
            "select 1; update all_types_test s",
            "with t as (select 1) update all_types_test s",
        ] {
            for suffix in ["", " username = 'x'"] {
                let sql = format!("{before}{suffix}");
                let candidates = complete(&sql, before.len(), dialect, &index, update_context());
                assert_eq!(candidates.len(), 1, "{dialect:?}: {sql}: {candidates:?}");
                let candidate = &candidates[0];
                assert_eq!(candidate.label, "SET", "{dialect:?}: {sql}");
                assert_eq!(candidate.kind, CompletionKind::Keyword);
                assert_eq!(candidate.insert_text, "SET");
                let prefix_start = before.rfind(char::is_whitespace).unwrap() + 1;
                assert_eq!(candidate.replace.start, prefix_start);
                assert_eq!(candidate.replace.end, before.len());
            }
        }
    }
}

#[test]
fn update_set_keyword_does_not_leak_into_other_contexts() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in [
        "update s",
        "update public.s",
        "update all_types_test as s",
        "update all_types_test set s",
        "update all_types_test set username = s",
        "update all_types_test set username = 'x' where s",
        "select * from all_types_test s",
        "update all_types_test set username = (select s",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            update_context(),
        );
        assert!(
            !has_builtin(&candidates, "SET", CompletionKind::Keyword),
            "{sql}"
        );
        if sql == "update s" {
            assert!(
                candidates
                    .iter()
                    .any(|candidate| candidate.label == "sys_user")
            );
        }
    }
}

#[test]
fn update_set_target_completion_lists_only_target_columns() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in [
        "update sys_user set ",
        "select * from user_agreement_accept; update sys_user set ",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            update_context(),
        );
        let mut labels = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        assert_eq!(
            labels,
            vec![
                "update_time",
                "update_user",
                "update_user_phone",
                "user_type",
                "username",
            ],
            "{sql}"
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind == CompletionKind::Column),
            "{sql}"
        );
    }
}

#[test]
fn update_set_target_completion_matches_prefix_and_replaces_only_it() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "update sys_user set update_";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    let mut labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    labels.sort_unstable();
    assert_eq!(
        labels,
        vec!["update_time", "update_user", "update_user_phone"]
    );
    for candidate in &candidates {
        assert_eq!(candidate.kind, CompletionKind::Column);
        assert_eq!(candidate.replace.start, sql.len() - "update_".len());
        assert_eq!(candidate.replace.end, sql.len());
        assert_eq!(candidate.insert_text, candidate.label);
        assert_eq!(candidate.detail.as_deref(), Some("text"));
    }
}

#[test]
fn update_set_second_assignment_returns_to_target_columns() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    for sql in [
        "update users set user_name = 'x', ",
        "update users set user_name = 'x', id = 1, ",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            update_context(),
        );
        let mut labels = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        assert_eq!(labels, vec!["id", "user_name"], "{sql}");
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind == CompletionKind::Column),
            "{sql}"
        );
    }
}

#[test]
fn update_set_value_offers_columns_and_expression_keywords() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "update users set user_name = ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    for keyword in ["CASE", "NULL", "TRUE", "FALSE", "DEFAULT"] {
        assert!(
            candidates.iter().any(|candidate| {
                candidate.kind == CompletionKind::Keyword && candidate.label == keyword
            }),
            "missing {keyword}: {:?}",
            candidates
                .iter()
                .map(|candidate| candidate.label.as_str())
                .collect::<Vec<_>>()
        );
    }
    for column in ["id", "user_name"] {
        assert!(candidates.iter().any(|candidate| {
            candidate.kind == CompletionKind::Column && candidate.label == column
        }));
    }
    assert!(candidates.iter().all(|candidate| matches!(
        candidate.kind,
        CompletionKind::Column
            | CompletionKind::Function
            | CompletionKind::Keyword
            | CompletionKind::BuiltinExpression
    )));
}

#[test]
fn update_set_target_excludes_source_relation_columns() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "update users set u from roles r";
    let cursor = sql.len() - " from roles r".len();
    let candidates = complete(sql, cursor, SqlDialect::Postgres, &index, update_context());
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"user_name"), "{labels:?}");
    assert!(!labels.contains(&"role_name"), "{labels:?}");
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Column)
    );
}

#[test]
fn update_set_value_qualifier_uses_source_relation_columns() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "update users set user_name = r. from roles r";
    let cursor = sql.len() - " from roles r".len();
    let candidates = complete(sql, cursor, SqlDialect::Postgres, &index, update_context());
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"role_name"), "{labels:?}");
    assert!(!labels.contains(&"user_name"), "{labels:?}");
}

#[test]
fn update_set_missing_target_offers_nothing_global() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in ["update missing_table set ", "update missing_table set s"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            update_context(),
        );
        assert!(candidates.is_empty(), "{sql}: {candidates:?}");
    }
}

#[test]
fn update_set_pg_as_alias_is_not_a_clause_boundary() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "update sys_user as output set update_";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    let mut labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    labels.sort_unstable();
    assert_eq!(
        labels,
        vec!["update_time", "update_user", "update_user_phone"]
    );
}

#[test]
fn update_set_array_brackets_keep_value_until_closed() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let inside = complete(
        "update users set user_name = ARRAY['a', ",
        "update users set user_name = ARRAY['a', ".len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    assert!(
        inside
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Keyword && candidate.label == "CASE"),
        "comma inside array brackets must keep value context"
    );
    let after = complete(
        "update users set user_name = ARRAY['a', 'b'], ",
        "update users set user_name = ARRAY['a', 'b'], ".len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    let mut labels = after
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    labels.sort_unstable();
    assert_eq!(labels, vec!["id", "user_name"]);
}

#[test]
fn update_set_nested_subquery_uses_its_own_scope() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "update users set user_name = (SELECT r from roles)";
    let cursor = sql.len() - " from roles)".len();
    let candidates = complete(sql, cursor, SqlDialect::Postgres, &index, update_context());
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Column
                && candidate.label == "role_name"),
        "{candidates:?}"
    );
}

#[test]
fn update_set_string_and_comments_do_not_change_stage() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    for sql in [
        "update users set user_name = 'a,b=1', \nuser_",
        "update users -- comment with , and =\nset user_name = 'x',\nuser_",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            update_context(),
        );
        let labels = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"user_name"), "{sql}: {labels:?}");
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind == CompletionKind::Column)
        );
    }
}

#[test]
fn update_set_tuple_assignment_is_bounded_to_target_columns() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let sql = "update users set (id, user_name) = (1, ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    assert!(!labels.contains(&"role_name"), "{labels:?}");
    assert!(candidates.iter().all(|candidate| matches!(
        candidate.kind,
        CompletionKind::Column | CompletionKind::Keyword | CompletionKind::BuiltinExpression
    )));
}

#[test]
fn standalone_set_and_ddl_set_start_no_assignment_context() {
    let index = CompletionIndex::new(&contextual_fixture());
    let standalone = complete(
        "set search_path = p",
        "set search_path = p".len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    assert!(
        !standalone
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Column),
        "{standalone:?}"
    );
    let ddl = complete(
        "alter table sys_user alter column user_name set default u",
        "alter table sys_user alter column user_name set default u".len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    assert!(
        !ddl.iter()
            .any(|candidate| candidate.kind == CompletionKind::Keyword),
        "assignment expression keywords must not leak into DDL: {ddl:?}"
    );
}

#[test]
fn update_set_sqlserver_alias_target_resolves_through_from_binding() {
    let index = CompletionIndex::new(&sqlserver_update_fixture());
    let sql = "update u set  from public.users as u";
    let cursor = sql.len() - " from public.users as u".len();
    let candidates = complete(sql, cursor, SqlDialect::SqlServer, &index, update_context());
    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"user_name"), "{labels:?}");
    assert!(!labels.contains(&"dumb"), "{labels:?}");
    let without_from = complete(
        "update u set ",
        "update u set ".len(),
        SqlDialect::SqlServer,
        &index,
        update_context(),
    );
    let mut labels = without_from
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    labels.sort_unstable();
    assert_eq!(labels, vec!["dumb"]);
}

#[test]
fn update_set_pg_qualified_column_name_is_not_offered() {
    let index = CompletionIndex::new(&multi_relation_fixture());
    let candidates = complete(
        "update users set users.u",
        "update users set users.u".len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    assert!(candidates.is_empty(), "{candidates:?}");
}

#[test]
fn update_set_dependencies_resolve_only_the_target() {
    let mut entries = multi_relation_fixture();
    entries.retain(|entry| entry.kind != CatalogKind::Column);
    let index = CompletionIndex::new(&entries);
    let users = index
        .entries()
        .iter()
        .find(|entry| entry.kind == CatalogKind::Table)
        .unwrap()
        .id
        .clone();
    for sql in [
        "update users set user_",
        "update users set user_name = ",
        "update users set user_name = 'x', ",
    ] {
        let dependencies = completion_dependencies(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            update_context(),
        );
        assert_eq!(dependencies.relation_children, vec![users.clone()], "{sql}");
    }
}

#[test]
fn update_set_cursor_mid_identifier_uses_token_boundary() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "update sys_user set update_time";
    let candidates = complete(
        sql,
        sql.len() - "time".len(),
        SqlDialect::Postgres,
        &index,
        update_context(),
    );
    let mut labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    labels.sort_unstable();
    assert_eq!(
        labels,
        vec!["update_time", "update_user", "update_user_phone"]
    );
    for candidate in &candidates {
        assert_eq!(
            candidate.replace.start,
            sql.len() - "time".len() - "update_".len()
        );
        assert_eq!(candidate.replace.end, sql.len() - "time".len());
    }
}

fn qualified(database: &str, schema: Option<&str>, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.into()),
        schema: schema.map(str::to_owned),
        object: object.into(),
    }
}

#[test]
fn snowflake_relation_completion_quotes_all_three_name_parts() {
    let index = CompletionIndex::new(&fixture());
    let text = "select * from ";
    let candidates = complete(
        text,
        text.len(),
        SqlDialect::Snowflake,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.insert_text.starts_with("\"app\".\"public\".")),
        "{candidates:?}"
    );
    let text = "select * from app.public.";
    let candidates = complete(
        text,
        text.len(),
        SqlDialect::Snowflake,
        &index,
        CompletionContext::default(),
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == CompletionKind::Table
                && candidate.insert_text.starts_with('"')),
        "{candidates:?}"
    );
}

#[test]
fn snowflake_completion_qualifies_case_sensitive_database_and_schema_names() {
    let index = CompletionIndex::new(&fixture());
    let text = "select * from ";
    for (database, schema, expected) in [
        ("APP", "public", "\"app\".\"public\".\"users\""),
        ("app", "PUBLIC", "\"public\".\"users\""),
        ("app", "public", "\"users\""),
    ] {
        let candidates = complete(
            text,
            text.len(),
            SqlDialect::Snowflake,
            &index,
            CompletionContext {
                database: Some(database),
                schema: Some(schema),
            },
        );
        let users = candidates
            .iter()
            .find(|candidate| candidate.kind == CompletionKind::Table && candidate.label == "users")
            .unwrap();
        assert_eq!(users.insert_text, expected, "{database}.{schema}");
    }
}
