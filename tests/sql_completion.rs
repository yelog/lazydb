use lazydb::{
    db::catalog::{CatalogId, CatalogKind, CatalogNode},
    sql::{
        CompletionIndex, CompletionKind, SqlDialect, complete, quote_identifier,
        should_offer_completion,
    },
};
use uuid::Uuid;

fn fixture() -> Vec<CatalogNode> {
    let connection = Uuid::new_v4();
    let database = CatalogId::new(connection, CatalogKind::Database, ["app"]);
    let schema = CatalogId::new(connection, CatalogKind::Schema, ["app", "public"]);
    let table = CatalogId::new(connection, CatalogKind::Table, ["app", "public", "users"]);
    vec![
        CatalogNode::new(database, None, "app", "database", None, true),
        CatalogNode::new(schema.clone(), None, "public", "schema", None, true),
        CatalogNode::new(table.clone(), Some(schema), "users", "table", None, true),
        CatalogNode::new(
            CatalogId::new(
                connection,
                CatalogKind::Column,
                ["app", "public", "users", "odd name"],
            ),
            Some(table),
            "odd name",
            "column",
            Some("text\x1b[31m".into()),
            false,
        ),
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
    nodes.push(CatalogNode::new(
        CatalogId::new(Uuid::new_v4(), CatalogKind::Table, ["x", "\x1b[2J"]),
        None,
        "\x1b[2J",
        "table",
        Some("\x00detail".into()),
        false,
    ));
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
            CatalogNode::new(
                CatalogId::new(connection, CatalogKind::Table, ["app", "public", name]),
                None,
                name,
                "table",
                None,
                false,
            )
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
fn automatic_completion_stops_at_delimiters_and_closed_quotes() {
    for text in [
        "tools.\"sys_config\"",
        "tools.`sys_config`",
        "SELECT * FROM tools;",
        "tools.sys ",
        "tools.sys)",
    ] {
        assert!(!should_offer_completion(text, text.len()), "{text:?}");
    }
    assert!(should_offer_completion("tools.sys", "tools.sys".len()));
    assert!(should_offer_completion("tools.", "tools.".len()));
}

#[test]
fn accepted_quoted_identifier_does_not_reopen_automatic_completion() {
    let text = "SELECT * FROM tools.\"sys_config\"";
    assert!(!should_offer_completion(text, text.len()));
}
