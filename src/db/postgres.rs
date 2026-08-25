use std::{collections::HashMap, time::Instant};

use futures_util::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    AssertSqlSafe, Column, Either, PgPool, Row, TypeInfo, ValueRef,
    postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode},
};
use uuid::Uuid;

use crate::profile::{ConnectionProfile, DatabaseKind, SslMode};

use super::{
    DatabaseError, ErrorCategory, ServerInfo,
    catalog::{CatalogId, CatalogKind, CatalogNode},
    query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
    value::CellValue,
};

pub const CATALOG_TABLES_SQL: &str = r#"
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_catalog = current_database()
  AND table_schema <> 'information_schema'
  AND table_schema NOT LIKE 'pg_%'
ORDER BY table_schema, table_name
"#;

const CATALOG_SCHEMAS_SQL: &str = r#"
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name <> 'information_schema'
  AND schema_name NOT LIKE 'pg_%'
ORDER BY schema_name
"#;

const CATALOG_COLUMNS_SQL: &str = r#"
SELECT table_schema, table_name, column_name, data_type, is_nullable
FROM information_schema.columns
WHERE table_catalog = current_database()
  AND table_schema <> 'information_schema'
  AND table_schema NOT LIKE 'pg_%'
ORDER BY table_schema, table_name, ordinal_position
"#;

pub const CATALOG_INDEXES_SQL: &str = r#"
SELECT schemaname AS table_schema, tablename AS table_name, indexname, indexdef
FROM pg_indexes
WHERE schemaname <> 'information_schema'
  AND schemaname NOT LIKE 'pg_%'
ORDER BY schemaname, tablename, indexname
"#;

const CATALOG_FOREIGN_KEYS_SQL: &str = r#"
SELECT tc.table_schema, tc.table_name, tc.constraint_name,
       kcu.column_name, ccu.table_schema AS target_schema,
       ccu.table_name AS target_table, ccu.column_name AS target_column
FROM information_schema.table_constraints tc
JOIN information_schema.key_column_usage kcu
  ON tc.constraint_catalog = kcu.constraint_catalog
 AND tc.constraint_schema = kcu.constraint_schema
 AND tc.constraint_name = kcu.constraint_name
JOIN information_schema.constraint_column_usage ccu
  ON tc.constraint_catalog = ccu.constraint_catalog
 AND tc.constraint_schema = ccu.constraint_schema
 AND tc.constraint_name = ccu.constraint_name
WHERE tc.constraint_type = 'FOREIGN KEY'
  AND tc.table_catalog = current_database()
ORDER BY tc.table_schema, tc.table_name, tc.constraint_name, kcu.ordinal_position
"#;

#[derive(Clone, Debug)]
pub struct PostgresAdapter {
    pool: PgPool,
    connection_id: Uuid,
}

impl PostgresAdapter {
    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::Postgres {
            return Err(DatabaseError::configuration("profile is not PostgreSQL"));
        }
        let host = profile
            .host
            .as_deref()
            .ok_or_else(|| DatabaseError::configuration("PostgreSQL profile has no host"))?;
        let mut options = PgConnectOptions::new()
            .host(host)
            .port(profile.port.unwrap_or(5432))
            .ssl_mode(pg_ssl_mode(profile.ssl_mode))
            .application_name("lazydb");
        if let Some(user) = &profile.user {
            options = options.username(user);
        }
        if let Some(database) = &profile.database {
            options = options.database(database);
        }
        if let Some(password) = password {
            options = options.password(password.expose_secret());
        }
        if profile.read_only {
            options = options.options([("default_transaction_read_only", "on")]);
        }

        let pool = PgPoolOptions::new()
            .max_connections(6)
            .connect_with(options)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(Self {
            pool,
            connection_id: profile.id,
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let row = sqlx::query("SELECT version() AS version, current_database() AS database")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(ServerInfo {
            kind: DatabaseKind::Postgres,
            version: row.try_get("version").map_err(decode_error)?,
            database: row.try_get("database").map_err(decode_error)?,
        })
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        let started = Instant::now();
        let mut first_event_at = None;
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&self.pool);
        let mut result_sets = Vec::new();
        let mut current: Option<ResultSet> = None;

        while let Some(event) = stream
            .try_next()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        {
            first_event_at.get_or_insert_with(|| started.elapsed());
            match event {
                Either::Right(row) => {
                    let result = current.get_or_insert_with(|| ResultSet {
                        columns: columns(&row),
                        rows: Vec::new(),
                        affected_rows: 0,
                    });
                    result.rows.push(decode_row(&row));
                }
                Either::Left(done) => {
                    let mut result = current.take().unwrap_or_default();
                    result.affected_rows = done.rows_affected();
                    result_sets.push(result);
                }
            }
        }
        let total = started.elapsed();
        let execution = first_event_at.unwrap_or(total);
        let row_count = result_sets.iter().map(|result| result.rows.len()).sum();
        Ok(QueryOutcome {
            result_sets,
            stats: QueryStats::new(execution, total.saturating_sub(execution), row_count),
        })
    }

    pub async fn load_catalog(&self) -> Result<Vec<CatalogNode>, DatabaseError> {
        let database = sqlx::query_scalar::<_, String>("SELECT current_database()")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let database_id = CatalogId::new(
            self.connection_id,
            CatalogKind::Database,
            [database.clone()],
        );
        let mut nodes = vec![CatalogNode::new(
            database_id.clone(),
            None,
            database.clone(),
            "database",
            None,
            true,
        )];
        let mut schema_ids = HashMap::new();
        for row in sqlx::query(CATALOG_SCHEMAS_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        {
            let schema: String = row.try_get("schema_name").map_err(decode_error)?;
            let id = CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [database.clone(), schema.clone()],
            );
            nodes.push(CatalogNode::new(
                id.clone(),
                Some(database_id.clone()),
                schema.clone(),
                "schema",
                None,
                true,
            ));
            schema_ids.insert(schema, id);
        }

        let mut object_ids = HashMap::new();
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
                schema_ids.get(&schema).cloned(),
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
            let data_type: String = row.try_get("data_type").map_err(decode_error)?;
            let nullable: String = row.try_get("is_nullable").map_err(decode_error)?;
            if let Some(parent) = object_ids.get(&(schema, table)) {
                let mut path = parent.native_path.clone();
                path.push(name.clone());
                nodes.push(CatalogNode::new(
                    CatalogId::new(self.connection_id, CatalogKind::Column, path),
                    Some(parent.clone()),
                    name,
                    "column",
                    Some(format!(
                        "{data_type}{}",
                        if nullable == "NO" { " NOT NULL" } else { "" }
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
        if kind != CatalogKind::View {
            return Ok(None);
        }
        sqlx::query_scalar::<_, String>(
            "SELECT 'CREATE OR REPLACE VIEW ' || quote_ident($1) || '.' || quote_ident($2) || \
             ' AS\n' || pg_get_viewdef((quote_ident($1) || '.' || quote_ident($2))::regclass, true)",
        )
        .bind(schema)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

async fn append_indexes(
    pool: &PgPool,
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
        let name: String = row.try_get("indexname").map_err(decode_error)?;
        let definition: String = row.try_get("indexdef").map_err(decode_error)?;
        if let Some(parent) = parents.get(&(schema, table)) {
            let mut path = parent.native_path.clone();
            path.push(name.clone());
            nodes.push(CatalogNode::new(
                CatalogId::new(connection_id, CatalogKind::Index, path),
                Some(parent.clone()),
                name,
                "index",
                Some(definition),
                false,
            ));
        }
    }
    Ok(())
}

async fn append_foreign_keys(
    pool: &PgPool,
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
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn pg_ssl_mode(mode: SslMode) -> PgSslMode {
    match mode {
        SslMode::Disable => PgSslMode::Disable,
        SslMode::Prefer => PgSslMode::Prefer,
        SslMode::Require => PgSslMode::Require,
        SslMode::VerifyCa => PgSslMode::VerifyCa,
        SslMode::VerifyFull => PgSslMode::VerifyFull,
    }
}

fn columns(row: &PgRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}

fn decode_row(row: &PgRow) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| decode_cell(row, index))
        .collect()
}

fn decode_cell(row: &PgRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return unsupported("unknown", "decode error");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_ascii_uppercase();
    let decoded = match type_name.as_str() {
        "BOOL" => row.try_get::<bool, _>(index).map(CellValue::Boolean),
        "INT2" => row
            .try_get::<i16, _>(index)
            .map(|value| CellValue::Integer(i64::from(value))),
        "INT4" => row
            .try_get::<i32, _>(index)
            .map(|value| CellValue::Integer(i64::from(value))),
        "INT8" => row.try_get::<i64, _>(index).map(CellValue::Integer),
        "FLOAT4" => row
            .try_get::<f32, _>(index)
            .map(|value| CellValue::Float(f64::from(value))),
        "FLOAT8" => row.try_get::<f64, _>(index).map(CellValue::Float),
        "BYTEA" => row.try_get::<Vec<u8>, _>(index).map(CellValue::Bytes),
        "TEXT" | "VARCHAR" | "CHAR" | "NAME" | "UNKNOWN" => {
            row.try_get::<String, _>(index).map(CellValue::Text)
        }
        _ => return fallback_pg(row, index, &type_name),
    };
    decoded.unwrap_or_else(|error| unsupported(&type_name, &error.to_string()))
}

fn fallback_pg(row: &PgRow, index: usize, type_name: &str) -> CellValue {
    if let Ok(value) = row.try_get_unchecked::<String, _>(index) {
        CellValue::Text(value)
    } else if let Ok(value) = row.try_get_unchecked::<Vec<u8>, _>(index) {
        CellValue::Bytes(value)
    } else {
        unsupported(type_name, "unsupported PostgreSQL value")
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
