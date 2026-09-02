use lazydb::sql::{FormatError, SqlDialect, format_sql};

#[test]
fn formatting_uppercases_keywords_without_changing_meaning() {
    let output = format_sql("select id from users where name = $1", SqlDialect::Postgres).unwrap();
    assert!(output.contains("SELECT"));
    assert!(output.contains("$1"));
}

#[test]
fn formatting_rejects_dollar_quoted_procedures() {
    assert_eq!(
        format_sql(
            "CREATE FUNCTION f() RETURNS void AS $$ BEGIN END $$ LANGUAGE plpgsql",
            SqlDialect::Postgres
        ),
        Err(FormatError::ProceduralBody)
    );
}

#[test]
fn formatting_accepts_unicode_identifiers() {
    let output = format_sql("select 数据 from 表", SqlDialect::Postgres).unwrap();
    assert!(output.contains("数据"));
    assert!(output.contains("表"));
}

#[test]
fn formatting_preserves_tsql_tokens() {
    let output = format_sql(
        "select top (1) [display name], @value from [user] where [name] = N'你好'",
        SqlDialect::SqlServer,
    )
    .unwrap();
    assert!(output.to_ascii_uppercase().contains("TOP"));
    assert!(output.contains("[display name]"));
    assert!(output.contains("@value"));
    assert!(output.contains("N'你好'"));
}
