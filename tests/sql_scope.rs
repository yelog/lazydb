use lazydb::sql::{
    ResolvedScope, ScopeKind, ScopeSelection, ScopeSource, SqlDialect, TextRange, resolve_scope,
    scan_statements,
};

fn selection(text: &str, start: &str, end: &str, kind: ScopeKind) -> ScopeSelection {
    let start = text.find(start).unwrap();
    let end = start + text[start..].find(end).unwrap() + end.len();
    ScopeSelection::contiguous(kind, TextRange::new(start, end))
}

fn resolved(kind: ScopeKind, source: ScopeSource, sql: &str) -> ResolvedScope {
    ResolvedScope {
        kind,
        source,
        sql: sql.to_owned(),
    }
}

#[test]
fn visual_selection_wins_and_blank_selection_does_not_fall_back() {
    let text = "select 1; select 2;";
    let visual = selection(text, "select 2", ";", ScopeKind::VisualChar);
    assert_eq!(
        resolve_scope(text, 2, Some(&visual), SqlDialect::Generic),
        Some(resolved(
            ScopeKind::VisualChar,
            ScopeSource::Contiguous(TextRange::new(10, 19)),
            "select 2;",
        ))
    );

    let blank = ScopeSelection::contiguous(ScopeKind::VisualChar, TextRange::new(9, 10));
    assert_eq!(
        resolve_scope(text, 2, Some(&blank), SqlDialect::Generic),
        None
    );
    let comment =
        ScopeSelection::contiguous(ScopeKind::VisualChar, TextRange::new(0, "-- comment".len()));
    assert_eq!(
        resolve_scope("-- comment", 0, Some(&comment), SqlDialect::Generic),
        None
    );
}

#[test]
fn ranges_are_utf8_bytes_and_visual_line_is_contiguous() {
    let text = "select 'é';\nselect 🙂;\n";
    let start = text.find("select 🙂").unwrap();
    let end = start + "select 🙂;".len();
    let visual = ScopeSelection::contiguous(ScopeKind::VisualLine, TextRange::new(start, end));
    assert_eq!(
        resolve_scope(text, 0, Some(&visual), SqlDialect::Generic),
        Some(resolved(
            ScopeKind::VisualLine,
            ScopeSource::Contiguous(TextRange::new(start, end)),
            "select 🙂;",
        ))
    );
    assert_eq!(text.as_bytes()[start], b's');
    assert_eq!(end - start, "select 🙂;".len());
}

#[test]
fn visual_block_preserves_order_and_never_executes_bounding_rectangle() {
    let text = "abCD\n12 34\nxyZW";
    let ranges = vec![
        TextRange::new(2, 4),
        TextRange::new(5, 7),
        TextRange::new(13, 15),
    ];
    let visual = ScopeSelection::block(ranges.clone());
    assert_eq!(
        resolve_scope(text, 0, Some(&visual), SqlDialect::Generic),
        Some(resolved(
            ScopeKind::VisualBlock,
            ScopeSource::Block(ranges),
            "CD\n12\nZW",
        ))
    );
}

#[test]
fn cursor_on_semicolon_selects_statement_but_gap_does_not() {
    let text = "select 1;\n\n -- gap\n\n select 2;";
    assert_eq!(
        resolve_scope(text, 8, None, SqlDialect::Generic).map(|scope| scope.sql),
        Some("select 1;".to_owned())
    );
    let gap = text.find("-- gap").unwrap();
    assert_eq!(resolve_scope(text, gap, None, SqlDialect::Generic), None);
}

#[test]
fn oracle_scope_ignores_semicolons_inside_alternative_quoted_literals() {
    let text = "SELECT q'[a;b]' AS value FROM dual; SELECT 2 FROM dual;";
    let first = text.find("q'").unwrap();
    assert_eq!(
        resolve_scope(text, first, None, SqlDialect::Oracle).map(|scope| scope.sql),
        Some("SELECT q'[a;b]' AS value FROM dual;".to_owned())
    );
}

#[test]
fn oracle_scope_keeps_anonymous_block_together() {
    let text = "BEGIN\n  IF 1 = 1 THEN NULL; END IF;\nEND;\n/\nSELECT 2 FROM dual;";
    let cursor = text.find("NULL").unwrap();
    assert_eq!(
        scan_statements(text, SqlDialect::Oracle)[0].get(text),
        Some("BEGIN\n  IF 1 = 1 THEN NULL; END IF;\nEND;\n/")
    );
    assert_eq!(
        resolve_scope(text, cursor, None, SqlDialect::Oracle).map(|scope| scope.sql),
        Some("BEGIN\n  IF 1 = 1 THEN NULL; END IF;\nEND;\n/".to_owned())
    );
}

#[test]
fn whitespace_inside_statement_resolves_to_current_scope() {
    let text = "Select * from sys_user";

    for cursor in text
        .char_indices()
        .filter_map(|(index, character)| character.is_whitespace().then_some(index))
    {
        assert_eq!(
            resolve_scope(text, cursor, None, SqlDialect::Generic).map(|scope| scope.sql),
            Some(text.to_owned()),
            "cursor at byte {cursor} should resolve the statement",
        );
    }
}

#[test]
fn multiline_whitespace_and_internal_comments_resolve_to_current_scope() {
    let text = "select\n  /* explain projection */\n  *\nfrom sys_user;";

    for marker in ["\n", "  /*", "explain", "\n  *", "\nfrom"] {
        let cursor = text.find(marker).unwrap();
        assert_eq!(
            resolve_scope(text, cursor, None, SqlDialect::Generic).map(|scope| scope.sql),
            Some(text.to_owned()),
            "cursor at {marker:?} should resolve the statement",
        );
    }
}

#[test]
fn adjacent_statement_start_belongs_to_the_second_statement() {
    let text = "select 1;select 2;";
    let cursor = text.find("select 2").unwrap();

    assert_eq!(
        resolve_scope(text, cursor, None, SqlDialect::Generic).map(|scope| scope.sql),
        Some("select 2;".to_owned())
    );
}

#[test]
fn scanner_ignores_semicolons_inside_all_supported_constructs() {
    let text = "select ';', \";\", `;`, [;], /* outer ; /* inner ; */ ; */ 1;\n".to_owned()
        + "-- ;\nselect 2; # ;\nselect $$ ; $$, $tag$ ; $tag$;";
    let ranges = scan_statements(&text, SqlDialect::Generic);
    let sql: Vec<_> = ranges
        .iter()
        .map(|range| &text[range.start..range.end])
        .collect();
    assert_eq!(sql.len(), 3);
    assert!(sql[0].ends_with("1;"));
    assert!(sql[1].contains("select 2"));
    assert!(sql[2].contains("$tag$ ; $tag$;"));
}

#[test]
fn dialect_specific_mysql_comments_and_unterminated_constructs_are_conservative() {
    let mysql = "select 1--not a comment; select 2-- comment\n; select 3# ;\n;";
    let ranges = scan_statements(mysql, SqlDialect::MySql);
    assert_eq!(ranges.len(), 3);
    assert!(mysql[ranges[0].start..ranges[0].end].contains("--not a comment"));

    let unterminated = "select 'not; done";
    assert_eq!(scan_statements(unterminated, SqlDialect::Generic).len(), 1);
    assert_eq!(
        resolve_scope(unterminated, 4, None, SqlDialect::Generic)
            .unwrap()
            .sql,
        unterminated
    );
}

#[test]
fn mysql_scanner_preserves_escaped_double_quotes_and_embedded_semicolons() {
    let text = r#"SELECT "a\";b" AS value; SELECT 2;"#;
    let ranges = scan_statements(text, SqlDialect::MySql);
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0].get(text), Some(r#"SELECT "a\";b" AS value;"#));
    assert_eq!(ranges[1].get(text), Some("SELECT 2;"));

    let identifier = r#"SELECT "ends_with_backslash\"; SELECT 2;"#;
    assert_eq!(scan_statements(identifier, SqlDialect::Postgres).len(), 2);
}

#[test]
fn mariadb_scanner_uses_mysql_quote_and_comment_rules() {
    for (text, expected) in [
        (
            r#"SELECT "a\";b" AS value; SELECT 2;"#,
            vec![r#"SELECT "a\";b" AS value;"#, "SELECT 2;"],
        ),
        (
            "SELECT `odd;name``;suffix`; SELECT 2;",
            vec!["SELECT `odd;name``;suffix`;", "SELECT 2;"],
        ),
        (
            "SELECT 1 # hidden ;\n; SELECT 2;",
            vec!["SELECT 1 # hidden ;\n;", "SELECT 2;"],
        ),
        (
            "# comment-only ;\nSELECT 1;",
            vec!["# comment-only ;\nSELECT 1;"],
        ),
        (
            "SELECT 1--not a comment; SELECT 2-- comment ;\n;",
            vec!["SELECT 1--not a comment;", "SELECT 2-- comment ;\n;"],
        ),
    ] {
        let actual: Vec<_> = scan_statements(text, SqlDialect::MariaDb)
            .into_iter()
            .map(|range| range.get(text).unwrap())
            .collect();
        assert_eq!(actual, expected, "SQL: {text}");
    }
    assert!(scan_statements("# comment only", SqlDialect::MariaDb).is_empty());
}

#[test]
fn mysql_family_executable_comments_are_preserved_as_statements_and_scopes() {
    for dialect in [SqlDialect::MySql, SqlDialect::MariaDb] {
        for sql in ["/*!40101 SET NAMES utf8mb4 */", "/*! SET @value = 'a;b' */"] {
            let text = format!("-- ordinary comment\n{sql}; SELECT 2;");
            let ranges = scan_statements(&text, dialect);
            assert_eq!(ranges.len(), 2, "dialect: {dialect:?}, SQL: {text}");
            assert_eq!(
                ranges[0].get(&text),
                Some(format!("-- ordinary comment\n{sql};").as_str())
            );
            assert_eq!(ranges[1].get(&text), Some("SELECT 2;"));
            assert!(resolve_scope(&text, text.find("/*!").unwrap(), None, dialect).is_some());
            assert_eq!(
                scan_statements(sql, dialect),
                vec![TextRange::new(0, sql.len())]
            );
            let visual =
                ScopeSelection::contiguous(ScopeKind::VisualChar, TextRange::new(0, sql.len()));
            assert!(resolve_scope(sql, 0, Some(&visual), dialect).is_some());
        }
    }

    let maria_sql = "/*M!100100 SET @value = 'a;b' */";
    assert_eq!(
        scan_statements(maria_sql, SqlDialect::MariaDb),
        vec![TextRange::new(0, maria_sql.len())]
    );
    for dialect in [SqlDialect::MySql, SqlDialect::Postgres, SqlDialect::Generic] {
        assert!(scan_statements(maria_sql, dialect).is_empty());
    }
    for dialect in [SqlDialect::MySql, SqlDialect::MariaDb] {
        assert!(scan_statements("/* ordinary comment */", dialect).is_empty());
        assert!(scan_statements("/*+ optimizer hint */", dialect).is_empty());
        assert!(scan_statements("/* outer /*! nested */ */", dialect).is_empty());
    }
    assert!(scan_statements("/*!40101 SET NAMES utf8mb4 */", SqlDialect::Postgres).is_empty());
}

#[test]
fn mysql_family_cursor_scope_refuses_mode_dependent_statement_ranges() {
    for dialect in [SqlDialect::MySql, SqlDialect::MariaDb] {
        for text in [
            r#"SELECT "路径\"; SELECT 2;"#,
            r"SELECT '路径\'; SELECT 2;",
            r#"SELECT "a\";b" AS value; SELECT 2;"#,
        ] {
            for cursor in [0, text.find("SELECT 2").unwrap()] {
                assert_eq!(resolve_scope(text, cursor, None, dialect), None);
            }
            let first_end = text.find(';').unwrap() + 1;
            let visual =
                ScopeSelection::contiguous(ScopeKind::VisualChar, TextRange::new(0, first_end));
            assert_eq!(
                resolve_scope(text, 0, Some(&visual), dialect).unwrap().sql,
                &text[..first_end]
            );
        }
        let text = r#"SELECT 'a'';b', "c"";d", `e\`; SELECT 2;"#;
        assert_eq!(
            resolve_scope(text, 0, None, dialect).unwrap().sql,
            &text[..text.find(" SELECT 2").unwrap()]
        );
        assert_eq!(
            resolve_scope(text, text.find("SELECT 2").unwrap(), None, dialect)
                .unwrap()
                .sql,
            "SELECT 2;"
        );
    }
}

#[test]
fn duckdb_scanner_handles_dollar_quotes_lists_and_nested_comments() {
    let text = "SELECT $$left;right$$, $tag$semi;colon$tag$, [1, 2]; /* outer ; /* inner ; */ end */ SELECT 3;";
    let ranges = scan_statements(text, SqlDialect::DuckDb);
    assert_eq!(ranges.len(), 2);
    assert_eq!(
        &text[ranges[0].start..ranges[0].end],
        "SELECT $$left;right$$, $tag$semi;colon$tag$, [1, 2];"
    );
    assert!(text[ranges[1].start..ranges[1].end].contains("SELECT 3"));
}

#[test]
fn sql_server_scope_ignores_semicolons_in_bracketed_identifiers() {
    let text = "SELECT [odd;name], N'value;still' FROM [dbo].[items]; SELECT @value;";
    let ranges = scan_statements(text, SqlDialect::SqlServer);
    assert_eq!(ranges.len(), 2);
    assert_eq!(
        &text[ranges[0].start..ranges[0].end],
        "SELECT [odd;name], N'value;still' FROM [dbo].[items];"
    );
    assert_eq!(&text[ranges[1].start..ranges[1].end], "SELECT @value;");
}

#[test]
fn snowflake_scope_keeps_identifier_quotes_independent_of_mysql_session_modes() {
    let text = r#"SELECT "ends_with_backslash\"; SELECT 2;"#;
    let ranges = scan_statements(text, SqlDialect::Snowflake);
    assert_eq!(ranges.len(), 2);
    assert_eq!(
        resolve_scope(text, 0, None, SqlDialect::Snowflake)
            .unwrap()
            .sql,
        r#"SELECT "ends_with_backslash\";"#
    );
    assert_eq!(
        resolve_scope(
            text,
            text.find("SELECT 2").unwrap(),
            None,
            SqlDialect::Snowflake,
        )
        .unwrap()
        .sql,
        "SELECT 2;"
    );
}

#[test]
fn snowflake_scope_keeps_dollar_quoted_body_and_comments_in_one_statement() {
    let text = "CREATE PROCEDURE p() RETURNS VARCHAR LANGUAGE SQL AS $$ BEGIN RETURN 'a;b'; END; $$; SELECT 2;";
    let statements = scan_statements(text, SqlDialect::Snowflake);
    assert_eq!(statements.len(), 2);
    let scope = resolve_scope(
        text,
        text.find("RETURN").unwrap(),
        None,
        SqlDialect::Snowflake,
    )
    .unwrap();
    assert!(scope.sql.ends_with("$$;"));
    let text = "SELECT $$a; -- still a string\n b$$; -- comment\nSELECT 2;";
    assert_eq!(scan_statements(text, SqlDialect::Snowflake).len(), 2);
}
