use lazydb::sql::{
    HighlightKind, HighlightSpan, LineIndex, SqlDialect, TextRange, highlight_sql,
    highlight_sql_ranges,
};

fn kinds_for(text: &str, spans: &[HighlightSpan], needle: &str) -> Vec<HighlightKind> {
    spans
        .iter()
        .filter(|span| &text[span.range.start..span.range.end] == needle)
        .map(|span| span.kind)
        .collect()
}

#[test]
fn locations_are_utf8_byte_ranges_and_half_open() {
    let text = "é SELECT";
    let index = LineIndex::new(text);
    let range = index.range(
        text,
        sqlparser::tokenizer::Location::new(1, 3),
        sqlparser::tokenizer::Location::new(1, 9),
    );
    assert_eq!(range, TextRange::new(3, text.len()));
}

#[test]
fn unicode_keywords_and_strings_are_highlighted() {
    let spans = highlight_sql("SELECT 数据, '🙂' -- note", SqlDialect::Postgres);
    assert!(spans.iter().any(|span| span.kind == HighlightKind::Keyword));
    assert!(
        spans
            .iter()
            .any(|span| { matches!(span.kind, HighlightKind::Identifier | HighlightKind::Column) })
    );
    assert!(spans.iter().any(|span| span.kind == HighlightKind::String));
    assert!(spans.iter().any(|span| span.kind == HighlightKind::Comment));
}

#[test]
fn incomplete_input_keeps_preceding_tokens_without_errors() {
    let spans = highlight_sql("SELECT 'unterminated", SqlDialect::Postgres);
    assert!(spans.iter().any(|span| span.kind == HighlightKind::Keyword));
}

#[test]
fn selected_ranges_highlight_sql_without_tokenizing_log_prefixes() {
    let text = "[2026-08-31] database> SELECT u.id FROM users u\n[2026-08-31] 1 row retrieved";
    let start = text.find("SELECT").unwrap();
    let end = text.find('\n').unwrap();
    let spans = highlight_sql_ranges(text, &[TextRange::new(start, end)], SqlDialect::Postgres);

    assert!(spans.iter().all(|span| span.range.start >= start));
    assert!(spans.iter().all(|span| span.range.end <= end));
    assert!(spans.iter().any(|span| {
        &text[span.range.start..span.range.end] == "SELECT" && span.kind == HighlightKind::Keyword
    }));
    assert!(spans.iter().any(|span| {
        &text[span.range.start..span.range.end] == "users" && span.kind == HighlightKind::Relation
    }));
    assert!(spans.iter().any(|span| {
        &text[span.range.start..span.range.end] == "u" && span.kind == HighlightKind::RelationAlias
    }));
    assert!(spans.iter().any(|span| {
        &text[span.range.start..span.range.end] == "id" && span.kind == HighlightKind::Column
    }));
    assert!(!spans.iter().any(|span| {
        &text[span.range.start..span.range.end] == "2026" && span.kind == HighlightKind::Number
    }));
}

#[test]
fn sql_server_highlights_tsql_identifiers_unicode_variables_and_comments() {
    let text = "SELECT TOP (1) [odd]]name], @value, N'你好' -- note";
    let spans = highlight_sql(text, SqlDialect::SqlServer);
    let kind = |needle: &str| {
        spans
            .iter()
            .find(|span| &text[span.range.start..span.range.end] == needle)
            .map(|span| span.kind)
    };

    assert_eq!(kind("TOP"), Some(HighlightKind::Keyword));
    assert_eq!(kind("[odd]]name]"), Some(HighlightKind::Column));
    assert_eq!(kind("@value"), Some(HighlightKind::Parameter));
    assert_eq!(kind("N'你好'"), Some(HighlightKind::String));
    assert!(spans.iter().any(|span| span.kind == HighlightKind::Comment));
}

#[test]
fn select_distinguishes_relations_aliases_and_columns() {
    let text = r#"SELECT A.USERNAME, A."name"
FROM SYS_USER A
LEFT JOIN sys_user_role B ON A.id = B.USER_ID
LEFT JOIN sys_role C ON B.ROLE_ID = C.id
WHERE C.NAME = 'MES团队'"#;
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "SYS_USER"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "sys_user_role"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "sys_role"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(text, &spans, "A")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(
        kinds_for(text, &spans, "USERNAME"),
        vec![HighlightKind::Column]
    );
    assert_eq!(
        kinds_for(text, &spans, "\"name\""),
        vec![HighlightKind::Column]
    );
    assert_eq!(
        kinds_for(text, &spans, "USER_ID"),
        vec![HighlightKind::Column]
    );
    assert_eq!(
        kinds_for(text, &spans, "ROLE_ID"),
        vec![HighlightKind::Column]
    );
    assert_eq!(kinds_for(text, &spans, "NAME"), vec![HighlightKind::Column]);
}

#[test]
fn qualified_relations_columns_and_functions_have_distinct_roles() {
    let text = "SELECT COUNT(u.id) AS total FROM app.public.users AS u ORDER BY total";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "COUNT"),
        vec![HighlightKind::Function]
    );
    assert_eq!(
        kinds_for(text, &spans, "app"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "public"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "users"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(text, &spans, "u")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(kinds_for(text, &spans, "id"), vec![HighlightKind::Column]);
    assert_eq!(
        kinds_for(text, &spans, "total"),
        vec![HighlightKind::Column, HighlightKind::Column]
    );
}

#[test]
fn query_level_ordering_can_resolve_relation_aliases() {
    let text = "SELECT u.id FROM users u ORDER BY u.id";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert!(
        kinds_for(text, &spans, "u")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(
        kinds_for(text, &spans, "id"),
        vec![HighlightKind::Column, HighlightKind::Column]
    );
}

#[test]
fn ctes_and_set_queries_keep_relation_scopes_local() {
    let cte = "WITH recent AS (SELECT id FROM events) SELECT r.id FROM recent r ORDER BY r.id";
    let cte_spans = highlight_sql(cte, SqlDialect::Postgres);
    assert_eq!(
        kinds_for(cte, &cte_spans, "recent"),
        vec![HighlightKind::Relation, HighlightKind::Relation]
    );
    assert!(
        kinds_for(cte, &cte_spans, "r")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );

    let union = "SELECT a.id FROM alpha a UNION SELECT b.id FROM beta b ORDER BY b.id";
    let union_spans = highlight_sql(union, SqlDialect::Postgres);
    assert_eq!(
        kinds_for(union, &union_spans, "alpha"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(union, &union_spans, "beta"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(union, &union_spans, "a")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert!(
        kinds_for(union, &union_spans, "b")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
}

#[test]
fn nested_queries_resolve_aliases_by_visible_scope() {
    let text = "SELECT u.id FROM users u WHERE EXISTS (SELECT 1 FROM roles r WHERE r.user_id = u.id); SELECT u.id";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    let u_kinds = kinds_for(text, &spans, "u");
    assert_eq!(u_kinds.len(), 4);
    assert!(
        u_kinds[..3]
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(u_kinds[3], HighlightKind::Identifier);
    assert!(
        kinds_for(text, &spans, "r")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
}

#[test]
fn dml_relations_aliases_and_target_columns_are_semantic() {
    let update = "UPDATE users u SET name = u.name WHERE u.id = 1";
    let update_spans = highlight_sql(update, SqlDialect::Postgres);
    assert_eq!(
        kinds_for(update, &update_spans, "users"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(update, &update_spans, "u")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert!(
        kinds_for(update, &update_spans, "name")
            .iter()
            .all(|kind| *kind == HighlightKind::Column)
    );

    let insert = "INSERT INTO users (id, name) VALUES (1, 'Ada')";
    let insert_spans = highlight_sql(insert, SqlDialect::Postgres);
    assert_eq!(
        kinds_for(insert, &insert_spans, "users"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(insert, &insert_spans, "id")
            .iter()
            .all(|kind| *kind == HighlightKind::Column)
    );
    assert!(
        kinds_for(insert, &insert_spans, "name")
            .iter()
            .all(|kind| *kind == HighlightKind::Column)
    );

    let delete = "DELETE FROM users u WHERE u.id = 1";
    let delete_spans = highlight_sql(delete, SqlDialect::Postgres);
    assert_eq!(
        kinds_for(delete, &delete_spans, "users"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(delete, &delete_spans, "u")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(
        kinds_for(delete, &delete_spans, "id"),
        vec![HighlightKind::Column]
    );
}

#[test]
fn quoted_identifiers_keep_semantic_roles_across_dialects() {
    let postgres = "SELECT \"u\".\"name\" FROM \"users\" AS \"u\"";
    let postgres_spans = highlight_sql(postgres, SqlDialect::Postgres);
    assert_eq!(
        kinds_for(postgres, &postgres_spans, "\"users\""),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(postgres, &postgres_spans, "\"u\"")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(
        kinds_for(postgres, &postgres_spans, "\"name\""),
        vec![HighlightKind::Column]
    );

    let mysql = "SELECT `u`.`name` FROM `users` AS `u`";
    let mysql_spans = highlight_sql(mysql, SqlDialect::MySql);
    assert_eq!(
        kinds_for(mysql, &mysql_spans, "`users`"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(mysql, &mysql_spans, "`u`")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(
        kinds_for(mysql, &mysql_spans, "`name`"),
        vec![HighlightKind::Column]
    );

    let sql_server = "SELECT [u].[name] FROM [users] AS [u]";
    let sql_server_spans = highlight_sql(sql_server, SqlDialect::SqlServer);
    assert_eq!(
        kinds_for(sql_server, &sql_server_spans, "[users]"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(sql_server, &sql_server_spans, "[u]")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(
        kinds_for(sql_server, &sql_server_spans, "[name]"),
        vec![HighlightKind::Column]
    );
}

#[test]
fn semantic_highlighting_is_isolated_per_statement() {
    let text = "SELECT a.id FROM alpha a; SELECT (1; SELECT b.id FROM beta b";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "alpha"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "beta"),
        vec![HighlightKind::Relation]
    );
    assert!(spans.iter().all(|span| span.range.end <= text.len()));
}

#[test]
fn incomplete_where_preserves_completed_semantic_highlights() {
    let text = r#"SELECT A.USERNAME, A."name", A.CREATE_TIME, A.EMAIL, A.USER_TYPE, A.PHONE
FROM SYS_USER A
LEFT JOIN sys_user_role B ON A.id = B.USER_ID
LEFT JOIN sys_role C ON B.ROLE_ID = C.id
WHERE"#;
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "SYS_USER"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "sys_user_role"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "sys_role"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(text, &spans, "A")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert!(
        kinds_for(text, &spans, "B")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert!(
        kinds_for(text, &spans, "C")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(
        kinds_for(text, &spans, "USERNAME"),
        vec![HighlightKind::Column]
    );
    assert_eq!(
        kinds_for(text, &spans, "WHERE"),
        vec![HighlightKind::Keyword]
    );
    assert!(spans.iter().all(|span| span.range.end <= text.len()));
}

#[test]
fn incomplete_expression_keeps_the_longest_semantic_prefix() {
    let text = "SELECT u.id FROM users u WHERE u.id =";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "users"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(text, &spans, "u")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert!(
        kinds_for(text, &spans, "id")
            .iter()
            .all(|kind| *kind == HighlightKind::Column)
    );
    assert!(spans.iter().all(|span| span.range.end <= text.len()));
}

#[test]
fn unmatched_trailing_parenthesis_preserves_earlier_semantics() {
    let text = "SELECT u.id FROM users u WHERE (";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "users"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(text, &spans, "u")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert_eq!(kinds_for(text, &spans, "id"), vec![HighlightKind::Column]);
    assert!(spans.iter().all(|span| span.range.end <= text.len()));
}

#[test]
fn incomplete_statement_recovery_is_isolated_per_statement() {
    let text = "SELECT a.id FROM alpha a; SELECT b.id FROM beta b WHERE";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "alpha"),
        vec![HighlightKind::Relation]
    );
    assert_eq!(
        kinds_for(text, &spans, "beta"),
        vec![HighlightKind::Relation]
    );
    assert!(
        kinds_for(text, &spans, "a")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert!(
        kinds_for(text, &spans, "b")
            .iter()
            .all(|kind| *kind == HighlightKind::RelationAlias)
    );
    assert!(spans.iter().all(|span| span.range.end <= text.len()));
}

#[test]
fn unrecoverable_sql_still_returns_safe_lexical_highlighting() {
    let text = "WHERE (";
    let spans = highlight_sql(text, SqlDialect::Postgres);

    assert_eq!(
        kinds_for(text, &spans, "WHERE"),
        vec![HighlightKind::Keyword]
    );
    assert!(spans.iter().all(|span| span.range.end <= text.len()));
}
