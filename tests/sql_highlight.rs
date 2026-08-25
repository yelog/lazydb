use lazydb::sql::{HighlightKind, LineIndex, SqlDialect, TextRange, highlight_sql};

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
