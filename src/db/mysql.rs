use std::collections::HashMap;

use futures_util::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    AssertSqlSafe, Column, Connection, Either, Executor, MySqlPool, Row, TypeInfo, ValueRef,
    mysql::{
        MySql, MySqlConnectOptions, MySqlConnection, MySqlPoolOptions, MySqlRow, MySqlSslMode,
    },
    pool::PoolConnection,
};
use sqlx_core::transaction::TransactionManager;
use uuid::Uuid;

use crate::profile::{ConnectionProfile, DatabaseKind, SslMode};

use super::transaction::{TransactionBackend, TransactionError};
use super::{
    DatabaseError, ErrorCategory, ServerInfo,
    catalog::{CatalogId, CatalogKind, CatalogNode},
    query::{ColumnMeta, QueryOutcome, QueryOutcomeAccumulator},
    value::CellValue,
};

pub const CATALOG_TABLES_SQL: &str = r#"
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_schema = DATABASE()
ORDER BY table_name
"#;

const CATALOG_COLUMNS_SQL: &str = r#"
SELECT table_schema, table_name, column_name, column_type, is_nullable, column_key
FROM information_schema.columns
WHERE table_schema = DATABASE()
ORDER BY table_name, ordinal_position
"#;

pub const CATALOG_ROUTINES_SQL: &str = r#"
SELECT routine_schema, routine_name, routine_type, data_type, dtd_identifier
FROM information_schema.routines
WHERE routine_schema = DATABASE()
ORDER BY routine_name, routine_type
"#;

pub const CATALOG_INDEXES_SQL: &str = r#"
SELECT table_schema, table_name, index_name, non_unique, seq_in_index, column_name
FROM information_schema.statistics
WHERE table_schema = DATABASE()
ORDER BY table_name, index_name, seq_in_index
"#;

const PROBE_SQL: &str = "SELECT VERSION() AS version, DATABASE() AS current_database";

const CATALOG_FOREIGN_KEYS_SQL: &str = r#"
SELECT constraint_schema AS table_schema, table_name, constraint_name,
       column_name, referenced_table_schema AS target_schema,
       referenced_table_name AS target_table,
       referenced_column_name AS target_column
FROM information_schema.key_column_usage
WHERE table_schema = DATABASE()
  AND referenced_table_name IS NOT NULL
ORDER BY table_name, constraint_name, ordinal_position
"#;

#[derive(Clone, Debug)]
pub struct MySqlAdapter {
    pool: MySqlPool,
    connection_id: Uuid,
}

impl MySqlAdapter {
    pub(crate) async fn transaction_backend(
        &self,
    ) -> Result<MySqlTransactionBackend, DatabaseError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let connection_id = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()")
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(MySqlTransactionBackend {
            connection,
            control: self.pool.clone(),
            connection_id,
            adapter: self.clone(),
        })
    }

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::MySql {
            return Err(DatabaseError::configuration("profile is not MySQL"));
        }
        let host = profile
            .host
            .as_deref()
            .ok_or_else(|| DatabaseError::configuration("MySQL profile has no host"))?;
        let mut options = MySqlConnectOptions::new()
            .host(host)
            .port(profile.port.unwrap_or(3306))
            .ssl_mode(mysql_ssl_mode(profile.ssl_mode));
        if let Some(user) = &profile.user {
            options = options.username(user);
        }
        if let Some(database) = &profile.database {
            options = options.database(database);
        }
        if let Some(password) = password {
            options = options.password(password.expose_secret());
        }

        let mut pool_options = MySqlPoolOptions::new().max_connections(6);
        if profile.read_only {
            pool_options = pool_options.after_connect(|connection, _| {
                Box::pin(async move {
                    connection
                        .execute("SET SESSION TRANSACTION READ ONLY")
                        .await?;
                    Ok(())
                })
            });
        }
        let pool = pool_options
            .connect_with(options)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(Self {
            pool,
            connection_id: profile.id,
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let row = sqlx::query(PROBE_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(ServerInfo {
            kind: DatabaseKind::MySql,
            version: row.try_get("version").map_err(decode_error)?,
            database: row
                .try_get::<Option<String>, _>("current_database")
                .map_err(decode_error)?
                .unwrap_or_default(),
        })
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_pool(sql).await
    }

    pub(crate) async fn execute_pool(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&self.pool);
        self.collect_stream(&mut stream).await
    }

    pub(crate) async fn execute_connection(
        &self,
        connection: &mut MySqlConnection,
        sql: &str,
    ) -> Result<QueryOutcome, DatabaseError> {
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut *connection);
        self.collect_stream(&mut stream).await
    }

    async fn collect_stream<E>(&self, stream: &mut E) -> Result<QueryOutcome, DatabaseError>
    where
        E: futures_util::TryStream<
                Ok = Either<sqlx::mysql::MySqlQueryResult, MySqlRow>,
                Error = sqlx::Error,
            > + Unpin,
    {
        let mut accumulator = QueryOutcomeAccumulator::new();
        while let Some(event) = stream
            .try_next()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        {
            match event {
                Either::Right(row) => {
                    accumulator.row(columns(&row), decode_row(&row));
                }
                Either::Left(done) => {
                    accumulator.done(done.rows_affected());
                }
            }
        }
        Ok(accumulator.finish())
    }

    pub async fn load_catalog(&self) -> Result<Vec<CatalogNode>, DatabaseError> {
        let database = sqlx::query_scalar::<_, Option<String>>("SELECT DATABASE()")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
            .ok_or_else(|| DatabaseError::configuration("MySQL connection has no database"))?;
        let database_id = CatalogId::new(
            self.connection_id,
            CatalogKind::Database,
            [database.clone()],
        );
        let schema_id = CatalogId::new(
            self.connection_id,
            CatalogKind::Schema,
            [database.clone(), database.clone()],
        );
        let mut nodes = vec![
            CatalogNode::new(
                database_id.clone(),
                None,
                database.clone(),
                "database",
                None,
                true,
            ),
            CatalogNode::new(
                schema_id.clone(),
                Some(database_id),
                database.clone(),
                "schema",
                Some("MySQL database/schema".to_owned()),
                true,
            ),
        ];
        let mut object_ids = HashMap::new();
        for row in sqlx::query(CATALOG_ROUTINES_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        {
            let schema: String = row.try_get("routine_schema").map_err(decode_error)?;
            let name: String = row.try_get("routine_name").map_err(decode_error)?;
            let routine_type: String = row.try_get("routine_type").map_err(decode_error)?;
            let data_type: Option<String> = row.try_get("data_type").map_err(decode_error)?;
            let signature: Option<String> = row.try_get("dtd_identifier").map_err(decode_error)?;
            let kind = if routine_type.eq_ignore_ascii_case("PROCEDURE") {
                CatalogKind::Procedure
            } else {
                CatalogKind::Function
            };
            let id = CatalogId::new(
                self.connection_id,
                kind,
                [
                    database.clone(),
                    schema.clone(),
                    name.clone(),
                    signature.clone().unwrap_or_default(),
                ],
            );
            nodes.push(CatalogNode::new(
                id,
                Some(schema_id.clone()),
                name,
                routine_type,
                Some(format!(
                    "{}{}",
                    signature.unwrap_or_default(),
                    data_type
                        .map(|value| format!(" -> {value}"))
                        .unwrap_or_default()
                )),
                false,
            ));
        }
        for row in sqlx::query(CATALOG_TABLES_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        {
            let schema: String = row.try_get("table_schema").map_err(decode_error)?;
            let name: String = row.try_get("table_name").map_err(decode_error)?;
            let native_kind: String = row.try_get("table_type").map_err(decode_error)?;
            let kind = if native_kind.eq_ignore_ascii_case("VIEW") {
                CatalogKind::View
            } else {
                CatalogKind::Table
            };
            let id = CatalogId::new(
                self.connection_id,
                kind,
                [database.clone(), schema.clone(), name.clone()],
            );
            nodes.push(CatalogNode::new(
                id.clone(),
                Some(schema_id.clone()),
                name.clone(),
                native_kind,
                None,
                true,
            ));
            object_ids.insert((schema, name), id);
        }

        for row in sqlx::query(CATALOG_COLUMNS_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        {
            let schema: String = row.try_get("table_schema").map_err(decode_error)?;
            let table: String = row.try_get("table_name").map_err(decode_error)?;
            let name: String = row.try_get("column_name").map_err(decode_error)?;
            let data_type: String = row.try_get("column_type").map_err(decode_error)?;
            let nullable: String = row.try_get("is_nullable").map_err(decode_error)?;
            let key: String = row.try_get("column_key").map_err(decode_error)?;
            if let Some(parent) = object_ids.get(&(schema, table)) {
                let mut path = parent.native_path.clone();
                path.push(name.clone());
                nodes.push(CatalogNode::new(
                    CatalogId::new(self.connection_id, CatalogKind::Column, path),
                    Some(parent.clone()),
                    name,
                    "column",
                    Some(format!(
                        "{data_type}{}{}",
                        if nullable == "NO" { " NOT NULL" } else { "" },
                        if key.is_empty() {
                            String::new()
                        } else {
                            format!(" [{key}]")
                        }
                    )),
                    false,
                ));
            }
        }
        append_indexes(&self.pool, self.connection_id, &object_ids, &mut nodes).await?;
        append_foreign_keys(&self.pool, self.connection_id, &object_ids, &mut nodes).await?;
        Ok(nodes)
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        if !matches!(kind, CatalogKind::Table | CatalogKind::View) {
            return Ok(None);
        }
        let statement = format!(
            "SHOW CREATE {} {}.{}",
            if kind == CatalogKind::View {
                "VIEW"
            } else {
                "TABLE"
            },
            quote_identifier(schema),
            quote_identifier(name)
        );
        let row = sqlx::query(AssertSqlSafe(statement))
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        row.map(|row| row.try_get::<String, _>(1).map_err(decode_error))
            .transpose()
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

pub(crate) struct MySqlTransactionBackend {
    connection: PoolConnection<MySql>,
    control: MySqlPool,
    connection_id: u64,
    adapter: MySqlAdapter,
}

#[async_trait::async_trait]
impl TransactionBackend for MySqlTransactionBackend {
    async fn begin(&mut self) -> Result<(), TransactionError> {
        <MySql as sqlx::Database>::TransactionManager::begin(&mut self.connection, None)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
        self.adapter
            .execute_connection(&mut self.connection, sql)
            .await
            .map_err(Into::into)
    }
    async fn commit(&mut self) -> Result<(), TransactionError> {
        <MySql as sqlx::Database>::TransactionManager::commit(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn rollback(&mut self) -> Result<(), TransactionError> {
        <MySql as sqlx::Database>::TransactionManager::rollback(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn cancel(&mut self) -> Result<(), TransactionError> {
        let sql = format!("KILL QUERY {}", self.connection_id);
        sqlx::query(AssertSqlSafe(sql))
            .execute(&self.control)
            .await
            .map_err(|error| TransactionError(error.to_string()))?;
        Ok(())
    }
    fn depth(&self) -> usize {
        <MySql as sqlx::Database>::TransactionManager::get_transaction_depth(&self.connection)
    }
    fn force_close(self) -> futures_util::future::BoxFuture<'static, Result<(), TransactionError>> {
        Box::pin(async move {
            // SQLx 0.9 has no close_hard; detaching before closing is the safe equivalent.
            let connection = self.connection.detach();
            connection
                .close()
                .await
                .map_err(|error| TransactionError(error.to_string()))
        })
    }
}

async fn append_indexes(
    pool: &MySqlPool,
    connection_id: Uuid,
    parents: &HashMap<(String, String), CatalogId>,
    nodes: &mut Vec<CatalogNode>,
) -> Result<(), DatabaseError> {
    for row in sqlx::query(CATALOG_INDEXES_SQL)
        .fetch_all(pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
    {
        let schema: String = row.try_get("table_schema").map_err(decode_error)?;
        let table: String = row.try_get("table_name").map_err(decode_error)?;
        let name: String = row.try_get("index_name").map_err(decode_error)?;
        let non_unique: i64 = row.try_get("non_unique").map_err(decode_error)?;
        let sequence: i64 = row.try_get("seq_in_index").map_err(decode_error)?;
        let column: Option<String> = row.try_get("column_name").map_err(decode_error)?;
        if let Some(parent) = parents.get(&(schema, table)) {
            let mut path = parent.native_path.clone();
            path.push(name.clone());
            path.push(sequence.to_string());
            nodes.push(CatalogNode::new(
                CatalogId::new(connection_id, CatalogKind::Index, path),
                Some(parent.clone()),
                name,
                "index",
                Some(format!(
                    "{} column {}",
                    if non_unique == 0 { "UNIQUE" } else { "INDEX" },
                    column.unwrap_or_else(|| "expression".to_owned())
                )),
                false,
            ));
        }
    }
    Ok(())
}

async fn append_foreign_keys(
    pool: &MySqlPool,
    connection_id: Uuid,
    parents: &HashMap<(String, String), CatalogId>,
    nodes: &mut Vec<CatalogNode>,
) -> Result<(), DatabaseError> {
    for row in sqlx::query(CATALOG_FOREIGN_KEYS_SQL)
        .fetch_all(pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
    {
        let schema: String = row.try_get("table_schema").map_err(decode_error)?;
        let table: String = row.try_get("table_name").map_err(decode_error)?;
        let name: String = row.try_get("constraint_name").map_err(decode_error)?;
        let column: String = row.try_get("column_name").map_err(decode_error)?;
        let target_schema: String = row.try_get("target_schema").map_err(decode_error)?;
        let target_table: String = row.try_get("target_table").map_err(decode_error)?;
        let target_column: String = row.try_get("target_column").map_err(decode_error)?;
        if let Some(parent) = parents.get(&(schema, table)) {
            let mut path = parent.native_path.clone();
            path.push(name.clone());
            path.push(column.clone());
            nodes.push(CatalogNode::new(
                CatalogId::new(connection_id, CatalogKind::ForeignKey, path),
                Some(parent.clone()),
                name,
                "foreign_key",
                Some(format!(
                    "{column} -> {target_schema}.{target_table}.{target_column}"
                )),
                false,
            ));
        }
    }
    Ok(())
}

pub fn quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn mysql_ssl_mode(mode: SslMode) -> MySqlSslMode {
    match mode {
        SslMode::Disable => MySqlSslMode::Disabled,
        SslMode::Prefer => MySqlSslMode::Preferred,
        SslMode::Require => MySqlSslMode::Required,
        SslMode::VerifyCa => MySqlSslMode::VerifyCa,
        SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
    }
}

fn columns(row: &MySqlRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}

fn decode_row(row: &MySqlRow) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| decode_cell(row, index))
        .collect()
}

fn decode_cell(row: &MySqlRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return unsupported("unknown", "decode error");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_ascii_uppercase();
    if type_name.ends_with(" UNSIGNED") || matches!(type_name.as_str(), "YEAR" | "BIT") {
        return row
            .try_get_unchecked::<u64, _>(index)
            .map(CellValue::Unsigned)
            .unwrap_or_else(|error| unsupported(&type_name, &error.to_string()));
    }
    let decoded = match type_name.as_str() {
        "BOOLEAN" => row
            .try_get_unchecked::<bool, _>(index)
            .map(CellValue::Boolean),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => row
            .try_get_unchecked::<i64, _>(index)
            .map(CellValue::Integer),
        "FLOAT" | "DOUBLE" => row.try_get_unchecked::<f64, _>(index).map(CellValue::Float),
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => row
            .try_get_unchecked::<Vec<u8>, _>(index)
            .map(CellValue::Bytes),
        "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" => row
            .try_get_unchecked::<String, _>(index)
            .map(CellValue::Text),
        _ => return fallback_mysql(row, index, &type_name),
    };
    decoded.unwrap_or_else(|error| unsupported(&type_name, &error.to_string()))
}

fn fallback_mysql(row: &MySqlRow, index: usize, type_name: &str) -> CellValue {
    if let Ok(value) = row.try_get_unchecked::<String, _>(index) {
        CellValue::Text(value)
    } else if let Ok(value) = row.try_get_unchecked::<Vec<u8>, _>(index) {
        CellValue::Bytes(value)
    } else {
        unsupported(type_name, "unsupported MySQL value")
    }
}

fn unsupported(type_name: &str, preview: &str) -> CellValue {
    CellValue::Unsupported {
        type_name: type_name.to_owned(),
        preview: preview.to_owned(),
    }
}

fn decode_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Internal)
}

#[cfg(test)]
mod tests {
    use super::PROBE_SQL;

    #[test]
    fn probe_query_avoids_the_reserved_database_alias() {
        assert!(PROBE_SQL.contains("AS current_database"));
        assert!(!PROBE_SQL.contains("AS database"));
    }
}
