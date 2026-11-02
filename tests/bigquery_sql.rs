use lazydb::{
    agent::policy::{PolicyError, WritePolicy, authorize_query, authorize_write},
    db::catalog::{CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName},
    profile::{DatabaseKind, Environment, import_connection_url},
    sql::{
        CompletionContext, CompletionIndex, CompletionInsertionMode, CompletionKind, HighlightKind,
        SqlDialect, SqlRisk, SqlRiskAggregate, bounded_query, classify_sql, complete,
        complete_with_mode, derived_query_capable, format_sql, highlight_sql, quote_identifier,
        scan_statements, should_offer_completion_for_dialect,
    },
};
use sqlparser::{dialect::BigQueryDialect, parser::Parser};
use uuid::Uuid;

#[test]
fn bigquery_tui_blocks_read_only_writes_and_rechecks_confirmation() {
    use lazydb::{
        action::{Action, Command},
        app::App,
        db::ServerInfo,
        model::{execution_target::ExecutionTarget, workspace::Overlay},
    };
    for change_after_confirmation in [false, true] {
        let mut profile =
            import_connection_url("bigquery://billing-project/analytics", Some("BigQuery"))
                .unwrap()
                .profile;
        profile.read_only = !change_after_confirmation;
        let id = profile.id;
        let target = ExecutionTarget::from_profile(&profile);
        let mut app = Box::new(App::new(vec![profile]));
        app.update(Action::RequestConnect(id));
        app.update(Action::ConnectionSucceeded {
            profile_id: id,
            generation: app.connection.pending_generation.unwrap(),
            server: ServerInfo {
                kind: DatabaseKind::BigQuery,
                version: "test".into(),
                database: "billing-project".into(),
                current_user: None,
            },
            mutation_capabilities: Default::default(),
        });
        app.update(Action::NewUnboundConsoleNamed("BigQuery".into()));
        app.active_console_mut().execution_target = Some(target);
        app.update(Action::ReplaceEditor(
            "DELETE FROM analytics.events WHERE TRUE".into(),
        ));
        let mut commands = app.update(Action::RunActiveSql);
        assert_eq!(
            matches!(app.overlay, Some(Overlay::ExecutionConfirm { .. })),
            change_after_confirmation
        );
        if change_after_confirmation {
            app.profiles[0].read_only = true;
            app.update(Action::ToggleExecutionConfirmationFocus);
            commands.extend(app.update(Action::ConfirmExecution));
        }
        assert!(!commands.iter().any(|command| matches!(
            command,
            Command::RunQuery { .. } | Command::RunQueryPage { .. }
        )));
        assert!(app.active_console().last_execution.is_none());
    }
}

#[test]
fn bigquery_preview_sort_cycles_escaped_identifiers() {
    use lazydb::sql::{cycle_relation_column_sort, relation_column_sort_projection};
    for name in ["order", "spaced column", "odd`name\\suffix"] {
        let columns = [name];
        let quoted = quote_identifier(name, SqlDialect::BigQuery);
        let desc = cycle_relation_column_sort("", &columns, 0, SqlDialect::BigQuery).unwrap();
        assert_eq!(desc, format!("{quoted} DESC"));
        let projection =
            relation_column_sort_projection(&desc, &columns, SqlDialect::BigQuery).unwrap();
        assert!(projection[0].is_some());
        let asc = cycle_relation_column_sort(&desc, &columns, 0, SqlDialect::BigQuery).unwrap();
        assert_eq!(asc, format!("{quoted} ASC"));
        assert_eq!(
            cycle_relation_column_sort(&asc, &columns, 0, SqlDialect::BigQuery).unwrap(),
            ""
        );
    }
}

#[test]
fn bigquery_preview_filters_use_googlesql_compatibility() {
    use lazydb::sql::validate_relation_preview_options;
    for filter in [
        r"`odd\`name` = 1",
        "id IN UNNEST(ARRAY<INT64>[1,2])",
        "EXISTS (SELECT 1 FROM UNNEST(ARRAY<STRUCT<x INT64, ys ARRAY<INT64>>>[(1, [2])]))",
    ] {
        let options = validate_relation_preview_options(filter, "", SqlDialect::BigQuery).unwrap();
        assert_eq!(options.where_clause.as_deref(), Some(filter));
    }
}

fn index() -> CompletionIndex {
    let connection = Uuid::new_v4();
    let schema = CatalogId::new(
        connection,
        CatalogKind::Schema,
        ["data-project", "analytics"],
    );
    let table = CatalogId::new(
        connection,
        CatalogKind::Table,
        ["data-project", "analytics", "events"],
    );
    CompletionIndex::new(&[
        CatalogEntry::schema(
            schema.clone(),
            CatalogId::new(connection, CatalogKind::Database, ["data-project"]),
            QualifiedName {
                database: Some("data-project".into()),
                schema: Some("analytics".into()),
                object: "analytics".into(),
            },
            "dataset",
            OptionalMetadata::Unsupported,
            true,
        )
        .unwrap(),
        CatalogEntry::relation(
            table,
            schema,
            QualifiedName {
                database: Some("data-project".into()),
                schema: Some("analytics".into()),
                object: "events".into(),
            },
            "TABLE",
            OptionalMetadata::Unsupported,
            true,
        )
        .unwrap(),
    ])
}

#[test]
fn google_sql_dialect_and_escaped_identifier_round_trip() {
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::BigQuery),
        SqlDialect::BigQuery
    );
    let path = "data-project.analytics.events";
    assert_eq!(
        quote_identifier(path, SqlDialect::BigQuery),
        "`data-project.analytics.events`"
    );
    assert_eq!(
        quote_identifier("odd`name\\suffix", SqlDialect::BigQuery),
        "`odd\\`name\\\\suffix`"
    );
    // sqlparser 0.62 does not decode backslash escapes in quoted identifiers.
    // Ordinary quoted identifiers still round-trip through its official API.
    let identifier = "odd name";
    let sql = format!(
        "SELECT 1 AS {}",
        quote_identifier(identifier, SqlDialect::BigQuery)
    );
    let statements = Parser::parse_sql(&BigQueryDialect, &sql).unwrap();
    assert_eq!(statements.len(), 1);
    assert!(matches!(
        &statements[0], sqlparser::ast::Statement::Query(query)
        if matches!(query.body.as_ref(), sqlparser::ast::SetExpr::Select(select)
            if matches!(&select.projection[0], sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } if alias.value == identifier))
    ));
}

#[test]
fn google_sql_scanner_preserves_literal_semicolons_and_comments() {
    let sql = "# comment ;\nSELECT r'''raw; 'inside';''', b\"\"\"bytes; \"inside\";\"\"\", \"escaped \\\"; value\", `data-project.analytics.odd;table`; -- comment ;\nSELECT ARRAY<STRUCT<x INT64>>[STRUCT(1)]";
    let ranges = scan_statements(sql, SqlDialect::BigQuery);
    assert_eq!(ranges.len(), 2, "{ranges:?}");
    assert!(
        ranges[0]
            .get(sql)
            .unwrap()
            .ends_with("`data-project.analytics.odd;table`;")
    );
    assert!(
        ranges[1]
            .get(sql)
            .unwrap()
            .contains("ARRAY<STRUCT<x INT64>>")
    );
    assert!(scan_statements("# only comment ;\n/* another ; */", SqlDialect::BigQuery).is_empty());
    assert_eq!(
        scan_statements("SELECT '''unfinished; SELECT 2", SqlDialect::BigQuery).len(),
        1
    );
}

#[test]
fn google_sql_classifies_arrays_structs_and_unnest_as_read_only() {
    for sql in [
        "SELECT * FROM `data-project.analytics.events`",
        "SELECT AS STRUCT 1 AS id, ARRAY<INT64>[1, 2] AS values",
        "SELECT item.x FROM UNNEST([STRUCT(1 AS x)]) AS item",
        "SELECT item.x FROM UNNEST(ARRAY<STRUCT<x INT64>>[STRUCT(1)]) AS item",
        "SELECT 1 AS `odd\\`name\\\\suffix`",
        "SELECT `odd\\`name` FROM `data-project.analytics.events`",
        "SELECT `odd\\`名` FROM `data-project.analytics.events`",
        "WITH data AS (SELECT 1 AS id) SELECT ARRAY_AGG(STRUCT(id)) FROM data",
        "SELECT * EXCEPT (secret) FROM `data-project.analytics.events` QUALIFY ROW_NUMBER() OVER (ORDER BY id) = 1",
    ] {
        assert_eq!(
            classify_sql(sql, SqlDialect::BigQuery).aggregate,
            SqlRiskAggregate::Single(SqlRisk::ReadOnly),
            "{sql}"
        );
    }
    for sql in [
        "INSERT INTO `data-project.analytics.events` (id) VALUES (1)",
        "UPDATE `data-project.analytics.events` SET id = 2 WHERE id = 1",
        "DELETE FROM `data-project.analytics.events` WHERE TRUE",
        "MERGE `data-project.analytics.events` t USING (SELECT 1 id) s ON t.id = s.id WHEN MATCHED THEN DELETE",
    ] {
        assert_eq!(
            classify_sql(sql, SqlDialect::BigQuery).risks,
            vec![SqlRisk::Dml],
            "{sql}"
        );
    }
}

#[test]
fn google_sql_rejects_scripts_sessions_and_unknown_execution() {
    let mut profile = import_connection_url(
        "bigquery://billing-project/analytics?data_project=data-project",
        Some("bq"),
    )
    .unwrap()
    .profile;
    profile.read_only = false;
    profile.environment = Environment::Development;
    assert_eq!(
        authorize_query(&profile, "SELECT ARRAY<INT64>[1, 2]"),
        Ok(())
    );
    for sql in [
        "EXECUTE IMMEDIATE 'SELECT 1'",
        "CALL `data-project.analytics.proc`()",
        "FROM `data-project.analytics.events` |> CALL custom_proc()",
        "SELECT (FROM `data-project.analytics.events` |> CALL custom_proc())",
        "SET @@dataset_id = 'other'",
        "DECLARE x INT64 DEFAULT 1",
        "BEGIN SELECT 1; END",
        "SELECT 1; SELECT 2",
        "UPDATE t SET id = 1; DELETE FROM t WHERE TRUE",
        "SELECT 1; UPDATE t SET id = 1",
        "SELECT 1 AS `odd\\`name`; DELETE FROM t WHERE TRUE",
        "SELECT 1 AS `odd\\`name; DELETE FROM t WHERE TRUE",
        "SELECT 1 AS `odd\\\\`; DELETE FROM t WHERE TRUE",
        "not valid sql",
    ] {
        assert_eq!(
            authorize_query(&profile, sql),
            Err(PolicyError::ReadOnlyQueryRequired),
            "{sql}"
        );
        assert_eq!(
            authorize_write(&profile, WritePolicy::All, sql),
            Err(PolicyError::UnknownSql),
            "{sql}"
        );
    }
    for sql in [
        "BEGIN TRANSACTION",
        "COMMIT TRANSACTION",
        "ROLLBACK TRANSACTION",
    ] {
        assert_eq!(
            authorize_query(&profile, sql),
            Err(PolicyError::ReadOnlyQueryRequired),
            "{sql}"
        );
        assert_eq!(
            authorize_write(&profile, WritePolicy::All, sql),
            Err(PolicyError::TransactionControlDisabled),
            "{sql}"
        );
    }
    let update = "UPDATE `data-project.analytics.events` SET id = 1 WHERE TRUE";
    assert_eq!(
        authorize_write(&profile, WritePolicy::Deny, update),
        Err(PolicyError::ServerWritePolicyDenied)
    );
    assert_eq!(
        authorize_write(&profile, WritePolicy::NonProduction, update),
        Ok(())
    );
    assert_eq!(profile.database.as_deref(), Some("billing-project"));
    assert_eq!(profile.catalog_database(), Some("data-project"));
}

#[test]
fn google_sql_completion_preserves_data_project_and_quotes_whole_path() {
    let index = index();
    for text in [
        "SELECT * FROM ",
        "SELECT * FROM `data-project.analytics.ev",
        "SELECT * FROM `data-project.analytics.ev`",
    ] {
        for mode in [
            CompletionInsertionMode::Contextual,
            CompletionInsertionMode::CurrentSegment,
        ] {
            let candidates = complete_with_mode(
                text,
                text.len(),
                SqlDialect::BigQuery,
                &index,
                CompletionContext {
                    database: Some("billing-project"),
                    schema: Some("analytics"),
                },
                mode,
            );
            let table = candidates
                .iter()
                .find(|candidate| candidate.kind == CompletionKind::Table)
                .unwrap_or_else(|| panic!("{text}: {candidates:?}"));
            let mut output = text.to_owned();
            output.replace_range(table.replace.start..table.replace.end, &table.insert_text);
            assert_eq!(
                output, "SELECT * FROM `data-project.analytics.events`",
                "{text}"
            );
            Parser::parse_sql(&BigQueryDialect, &output).unwrap();
        }
    }
    let text = "SELECT * FROM analytics.ev";
    let table = complete(
        text,
        text.len(),
        SqlDialect::BigQuery,
        &index,
        CompletionContext::default(),
    )
    .into_iter()
    .find(|candidate| candidate.kind == CompletionKind::Table)
    .unwrap();
    let mut output = text.to_owned();
    output.replace_range(table.replace.start..table.replace.end, &table.insert_text);
    assert_eq!(output, "SELECT * FROM `analytics.events`");
}

#[test]
fn google_sql_completion_loads_columns_from_whole_quoted_table_path() {
    let index = index();
    let text = "SELECT e. FROM `data-project.analytics.events` AS e";
    let dependencies = lazydb::sql::completion_dependencies(
        text,
        "SELECT e.".len(),
        SqlDialect::BigQuery,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(dependencies.relation_children.len(), 1);
    assert_eq!(
        dependencies.relation_children[0].native_path,
        ["data-project", "analytics", "events"]
    );
}

#[test]
fn google_sql_completion_knows_native_types_and_relation_unnest() {
    let index = CompletionIndex::default();
    for (text, expected) in [
        ("SELECT * FROM UN", "UNNEST"),
        ("CREATE TABLE t (value INT", "INT64"),
        ("SELECT STR", "STRUCT"),
    ] {
        let candidates = complete(
            text,
            text.len(),
            SqlDialect::BigQuery,
            &index,
            CompletionContext::default(),
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.label == expected),
            "{text}: {candidates:?}"
        );
    }
    for text in [
        "SELECT \"unfinished",
        "SELECT r'''unfinished",
        "# comment",
        "SELECT 1 /* comment",
    ] {
        assert!(
            !should_offer_completion_for_dialect(text, text.len(), SqlDialect::BigQuery),
            "{text}"
        );
    }
    let text = "SELECT * FROM `data-project.analytics.ev";
    assert!(should_offer_completion_for_dialect(
        text,
        text.len(),
        SqlDialect::BigQuery
    ));
}

#[test]
fn google_sql_formatter_and_highlight_preserve_native_literals() {
    let sql =
        "select item.x, r'''semi; 'inside' ''' as raw_text from unnest([struct(1 as x)]) item";
    let formatted = format_sql(sql, SqlDialect::BigQuery).unwrap();
    Parser::parse_sql(&BigQueryDialect, &formatted).unwrap();
    assert!(formatted.contains("r'''semi; 'inside' '''"));
    for source in [
        "select 1 as `odd\\`name`",
        "select array<struct<x int64>>[struct(1)]",
    ] {
        let formatted = format_sql(source, SqlDialect::BigQuery).unwrap();
        assert_eq!(
            classify_sql(&formatted, SqlDialect::BigQuery).aggregate,
            SqlRiskAggregate::Single(SqlRisk::ReadOnly)
        );
        assert!(lazydb::sql::diagnose_sql(&formatted, SqlDialect::BigQuery).is_empty());
    }
    let text = "SELECT r'''semi; inside''', b\"bytes\", \"text\" FROM `data-project.analytics.events` # comment";
    let spans = highlight_sql(text, SqlDialect::BigQuery);
    for expected in ["r'''semi; inside'''", "b\"bytes\"", "\"text\""] {
        assert!(
            spans
                .iter()
                .any(|span| span.kind == HighlightKind::String
                    && span.range.get(text) == Some(expected)),
            "{expected}: {spans:?}"
        );
    }
    assert!(
        spans.iter().any(|span| span.kind == HighlightKind::Relation
            && span.range.get(text) == Some("`data-project.analytics.events`")),
        "{spans:?}"
    );
}

#[test]
fn google_sql_derived_results_do_not_wrap_value_tables_or_ordered_queries() {
    for sql in [
        "SELECT AS VALUE STRUCT(1 AS id)",
        "SELECT 1 AS id ORDER BY id",
        "SELECT ARRAY<INT64>[1, 2]",
    ] {
        assert!(!derived_query_capable(sql, SqlDialect::BigQuery));
        assert_eq!(bounded_query(sql, SqlDialect::BigQuery), None);
    }
}
