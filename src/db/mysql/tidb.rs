//! TiDB's first native adapter phase targets the stable 8.5 LTS series (>=8.5.0).
//! The leading VERSION() triplet describes MySQL compatibility, not TiDB itself.
//! Reference: https://docs.pingcap.com/tidb/v8.5/sql-statement-start-transaction/
//! READ ONLY requires tidb_enable_noop_functions and never enforces read-only
//! access. Use the supported snapshot begin syntax instead. These private
//! metadata transactions issue only reads; user SQL retains normal rules.

use std::ops::ControlFlow;

use sqlparser::{
    ast::{Expr, visit_expressions},
    dialect::MySqlDialect,
    parser::Parser,
};

use super::*;

pub(super) const CATALOG_BEGIN_SQL: &str = "START TRANSACTION WITH CONSISTENT SNAPSHOT";

// TiDB 8.5 exposes EXPRESSION, but NON_UNIQUE is varchar and SEQ_IN_INDEX
// is signed bigint. Cast explicitly to the types used by the shared decoder.
// https://docs.pingcap.com/tidb/v8.5/information-schema-statistics/
pub(super) const INDEXES_SQL: &str = r#"
SELECT index_name, CAST(non_unique AS SIGNED), CAST(seq_in_index AS UNSIGNED), column_name, expression
FROM information_schema.statistics
WHERE BINARY table_schema=BINARY ? AND BINARY table_name=BINARY ?
ORDER BY BINARY index_name, seq_in_index
"#;

pub(super) const GROUP_COUNTS_SQL: &str = "SELECT \
    (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='BASE TABLE') AS tables, \
    (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='VIEW') AS views";

pub(super) fn product_version(version: &str) -> Option<(u32, u32, u32)> {
    let version = version.to_ascii_lowercase();
    if version.contains("mariadb") {
        return None;
    }
    let (_, product) = version.split_once("-tidb-v")?;
    let patch_end = product
        .char_indices()
        .find(|(_, character)| !character.is_ascii_digit() && *character != '.')
        .map_or(product.len(), |(index, _)| index);
    // Development/RC builds do not establish the stable LTS baseline.
    let suffix = &product[patch_end..];
    if suffix.starts_with("-alpha") || suffix.starts_with("-beta") || suffix.starts_with("-rc") {
        return None;
    }
    let core = &product[..patch_end];
    if core.split('.').count() != 3 {
        return None;
    }
    parse_version_triplet(core)
}

pub(super) fn search_candidates_sql() -> String {
    // Remove unsupported sources from the SQL itself: TiDB's compatibility
    // information_schema placeholders must never be treated as real objects.
    let start = CATALOG_SEARCH_CANDIDATES_SQL
        .find("    UNION ALL\n    SELECT LOWER(routine_type)")
        .expect("routine search branch");
    let end = CATALOG_SEARCH_CANDIDATES_SQL
        .find("    UNION ALL\n    SELECT 'column'")
        .expect("column search branch");
    format!(
        "{}{}",
        &CATALOG_SEARCH_CANDIDATES_SQL[..start],
        &CATALOG_SEARCH_CANDIDATES_SQL[end..]
    )
}

fn contains_sequence_mutation(sql: &str) -> bool {
    let Ok(statements) = Parser::parse_sql(&MySqlDialect {}, sql) else {
        return true;
    };
    visit_expressions(&statements, |expression| {
        if let Expr::Function(function) = expression
            && function
                .name
                .0
                .last()
                .and_then(|part| part.as_ident())
                .is_some_and(|name| {
                    name.value.eq_ignore_ascii_case("nextval")
                        || name.value.eq_ignore_ascii_case("setval")
                })
        {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    })
    .is_break()
}

// TiDB's compatibility READ ONLY flag does not enforce policy. Check the whole
// batch, including sequence writes inside otherwise read-only query expressions.
pub(super) fn validate_read_only_sql(sql: &str) -> Result<(), DatabaseError> {
    let analysis = crate::sql::classify_sql(sql, crate::sql::SqlDialect::MySql);
    if analysis.risks.is_empty()
        || analysis
            .risks
            .iter()
            .any(|risk| *risk != crate::sql::SqlRisk::ReadOnly)
        || contains_sequence_mutation(sql)
    {
        return Err(DatabaseError::unsupported(
            "TiDB profile is read-only; only SQL classified as read-only can execute",
        ));
    }
    Ok(())
}

pub(super) fn supported_relation(kind: CatalogKind) -> bool {
    matches!(kind, CatalogKind::Table | CatalogKind::View)
}

impl MySqlAdapter {
    /// TiDB 8.5 LTS catalog fields read directly from INFORMATION_SCHEMA.
    pub fn tidb_catalog_capabilities() -> CatalogCapabilities {
        let mut capabilities = Self::catalog_capabilities();
        capabilities.top_level_groups = vec![ObjectGroup::Tables, ObjectGroup::Views];
        capabilities
    }

    pub(super) fn reject_tidb_operation(&self, operation: &str) -> Result<(), DatabaseError> {
        if self.kind == DatabaseKind::TiDb {
            return Err(DatabaseError::unsupported(format!(
                "TiDB {operation} is not enabled in the initial native adapter; use native SQL with normal client permissions"
            )));
        }
        Ok(())
    }

    pub(super) fn catalog_begin_sql(&self) -> &'static str {
        if self.kind == DatabaseKind::TiDb {
            CATALOG_BEGIN_SQL
        } else {
            CATALOG_PAGE_BEGIN_SQL
        }
    }

    pub(super) async fn check_tidb_catalog_version(
        &self,
        connection: &mut MySqlConnection,
    ) -> Result<(), DatabaseError> {
        if self.kind == DatabaseKind::TiDb {
            let version: String = sqlx::query_scalar("SELECT VERSION()")
                .fetch_one(connection)
                .await
                .map_err(sql_error)?;
            if !supports_catalog_version_for_kind(self.kind, &version) {
                return Err(unsupported_catalog_version(self.kind, &version));
            }
        }
        Ok(())
    }

    // SQLx's raw MySQL row stream does not emit metadata for zero-row results.
    // Prepare read statements for metadata, then consume the original SQL once.
    // One acquired connection preserves session state across the whole batch.
    pub(super) async fn execute_tidb_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        if self.read_only {
            validate_read_only_sql(sql)?;
        }
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let mut accumulator = QueryOutcomeAccumulator::with_budget(budget);
        for range in crate::sql::scan_statements(sql, crate::sql::SqlDialect::MySql) {
            let statement = &sql[range.start..range.end];
            if crate::sql::classify_statement_kind(statement, crate::sql::SqlDialect::MySql)
                == crate::sql::SqlStatementKind::Dql
            {
                let description = connection
                    .prepare(AssertSqlSafe(statement).into_sql_str())
                    .await
                    .map_err(sql_error)?;
                accumulator.metadata(
                    description
                        .columns()
                        .iter()
                        .map(|column| ColumnMeta {
                            name: column.name().to_owned(),
                            type_name: column.type_info().name().to_owned(),
                        })
                        .collect(),
                );
            }
            let mut stream = sqlx::raw_sql(AssertSqlSafe(statement)).fetch_many(&mut *connection);
            while let Some(event) = stream.try_next().await.map_err(sql_error)? {
                match event {
                    Either::Right(row) => accumulator.row(columns(&row), decode_row(&row)),
                    Either::Left(done) => accumulator.done(done.rows_affected()),
                }
            }
        }
        Ok(accumulator.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_policy_validates_the_entire_batch() {
        for sql in [
            "SELECT 1",
            "SELECT 1; SELECT 2",
            "SELECT 'DELETE FROM t' AS text",
            "SELECT 'NEXTVAL(seq)' AS text /* SETVAL(seq, 1) */",
            "SELECT LASTVAL(seq)",
        ] {
            assert!(validate_read_only_sql(sql).is_ok(), "{sql}");
        }
        for sql in [
            "SELECT 1; DELETE FROM t",
            "INSERT INTO t VALUES (1)",
            "BEGIN",
            "COMMIT",
            "SET @x=1",
            "nonsense",
            "SELECT 1 FOR UPDATE",
            "SELECT NEXTVAL(seq)",
            "SELECT `nextval`(seq)",
            "SELECT SETVAL(seq, 100)",
            "SELECT 1; SELECT (SELECT NEXTVAL(seq))",
            "SELECT * FROM items WHERE id = NEXTVAL(seq)",
            "WITH values_cte AS (SELECT NEXTVAL(seq)) SELECT * FROM values_cte",
            "SELECT NEXT VALUE FOR seq",
        ] {
            assert!(validate_read_only_sql(sql).is_err(), "{sql}");
        }
    }

    #[tokio::test]
    async fn excluded_adapter_operations_fail_before_acquiring_a_connection() {
        let adapter = MySqlAdapter {
            pool: MySqlPoolOptions::new().connect_lazy_with(MySqlConnectOptions::new()),
            kind: DatabaseKind::TiDb,
            connection_id: Uuid::new_v4(),
            catalog_scope: CatalogScope::for_profile(DatabaseKind::TiDb, "test", None),
            read_only: false,
        };
        assert!(
            matches!(adapter.transaction_backend().await, Err(error) if error.category == ErrorCategory::Unsupported)
        );
        assert_eq!(
            adapter.list_principals().await.unwrap_err().category,
            ErrorCategory::Unsupported
        );
        assert_eq!(
            adapter.load_monitor_snapshot().await.unwrap_err().category,
            ErrorCategory::Unsupported
        );
        assert_eq!(
            adapter.load_monitor_metadata().await.unwrap_err().category,
            ErrorCategory::Unsupported
        );
        assert_eq!(
            adapter.load_process_snapshot().await.unwrap_err().category,
            ErrorCategory::Unsupported
        );
        for kind in [
            CatalogKind::Function,
            CatalogKind::Procedure,
            CatalogKind::Trigger,
            CatalogKind::Sequence,
        ] {
            assert_eq!(
                adapter
                    .object_ddl(kind, "test", "unavailable")
                    .await
                    .unwrap_err()
                    .category,
                ErrorCategory::Unsupported
            );
        }
        adapter.close().await;
    }

    #[test]
    fn tidb_search_never_queries_unsupported_programmable_objects() {
        let sql = search_candidates_sql();
        for source in ["routines", "triggers", "events"] {
            assert!(!sql.contains(&format!("information_schema.{source}")));
        }
        assert!(sql.contains("information_schema.tables"));
        assert!(sql.contains("information_schema.columns"));
        assert!(sql.contains("information_schema.statistics"));
        assert!(sql.contains("{scope_predicate}"));
    }
}
