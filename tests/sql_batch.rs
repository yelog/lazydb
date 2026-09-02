use lazydb::sql::split_sql_server_batches;

#[test]
fn splits_standalone_go_lines_with_crlf_comments_and_count_one() {
    let sql = "SELECT 1\r\nGO -- next\r\n/* before */ GO 1 /* after */\r\nSELECT 2";

    assert_eq!(
        split_sql_server_batches(sql).unwrap(),
        vec!["SELECT 1", "SELECT 2"]
    );
}

#[test]
fn ignores_go_in_strings_identifiers_comments_and_non_separator_lines() {
    let sql = "SELECT 'GO', 'it''s GO', \"GO\", [GO]], name]\n\
               /* outer\n/* GO */\nGO\n*/\n\
               SELECT 2 -- GO\n\
               SELECT GO FROM t";

    assert_eq!(split_sql_server_batches(sql).unwrap(), vec![sql.trim()]);
}

#[test]
fn rejects_repeat_counts_other_than_one_with_an_explicit_error() {
    let error = split_sql_server_batches("SELECT 1\nGO 3\nDELETE FROM users").unwrap_err();

    assert_eq!(error.count(), "3");
    assert_eq!(
        error.to_string(),
        "SQL Server GO repeat count 3 is unsupported; only GO and GO 1 are allowed"
    );
}
