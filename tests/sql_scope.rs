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
