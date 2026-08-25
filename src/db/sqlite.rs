use std::time::Instant;

use futures_util::TryStreamExt;
use sqlx::{
    AssertSqlSafe, Column, Either, Row, SqlitePool, TypeInfo, ValueRef,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};
use uuid::Uuid;

use crate::profile::{ConnectionProfile, DatabaseKind};

use super::{
    DatabaseError, ErrorCategory, ServerInfo,
    catalog::{CatalogId, CatalogKind, CatalogNode},
    query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
    value::CellValue,
};

#[derive(Clone, Debug)]
pub struct SqliteAdapter {
    pool: SqlitePool,
    connection_id: Uuid,
    database: String,
}

impl SqliteAdapter {
    pub async fn connect(profile: &ConnectionProfile) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::Sqlite {
            return Err(DatabaseError::configuration("profile is not SQLite"));
        }

        let in_memory = profile.database.as_deref() == Some(":memory:");
        let mut options = SqliteConnectOptions::new()
            .foreign_keys(true)
            .read_only(profile.read_only)
            .create_if_missing(!profile.read_only);
        if in_memory {
            options = options.in_memory(true);
        } else if let Some(path) = &profile.sqlite_path {
            options = options.filename(path);
        } else {
            return Err(DatabaseError::configuration(
                "SQLite profile has no database path",
            ));
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(if in_memory { 1 } else { 4 })
            .connect_with(options)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;

        Ok(Self {
            pool,
            connection_id: profile.id,
            database: profile
                .database
                .clone()
                .unwrap_or_else(|| ":memory:".to_owned()),
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let version = sqlx::query_scalar::<_, String>("SELECT sqlite_version()")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(ServerInfo {
            kind: DatabaseKind::Sqlite,
            version,
            database: self.database.clone(),
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

        if let Some(result) = current {
            result_sets.push(result);
        }
        let total = started.elapsed();
        let execution = first_event_at.unwrap_or(total);
        let fetch = total.saturating_sub(execution);
        let row_count = result_sets.iter().map(|result| result.rows.len()).sum();

        Ok(QueryOutcome {
            result_sets,
            stats: QueryStats::new(execution, fetch, row_count),
        })
    }

    pub async fn load_catalog(&self) -> Result<Vec<CatalogNode>, DatabaseError> {
        let mut nodes = Vec::new();
        let database_id = CatalogId::new(
            self.connection_id,
            CatalogKind::Database,
            [self.database.clone()],
        );
        nodes.push(CatalogNode::new(
            database_id.clone(),
            None,
            self.database.clone(),
            "database",
            None,
            true,
        ));
        let schema_id = CatalogId::new(
            self.connection_id,
            CatalogKind::Schema,
            [self.database.clone(), "main".to_owned()],
        );
        nodes.push(CatalogNode::new(
            schema_id.clone(),
            Some(database_id),
            "main",
            "schema",
            None,
            true,
        ));

        let objects = sqlx::query(
            "SELECT type, name, tbl_name, sql FROM sqlite_schema \
             WHERE type IN ('table', 'view', 'trigger') \
             AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;

        let mut tables = Vec::new();
        for row in objects {
            let native_kind: String = row.try_get("type").map_err(decode_error)?;
            let name: String = row.try_get("name").map_err(decode_error)?;
            let table_name: String = row.try_get("tbl_name").map_err(decode_error)?;
            let ddl: Option<String> = row.try_get("sql").map_err(decode_error)?;
            let kind = match native_kind.as_str() {
                "table" => CatalogKind::Table,
                "view" => CatalogKind::View,
                "trigger" => CatalogKind::Trigger,
                _ => continue,
            };
            let parent_id = if kind == CatalogKind::Trigger {
                CatalogId::new(
                    self.connection_id,
                    CatalogKind::Table,
                    [self.database.clone(), "main".to_owned(), table_name],
                )
            } else {
                schema_id.clone()
            };
            let id = CatalogId::new(
                self.connection_id,
                kind,
                [self.database.clone(), "main".to_owned(), name.clone()],
            );
            nodes.push(CatalogNode::new(
                id,
                Some(parent_id),
                name.clone(),
                native_kind,
                ddl,
                matches!(kind, CatalogKind::Table | CatalogKind::View),
            ));
            if matches!(kind, CatalogKind::Table | CatalogKind::View) {
                tables.push((name, kind));
            }
        }

        for (table, table_kind) in tables {
            let table_id = CatalogId::new(
                self.connection_id,
                table_kind,
                [self.database.clone(), "main".to_owned(), table.clone()],
            );
            self.load_columns(&table, &table_id, &mut nodes).await?;
            if table_kind == CatalogKind::Table {
                self.load_indexes(&table, &table_id, &mut nodes).await?;
                self.load_foreign_keys(&table, &table_id, &mut nodes)
                    .await?;
            }
        }

        Ok(nodes)
    }

    async fn load_columns(
        &self,
        table: &str,
        parent: &CatalogId,
        nodes: &mut Vec<CatalogNode>,
    ) -> Result<(), DatabaseError> {
        let rows = sqlx::query(
            "SELECT name, type, \"notnull\" AS not_null, pk, hidden \
             FROM pragma_table_xinfo(?) ORDER BY cid",
        )
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;

        for row in rows {
            let name: String = row.try_get("name").map_err(decode_error)?;
            let data_type: String = row.try_get("type").map_err(decode_error)?;
            let not_null: i64 = row.try_get("not_null").map_err(decode_error)?;
            let primary_key: i64 = row.try_get("pk").map_err(decode_error)?;
            let hidden: i64 = row.try_get("hidden").map_err(decode_error)?;
            let detail = format!(
                "{}{}{}{}",
                if data_type.is_empty() {
                    "ANY"
                } else {
                    &data_type
                },
                if not_null != 0 { " NOT NULL" } else { "" },
                if primary_key != 0 { " PRIMARY KEY" } else { "" },
                if hidden != 0 { " HIDDEN" } else { "" },
            );
            let mut path = parent.native_path.clone();
            path.push(name.clone());
            nodes.push(CatalogNode::new(
                CatalogId::new(self.connection_id, CatalogKind::Column, path),
                Some(parent.clone()),
                name,
                "column",
                Some(detail),
                false,
            ));
        }
        Ok(())
    }

    async fn load_indexes(
        &self,
        table: &str,
        parent: &CatalogId,
        nodes: &mut Vec<CatalogNode>,
    ) -> Result<(), DatabaseError> {
        let rows = sqlx::query(
            "SELECT name, \"unique\" AS is_unique, origin, partial \
             FROM pragma_index_list(?) ORDER BY seq",
        )
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        for row in rows {
            let name: String = row.try_get("name").map_err(decode_error)?;
            let unique: i64 = row.try_get("is_unique").map_err(decode_error)?;
            let origin: String = row.try_get("origin").map_err(decode_error)?;
            let partial: i64 = row.try_get("partial").map_err(decode_error)?;
            let detail = format!(
                "{}{}{}",
                if unique != 0 { "UNIQUE" } else { "INDEX" },
                if partial != 0 { " PARTIAL" } else { "" },
                if origin.is_empty() {
                    String::new()
                } else {
                    format!(" [{origin}]")
                }
            );
            let mut path = parent.native_path.clone();
            path.push(name.clone());
            nodes.push(CatalogNode::new(
                CatalogId::new(self.connection_id, CatalogKind::Index, path),
                Some(parent.clone()),
                name,
                "index",
                Some(detail),
                false,
            ));
        }
        Ok(())
    }

    async fn load_foreign_keys(
        &self,
        table: &str,
        parent: &CatalogId,
        nodes: &mut Vec<CatalogNode>,
    ) -> Result<(), DatabaseError> {
        let rows = sqlx::query(
            "SELECT id, seq, \"table\" AS target_table, \"from\" AS source_column, \
             \"to\" AS target_column FROM pragma_foreign_key_list(?) ORDER BY id, seq",
        )
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        for row in rows {
            let id: i64 = row.try_get("id").map_err(decode_error)?;
            let seq: i64 = row.try_get("seq").map_err(decode_error)?;
            let target_table: String = row.try_get("target_table").map_err(decode_error)?;
            let source_column: String = row.try_get("source_column").map_err(decode_error)?;
            let target_column: Option<String> =
                row.try_get("target_column").map_err(decode_error)?;
            let name = format!("fk_{table}_{id}_{seq}");
            let mut path = parent.native_path.clone();
            path.push(name.clone());
            nodes.push(CatalogNode::new(
                CatalogId::new(self.connection_id, CatalogKind::ForeignKey, path),
                Some(parent.clone()),
                name,
                "foreign_key",
                Some(format!(
                    "{source_column} -> {target_table}.{}",
                    target_column.unwrap_or_else(|| "rowid".to_owned())
                )),
                false,
            ));
        }
        Ok(())
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        _schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let object_type = match kind {
            CatalogKind::Table => "table",
            CatalogKind::View => "view",
            CatalogKind::Index => "index",
            CatalogKind::Trigger => "trigger",
            _ => return Ok(None),
        };
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT sql FROM sqlite_schema WHERE type = ? AND name = ?",
        )
        .bind(object_type)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map(|value| value.flatten())
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))
    }

    pub fn quote_identifier(&self, value: &str) -> String {
        format!("\"{}\"", value.replace('"', "\"\""))
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

fn columns(row: &SqliteRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}

fn decode_row(row: &SqliteRow) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| decode_cell(row, index))
        .collect()
}

fn decode_cell(row: &SqliteRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::Unsupported {
            type_name: "unknown".to_owned(),
            preview: "decode error".to_owned(),
        };
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_ascii_uppercase();
    let decoded = match type_name.as_str() {
        "BOOLEAN" | "BOOL" => row.try_get::<bool, _>(index).map(CellValue::Boolean),
        "INTEGER" | "INT" | "INT64" => row.try_get::<i64, _>(index).map(CellValue::Integer),
        "REAL" | "FLOAT" | "DOUBLE" => row.try_get::<f64, _>(index).map(CellValue::Float),
        "TEXT" => row.try_get::<String, _>(index).map(CellValue::Text),
        "BLOB" => row.try_get::<Vec<u8>, _>(index).map(CellValue::Bytes),
        _ => row.try_get::<String, _>(index).map(CellValue::Text),
    };
    decoded.unwrap_or_else(|error| CellValue::Unsupported {
        type_name,
        preview: error.to_string(),
    })
}

fn decode_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Internal)
}
