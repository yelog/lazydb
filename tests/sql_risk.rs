use lazydb::sql::{SqlDialect, SqlRisk, SqlRiskAggregate, classify_sql};

fn risk(sql: &str) -> SqlRisk {
    classify_sql(sql, SqlDialect::Postgres).risks[0]
}

#[test]
fn classifies_queries_and_nested_query_features_conservatively() {
    assert_eq!(risk("SELECT 1"), SqlRisk::ReadOnly);
    assert_eq!(risk("VALUES (1)"), SqlRisk::ReadOnly);
    assert_eq!(
        risk("WITH rows AS (SELECT 1) SELECT * FROM rows"),
        SqlRisk::ReadOnly
    );
    assert_eq!(
        risk("WITH changed AS (UPDATE t SET x = 1) SELECT * FROM changed"),
        SqlRisk::Dml
    );
    assert_eq!(risk("SELECT * INTO new_t FROM old_t"), SqlRisk::Dml);
    assert_eq!(risk("SELECT * FROM t FOR UPDATE"), SqlRisk::Dml);
    assert_eq!(risk("EXPLAIN UPDATE t SET x = 1"), SqlRisk::Dml);
    assert_eq!(risk("EXPLAIN SELECT 1"), SqlRisk::ReadOnly);
}

#[test]
fn classifies_mutation_ddl_and_transaction_statements() {
    for sql in [
        "INSERT INTO t VALUES (1)",
        "UPDATE t SET x = 1",
        "DELETE FROM t",
        "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN UPDATE SET x = 1",
    ] {
        assert_eq!(risk(sql), SqlRisk::Dml, "{sql}");
    }
    for sql in [
        "CREATE TABLE t (id INT)",
        "ALTER TABLE t ADD COLUMN x INT",
        "DROP TABLE t",
        "TRUNCATE TABLE t",
    ] {
        assert_eq!(risk(sql), SqlRisk::Ddl, "{sql}");
    }
    for sql in ["BEGIN", "COMMIT", "ROLLBACK", "SAVEPOINT s"] {
        assert_eq!(risk(sql), SqlRisk::TransactionControl, "{sql}");
    }
}

#[test]
fn unknowns_include_calls_dynamic_sql_and_parse_failures() {
    assert_eq!(risk("CALL do_work()"), SqlRisk::Unknown);
    assert_eq!(risk("EXECUTE 'SELECT 1'"), SqlRisk::Unknown);
    assert_eq!(risk("SELECT (SELECT 1"), SqlRisk::Unknown);
    assert_eq!(risk("SHOW TABLES"), SqlRisk::Unknown);
}

#[test]
fn preserves_each_statement_and_explicitly_marks_multi_statement() {
    let analysis = classify_sql("SELECT 1; UPDATE t SET x = 1;", SqlDialect::Postgres);
    assert_eq!(analysis.statement_count, 2);
    assert_eq!(analysis.risks, vec![SqlRisk::ReadOnly, SqlRisk::Dml]);
    assert_eq!(analysis.aggregate, SqlRiskAggregate::MultiStatement);
}

#[test]
fn read_only_is_only_a_risk_signal() {
    assert_eq!(risk("SELECT side_effecting_function()"), SqlRisk::ReadOnly);
}

#[test]
fn sql_server_classifies_every_go_batch() {
    let analysis = classify_sql(
        "SELECT TOP (1) [id] FROM [dbo].[items]\r\nGO\r\nUPDATE [dbo].[items] SET [id] = 2 OUTPUT inserted.[id]\r\nGO 1\r\nCREATE TABLE [dbo].[audit] ([id] int)",
        SqlDialect::SqlServer,
    );
    assert_eq!(analysis.statement_count, 3);
    assert_eq!(
        analysis.risks,
        vec![SqlRisk::ReadOnly, SqlRisk::Dml, SqlRisk::Ddl]
    );
    assert_eq!(analysis.aggregate, SqlRiskAggregate::MultiStatement);

    let rejected = classify_sql("SELECT 1\nGO 2\nDELETE FROM users", SqlDialect::SqlServer);
    assert_eq!(rejected.risks, vec![SqlRisk::Unknown]);

    let malformed = classify_sql(
        "not valid sql\nGO\nDELETE FROM [dbo].[items]",
        SqlDialect::SqlServer,
    );
    assert_eq!(malformed.risks, vec![SqlRisk::Unknown, SqlRisk::Dml]);
}

#[test]
fn oracle_statement_terminators_do_not_change_read_only_risk() {
    assert_eq!(
        classify_sql("SELECT 1 FROM dual;", SqlDialect::Oracle),
        classify_sql("SELECT 1 FROM dual", SqlDialect::Oracle)
    );
    assert_eq!(
        classify_sql("BEGIN NULL; END;", SqlDialect::Oracle).risks,
        vec![SqlRisk::Unknown]
    );
}

#[test]
fn snowflake_side_effecting_expressions_require_approval_in_every_query_position() {
    for sql in [
        "SELECT DB.PUBLIC.S.NEXTVAL",
        "SELECT s.nextval",
        "SELECT \"DB\".\"PUBLIC\".\"S\".\"NEXTVAL\"",
        "SELECT SYSTEM$CANCEL_QUERY('query-id')",
        "SELECT system$abort_session(123)",
        "SELECT SYSTEM$WAIT(1)",
        "SELECT GETNEXTVAL('s')",
        "SELECT * FROM TABLE(GETNEXTVAL(s))",
        "WITH ids AS (SELECT DB.PUBLIC.S.NEXTVAL AS id) SELECT * FROM ids",
        "SELECT (SELECT SYSTEM$CANCEL_QUERY('query-id'))",
        "SELECT 1 WHERE SYSTEM$CANCEL_QUERY('query-id') IS NOT NULL",
        "SELECT 1 ORDER BY DB.PUBLIC.S.NEXTVAL",
        "SELECT 1 UNION ALL SELECT DB.PUBLIC.S.NEXTVAL",
        "EXPLAIN SELECT DB.PUBLIC.S.NEXTVAL",
    ] {
        assert!(
            sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::SnowflakeDialect {}, sql)
                .is_ok(),
            "regression must exercise a parsed expression: {sql}"
        );
        assert_eq!(
            classify_sql(sql, SqlDialect::Snowflake).risks,
            vec![SqlRisk::Unknown],
            "{sql}"
        );
    }
    let analysis = classify_sql(
        "SELECT 1; SELECT DB.PUBLIC.S.NEXTVAL; SELECT SYSTEM$CANCEL_QUERY('id')",
        SqlDialect::Snowflake,
    );
    assert_eq!(
        analysis.risks,
        vec![SqlRisk::ReadOnly, SqlRisk::Unknown, SqlRisk::Unknown]
    );
    assert_eq!(analysis.aggregate, SqlRiskAggregate::MultiStatement);
}

#[test]
fn snowflake_risk_classification_ignores_side_effect_names_in_text_and_aliases() {
    for sql in [
        "SELECT 1",
        "SELECT COUNT(*) FROM DB.PUBLIC.USERS",
        "SELECT 'DB.PUBLIC.S.NEXTVAL', 'SYSTEM$CANCEL_QUERY()'",
        "SELECT 1 AS NEXTVAL, 2 AS \"SYSTEM$CANCEL_QUERY\"",
        "SELECT 1 /* SELECT DB.PUBLIC.S.NEXTVAL */",
        "SELECT 1 -- SYSTEM$CANCEL_QUERY('id')",
        "WITH rows AS (SELECT 1) SELECT * FROM rows",
    ] {
        assert_eq!(
            classify_sql(sql, SqlDialect::Snowflake).risks,
            vec![SqlRisk::ReadOnly],
            "{sql}"
        );
    }
}

#[test]
fn duckdb_file_and_extension_side_effects_are_unknown() {
    for sql in [
        "COPY (SELECT 1) TO '/tmp/export.csv'",
        "ATTACH '/tmp/other.duckdb' AS other",
        "DETACH other",
        "INSTALL httpfs",
        "LOAD httpfs",
        "CREATE SECRET s (TYPE S3, KEY_ID 'x', SECRET 'y')",
        "SET memory_limit = '1GB'",
        "SELECT nextval('s')",
        "SELECT setval('s', 4)",
    ] {
        let analysis = classify_sql(sql, SqlDialect::DuckDb);
        assert_eq!(analysis.risks, vec![SqlRisk::Unknown], "{sql}");
    }
    assert_eq!(
        classify_sql("SELECT 1; SELECT 2", SqlDialect::DuckDb).risks,
        vec![SqlRisk::ReadOnly, SqlRisk::ReadOnly]
    );
}
