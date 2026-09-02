use lazydb::sql::{
    HighlightKind, LineIndex, SqlDialect, TextRange, highlight_sql, highlight_sql_ranges,
};

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
            .any(|span| span.kind == HighlightKind::Identifier)
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
    let text = "[2026-08-31] database> SELECT 'Ada'\n[2026-08-31] 1 row retrieved";
    let start = text.find("SELECT").unwrap();
    let end = text.find('\n').unwrap();
    let spans = highlight_sql_ranges(text, &[TextRange::new(start, end)], SqlDialect::Postgres);

    assert!(spans.iter().all(|span| span.range.start >= start));
    assert!(spans.iter().all(|span| span.range.end <= end));
    assert!(spans.iter().any(|span| {
        &text[span.range.start..span.range.end] == "SELECT" && span.kind == HighlightKind::Keyword
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
    assert_eq!(kind("[odd]]name]"), Some(HighlightKind::Identifier));
    assert_eq!(kind("@value"), Some(HighlightKind::Parameter));
    assert_eq!(kind("N'你好'"), Some(HighlightKind::String));
    assert!(spans.iter().any(|span| span.kind == HighlightKind::Comment));
}
