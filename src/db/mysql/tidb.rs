//! TiDB's first native adapter phase targets the stable 8.5 LTS series (>=8.5.0).
//! The leading VERSION() triplet describes MySQL compatibility, not TiDB itself.
//! Reference: https://docs.pingcap.com/tidb/v8.5/sql-statement-start-transaction/
//! READ ONLY requires tidb_enable_noop_functions and never enforces read-only
//! access. Use the supported snapshot begin syntax instead. These private
//! metadata transactions issue only reads; user SQL retains normal rules.

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
    CATALOG_SEARCH_SQL.to_owned()
}

const CATALOG_SEARCH_SQL: &str = r#"
WITH candidates AS (
    SELECT 'database' AS kind, schema_name AS database_name, schema_name AS object_name,
           NULL AS relation_name, NULL AS relation_type, schema_name AS native_identity,
           schema_name AS qualified_path, NULL AS comment
    FROM information_schema.schemata
    WHERE schema_name NOT IN ('information_schema','mysql','performance_schema','sys')
    UNION ALL
    SELECT 'schema', schema_name, schema_name, NULL, NULL, schema_name, schema_name, NULL
    FROM information_schema.schemata
    WHERE schema_name NOT IN ('information_schema','mysql','performance_schema','sys')
    UNION ALL
    SELECT IF(table_type='VIEW','view','table'), table_schema, table_name, table_name, table_type,
           table_name, CONCAT(table_schema,'.',table_name), table_comment
    FROM information_schema.tables WHERE table_type IN ('BASE TABLE','VIEW')
    UNION ALL
    SELECT 'column', c.table_schema, c.column_name, c.table_name, t.table_type,
           CAST(c.ordinal_position AS CHAR), CONCAT(c.table_schema,'.',c.table_name,'.',c.column_name), NULL
    FROM information_schema.columns c JOIN information_schema.tables t
      ON BINARY t.table_schema=BINARY c.table_schema AND BINARY t.table_name=BINARY c.table_name
     AND t.table_type IN ('BASE TABLE','VIEW')
    UNION ALL
    SELECT 'index', s.table_schema, s.index_name, s.table_name, t.table_type, s.index_name,
           CONCAT(s.table_schema,'.',s.table_name,'.',s.index_name), NULL
    FROM information_schema.statistics s JOIN information_schema.tables t
      ON BINARY t.table_schema=BINARY s.table_schema AND BINARY t.table_name=BINARY s.table_name
     AND t.table_type IN ('BASE TABLE','VIEW')
    GROUP BY s.table_schema, s.table_name, s.index_name, t.table_type
    UNION ALL
    SELECT CASE constraint_type WHEN 'PRIMARY KEY' THEN 'primary_key'
               WHEN 'UNIQUE' THEN 'unique_constraint' ELSE 'foreign_key' END,
           tc.table_schema, tc.constraint_name, tc.table_name, t.table_type, tc.constraint_name,
           CONCAT(tc.table_schema,'.',tc.table_name,'.',tc.constraint_name), NULL
    FROM information_schema.table_constraints tc JOIN information_schema.tables t
      ON BINARY t.table_schema=BINARY tc.table_schema AND BINARY t.table_name=BINARY tc.table_name
     AND t.table_type IN ('BASE TABLE','VIEW')
    WHERE tc.constraint_type IN ('PRIMARY KEY','UNIQUE','FOREIGN KEY')
), normalized AS (
    SELECT *, REGEXP_REPLACE(LOWER(object_name), '[^[:alnum:]]', '') AS normalized_name,
              REGEXP_REPLACE(LOWER(qualified_path), '[^[:alnum:]]', '') AS normalized_path
    FROM candidates
), searchable AS (
    SELECT *, IF(?, normalized_name, LOWER(object_name)) AS search_name,
              IF(?, normalized_path, LOWER(qualified_path)) AS search_path
    FROM normalized
)
SELECT kind, database_name, object_name, relation_name, relation_type, native_identity, comment
FROM searchable
WHERE {scope_predicate}
  AND database_name NOT IN ('information_schema','mysql','performance_schema','sys')
  AND (? OR kind IN ('table','view'))
  AND (LOCATE(?, search_name) > 0 OR LOCATE(?, search_path) > 0)
ORDER BY CASE
    WHEN search_name=? THEN 0
    WHEN LOCATE(?, search_name)=1 THEN 1
    WHEN LOCATE(?, search_name)>0 THEN 2
    ELSE 3 END,
    LOWER(qualified_path), kind, BINARY native_identity
LIMIT 101
"#;

// TiDB's compatibility READ ONLY flag does not enforce policy. Check the whole
// batch, including sequence writes inside otherwise read-only query expressions.
pub(super) fn validate_read_only_sql(sql: &str) -> Result<(), DatabaseError> {
    let analysis = crate::sql::classify_sql(sql, crate::sql::SqlDialect::MySql);
    if analysis.risks.is_empty()
        || analysis
            .risks
            .iter()
            .any(|risk| *risk != crate::sql::SqlRisk::ReadOnly)
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
        let mut sql_mode: String = sqlx::query_scalar("SELECT @@SESSION.sql_mode")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let mut accumulator = QueryOutcomeAccumulator::with_budget(budget);
        let mut remaining = sql;
        loop {
            let ansi_quotes = sql_mode
                .split(',')
                .any(|mode| mode.eq_ignore_ascii_case("ANSI_QUOTES"));
            let no_backslash_escapes = sql_mode
                .split(',')
                .any(|mode| mode.eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES"));
            if self.read_only && (ansi_quotes || no_backslash_escapes) {
                return Err(DatabaseError::unsupported(
                    "TiDB read-only SQL validation requires the default SQL quote modes",
                ));
            }
            let Some(range) =
                crate::sql::scan_mysql_statements(remaining, ansi_quotes, no_backslash_escapes)
                    .into_iter()
                    .next()
            else {
                break;
            };
            let statement = &remaining[range.start..range.end];
            let is_query =
                crate::sql::classify_statement_kind(statement, crate::sql::SqlDialect::MySql)
                    == crate::sql::SqlStatementKind::Dql;
            if is_query {
                let description = connection
                    .prepare(AssertSqlSafe(statement).into_sql_str())
                    .await
                    .map_err(sql_error)?;
                accumulator.prepared_metadata(
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
            drop(stream);
            remaining = &remaining[range.end..];
            if !is_query && !remaining.trim().is_empty() {
                sql_mode = sqlx::query_scalar("SELECT @@SESSION.sql_mode")
                    .fetch_one(&mut *connection)
                    .await
                    .map_err(sql_error)?;
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
            "SELECT (SELECT id FROM items WHERE id=1 FOR UPDATE)",
            "SELECT * FROM items WHERE EXISTS (SELECT id FROM items FOR UPDATE)",
            "/*!40101 SELECT NEXTVAL(seq) */",
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
