#![allow(clippy::collapsible_if)]
#![allow(clippy::explicit_auto_deref)]
#![allow(clippy::format_in_format_args)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::obfuscated_if_else)]
#![allow(clippy::redundant_closure)]

use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    str::FromStr,
    time::{Duration, Instant},
};

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use sqlx::{
    AssertSqlSafe, Column, Connection, Executor, PgPool, Row, SqlSafeStr, Statement, TypeInfo,
    pool::PoolConnection,
    postgres::{PgConnection, PgPoolOptions, PgRow, Postgres},
    types::{BigDecimal, Json, ipnet::IpNet},
};
use sqlx_core::transaction::TransactionManager;
use uuid::Uuid;

use crate::model::dashboard::MetricKey;
use crate::{
    identity::ConnectionIdentity,
    model::catalog_editor::CatalogDraft,
    model::execution_target::ExecutionTarget,
    profile::{CatalogScope, CatalogSelection, ConnectionProfile, DatabaseKind},
    security::sanitize_terminal_text,
};

use super::pgwire::decode::{decode_row, parse_pg_interval};
#[cfg(test)]
use super::pgwire::decode::{format_pg_array, format_pg_interval};

use super::transaction::{
    RelationMutationCategory, RelationMutationDiagnostic, TransactionBackend, TransactionError,
};
use super::{
    DatabaseError, ErrorCategory, ServerInfo,
    catalog::{
        CatalogCapabilities, CatalogCount, CatalogCursor, CatalogDiscovery, CatalogEntry,
        CatalogGroupSummary, CatalogId, CatalogKind, CatalogMetadata, CatalogPage, CatalogRequest,
        CatalogRequestKey, CatalogSearchHit, CatalogSearchPage, CatalogSearchRequest,
        CatalogTarget, CatalogValidationError, ColumnMetadata, ColumnMetadataCapabilities,
        ConstraintMembership, ConstraintMetadata, DdlProvenance, DiscoveredDatabase, IndexMetadata,
        NamespaceModel, ObjectGroup, OptionalMetadata, QualifiedName, RelationDdl,
        finalize_keyset_page,
    },
    catalog_drop::{
        CatalogDropError, CatalogDropExecutionTarget, CatalogDropPlan, CatalogDropRequest,
    },
    catalog_mutation::{
        CatalogMutationAnchor, CatalogMutationAvailability, CatalogMutationCapabilities,
        CatalogMutationError, CatalogMutationExecutionMode, CatalogMutationMode,
        CatalogMutationOption, CatalogMutationPlan, CatalogMutationRequest,
        CatalogObjectDefinition, CatalogObjectDefinitionRequest, CatalogObjectType,
        CatalogOwnerChoice, CatalogOwnerContext, CatalogOwnerContextRequest, CatalogSelectionHint,
        ColumnDefinition, ConstraintDefinition, ConstraintDefinitionKind, DatabaseDefinition,
        IndexColumnDefinition, IndexDefinition, MaterializedViewDefinition, RoleDefinition,
        SchemaDefinition, TableDefinition, ViewDefinition, ViewMutationCapabilities,
        ViewMutationOptionAvailability, ViewOption,
    },
    ddl::{DdlSection, assemble_ddl},
    mutation::{InputValue, MutationResult, RelationMutation, RelationMutationRequest},
    principal::{
        PrincipalCoverage, PrincipalDdl, PrincipalDetails, PrincipalEntry, PrincipalId,
        PrincipalKind, PrincipalMembership, PrincipalMutation, PrincipalMutationPlan,
        PrincipalMutationTarget, PrincipalPage, PrincipalPermission, PrincipalPermissionSource,
        PrincipalReadTarget, PrincipalScope,
    },
    query::{ColumnMeta, QueryBudget, QueryOutcome, RELATION_PREVIEW_LIMIT, ResultSet},
    value::CellValue,
};

pub const CATALOG_TABLES_SQL: &str = r#"
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_catalog = current_database()
  AND table_schema <> 'information_schema'
  AND table_schema NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY table_schema, table_name
"#;

pub const CATALOG_SCHEMAS_SQL: &str = r#"
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name <> 'information_schema'
  AND schema_name NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY schema_name COLLATE "C"
"#;

pub const CATALOG_COLUMNS_SQL: &str = r#"
SELECT table_schema, table_name, column_name, data_type, is_nullable
FROM information_schema.columns
WHERE table_catalog = current_database()
  AND table_schema <> 'information_schema'
  AND table_schema NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY table_schema, table_name, ordinal_position
"#;

pub const CATALOG_ROUTINES_SQL: &str = r#"
SELECT n.nspname AS routine_schema, p.proname AS routine_name,
       p.prokind::text AS prokind,
       pg_get_function_identity_arguments(p.oid) AS arguments,
       pg_get_function_result(p.oid) AS result
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname <> 'information_schema'
  AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY n.nspname, p.proname, p.oid
"#;

pub const CATALOG_INDEXES_SQL: &str = r#"
SELECT schemaname AS table_schema, tablename AS table_name, indexname, indexdef
FROM pg_indexes
WHERE schemaname <> 'information_schema'
  AND schemaname NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY schemaname, tablename, indexname
"#;

pub const CATALOG_CONSTRAINT_DEFINITION_SQL: &str = r#"
SELECT con.conname, con.contype::text, con.conkey, con.confkey,
       con.confmatchtype::text, con.confupdtype::text, con.confdeltype::text,
       con.condeferrable, con.condeferred, con.convalidated, con.connoinherit,
       pg_get_expr(con.conbin, con.conrelid, true)
FROM pg_constraint con
WHERE con.conrelid = $1::oid AND con.oid = $2::oid
"#;

pub const SEARCH_CATALOG_SQL: &str = r#"
WITH candidates AS (
    SELECT 'database'::text AS kind, current_database() AS database_name,
           NULL::text AS schema_name, current_database() AS object_name,
           NULL::bigint AS object_oid, NULL::text AS relation_kind,
           NULL::text AS relation_name, NULL::bigint AS relation_oid,
           shobj_description(d.oid, 'pg_database') AS comment, NULL::text AS relation_comment,
           shobj_description(d.oid, 'pg_database') AS database_comment,
           NULL::text AS schema_comment,
           current_database() AS qualified_path
    FROM pg_database d
    WHERE d.datname = current_database()
    UNION ALL
    SELECT 'schema', current_database(), n.nspname, n.nspname, n.oid::bigint,
           NULL, NULL, NULL, obj_description(n.oid, 'pg_namespace'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname
    FROM pg_namespace n
    WHERE n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'table'
               WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized_view' ELSE 'sequence' END,
           current_database(), n.nspname, c.relname, c.oid::bigint,
           NULL, NULL, NULL, obj_description(c.oid, 'pg_class'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'S')
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT CASE p.prokind WHEN 'f' THEN 'function' ELSE 'procedure' END,
           current_database(), n.nspname, p.proname, p.oid::bigint,
           NULL, NULL, NULL, obj_description(p.oid, 'pg_proc'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || p.proname
    FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
    WHERE p.prokind IN ('f', 'p')
       AND (p.prokind <> 'f' OR NOT EXISTS (SELECT 1 FROM pg_trigger tr WHERE tr.tgfoid = p.oid))
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT 'type', current_database(), n.nspname, t.typname, t.oid::bigint,
           NULL, NULL, NULL, obj_description(t.oid, 'pg_type'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || t.typname
    FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
    WHERE t.typtype IN ('e', 'd') AND t.typisdefined AND t.typelem = 0
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT 'column', current_database(), n.nspname, a.attname, a.attnum::bigint,
           c.relkind::text, c.relname, c.oid::bigint, col_description(c.oid, a.attnum),
           obj_description(c.oid, 'pg_class'),
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname || '.' || a.attname
    FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm') AND a.attnum > 0 AND NOT a.attisdropped
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT 'index', current_database(), n.nspname, ic.relname, idx.indexrelid::bigint,
           c.relkind::text, c.relname, c.oid::bigint, obj_description(ic.oid, 'pg_class'),
           obj_description(c.oid, 'pg_class'),
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname || '.' || ic.relname
    FROM pg_index idx JOIN pg_class c ON c.oid = idx.indrelid
         JOIN pg_class ic ON ic.oid = idx.indexrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm')
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT CASE con.contype WHEN 'p' THEN 'primary_key' WHEN 'u' THEN 'unique_constraint'
               WHEN 'f' THEN 'foreign_key' ELSE 'check_constraint' END,
           current_database(), n.nspname, con.conname, con.oid::bigint,
           c.relkind::text, c.relname, c.oid::bigint, obj_description(con.oid, 'pg_constraint'),
           obj_description(c.oid, 'pg_class'),
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname || '.' || con.conname
    FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE con.contype IN ('p', 'u', 'f', 'c') AND c.relkind IN ('r', 'p', 'v', 'm')
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
), normalized AS (
    SELECT *, CASE WHEN $5 THEN regexp_replace(lower(object_name), '[^[:alnum:]]', '', 'g')
                   ELSE lower(object_name) END AS search_name,
              CASE WHEN $5 THEN regexp_replace(lower(qualified_path), '[^[:alnum:]]', '', 'g')
                   ELSE lower(qualified_path) END AS search_path
    FROM candidates
), ranked AS (
    SELECT *, CASE
        WHEN search_name = $1 OR search_path = $1 OR right(search_path, length($1)) = $1 THEN 0
        WHEN strpos(search_name, $1) = 1 OR strpos(search_path, $1) = 1 THEN 1
        WHEN strpos(search_name, $1) > 0 OR strpos(search_path, $1) > 0 THEN 2
        ELSE 3 END AS relevance
    FROM normalized
    WHERE $3
      AND (strpos(search_name, $1) > 0 OR strpos(search_path, $1) > 0)
      AND ($6 OR kind IN ('table', 'view', 'materialized_view'))
)
SELECT kind, database_name, schema_name, object_name, object_oid,
       relation_kind, relation_name, relation_oid, comment, relation_comment,
       database_comment, schema_comment
FROM ranked
ORDER BY relevance, lower(qualified_path) COLLATE "C", qualified_path COLLATE "C",
         kind COLLATE "C", object_oid
LIMIT $4
"#;

#[derive(Clone, Debug)]
pub struct PostgresAdapter {
    pool: PgPool,
    connection_id: Uuid,
    catalog_scope: CatalogScope,
    server_version_num: i32,
}

struct PgIndexInfo {
    oid: i64,
    name: String,
    columns: Vec<String>,
    unique: bool,
    comment: Option<String>,
}

struct PgConstraintInfo {
    oid: i64,
    name: String,
    kind: CatalogKind,
    columns: Vec<String>,
    referenced_schema: Option<String>,
    referenced_relation: Option<String>,
    referenced_columns: Vec<String>,
    check_expression: Option<String>,
    comment: Option<String>,
}

#[derive(Debug)]
struct PgDdlColumn {
    name: String,
    native_type: String,
    default_expression: Option<String>,
    not_null: bool,
    identity_kind: String,
    generated_kind: String,
}

#[derive(Debug)]
struct PgDdlRelation {
    schema: String,
    name: String,
    relation_kind: String,
    persistence: String,
    view_definition: Option<String>,
    materialized_populated: bool,
    partition_key: Option<String>,
    partition_parent: Option<(String, String)>,
    partition_bound: Option<String>,
    columns: Vec<PgDdlColumn>,
    constraints: Vec<(String, String)>,
    relation_comment: Option<String>,
    column_comments: Vec<(String, String)>,
    indexes: Vec<(String, String)>,
    triggers: Vec<(String, String)>,
}

struct PgSearchCandidate {
    kind: CatalogKind,
    database: String,
    schema: Option<String>,
    name: String,
    oid: Option<i64>,
    relation_kind: Option<String>,
    relation_name: Option<String>,
    relation_oid: Option<i64>,
    comment: Option<String>,
    relation_comment: Option<String>,
    database_comment: Option<String>,
    schema_comment: Option<String>,
}

impl PostgresAdapter {
    pub fn plan_principal_mutation(
        &self,
        principal: &PrincipalEntry,
        connection: ConnectionIdentity,
        database: Option<&str>,
        mutation: PrincipalMutation,
    ) -> Result<PrincipalMutationPlan, DatabaseError> {
        if principal.id.profile_id != self.connection_id || principal.name.trim().is_empty() {
            return Err(DatabaseError::configuration(
                "invalid principal mutation target",
            ));
        }
        let grantee = quote_identifier(&principal.name);
        let sql = match mutation {
            PrincipalMutation::Grant {
                target,
                privilege,
                grant_option,
            } => {
                let privilege = validate_privilege(&privilege)?;
                let (target, column) = mutation_target_sql(target, database)?;
                let privilege = column.map_or(privilege.clone(), |column| {
                    format!("{privilege} ({})", quote_identifier(&column))
                });
                format!(
                    "GRANT {privilege} ON {target} TO {grantee}{};",
                    if grant_option {
                        " WITH GRANT OPTION"
                    } else {
                        ""
                    }
                )
            }
            PrincipalMutation::Revoke {
                target,
                privilege,
                grant_option,
            } => {
                let privilege = validate_privilege(&privilege)?;
                let (target, column) = mutation_target_sql(target, database)?;
                let privilege = column.map_or(privilege.clone(), |column| {
                    format!("{privilege} ({})", quote_identifier(&column))
                });
                if grant_option {
                    format!("REVOKE GRANT OPTION FOR {privilege} ON {target} FROM {grantee};")
                } else {
                    format!("REVOKE {privilege} ON {target} FROM {grantee};")
                }
            }
            PrincipalMutation::GrantRole { role, admin_option } => {
                format!(
                    "GRANT {} TO {grantee}{};",
                    quote_identifier(&role),
                    if admin_option {
                        " WITH ADMIN OPTION"
                    } else {
                        ""
                    }
                )
            }
            PrincipalMutation::RevokeRole { role } => {
                format!("REVOKE {} FROM {grantee};", quote_identifier(&role))
            }
        };
        Ok(PrincipalMutationPlan {
            connection,
            principal: principal.clone(),
            database: database.map(str::to_owned),
            sql,
        })
    }

    pub async fn principal_details(
        &self,
        principal: &PrincipalEntry,
        target: &PrincipalReadTarget,
    ) -> Result<PrincipalDetails, DatabaseError> {
        if principal.id.profile_id != self.connection_id {
            return Err(DatabaseError::configuration(
                "principal does not belong to this connection",
            ));
        }
        if target.principal != principal.id {
            return Err(DatabaseError::configuration(
                "principal details target does not match the principal",
            ));
        }
        let name =
            sqlx::query_scalar::<_, String>("SELECT rolname FROM pg_roles WHERE oid = $1::oid")
                .bind(&principal.id.native_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(sql_error)?
                .ok_or_else(|| DatabaseError::configuration("principal no longer exists"))?;
        let mut permissions = sqlx::query(
            "SELECT table_schema, table_name, privilege_type, is_grantable
             FROM information_schema.role_table_grants
             WHERE grantee = $1
             ORDER BY table_schema, table_name, privilege_type",
        )
        .bind(&name)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok(PrincipalPermission {
                target: format!(
                    "{}.{}",
                    row.try_get::<String, _>("table_schema")?,
                    row.try_get::<String, _>("table_name")?
                ),
                mutation_target: Some(PrincipalMutationTarget::Relation {
                    schema: row.try_get("table_schema")?,
                    relation: row.try_get("table_name")?,
                }),
                privilege: row.try_get("privilege_type")?,
                source: "direct".to_owned(),
                grantable: row
                    .try_get::<String, _>("is_grantable")?
                    .eq_ignore_ascii_case("YES"),
                source_kind: PrincipalPermissionSource::Direct,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(decode_error)?;
        // PostgreSQL has no information_schema.role_schema_grants view. Schema
        // privileges are read from the native ACL below.
        let column_permissions = sqlx::query(
            "SELECT table_schema, table_name, column_name, privilege_type, is_grantable
             FROM information_schema.role_column_grants
             WHERE grantee = $1 ORDER BY table_schema, table_name, column_name, privilege_type",
        )
        .bind(&name)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok(PrincipalPermission {
                target: format!(
                    "{}.{}({})",
                    row.try_get::<String, _>("table_schema")?,
                    row.try_get::<String, _>("table_name")?,
                    row.try_get::<String, _>("column_name")?
                ),
                mutation_target: Some(PrincipalMutationTarget::Column {
                    schema: row.try_get("table_schema")?,
                    relation: row.try_get("table_name")?,
                    column: row.try_get("column_name")?,
                }),
                privilege: row.try_get("privilege_type")?,
                source: "direct".to_owned(),
                grantable: row
                    .try_get::<String, _>("is_grantable")?
                    .eq_ignore_ascii_case("YES"),
                source_kind: PrincipalPermissionSource::Direct,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(decode_error)?;
        permissions.extend(column_permissions);
        let routine_permissions = sqlx::query(
            "SELECT routine_schema, routine_name, privilege_type, is_grantable
             FROM information_schema.role_routine_grants
             WHERE grantee = $1 ORDER BY routine_schema, routine_name, privilege_type",
        )
        .bind(&name)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok(PrincipalPermission {
                target: format!(
                    "{}.{}()",
                    row.try_get::<String, _>("routine_schema")?,
                    row.try_get::<String, _>("routine_name")?
                ),
                mutation_target: None,
                privilege: row.try_get("privilege_type")?,
                source: "direct".to_owned(),
                grantable: row
                    .try_get::<String, _>("is_grantable")?
                    .eq_ignore_ascii_case("YES"),
                source_kind: PrincipalPermissionSource::Direct,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(decode_error)?;
        permissions.extend(routine_permissions);
        let usage_permissions = sqlx::query(
            "SELECT object_schema, object_name, object_type, privilege_type, is_grantable
             FROM information_schema.role_usage_grants
             WHERE grantee = $1 ORDER BY object_schema, object_name, privilege_type",
        )
        .bind(&name)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok(PrincipalPermission {
                target: format!(
                    "{}.{} ({})",
                    row.try_get::<String, _>("object_schema")?,
                    row.try_get::<String, _>("object_name")?,
                    row.try_get::<String, _>("object_type")?
                ),
                mutation_target: None,
                privilege: row.try_get("privilege_type")?,
                source: "direct".to_owned(),
                grantable: row
                    .try_get::<String, _>("is_grantable")?
                    .eq_ignore_ascii_case("YES"),
                source_kind: PrincipalPermissionSource::Direct,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(decode_error)?;
        permissions.extend(usage_permissions);
        let mut native_acl_unavailable = false;
        let acl_rows = sqlx::query(
            "SELECT n.nspname AS schema_name, c.relname AS relation_name,
                    (x.grantee = 0) AS is_public, x.privilege_type,
                    x.is_grantable, r.rolname AS grantee_name
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             CROSS JOIN LATERAL aclexplode(COALESCE(c.relacl, acldefault('r', c.relowner))) x
             LEFT JOIN pg_roles r ON r.oid = x.grantee
             WHERE (x.grantee = $1::oid OR x.grantee = 0)
             ORDER BY n.nspname, c.relname, x.privilege_type",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await;
        match acl_rows {
            Ok(rows) => {
                for row in rows {
                    let public = row.try_get::<bool, _>("is_public").map_err(decode_error)?;
                    permissions.push(PrincipalPermission {
                        target: format!(
                            "{}.{}",
                            row.try_get::<String, _>("schema_name")
                                .map_err(decode_error)?,
                            row.try_get::<String, _>("relation_name")
                                .map_err(decode_error)?
                        ),
                        mutation_target: Some(PrincipalMutationTarget::Relation {
                            schema: row.try_get("schema_name").map_err(decode_error)?,
                            relation: row.try_get("relation_name").map_err(decode_error)?,
                        }),
                        privilege: row.try_get("privilege_type").map_err(decode_error)?,
                        source: if public { "PUBLIC" } else { "native ACL" }.to_owned(),
                        grantable: row.try_get("is_grantable").map_err(decode_error)?,
                        source_kind: if public {
                            PrincipalPermissionSource::Public
                        } else {
                            PrincipalPermissionSource::Direct
                        },
                    });
                }
            }
            Err(error) => {
                let _ = error;
                native_acl_unavailable = true;
            }
        }
        let schema_acl_rows = sqlx::query(
            "SELECT n.nspname AS schema_name, (x.grantee = 0) AS is_public,
                    x.privilege_type, x.is_grantable
             FROM pg_namespace n
             CROSS JOIN LATERAL aclexplode(COALESCE(n.nspacl, acldefault('n', n.nspowner))) x
             WHERE (x.grantee = $1::oid OR x.grantee = 0)
               AND n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\'
             ORDER BY n.nspname, x.privilege_type",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await;
        match schema_acl_rows {
            Ok(rows) => {
                for row in rows {
                    let public = row.try_get::<bool, _>("is_public").map_err(decode_error)?;
                    permissions.push(PrincipalPermission {
                        target: format!(
                            "schema:{}",
                            row.try_get::<String, _>("schema_name")
                                .map_err(decode_error)?
                        ),
                        mutation_target: Some(PrincipalMutationTarget::Schema {
                            schema: row.try_get("schema_name").map_err(decode_error)?,
                        }),
                        privilege: row.try_get("privilege_type").map_err(decode_error)?,
                        source: if public { "PUBLIC" } else { "native ACL" }.into(),
                        grantable: row.try_get("is_grantable").map_err(decode_error)?,
                        source_kind: if public {
                            PrincipalPermissionSource::Public
                        } else {
                            PrincipalPermissionSource::Direct
                        },
                    });
                }
            }
            Err(_) => native_acl_unavailable = true,
        }
        let database_acl_rows = sqlx::query(
            "SELECT d.datname AS database_name, (x.grantee = 0) AS is_public,
                    x.privilege_type, x.is_grantable
             FROM pg_database d
             CROSS JOIN LATERAL aclexplode(COALESCE(d.datacl, acldefault('d', d.datdba))) x
             WHERE d.datname = current_database() AND (x.grantee = $1::oid OR x.grantee = 0)
             ORDER BY x.privilege_type",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await;
        match database_acl_rows {
            Ok(rows) => {
                for row in rows {
                    let public = row.try_get::<bool, _>("is_public").map_err(decode_error)?;
                    permissions.push(PrincipalPermission {
                        target: format!(
                            "database:{}",
                            row.try_get::<String, _>("database_name")
                                .map_err(decode_error)?
                        ),
                        mutation_target: Some(PrincipalMutationTarget::Database),
                        privilege: row.try_get("privilege_type").map_err(decode_error)?,
                        source: if public { "PUBLIC" } else { "native ACL" }.into(),
                        grantable: row.try_get("is_grantable").map_err(decode_error)?,
                        source_kind: if public {
                            PrincipalPermissionSource::Public
                        } else {
                            PrincipalPermissionSource::Direct
                        },
                    });
                }
            }
            Err(_) => native_acl_unavailable = true,
        }
        let default_acl_rows = sqlx::query(
            "SELECT n.nspname AS schema_name, (x.grantee = 0) AS is_public,
                    x.privilege_type, x.is_grantable
             FROM pg_default_acl d
             LEFT JOIN pg_namespace n ON n.oid = d.defaclnamespace
             CROSS JOIN LATERAL aclexplode(d.defaclacl) x
             WHERE d.defaclrole = $1::oid AND (x.grantee = $1::oid OR x.grantee = 0)
             ORDER BY n.nspname, x.privilege_type",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await;
        match default_acl_rows {
            Ok(rows) => {
                for row in rows {
                    let public = row.try_get::<bool, _>("is_public").map_err(decode_error)?;
                    permissions.push(PrincipalPermission {
                        target: format!(
                            "default:{}",
                            row.try_get::<Option<String>, _>("schema_name")
                                .map_err(decode_error)?
                                .unwrap_or_else(|| "database".into())
                        ),
                        mutation_target: None,
                        privilege: row.try_get("privilege_type").map_err(decode_error)?,
                        source: "default ACL".into(),
                        grantable: row.try_get("is_grantable").map_err(decode_error)?,
                        source_kind: if public {
                            PrincipalPermissionSource::Public
                        } else {
                            PrincipalPermissionSource::Default
                        },
                    });
                }
            }
            Err(_) => native_acl_unavailable = true,
        }
        let sequence_acl_rows = sqlx::query(
            "SELECT n.nspname AS schema_name, c.relname AS sequence_name,
                    (x.grantee = 0) AS is_public, x.privilege_type, x.is_grantable
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             CROSS JOIN LATERAL aclexplode(COALESCE(c.relacl, acldefault('S', c.relowner))) x
             WHERE c.relkind = 'S' AND (x.grantee = $1::oid OR x.grantee = 0)
             ORDER BY n.nspname, c.relname, x.privilege_type",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await;
        match sequence_acl_rows {
            Ok(rows) => {
                for row in rows {
                    let public = row.try_get::<bool, _>("is_public").map_err(decode_error)?;
                    permissions.push(PrincipalPermission {
                        target: format!(
                            "{}.{} (sequence)",
                            row.try_get::<String, _>("schema_name")
                                .map_err(decode_error)?,
                            row.try_get::<String, _>("sequence_name")
                                .map_err(decode_error)?
                        ),
                        mutation_target: None,
                        privilege: row.try_get("privilege_type").map_err(decode_error)?,
                        source: if public { "PUBLIC" } else { "native ACL" }.into(),
                        grantable: row.try_get("is_grantable").map_err(decode_error)?,
                        source_kind: if public {
                            PrincipalPermissionSource::Public
                        } else {
                            PrincipalPermissionSource::Direct
                        },
                    });
                }
            }
            Err(_) => native_acl_unavailable = true,
        }
        let routine_acl_rows = sqlx::query(
            "SELECT n.nspname AS schema_name, p.proname AS routine_name,
                    pg_get_function_identity_arguments(p.oid) AS arguments,
                    (x.grantee = 0) AS is_public, x.privilege_type, x.is_grantable
             FROM pg_proc p
             JOIN pg_namespace n ON n.oid = p.pronamespace
             CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl, acldefault('f', p.proowner))) x
             WHERE (x.grantee = $1::oid OR x.grantee = 0)
             ORDER BY n.nspname, p.proname, arguments, x.privilege_type",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await;
        match routine_acl_rows {
            Ok(rows) => {
                for row in rows {
                    let public = row.try_get::<bool, _>("is_public").map_err(decode_error)?;
                    permissions.push(PrincipalPermission {
                        target: format!(
                            "{}.{}({})",
                            row.try_get::<String, _>("schema_name")
                                .map_err(decode_error)?,
                            row.try_get::<String, _>("routine_name")
                                .map_err(decode_error)?,
                            row.try_get::<String, _>("arguments")
                                .map_err(decode_error)?
                        ),
                        mutation_target: None,
                        privilege: row.try_get("privilege_type").map_err(decode_error)?,
                        source: if public { "PUBLIC" } else { "native ACL" }.into(),
                        grantable: row.try_get("is_grantable").map_err(decode_error)?,
                        source_kind: if public {
                            PrincipalPermissionSource::Public
                        } else {
                            PrincipalPermissionSource::Direct
                        },
                    });
                }
            }
            Err(_) => native_acl_unavailable = true,
        }
        let owner_rows = sqlx::query(
            "SELECT n.nspname AS schema_name, c.relname AS relation_name, c.relkind::text AS relation_kind
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE c.relowner = $1::oid AND c.relkind IN ('r','p','v','m','S')
             UNION ALL
             SELECT n.nspname, NULL, 'schema' FROM pg_namespace n WHERE n.nspowner = $1::oid
             ORDER BY schema_name, relation_name NULLS FIRST",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await;
        match owner_rows {
            Ok(rows) => {
                for row in rows {
                    let relation = row
                        .try_get::<Option<String>, _>("relation_name")
                        .map_err(decode_error)?;
                    let kind: String = row.try_get("relation_kind").map_err(decode_error)?;
                    permissions.push(PrincipalPermission {
                        target: relation.map_or_else(
                            || {
                                format!(
                                    "schema:{}",
                                    row.try_get::<String, _>("schema_name").unwrap_or_default()
                                )
                            },
                            |name| {
                                format!(
                                    "{}.{}",
                                    row.try_get::<String, _>("schema_name").unwrap_or_default(),
                                    name
                                )
                            },
                        ),
                        mutation_target: None,
                        privilege: format!("OWNER ({kind})"),
                        source: "owner".into(),
                        grantable: true,
                        source_kind: PrincipalPermissionSource::Owner,
                    });
                }
            }
            Err(_) => native_acl_unavailable = true,
        }
        let member_of = sqlx::query(
            "SELECT granted.rolname AS role_name, member.rolname AS member_name,
                    m.admin_option
             FROM pg_auth_members m
             JOIN pg_roles member ON member.oid = m.member
             JOIN pg_roles granted ON granted.oid = m.roleid
             WHERE member.oid = $1::oid ORDER BY granted.rolname COLLATE \"C\"",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok(PrincipalMembership {
                role: row.try_get("role_name")?,
                member: row.try_get("member_name")?,
                admin_option: row.try_get("admin_option")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(decode_error)?;
        let members = sqlx::query(
            "SELECT granted.rolname AS role_name, member.rolname AS member_name,
                    m.admin_option
             FROM pg_auth_members m
             JOIN pg_roles member ON member.oid = m.member
             JOIN pg_roles granted ON granted.oid = m.roleid
             WHERE granted.oid = $1::oid ORDER BY member.rolname COLLATE \"C\"",
        )
        .bind(&principal.id.native_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok(PrincipalMembership {
                role: row.try_get("role_name")?,
                member: row.try_get("member_name")?,
                admin_option: row.try_get("admin_option")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(decode_error)?;
        Ok(PrincipalDetails {
            principal: PrincipalEntry {
                name,
                ..principal.clone()
            },
            database: target.database.clone(),
            permissions,
            member_of,
            members,
            permissions_coverage: if native_acl_unavailable {
                PrincipalCoverage::Unavailable(
                    "One or more PostgreSQL native ACL dictionaries were not readable".into(),
                )
            } else {
                PrincipalCoverage::Partial(
                    "Effective inherited privileges are not fully computed".into(),
                )
            },
            membership_coverage: PrincipalCoverage::Complete,
        })
    }

    pub async fn list_principals(&self) -> Result<PrincipalPage, DatabaseError> {
        let rows = sqlx::query(
            "SELECT oid::text AS oid, rolname, rolcanlogin, rolsuper FROM pg_roles ORDER BY rolcanlogin DESC, rolname COLLATE \"C\", oid",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql_error)?;
        let entries = rows
            .into_iter()
            .map(|row| {
                let can_login: bool = row.try_get("rolcanlogin").map_err(decode_error)?;
                let name: String = row.try_get("rolname").map_err(decode_error)?;
                let system = name.starts_with("pg_") || name == "postgres";
                Ok(PrincipalEntry {
                    id: PrincipalId {
                        profile_id: self.connection_id,
                        scope: PrincipalScope::Cluster,
                        native_id: row.try_get::<String, _>("oid").map_err(decode_error)?,
                        host: None,
                    },
                    kind: if can_login {
                        PrincipalKind::User
                    } else {
                        PrincipalKind::Role
                    },
                    native_kind: if can_login { "login_role" } else { "role" }.into(),
                    name,
                    system,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        Ok(PrincipalPage {
            connection: ConnectionIdentity {
                profile_id: self.connection_id,
                generation: 0,
            },
            entries,
            complete: true,
        })
    }

    pub async fn principal_ddl(
        &self,
        principal: &PrincipalEntry,
    ) -> Result<PrincipalDdl, DatabaseError> {
        let oid = &principal.id.native_id;
        let row = sqlx::query("SELECT rolname, rolcanlogin, rolsuper, rolcreatedb, rolcreaterole, rolinherit, rolreplication, rolbypassrls, rolconnlimit, rolvaliduntil::text FROM pg_roles WHERE oid = $1::oid")
            .bind(oid)
            .fetch_optional(&self.pool)
            .await
            .map_err(sql_error)?
            .ok_or_else(|| DatabaseError::configuration("principal no longer exists"))?;
        let name: String = row.try_get("rolname").map_err(decode_error)?;
        let login: bool = row.try_get("rolcanlogin").map_err(decode_error)?;
        let superuser: bool = row.try_get("rolsuper").map_err(decode_error)?;
        let createdb: bool = row.try_get("rolcreatedb").map_err(decode_error)?;
        let createrole: bool = row.try_get("rolcreaterole").map_err(decode_error)?;
        let inherit: bool = row.try_get("rolinherit").map_err(decode_error)?;
        let replication: bool = row.try_get("rolreplication").map_err(decode_error)?;
        let bypass_rls: bool = row.try_get("rolbypassrls").map_err(decode_error)?;
        let connlimit: i32 = row.try_get("rolconnlimit").map_err(decode_error)?;
        let valid_until: Option<String> = row.try_get("rolvaliduntil").map_err(decode_error)?;
        let mut sql = format!(
            "-- Password is intentionally omitted from read-only catalog DDL.\nCREATE ROLE {};\nALTER ROLE {} {} {} {} {} {} {} {} CONNECTION LIMIT {};",
            quote_identifier(&name),
            quote_identifier(&name),
            if login { "LOGIN" } else { "NOLOGIN" },
            if superuser {
                "SUPERUSER"
            } else {
                "NOSUPERUSER"
            },
            if createdb { "CREATEDB" } else { "NOCREATEDB" },
            if createrole {
                "CREATEROLE"
            } else {
                "NOCREATEROLE"
            },
            if inherit { "INHERIT" } else { "NOINHERIT" },
            if replication {
                "REPLICATION"
            } else {
                "NOREPLICATION"
            },
            if bypass_rls {
                "BYPASSRLS"
            } else {
                "NOBYPASSRLS"
            },
            connlimit
        );
        if let Some(valid_until) = valid_until {
            sql.push_str(&format!("\n-- VALID UNTIL {}", valid_until));
        }
        let memberships = sqlx::query_scalar::<_, String>("SELECT granted_role.rolname FROM pg_auth_members m JOIN pg_roles member_role ON member_role.oid = m.member JOIN pg_roles granted_role ON granted_role.oid = m.roleid WHERE member_role.oid = $1::oid ORDER BY granted_role.rolname COLLATE \"C\"")
            .bind(oid).fetch_all(&self.pool).await.map_err(sql_error)?;
        for role in memberships {
            sql.push_str(&format!(
                "\nGRANT {} TO {};",
                quote_identifier(&role),
                quote_identifier(&name)
            ));
        }
        Ok(PrincipalDdl {
            principal: principal.clone(),
            sql,
        })
    }
    pub async fn preview_relation_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
        options: &crate::model::relation::RelationPreviewOptions,
        page: crate::model::pagination::PageRequest,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        let mut adapter = self.clone();
        adapter.catalog_scope = scope.clone();
        adapter.preview_relation(relation, options, page).await
    }

    pub async fn relation_ddl_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<RelationDdl, DatabaseError> {
        let mut adapter = self.clone();
        adapter.catalog_scope = scope.clone();
        adapter.relation_ddl(relation).await
    }

    pub async fn resolve_relation_identity_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        let mut adapter = self.clone();
        adapter.catalog_scope = scope.clone();
        adapter.resolve_relation_identity(relation).await
    }
    pub const OWNER_CONTEXT_SQL: &str = r#"
        SELECT
            role.rolname AS name,
            role.rolcanlogin AS can_login,
            role.rolname = current_user AS is_current,
            current_user AS current_user,
            role.rolname = current_user
                OR pg_has_role(current_user, role.oid, $1) AS selectable
        FROM pg_roles AS role
        ORDER BY role.rolname = current_user DESC, role.rolname COLLATE "C"
    "#;

    pub const fn owner_role_privilege(_server_version_num: i32) -> &'static str {
        "SET"
    }

    pub const MONITOR_STATUS_SQL: &str = r#"
WITH db_stats AS (
    SELECT coalesce(sum(xact_commit), 0)::double precision AS commits,
           coalesce(sum(xact_rollback), 0)::double precision AS rollbacks,
           coalesce(sum(blks_hit), 0)::double precision AS block_hits,
           coalesce(sum(blks_read), 0)::double precision AS block_reads,
           coalesce(sum(tup_returned), 0)::double precision AS selects,
           coalesce(sum(tup_inserted), 0)::double precision AS inserts,
           coalesce(sum(tup_updated), 0)::double precision AS updates,
           coalesce(sum(tup_deleted), 0)::double precision AS deletes,
           coalesce(sum(deadlocks), 0)::double precision AS deadlocks,
           coalesce(sum(temp_files), 0)::double precision AS temp_files,
           coalesce(sum(temp_bytes), 0)::double precision AS temp_bytes,
           coalesce(pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0'), 0)::double precision AS wal_bytes
    FROM pg_stat_database WHERE datname = current_database()
), activity_stats AS (
    SELECT count(*)::double precision AS connections,
           count(*) FILTER (WHERE state = 'active')::double precision AS active_connections,
           count(*) FILTER (WHERE state = 'idle')::double precision AS idle_connections
    FROM pg_stat_activity WHERE pid <> pg_backend_pid()
)
SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint AS server_time_millis,
       floor(extract(epoch FROM pg_postmaster_start_time()) * 1000)::bigint AS server_generation,
       extract(epoch FROM clock_timestamp() - pg_postmaster_start_time())::double precision AS server_uptime,
       db_stats.*, activity_stats.*
FROM db_stats CROSS JOIN activity_stats
"#;

    pub const MONITOR_METADATA_SQL: &str = "SELECT current_setting('server_version') AS version, current_setting('max_connections')::bigint AS max_connections";

    pub const PROCESS_LIST_SQL: &str = r#"
SELECT pid, usename AS user_name, datname AS database_name,
       coalesce(host(client_addr), client_hostname, 'local') AS client,
       application_name, state,
       coalesce(nullif(wait_event_type, '') || ':' || wait_event, wait_event_type) AS wait,
       extract(epoch FROM clock_timestamp() - coalesce(query_start, xact_start, backend_start))::double precision AS elapsed_seconds,
       query
FROM pg_stat_activity
WHERE pid <> pg_backend_pid()
ORDER BY CASE WHEN state = 'active' THEN 0 ELSE 1 END,
         elapsed_seconds DESC NULLS LAST
LIMIT 2001
"#;

    pub async fn load_monitor_snapshot(
        &self,
    ) -> Result<crate::db::monitor::MonitorSnapshot, DatabaseError> {
        let row = sqlx::query(Self::MONITOR_STATUS_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let mut values = std::collections::BTreeMap::new();
        for (name, key) in [
            ("commits", MetricKey::Commits),
            ("rollbacks", MetricKey::Rollbacks),
            ("block_hits", MetricKey::BlockHits),
            ("block_reads", MetricKey::BlockReads),
            ("selects", MetricKey::Selects),
            ("inserts", MetricKey::Inserts),
            ("updates", MetricKey::Updates),
            ("deletes", MetricKey::Deletes),
            ("deadlocks", MetricKey::Deadlocks),
            ("temp_files", MetricKey::TempFiles),
            ("temp_bytes", MetricKey::TempBytes),
            ("wal_bytes", MetricKey::WalBytes),
            ("connections", MetricKey::Connections),
            ("active_connections", MetricKey::ActiveConnections),
            ("idle_connections", MetricKey::IdleConnections),
        ] {
            values.insert(key, row.try_get(name).map_err(decode_error)?);
        }
        let commits = values[&MetricKey::Commits];
        let rollbacks = values[&MetricKey::Rollbacks];
        values.insert(MetricKey::Transactions, commits + rollbacks);
        values.insert(
            MetricKey::ServerUptime,
            row.try_get("server_uptime").map_err(decode_error)?,
        );
        Ok(crate::db::monitor::MonitorSnapshot {
            server_time_millis: monitor_timestamp(&row, "server_time_millis")?,
            server_generation: monitor_timestamp(&row, "server_generation")?,
            values,
            redis_details: None,
            visibility: crate::db::monitor::MonitorVisibility::Unknown,
        })
    }

    pub async fn load_monitor_metadata(
        &self,
    ) -> Result<crate::db::monitor::MonitorMetadata, DatabaseError> {
        let row = sqlx::query(Self::MONITOR_METADATA_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        Ok(crate::db::monitor::MonitorMetadata {
            version: row.try_get("version").map_err(decode_error)?,
            max_connections: row
                .try_get::<i64, _>("max_connections")
                .map_err(decode_error)?
                .try_into()
                .ok(),
        })
    }

    pub async fn load_process_snapshot(
        &self,
    ) -> Result<crate::db::monitor::ProcessSnapshot, DatabaseError> {
        let rows = sqlx::query(Self::PROCESS_LIST_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let truncated = rows.len() > crate::db::monitor::MAX_PROCESS_ROWS;
        let rows = rows
            .into_iter()
            .take(crate::db::monitor::MAX_PROCESS_ROWS)
            .map(|row| {
                Ok(crate::model::dashboard::ProcessRow {
                    id: row.try_get::<i32, _>("pid").map_err(decode_error)? as u64,
                    native_id: None,
                    user: sanitize_terminal_text(
                        &row.try_get::<Option<String>, _>("user_name")
                            .map_err(decode_error)?
                            .unwrap_or_default(),
                    ),
                    database: row.try_get("database_name").map_err(decode_error)?,
                    client: row.try_get("client").map_err(decode_error)?,
                    application: row.try_get("application_name").map_err(decode_error)?,
                    state: row.try_get("state").map_err(decode_error)?,
                    wait: row.try_get("wait").map_err(decode_error)?,
                    elapsed: crate::db::monitor::parse_process_duration(
                        row.try_get("elapsed_seconds").map_err(decode_error)?,
                    ),
                    query: row
                        .try_get::<Option<String>, _>("query")
                        .map_err(decode_error)?
                        .map(|value| sanitize_terminal_text(&value)),
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        Ok(crate::db::monitor::ProcessSnapshot {
            rows,
            truncated,
            visibility: crate::db::monitor::MonitorVisibility::Unknown,
        })
    }
    pub fn plan_catalog_drop(
        request: CatalogDropRequest,
        entry: &CatalogEntry,
    ) -> Result<CatalogDropPlan, CatalogDropError> {
        let sql = match entry.kind {
            CatalogKind::Schema => format!("DROP SCHEMA {} RESTRICT", postgres_schema_name(entry)?),
            CatalogKind::Database => {
                let maintenance_database = request
                    .maintenance_database
                    .as_deref()
                    .filter(|database| !database.trim().is_empty())
                    .ok_or_else(|| CatalogDropError::Unsupported {
                        kind: CatalogKind::Database,
                        reason: "database drops require an explicit maintenance database"
                            .to_owned(),
                    })?;
                let target = maintenance_database.to_owned();
                if target == entry.qualified_name.object {
                    return Err(CatalogDropError::Unsupported {
                        kind: CatalogKind::Database,
                        reason: "maintenance database must differ from the target database"
                            .to_owned(),
                    });
                }
                return CatalogDropPlan::new(
                    request,
                    entry,
                    format!(
                        "DROP DATABASE {}",
                        quote_identifier(&entry.qualified_name.object)
                    ),
                )
                .map(|plan| {
                    plan.with_execution_target(CatalogDropExecutionTarget::MaintenanceDatabase(
                        target,
                    ))
                });
            }
            CatalogKind::Table => {
                format!("DROP TABLE {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::View => {
                format!("DROP VIEW {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::MaterializedView => format!(
                "DROP MATERIALIZED VIEW {}",
                postgres_qualified_name(entry, entry.kind)?
            ),
            CatalogKind::Sequence => format!(
                "DROP SEQUENCE {}",
                postgres_qualified_name(entry, entry.kind)?
            ),
            CatalogKind::Type => {
                format!("DROP TYPE {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::Column => format!(
                "ALTER TABLE {} DROP COLUMN {}",
                relation_name_for_drop(entry)?,
                quote_identifier(&entry.qualified_name.object)
            ),
            CatalogKind::Index => {
                format!("DROP INDEX {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::PrimaryKey
            | CatalogKind::UniqueConstraint
            | CatalogKind::ForeignKey
            | CatalogKind::CheckConstraint => format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                relation_name_for_drop(entry)?,
                quote_identifier(&entry.qualified_name.object)
            ),
            CatalogKind::Trigger => format!(
                "DROP TRIGGER {} ON {}",
                quote_identifier(&entry.qualified_name.object),
                relation_name_for_drop(entry)?
            ),
            kind => {
                return Err(CatalogDropError::Unsupported {
                    kind,
                    reason: "PostgreSQL catalog drop is not implemented for this object type"
                        .to_owned(),
                });
            }
        };
        CatalogDropPlan::new(request, entry, sql)
    }

    pub fn plan_catalog_drop_cascade(
        request: CatalogDropRequest,
        entry: &CatalogEntry,
    ) -> Result<CatalogDropPlan, CatalogDropError> {
        if entry.kind != CatalogKind::Schema || request.object != entry.id {
            return Err(CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "CASCADE is only available for PostgreSQL schema drops".to_owned(),
            });
        }
        let schema_name = postgres_schema_name(entry)?;
        CatalogDropPlan::postgres_schema_cascade(request, entry, &schema_name)
    }

    pub fn plan_principal_drop(
        request: crate::db::principal_drop::PrincipalDropRequest,
    ) -> Result<
        crate::db::principal_drop::PrincipalDropPlan,
        crate::db::principal_drop::PrincipalDropError,
    > {
        request.validate()?;
        let name = quote_identifier(&request.entry.name);
        crate::db::principal_drop::PrincipalDropPlan::new(request, format!("DROP ROLE {name}"))
    }

    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::MaterializedViews,
                ObjectGroup::Sequences,
                ObjectGroup::Functions,
                ObjectGroup::Procedures,
                ObjectGroup::Types,
            ],
            column_metadata: ColumnMetadataCapabilities {
                type_family: true,
                default_expression: true,
                identity: true,
                generated_expression: true,
                numeric_precision_and_scale: false,
                character_length: true,
                collation: true,
                comment: true,
                ..ColumnMetadataCapabilities::default()
            },
            supports_lazy_children: true,
        }
    }

    pub fn catalog_mutation_capabilities() -> CatalogMutationCapabilities {
        Self::catalog_mutation_capabilities_for_version(120_000)
    }

    pub fn catalog_mutation_capabilities_for_version(
        server_version_num: i32,
    ) -> CatalogMutationCapabilities {
        CatalogMutationCapabilities {
            profile_create: vec![
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Database),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::LoginRole,
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Role,
                    availability: CatalogMutationAvailability::Available,
                },
            ],
            create: vec![
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Schema),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Table),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Column),
                    availability: CatalogMutationAvailability::Unavailable {
                        reason: "column creation form is not implemented",
                    },
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Index),
                    availability: CatalogMutationAvailability::Unavailable {
                        reason: "index creation form is not implemented",
                    },
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::PrimaryKey),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::UniqueConstraint),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::ForeignKey),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::CheckConstraint),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::View),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::MaterializedView),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Sequence),
                    availability: CatalogMutationAvailability::Available,
                },
            ],
            edit: vec![
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Schema),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Table),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Column),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Index),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::PrimaryKey),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::UniqueConstraint),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::ForeignKey),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::CheckConstraint),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::View),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::MaterializedView),
                    availability: CatalogMutationAvailability::Available,
                },
                CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(CatalogKind::Sequence),
                    availability: CatalogMutationAvailability::Available,
                },
            ],
            view_options: ViewMutationCapabilities {
                security_barrier: if server_version_num >= 90200 {
                    ViewMutationOptionAvailability::Available
                } else {
                    ViewMutationOptionAvailability::Unavailable {
                        reason: "security_barrier requires PostgreSQL 9.2 or newer",
                    }
                },
                security_invoker: if server_version_num >= 150000 {
                    ViewMutationOptionAvailability::Available
                } else {
                    ViewMutationOptionAvailability::Unavailable {
                        reason: "security_invoker requires PostgreSQL 15 or newer",
                    }
                },
                check_option: if server_version_num >= 90400 {
                    ViewMutationOptionAvailability::Available
                } else {
                    ViewMutationOptionAvailability::Unavailable {
                        reason: "view check options require PostgreSQL 9.4 or newer",
                    }
                },
            },
        }
    }

    pub fn plan_catalog_mutation(
        request: CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        Self::plan_catalog_mutation_for_version(request, draft, baseline, 120_000)
    }

    fn plan_role_mutation(
        request: CatalogMutationRequest,
        draft: CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        let object_type = request.object_type;
        let CatalogDraft::Role(mut draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "role draft required".into(),
            });
        };
        draft.validate()?;
        let old = match baseline {
            Some(CatalogObjectDefinition::Role(r)) => Some(r),
            None => None,
            _ => return Err(CatalogMutationError::StaleState),
        };
        let name = draft.name.value().trim();
        let mut statements = Vec::new();
        let mut secret = None;
        if let Some(old) = old.as_ref() {
            if old.name != name {
                statements.push(format!(
                    "ALTER ROLE {} RENAME TO {}",
                    quote_identifier(&old.name),
                    quote_identifier(name)
                ));
            }
            let attrs = [
                (old.login, draft.login, "LOGIN", "NOLOGIN"),
                (old.superuser, draft.superuser, "SUPERUSER", "NOSUPERUSER"),
                (old.createdb, draft.createdb, "CREATEDB", "NOCREATEDB"),
                (
                    old.createrole,
                    draft.createrole,
                    "CREATEROLE",
                    "NOCREATEROLE",
                ),
                (old.inherit, draft.inherit, "INHERIT", "NOINHERIT"),
                (
                    old.replication,
                    draft.replication,
                    "REPLICATION",
                    "NOREPLICATION",
                ),
                (old.bypass_rls, draft.bypass_rls, "BYPASSRLS", "NOBYPASSRLS"),
            ];
            let changed = attrs
                .iter()
                .filter_map(|(a, b, yes, no)| (a != b).then_some(if *b { *yes } else { *no }))
                .collect::<Vec<_>>();
            if old.connection_limit.to_string() != draft.connection_limit.value().trim() {
                statements.push(format!(
                    "ALTER ROLE {} CONNECTION LIMIT {}",
                    quote_identifier(name),
                    draft.connection_limit.value().trim()
                ));
            }
            if !changed.is_empty() {
                statements.push(format!(
                    "ALTER ROLE {} {}",
                    quote_identifier(name),
                    changed.join(" ")
                ));
            }
            if let Some(password) = draft.password.take() {
                if !password.is_empty() {
                    statements.push(format!(
                        "ALTER ROLE {} PASSWORD '<REDACTED>'",
                        quote_identifier(name)
                    ));
                    secret = Some(password);
                }
            }
            if old.valid_until
                != OptionalMetadata::Supported(Some(draft.valid_until.value().to_owned()))
                && !draft.valid_until.value().trim().is_empty()
            {
                statements.push(format!(
                    "ALTER ROLE {} VALID UNTIL {}",
                    quote_identifier(name),
                    quote_literal(draft.valid_until.value().trim())
                ));
            }
            if old.comment
                != OptionalMetadata::Supported(
                    (!draft.comment.value().trim().is_empty())
                        .then(|| draft.comment.value().trim().to_owned()),
                )
            {
                statements.push(format!(
                    "COMMENT ON ROLE {} IS {}",
                    quote_identifier(name),
                    if draft.comment.value().trim().is_empty() {
                        "NULL".into()
                    } else {
                        quote_literal(draft.comment.value().trim())
                    }
                ));
            }
            let current: std::collections::HashSet<_> =
                old.memberships.iter().map(String::as_str).collect();
            let next = draft
                .memberships
                .value()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<std::collections::HashSet<_>>();
            for member in next.difference(&current) {
                statements.push(format!(
                    "GRANT {} TO {}",
                    quote_identifier(member),
                    quote_identifier(name)
                ));
            }
            for member in current.difference(&next) {
                statements.push(format!(
                    "REVOKE {} FROM {}",
                    quote_identifier(member),
                    quote_identifier(name)
                ));
            }
        } else {
            let mut create = format!(
                "CREATE ROLE {} {}",
                quote_identifier(name),
                if draft.login { "LOGIN" } else { "NOLOGIN" }
            );
            for enabled in [
                (draft.superuser, "SUPERUSER"),
                (draft.createdb, "CREATEDB"),
                (draft.createrole, "CREATEROLE"),
                (draft.inherit, "INHERIT"),
                (draft.replication, "REPLICATION"),
                (draft.bypass_rls, "BYPASSRLS"),
            ] {
                create.push(' ');
                create.push_str(if enabled.0 {
                    enabled.1
                } else {
                    match enabled.1 {
                        "SUPERUSER" => "NOSUPERUSER",
                        "CREATEDB" => "NOCREATEDB",
                        "CREATEROLE" => "NOCREATEROLE",
                        "INHERIT" => "NOINHERIT",
                        "REPLICATION" => "NOREPLICATION",
                        "BYPASSRLS" => "NOBYPASSRLS",
                        _ => "",
                    }
                });
            }
            create.push_str(&format!(
                " CONNECTION LIMIT {} VALID UNTIL {}",
                draft.connection_limit.value().trim(),
                quote_literal(draft.valid_until.value().trim())
            ));
            if let Some(password) = draft.password.take() {
                if !password.is_empty() {
                    create.push_str(" PASSWORD '<REDACTED>'");
                    secret = Some(password);
                }
            }
            statements.push(create);
            if !draft.comment.value().trim().is_empty() {
                statements.push(format!(
                    "COMMENT ON ROLE {} IS {}",
                    quote_identifier(name),
                    quote_literal(draft.comment.value().trim())
                ));
            }
            for member in draft
                .memberships
                .value()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                statements.push(format!(
                    "GRANT {} TO {}",
                    quote_identifier(member),
                    quote_identifier(name)
                ));
            }
        }
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Database,
            ["__role__", name],
        );
        let maintenance_database = request
            .current_database
            .clone()
            .unwrap_or_else(|| "postgres".into());
        let execution_target =
            crate::db::catalog_mutation::CatalogMutationTarget::maintenance(maintenance_database)?;
        let plan = CatalogMutationPlan::new(
            request,
            object_type,
            CatalogMutationExecutionMode::Transactional,
            execution_target,
            vec![CatalogTarget::Databases],
            CatalogSelectionHint::Parent(CatalogTarget::Databases),
            old.as_ref().map(|r| r.baseline_fingerprint.clone()),
            Vec::new(),
            statements,
        )?;
        Ok(plan.with_execution_secret_opt(secret).with_impact(
            crate::db::catalog_mutation::CatalogMutationImpact {
                old_object_id: id,
                owning_relation_id: None,
                namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                    database: None,
                    schema: None,
                },
                native_identity_changed: false,
            },
        ))
    }

    fn plan_database_mutation(
        request: CatalogMutationRequest,
        draft: CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        let CatalogDraft::Database(draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "database draft required".into(),
            });
        };
        draft.validate()?;
        let old = match baseline {
            Some(CatalogObjectDefinition::Database(value)) => Some(value),
            None => None,
            _ => return Err(CatalogMutationError::StaleState),
        };
        let name = draft.name.value().trim();
        let owner = draft.owner.value().trim();
        let limit = draft.connection_limit.value().trim();
        let mut statements = Vec::new();
        let mut warnings = Vec::new();
        let (old_name, fingerprint) = if let Some(old) = old.as_ref() {
            if let CatalogMutationAnchor::Catalog(id) = &request.anchor
                && id.native_path.first() != Some(&old.name)
            {
                return Err(CatalogMutationError::StaleState);
            }
            if old.name != name {
                if request.current_database.as_deref() == Some(old.name.as_str()) {
                    return Err(CatalogMutationError::InvalidPlan {
                        reason: "cannot rename the currently connected database".into(),
                    });
                }
                statements.push(format!(
                    "ALTER DATABASE {} RENAME TO {}",
                    quote_identifier(&old.name),
                    quote_identifier(name)
                ));
                warnings.push("Connection profiles and saved SQL execution targets were not rewritten after the rename.".into());
            }
            if old.owner != owner {
                statements.push(format!(
                    "ALTER DATABASE {} OWNER TO {}",
                    quote_identifier(name),
                    quote_identifier(owner)
                ));
            }
            if old.connection_limit.to_string() != limit {
                statements.push(format!(
                    "ALTER DATABASE {} CONNECTION LIMIT {}",
                    quote_identifier(name),
                    limit
                ));
            }
            if old.allow_connections != draft.allow_connections {
                statements.push(format!(
                    "ALTER DATABASE {} ALLOW_CONNECTIONS {}",
                    quote_identifier(name),
                    draft.allow_connections
                ));
            }
            if old.is_template != draft.is_template {
                statements.push(format!(
                    "ALTER DATABASE {} {}TEMPLATE",
                    quote_identifier(name),
                    if draft.is_template { "IS " } else { "IS NOT " }
                ));
            }
            let current_comment = match &old.comment {
                OptionalMetadata::Supported(value) => value.clone(),
                OptionalMetadata::Unsupported => None,
            };
            let next_comment = (!draft.comment.value().trim().is_empty())
                .then(|| draft.comment.value().trim().to_owned());
            if current_comment != next_comment {
                statements.push(format!(
                    "COMMENT ON DATABASE {} IS {}",
                    quote_identifier(name),
                    next_comment
                        .as_deref()
                        .map_or_else(|| "NULL".into(), quote_literal)
                ));
            }
            (
                Some(old.name.clone()),
                Some(old.baseline_fingerprint.clone()),
            )
        } else {
            let mut create = format!(
                "CREATE DATABASE {} OWNER {}",
                quote_identifier(name),
                quote_identifier(owner)
            );
            for (key, value) in [
                ("TEMPLATE", draft.template.value().trim()),
                ("ENCODING", draft.encoding.value().trim()),
                ("LOCALE_PROVIDER", draft.locale_provider.value().trim()),
                ("LOCALE", draft.locale.value().trim()),
                ("LC_COLLATE", draft.collation.value().trim()),
                ("LC_CTYPE", draft.ctype.value().trim()),
                ("TABLESPACE", draft.tablespace.value().trim()),
            ] {
                if !value.is_empty() {
                    create.push_str(&format!(" {key} {}", quote_literal(value)));
                }
            }
            create.push_str(&format!(" CONNECTION LIMIT {limit}"));
            statements.push(create);
            if !draft.comment.value().trim().is_empty() {
                statements.push(format!(
                    "COMMENT ON DATABASE {} IS {}",
                    quote_identifier(name),
                    quote_literal(draft.comment.value().trim())
                ));
            }
            (None, None)
        };
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let id = CatalogId::new(request.connection.profile_id, CatalogKind::Database, [name]);
        let renamed = old_name.as_ref().is_some_and(|old| old != name);
        let old_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Database,
            [old_name.clone().unwrap_or_else(|| name.to_owned())],
        );
        let maintenance_database = request
            .current_database
            .clone()
            .unwrap_or_else(|| "postgres".into());
        CatalogMutationPlan::new(
            request,
            CatalogObjectType::Catalog(CatalogKind::Database),
            CatalogMutationExecutionMode::Autocommit,
            crate::db::catalog_mutation::CatalogMutationTarget::maintenance(maintenance_database)?,
            vec![CatalogTarget::Databases],
            CatalogSelectionHint::Object(id.clone()),
            fingerprint,
            warnings,
            statements,
        )
        .map(|plan| {
            plan.with_impact(crate::db::catalog_mutation::CatalogMutationImpact {
                old_object_id: old_id,
                owning_relation_id: None,
                namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                    database: Some(id),
                    schema: None,
                },
                native_identity_changed: renamed,
            })
        })
    }

    pub fn plan_catalog_mutation_for_version(
        request: CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
        server_version_num: i32,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        if request.object_type == CatalogObjectType::Catalog(CatalogKind::Database) {
            return Self::plan_database_mutation(request, draft, baseline);
        }
        if matches!(
            request.object_type,
            CatalogObjectType::LoginRole | CatalogObjectType::Role
        ) {
            return Self::plan_role_mutation(request, draft, baseline);
        }
        if request.object_type == CatalogObjectType::Catalog(CatalogKind::Sequence) {
            return Self::plan_sequence_mutation(request, draft, baseline);
        }
        if request.object_type == CatalogObjectType::Catalog(CatalogKind::Table) {
            return Self::plan_table_mutation(request, draft, baseline);
        }
        if request.object_type == CatalogObjectType::Catalog(CatalogKind::Column) {
            let CatalogMutationAnchor::Catalog(column_id) = request.anchor.clone() else {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "column mutation requires a column anchor",
                });
            };
            let Some(CatalogObjectDefinition::Table(table)) = baseline else {
                return Err(CatalogMutationError::StaleState);
            };
            let CatalogDraft::Table(table_draft) = draft else {
                return Err(CatalogMutationError::InvalidDraft {
                    reason: "column mutation requires a table draft".into(),
                });
            };
            let ordinal = column_id
                .native_path
                .last()
                .and_then(|value| value.parse::<u32>().ok());
            let Some(row) = table_draft.columns.iter().find(|row| {
                row.ordinal_position == ordinal.unwrap_or_default()
                    || row.existing_name.as_deref()
                        == column_id.native_path.last().map(String::as_str)
            }) else {
                return Err(CatalogMutationError::StaleState);
            };
            let relation = format!(
                "{}.{}",
                quote_identifier(&table.schema),
                quote_identifier(&table.name)
            );
            let old_name = row
                .existing_name
                .as_deref()
                .unwrap_or(row.name.value().trim());
            let mut statements = Vec::new();
            if old_name != row.name.value().trim() {
                statements.push(format!(
                    "ALTER TABLE {} RENAME COLUMN {} TO {}",
                    relation,
                    quote_identifier(old_name),
                    quote_identifier(row.name.value().trim())
                ));
            }
            if let Some(old) = table.columns.iter().find(|column| column.name == old_name) {
                if old.native_type != row.native_type.value().trim() {
                    statements.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        relation,
                        quote_identifier(old_name),
                        row.native_type.value().trim()
                    ));
                }
                let old_default = match &old.default_expression {
                    OptionalMetadata::Supported(value) => value.clone(),
                    OptionalMetadata::Unsupported => None,
                };
                let new_default = (!row.default_expression.value().trim().is_empty())
                    .then(|| row.default_expression.value().trim().to_owned());
                if old_default != new_default {
                    statements.push(if let Some(value) = new_default {
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                            relation,
                            quote_identifier(old_name),
                            value
                        )
                    } else {
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                            relation,
                            quote_identifier(old_name)
                        )
                    });
                }
                if old.nullable != row.nullable {
                    statements.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} {} NOT NULL",
                        relation,
                        quote_identifier(old_name),
                        if row.nullable { "DROP" } else { "SET" }
                    ));
                }
                let old_comment = match &old.comment {
                    OptionalMetadata::Supported(value) => value.clone(),
                    OptionalMetadata::Unsupported => None,
                };
                let new_comment =
                    (!row.comment.value().is_empty()).then(|| row.comment.value().to_owned());
                if old_comment != new_comment {
                    statements.push(format!(
                        "COMMENT ON COLUMN {}.{} IS {}",
                        relation,
                        quote_identifier(row.name.value().trim()),
                        new_comment.as_deref().map_or("NULL".into(), quote_literal)
                    ));
                }
            }
            if statements.is_empty() {
                return Err(CatalogMutationError::NoChanges);
            }
            let table_id = CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Table,
                [
                    table.database.clone(),
                    table.schema.clone(),
                    table.name.clone(),
                ],
            );
            let schema_id = CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Schema,
                [table.database.clone(), table.schema.clone()],
            );
            return CatalogMutationPlan::new(
                request,
                CatalogObjectType::Catalog(CatalogKind::Column),
                CatalogMutationExecutionMode::Transactional,
                crate::db::catalog_mutation::CatalogMutationTarget::database_target(
                    ExecutionTarget {
                        profile_id: table_id.profile_id(),
                        database: table.database.clone(),
                        schema: Some(table.schema.clone()),
                    },
                )?,
                vec![CatalogTarget::RelationChildren {
                    relation: table_id.clone(),
                }],
                CatalogSelectionHint::Object(column_id.clone()),
                Some(table.baseline_fingerprint),
                Vec::new(),
                statements,
            )
            .map(|plan| {
                plan.with_impact(crate::db::catalog_mutation::CatalogMutationImpact {
                    old_object_id: column_id,
                    owning_relation_id: Some(table_id),
                    namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                        database: None,
                        schema: Some(schema_id),
                    },
                    native_identity_changed: old_name != row.name.value().trim(),
                })
            });
        }
        if request.object_type == CatalogObjectType::Catalog(CatalogKind::Index) {
            return Self::plan_index_mutation(request, draft, baseline);
        }
        if request.object_type == CatalogObjectType::Catalog(CatalogKind::View) {
            return Self::plan_view_mutation(request, draft, baseline, server_version_num);
        }
        if request.object_type == CatalogObjectType::Catalog(CatalogKind::MaterializedView) {
            return Self::plan_materialized_view_mutation(request, draft, baseline);
        }
        if matches!(
            request.object_type,
            CatalogObjectType::Catalog(
                CatalogKind::PrimaryKey
                    | CatalogKind::UniqueConstraint
                    | CatalogKind::ForeignKey
                    | CatalogKind::CheckConstraint
            )
        ) {
            return Self::plan_constraint_mutation(request, draft, baseline);
        }
        if request.object_type != CatalogObjectType::Catalog(CatalogKind::Schema) {
            return Err(CatalogMutationError::UnsupportedOperation {
                object_type: request.object_type,
            });
        }
        let crate::model::catalog_editor::CatalogDraft::Schema(draft) = draft else {
            return Err(CatalogMutationError::UnsupportedOperation {
                object_type: request.object_type,
            });
        };
        draft.validate()?;
        let (database, old_name, baseline_fingerprint) =
            match (&request.mode, &request.anchor, baseline.as_ref()) {
                (
                    crate::db::catalog_mutation::CatalogMutationMode::Create,
                    CatalogMutationAnchor::Catalog(id),
                    _,
                ) => {
                    let [database] = id.native_path.as_slice() else {
                        return Err(CatalogMutationError::InvalidAnchor {
                            reason: "schema create requires a database anchor",
                        });
                    };
                    (database.clone(), None, None)
                }
                (
                    crate::db::catalog_mutation::CatalogMutationMode::Edit,
                    CatalogMutationAnchor::Catalog(id),
                    Some(CatalogObjectDefinition::Schema(schema)),
                ) => {
                    let [database, name] = id.native_path.as_slice() else {
                        return Err(CatalogMutationError::InvalidAnchor {
                            reason: "schema edit requires a schema ID",
                        });
                    };
                    if schema.database != *database || schema.name != *name {
                        return Err(CatalogMutationError::StaleState);
                    }
                    if schema.baseline_fingerprint.is_empty() {
                        return Err(CatalogMutationError::StaleState);
                    }
                    (
                        database.clone(),
                        Some(name.clone()),
                        Some(schema.baseline_fingerprint.clone()),
                    )
                }
                (crate::db::catalog_mutation::CatalogMutationMode::Edit, _, _) => {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "schema edit requires a schema anchor",
                    });
                }
                _ => {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "schema baseline is missing".into(),
                    });
                }
            };
        let mut statements = Vec::new();
        let name = draft.name.value().trim();
        let owner = draft.owner.value().trim();
        let comment = draft.comment.value();
        if old_name.is_some() {
            let CatalogObjectDefinition::Schema(schema) =
                baseline.expect("validated schema baseline")
            else {
                return Err(CatalogMutationError::StaleState);
            };
            if schema.name != name {
                statements.push(format!(
                    "ALTER SCHEMA {} RENAME TO {}",
                    quote_identifier(&schema.name),
                    quote_identifier(name)
                ));
            }
            if schema.owner != owner {
                statements.push(format!(
                    "ALTER SCHEMA {} OWNER TO {}",
                    quote_identifier(name),
                    quote_identifier(owner)
                ));
            }
            let current_comment = match schema.comment {
                OptionalMetadata::Supported(value) => value,
                OptionalMetadata::Unsupported => None,
            };
            let next_comment = (!comment.is_empty()).then(|| comment.to_owned());
            if current_comment != next_comment {
                statements.push(format!(
                    "COMMENT ON SCHEMA {} IS {}",
                    quote_identifier(name),
                    next_comment
                        .as_deref()
                        .map_or_else(|| "NULL".to_owned(), quote_literal)
                ));
            }
        } else {
            statements.push(format!(
                "CREATE SCHEMA {} AUTHORIZATION {}",
                quote_identifier(name),
                quote_identifier(owner)
            ));
            if !comment.is_empty() {
                statements.push(format!(
                    "COMMENT ON SCHEMA {} IS {}",
                    quote_identifier(name),
                    quote_literal(comment)
                ));
            }
        }
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [database.clone(), name.to_owned()],
        );
        let database_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Database,
            [database.clone()],
        );
        let old_schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [
                database.clone(),
                old_name.clone().unwrap_or_else(|| name.to_owned()),
            ],
        );
        let native_identity_changed = old_name.as_deref() != Some(name);
        CatalogMutationPlan::new(
            request.clone(),
            CatalogObjectType::Catalog(CatalogKind::Schema),
            CatalogMutationExecutionMode::Transactional,
            crate::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
                profile_id: request.connection.profile_id,
                database: database.clone(),
                schema: None,
            })?,
            vec![CatalogTarget::Schemas {
                database: database_id.clone(),
            }],
            CatalogSelectionHint::Object(schema_id),
            baseline_fingerprint,
            Vec::new(),
            statements,
        )
        .map(|plan| {
            plan.with_impact(crate::db::catalog_mutation::CatalogMutationImpact {
                old_object_id: old_schema_id.clone(),
                owning_relation_id: None,
                namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                    database: Some(database_id),
                    schema: Some(old_schema_id.clone()),
                },
                native_identity_changed,
            })
        })
    }

    pub fn plan_catalog_mutation_for_adapter(
        &self,
        request: CatalogMutationRequest,
        draft: CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        Self::plan_catalog_mutation_for_version(request, draft, baseline, self.server_version_num)
    }

    fn plan_sequence_mutation(
        request: CatalogMutationRequest,
        draft: CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        let CatalogDraft::Sequence(draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "sequence draft required".into(),
            });
        };
        draft.validate()?;
        let old = match baseline {
            Some(CatalogObjectDefinition::Sequence(s)) => Some(s),
            None => None,
            _ => return Err(CatalogMutationError::StaleState),
        };
        let name = draft.name.value().trim();
        let schema = draft.schema.value().trim();
        let qualified = format!("{}.{}", quote_identifier(schema), quote_identifier(name));
        let bound = |b: &crate::db::catalog_mutation::SequenceBound, keyword: &str| match b {
            crate::db::catalog_mutation::SequenceBound::Unset => String::new(),
            crate::db::catalog_mutation::SequenceBound::NoLimit => format!(" NO {keyword}"),
            crate::db::catalog_mutation::SequenceBound::Value(v) => format!(" {keyword} {v}"),
        };
        let mut clauses = format!(
            " AS {} INCREMENT BY {} START WITH {} CACHE {}",
            draft.data_type.value().trim(),
            draft.increment.value().trim(),
            draft.start_value.value().trim(),
            draft.cache.value().trim()
        );
        clauses.push_str(&bound(&draft.min_value.to_bound(), "MINVALUE"));
        clauses.push_str(&bound(&draft.max_value.to_bound(), "MAXVALUE"));
        if draft.cycle {
            clauses.push_str(" CYCLE");
        }
        let owned = draft.owned_by.value().trim();
        if !owned.is_empty() {
            clauses.push_str(&format!(
                " OWNED BY {}",
                if owned.eq_ignore_ascii_case("NONE") {
                    "NONE".into()
                } else {
                    owned
                        .split('.')
                        .map(quote_identifier)
                        .collect::<Vec<_>>()
                        .join(".")
                }
            ));
        }
        let mut statements = Vec::new();
        let old_name = old.as_ref().map(|s| s.name.as_str());
        let baseline_fingerprint = old.as_ref().map(|s| s.baseline_fingerprint.clone());
        if let Some(old) = old.as_ref() {
            if old.name != name || old.schema != schema {
                statements.push(format!(
                    "ALTER SEQUENCE {} RENAME TO {}",
                    format!(
                        "{}.{}",
                        quote_identifier(&old.schema),
                        quote_identifier(&old.name)
                    ),
                    quote_identifier(name)
                ));
            }
            statements.push(format!("ALTER SEQUENCE {}{}", qualified, clauses));
            if old.owner != draft.owner.value().trim() {
                statements.push(format!(
                    "ALTER SEQUENCE {} OWNER TO {}",
                    qualified,
                    quote_identifier(draft.owner.value().trim())
                ));
            }
        } else {
            statements.push(format!("CREATE SEQUENCE {}{}", qualified, clauses));
            if !draft.owner.value().trim().is_empty() {
                statements.push(format!(
                    "ALTER SEQUENCE {} OWNER TO {}",
                    qualified,
                    quote_identifier(draft.owner.value().trim())
                ));
            }
        }
        let comment = draft.comment.value().trim();
        let old_comment = old.as_ref().and_then(|sequence| match &sequence.comment {
            OptionalMetadata::Supported(value) => value.as_deref(),
            OptionalMetadata::Unsupported => None,
        });
        if old_comment != (!comment.is_empty()).then_some(comment) {
            statements.push(format!(
                "COMMENT ON SEQUENCE {} IS {}",
                qualified,
                (!comment.is_empty())
                    .then(|| quote_literal(comment))
                    .unwrap_or_else(|| "NULL".into())
            ));
        }
        if !draft.restart_value.value().trim().is_empty() {
            statements.push(format!(
                "ALTER SEQUENCE {} RESTART WITH {}",
                qualified,
                draft.restart_value.value().trim()
            ));
        }
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let database = match &request.anchor {
            CatalogMutationAnchor::Catalog(id)
            | CatalogMutationAnchor::Group { schema: id, .. } => {
                id.native_path.first().cloned().unwrap_or_default()
            }
            CatalogMutationAnchor::Profile { .. } => String::new(),
            CatalogMutationAnchor::Principal(_) => String::new(),
        };
        let schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [database.clone(), schema.to_owned()],
        );
        let sequence_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Sequence,
            [
                database.clone(),
                schema.to_owned(),
                name.to_owned(),
                String::new(),
            ],
        );
        CatalogMutationPlan::new(
            request.clone(),
            CatalogObjectType::Catalog(CatalogKind::Sequence),
            CatalogMutationExecutionMode::Transactional,
            crate::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
                profile_id: request.connection.profile_id,
                database: database.clone(),
                schema: Some(schema.to_owned()),
            })?,
            vec![CatalogTarget::Objects {
                schema: schema_id.clone(),
                group: ObjectGroup::Sequences,
            }],
            CatalogSelectionHint::Object(sequence_id.clone()),
            baseline_fingerprint,
            Vec::new(),
            statements,
        )
        .map(|p| {
            p.with_impact(crate::db::catalog_mutation::CatalogMutationImpact {
                old_object_id: sequence_id,
                owning_relation_id: None,
                namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                    database: Some(CatalogId::new(
                        request.connection.profile_id,
                        CatalogKind::Database,
                        [database],
                    )),
                    schema: Some(schema_id),
                },
                native_identity_changed: old_name != Some(name),
            })
        })
    }

    fn plan_view_mutation(
        request: CatalogMutationRequest,
        draft: CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
        server_version_num: i32,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        let CatalogDraft::View(draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "view draft required".into(),
            });
        };
        draft.validate()?;
        if draft.security_barrier.value.is_some() && server_version_num < 90200
            || draft.check_option.value.is_some() && server_version_num < 90400
            || draft.security_invoker.value.is_some() && server_version_num < 150000
        {
            return Err(CatalogMutationError::UnsupportedOperation {
                object_type: CatalogObjectType::Catalog(CatalogKind::View),
            });
        }
        let (database, old, fingerprint) = match (&request.mode, &request.anchor, baseline.as_ref())
        {
            (CatalogMutationMode::Create, CatalogMutationAnchor::Group { schema, .. }, _)
            | (CatalogMutationMode::Create, CatalogMutationAnchor::Catalog(schema), _)
                if schema.kind == CatalogKind::Schema =>
            {
                (
                    schema.native_path.first().cloned().unwrap_or_default(),
                    None,
                    None,
                )
            }
            (
                CatalogMutationMode::Edit,
                CatalogMutationAnchor::Catalog(id),
                Some(CatalogObjectDefinition::View(view)),
            ) => {
                let [database, schema, name, ..] = id.native_path.as_slice() else {
                    return Err(CatalogMutationError::StaleState);
                };
                if *database != view.database || *schema != view.schema || *name != view.name {
                    return Err(CatalogMutationError::StaleState);
                }
                (
                    database.clone(),
                    Some(view.clone()),
                    Some(view.baseline_fingerprint.clone()),
                )
            }
            (CatalogMutationMode::Edit, ..) => return Err(CatalogMutationError::StaleState),
            _ => {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "view create requires a schema group anchor",
                });
            }
        };
        let name = draft.name.value().trim();
        let schema = draft.schema.value().trim();
        let qualified = format!("{}.{}", quote_identifier(schema), quote_identifier(name));
        let columns = split_view_columns(draft.output_columns.value())?;
        let query = draft.query.value().trim().trim_end_matches(';').trim();
        let mut statements = Vec::new();
        let mut warnings = Vec::new();
        if let Some(old) = old {
            if old.name != name || old.schema != schema {
                warnings.push("CREATE OR REPLACE VIEW cannot rename or move a view; the existing view identity is retained".into());
            }
            let column_clause = (!columns.is_empty())
                .then(|| {
                    format!(
                        " ({})",
                        columns
                            .iter()
                            .map(|v| quote_identifier(v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .unwrap_or_default();
            let options = view_options_clause(&draft, server_version_num)?;
            statements.push(format!(
                "CREATE OR REPLACE VIEW {}{}{} AS {}{}",
                qualified, column_clause, options.0, query, options.1
            ));
            if old.owner != draft.owner.value().trim() {
                statements.push(format!(
                    "ALTER VIEW {} OWNER TO {}",
                    qualified,
                    quote_identifier(draft.owner.value().trim())
                ));
            }
            let old_comment = match &old.comment {
                OptionalMetadata::Supported(v) => v.clone(),
                OptionalMetadata::Unsupported => None,
            };
            let new_comment = (!draft.comment.value().trim().is_empty())
                .then(|| draft.comment.value().trim().to_owned());
            if old_comment != new_comment {
                statements.push(format!(
                    "COMMENT ON VIEW {} IS {}",
                    qualified,
                    new_comment.as_deref().map_or("NULL".into(), quote_literal)
                ));
            }
            if old.query.trim() != query || old.output_columns != columns {
                warnings.push(
                    "Replacing a view may fail when existing output columns change type or name"
                        .into(),
                );
            }
        } else {
            let column_clause = (!columns.is_empty())
                .then(|| {
                    format!(
                        " ({})",
                        columns
                            .iter()
                            .map(|v| quote_identifier(v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .unwrap_or_default();
            let options = view_options_clause(&draft, server_version_num)?;
            statements.push(format!(
                "CREATE VIEW {}{}{} AS {}{}",
                qualified, column_clause, options.0, query, options.1
            ));
            if !draft.owner.value().trim().is_empty() {
                statements.push(format!(
                    "ALTER VIEW {} OWNER TO {}",
                    qualified,
                    quote_identifier(draft.owner.value().trim())
                ));
            }
            if !draft.comment.value().trim().is_empty() {
                statements.push(format!(
                    "COMMENT ON VIEW {} IS {}",
                    qualified,
                    quote_literal(draft.comment.value().trim())
                ));
            }
        }
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [database.clone(), schema.to_owned()],
        );
        let view_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::View,
            [
                database.clone(),
                schema.to_owned(),
                name.to_owned(),
                String::new(),
            ],
        );
        CatalogMutationPlan::new(
            request.clone(),
            CatalogObjectType::Catalog(CatalogKind::View),
            CatalogMutationExecutionMode::Transactional,
            crate::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
                profile_id: request.connection.profile_id,
                database: database.clone(),
                schema: Some(schema.to_owned()),
            })?,
            vec![CatalogTarget::Objects {
                schema: schema_id.clone(),
                group: ObjectGroup::Views,
            }],
            CatalogSelectionHint::Object(view_id.clone()),
            fingerprint,
            warnings,
            statements,
        )
        .map(|plan| {
            plan.with_impact(crate::db::catalog_mutation::CatalogMutationImpact {
                old_object_id: view_id,
                owning_relation_id: None,
                namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                    database: Some(CatalogId::new(
                        request.connection.profile_id,
                        CatalogKind::Database,
                        [database],
                    )),
                    schema: Some(schema_id),
                },
                native_identity_changed: false,
            })
        })
    }

    fn plan_materialized_view_mutation(
        request: CatalogMutationRequest,
        draft: CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        let CatalogDraft::MaterializedView(draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "materialized view draft required".into(),
            });
        };
        draft.validate()?;
        let (database, old, fingerprint) = match (&request.mode, &request.anchor, baseline.as_ref())
        {
            (CatalogMutationMode::Create, CatalogMutationAnchor::Group { schema, .. }, _)
            | (CatalogMutationMode::Create, CatalogMutationAnchor::Catalog(schema), _)
                if schema.kind == CatalogKind::Schema =>
            {
                (
                    schema.native_path.first().cloned().unwrap_or_default(),
                    None,
                    None,
                )
            }
            (
                CatalogMutationMode::Edit,
                CatalogMutationAnchor::Catalog(id),
                Some(CatalogObjectDefinition::MaterializedView(view)),
            ) => {
                let [database, schema, name, ..] = id.native_path.as_slice() else {
                    return Err(CatalogMutationError::StaleState);
                };
                if *database != view.database || *schema != view.schema || *name != view.name {
                    return Err(CatalogMutationError::StaleState);
                }
                (
                    database.clone(),
                    Some(view),
                    Some(view.baseline_fingerprint.clone()),
                )
            }
            (CatalogMutationMode::Edit, ..) => return Err(CatalogMutationError::StaleState),
            _ => {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "materialized view create requires a schema anchor",
                });
            }
        };
        let name = draft.name.value().trim();
        let schema = draft.schema.value().trim();
        let qualified = format!("{}.{}", quote_identifier(schema), quote_identifier(name));
        let mut statements = Vec::new();
        let mut native_identity_changed = false;
        if let Some(old) = old {
            let mut current_schema = old.schema.clone();
            let mut current_name = old.name.clone();
            if old.name != name {
                statements.push(format!(
                    "ALTER MATERIALIZED VIEW {}.{} RENAME TO {}",
                    quote_identifier(&current_schema),
                    quote_identifier(&current_name),
                    quote_identifier(name)
                ));
                current_name = name.to_owned();
                native_identity_changed = true;
            }
            if old.schema != schema {
                statements.push(format!(
                    "ALTER MATERIALIZED VIEW {} SET SCHEMA {}",
                    format!(
                        "{}.{}",
                        quote_identifier(&current_schema),
                        quote_identifier(&current_name)
                    ),
                    quote_identifier(schema)
                ));
                current_schema = schema.to_owned();
                native_identity_changed = true;
            }
            if old.owner != draft.owner.value().trim() {
                statements.push(format!(
                    "ALTER MATERIALIZED VIEW {} OWNER TO {}",
                    format!(
                        "{}.{}",
                        quote_identifier(&current_schema),
                        quote_identifier(&current_name)
                    ),
                    quote_identifier(draft.owner.value().trim())
                ));
            }
            let old_comment = match &old.comment {
                OptionalMetadata::Supported(v) => v.clone(),
                OptionalMetadata::Unsupported => None,
            };
            let new_comment = (!draft.comment.value().trim().is_empty())
                .then(|| draft.comment.value().trim().to_owned());
            if old_comment != new_comment {
                statements.push(format!(
                    "COMMENT ON MATERIALIZED VIEW {} IS {}",
                    format!(
                        "{}.{}",
                        quote_identifier(&current_schema),
                        quote_identifier(&current_name)
                    ),
                    new_comment.as_deref().map_or("NULL".into(), quote_literal)
                ));
            }
            let old_tablespace = match &old.tablespace {
                OptionalMetadata::Supported(v) => v.clone(),
                OptionalMetadata::Unsupported => None,
            };
            let new_tablespace = (!draft.tablespace.value().trim().is_empty())
                .then(|| draft.tablespace.value().trim().to_owned());
            if old_tablespace != new_tablespace {
                statements.push(format!(
                    "ALTER MATERIALIZED VIEW {} SET TABLESPACE {}",
                    format!(
                        "{}.{}",
                        quote_identifier(&current_schema),
                        quote_identifier(&current_name)
                    ),
                    new_tablespace
                        .as_deref()
                        .map_or_else(|| "pg_default".to_owned(), quote_identifier)
                ));
            }
        } else {
            let data = if draft.with_data {
                "WITH DATA"
            } else {
                "WITH NO DATA"
            };
            statements.push(format!(
                "CREATE MATERIALIZED VIEW {} TABLESPACE {} AS {} {}",
                qualified,
                if draft.tablespace.value().trim().is_empty() {
                    "pg_default".into()
                } else {
                    quote_identifier(draft.tablespace.value().trim())
                },
                draft.query.value().trim().trim_end_matches(';').trim(),
                data
            ));
            if !draft.owner.value().trim().is_empty() {
                statements.push(format!(
                    "ALTER MATERIALIZED VIEW {} OWNER TO {}",
                    qualified,
                    quote_identifier(draft.owner.value().trim())
                ));
            }
            if !draft.comment.value().trim().is_empty() {
                statements.push(format!(
                    "COMMENT ON MATERIALIZED VIEW {} IS {}",
                    qualified,
                    quote_literal(draft.comment.value().trim())
                ));
            }
        }
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [database.clone(), schema.to_owned()],
        );
        let view_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::MaterializedView,
            [
                database.clone(),
                schema.to_owned(),
                name.to_owned(),
                String::new(),
            ],
        );
        CatalogMutationPlan::new(
            request.clone(),
            CatalogObjectType::Catalog(CatalogKind::MaterializedView),
            CatalogMutationExecutionMode::Transactional,
            crate::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
                profile_id: request.connection.profile_id,
                database: database.clone(),
                schema: Some(schema.to_owned()),
            })?,
            vec![CatalogTarget::Objects {
                schema: schema_id.clone(),
                group: ObjectGroup::MaterializedViews,
            }],
            CatalogSelectionHint::Object(view_id.clone()),
            fingerprint,
            Vec::new(),
            statements,
        )
        .map(|plan| {
            plan.with_impact(crate::db::catalog_mutation::CatalogMutationImpact {
                old_object_id: match &request.anchor {
                    CatalogMutationAnchor::Catalog(id) => id.clone(),
                    _ => view_id,
                },
                owning_relation_id: None,
                namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                    database: Some(CatalogId::new(
                        request.connection.profile_id,
                        CatalogKind::Database,
                        [database],
                    )),
                    schema: Some(schema_id),
                },
                native_identity_changed,
            })
        })
    }

    fn plan_constraint_mutation(
        request: CatalogMutationRequest,
        draft: CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        use crate::model::catalog_editor::CatalogDraft;
        let CatalogDraft::Constraint(draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "constraint draft required".into(),
            });
        };
        draft.validate()?;
        let kind = match draft.kind {
            ConstraintDefinitionKind::PrimaryKey { .. } => ConstraintDefinitionKind::PrimaryKey {
                columns: draft
                    .columns
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect(),
            },
            ConstraintDefinitionKind::Unique { .. } => ConstraintDefinitionKind::Unique {
                columns: draft
                    .columns
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect(),
            },
            ConstraintDefinitionKind::ForeignKey { .. } => ConstraintDefinitionKind::ForeignKey {
                columns: draft
                    .columns
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect(),
                referenced_schema: draft.referenced_schema.value().trim().to_owned(),
                referenced_relation: draft.referenced_relation.value().trim().to_owned(),
                referenced_columns: draft
                    .referenced_columns
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect(),
                match_type: draft.match_type.value().trim().to_owned(),
                on_update: draft.on_update.value().trim().to_owned(),
                on_delete: draft.on_delete.value().trim().to_owned(),
            },
            ConstraintDefinitionKind::Check { .. } => ConstraintDefinitionKind::Check {
                expression: draft.expression.value().trim().to_owned(),
                no_inherit: draft.no_inherit,
            },
        };
        let columns = constraint_columns(&kind)
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(quote_identifier)
            .collect::<Vec<_>>()
            .join(", ");
        let name = (!draft.name.value().trim().is_empty())
            .then(|| quote_identifier(draft.name.value().trim()));
        let relation = format!(
            "{}.{}",
            quote_identifier(draft.schema.value().trim()),
            quote_identifier(draft.relation.value().trim())
        );
        let mut body = match &kind {
            ConstraintDefinitionKind::PrimaryKey { .. } => format!("PRIMARY KEY ({columns})"),
            ConstraintDefinitionKind::Unique { .. } => format!("UNIQUE ({columns})"),
            ConstraintDefinitionKind::ForeignKey { .. } => format!(
                "FOREIGN KEY ({columns}) REFERENCES {}.{} ({}) MATCH {} ON UPDATE {} ON DELETE {}",
                quote_identifier(draft.referenced_schema.value().trim()),
                quote_identifier(draft.referenced_relation.value().trim()),
                draft
                    .referenced_columns
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(quote_identifier)
                    .collect::<Vec<_>>()
                    .join(", "),
                draft.match_type.value().trim(),
                draft.on_update.value().trim(),
                draft.on_delete.value().trim()
            ),
            ConstraintDefinitionKind::Check { .. } => format!(
                "CHECK ({}){}",
                draft.expression.value().trim(),
                if draft.no_inherit { " NO INHERIT" } else { "" }
            ),
        };
        if draft.deferrable {
            body.push_str(" DEFERRABLE");
        } else if matches!(
            &kind,
            ConstraintDefinitionKind::PrimaryKey { .. }
                | ConstraintDefinitionKind::Unique { .. }
                | ConstraintDefinitionKind::ForeignKey { .. }
        ) {
            body.push_str(" NOT DEFERRABLE");
        }
        if draft.initially_deferred {
            body.push_str(" INITIALLY DEFERRED");
        }
        if draft.not_valid
            && matches!(
                &kind,
                ConstraintDefinitionKind::ForeignKey { .. }
                    | ConstraintDefinitionKind::Check { .. }
            )
        {
            body.push_str(" NOT VALID");
        }
        let create = if let Some(name) = name {
            format!("ALTER TABLE {relation} ADD CONSTRAINT {name} {body}")
        } else {
            format!("ALTER TABLE {relation} ADD {body}")
        };
        let relation_id = match &request.anchor {
            CatalogMutationAnchor::Catalog(id) if id.kind.is_relation() => id.clone(),
            CatalogMutationAnchor::Catalog(id) if id.native_path.len() >= 4 => CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Table,
                id.native_path[..4].to_vec(),
            ),
            _ => CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Table,
                [
                    draft.database.value().trim(),
                    draft.schema.value().trim(),
                    draft.relation.value().trim(),
                    "",
                ],
            ),
        };
        let (statements, destructive, baseline_fp, old_id) =
            if let Some(CatalogObjectDefinition::Constraint(old)) = baseline {
                let CatalogMutationAnchor::Catalog(id) = &request.anchor else {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "constraint edit requires an anchor",
                    });
                };
                let structural_same = match (&old.kind, &kind) {
                    (
                        ConstraintDefinitionKind::PrimaryKey { columns: a },
                        ConstraintDefinitionKind::PrimaryKey { columns: b },
                    )
                    | (
                        ConstraintDefinitionKind::Unique { columns: a },
                        ConstraintDefinitionKind::Unique { columns: b },
                    ) => a == b,
                    (
                        ConstraintDefinitionKind::Check {
                            expression: a,
                            no_inherit: ai,
                        },
                        ConstraintDefinitionKind::Check { .. },
                    ) => a == draft.expression.value().trim() && *ai == draft.no_inherit,
                    (
                        ConstraintDefinitionKind::ForeignKey {
                            columns,
                            referenced_schema,
                            referenced_relation,
                            referenced_columns,
                            match_type,
                            on_update,
                            on_delete,
                        },
                        ConstraintDefinitionKind::ForeignKey { .. },
                    ) => {
                        columns
                            == &draft
                                .columns
                                .value()
                                .split(',')
                                .map(|v| v.trim().to_owned())
                                .collect::<Vec<_>>()
                            && referenced_schema == draft.referenced_schema.value().trim()
                            && referenced_relation == draft.referenced_relation.value().trim()
                            && referenced_columns
                                == &draft
                                    .referenced_columns
                                    .value()
                                    .split(',')
                                    .map(|v| v.trim().to_owned())
                                    .collect::<Vec<_>>()
                            && match_type == draft.match_type.value().trim()
                            && on_update == draft.on_update.value().trim()
                            && on_delete == draft.on_delete.value().trim()
                    }
                    _ => false,
                };
                if old.name == draft.name.value().trim()
                    && structural_same
                    && old.deferrable == draft.deferrable
                    && old.initially_deferred == draft.initially_deferred
                    && old.validated == !draft.not_valid
                {
                    return Err(CatalogMutationError::NoChanges);
                }
                if structural_same
                    && old.deferrable == draft.deferrable
                    && old.initially_deferred == draft.initially_deferred
                    && old.name != draft.name.value().trim()
                {
                    (
                        vec![format!(
                            "ALTER TABLE {relation} RENAME CONSTRAINT {} TO {}",
                            quote_identifier(&old.name),
                            quote_identifier(draft.name.value().trim())
                        )],
                        false,
                        Some(old.baseline_fingerprint),
                        id.clone(),
                    )
                } else if structural_same
                    && old.deferrable == draft.deferrable
                    && old.initially_deferred == draft.initially_deferred
                    && old.name == draft.name.value().trim()
                    && !old.validated
                    && !draft.not_valid
                {
                    (
                        vec![format!(
                            "ALTER TABLE {relation} VALIDATE CONSTRAINT {}",
                            quote_identifier(&old.name)
                        )],
                        false,
                        Some(old.baseline_fingerprint),
                        id.clone(),
                    )
                } else {
                    (
                        vec![
                            format!(
                                "ALTER TABLE {relation} DROP CONSTRAINT {}",
                                quote_identifier(&old.name)
                            ),
                            create,
                        ],
                        true,
                        Some(old.baseline_fingerprint),
                        id.clone(),
                    )
                }
            } else {
                (vec![create], false, None, {
                    let mut path = relation_id.native_path.clone();
                    path.push(draft.name.value().trim().to_owned());
                    CatalogId::new(
                        request.connection.profile_id,
                        match request.object_type {
                            CatalogObjectType::Catalog(kind) => kind,
                            _ => CatalogKind::CheckConstraint,
                        },
                        path,
                    )
                })
            };
        CatalogMutationPlan::new(
            request.clone(),
            request.object_type,
            CatalogMutationExecutionMode::Transactional,
            crate::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
                profile_id: request.connection.profile_id,
                database: draft.database.value().trim().to_owned(),
                schema: Some(draft.schema.value().trim().to_owned()),
            })?,
            vec![CatalogTarget::RelationChildren {
                relation: relation_id.clone(),
            }],
            CatalogSelectionHint::Object(old_id.clone()),
            baseline_fp,
            Vec::new(),
            statements,
        )
        .map(|plan| {
            plan.with_destructive(destructive).with_impact(
                crate::db::catalog_mutation::CatalogMutationImpact {
                    old_object_id: old_id,
                    owning_relation_id: Some(relation_id),
                    namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                        database: Some(CatalogId::new(
                            request.connection.profile_id,
                            CatalogKind::Database,
                            [draft.database.value().trim()],
                        )),
                        schema: Some(CatalogId::new(
                            request.connection.profile_id,
                            CatalogKind::Schema,
                            [draft.database.value().trim(), draft.schema.value().trim()],
                        )),
                    },
                    native_identity_changed: false,
                },
            )
        })
    }

    fn plan_table_mutation(
        request: CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        use crate::model::catalog_editor::{CatalogDraft, DraftRowState};
        let CatalogDraft::Table(draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "table draft required".into(),
            });
        };
        draft.validate()?;
        let (database, old_schema, old_name, old_oid, baseline_fingerprint, base) =
            match (&request.mode, &request.anchor, baseline) {
                (
                    CatalogMutationMode::Create,
                    CatalogMutationAnchor::Group { schema, .. },
                    None,
                ) => {
                    let [database, schema] = schema.native_path.as_slice() else {
                        return Err(CatalogMutationError::InvalidAnchor {
                            reason: "table create requires a schema anchor",
                        });
                    };
                    (database.clone(), schema.clone(), None, None, None, None)
                }
                (
                    CatalogMutationMode::Edit,
                    CatalogMutationAnchor::Catalog(id),
                    Some(CatalogObjectDefinition::Table(table)),
                ) => {
                    let [database, schema, name, oid] = id.native_path.as_slice() else {
                        return Err(CatalogMutationError::InvalidAnchor {
                            reason: "table edit requires a table ID",
                        });
                    };
                    if table.database != *database
                        || table.schema != *schema
                        || table.name != *name
                        || table.baseline_fingerprint.is_empty()
                    {
                        return Err(CatalogMutationError::StaleState);
                    }
                    (
                        database.clone(),
                        schema.clone(),
                        Some(name.clone()),
                        oid.parse::<u32>().ok(),
                        Some(table.baseline_fingerprint.clone()),
                        Some(table),
                    )
                }
                (CatalogMutationMode::Edit, _, _) => {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "table edit requires a table baseline",
                    });
                }
                _ => {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "table baseline is missing".into(),
                    });
                }
            };
        let schema = draft.schema.value().trim();
        let name = draft.name.value().trim();
        let owner = draft.owner.value().trim();
        let relation =
            |s: &str, n: &str| format!("{}.{}", quote_identifier(s), quote_identifier(n));
        let mut statements = Vec::new();
        let mut warnings = Vec::new();
        let mut destructive = false;
        if let Some(table) = &base {
            let baseline_existing = table
                .columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>();
            let draft_existing = draft
                .columns
                .iter()
                .filter(|row| {
                    !matches!(
                        row.state,
                        DraftRowState::Added | DraftRowState::Removed { .. }
                    )
                })
                .filter_map(|row| row.existing_name.as_deref())
                .collect::<Vec<_>>();
            let surviving = baseline_existing
                .iter()
                .copied()
                .filter(|name| draft_existing.contains(name))
                .collect::<Vec<_>>();
            let draft_order = draft
                .columns
                .iter()
                .filter(|row| !matches!(row.state, DraftRowState::Removed { .. }))
                .collect::<Vec<_>>();
            let draft_existing_order = draft_order
                .iter()
                .filter_map(|row| row.existing_name.as_deref())
                .collect::<Vec<_>>();
            let additions_are_at_end = draft_order
                .iter()
                .position(|row| matches!(row.state, DraftRowState::Added))
                .is_none_or(|first_added| {
                    draft_order[first_added..]
                        .iter()
                        .all(|row| matches!(row.state, DraftRowState::Added))
                });
            if surviving != draft_existing_order {
                return Err(CatalogMutationError::InvalidDraft {
                    reason: "PostgreSQL table edits cannot reorder existing columns or insert a column before an existing column".into(),
                });
            }
            if !additions_are_at_end {
                warnings.push(
                    "PostgreSQL appends newly added columns; their requested position is not preserved"
                        .into(),
                );
            }
            let old_relation = relation(&table.schema, &table.name);
            let current_relation = relation(schema, name);
            if table.name != name {
                statements.push(format!(
                    "ALTER TABLE {} RENAME TO {}",
                    old_relation,
                    quote_identifier(name)
                ));
            }
            if table.schema != schema {
                statements.push(format!(
                    "ALTER TABLE {} SET SCHEMA {}",
                    current_relation,
                    quote_identifier(schema)
                ));
            }
            let relation = relation(schema, name);
            if table.owner != owner {
                statements.push(format!(
                    "ALTER TABLE {} OWNER TO {}",
                    relation,
                    quote_identifier(owner)
                ));
            }
            let old_comment = match &table.comment {
                OptionalMetadata::Supported(value) => value.clone(),
                OptionalMetadata::Unsupported => None,
            };
            let new_comment =
                (!draft.comment.value().is_empty()).then(|| draft.comment.value().to_owned());
            if old_comment != new_comment {
                statements.push(format!(
                    "COMMENT ON TABLE {} IS {}",
                    relation,
                    new_comment.as_deref().map_or("NULL".into(), quote_literal)
                ));
            }
            let mut existing = table.columns.iter();
            for row in &draft.columns {
                if matches!(row.state, DraftRowState::Removed { .. }) {
                    continue;
                }
                let old = row
                    .existing_name
                    .as_deref()
                    .and_then(|n| table.columns.iter().find(|c| c.name == n));
                let Some(old) = old else {
                    statements.push(format!(
                        "ALTER TABLE {} ADD COLUMN {}",
                        relation,
                        column_definition_fragment(row)?
                    ));
                    if !row.comment.value().is_empty() {
                        statements.push(format!(
                            "COMMENT ON COLUMN {}.{} IS {}",
                            relation,
                            quote_identifier(row.name.value().trim()),
                            quote_literal(row.comment.value())
                        ));
                    }
                    continue;
                };
                if old.native_type != row.native_type.value().trim() {
                    statements.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
                        relation,
                        quote_identifier(&old.name),
                        row.native_type.value().trim()
                    ));
                    warnings.push(format!(
                        "Changing type of column {} may fail or lose data",
                        old.name
                    ));
                    destructive = true;
                }
                let old_default = match &old.default_expression {
                    OptionalMetadata::Supported(v) => v.clone(),
                    OptionalMetadata::Unsupported => None,
                };
                let new_default = (!row.default_expression.value().trim().is_empty())
                    .then(|| row.default_expression.value().trim().to_owned());
                if old_default != new_default {
                    if let Some(value) = new_default.as_deref() {
                        statements.push(format!(
                            "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                            relation,
                            quote_identifier(&old.name),
                            value
                        ));
                    } else {
                        statements.push(format!(
                            "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                            relation,
                            quote_identifier(&old.name)
                        ));
                    }
                }
                if old.nullable != row.nullable {
                    statements.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} {} NOT NULL",
                        relation,
                        quote_identifier(&old.name),
                        if row.nullable { "DROP" } else { "SET" }
                    ));
                }
                let old_identity = matches!(old.identity, OptionalMetadata::Supported(Some(true)));
                if old_identity != row.identity {
                    if row.identity {
                        statements.push(format!(
                            "ALTER TABLE {} ALTER COLUMN {} ADD GENERATED BY DEFAULT AS IDENTITY",
                            relation,
                            quote_identifier(&old.name)
                        ));
                    } else {
                        statements.push(format!(
                            "ALTER TABLE {} ALTER COLUMN {} DROP IDENTITY",
                            relation,
                            quote_identifier(&old.name)
                        ));
                    }
                }
                let old_generated = match &old.generated_expression {
                    OptionalMetadata::Supported(v) => v.clone(),
                    OptionalMetadata::Unsupported => None,
                };
                let new_generated = (!row.generated_expression.value().trim().is_empty())
                    .then(|| row.generated_expression.value().trim().to_owned());
                if old_generated != new_generated {
                    return Err(CatalogMutationError::UnsupportedOperation {
                        object_type: request.object_type,
                    });
                }
                let old_comment = match &old.comment {
                    OptionalMetadata::Supported(v) => v.clone(),
                    OptionalMetadata::Unsupported => None,
                };
                let new_comment =
                    (!row.comment.value().is_empty()).then(|| row.comment.value().to_owned());
                if old_comment != new_comment {
                    statements.push(format!(
                        "COMMENT ON COLUMN {}.{} IS {}",
                        relation,
                        quote_identifier(&old.name),
                        new_comment.as_deref().map_or("NULL".into(), quote_literal)
                    ));
                }
                if row.name.value().trim() != old.name {
                    statements.push(format!(
                        "ALTER TABLE {} RENAME COLUMN {} TO {}",
                        relation,
                        quote_identifier(&old.name),
                        quote_identifier(row.name.value().trim())
                    ));
                }
            }
            for old in &table.columns {
                if !draft.columns.iter().any(|row| {
                    row.existing_name.as_deref() == Some(old.name.as_str())
                        && !matches!(row.state, DraftRowState::Removed { .. })
                }) {
                    statements.push(format!(
                        "ALTER TABLE {} DROP COLUMN {}",
                        relation,
                        quote_identifier(&old.name)
                    ));
                    warnings.push(format!(
                        "Dropping column {} permanently deletes its data",
                        old.name
                    ));
                    destructive = true;
                }
            }
            let _ = existing.next();
        } else {
            if draft.columns.is_empty() {
                return Err(CatalogMutationError::InvalidDraft {
                    reason: "a table requires at least one column".into(),
                });
            }
            let columns = draft
                .columns
                .iter()
                .map(|row| column_definition_fragment(row))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            statements.push(format!(
                "CREATE TABLE {} ({})",
                relation(schema, name),
                columns
            ));
            if !owner.is_empty() {
                statements.push(format!(
                    "ALTER TABLE {} OWNER TO {}",
                    relation(schema, name),
                    quote_identifier(owner)
                ));
            }
            if !draft.comment.value().is_empty() {
                statements.push(format!(
                    "COMMENT ON TABLE {} IS {}",
                    relation(schema, name),
                    quote_literal(draft.comment.value())
                ));
            }
            for row in &draft.columns {
                if !row.comment.value().is_empty() {
                    statements.push(format!(
                        "COMMENT ON COLUMN {}.{} IS {}",
                        relation(schema, name),
                        quote_identifier(row.name.value().trim()),
                        quote_literal(row.comment.value())
                    ));
                }
            }
        }
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let old_name = old_name.clone().unwrap_or_else(|| name.to_owned());
        let old_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Table,
            [
                database.clone(),
                old_schema.clone(),
                old_name.clone(),
                old_oid.map_or_else(String::new, |v| v.to_string()),
            ],
        );
        let new_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Table,
            [
                database.clone(),
                schema.to_owned(),
                name.to_owned(),
                old_oid.map_or_else(String::new, |v| v.to_string()),
            ],
        );
        let database_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Database,
            [database.clone()],
        );
        let old_schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [database.clone(), old_schema],
        );
        let new_schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [database.clone(), schema.to_owned()],
        );
        CatalogMutationPlan::new(
            request,
            CatalogObjectType::Catalog(CatalogKind::Table),
            CatalogMutationExecutionMode::Transactional,
            crate::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
                profile_id: old_id.profile_id(),
                database: database.clone(),
                schema: Some(schema.to_owned()),
            })?,
            vec![
                CatalogTarget::Objects {
                    schema: old_schema_id.clone(),
                    group: ObjectGroup::Tables,
                },
                CatalogTarget::Objects {
                    schema: new_schema_id.clone(),
                    group: ObjectGroup::Tables,
                },
                CatalogTarget::Groups {
                    schema: old_schema_id.clone(),
                },
                CatalogTarget::Groups {
                    schema: new_schema_id.clone(),
                },
                CatalogTarget::RelationChildren {
                    relation: new_id.clone(),
                },
            ],
            CatalogSelectionHint::Object(new_id.clone()),
            baseline_fingerprint,
            warnings,
            statements,
        )
        .map(|plan| {
            plan.with_destructive(destructive).with_impact(
                crate::db::catalog_mutation::CatalogMutationImpact {
                    old_object_id: old_id,
                    owning_relation_id: Some(new_id),
                    namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                        database: Some(database_id),
                        schema: Some(new_schema_id.clone()),
                    },
                    native_identity_changed: old_name != name || old_schema_id != new_schema_id,
                },
            )
        })
    }

    fn plan_index_mutation(
        request: CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        use crate::model::catalog_editor::CatalogDraft;
        let CatalogDraft::Index(draft) = draft else {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "index draft required".into(),
            });
        };
        draft.validate()?;
        let current = match baseline {
            Some(CatalogObjectDefinition::Index(index)) => Some(index),
            None => None,
            _ => return Err(CatalogMutationError::StaleState),
        };
        let schema = draft.schema.value().trim();
        let relation_name = draft.relation.value().trim();
        let name = draft.name.value().trim();
        let qualified_relation = format!(
            "{}.{}",
            quote_identifier(schema),
            quote_identifier(relation_name)
        );
        let definition = draft
            .columns
            .iter()
            .map(|column| {
                let expression = if column.is_expression {
                    column.expression.value().trim().to_owned()
                } else {
                    quote_identifier(column.expression.value().trim())
                };
                format!(
                    "{} {}",
                    expression,
                    if column.descending { "DESC" } else { "ASC" }
                ) + if column.nulls_first {
                    " NULLS FIRST"
                } else {
                    " NULLS LAST"
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let include = draft
            .include_columns
            .value()
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(quote_identifier)
            .collect::<Vec<_>>()
            .join(", ");
        let mut create = format!(
            "CREATE {}INDEX {} ON {} USING {} ({})",
            if draft.unique { "UNIQUE " } else { "" },
            quote_identifier(name),
            qualified_relation,
            quote_identifier(draft.access_method.value().trim()),
            definition
        );
        if !include.is_empty() {
            create.push_str(&format!(" INCLUDE ({include})"));
        }
        if !draft.predicate.value().trim().is_empty() {
            create.push_str(&format!(" WHERE {}", draft.predicate.value().trim()));
        }
        if !draft.tablespace.value().trim().is_empty() {
            create.push_str(&format!(
                " TABLESPACE {}",
                quote_identifier(draft.tablespace.value().trim())
            ));
        }

        let (database, old_name, old_id, fingerprint, statements, destructive, warnings) =
            if let Some(old) = current.clone() {
                let old_id = match &request.anchor {
                    CatalogMutationAnchor::Catalog(id) => id.clone(),
                    _ => {
                        return Err(CatalogMutationError::InvalidAnchor {
                            reason: "index edit requires an index anchor",
                        });
                    }
                };
                let mut warnings = Vec::new();
                let mut destructive = false;
                let mut statements = Vec::new();
                let structural_changed = old.unique != draft.unique
                    || old.access_method != draft.access_method.value().trim()
                    || old.columns.len() != draft.columns.len()
                    || old.columns.iter().zip(&draft.columns).any(|(a, b)| {
                        a.expression != b.expression.value().trim()
                            || a.descending != b.descending
                            || a.nulls_first != b.nulls_first
                    })
                    || old.include_columns
                        != draft
                            .include_columns
                            .value()
                            .split(',')
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                let predicate = (!draft.predicate.value().trim().is_empty())
                    .then(|| draft.predicate.value().trim().to_owned());
                let tablespace = (!draft.tablespace.value().trim().is_empty())
                    .then(|| draft.tablespace.value().trim().to_owned());
                let old_predicate = matches!(old.predicate, OptionalMetadata::Supported(Some(_)));
                let old_tablespace = matches!(old.tablespace, OptionalMetadata::Supported(Some(_)));
                let predicate_changed = old_predicate != predicate.is_some();
                let tablespace_changed = old_tablespace != tablespace.is_some();
                if structural_changed || predicate_changed || tablespace_changed {
                    statements.push(format!(
                        "DROP INDEX {}",
                        qualified_index_name(&old.schema, &old.name)
                    ));
                    statements.push(create);
                    warnings.push(
                        "Changing index definition requires dropping and recreating the index"
                            .into(),
                    );
                    destructive = true;
                } else if old.name != name {
                    statements.push(format!(
                        "ALTER INDEX {} RENAME TO {}",
                        qualified_index_name(&old.schema, &old.name),
                        quote_identifier(name)
                    ));
                }
                (
                    old.database,
                    Some(old.name),
                    old_id,
                    Some(old.baseline_fingerprint),
                    statements,
                    destructive,
                    warnings,
                )
            } else {
                if request.mode == CatalogMutationMode::Edit {
                    return Err(CatalogMutationError::StaleState);
                }
                let database = match &request.anchor {
                    CatalogMutationAnchor::Catalog(relation) if relation.kind.is_relation() => {
                        relation.native_path.first().cloned().unwrap_or_default()
                    }
                    CatalogMutationAnchor::Group { schema, .. } => {
                        schema.native_path.first().cloned().unwrap_or_default()
                    }
                    _ => {
                        return Err(CatalogMutationError::InvalidAnchor {
                            reason: "index create requires a relation anchor",
                        });
                    }
                };
                (
                    database,
                    None,
                    CatalogId::new(
                        request.connection.profile_id,
                        CatalogKind::Index,
                        ["", schema, relation_name, name],
                    ),
                    None,
                    vec![create],
                    false,
                    Vec::new(),
                )
            };
        if statements.is_empty() {
            return Err(CatalogMutationError::NoChanges);
        }
        let relation_id = CatalogId::new(
            request.connection.profile_id,
            match &request.anchor {
                CatalogMutationAnchor::Catalog(id) if id.kind.is_relation() => id.kind,
                CatalogMutationAnchor::Catalog(_) => current
                    .as_ref()
                    .map_or(CatalogKind::Table, |index| index.relation_kind),
                _ => CatalogKind::Table,
            },
            match &request.anchor {
                CatalogMutationAnchor::Catalog(id) if id.kind.is_relation() => {
                    id.native_path.clone()
                }
                CatalogMutationAnchor::Catalog(id) => id.native_path[..4].to_vec(),
                _ => vec![
                    database.clone(),
                    schema.to_owned(),
                    relation_name.to_owned(),
                    String::new(),
                ],
            },
        );
        let schema_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Schema,
            [database.clone(), schema.to_owned()],
        );
        let new_id = CatalogId::new(
            request.connection.profile_id,
            CatalogKind::Index,
            [
                database.clone(),
                schema.to_owned(),
                relation_name.to_owned(),
                match &request.anchor {
                    CatalogMutationAnchor::Catalog(id) => {
                        id.native_path.get(3).cloned().unwrap_or_default()
                    }
                    _ => String::new(),
                },
                name.to_owned(),
            ],
        );
        CatalogMutationPlan::new(
            request.clone(),
            CatalogObjectType::Catalog(CatalogKind::Index),
            CatalogMutationExecutionMode::Transactional,
            crate::db::catalog_mutation::CatalogMutationTarget::database_target(ExecutionTarget {
                profile_id: request.connection.profile_id,
                database: database.clone(),
                schema: Some(schema.to_owned()),
            })?,
            vec![CatalogTarget::RelationChildren {
                relation: relation_id.clone(),
            }],
            CatalogSelectionHint::Object(new_id),
            fingerprint,
            warnings,
            statements,
        )
        .map(|plan| {
            plan.with_destructive(destructive).with_impact(
                crate::db::catalog_mutation::CatalogMutationImpact {
                    old_object_id: old_id,
                    owning_relation_id: Some(relation_id),
                    namespace: crate::db::catalog_mutation::CatalogMutationNamespace {
                        database: Some(CatalogId::new(
                            request.connection.profile_id,
                            CatalogKind::Database,
                            [database],
                        )),
                        schema: Some(schema_id),
                    },
                    native_identity_changed: old_name.as_deref() != Some(name),
                },
            )
        })
    }

    pub async fn transaction_backend(&self) -> Result<PostgresTransactionBackend, DatabaseError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let pid = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(PostgresTransactionBackend {
            connection,
            control: self.pool.clone(),
            pid,
            adapter: self.clone(),
        })
    }

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::Postgres {
            return Err(DatabaseError::configuration("profile is not PostgreSQL"));
        }
        let options = super::pgwire::connect_options(profile, password, 5432).await?;

        let pool = PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let server_version =
            match sqlx::query_scalar::<_, i32>("SELECT current_setting('server_version_num')::int")
                .fetch_one(&pool)
                .await
            {
                Ok(version) => version,
                Err(error) => {
                    pool.close().await;
                    return Err(DatabaseError::from_sqlx(error, ErrorCategory::Network));
                }
            };
        if !supports_server_version(server_version) {
            pool.close().await;
            return Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("postgres_version_unsupported".to_owned()),
                message: format!(
                    "PostgreSQL 12 or newer is required; server_version_num is {server_version}"
                ),
                diagnostic: None,
            });
        }
        Ok(Self {
            pool,
            connection_id: profile.id,
            catalog_scope: profile.catalog_scope.clone(),
            server_version_num: server_version,
        })
    }

    pub fn mutation_capabilities(&self) -> CatalogMutationCapabilities {
        Self::catalog_mutation_capabilities_for_version(self.server_version_num)
    }

    pub fn server_version_num(&self) -> i32 {
        self.server_version_num
    }

    pub const PROBE_SQL: &str =
        "SELECT version() AS version, current_database() AS database, current_user AS current_user";

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let row = sqlx::query(Self::PROBE_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(ServerInfo {
            kind: DatabaseKind::Postgres,
            version: row.try_get("version").map_err(decode_error)?,
            database: row.try_get("database").map_err(decode_error)?,
            current_user: Some(row.try_get("current_user").map_err(decode_error)?),
        })
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let database = sqlx::query_scalar::<_, String>("SELECT current_database()")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let schemas = sqlx::query_scalar::<_, String>(CATALOG_SCHEMAS_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;

        Ok(CatalogDiscovery {
            databases: vec![DiscoveredDatabase {
                name: database,
                schemas,
            }],
            warnings: Vec::new(),
        })
    }

    pub async fn discoverable_databases(&self) -> Result<Vec<String>, DatabaseError> {
        sqlx::query_scalar::<_, String>(
            "SELECT datname FROM pg_database \
             WHERE datallowconn AND NOT datistemplate \
               AND has_database_privilege(datname, 'CONNECT') \
             ORDER BY datname COLLATE \"C\"",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))
    }

    /// Resolve a relation whose stored name may be stale, using its stable PostgreSQL OID.
    ///
    /// This is intentionally separate from `verify_relation`: callers that need the old
    /// identity to remain strict must continue using that method.
    pub async fn resolve_relation_identity(
        &self,
        relation: &CatalogId,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        let Some((database, oid)) = stale_relation_identity(self.connection_id, relation) else {
            return Ok(None);
        };
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let current: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if current != database {
            return Ok(None);
        }

        let row = sqlx::query(
            "SELECT n.nspname AS schema_name, c.relname AS object_name, \
                    c.relkind::text AS relkind, obj_description(c.oid, 'pg_class') AS comment \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.oid = $1::oid",
        )
        .bind(oid)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let schema: String = row.try_get("schema_name").map_err(decode_error)?;
        let name: String = row.try_get("object_name").map_err(decode_error)?;
        let native_kind: String = row.try_get("relkind").map_err(decode_error)?;
        let (kind, native_kind) = match native_kind.as_str() {
            "r" | "p" => (CatalogKind::Table, "table"),
            "v" => (CatalogKind::View, "view"),
            "m" => (CatalogKind::MaterializedView, "materialized_view"),
            _ => return Ok(None),
        };
        if !self.catalog_scope.allows_schema(database, &schema) {
            return Ok(None);
        }
        let schema_id = CatalogId::new(
            self.connection_id,
            CatalogKind::Schema,
            [database.to_owned(), schema.clone()],
        );
        let id = CatalogId::new(
            self.connection_id,
            kind,
            [
                database.to_owned(),
                schema.clone(),
                name.clone(),
                oid.to_string(),
            ],
        );
        CatalogEntry::relation(
            id,
            schema_id,
            qualified_object(database, &schema, &name),
            native_kind,
            OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
            true,
        )
        .map(Some)
        .map_err(catalog_invariant)
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_pool(sql).await
    }

    pub(crate) async fn execute_pool(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_pool_with_budget(sql, QueryBudget::UNBOUNDED)
            .await
    }

    pub(crate) async fn execute_pool_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        super::pgwire::execute_pool(&self.pool, super::pgwire::Server::Postgres, sql, budget).await
    }

    pub(crate) async fn execute_connection(
        &self,
        connection: &mut PgConnection,
        sql: &str,
    ) -> Result<QueryOutcome, DatabaseError> {
        super::pgwire::execute_connection(connection, sql, QueryBudget::UNBOUNDED).await
    }

    pub(crate) async fn execute_catalog_mutation(
        &self,
        plan: &CatalogMutationPlan,
    ) -> Result<QueryOutcome, DatabaseError> {
        plan.validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        if matches!(
            plan.execution_mode,
            CatalogMutationExecutionMode::Autocommit
        ) {
            let mut outcome = None;
            for statement in plan.statements() {
                let executable = plan
                    .execution_secret()
                    .map(|secret| {
                        statement.replace("'<REDACTED>'", &quote_literal(secret.expose()))
                    })
                    .unwrap_or_else(|| statement.clone());
                outcome = Some(
                    self.execute_connection(&mut *connection, &executable)
                        .await?,
                );
            }
            return outcome.ok_or_else(|| {
                DatabaseError::configuration("catalog mutation plan has no statements")
            });
        }
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        let mut outcome = None;
        for statement in plan.statements() {
            let executable = plan
                .execution_secret()
                .map(|secret| statement.replace("'<REDACTED>'", &quote_literal(secret.expose())))
                .unwrap_or_else(|| statement.clone());
            outcome = Some(
                self.execute_connection(&mut *transaction, &executable)
                    .await?,
            );
        }
        transaction.commit().await.map_err(sql_error)?;
        outcome
            .ok_or_else(|| DatabaseError::configuration("catalog mutation plan has no statements"))
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        request
            .validate_for_profile(self.connection_id)
            .map_err(DatabaseError::invalid_catalog_request)?;
        if matches!(
            request.key.target,
            CatalogTarget::Objects {
                group: ObjectGroup::Triggers,
                ..
            }
        ) {
            return Err(DatabaseError::unsupported_catalog_target(
                DatabaseKind::Postgres,
                &request.key.target,
            ));
        }

        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let mut transaction = connection
            .begin()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let page = match &request.key.target {
            CatalogTarget::Databases => self.load_database_page(&mut transaction, request).await,
            CatalogTarget::Schemas { database } => {
                self.load_schema_page(&mut transaction, request, database)
                    .await
            }
            CatalogTarget::Groups { schema } => {
                self.load_group_page(&mut transaction, request, schema)
                    .await
            }
            CatalogTarget::Objects { schema, group } => {
                self.load_object_page(&mut transaction, request, schema, *group)
                    .await
            }
            CatalogTarget::ProjectionChildren { .. } => Err(DatabaseError::unsupported(
                "projection catalog target is not supported by PostgreSQL",
            )),
            CatalogTarget::RelationChildren { relation } => {
                self.load_relation_children_page(&mut transaction, request, relation)
                    .await
            }
        };
        let rollback = transaction
            .rollback()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql));
        if let Err(rollback_error) = rollback {
            return match page {
                Ok(_) => Err(rollback_error),
                Err(page_error) => Err(page_error),
            };
        }
        page
    }

    pub async fn load_catalog_object_definition(
        &self,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        request
            .validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        if request.connection.profile_id != self.connection_id {
            return Err(DatabaseError::configuration(
                "catalog definition profile mismatch",
            ));
        }
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let result = if request.is_role() {
            self.load_role_definition_snapshot(&mut transaction, request)
                .await
        } else {
            match request.object.kind {
                CatalogKind::Database => {
                    self.load_database_definition_snapshot(&mut transaction, request)
                        .await
                }
                CatalogKind::Schema => {
                    self.load_schema_definition_snapshot(&mut transaction, request)
                        .await
                }
                CatalogKind::View => {
                    self.load_view_definition_snapshot(&mut transaction, request)
                        .await
                }
                CatalogKind::MaterializedView => {
                    self.load_materialized_view_definition_snapshot(&mut transaction, request)
                        .await
                }
                CatalogKind::Sequence => {
                    self.load_sequence_definition_snapshot(&mut transaction, request)
                        .await
                }
                CatalogKind::Table | CatalogKind::Column => {
                    self.load_table_definition_snapshot(&mut transaction, request)
                        .await
                }
                CatalogKind::Index => {
                    self.load_index_definition_snapshot(&mut transaction, request)
                        .await
                }
                CatalogKind::PrimaryKey
                | CatalogKind::UniqueConstraint
                | CatalogKind::ForeignKey
                | CatalogKind::CheckConstraint => {
                    self.load_constraint_definition_snapshot(&mut transaction, request)
                        .await
                }
                _ => Err(DatabaseError::configuration(
                    "unsupported catalog definition object",
                )),
            }
        };
        let rollback = transaction.rollback().await.map_err(sql_error);
        if let Err(error) = rollback {
            return result.and(Err(error));
        }
        result
    }

    pub async fn load_catalog_owner_context(
        &self,
        request: &CatalogOwnerContextRequest,
    ) -> Result<CatalogOwnerContext, DatabaseError> {
        request
            .validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        if request.connection.profile_id != self.connection_id {
            return Err(DatabaseError::configuration(
                "catalog owner context profile mismatch",
            ));
        }
        let rows = sqlx::query(Self::OWNER_CONTEXT_SQL)
            .bind(Self::owner_role_privilege(self.server_version_num))
            .fetch_all(&self.pool)
            .await
            .map_err(sql_error)?;
        let current_user = rows
            .first()
            .map(|row| row.try_get("current_user").map_err(decode_error))
            .transpose()?
            .ok_or_else(|| {
                DatabaseError::configuration("PostgreSQL owner role query returned no rows")
            })?;
        let choices = rows
            .into_iter()
            .map(|row| {
                Ok(CatalogOwnerChoice {
                    name: row.try_get("name").map_err(decode_error)?,
                    can_login: row.try_get("can_login").map_err(decode_error)?,
                    selectable: row.try_get("selectable").map_err(decode_error)?,
                    is_current: row.try_get("is_current").map_err(decode_error)?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        Ok(CatalogOwnerContext {
            current_user,
            choices,
        })
    }

    async fn load_database_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let [name] = request.object.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "database object ID has invalid shape",
            ));
        };
        let row = sqlx::query("SELECT datname AS name, pg_get_userbyid(datdba) AS owner, datistemplate AS is_template, datallowconn AS allow_connections, datconnlimit AS connection_limit, pg_encoding_to_char(encoding) AS encoding, datlocprovider::text AS locale_provider, datcollate AS collation, datctype AS ctype, COALESCE(pg_tablespace.spcname, 'pg_default') AS tablespace, obj_description(oid, 'pg_database') AS comment FROM pg_database LEFT JOIN pg_tablespace ON pg_tablespace.oid = dattablespace WHERE datname = $1").bind(name).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or_else(|| DatabaseError::configuration("database catalog entry was not found"))?;
        let definition = DatabaseDefinition {
            name: row.try_get("name").map_err(decode_error)?,
            owner: row.try_get("owner").map_err(decode_error)?,
            template: "template0".into(),
            encoding: row.try_get("encoding").map_err(decode_error)?,
            locale_provider: row.try_get("locale_provider").map_err(decode_error)?,
            locale: String::new(),
            collation: row.try_get("collation").map_err(decode_error)?,
            ctype: row.try_get("ctype").map_err(decode_error)?,
            tablespace: row.try_get("tablespace").map_err(decode_error)?,
            connection_limit: row.try_get("connection_limit").map_err(decode_error)?,
            allow_connections: row.try_get("allow_connections").map_err(decode_error)?,
            is_template: row.try_get("is_template").map_err(decode_error)?,
            comment: OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
            baseline_fingerprint: format!("sha256:{:x}", Sha256::digest(name.as_bytes())),
        };
        Ok(CatalogObjectDefinition::Database(definition))
    }

    async fn load_role_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let name = request
            .principal
            .as_ref()
            .map(|principal| principal.name.as_str())
            .or_else(|| request.object.native_path.get(1).map(String::as_str));
        let Some(name) = name else {
            return Err(DatabaseError::configuration(
                "role object ID has invalid shape",
            ));
        };
        let row = sqlx::query("SELECT oid::text AS oid, rolname, rolsuper, rolcreatedb, rolcreaterole, rolinherit, rolreplication, rolbypassrls, rolcanlogin, rolconnlimit, rolvaliduntil::text, obj_description(oid, 'pg_authid') AS comment FROM pg_roles WHERE rolname = $1")
            .bind(name).fetch_optional(&mut *connection).await.map_err(sql_error)?
            .ok_or_else(|| DatabaseError::configuration("role catalog entry was not found"))?;
        let memberships = sqlx::query_scalar::<_, String>("SELECT granted_role.rolname FROM pg_auth_members m JOIN pg_roles member_role ON member_role.oid = m.member JOIN pg_roles granted_role ON granted_role.oid = m.roleid WHERE member_role.rolname = $1 ORDER BY granted_role.rolname COLLATE \"C\"")
            .bind(name).fetch_all(&mut *connection).await.map_err(sql_error)?;
        let oid: String = row.try_get("oid").map_err(decode_error)?;
        if let Some(principal) = &request.principal
            && principal.id.native_id != oid
        {
            return Err(DatabaseError::configuration("principal identity changed"));
        }
        let mut fingerprint = Sha256::new();
        for value in [
            oid.as_str(),
            row.try_get::<String, _>("rolname")
                .map_err(decode_error)?
                .as_str(),
            row.try_get::<bool, _>("rolsuper")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
            row.try_get::<bool, _>("rolcreatedb")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
            row.try_get::<bool, _>("rolcreaterole")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
            row.try_get::<bool, _>("rolinherit")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
            row.try_get::<bool, _>("rolreplication")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
            row.try_get::<bool, _>("rolbypassrls")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
            row.try_get::<bool, _>("rolcanlogin")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
            row.try_get::<i32, _>("rolconnlimit")
                .map_err(decode_error)?
                .to_string()
                .as_str(),
        ] {
            fingerprint.update(value.as_bytes());
            fingerprint.update([0]);
        }
        for member in &memberships {
            fingerprint.update(member.as_bytes());
            fingerprint.update([0]);
        }
        Ok(CatalogObjectDefinition::Role(RoleDefinition {
            name: row.try_get("rolname").map_err(decode_error)?,
            login: row.try_get("rolcanlogin").map_err(decode_error)?,
            superuser: row.try_get("rolsuper").map_err(decode_error)?,
            createdb: row.try_get("rolcreatedb").map_err(decode_error)?,
            createrole: row.try_get("rolcreaterole").map_err(decode_error)?,
            inherit: row.try_get("rolinherit").map_err(decode_error)?,
            replication: row.try_get("rolreplication").map_err(decode_error)?,
            bypass_rls: row.try_get("rolbypassrls").map_err(decode_error)?,
            connection_limit: row.try_get("rolconnlimit").map_err(decode_error)?,
            valid_until: OptionalMetadata::Supported(
                row.try_get("rolvaliduntil").map_err(decode_error)?,
            ),
            memberships,
            comment: OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
            baseline_fingerprint: format!("sha256:{:x}", fingerprint.finalize()),
        }))
    }

    async fn load_table_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let path = &request.object.native_path;
        if path.len() < 3 {
            return Err(DatabaseError::configuration(
                "table object ID has invalid shape",
            ));
        }
        let database = &path[0];
        let schema = &path[1];
        let name = &path[2];
        let relation_oid = path.get(3).and_then(|value| value.parse::<u32>().ok());
        let row = sqlx::query(
            "SELECT current_database() AS database, c.oid::int4 AS oid, c.relname AS name, n.nspname AS schema, r.rolname AS owner, obj_description(c.oid, 'pg_class') AS comment FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace JOIN pg_roles r ON r.oid = c.relowner WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r','p')"
        )
        .bind(schema)
        .bind(name)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| DatabaseError::configuration("table catalog entry was not found"))?;
        let current: String = row.try_get("database").map_err(decode_error)?;
        if current != *database || request.target.database != current {
            return Err(DatabaseError::configuration(
                "table target database does not match object ID",
            ));
        }
        let oid: u32 = u32::try_from(row.try_get::<i32, _>("oid").map_err(decode_error)?)
            .map_err(|_| catalog_internal("invalid PostgreSQL relation OID"))?;
        if relation_oid.is_some_and(|expected| expected != oid) {
            return Err(DatabaseError::configuration("table object ID is stale"));
        }
        let owner: String = row.try_get("owner").map_err(decode_error)?;
        let comment = OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?);
        let rows = sqlx::query(
            "SELECT a.attnum::int AS ordinal_position, a.attname AS name, format_type(a.atttypid,a.atttypmod) AS native_type, NOT a.attnotnull AS nullable, pg_get_expr(d.adbin,d.adrelid) AS expression, a.attidentity::text AS identity_kind, a.attgenerated::text AS generated_kind, col_description(a.attrelid,a.attnum) AS comment, CASE WHEN a.attcollation <> 0 THEN coll_ns.nspname || '.' || coll.collname END AS collation FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum LEFT JOIN pg_collation coll ON coll.oid=a.attcollation LEFT JOIN pg_namespace coll_ns ON coll_ns.oid=coll.collnamespace WHERE a.attrelid=$1::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum"
        )
        .bind(oid as i64)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut columns = Vec::with_capacity(rows.len());
        for row in rows {
            let generated = row
                .try_get::<String, _>("generated_kind")
                .map_err(decode_error)?;
            let expression: Option<String> = row.try_get("expression").map_err(decode_error)?;
            columns.push(ColumnDefinition {
                name: row.try_get("name").map_err(decode_error)?,
                ordinal_position: u32::try_from(
                    row.try_get::<i32, _>("ordinal_position")
                        .map_err(decode_error)?,
                )
                .map_err(|_| catalog_internal("invalid PostgreSQL column ordinal"))?,
                native_type: row.try_get("native_type").map_err(decode_error)?,
                nullable: row.try_get("nullable").map_err(decode_error)?,
                default_expression: OptionalMetadata::Supported(if generated.is_empty() {
                    expression.clone()
                } else {
                    None
                }),
                identity: OptionalMetadata::Supported(Some(
                    !row.try_get::<String, _>("identity_kind")
                        .map_err(decode_error)?
                        .is_empty(),
                )),
                generated_expression: OptionalMetadata::Supported(if generated.is_empty() {
                    None
                } else {
                    expression
                }),
                collation: OptionalMetadata::Supported(
                    row.try_get("collation").map_err(decode_error)?,
                ),
                comment: OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
            });
        }
        let definition = TableDefinition {
            database: current,
            schema: schema.clone(),
            name: name.clone(),
            owner,
            comment,
            columns,
            indexes: Vec::new(),
            constraints: Vec::new(),
            structured_constraints: Vec::new(),
            baseline_fingerprint: format!(
                "sha256:{:x}",
                Sha256::digest(format!("{schema}.{name}:{oid}").as_bytes())
            ),
        };
        Ok(CatalogObjectDefinition::Table(definition))
    }

    async fn load_sequence_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let [database, schema, name, oid] = request.object.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "sequence object ID has invalid shape",
            ));
        };
        let row = sqlx::query("SELECT current_database() AS database, c.oid::int8 AS oid, n.nspname AS schema, c.relname AS name, r.rolname AS owner, obj_description(c.oid, 'pg_class') AS comment, format_type(s.seqtypid, NULL) AS data_type, s.seqincrement::text AS increment, s.seqmin::text AS min_value, s.seqmax::text AS max_value, s.seqstart::text AS start_value, s.seqcache::text AS cache, s.seqcycle AS cycle, own_ns.nspname AS owned_schema, own_rel.relname AS owned_table, own_col.attname AS owned_column FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_roles r ON r.oid=c.relowner JOIN pg_sequence s ON s.seqrelid=c.oid LEFT JOIN pg_depend dep ON dep.objid=c.oid AND dep.deptype='a' LEFT JOIN pg_class own_rel ON own_rel.oid=dep.refobjid LEFT JOIN pg_namespace own_ns ON own_ns.oid=own_rel.relnamespace LEFT JOIN pg_attribute own_col ON own_col.attrelid=own_rel.oid AND own_col.attnum=dep.refobjsubid WHERE c.oid=$1::oid AND c.relkind='S' AND n.nspname=$2 AND c.relname=$3")
            .bind(oid).bind(schema).bind(name).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or_else(|| DatabaseError::configuration("sequence catalog entry was not found"))?;
        let current: String = row.try_get("database").map_err(decode_error)?;
        if current != *database || request.target.database != current {
            return Err(DatabaseError::configuration(
                "sequence target database does not match object ID",
            ));
        }
        let parse_bound = |value: String, default: &str| {
            if value == default {
                crate::db::catalog_mutation::SequenceBound::NoLimit
            } else {
                crate::db::catalog_mutation::SequenceBound::Value(value)
            }
        };
        let owned_schema: Option<String> = row.try_get("owned_schema").map_err(decode_error)?;
        let definition = crate::db::catalog_mutation::SequenceDefinition {
            database: current,
            schema: schema.clone(),
            name: name.clone(),
            owner: row.try_get("owner").map_err(decode_error)?,
            comment: OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
            data_type: row.try_get("data_type").map_err(decode_error)?,
            increment: row.try_get("increment").map_err(decode_error)?,
            min_value: parse_bound(row.try_get("min_value").map_err(decode_error)?, "1"),
            max_value: parse_bound(
                row.try_get("max_value").map_err(decode_error)?,
                "9223372036854775807",
            ),
            start_value: row.try_get("start_value").map_err(decode_error)?,
            cache: row.try_get("cache").map_err(decode_error)?,
            cycle: row.try_get("cycle").map_err(decode_error)?,
            owned_by: owned_schema.map(|s| {
                (
                    s,
                    row.try_get("owned_table").ok().unwrap_or_default(),
                    row.try_get("owned_column").ok().unwrap_or_default(),
                )
            }),
            baseline_fingerprint: format!(
                "sha256:{:x}",
                Sha256::digest(format!("{database}.{schema}.{name}.{oid}").as_bytes())
            ),
        };
        Ok(CatalogObjectDefinition::Sequence(definition))
    }

    async fn load_index_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let [database, schema, relation, _relation_oid, oid] =
            request.object.native_path.as_slice()
        else {
            return Err(DatabaseError::configuration(
                "index object ID has invalid shape",
            ));
        };
        let row = sqlx::query("SELECT current_database() AS database, i.indexrelid::int8 AS oid, ic.relname AS name, c.relname AS relation, c.relkind::text AS relation_kind, n.nspname AS schema, i.indisunique AS unique, am.amname AS access_method, pg_get_expr(i.indpred, i.indrelid) AS predicate, ts.spcname AS tablespace, i.indnkeyatts::int AS key_count FROM pg_index i JOIN pg_class c ON c.oid=i.indrelid JOIN pg_class ic ON ic.oid=i.indexrelid JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_am am ON am.oid=ic.relam LEFT JOIN pg_tablespace ts ON ts.oid=ic.reltablespace WHERE n.nspname=$1 AND c.relname=$2 AND i.indexrelid=$3::oid AND c.relkind IN ('r','p','m')")
            .bind(schema).bind(relation).bind(oid).fetch_optional(&mut *connection).await.map_err(sql_error)?
            .ok_or_else(|| DatabaseError::configuration("index catalog entry was not found"))?;
        let current: String = row.try_get("database").map_err(decode_error)?;
        if current != *database
            || request.target.database != current
            || oid.parse::<i64>().ok() != row.try_get("oid").ok()
        {
            return Err(DatabaseError::configuration("index object ID is stale"));
        }
        let key_count: i32 = row.try_get("key_count").map_err(decode_error)?;
        let parts = sqlx::query("SELECT k.ord::int AS ordinal, a.attname AS column_name, pg_get_indexdef(i.indexrelid, k.ord::int, true) AS expression, (i.indoption[k.ord-1] & 1) <> 0 AS descending, (i.indoption[k.ord-1] & 2) <> 0 AS nulls_first FROM pg_index i CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) LEFT JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.attnum WHERE i.indexrelid=$1::oid ORDER BY k.ord")
            .bind(oid).fetch_all(&mut *connection).await.map_err(sql_error)?;
        let mut columns = Vec::new();
        let mut include_columns = Vec::new();
        for part in parts {
            let ordinal: i32 = part.try_get("ordinal").map_err(decode_error)?;
            let expression: String = part.try_get("expression").map_err(decode_error)?;
            let column_name: Option<String> = part.try_get("column_name").map_err(decode_error)?;
            let is_expression = column_name.is_none();
            if ordinal <= key_count {
                columns.push(IndexColumnDefinition {
                    expression: column_name.unwrap_or(expression),
                    descending: part.try_get("descending").map_err(decode_error)?,
                    nulls_first: part.try_get("nulls_first").map_err(decode_error)?,
                    is_expression,
                });
            } else if let Some(column) = column_name {
                include_columns.push(column);
            }
        }
        let predicate: Option<String> = row.try_get("predicate").map_err(decode_error)?;
        let tablespace: Option<String> = row.try_get("tablespace").map_err(decode_error)?;
        let mut hasher = Sha256::new();
        let name: String = row.try_get("name").map_err(decode_error)?;
        let relation_kind = match row
            .try_get::<String, _>("relation_kind")
            .map_err(decode_error)?
            .as_str()
        {
            "m" => CatalogKind::MaterializedView,
            "v" => CatalogKind::View,
            _ => CatalogKind::Table,
        };
        hasher.update(format!("{database}.{schema}.{relation}.{name}.{oid}").as_bytes());
        Ok(CatalogObjectDefinition::Index(IndexDefinition {
            database: current,
            schema: schema.clone(),
            relation: relation.clone(),
            relation_kind,
            name,
            unique: row.try_get("unique").map_err(decode_error)?,
            access_method: row.try_get("access_method").map_err(decode_error)?,
            columns,
            include_columns,
            predicate: OptionalMetadata::Supported(predicate),
            tablespace: OptionalMetadata::Supported(tablespace),
            baseline_fingerprint: format!("sha256:{:x}", hasher.finalize()),
        }))
    }

    async fn load_constraint_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let [database, schema, relation, relation_oid, constraint_oid] =
            request.object.native_path.as_slice()
        else {
            return Err(DatabaseError::configuration(
                "constraint object ID has invalid shape",
            ));
        };
        let row = sqlx::query("SELECT current_database() AS database, con.conname AS name, con.contype::text AS kind, con.condeferrable AS deferrable, con.condeferred AS initially_deferred, con.convalidated AS validated, con.connoinherit AS no_inherit, pg_get_expr(con.conbin, con.conrelid, true) AS check_expression, ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord) JOIN pg_attribute a ON a.attrelid=con.conrelid AND a.attnum=k.attnum ORDER BY k.ord) AS columns, target_ns.nspname AS referenced_schema, target.relname AS referenced_relation, ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord) JOIN pg_attribute a ON a.attrelid=con.confrelid AND a.attnum=k.attnum ORDER BY k.ord) AS referenced_columns, obj_description(con.oid, 'pg_constraint') AS comment FROM pg_constraint con JOIN pg_class c ON c.oid=con.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_class target ON target.oid=con.confrelid LEFT JOIN pg_namespace target_ns ON target_ns.oid=target.relnamespace WHERE con.conrelid=$1::oid AND con.oid=$2::oid AND n.nspname=$3") .bind(relation_oid).bind(constraint_oid).bind(schema).fetch_optional(&mut *connection).await.map_err(sql_error)?.ok_or_else(|| DatabaseError::configuration("constraint catalog entry was not found"))?;
        let current: String = row.try_get("database").map_err(decode_error)?;
        if current != *database || request.target.database != current {
            return Err(DatabaseError::configuration(
                "constraint target database does not match object ID",
            ));
        }
        let code: String = row.try_get("kind").map_err(decode_error)?;
        let expected_kind = match request.object.kind {
            CatalogKind::PrimaryKey => "p",
            CatalogKind::UniqueConstraint => "u",
            CatalogKind::ForeignKey => "f",
            CatalogKind::CheckConstraint => "c",
            _ => {
                return Err(DatabaseError::configuration(
                    "constraint request has invalid kind",
                ));
            }
        };
        if code != expected_kind {
            return Err(DatabaseError::configuration(
                "constraint object kind is stale",
            ));
        }
        let columns: Vec<String> = row.try_get("columns").map_err(decode_error)?;
        let fk_options = sqlx::query("SELECT con.confmatchtype::text AS match_type, con.confupdtype::text AS update_action, con.confdeltype::text AS delete_action FROM pg_constraint con WHERE con.conrelid=$1::oid AND con.oid=$2::oid")
            .bind(relation_oid)
            .bind(constraint_oid)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let kind = match code.as_str() {
            "p" => ConstraintDefinitionKind::PrimaryKey { columns },
            "u" => ConstraintDefinitionKind::Unique { columns },
            "f" => ConstraintDefinitionKind::ForeignKey {
                columns,
                referenced_schema: row.try_get("referenced_schema").map_err(decode_error)?,
                referenced_relation: row.try_get("referenced_relation").map_err(decode_error)?,
                referenced_columns: row.try_get("referenced_columns").map_err(decode_error)?,
                match_type: pg_fk_match(fk_options.try_get("match_type").map_err(decode_error)?),
                on_update: pg_fk_action(fk_options.try_get("update_action").map_err(decode_error)?),
                on_delete: pg_fk_action(fk_options.try_get("delete_action").map_err(decode_error)?),
            },
            "c" => ConstraintDefinitionKind::Check {
                expression: row.try_get("check_expression").map_err(decode_error)?,
                no_inherit: row.try_get("no_inherit").map_err(decode_error)?,
            },
            _ => return Err(catalog_internal("unexpected PostgreSQL constraint type")),
        };
        let name: String = row.try_get("name").map_err(decode_error)?;
        Ok(CatalogObjectDefinition::Constraint(ConstraintDefinition {
            database: current,
            schema: schema.clone(),
            relation: relation.clone(),
            relation_kind: CatalogKind::Table,
            name,
            kind,
            deferrable: row.try_get("deferrable").map_err(decode_error)?,
            initially_deferred: row.try_get("initially_deferred").map_err(decode_error)?,
            validated: row.try_get("validated").map_err(decode_error)?,
            comment: OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
            baseline_fingerprint: format!(
                "sha256:{:x}",
                Sha256::digest(
                    format!("{database}.{schema}.{relation}.{constraint_oid}").as_bytes()
                )
            ),
        }))
    }

    async fn load_schema_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let [database, schema] = request.object.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "schema object ID has invalid shape",
            ));
        };
        let row = sqlx::query(
            "SELECT current_database() AS database, n.nspname AS name, r.rolname AS owner, obj_description(n.oid, 'pg_namespace') AS comment FROM pg_namespace n JOIN pg_roles r ON r.oid = n.nspowner WHERE n.nspname = $1 AND n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\'"
        ).bind(schema).fetch_optional(&mut *connection).await.map_err(sql_error)?;
        let Some(row) = row else {
            return Err(DatabaseError::configuration(
                "schema catalog entry was not found",
            ));
        };
        let current: String = row.try_get("database").map_err(decode_error)?;
        if current != *database || request.target.database != current {
            return Err(DatabaseError::configuration(
                "schema target database does not match object ID",
            ));
        }
        let name: String = row.try_get("name").map_err(decode_error)?;
        let owner: String = row.try_get("owner").map_err(decode_error)?;
        let comment: OptionalMetadata<String> =
            OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?);
        let mut hasher = Sha256::new();
        hasher.update(b"postgres-schema-definition\0");
        for value in [&current, &name, &owner] {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
        match &comment {
            OptionalMetadata::Supported(Some(value)) => {
                hasher.update([1]);
                hasher.update((value.len() as u64).to_be_bytes());
                hasher.update(value.as_bytes());
            }
            OptionalMetadata::Supported(None) => hasher.update([0]),
            OptionalMetadata::Unsupported => hasher.update([2]),
        }
        Ok(CatalogObjectDefinition::Schema(SchemaDefinition {
            database: current,
            name,
            owner,
            comment,
            baseline_fingerprint: format!("sha256:{:x}", hasher.finalize()),
        }))
    }

    async fn load_view_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let [database, schema, name, oid] = request.object.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "view object ID has invalid shape",
            ));
        };
        let row = sqlx::query("SELECT current_database() AS database, c.oid::int8 AS oid, n.nspname AS schema, c.relname AS name, r.rolname AS owner, pg_get_viewdef(c.oid, true) AS definition, obj_description(c.oid, 'pg_class') AS comment, c.reloptions, v.check_option FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_roles r ON r.oid=c.relowner JOIN information_schema.views v ON v.table_schema=n.nspname AND v.table_name=c.relname WHERE c.oid=$1::oid AND c.relkind='v' AND n.nspname=$2")
            .bind(oid).bind(schema).fetch_optional(&mut *connection).await.map_err(sql_error)?
            .ok_or_else(|| DatabaseError::configuration("view catalog entry was not found"))?;
        let current: String = row.try_get("database").map_err(decode_error)?;
        if current != *database || request.target.database != current {
            return Err(DatabaseError::configuration(
                "view target database does not match object ID",
            ));
        }
        let definition: String = row.try_get("definition").map_err(decode_error)?;
        let options: Option<Vec<String>> = row.try_get("reloptions").map_err(decode_error)?;
        let option = |prefix: &str| {
            options
                .as_ref()
                .and_then(|values| values.iter().find_map(|v| v.strip_prefix(prefix)))
                .map(|v| v.eq_ignore_ascii_case("true"))
        };
        let output_columns = sqlx::query_scalar::<_, String>("SELECT a.attname FROM pg_attribute a WHERE a.attrelid=$1::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum").bind(oid).fetch_all(&mut *connection).await.map_err(sql_error)?;
        let mut hasher = Sha256::new();
        hasher.update(
            format!(
                "{database}\0{schema}\0{name}\0{definition}\0{:?}",
                output_columns
            )
            .as_bytes(),
        );
        Ok(CatalogObjectDefinition::View(ViewDefinition {
            database: current,
            schema: schema.clone(),
            name: name.clone(),
            owner: row.try_get("owner").map_err(decode_error)?,
            comment: OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
            query: definition,
            output_columns,
            security_barrier: if self.server_version_num >= 90_200 {
                ViewOption::available(option("security_barrier="))
            } else {
                ViewOption::unavailable("security_barrier requires PostgreSQL 9.2 or newer")
            },
            security_invoker: if self.server_version_num >= 150_000 {
                ViewOption::available(option("security_invoker="))
            } else {
                ViewOption::unavailable("security_invoker requires PostgreSQL 15 or newer")
            },
            check_option: if self.server_version_num >= 90_400 {
                ViewOption::available(row.try_get("check_option").map_err(decode_error)?)
            } else {
                ViewOption::unavailable("view check options require PostgreSQL 9.4 or newer")
            },
            baseline_fingerprint: format!("sha256:{:x}", hasher.finalize()),
            clickhouse_native: None,
        }))
    }

    async fn load_materialized_view_definition_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        let [database, schema, name, oid] = request.object.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "materialized view object ID has invalid shape",
            ));
        };
        let row = sqlx::query("SELECT current_database() AS database, c.oid::int8 AS oid, n.nspname AS schema, c.relname AS name, r.rolname AS owner, pg_get_viewdef(c.oid, true) AS definition, obj_description(c.oid, 'pg_class') AS comment, c.reloptions, c.relispopulated, ts.spcname AS tablespace FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_roles r ON r.oid=c.relowner LEFT JOIN pg_tablespace ts ON ts.oid=c.reltablespace WHERE c.oid=$1::oid AND c.relkind='m' AND n.nspname=$2")
            .bind(oid).bind(schema).fetch_optional(&mut *connection).await.map_err(sql_error)?
            .ok_or_else(|| DatabaseError::configuration("materialized view catalog entry was not found"))?;
        let current: String = row.try_get("database").map_err(decode_error)?;
        if current != *database || request.target.database != current {
            return Err(DatabaseError::configuration(
                "materialized view target database does not match object ID",
            ));
        }
        let definition: String = row.try_get("definition").map_err(decode_error)?;
        let populated: bool = row.try_get("relispopulated").map_err(decode_error)?;
        let tablespace: Option<String> = row.try_get("tablespace").map_err(decode_error)?;
        let mut hasher = Sha256::new();
        hasher.update(
            format!("{database}\0{schema}\0{name}\0{definition}\0{tablespace:?}\0{populated}")
                .as_bytes(),
        );
        Ok(CatalogObjectDefinition::MaterializedView(
            MaterializedViewDefinition {
                database: current,
                schema: schema.clone(),
                name: name.clone(),
                owner: row.try_get("owner").map_err(decode_error)?,
                comment: OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
                query: definition,
                tablespace: OptionalMetadata::Supported(tablespace),
                populated,
                baseline_fingerprint: format!("sha256:{:x}", hasher.finalize()),
                clickhouse_native: None,
            },
        ))
    }

    pub async fn search_catalog(
        &self,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        request
            .validate()
            .map_err(DatabaseError::invalid_catalog_request)?;
        if request.connection.profile_id != self.connection_id {
            return Err(DatabaseError::invalid_catalog_request(
                CatalogValidationError::ProfileMismatch {
                    child_profile_id: request.connection.profile_id,
                    parent_profile_id: self.connection_id,
                },
            ));
        }

        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let result = self
            .search_catalog_snapshot(&mut transaction, request)
            .await;
        let rollback = transaction.rollback().await.map_err(sql_error);
        if let Err(rollback_error) = rollback {
            return result.and(Err(rollback_error));
        }
        result
    }

    async fn search_catalog_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let database_allowed = request.scope.allows_database(&database);
        let schemas = search_schemas(&request.scope, &database);
        let sql_limit = i64::try_from(request.limit.saturating_add(1))
            .map_err(|_| catalog_internal("PostgreSQL catalog search limit overflowed"))?;
        let (search_query, ignore_separators) = crate::db::catalog::search_query(&request.query);
        let rows = sqlx::query(SEARCH_CATALOG_SQL)
            .bind(search_query)
            .bind(schemas)
            .bind(database_allowed)
            .bind(sql_limit)
            .bind(ignore_separators)
            .bind(request.object_scope == crate::db::catalog::CatalogSearchObjectScope::AllObjects)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        let mut candidates = rows
            .into_iter()
            .map(decode_search_candidate)
            .collect::<Result<Vec<_>, _>>()?;
        let truncated = candidates.len() > request.limit;
        candidates.truncate(request.limit);

        let relation_ids = candidates
            .iter()
            .filter_map(|candidate| candidate_relation_id(self.connection_id, candidate))
            .collect::<HashSet<_>>();
        let mut children = HashMap::new();
        for relation in &relation_ids {
            let [database, schema, _, oid] = relation.native_path.as_slice() else {
                return Err(catalog_internal(
                    "invalid PostgreSQL search relation identity",
                ));
            };
            let relation_oid = oid
                .parse::<i64>()
                .map_err(|_| catalog_internal("invalid PostgreSQL search relation OID"))?;
            for child in self
                .load_relation_children_entries(
                    connection,
                    database,
                    schema,
                    relation_oid,
                    relation,
                )
                .await?
            {
                children.insert(child.id.clone(), child);
            }
        }

        let mut hits = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            hits.push(self.search_hit(&database, candidate, &children)?);
        }
        CatalogSearchPage::new(request, hits, None, truncated)
            .map_err(DatabaseError::invalid_catalog_request)
    }

    fn search_hit(
        &self,
        database: &str,
        candidate: PgSearchCandidate,
        children: &HashMap<CatalogId, CatalogEntry>,
    ) -> Result<CatalogSearchHit, DatabaseError> {
        let database_entry = CatalogEntry::database(
            CatalogId::new(self.connection_id, CatalogKind::Database, [database]),
            qualified_database(database),
            "database",
            OptionalMetadata::Supported(candidate.database_comment.clone()),
            true,
        )
        .map_err(catalog_invariant)?;
        if candidate.kind == CatalogKind::Database {
            return Ok(CatalogSearchHit {
                entry: database_entry,
                ancestors: Vec::new(),
            });
        }
        let schema = candidate
            .schema
            .as_deref()
            .ok_or_else(|| catalog_internal("PostgreSQL search object has no schema"))?;
        let schema_id = CatalogId::new(self.connection_id, CatalogKind::Schema, [database, schema]);
        let schema_entry = CatalogEntry::schema(
            schema_id.clone(),
            database_entry.id.clone(),
            qualified_schema(database, schema),
            "schema",
            OptionalMetadata::Supported(candidate.schema_comment.clone()),
            true,
        )
        .map_err(catalog_invariant)?;
        if candidate.kind == CatalogKind::Schema {
            return Ok(CatalogSearchHit {
                entry: schema_entry,
                ancestors: vec![database_entry],
            });
        }

        if candidate.kind.is_relation_child() {
            let relation = candidate_relation_entry(self.connection_id, &candidate, &schema_id)?;
            let child_id = candidate_child_id(&relation.id, &candidate)?;
            let entry = children.get(&child_id).cloned().ok_or_else(|| {
                catalog_internal("PostgreSQL search child disappeared during hydration")
            })?;
            return Ok(CatalogSearchHit {
                entry,
                ancestors: vec![database_entry, schema_entry, relation],
            });
        }

        let oid = candidate
            .oid
            .ok_or_else(|| catalog_internal("PostgreSQL search object has no OID"))?;
        let id = CatalogId::new(
            self.connection_id,
            candidate.kind,
            [
                database.to_owned(),
                schema.to_owned(),
                candidate.name.clone(),
                oid.to_string(),
            ],
        );
        let qualified = qualified_object(database, schema, &candidate.name);
        let (native_kind, expandable) = search_kind_properties(candidate.kind)?;
        let entry = if candidate.kind.is_relation() {
            CatalogEntry::relation(
                id,
                schema_id,
                qualified,
                native_kind,
                OptionalMetadata::Supported(candidate.comment),
                expandable,
            )
        } else {
            CatalogEntry::object(
                id,
                schema_id,
                qualified,
                native_kind,
                OptionalMetadata::Supported(candidate.comment),
                expandable,
            )
        }
        .map_err(catalog_invariant)?;
        Ok(CatalogSearchHit {
            entry,
            ancestors: vec![database_entry, schema_entry],
        })
    }

    async fn load_database_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let mut entries = if request.scope.allows_database(&database) {
            vec![
                CatalogEntry::database(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Database,
                        [database.clone()],
                    ),
                    qualified_database(&database),
                    "database",
                    OptionalMetadata::Supported(None),
                    true,
                )
                .map_err(catalog_invariant)?,
            ]
        } else {
            Vec::new()
        };
        let total_count = exact_count(entries.len())?;
        let next_cursor = paginate_in_memory(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_schema_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        database_id: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let database = self
            .verify_database(connection, database_id, &request.key.target)
            .await?;
        let selected = selected_schemas(request, &database);
        let rows = match selected {
            Some(schemas) => {
                sqlx::query(
                    "SELECT n.oid::bigint AS oid, n.nspname AS name, obj_description(n.oid, 'pg_namespace') AS comment \
                     FROM pg_namespace n WHERE n.nspname <> 'information_schema' \
                     AND n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\' AND n.nspname = ANY($1) \
                     ORDER BY n.nspname COLLATE \"C\", n.oid",
                )
                .bind(schemas)
                .fetch_all(&mut *connection)
                .await
            }
            None => {
                sqlx::query(
                    "SELECT n.oid::bigint AS oid, n.nspname AS name, obj_description(n.oid, 'pg_namespace') AS comment \
                     FROM pg_namespace n WHERE n.nspname <> 'information_schema' \
                     AND n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\' \
                     ORDER BY n.nspname COLLATE \"C\", n.oid",
                )
                .fetch_all(&mut *connection)
                .await
            }
        }
        .map_err(sql_error)?;
        let database_parent = CatalogId::new(
            self.connection_id,
            CatalogKind::Database,
            [database.clone()],
        );
        let mut entries = rows
            .into_iter()
            .map(|row| {
                let name: String = row.try_get("name").map_err(decode_error)?;
                let comment: Option<String> = row.try_get("comment").map_err(decode_error)?;
                CatalogEntry::schema(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Schema,
                        [database.clone(), name.clone()],
                    ),
                    database_parent.clone(),
                    qualified_schema(&database, &name),
                    "schema",
                    OptionalMetadata::Supported(comment),
                    true,
                )
                .map_err(catalog_invariant)
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let total_count = exact_count(entries.len())?;
        let next_cursor = paginate_in_memory(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_group_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        schema_id: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let (_, schema) = self
            .verify_schema(connection, schema_id, &request.key.target)
            .await?;
        let row = sqlx::query(
            "SELECT \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p'))::bigint AS tables, \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind='v')::bigint AS views, \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind='m')::bigint AS materialized_views, \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind='S')::bigint AS sequences, \
              (SELECT COUNT(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=$1 AND p.prokind='f' AND NOT EXISTS (SELECT 1 FROM pg_trigger tr WHERE tr.tgfoid=p.oid))::bigint AS functions, \
             (SELECT COUNT(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=$1 AND p.prokind='p')::bigint AS procedures, \
             (SELECT COUNT(*) FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname=$1 AND t.typtype IN ('e','d') AND t.typisdefined AND t.typelem=0)::bigint AS types",
        )
        .bind(&schema)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut summaries = Vec::new();
        for (group, column) in [
            (ObjectGroup::Tables, "tables"),
            (ObjectGroup::Views, "views"),
            (ObjectGroup::MaterializedViews, "materialized_views"),
            (ObjectGroup::Sequences, "sequences"),
            (ObjectGroup::Functions, "functions"),
            (ObjectGroup::Procedures, "procedures"),
            (ObjectGroup::Types, "types"),
        ] {
            summaries.push(CatalogGroupSummary {
                group,
                object_count: CatalogCount::Exact(non_negative_count(
                    row.try_get(column).map_err(decode_error)?,
                )?),
            });
        }
        let total_count = exact_count(summaries.len())?;
        let next_cursor = paginate_in_memory(
            &mut summaries,
            request,
            |summary| group_sort_key(summary.group).to_owned(),
            |summary| group_sort_key(summary.group).to_owned(),
        )?;
        CatalogPage::groups(request, summaries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_object_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        schema_id: &CatalogId,
        group: ObjectGroup,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, schema) = self
            .verify_schema(connection, schema_id, &request.key.target)
            .await?;
        let (source, predicate, kind, native_kind, expandable) = match group {
            ObjectGroup::Tables => (
                "pg_class c",
                "c.relkind IN ('r','p')",
                CatalogKind::Table,
                "table",
                true,
            ),
            ObjectGroup::Dictionaries => {
                return Err(DatabaseError::unsupported_catalog_target(
                    DatabaseKind::Postgres,
                    &request.key.target,
                ));
            }
            ObjectGroup::UserDefinedFunctions => {
                return Err(DatabaseError::unsupported_catalog_target(
                    DatabaseKind::Postgres,
                    &request.key.target,
                ));
            }
            ObjectGroup::Views => (
                "pg_class c",
                "c.relkind='v'",
                CatalogKind::View,
                "view",
                true,
            ),
            ObjectGroup::MaterializedViews => (
                "pg_class c",
                "c.relkind='m'",
                CatalogKind::MaterializedView,
                "materialized_view",
                true,
            ),
            ObjectGroup::Sequences => (
                "pg_class c",
                "c.relkind='S'",
                CatalogKind::Sequence,
                "sequence",
                false,
            ),
            ObjectGroup::Functions => (
                "pg_proc c",
                "c.prokind='f' AND NOT EXISTS (SELECT 1 FROM pg_trigger tr WHERE tr.tgfoid=c.oid)",
                CatalogKind::Function,
                "function",
                false,
            ),
            ObjectGroup::Procedures => (
                "pg_proc c",
                "c.prokind='p'",
                CatalogKind::Procedure,
                "procedure",
                false,
            ),
            ObjectGroup::Types => (
                "pg_type c",
                "c.typtype IN ('e','d') AND c.typisdefined AND c.typelem=0",
                CatalogKind::Type,
                "type",
                false,
            ),
            ObjectGroup::Triggers => {
                return Err(DatabaseError::unsupported_catalog_target(
                    DatabaseKind::Postgres,
                    &request.key.target,
                ));
            }
        };
        let namespace_column = match group {
            ObjectGroup::Functions | ObjectGroup::Procedures => "c.pronamespace",
            ObjectGroup::Types => "c.typnamespace",
            _ => "c.relnamespace",
        };
        let name_column = match group {
            ObjectGroup::Functions | ObjectGroup::Procedures => "c.proname",
            ObjectGroup::Types => "c.typname",
            _ => "c.relname",
        };
        let count_sql = format!(
            "SELECT COUNT(*)::bigint FROM {source} JOIN pg_namespace n ON n.oid={namespace_column} WHERE n.nspname=$1 AND {predicate}"
        );
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(count_sql))
            .bind(&schema)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let total_count = CatalogCount::Exact(non_negative_count(count)?);
        let cursor = request
            .key
            .cursor
            .as_ref()
            .map(CatalogCursor::keyset_parts)
            .transpose()
            .map_err(DatabaseError::invalid_catalog_request)?;
        let limit = page_limit(request.page_size)?;
        let description = match group {
            ObjectGroup::Functions | ObjectGroup::Procedures => "obj_description(c.oid, 'pg_proc')",
            ObjectGroup::Types => "obj_description(c.oid, 'pg_type')",
            _ => "obj_description(c.oid, 'pg_class')",
        };
        let select = format!(
            "SELECT c.oid::bigint AS oid, {name_column} AS name, {description} AS comment FROM {source} JOIN pg_namespace n ON n.oid={namespace_column} WHERE n.nspname=$1 AND {predicate}"
        );
        let rows = if let Some((sort_key, tie_breaker)) = cursor {
            let oid = tie_breaker.parse::<i64>().map_err(|_| DatabaseError::invalid_catalog_request(CatalogValidationError::MalformedCursor))?;
            let sql = format!("{select} AND (({name_column} COLLATE \"C\") > ($2 COLLATE \"C\") OR ({name_column} COLLATE \"C\") = ($2 COLLATE \"C\") AND c.oid > $3::oid) ORDER BY {name_column} COLLATE \"C\", c.oid LIMIT $4");
            sqlx::query(AssertSqlSafe(sql)).bind(&schema).bind(sort_key).bind(oid).bind(limit).fetch_all(&mut *connection).await
        } else {
            let sql = format!("{select} ORDER BY {name_column} COLLATE \"C\", c.oid LIMIT $2");
            sqlx::query(AssertSqlSafe(sql)).bind(&schema).bind(limit).fetch_all(&mut *connection).await
        }.map_err(sql_error)?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let oid: i64 = row.try_get("oid").map_err(decode_error)?;
            let name: String = row.try_get("name").map_err(decode_error)?;
            let comment: Option<String> = row.try_get("comment").map_err(decode_error)?;
            let id = CatalogId::new(
                self.connection_id,
                kind,
                [
                    database.clone(),
                    schema.clone(),
                    name.clone(),
                    oid.to_string(),
                ],
            );
            let qualified = qualified_object(&database, &schema, &name);
            let entry = if kind.is_relation() {
                CatalogEntry::relation(
                    id,
                    schema_id.clone(),
                    qualified,
                    native_kind,
                    OptionalMetadata::Supported(comment),
                    expandable,
                )
            } else {
                CatalogEntry::object(
                    id,
                    schema_id.clone(),
                    qualified,
                    native_kind,
                    OptionalMetadata::Supported(comment),
                    expandable,
                )
            }
            .map_err(catalog_invariant)?;
            entries.push(entry);
        }
        let next_cursor = finalize_keyset_page(
            &mut entries,
            request.page_size,
            |entry| entry.qualified_name.object.clone(),
            |entry| {
                entry
                    .id
                    .native_path
                    .last()
                    .and_then(|oid| oid.parse::<u64>().ok())
                    .map_or_else(String::new, |oid| format!("{oid:020}"))
            },
        )
        .map_err(catalog_invariant)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_relation_children_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        relation: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, schema, _, relation_oid, _, _) = self
            .verify_relation(connection, relation, &request.key.target)
            .await?;
        let mut entries = self
            .load_relation_children_entries(connection, &database, &schema, relation_oid, relation)
            .await?;
        let total_count = exact_count(entries.len())?;
        let next_cursor =
            paginate_in_memory(&mut entries, request, child_sort_key, child_tie_breaker)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_relation_children_entries(
        &self,
        connection: &mut PgConnection,
        database: &str,
        schema: &str,
        relation_oid: i64,
        relation: &CatalogId,
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        let indexes = self.load_pg_indexes(connection, relation_oid).await?;
        let constraints = self.load_pg_constraints(connection, relation_oid).await?;
        let mut memberships: HashMap<String, Vec<ConstraintMembership>> = HashMap::new();
        let mut entries = Vec::new();

        for index in indexes {
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Index, &index.oid.to_string()),
                    relation.clone(),
                    qualified_object(database, schema, &index.name),
                    "index",
                    OptionalMetadata::Supported(index.comment),
                    CatalogMetadata::Index(IndexMetadata {
                        columns: index.columns,
                        unique: index.unique,
                    }),
                )
                .map_err(catalog_invariant)?,
            );
        }
        for constraint in constraints {
            let id = relation_child_id(relation, constraint.kind, &constraint.oid.to_string());
            if matches!(
                constraint.kind,
                CatalogKind::PrimaryKey | CatalogKind::UniqueConstraint | CatalogKind::ForeignKey
            ) {
                add_memberships(&mut memberships, &constraint.columns, &id)?;
            }
            let metadata = match constraint.kind {
                CatalogKind::PrimaryKey => {
                    CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
                        columns: constraint.columns,
                    })
                }
                CatalogKind::UniqueConstraint => {
                    CatalogMetadata::Constraint(ConstraintMetadata::Unique {
                        columns: constraint.columns,
                    })
                }
                CatalogKind::ForeignKey => {
                    CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
                        columns: constraint.columns,
                        referenced_relation: QualifiedName {
                            database: Some(database.to_owned()),
                            schema: constraint.referenced_schema,
                            object: constraint.referenced_relation.ok_or_else(|| {
                                catalog_internal("foreign key has no referenced relation")
                            })?,
                        },
                        referenced_columns: constraint.referenced_columns,
                    })
                }
                CatalogKind::CheckConstraint => {
                    CatalogMetadata::Constraint(ConstraintMetadata::Check {
                        expression: constraint.check_expression.ok_or_else(|| {
                            catalog_internal("check constraint has no expression")
                        })?,
                    })
                }
                _ => return Err(catalog_internal("unexpected PostgreSQL constraint kind")),
            };
            entries.push(
                CatalogEntry::relation_child(
                    id,
                    relation.clone(),
                    qualified_object(database, schema, &constraint.name),
                    "constraint",
                    OptionalMetadata::Supported(constraint.comment),
                    metadata,
                )
                .map_err(catalog_invariant)?,
            );
        }

        let rows = sqlx::query(
            "SELECT a.attnum::int AS ordinal_position, a.attname AS name, format_type(a.atttypid,a.atttypmod) AS native_type, \
             t.typcategory::text AS type_family, NOT a.attnotnull AS nullable, pg_get_expr(d.adbin,d.adrelid) AS expression, \
             a.attidentity::text AS identity_kind, a.attgenerated::text AS generated_kind, col_description(a.attrelid,a.attnum) AS comment, \
             CASE WHEN a.atttypmod >= 4 AND t.typname IN ('varchar','bpchar') THEN (a.atttypmod - 4)::bigint END AS character_length, \
             CASE WHEN a.attcollation <> 0 THEN coll_ns.nspname || '.' || coll.collname END AS collation \
             FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum \
             LEFT JOIN pg_collation coll ON coll.oid=a.attcollation LEFT JOIN pg_namespace coll_ns ON coll_ns.oid=coll.collnamespace \
             WHERE a.attrelid=$1::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum"
        ).bind(relation_oid).fetch_all(&mut *connection).await.map_err(sql_error)?;
        for row in rows {
            let ordinal: i32 = row.try_get("ordinal_position").map_err(decode_error)?;
            let name: String = row.try_get("name").map_err(decode_error)?;
            let expression: Option<String> = row.try_get("expression").map_err(decode_error)?;
            let generated_kind: String = row.try_get("generated_kind").map_err(decode_error)?;
            let identity_kind: String = row.try_get("identity_kind").map_err(decode_error)?;
            let mut metadata = ColumnMetadata::new(
                u32::try_from(ordinal)
                    .map_err(|_| catalog_internal("invalid PostgreSQL column ordinal"))?,
                row.try_get::<String, _>("native_type")
                    .map_err(decode_error)?,
                row.try_get("nullable").map_err(decode_error)?,
            );
            metadata.type_family = OptionalMetadata::Supported(Some(
                row.try_get("type_family").map_err(decode_error)?,
            ));
            metadata.default_expression =
                OptionalMetadata::Supported(if generated_kind.is_empty() {
                    expression.clone()
                } else {
                    None
                });
            metadata.identity = OptionalMetadata::Supported(Some(!identity_kind.is_empty()));
            metadata.auto_increment = OptionalMetadata::Unsupported;
            metadata.generated_expression =
                OptionalMetadata::Supported(if generated_kind.is_empty() {
                    None
                } else {
                    expression
                });
            metadata.hidden = OptionalMetadata::Unsupported;
            metadata.numeric_precision = OptionalMetadata::Unsupported;
            metadata.numeric_scale = OptionalMetadata::Unsupported;
            metadata.character_maximum_length = OptionalMetadata::Supported(
                row.try_get::<Option<i64>, _>("character_length")
                    .map_err(decode_error)?
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| catalog_internal("invalid PostgreSQL character length"))?,
            );
            metadata.collation =
                OptionalMetadata::Supported(row.try_get("collation").map_err(decode_error)?);
            metadata.character_set = OptionalMetadata::Unsupported;
            metadata.constraint_memberships = memberships.remove(&name).unwrap_or_default();
            metadata.constraint_memberships.sort_by(|left, right| {
                catalog_kind_rank(left.constraint_id.kind)
                    .cmp(&catalog_kind_rank(right.constraint_id.kind))
                    .then_with(|| left.ordinal_position.cmp(&right.ordinal_position))
                    .then_with(|| {
                        left.constraint_id
                            .native_path
                            .cmp(&right.constraint_id.native_path)
                    })
            });
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Column, &ordinal.to_string()),
                    relation.clone(),
                    qualified_object(database, schema, &name),
                    "column",
                    OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
                    CatalogMetadata::Column(metadata),
                )
                .map_err(catalog_invariant)?,
            );
        }
        Ok(entries)
    }

    async fn verify_database(
        &self,
        connection: &mut PgConnection,
        database_id: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<String, DatabaseError> {
        let current: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if matches!(database_id.native_path.as_slice(), [database] if database == &current) {
            Ok(current)
        } else {
            Err(catalog_target_not_found(target))
        }
    }

    async fn verify_schema(
        &self,
        connection: &mut PgConnection,
        schema_id: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<(String, String), DatabaseError> {
        let (database, schema) = match schema_id.native_path.as_slice() {
            [database, schema] => (database.as_str(), schema.as_str()),
            _ => return Err(catalog_target_not_found(target)),
        };
        let current: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if database != current {
            return Err(catalog_target_not_found(target));
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname=$1 AND nspname <> 'information_schema' AND nspname NOT LIKE 'pg\\_%' ESCAPE '\\')",
        )
        .bind(schema)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        if exists {
            Ok((current, schema.to_owned()))
        } else {
            Err(catalog_target_not_found(target))
        }
    }

    async fn verify_relation(
        &self,
        connection: &mut PgConnection,
        relation: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<(String, String, String, i64, &'static str, bool), DatabaseError> {
        if relation.profile_id() != self.connection_id || !relation.kind.is_relation() {
            return Err(catalog_target_not_found(target));
        }
        let (database, schema, name, oid) = match relation.native_path.as_slice() {
            [database, schema, name, oid] => (
                database.as_str(),
                schema.as_str(),
                name.as_str(),
                oid.parse::<i64>()
                    .map_err(|_| catalog_target_not_found(target))?,
            ),
            _ => return Err(catalog_target_not_found(target)),
        };
        let current: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if current != database {
            return Err(catalog_target_not_found(target));
        }
        let relation_info = sqlx::query(
            "SELECT c.relkind::text AS relkind, c.relispartition, EXISTS(SELECT 1 FROM pg_inherits i WHERE i.inhrelid=c.oid OR i.inhparent=c.oid) AS has_inheritance FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=$1::oid AND n.nspname=$2 AND c.relname=$3",
        )
        .bind(oid)
        .bind(schema)
        .bind(name)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
        let Some(relation_info) = relation_info else {
            return Err(catalog_target_not_found(target));
        };
        let native_kind: String = relation_info.try_get("relkind").map_err(decode_error)?;
        let is_partition: bool = relation_info
            .try_get("relispartition")
            .map_err(decode_error)?;
        let has_inheritance: bool = relation_info
            .try_get("has_inheritance")
            .map_err(decode_error)?;
        let (expected, verified_native_kind) = match native_kind.as_str() {
            "r" | "p" => (CatalogKind::Table, "table"),
            "v" => (CatalogKind::View, "view"),
            "m" => (CatalogKind::MaterializedView, "materialized_view"),
            _ => return Err(catalog_target_not_found(target)),
        };
        if relation.kind != expected {
            return Err(catalog_target_not_found(target));
        }
        Ok((
            current,
            schema.to_owned(),
            name.to_owned(),
            oid,
            verified_native_kind,
            native_kind == "r" && !is_partition && !has_inheritance,
        ))
    }

    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &crate::model::relation::RelationPreviewOptions,
        mut page: crate::model::pagination::PageRequest,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let (_, schema, name, _, _, supports_xmin) = self
            .verify_relation(&mut connection, relation, &target)
            .await?;
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if !self.catalog_scope.allows_schema(&database, &schema) {
            return Err(catalog_target_not_found(&target));
        }
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&name));
        let mut business_sql = format!("SELECT * FROM {qualified}");
        append_preview_options(&mut business_sql, options);
        let mut base_sql = if supports_xmin {
            format!(
                "SELECT {qualified}.*, {qualified}.xmin::text AS \"__lazydb_row_version\" FROM {qualified}"
            )
        } else {
            business_sql.clone()
        };
        if supports_xmin {
            append_preview_options(&mut base_sql, options);
        }
        let total = if page.resolve_total {
            let count_sql = relation_count_sql(&business_sql);
            let count = sqlx::query_scalar::<_, i64>(AssertSqlSafe(count_sql))
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
            let total = u64::try_from(count).map_err(|_| {
                catalog_internal("PostgreSQL returned an invalid relation row count")
            })?;
            page.offset = crate::model::pagination::ResultPagination::last_offset(page.size, total);
            Some(total)
        } else {
            None
        };
        let sql = format!(
            "{base_sql} LIMIT {} OFFSET {}",
            page.size.lookahead_limit(),
            page.offset
        );
        let started = Instant::now();
        let statement = connection
            .prepare(AssertSqlSafe(sql.clone()).into_sql_str())
            .await
            .map_err(sql_error)?;
        let mut columns: Vec<ColumnMeta> = statement
            .columns()
            .iter()
            .map(|column| ColumnMeta {
                name: column.name().to_owned(),
                type_name: column.type_info().name().to_owned(),
            })
            .collect();
        if supports_xmin {
            let internal = columns.pop();
            if internal.as_ref().map(|column| column.name.as_str()) != Some("__lazydb_row_version")
                || internal
                    .as_ref()
                    .map(|column| column.type_name.to_ascii_uppercase())
                    != Some("TEXT".to_owned())
            {
                return Err(catalog_internal(
                    "PostgreSQL relation preview has an invalid row version column",
                ));
            }
        }
        let mut rows = statement
            .query()
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        let fetched_len = rows.len();
        rows.truncate(page.size.get());
        let row_versions = if supports_xmin {
            let mut versions = Vec::with_capacity(rows.len());
            for row in &rows {
                let value: String = row.try_get(row.len() - 1).map_err(decode_error)?;
                versions.push(crate::db::mutation::RowVersion::PostgresXmin(
                    value.parse().map_err(|_| {
                        catalog_internal("PostgreSQL returned an invalid row version")
                    })?,
                ));
            }
            Some(versions)
        } else {
            None
        };
        let result_set = ResultSet {
            columns,
            rows: rows
                .iter()
                .map(|row| {
                    let mut values = decode_row(row);
                    if supports_xmin {
                        values.pop();
                    }
                    values
                })
                .collect(),
            affected_rows: 0,
        };
        Ok(crate::db::RelationPreview {
            sql,
            result: QueryOutcome::from_result_set(result_set, started.elapsed(), Duration::ZERO),
            pagination: relation_pagination(page, fetched_len, total),
            row_versions,
        })
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let result = self.relation_ddl_snapshot(&mut transaction, relation).await;
        let rollback = transaction.rollback().await.map_err(sql_error);
        if let Err(rollback_error) = rollback {
            return result.and(Err(rollback_error));
        }
        result
    }

    async fn relation_ddl_snapshot(
        &self,
        connection: &mut PgConnection,
        relation: &CatalogId,
    ) -> Result<RelationDdl, DatabaseError> {
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let (database, schema, name, relation_oid, native_kind, _) =
            self.verify_relation(connection, relation, &target).await?;
        let relation_entry = CatalogEntry::relation(
            relation.clone(),
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [database.clone(), schema.clone()],
            ),
            qualified_object(&database, &schema, &name),
            native_kind,
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)?;
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: self.connection_id,
                    generation: 0,
                },
                catalog_epoch: 0,
                request_id: 0,
                target,
                cursor: None,
            },
            scope: CatalogScope::for_profile(DatabaseKind::Postgres, &database, Some(&schema)),
            page_size: RELATION_PREVIEW_LIMIT,
        };
        let mut children_entries = self
            .load_relation_children_entries(connection, &database, &schema, relation_oid, relation)
            .await?;
        let children_total = exact_count(children_entries.len())?;
        let children_cursor = paginate_in_memory(
            &mut children_entries,
            &request,
            child_sort_key,
            child_tie_breaker,
        )?;
        let children =
            CatalogPage::new(&request, children_entries, children_total, children_cursor)
                .map_err(catalog_invariant)?;
        let ddl = self
            .load_relation_ddl_parts(connection, relation_oid, &schema, &name)
            .await?;
        let sql = assemble_relation_ddl(ddl)?;
        Ok(RelationDdl {
            relation: relation_entry,
            children,
            sql,
            provenance: DdlProvenance::AdapterGenerated,
        })
    }

    async fn load_relation_ddl_parts(
        &self,
        connection: &mut PgConnection,
        relation_oid: i64,
        schema: &str,
        name: &str,
    ) -> Result<PgDdlRelation, DatabaseError> {
        let relation_row = sqlx::query(
            "SELECT c.relkind::text AS relation_kind, c.relpersistence::text AS persistence, \
             c.relispopulated AS materialized_populated, obj_description(c.oid, 'pg_class') AS comment, \
             CASE WHEN c.relkind IN ('v','m') THEN pg_get_viewdef(c.oid, true) END AS view_definition, \
             CASE WHEN c.relkind='p' THEN pg_get_partkeydef(c.oid) END AS partition_key, \
             CASE WHEN c.relispartition THEN parent_ns.nspname END AS partition_parent_schema, \
             CASE WHEN c.relispartition THEN parent.relname END AS partition_parent_name, \
             CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid, true) END AS partition_bound \
             FROM pg_class c \
             LEFT JOIN pg_inherits inh ON inh.inhrelid=c.oid \
             LEFT JOIN pg_class parent ON parent.oid=inh.inhparent \
             LEFT JOIN pg_namespace parent_ns ON parent_ns.oid=parent.relnamespace \
             WHERE c.oid=$1::oid",
        )
        .bind(relation_oid)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| catalog_internal(format!("PostgreSQL relation {schema}.{name} has no main definition")))?;
        let relation_kind: String = relation_row
            .try_get("relation_kind")
            .map_err(decode_error)?;

        let column_rows = sqlx::query(
            "SELECT a.attname AS name, format_type(a.atttypid,a.atttypmod) AS native_type, \
             pg_get_expr(d.adbin,d.adrelid) AS default_expression, a.attnotnull AS not_null, \
             a.attidentity::text AS identity_kind, a.attgenerated::text AS generated_kind, \
             col_description(a.attrelid,a.attnum) AS comment \
             FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum \
             WHERE a.attrelid=$1::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut columns = Vec::with_capacity(column_rows.len());
        let mut column_comments = Vec::new();
        for row in column_rows {
            let column_name: String = row.try_get("name").map_err(decode_error)?;
            if let Some(comment) = row.try_get("comment").map_err(decode_error)? {
                column_comments.push((column_name.clone(), comment));
            }
            columns.push(PgDdlColumn {
                name: column_name,
                native_type: row.try_get("native_type").map_err(decode_error)?,
                default_expression: row.try_get("default_expression").map_err(decode_error)?,
                not_null: row.try_get("not_null").map_err(decode_error)?,
                identity_kind: row.try_get("identity_kind").map_err(decode_error)?,
                generated_kind: row.try_get("generated_kind").map_err(decode_error)?,
            });
        }

        let constraints = sqlx::query(
            "SELECT con.conname AS name, pg_get_constraintdef(con.oid, true) AS definition \
             FROM pg_constraint con WHERE con.conrelid=$1::oid \
               AND con.contype IN ('c','f','p','u','x') \
             ORDER BY con.conname COLLATE \"C\", con.oid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get("name").map_err(decode_error)?,
                row.try_get("definition").map_err(decode_error)?,
            ))
        })
        .collect::<Result<Vec<_>, DatabaseError>>()?;

        let indexes = sqlx::query(
            "SELECT ic.relname AS name, pg_get_indexdef(idx.indexrelid) AS definition \
             FROM pg_index idx JOIN pg_class ic ON ic.oid=idx.indexrelid \
             WHERE idx.indrelid=$1::oid AND idx.indisvalid AND idx.indisready \
               AND NOT EXISTS (SELECT 1 FROM pg_constraint con WHERE con.conindid=idx.indexrelid) \
             ORDER BY ic.relname COLLATE \"C\", idx.indexrelid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get("name").map_err(decode_error)?,
                row.try_get("definition").map_err(decode_error)?,
            ))
        })
        .collect::<Result<Vec<_>, DatabaseError>>()?;

        let triggers = sqlx::query(
            "SELECT t.tgname AS name, pg_get_triggerdef(t.oid, true) AS definition \
             FROM pg_trigger t WHERE t.tgrelid=$1::oid AND NOT t.tgisinternal \
             ORDER BY t.tgname COLLATE \"C\", t.oid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get("name").map_err(decode_error)?,
                row.try_get("definition").map_err(decode_error)?,
            ))
        })
        .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(PgDdlRelation {
            schema: schema.to_owned(),
            name: name.to_owned(),
            relation_kind,
            persistence: relation_row.try_get("persistence").map_err(decode_error)?,
            view_definition: relation_row
                .try_get("view_definition")
                .map_err(decode_error)?,
            materialized_populated: relation_row
                .try_get("materialized_populated")
                .map_err(decode_error)?,
            partition_key: relation_row
                .try_get("partition_key")
                .map_err(decode_error)?,
            partition_parent: match (
                relation_row
                    .try_get("partition_parent_schema")
                    .map_err(decode_error)?,
                relation_row
                    .try_get("partition_parent_name")
                    .map_err(decode_error)?,
            ) {
                (Some(schema), Some(name)) => Some((schema, name)),
                (None, None) => None,
                _ => {
                    return Err(catalog_internal(
                        "PostgreSQL partition parent is incomplete",
                    ));
                }
            },
            partition_bound: relation_row
                .try_get("partition_bound")
                .map_err(decode_error)?,
            columns,
            constraints,
            relation_comment: relation_row.try_get("comment").map_err(decode_error)?,
            column_comments,
            indexes,
            triggers,
        })
    }

    async fn load_pg_indexes(
        &self,
        connection: &mut PgConnection,
        relation_oid: i64,
    ) -> Result<Vec<PgIndexInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT idx.indexrelid::bigint AS oid, ic.relname AS name, idx.indisunique AS is_unique, obj_description(idx.indexrelid, 'pg_class') AS comment, \
             ARRAY(SELECT pg_get_indexdef(idx.indexrelid, key.ordinality::int, true) FROM unnest(idx.indkey) WITH ORDINALITY key(attnum, ordinality) WHERE key.ordinality <= idx.indnkeyatts ORDER BY key.ordinality) AS columns \
             FROM pg_index idx JOIN pg_class ic ON ic.oid=idx.indexrelid WHERE idx.indrelid=$1::oid ORDER BY ic.relname COLLATE \"C\", idx.indexrelid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(PgIndexInfo {
                    oid: row.try_get("oid").map_err(decode_error)?,
                    name: row.try_get("name").map_err(decode_error)?,
                    columns: row.try_get("columns").map_err(decode_error)?,
                    unique: row.try_get("is_unique").map_err(decode_error)?,
                    comment: row.try_get("comment").map_err(decode_error)?,
                })
            })
            .collect()
    }

    async fn load_pg_constraints(
        &self,
        connection: &mut PgConnection,
        relation_oid: i64,
    ) -> Result<Vec<PgConstraintInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT con.oid::bigint AS oid, con.conname AS name, con.contype::text AS constraint_type, obj_description(con.oid, 'pg_constraint') AS comment, \
             ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY key(attnum, ordinality) JOIN pg_attribute a ON a.attrelid=con.conrelid AND a.attnum=key.attnum ORDER BY key.ordinality) AS columns, \
             target_ns.nspname AS referenced_schema, target.relname AS referenced_relation, \
             ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY key(attnum, ordinality) JOIN pg_attribute a ON a.attrelid=con.confrelid AND a.attnum=key.attnum ORDER BY key.ordinality) AS referenced_columns, \
             CASE WHEN con.contype='c' THEN pg_get_constraintdef(con.oid, true) END AS check_expression \
             FROM pg_constraint con LEFT JOIN pg_class target ON target.oid=con.confrelid LEFT JOIN pg_namespace target_ns ON target_ns.oid=target.relnamespace \
             WHERE con.conrelid=$1::oid AND con.contype IN ('p','u','f','c') ORDER BY con.conname COLLATE \"C\", con.oid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        rows.into_iter()
            .map(|row| {
                let native: String = row.try_get("constraint_type").map_err(decode_error)?;
                let kind = match native.as_str() {
                    "p" => CatalogKind::PrimaryKey,
                    "u" => CatalogKind::UniqueConstraint,
                    "f" => CatalogKind::ForeignKey,
                    "c" => CatalogKind::CheckConstraint,
                    _ => return Err(catalog_internal("unexpected PostgreSQL constraint type")),
                };
                Ok(PgConstraintInfo {
                    oid: row.try_get("oid").map_err(decode_error)?,
                    name: row.try_get("name").map_err(decode_error)?,
                    kind,
                    columns: row.try_get("columns").map_err(decode_error)?,
                    referenced_schema: row.try_get("referenced_schema").map_err(decode_error)?,
                    referenced_relation: row
                        .try_get("referenced_relation")
                        .map_err(decode_error)?,
                    referenced_columns: row.try_get("referenced_columns").map_err(decode_error)?,
                    check_expression: row.try_get("check_expression").map_err(decode_error)?,
                    comment: row.try_get("comment").map_err(decode_error)?,
                })
            })
            .collect()
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

fn validate_privilege(privilege: &str) -> Result<String, DatabaseError> {
    let normalized = privilege.trim().to_ascii_uppercase();
    if normalized.is_empty()
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(DatabaseError::configuration("invalid PostgreSQL privilege"));
    }
    Ok(normalized)
}

fn mutation_target_sql(
    target: PrincipalMutationTarget,
    database: Option<&str>,
) -> Result<(String, Option<String>), DatabaseError> {
    match target {
        PrincipalMutationTarget::Database => Ok((
            format!(
                "DATABASE {}",
                quote_identifier(database.ok_or_else(|| {
                    DatabaseError::configuration(
                        "database privilege mutation requires a database target",
                    )
                })?)
            ),
            None,
        )),
        PrincipalMutationTarget::Schema { schema } => {
            Ok((format!("SCHEMA {}", quote_identifier(&schema)), None))
        }
        PrincipalMutationTarget::Relation { schema, relation } => Ok((
            format!(
                "TABLE {}.{}",
                quote_identifier(&schema),
                quote_identifier(&relation)
            ),
            None,
        )),
        PrincipalMutationTarget::Column {
            schema,
            relation,
            column,
        } => Ok((
            format!(
                "TABLE {}.{}",
                quote_identifier(&schema),
                quote_identifier(&relation)
            ),
            Some(column),
        )),
    }
}

fn assemble_relation_ddl(mut relation: PgDdlRelation) -> Result<String, DatabaseError> {
    relation
        .constraints
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    relation
        .column_comments
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    relation
        .indexes
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    relation
        .triggers
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));

    let qualified = format!(
        "{}.{}",
        quote_identifier(&relation.schema),
        quote_identifier(&relation.name)
    );
    let (main_label, main_sql, comment_kind) = match relation.relation_kind.as_str() {
        "r" if relation.partition_parent.is_some() => {
            let (parent_schema, parent_name) = relation.partition_parent.as_ref().unwrap();
            let bound = relation.partition_bound.as_deref().ok_or_else(|| {
                catalog_internal(format!(
                    "PostgreSQL partition {}.{} has no partition bound",
                    relation.schema, relation.name
                ))
            })?;
            (
                "Table",
                format!(
                    "CREATE TABLE {qualified} PARTITION OF {}.{} {bound}",
                    quote_identifier(parent_schema),
                    quote_identifier(parent_name)
                ),
                "TABLE",
            )
        }
        "r" | "p" => {
            let mut definitions = relation
                .columns
                .iter()
                .map(column_definition)
                .collect::<Result<Vec<_>, _>>()?;
            definitions.extend(relation.constraints.iter().map(|(name, definition)| {
                format!("CONSTRAINT {} {definition}", quote_identifier(name))
            }));
            let persistence = match relation.persistence.as_str() {
                "p" => "",
                "u" => "UNLOGGED ",
                "t" => "TEMPORARY ",
                value => {
                    return Err(catalog_internal(format!(
                        "unexpected PostgreSQL relation persistence {value}"
                    )));
                }
            };
            let mut statement = format!(
                "CREATE {persistence}TABLE {qualified} (\n{}\n)",
                definitions
                    .iter()
                    .map(|definition| format!("  {definition}"))
                    .collect::<Vec<_>>()
                    .join(",\n")
            );
            if relation.relation_kind == "p" {
                let partition_key = relation.partition_key.as_deref().ok_or_else(|| {
                    catalog_internal(format!(
                        "PostgreSQL partitioned table {}.{} has no partition key",
                        relation.schema, relation.name
                    ))
                })?;
                statement.push_str(" PARTITION BY ");
                statement.push_str(partition_key);
            }
            ("Table", statement, "TABLE")
        }
        "v" | "m" => {
            let definition = relation
                .view_definition
                .as_deref()
                .map(str::trim)
                .filter(|definition| !definition.is_empty())
                .ok_or_else(|| {
                    catalog_internal(format!(
                        "PostgreSQL relation {}.{} has no main view definition",
                        relation.schema, relation.name
                    ))
                })?;
            if relation.relation_kind == "v" {
                (
                    "View",
                    format!("CREATE VIEW {qualified} AS\n{definition}"),
                    "VIEW",
                )
            } else {
                let no_data = if relation.materialized_populated {
                    ""
                } else {
                    "\nWITH NO DATA"
                };
                (
                    "View",
                    format!("CREATE MATERIALIZED VIEW {qualified} AS\n{definition}{no_data}"),
                    "MATERIALIZED VIEW",
                )
            }
        }
        value => {
            return Err(catalog_internal(format!(
                "unsupported PostgreSQL relation kind {value} for DDL"
            )));
        }
    };

    let mut comments = Vec::new();
    if let Some(comment) = relation.relation_comment {
        comments.push(format!(
            "COMMENT ON {comment_kind} {qualified} IS {}",
            quote_literal(&comment)
        ));
    }
    comments.extend(
        relation
            .column_comments
            .into_iter()
            .map(|(column, comment)| {
                format!(
                    "COMMENT ON COLUMN {qualified}.{} IS {}",
                    quote_identifier(&column),
                    quote_literal(&comment)
                )
            }),
    );

    assemble_ddl(vec![
        DdlSection {
            label: main_label,
            statements: vec![main_sql],
        },
        DdlSection {
            label: "Comments",
            statements: comments,
        },
        DdlSection {
            label: "Indexes",
            statements: relation.indexes.into_iter().map(|(_, sql)| sql).collect(),
        },
        DdlSection {
            label: "Triggers",
            statements: relation.triggers.into_iter().map(|(_, sql)| sql).collect(),
        },
    ])
    .ok_or_else(|| catalog_internal("PostgreSQL relation DDL assembly produced no statements"))
}

fn column_definition(column: &PgDdlColumn) -> Result<String, DatabaseError> {
    let mut definition = format!("{} {}", quote_identifier(&column.name), column.native_type);
    match (
        column.identity_kind.as_str(),
        column.generated_kind.as_str(),
    ) {
        ("a", "") => definition.push_str(" GENERATED ALWAYS AS IDENTITY"),
        ("d", "") => definition.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
        ("", "s") => {
            let expression = column.default_expression.as_deref().ok_or_else(|| {
                catalog_internal(format!(
                    "PostgreSQL generated column {} has no expression",
                    column.name
                ))
            })?;
            definition.push_str(&format!(" GENERATED ALWAYS AS ({expression}) STORED"));
        }
        ("", "") => {
            if let Some(expression) = &column.default_expression {
                definition.push_str(" DEFAULT ");
                definition.push_str(expression);
            }
        }
        (identity, generated) => {
            return Err(catalog_internal(format!(
                "unsupported PostgreSQL identity/generated combination {identity:?}/{generated:?} for column {}",
                column.name
            )));
        }
    }
    if column.not_null {
        definition.push_str(" NOT NULL");
    }
    Ok(definition)
}

fn append_preview_options(
    sql: &mut String,
    options: &crate::model::relation::RelationPreviewOptions,
) {
    if let Some(clause) = &options.where_clause {
        sql.push_str(" WHERE ");
        sql.push_str(clause);
    }
    if let Some(clause) = &options.order_by_clause {
        sql.push_str(" ORDER BY ");
        sql.push_str(clause);
    }
}

fn relation_pagination(
    page: crate::model::pagination::PageRequest,
    fetched_len: usize,
    total: Option<u64>,
) -> crate::model::pagination::ResultPagination {
    let mut pagination = crate::model::pagination::ResultPagination::from_page(page, fetched_len);
    if let Some(total) = total {
        pagination.total = crate::model::pagination::TotalRows::Exact(total);
    }
    pagination
}

fn relation_count_sql(sql: &str) -> String {
    format!("SELECT COUNT(*) FROM ({sql}) AS __lazydb_count")
}

pub struct PostgresTransactionBackend {
    connection: PoolConnection<Postgres>,
    control: PgPool,
    pid: i32,
    adapter: PostgresAdapter,
}

#[async_trait::async_trait]
impl TransactionBackend for PostgresTransactionBackend {
    async fn begin(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::begin(&mut self.connection, None)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
        self.adapter
            .execute_connection(&mut self.connection, sql)
            .await
            .map_err(Into::into)
    }
    async fn relation_mutation(
        &mut self,
        request: RelationMutationRequest,
    ) -> Result<MutationResult, TransactionError> {
        let [_, schema, relation, _] = request.relation.native_path.as_slice() else {
            return Err(TransactionError(
                "PostgreSQL relation has no canonical database, schema, table, and oid path".into(),
            ));
        };
        let columns = &request.metadata.columns;
        let quoted_table = format!(
            "{}.{}",
            quote_identifier(schema),
            quote_identifier(relation)
        );
        match request.operation {
            RelationMutation::DeleteRows(ref rows) => {
                validate_postgres_delete_relation(&mut self.connection, &request, columns).await?;
                let primary_key_columns =
                    primary_key_indexes(columns, &request.metadata.primary_key)?;
                for mutation in rows {
                    if mutation.row.columns != primary_key_columns
                        || mutation.row.values.len() != primary_key_columns.len()
                    {
                        return Err(TransactionError(
                            "PostgreSQL delete mutation is malformed".into(),
                        ));
                    }
                    let Some(crate::db::mutation::RowVersion::PostgresXmin(version)) =
                        mutation.version
                    else {
                        return Err(TransactionError(
                            "PostgreSQL delete mutation is missing a row version; refresh the relation".into(),
                        ));
                    };
                    let sql = postgres_delete_sql(&quoted_table, columns, &primary_key_columns)?;
                    let mut query = sqlx::query(AssertSqlSafe(sql));
                    for (index, value) in primary_key_columns.iter().zip(&mutation.row.values) {
                        query = bind_cell(query, value, &columns[*index].1)?;
                    }
                    query = query.bind(version.to_string());
                    let affected = query
                        .execute(&mut *self.connection)
                        .await
                        .map_err(|e| postgres_relation_error(e, "delete", &quoted_table))?
                        .rows_affected();
                    if affected == 0 {
                        return Err(TransactionError::relation(RelationMutationDiagnostic {
                            category: RelationMutationCategory::Conflict,
                            sqlstate: None,
                            operation: "delete".into(),
                            relation: quoted_table.clone(),
                            context: Some("row version did not match".into()),
                            message: "row was changed, deleted, or is no longer visible; refresh the relation".into(),
                        }));
                    }
                    if affected != 1 {
                        return Err(TransactionError(
                            "PostgreSQL delete locator matched multiple rows".into(),
                        ));
                    }
                }
                return Ok(MutationResult::Deleted { rows: rows.len() });
            }
            RelationMutation::InsertRow(ref insert) => {
                validate_postgres_mutation_relation(&mut self.connection, &request).await?;
                if insert.columns.len() != insert.values.len()
                    || insert.columns.iter().any(|i| *i >= columns.len())
                {
                    return Err(TransactionError(
                        "PostgreSQL insert mutation is malformed".into(),
                    ));
                }
                let mut supplied = Vec::new();
                let mut expressions = Vec::new();
                let mut bind_count = 1;
                for (index, value) in insert.columns.iter().zip(&insert.values) {
                    supplied.push(quote_identifier(&columns[*index].0));
                    if matches!(value, InputValue::Default) {
                        expressions.push("DEFAULT".into());
                    } else {
                        expressions.push(postgres_placeholder(
                            bind_count,
                            &columns[*index].1,
                            PgParameterMode::Assignment,
                        )?);
                        bind_count += 1;
                    }
                }
                let sql = if supplied.is_empty() {
                    format!(
                        "INSERT INTO {quoted_table} DEFAULT VALUES RETURNING *, xmin::text AS \"__lazydb_row_version\""
                    )
                } else {
                    format!(
                        "INSERT INTO {quoted_table} ({}) VALUES ({}) RETURNING *, xmin::text AS \"__lazydb_row_version\"",
                        supplied.join(", "),
                        expressions.join(", ")
                    )
                };
                let mut query = sqlx::query(AssertSqlSafe(sql));
                for (index, value) in insert.columns.iter().zip(&insert.values) {
                    match value {
                        InputValue::Default => {}
                        InputValue::Null => {
                            query = bind_cell(query, &CellValue::Null, &columns[*index].1)?
                        }
                        InputValue::Value(value) => {
                            query = bind_cell(query, value, &columns[*index].1)?
                        }
                    }
                }
                let row = query
                    .fetch_one(&mut *self.connection)
                    .await
                    .map_err(|e| postgres_relation_error(e, "insert", &quoted_table))?;
                let (row, version) = decode_mutation_returning_row(&row, columns.len())?;
                return Ok(MutationResult::Inserted {
                    row,
                    version: Some(version),
                });
            }
            RelationMutation::UpdateCell(ref update) => {
                validate_postgres_mutation_relation(&mut self.connection, &request).await?;
                let Some((column_name, _, _)) = columns.get(update.column) else {
                    return Err(TransactionError(
                        "PostgreSQL update column is out of range".into(),
                    ));
                };
                if update.row.columns.len() != update.row.values.len() {
                    return Err(TransactionError(
                        "PostgreSQL row locator is malformed".into(),
                    ));
                }
                let primary_key_columns = request
                    .metadata
                    .primary_key
                    .iter()
                    .map(|name| {
                        columns
                            .iter()
                            .position(|(column, _, _)| column == name)
                            .ok_or_else(|| {
                                TransactionError("PostgreSQL primary key column is missing".into())
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if primary_key_columns != update.row.columns {
                    return Err(TransactionError(
                        "PostgreSQL row locator must contain the primary key columns in order"
                            .into(),
                    ));
                }
                if update
                    .row
                    .columns
                    .iter()
                    .any(|index| *index >= columns.len())
                {
                    return Err(TransactionError(
                        "PostgreSQL row locator column is out of range".into(),
                    ));
                }

                let quoted_column = quote_identifier(column_name);
                let mut sql = format!("UPDATE {quoted_table} SET {quoted_column} = ");
                let mut bind_count = 1;
                match &update.value {
                    InputValue::Default => sql.push_str("DEFAULT"),
                    InputValue::Null | InputValue::Value(_) => {
                        sql.push_str(&postgres_placeholder(
                            bind_count,
                            &columns[update.column].1,
                            PgParameterMode::Assignment,
                        )?);
                        bind_count += 1;
                    }
                }
                sql.push_str(" WHERE ");
                for (position, column_index) in update.row.columns.iter().enumerate() {
                    if position > 0 {
                        sql.push_str(" AND ");
                    }
                    let name = quote_identifier(&columns[*column_index].0);
                    sql.push_str(&format!(
                        "{name} IS NOT DISTINCT FROM {}",
                        postgres_placeholder(
                            bind_count,
                            &columns[*column_index].1,
                            PgParameterMode::Equality,
                        )?
                    ));
                    bind_count += 1;
                }
                if !update.row.columns.is_empty() {
                    sql.push_str(" AND ");
                }
                sql.push_str(&format!(
                    "{quoted_column} IS NOT DISTINCT FROM {}",
                    postgres_placeholder(
                        bind_count,
                        &columns[update.column].1,
                        PgParameterMode::Equality,
                    )?
                ));
                sql.push_str(" RETURNING *, xmin::text AS \"__lazydb_row_version\"");

                let mut query = sqlx::query(AssertSqlSafe(sql));
                match &update.value {
                    InputValue::Default => {}
                    InputValue::Null => {
                        query = bind_cell(query, &CellValue::Null, &columns[update.column].1)?
                    }
                    InputValue::Value(value) => {
                        query = bind_cell(query, value, &columns[update.column].1)?
                    }
                }
                for (column_index, value) in update.row.columns.iter().zip(&update.row.values) {
                    query = bind_cell(query, value, &columns[*column_index].1)?;
                }
                query = bind_cell(query, &update.original, &columns[update.column].1)?;
                let rows = query
                    .fetch_all(&mut *self.connection)
                    .await
                    .map_err(|error| postgres_relation_error(error, "update", &quoted_table))?;
                let row = match rows.as_slice() {
                    [] => {
                        return Err(TransactionError::relation(RelationMutationDiagnostic {
                            category: RelationMutationCategory::Conflict,
                            sqlstate: None,
                            operation: "update".into(),
                            relation: quoted_table.clone(),
                            context: Some("original cell value did not match".into()),
                            message: "row was changed, deleted, or is no longer visible; refresh the relation".into(),
                        }));
                    }
                    [row] => row,
                    _ => {
                        return Err(TransactionError(
                            "PostgreSQL update mutation matched multiple rows".into(),
                        ));
                    }
                };
                let (row, version) = decode_mutation_returning_row(row, columns.len())?;
                Ok(MutationResult::Updated {
                    row,
                    version: Some(version),
                })
            }
        }
    }

    async fn commit(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::commit(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn rollback(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::rollback(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn cancel(&mut self) -> Result<(), TransactionError> {
        let cancelled = sqlx::query_scalar::<_, bool>("SELECT pg_cancel_backend($1)")
            .bind(self.pid)
            .fetch_one(&self.control)
            .await
            .map_err(|error| TransactionError(error.to_string()))?;
        if cancelled {
            Ok(())
        } else {
            Err(TransactionError(
                "PostgreSQL backend refused cancellation".into(),
            ))
        }
    }
    fn depth(&self) -> usize {
        <Postgres as sqlx::Database>::TransactionManager::get_transaction_depth(&self.connection)
    }
    fn force_close(self) -> futures_util::future::BoxFuture<'static, Result<(), TransactionError>> {
        Box::pin(async move {
            // SQLx 0.9 has no close_hard on PoolConnection. Detach first, then close the raw
            // connection so a possibly dirty session can never be returned to this pool.
            let connection = self.connection.detach();
            connection
                .close()
                .await
                .map_err(|error| TransactionError(error.to_string()))
        })
    }
}

fn postgres_delete_sql(
    quoted_table: &str,
    columns: &[(String, String, bool)],
    locator_columns: &[usize],
) -> Result<String, TransactionError> {
    if columns.is_empty() {
        return Err(TransactionError(
            "PostgreSQL delete mutation has no relation columns".into(),
        ));
    }
    if locator_columns.is_empty() {
        return Err(TransactionError(
            "PostgreSQL delete mutation has no row locator".into(),
        ));
    }
    let mut predicates = Vec::with_capacity(locator_columns.len() + 1);
    for (position, index) in locator_columns.iter().enumerate() {
        if *index >= columns.len() {
            return Err(TransactionError(
                "PostgreSQL row locator column is out of range".into(),
            ));
        }
        let name = quote_identifier(&columns[*index].0);
        predicates.push(format!(
            "{name} IS NOT DISTINCT FROM {}",
            postgres_placeholder(position + 1, &columns[*index].1, PgParameterMode::Equality)?
        ));
    }
    predicates.push(format!("xmin = ${}::xid", locator_columns.len() + 1));
    Ok(format!(
        "DELETE FROM {quoted_table} WHERE {}",
        predicates.join(" AND ")
    ))
}

fn decode_mutation_returning_row(
    row: &PgRow,
    business_column_count: usize,
) -> Result<(Vec<CellValue>, crate::db::mutation::RowVersion), TransactionError> {
    if row.len() != business_column_count + 1 {
        return Err(TransactionError(
            "PostgreSQL mutation returned an invalid row shape".into(),
        ));
    }
    let version: String = row
        .try_get(business_column_count)
        .map_err(|error| TransactionError(error.to_string()))?;
    let version = version.parse().map_err(|_| {
        TransactionError("PostgreSQL mutation returned an invalid row version".into())
    })?;
    let mut values = decode_row(row);
    values.truncate(business_column_count);
    Ok((
        values,
        crate::db::mutation::RowVersion::PostgresXmin(version),
    ))
}

fn primary_key_indexes(
    columns: &[(String, String, bool)],
    primary_key: &[String],
) -> Result<Vec<usize>, TransactionError> {
    if primary_key.is_empty() {
        return Err(TransactionError(
            "PostgreSQL deletes require a complete primary key".into(),
        ));
    }
    if primary_key.windows(2).any(|names| names[0] == names[1]) {
        return Err(TransactionError(
            "PostgreSQL primary key metadata contains duplicate columns".into(),
        ));
    }
    primary_key
        .iter()
        .map(|name| {
            columns
                .iter()
                .position(|(column, _, _)| column == name)
                .ok_or_else(|| TransactionError("PostgreSQL primary key column is missing".into()))
        })
        .collect()
}

async fn validate_postgres_delete_relation(
    connection: &mut PoolConnection<Postgres>,
    request: &RelationMutationRequest,
    columns: &[(String, String, bool)],
) -> Result<(), TransactionError> {
    if request.relation.kind != CatalogKind::Table {
        return Err(TransactionError(
            "PostgreSQL deletes are supported only for ordinary tables".into(),
        ));
    }
    let Some(oid) = request
        .relation
        .native_path
        .get(3)
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return Err(TransactionError(
            "PostgreSQL relation has an invalid table identity".into(),
        ));
    };
    let Some(schema) = request.relation.native_path.get(1) else {
        return Err(TransactionError(
            "PostgreSQL relation has an invalid schema identity".into(),
        ));
    };
    let Some(name) = request.relation.native_path.get(2) else {
        return Err(TransactionError(
            "PostgreSQL relation has an invalid table identity".into(),
        ));
    };
    let row = sqlx::query(
        "SELECT c.relkind::text, c.relispartition, EXISTS (SELECT 1 FROM pg_inherits WHERE inhrelid = c.oid), EXISTS (SELECT 1 FROM pg_inherits WHERE inhparent = c.oid) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = $1::oid AND n.nspname = $2 AND c.relname = $3",
    )
    .bind(oid as i64)
    .bind(schema)
    .bind(name)
    .fetch_optional(&mut **connection)
    .await
    .map_err(|error| TransactionError(error.to_string()))?
    .ok_or_else(|| TransactionError("PostgreSQL relation no longer exists".into()))?;
    let kind: String = row
        .try_get(0)
        .map_err(|error| TransactionError(error.to_string()))?;
    let is_partition: bool = row
        .try_get(1)
        .map_err(|error| TransactionError(error.to_string()))?;
    let has_parent: bool = row
        .try_get(2)
        .map_err(|error| TransactionError(error.to_string()))?;
    let has_children: bool = row
        .try_get(3)
        .map_err(|error| TransactionError(error.to_string()))?;
    if kind != "r" || is_partition || has_parent || has_children {
        return Err(TransactionError(
            "PostgreSQL deletes are supported only for independent ordinary tables".into(),
        ));
    }
    let _ = primary_key_indexes(columns, &request.metadata.primary_key)?;
    Ok(())
}

async fn validate_postgres_mutation_relation(
    connection: &mut PoolConnection<Postgres>,
    request: &RelationMutationRequest,
) -> Result<(), TransactionError> {
    if request.relation.kind != CatalogKind::Table {
        return Err(TransactionError(
            "PostgreSQL relation mutations are supported only for ordinary tables".into(),
        ));
    }
    let Some(oid) = request
        .relation
        .native_path
        .get(3)
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return Err(TransactionError(
            "PostgreSQL relation has an invalid table identity".into(),
        ));
    };
    let schema = request.relation.native_path.get(1).ok_or_else(|| {
        TransactionError("PostgreSQL relation has an invalid schema identity".into())
    })?;
    let name = request.relation.native_path.get(2).ok_or_else(|| {
        TransactionError("PostgreSQL relation has an invalid table identity".into())
    })?;
    let row = sqlx::query(
        "SELECT c.relkind::text, c.relispartition, EXISTS (SELECT 1 FROM pg_inherits WHERE inhrelid = c.oid), EXISTS (SELECT 1 FROM pg_inherits WHERE inhparent = c.oid) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = $1::oid AND n.nspname = $2 AND c.relname = $3",
    )
    .bind(oid as i64)
    .bind(schema)
    .bind(name)
    .fetch_optional(&mut **connection)
    .await
    .map_err(|error| TransactionError(error.to_string()))?
    .ok_or_else(|| TransactionError("PostgreSQL relation no longer exists".into()))?;
    let kind: String = row
        .try_get(0)
        .map_err(|error| TransactionError(error.to_string()))?;
    let is_partition: bool = row
        .try_get(1)
        .map_err(|error| TransactionError(error.to_string()))?;
    let has_parent: bool = row
        .try_get(2)
        .map_err(|error| TransactionError(error.to_string()))?;
    let has_children: bool = row
        .try_get(3)
        .map_err(|error| TransactionError(error.to_string()))?;
    if kind != "r" || is_partition || has_parent || has_children {
        return Err(TransactionError(
            "PostgreSQL relation mutations are supported only for independent ordinary tables"
                .into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PgParameterMode {
    Assignment,
    Equality,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PgParameterType {
    Boolean,
    Smallint,
    Integer,
    Bigint,
    Real,
    Double,
    Numeric,
    Text,
    Varchar,
    Char,
    Date,
    Time,
    Timetz,
    Timestamp,
    Timestamptz,
    Interval,
    Uuid,
    Json,
    Jsonb,
    TextArray,
    VarcharArray,
    CharArray,
    NameArray,
    Inet,
    Cidr,
    Point,
    Bytea,
}

impl PgParameterType {
    fn cast_name(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Smallint => "smallint",
            Self::Integer => "integer",
            Self::Bigint => "bigint",
            Self::Real => "real",
            Self::Double => "double precision",
            Self::Numeric => "numeric",
            Self::Text => "text",
            Self::Varchar => "varchar",
            Self::Char => "char",
            Self::Date => "date",
            Self::Time => "time",
            Self::Timetz => "timetz",
            Self::Timestamp => "timestamp",
            Self::Timestamptz => "timestamptz",
            Self::Interval => "interval",
            Self::Uuid => "uuid",
            Self::Json => "json",
            Self::Jsonb => "jsonb",
            Self::TextArray => "text[]",
            Self::VarcharArray => "varchar[]",
            Self::CharArray => "char[]",
            Self::NameArray => "name[]",
            Self::Inet => "inet",
            Self::Cidr => "cidr",
            Self::Point => "point",
            Self::Bytea => "bytea",
        }
    }
}

fn postgres_parameter_type(
    type_name: &str,
    mode: PgParameterMode,
) -> Result<PgParameterType, TransactionError> {
    let normalized = type_name.trim().to_ascii_lowercase();
    let (base, modifier) = if let Some(open) = normalized.find('(') {
        let Some(close) = normalized[open + 1..].find(')') else {
            return Err(TransactionError(
                "PostgreSQL column type is malformed".into(),
            ));
        };
        let close = open + close + 1;
        let modifier = normalized[open + 1..close].trim();
        if modifier.is_empty() || modifier.contains(['(', ')']) {
            return Err(TransactionError(
                "PostgreSQL column type modifier is malformed".into(),
            ));
        }
        let base = format!("{}{}", &normalized[..open], &normalized[close + 1..]);
        (base.trim().to_owned(), Some(modifier.to_owned()))
    } else {
        (normalized.trim().to_owned(), None)
    };
    if base.is_empty() || base.contains(')') || base.split_whitespace().count() > 4 {
        return Err(TransactionError(
            "PostgreSQL column type is unsupported".into(),
        ));
    }
    if let Some(modifier) = modifier {
        let valid = match base.as_str() {
            "numeric" | "decimal" => modifier
                .split(',')
                .all(|part| part.trim().parse::<u16>().is_ok()),
            "varchar" | "character varying" | "char" | "character" => {
                modifier.parse::<u32>().is_ok()
            }
            "timestamp"
            | "timestamp without time zone"
            | "timestamptz"
            | "timestamp with time zone"
            | "time"
            | "time without time zone"
            | "timetz"
            | "time with time zone" => modifier.parse::<u32>().is_ok(),
            _ => false,
        };
        if !valid {
            return Err(TransactionError(
                "PostgreSQL column type modifier is unsupported".into(),
            ));
        }
    }
    let parameter_type = match base.as_str() {
        "bool" | "boolean" => PgParameterType::Boolean,
        "int2" | "smallint" => PgParameterType::Smallint,
        "int4" | "integer" | "serial" => PgParameterType::Integer,
        "int8" | "bigint" | "bigserial" => PgParameterType::Bigint,
        "float4" | "real" => PgParameterType::Real,
        "float8" | "double precision" => PgParameterType::Double,
        "numeric" | "decimal" => PgParameterType::Numeric,
        "text" => PgParameterType::Text,
        "varchar" | "character varying" => PgParameterType::Varchar,
        "char" | "character" => PgParameterType::Char,
        "date" => PgParameterType::Date,
        "time" | "time without time zone" => PgParameterType::Time,
        "timetz" | "time with time zone" => PgParameterType::Timetz,
        "timestamp" | "timestamp without time zone" => PgParameterType::Timestamp,
        "timestamptz" | "timestamp with time zone" => PgParameterType::Timestamptz,
        "interval" => PgParameterType::Interval,
        "uuid" => PgParameterType::Uuid,
        "json" => PgParameterType::Json,
        "jsonb" => PgParameterType::Jsonb,
        "text[]" => PgParameterType::TextArray,
        "varchar[]" | "character varying[]" => PgParameterType::VarcharArray,
        "char[]" | "character[]" => PgParameterType::CharArray,
        "name[]" => PgParameterType::NameArray,
        "inet" => PgParameterType::Inet,
        "cidr" => PgParameterType::Cidr,
        "point" => PgParameterType::Point,
        "bytea" => PgParameterType::Bytea,
        _ => {
            return Err(TransactionError(
                "PostgreSQL column type is unsupported".into(),
            ));
        }
    };
    if mode == PgParameterMode::Equality
        && matches!(
            parameter_type,
            PgParameterType::Json | PgParameterType::Jsonb | PgParameterType::Point
        )
    {
        return Err(TransactionError(
            "PostgreSQL type does not support relation equality comparison".into(),
        ));
    }
    Ok(parameter_type)
}

fn postgres_placeholder(
    index: usize,
    type_name: &str,
    mode: PgParameterMode,
) -> Result<String, TransactionError> {
    Ok(format!(
        "${index}::{}",
        postgres_parameter_type(type_name, mode)?.cast_name()
    ))
}

fn selected_schemas<'a>(request: &'a CatalogRequest, database: &str) -> Option<&'a Vec<String>> {
    match &request.scope.databases {
        CatalogSelection::All => None,
        CatalogSelection::Selected(databases) => databases
            .iter()
            .find(|selected| selected.name == database)
            .and_then(|selected| match &selected.schemas {
                CatalogSelection::All => None,
                CatalogSelection::Selected(schemas) => Some(schemas),
            }),
    }
}

fn search_schemas(scope: &CatalogScope, database: &str) -> Option<Vec<String>> {
    match &scope.databases {
        CatalogSelection::All => None,
        CatalogSelection::Selected(databases) => databases
            .iter()
            .find(|selected| selected.name == database)
            .map_or_else(
                || Some(Vec::new()),
                |selected| match &selected.schemas {
                    CatalogSelection::All => None,
                    CatalogSelection::Selected(schemas) => Some(schemas.clone()),
                },
            ),
    }
}

fn decode_search_candidate(row: PgRow) -> Result<PgSearchCandidate, DatabaseError> {
    let native_kind: String = row.try_get("kind").map_err(decode_error)?;
    let kind = match native_kind.as_str() {
        "database" => CatalogKind::Database,
        "schema" => CatalogKind::Schema,
        "table" => CatalogKind::Table,
        "view" => CatalogKind::View,
        "materialized_view" => CatalogKind::MaterializedView,
        "sequence" => CatalogKind::Sequence,
        "function" => CatalogKind::Function,
        "procedure" => CatalogKind::Procedure,
        "type" => CatalogKind::Type,
        "column" => CatalogKind::Column,
        "index" => CatalogKind::Index,
        "primary_key" => CatalogKind::PrimaryKey,
        "unique_constraint" => CatalogKind::UniqueConstraint,
        "foreign_key" => CatalogKind::ForeignKey,
        "check_constraint" => CatalogKind::CheckConstraint,
        _ => {
            return Err(catalog_internal(
                "unexpected PostgreSQL search catalog kind",
            ));
        }
    };
    Ok(PgSearchCandidate {
        kind,
        database: row.try_get("database_name").map_err(decode_error)?,
        schema: row.try_get("schema_name").map_err(decode_error)?,
        name: row.try_get("object_name").map_err(decode_error)?,
        oid: row.try_get("object_oid").map_err(decode_error)?,
        relation_kind: row.try_get("relation_kind").map_err(decode_error)?,
        relation_name: row.try_get("relation_name").map_err(decode_error)?,
        relation_oid: row.try_get("relation_oid").map_err(decode_error)?,
        comment: row.try_get("comment").map_err(decode_error)?,
        relation_comment: row.try_get("relation_comment").map_err(decode_error)?,
        database_comment: row.try_get("database_comment").map_err(decode_error)?,
        schema_comment: row.try_get("schema_comment").map_err(decode_error)?,
    })
}

fn candidate_relation_id(profile_id: Uuid, candidate: &PgSearchCandidate) -> Option<CatalogId> {
    if !candidate.kind.is_relation_child() {
        return None;
    }
    let schema = candidate.schema.as_ref()?;
    let name = candidate.relation_name.as_ref()?;
    let oid = candidate.relation_oid?;
    let (kind, _) = relation_kind(candidate.relation_kind.as_deref()?).ok()?;
    Some(CatalogId::new(
        profile_id,
        kind,
        [
            candidate.database.clone(),
            schema.clone(),
            name.clone(),
            oid.to_string(),
        ],
    ))
}

fn stale_relation_identity(connection_id: Uuid, relation: &CatalogId) -> Option<(&str, i64)> {
    if relation.connection_id != connection_id || !relation.kind.is_relation() {
        return None;
    }
    let [database, schema, name, oid] = relation.native_path.as_slice() else {
        return None;
    };
    if database.is_empty() || schema.is_empty() || name.is_empty() {
        return None;
    }
    let oid = oid.parse::<i64>().ok()?;
    (oid > 0).then_some((database.as_str(), oid))
}

fn candidate_relation_entry(
    profile_id: Uuid,
    candidate: &PgSearchCandidate,
    schema_id: &CatalogId,
) -> Result<CatalogEntry, DatabaseError> {
    let id = candidate_relation_id(profile_id, candidate)
        .ok_or_else(|| catalog_internal("incomplete PostgreSQL search relation identity"))?;
    let schema = candidate
        .schema
        .as_deref()
        .ok_or_else(|| catalog_internal("PostgreSQL search relation has no schema"))?;
    let name = candidate
        .relation_name
        .as_deref()
        .ok_or_else(|| catalog_internal("PostgreSQL search relation has no name"))?;
    let (_, native_kind) = relation_kind(
        candidate
            .relation_kind
            .as_deref()
            .ok_or_else(|| catalog_internal("PostgreSQL search relation has no native kind"))?,
    )?;
    CatalogEntry::relation(
        id,
        schema_id.clone(),
        qualified_object(&candidate.database, schema, name),
        native_kind,
        OptionalMetadata::Supported(candidate.relation_comment.clone()),
        true,
    )
    .map_err(catalog_invariant)
}

fn candidate_child_id(
    relation: &CatalogId,
    candidate: &PgSearchCandidate,
) -> Result<CatalogId, DatabaseError> {
    let oid = candidate
        .oid
        .ok_or_else(|| catalog_internal("PostgreSQL search child has no native identity"))?;
    Ok(relation_child_id(
        relation,
        candidate.kind,
        &oid.to_string(),
    ))
}

fn relation_kind(native: &str) -> Result<(CatalogKind, &'static str), DatabaseError> {
    match native {
        "r" | "p" => Ok((CatalogKind::Table, "table")),
        "v" => Ok((CatalogKind::View, "view")),
        "m" => Ok((CatalogKind::MaterializedView, "materialized_view")),
        _ => Err(catalog_internal(
            "unexpected PostgreSQL search relation kind",
        )),
    }
}

fn search_kind_properties(kind: CatalogKind) -> Result<(&'static str, bool), DatabaseError> {
    match kind {
        CatalogKind::Table => Ok(("table", true)),
        CatalogKind::View => Ok(("view", true)),
        CatalogKind::MaterializedView => Ok(("materialized_view", true)),
        CatalogKind::Sequence => Ok(("sequence", false)),
        CatalogKind::Function => Ok(("function", false)),
        CatalogKind::Procedure => Ok(("procedure", false)),
        CatalogKind::Type => Ok(("type", false)),
        _ => Err(catalog_internal(
            "unexpected PostgreSQL top-level search kind",
        )),
    }
}

fn paginate_in_memory<T, SortKey, TieBreaker>(
    rows: &mut Vec<T>,
    request: &CatalogRequest,
    sort_key: SortKey,
    tie_breaker: TieBreaker,
) -> Result<Option<CatalogCursor>, DatabaseError>
where
    SortKey: Fn(&T) -> String,
    TieBreaker: Fn(&T) -> String,
{
    rows.sort_by(|left, right| {
        sort_key(left)
            .cmp(&sort_key(right))
            .then_with(|| tie_breaker(left).cmp(&tie_breaker(right)))
    });
    if let Some(cursor) = request.key.cursor.as_ref() {
        let (cursor_sort_key, cursor_tie_breaker) = cursor
            .keyset_parts()
            .map_err(DatabaseError::invalid_catalog_request)?;
        rows.retain(|row| {
            let row_sort_key = sort_key(row);
            let row_tie_breaker = tie_breaker(row);
            (row_sort_key.as_str(), row_tie_breaker.as_str())
                > (cursor_sort_key, cursor_tie_breaker)
        });
    }
    rows.truncate(request.page_size.saturating_add(1));
    finalize_keyset_page(rows, request.page_size, sort_key, tie_breaker).map_err(catalog_invariant)
}

fn qualified_database(database: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: None,
        object: database.to_owned(),
    }
}

fn qualified_schema(database: &str, schema: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: Some(schema.to_owned()),
        object: schema.to_owned(),
    }
}

fn qualified_object(database: &str, schema: &str, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: Some(schema.to_owned()),
        object: object.to_owned(),
    }
}

fn relation_child_id(relation: &CatalogId, kind: CatalogKind, native_identity: &str) -> CatalogId {
    let mut path = relation.native_path.clone();
    path.push(native_identity.to_owned());
    CatalogId::new(relation.profile_id(), kind, path)
}

fn qualified_index_name(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(name))
}

fn add_memberships(
    memberships: &mut HashMap<String, Vec<ConstraintMembership>>,
    columns: &[String],
    constraint_id: &CatalogId,
) -> Result<(), DatabaseError> {
    for (index, column) in columns.iter().enumerate() {
        memberships
            .entry(column.clone())
            .or_default()
            .push(ConstraintMembership {
                constraint_id: constraint_id.clone(),
                ordinal_position: u32::try_from(index.saturating_add(1))
                    .map_err(|_| catalog_internal("PostgreSQL constraint has too many columns"))?,
            });
    }
    Ok(())
}

fn group_sort_key(group: ObjectGroup) -> &'static str {
    match group {
        ObjectGroup::Tables => "00:tables",
        ObjectGroup::Dictionaries => "08:dictionaries",
        ObjectGroup::UserDefinedFunctions => "09:user_defined_functions",
        ObjectGroup::Views => "01:views",
        ObjectGroup::MaterializedViews => "02:materialized_views",
        ObjectGroup::Sequences => "03:sequences",
        ObjectGroup::Functions => "04:functions",
        ObjectGroup::Procedures => "05:procedures",
        ObjectGroup::Types => "06:types",
        ObjectGroup::Triggers => "07:triggers",
    }
}

fn child_sort_key(entry: &CatalogEntry) -> String {
    let value = match &entry.metadata {
        CatalogMetadata::Column(column) => format!("{:010}", column.ordinal_position),
        _ => entry.qualified_name.object.clone(),
    };
    format!("{:02}\0{}", catalog_kind_rank(entry.kind), value)
}

fn child_tie_breaker(entry: &CatalogEntry) -> String {
    format!(
        "{:02}\0{}",
        catalog_kind_rank(entry.kind),
        entry.id.native_path.last().map_or("", String::as_str)
    )
}

const fn catalog_kind_rank(kind: CatalogKind) -> u8 {
    match kind {
        CatalogKind::Column => 0,
        CatalogKind::Index => 1,
        CatalogKind::PrimaryKey => 2,
        CatalogKind::UniqueConstraint => 3,
        CatalogKind::ForeignKey => 4,
        CatalogKind::CheckConstraint => 5,
        CatalogKind::Trigger => 6,
        _ => 7,
    }
}

fn exact_count(count: usize) -> Result<CatalogCount, DatabaseError> {
    u64::try_from(count)
        .map(CatalogCount::Exact)
        .map_err(|_| catalog_internal("PostgreSQL catalog count exceeds u64"))
}

fn non_negative_count(count: i64) -> Result<u64, DatabaseError> {
    u64::try_from(count)
        .map_err(|_| catalog_internal("PostgreSQL returned a negative catalog count"))
}

fn page_limit(page_size: usize) -> Result<i64, DatabaseError> {
    page_size
        .checked_add(1)
        .and_then(|limit| i64::try_from(limit).ok())
        .ok_or_else(|| catalog_internal("PostgreSQL catalog page limit overflowed"))
}

fn catalog_target_not_found(target: &CatalogTarget) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("catalog_target_not_found".to_owned()),
        message: sanitize_terminal_text(&format!(
            "PostgreSQL catalog target was not found: {}",
            target.description()
        )),
        diagnostic: None,
    }
}

fn catalog_invariant(error: CatalogValidationError) -> DatabaseError {
    catalog_internal(format!("PostgreSQL catalog invariant failed: {error}"))
}

fn catalog_internal(message: impl AsRef<str>) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("postgres_catalog_invariant".to_owned()),
        message: sanitize_terminal_text(message.as_ref()),
        diagnostic: None,
    }
}

fn sql_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Sql)
}

fn postgres_relation_error(
    error: sqlx::Error,
    operation: &str,
    relation: &str,
) -> TransactionError {
    let (sqlstate, category, message) = match &error {
        sqlx::Error::Database(database) => {
            let sqlstate = database.code().map(|code| code.into_owned());
            let category = match sqlstate.as_deref() {
                Some(code) if code.starts_with("08") => RelationMutationCategory::ConnectionUnknown,
                Some("0A000" | "42883") => RelationMutationCategory::UnsupportedComparison,
                Some(code) if code.starts_with("22") || code == "42804" => {
                    RelationMutationCategory::TypeMismatch
                }
                Some(code) if code.starts_with("23") => RelationMutationCategory::Constraint,
                _ => RelationMutationCategory::InvalidRequest,
            };
            let message = match category {
                RelationMutationCategory::UnsupportedComparison => {
                    "the PostgreSQL comparison is not supported"
                }
                RelationMutationCategory::TypeMismatch => {
                    "the value does not match the PostgreSQL column type"
                }
                RelationMutationCategory::Constraint => {
                    "the mutation was rejected by a database constraint"
                }
                RelationMutationCategory::ConnectionUnknown => {
                    "the database connection failed; the outcome is unknown"
                }
                RelationMutationCategory::InvalidRequest => {
                    "PostgreSQL rejected the relation mutation"
                }
                RelationMutationCategory::Conflict => "relation mutation conflict",
            };
            (sqlstate, category, message.to_owned())
        }
        _ => (
            None,
            RelationMutationCategory::ConnectionUnknown,
            "PostgreSQL relation mutation failed without a server diagnostic".to_owned(),
        ),
    };
    TransactionError::relation(RelationMutationDiagnostic {
        category,
        sqlstate,
        operation: operation.to_owned(),
        relation: relation.to_owned(),
        context: None,
        message,
    })
}

fn bind_cell<'q>(
    query: sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    value: &CellValue,
    type_name: &str,
) -> Result<sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>, TransactionError> {
    let parameter_type = postgres_parameter_type(type_name, PgParameterMode::Assignment)?;
    Ok(match value {
        CellValue::Null => match parameter_type {
            PgParameterType::Boolean => query.bind(Option::<bool>::None),
            PgParameterType::Smallint => query.bind(Option::<i16>::None),
            PgParameterType::Integer => query.bind(Option::<i32>::None),
            PgParameterType::Bigint => query.bind(Option::<i64>::None),
            PgParameterType::Real => query.bind(Option::<f32>::None),
            PgParameterType::Double => query.bind(Option::<f64>::None),
            PgParameterType::Numeric => query.bind(Option::<BigDecimal>::None),
            PgParameterType::Date => query.bind(Option::<NaiveDate>::None),
            PgParameterType::Time => query.bind(Option::<NaiveTime>::None),
            PgParameterType::Timestamp => query.bind(Option::<NaiveDateTime>::None),
            PgParameterType::Timestamptz => query.bind(Option::<DateTime<Utc>>::None),
            PgParameterType::Uuid => query.bind(Option::<Uuid>::None),
            PgParameterType::Bytea => query.bind(Option::<Vec<u8>>::None),
            _ => query.bind(Option::<String>::None),
        },
        CellValue::Boolean(value) => query.bind(*value),
        CellValue::Integer(value) => query.bind(*value),
        CellValue::Unsigned(value) => query.bind(i64::try_from(*value).map_err(|_| {
            TransactionError("PostgreSQL cannot bind an unsigned value larger than i64".into())
        })?),
        CellValue::Float(value) => query.bind(*value),
        CellValue::Text(value) => match parameter_type {
            PgParameterType::Numeric => {
                query.bind(BigDecimal::from_str(value).map_err(|_| {
                    TransactionError("PostgreSQL numeric value is malformed".into())
                })?)
            }
            PgParameterType::Interval => query.bind(parse_pg_interval(value).ok_or_else(|| {
                TransactionError("PostgreSQL interval value is malformed".into())
            })?),
            PgParameterType::Uuid => query.bind(
                Uuid::parse_str(value)
                    .map_err(|_| TransactionError("PostgreSQL UUID value is malformed".into()))?,
            ),
            PgParameterType::Inet => {
                query.bind(value.parse::<IpAddr>().map_err(|_| {
                    TransactionError("PostgreSQL network value is malformed".into())
                })?)
            }
            PgParameterType::Cidr => {
                query.bind(IpNet::from_str(value).map_err(|_| {
                    TransactionError("PostgreSQL network value is malformed".into())
                })?)
            }
            PgParameterType::Json | PgParameterType::Jsonb => {
                query.bind(Json::<serde_json::Value>(
                    serde_json::from_str(value).map_err(|_| {
                        TransactionError("PostgreSQL JSON value is malformed".into())
                    })?,
                ))
            }
            PgParameterType::TextArray
            | PgParameterType::VarcharArray
            | PgParameterType::CharArray
            | PgParameterType::NameArray => {
                query.bind(parse_pg_text_array(value).ok_or_else(|| {
                    TransactionError("PostgreSQL text array value is malformed".into())
                })?)
            }
            _ => query.bind(value.clone()),
        },
        CellValue::Bytes(value) => query.bind(value.clone()),
        CellValue::MySqlGeometry { .. } => {
            return Err(TransactionError(
                "PostgreSQL cannot bind a MySQL geometry value".into(),
            ));
        }
        CellValue::Date(value) => query.bind(*value),
        CellValue::Time(value) => query.bind(*value),
        CellValue::DateTime(value) => query.bind(*value),
        CellValue::Timestamp(value) => query.bind(*value),
        CellValue::Unsupported { .. } => {
            return Err(TransactionError(
                "PostgreSQL cannot bind an unsupported cell value".into(),
            ));
        }
    })
}

fn parse_pg_text_array(value: &str) -> Option<Vec<Option<String>>> {
    let value = value.strip_prefix('{')?.strip_suffix('}')?;
    if value.is_empty() {
        return Some(Vec::new());
    }
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut element_quoted = false;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
            element_quoted = true;
        } else if character == ',' && !quoted {
            result.push(array_element(&current, element_quoted));
            current.clear();
            element_quoted = false;
        } else {
            current.push(character);
        }
    }
    if quoted || escaped {
        return None;
    }
    result.push(array_element(&current, element_quoted));
    Some(result)
}

fn array_element(value: &str, quoted: bool) -> Option<String> {
    let value = if quoted { value } else { value.trim() };
    if !quoted && value.eq_ignore_ascii_case("NULL") {
        None
    } else {
        Some(value.to_owned())
    }
}

pub const fn supports_server_version(server_version_num: i32) -> bool {
    server_version_num >= 120_000
}

pub fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn constraint_columns(kind: &ConstraintDefinitionKind) -> String {
    match kind {
        ConstraintDefinitionKind::PrimaryKey { columns }
        | ConstraintDefinitionKind::Unique { columns }
        | ConstraintDefinitionKind::ForeignKey { columns, .. } => columns.join(", "),
        ConstraintDefinitionKind::Check { .. } => String::new(),
    }
}

fn pg_fk_match(value: String) -> String {
    match value.as_str() {
        "f" => "FULL",
        "p" => "PARTIAL",
        _ => "SIMPLE",
    }
    .into()
}

fn pg_fk_action(value: String) -> String {
    match value.as_str() {
        "c" => "CASCADE",
        "r" => "RESTRICT",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "NO ACTION",
    }
    .into()
}

fn column_definition_fragment(
    column: &crate::model::catalog_editor::ColumnDraft,
) -> Result<String, CatalogMutationError> {
    let name = column.name.value().trim();
    let native_type = column.native_type.value().trim();
    if name.is_empty() || native_type.is_empty() {
        return Err(CatalogMutationError::InvalidDraft {
            reason: "column name and type are required".into(),
        });
    }
    if !column.generated_expression.value().trim().is_empty() {
        return Err(CatalogMutationError::UnsupportedOperation {
            object_type: CatalogObjectType::Catalog(CatalogKind::Column),
        });
    }
    let mut result = format!("{} {}", quote_identifier(name), native_type);
    if column.identity {
        result.push_str(" GENERATED BY DEFAULT AS IDENTITY");
    } else if !column.default_expression.value().trim().is_empty() {
        result.push_str(" DEFAULT ");
        result.push_str(column.default_expression.value().trim());
    }
    if !column.nullable {
        result.push_str(" NOT NULL");
    }
    Ok(result)
}

fn postgres_qualified_name(
    entry: &CatalogEntry,
    kind: CatalogKind,
) -> Result<String, CatalogDropError> {
    let name = &entry.qualified_name;
    if name.object.is_empty() {
        return Err(CatalogDropError::EmptyObjectName);
    }
    let schema = name
        .schema
        .as_deref()
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind,
            reason: "catalog entry has no schema-qualified name".to_owned(),
        })?;
    if schema.is_empty() {
        return Err(CatalogDropError::Unsupported {
            kind,
            reason: "catalog entry has an empty schema name".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(schema),
        quote_identifier(&name.object)
    ))
}

fn postgres_schema_name(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    let database = entry
        .qualified_name
        .database
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CatalogDropError::InvalidMetadata {
            kind: CatalogKind::Schema,
            reason: "catalog entry has no database name".to_owned(),
        })?;
    let schema = entry
        .qualified_name
        .schema
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CatalogDropError::InvalidMetadata {
            kind: CatalogKind::Schema,
            reason: "catalog entry has no schema name".to_owned(),
        })?;
    if entry.id.native_path.len() != 2
        || entry.id.native_path[0] != database
        || entry.id.native_path[1] != schema
        || entry.parent_id.as_ref().is_none_or(|parent| {
            parent.connection_id != entry.id.connection_id
                || parent.kind != CatalogKind::Database
                || parent.native_path.as_slice() != [database]
        })
    {
        return Err(CatalogDropError::InvalidMetadata {
            kind: CatalogKind::Schema,
            reason: "catalog entry namespace identity is inconsistent".to_owned(),
        });
    }
    if entry.qualified_name.object != schema {
        return Err(CatalogDropError::InvalidMetadata {
            kind: CatalogKind::Schema,
            reason: "catalog entry object name does not match its schema name".to_owned(),
        });
    }
    if matches!(schema, "information_schema") || schema.starts_with("pg_") {
        return Err(CatalogDropError::Unsupported {
            kind: CatalogKind::Schema,
            reason: "system schemas cannot be dropped from the catalog".to_owned(),
        });
    }
    Ok(quote_identifier(schema))
}

fn relation_name_for_drop(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    let relation = entry
        .relation_id
        .as_ref()
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has no owning relation identity".to_owned(),
        })?;
    if !relation.kind.is_relation() || relation.native_path.len() < 3 {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an invalid owning relation identity".to_owned(),
        });
    }
    let schema =
        entry
            .qualified_name
            .schema
            .as_deref()
            .ok_or_else(|| CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "catalog entry has no owning relation schema".to_owned(),
            })?;
    let relation_name = relation.native_path[2].as_str();
    if schema.is_empty() || relation_name.is_empty() {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an incomplete owning relation identity".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(schema),
        quote_identifier(relation_name)
    ))
}

pub fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn split_view_columns(value: &str) -> Result<Vec<String>, CatalogMutationError> {
    let columns = value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok(columns)
}

fn view_options_clause(
    draft: &crate::model::catalog_editor::ViewDraft,
    version: i32,
) -> Result<(String, String), CatalogMutationError> {
    if draft.security_barrier.value.is_some() && version < 90_200
        || draft.security_invoker.value.is_some() && version < 150_000
        || draft.check_option.value.is_some() && version < 90_400
    {
        return Err(CatalogMutationError::UnsupportedOperation {
            object_type: CatalogObjectType::Catalog(CatalogKind::View),
        });
    }
    let mut options = Vec::new();
    if let Some(value) = draft.security_barrier.value {
        options.push(format!(
            "security_barrier={}",
            if value { "true" } else { "false" }
        ));
    }
    if let Some(value) = draft.security_invoker.value {
        options.push(format!(
            "security_invoker={}",
            if value { "true" } else { "false" }
        ));
    }
    let mut check_suffix = String::new();
    if let Some(value) = draft.check_option.value.as_deref() {
        if !matches!(value, "LOCAL" | "CASCADED" | "NONE") {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "check option must be LOCAL, CASCADED, or NONE".into(),
            });
        }
        if value != "NONE" {
            check_suffix = format!(" {value} CHECK OPTION");
        }
    }
    if options.is_empty() {
        Ok((String::new(), check_suffix))
    } else {
        Ok((format!(" WITH ({})", options.join(", ")), check_suffix))
    }
}

fn decode_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Internal)
}

fn monitor_timestamp(row: &PgRow, name: &str) -> Result<u64, DatabaseError> {
    let value = row.try_get::<i64, _>(name).map_err(decode_error)?;
    u64::try_from(value).map_err(|_| DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("postgres_monitor_decode".to_owned()),
        message: format!("PostgreSQL returned a negative monitoring timestamp: {name}"),
        diagnostic: None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::catalog::{CatalogId, CatalogKind};
    use super::{
        PgDdlColumn, PgDdlRelation, PgParameterMode, assemble_relation_ddl, column_definition,
        format_pg_array, format_pg_interval, parse_pg_interval, postgres_delete_sql,
        postgres_placeholder, quote_identifier, quote_literal, stale_relation_identity,
    };
    use sqlx::postgres::types::PgInterval;
    use uuid::Uuid;

    #[test]
    fn stale_relation_identity_requires_matching_profile_and_relation_path() {
        let profile = Uuid::from_u128(1);
        let valid = CatalogId::new(profile, CatalogKind::Table, ["db", "public", "old", "42"]);
        assert_eq!(stale_relation_identity(profile, &valid), Some(("db", 42)));

        for invalid in [
            CatalogId::new(
                Uuid::from_u128(2),
                CatalogKind::Table,
                ["db", "public", "old", "42"],
            ),
            CatalogId::new(profile, CatalogKind::Column, ["db", "public", "old", "42"]),
            CatalogId::new(profile, CatalogKind::Table, ["db", "public", "old"]),
            CatalogId::new(
                profile,
                CatalogKind::Table,
                ["db", "public", "old", "not-an-oid"],
            ),
            CatalogId::new(profile, CatalogKind::Table, ["db", "public", "old", "0"]),
            CatalogId::new(profile, CatalogKind::Table, ["", "public", "old", "42"]),
            CatalogId::new(profile, CatalogKind::Table, ["db", "", "old", "42"]),
            CatalogId::new(profile, CatalogKind::Table, ["db", "public", "", "42"]),
        ] {
            assert_eq!(stale_relation_identity(profile, &invalid), None);
        }
    }

    fn catalog_dispatch_connection(
        profile_id: Uuid,
        server_version_num: i32,
    ) -> crate::db::DatabaseConnection {
        crate::db::DatabaseConnection::Postgres(super::PostgresAdapter {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://localhost/test")
                .unwrap(),
            connection_id: profile_id,
            catalog_scope: crate::profile::CatalogScope::for_profile(
                crate::profile::DatabaseKind::Postgres,
                "test",
                None,
            ),
            server_version_num,
        })
    }

    fn catalog_dispatch_request(
        profile_id: Uuid,
        request_id: u64,
        mode: crate::db::catalog_mutation::CatalogMutationMode,
        anchor: crate::db::catalog_mutation::CatalogMutationAnchor,
        object_type: crate::db::catalog_mutation::CatalogObjectType,
    ) -> crate::db::catalog_mutation::CatalogMutationRequest {
        crate::db::catalog_mutation::CatalogMutationRequest::new(
            crate::identity::ConnectionIdentity {
                profile_id,
                generation: 1,
            },
            request_id,
            1,
            mode,
            anchor,
            object_type,
        )
        .unwrap()
        .with_current_database("test")
    }

    fn catalog_dispatch_database_definition() -> crate::db::catalog_mutation::DatabaseDefinition {
        crate::db::catalog_mutation::DatabaseDefinition {
            name: "app".into(),
            owner: "postgres".into(),
            template: "template0".into(),
            encoding: "UTF8".into(),
            locale_provider: "libc".into(),
            locale: "C".into(),
            collation: "C".into(),
            ctype: "C".into(),
            tablespace: "pg_default".into(),
            connection_limit: -1,
            allow_connections: true,
            is_template: false,
            comment: crate::db::catalog::OptionalMetadata::Supported(None),
            baseline_fingerprint: "sha256:app".into(),
        }
    }

    fn catalog_dispatch_role_definition(
        login: bool,
    ) -> crate::db::catalog_mutation::RoleDefinition {
        crate::db::catalog_mutation::RoleDefinition {
            name: "alice".into(),
            login,
            superuser: false,
            createdb: false,
            createrole: false,
            inherit: true,
            replication: false,
            bypass_rls: false,
            connection_limit: -1,
            valid_until: crate::db::catalog::OptionalMetadata::Supported(Some("infinity".into())),
            memberships: vec!["old_group".into()],
            comment: crate::db::catalog::OptionalMetadata::Supported(None),
            baseline_fingerprint: "sha256:role".into(),
        }
    }

    #[tokio::test]
    async fn catalog_mutation_dispatch_creates_roles() {
        use crate::db::catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationMode, CatalogObjectType,
        };
        use crate::model::catalog_editor::{CatalogDraft, RoleDraft};

        let profile_id = Uuid::from_u128(101);
        for (object_type, login) in [
            (CatalogObjectType::Role, false),
            (CatalogObjectType::LoginRole, true),
        ] {
            for server_version_num in [120_000, 150_000, 180_000] {
                let mut draft = RoleDraft::new(login);
                draft.name = "test1".into();
                if login {
                    draft.set_password("dispatch-test-password");
                }
                let request = catalog_dispatch_request(
                    profile_id,
                    1,
                    CatalogMutationMode::Create,
                    CatalogMutationAnchor::Profile { profile_id },
                    object_type,
                );
                let connection = catalog_dispatch_connection(profile_id, server_version_num);
                let plan = connection
                    .plan_catalog_mutation(request.clone(), CatalogDraft::Role(draft.clone()), None)
                    .unwrap();
                let version_plan = super::PostgresAdapter::plan_catalog_mutation_for_version(
                    request.clone(),
                    CatalogDraft::Role(draft.clone()),
                    None,
                    server_version_num,
                )
                .unwrap();
                let default_plan = super::PostgresAdapter::plan_catalog_mutation(
                    request,
                    CatalogDraft::Role(draft),
                    None,
                )
                .unwrap();

                assert_eq!(plan.sql(), version_plan.sql());
                assert_eq!(plan.sql(), default_plan.sql());
                assert_eq!(plan.object_type, object_type);
                assert_eq!(
                    plan.execution_mode,
                    crate::db::catalog_mutation::CatalogMutationExecutionMode::Transactional
                );
                assert_eq!(plan.execution_target.database(), "test");
                assert!(
                    plan.execution_target
                        .execution_target(profile_id)
                        .schema
                        .is_none()
                );
                assert!(plan.baseline_fingerprint.is_none());
                assert!(
                    plan.sql()
                        .contains(if login { " LOGIN " } else { " NOLOGIN " })
                );
                if login {
                    assert!(plan.sql().contains("PASSWORD '<REDACTED>'"));
                    assert!(!plan.sql().contains("dispatch-test-password"));
                    assert!(!format!("{plan:?}").contains("dispatch-test-password"));
                }
            }
        }
    }

    #[tokio::test]
    async fn catalog_mutation_dispatch_edits_roles() {
        use crate::db::catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationMode, CatalogObjectDefinition, CatalogObjectType,
        };
        use crate::model::catalog_editor::{CatalogDraft, RoleDraft};

        let profile_id = Uuid::from_u128(102);
        for (object_type, login) in [
            (CatalogObjectType::Role, false),
            (CatalogObjectType::LoginRole, true),
        ] {
            for server_version_num in [120_000, 150_000, 180_000] {
                let definition = catalog_dispatch_role_definition(login);
                let mut draft = RoleDraft::from_definition(&definition);
                draft.name = "bob".into();
                draft.memberships = "new_group".into();
                let request = catalog_dispatch_request(
                    profile_id,
                    2,
                    CatalogMutationMode::Edit,
                    CatalogMutationAnchor::Catalog(CatalogId::new(
                        profile_id,
                        CatalogKind::Database,
                        ["__role__", "alice"],
                    )),
                    object_type,
                );
                let baseline = Some(CatalogObjectDefinition::Role(definition));
                let connection = catalog_dispatch_connection(profile_id, server_version_num);
                let plan = connection
                    .plan_catalog_mutation(
                        request.clone(),
                        CatalogDraft::Role(draft.clone()),
                        baseline.clone(),
                    )
                    .unwrap();
                let version_plan = super::PostgresAdapter::plan_catalog_mutation_for_version(
                    request.clone(),
                    CatalogDraft::Role(draft.clone()),
                    baseline.clone(),
                    server_version_num,
                )
                .unwrap();
                let default_plan = super::PostgresAdapter::plan_catalog_mutation(
                    request,
                    CatalogDraft::Role(draft),
                    baseline,
                )
                .unwrap();

                assert_eq!(plan.sql(), version_plan.sql());
                assert_eq!(plan.sql(), default_plan.sql());
                assert!(
                    plan.sql()
                        .contains("ALTER ROLE \"alice\" RENAME TO \"bob\"")
                );
                assert!(plan.sql().contains("GRANT \"new_group\" TO \"bob\""));
                assert!(plan.sql().contains("REVOKE \"old_group\" FROM \"bob\""));
                assert_eq!(plan.object_type, object_type);
                assert_eq!(
                    plan.execution_mode,
                    crate::db::catalog_mutation::CatalogMutationExecutionMode::Transactional
                );
                assert_eq!(plan.execution_target.database(), "test");
                assert_eq!(plan.baseline_fingerprint.as_deref(), Some("sha256:role"));
                assert!(!plan.sql().contains("PASSWORD"));
            }
        }
    }

    #[tokio::test]
    async fn catalog_mutation_dispatch_creates_database() {
        use crate::db::catalog::CatalogTarget;
        use crate::db::catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationExecutionMode, CatalogMutationMode,
            CatalogObjectType, CatalogSelectionHint,
        };
        use crate::model::catalog_editor::{CatalogDraft, DatabaseDraft};

        let profile_id = Uuid::from_u128(103);
        let mut draft = DatabaseDraft::new("test");
        draft.name = "app".into();
        draft.owner = "postgres".into();
        draft.comment = "created by dispatch test".into();
        let request = catalog_dispatch_request(
            profile_id,
            3,
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Profile { profile_id },
            CatalogObjectType::Catalog(CatalogKind::Database),
        );
        let plan = catalog_dispatch_connection(profile_id, 150_000)
            .plan_catalog_mutation(request, CatalogDraft::Database(draft), None)
            .unwrap();

        assert!(
            plan.sql()
                .contains("CREATE DATABASE \"app\" OWNER \"postgres\"")
        );
        assert!(plan.sql().contains("COMMENT ON DATABASE \"app\""));
        assert_eq!(
            plan.execution_mode,
            CatalogMutationExecutionMode::Autocommit
        );
        assert_eq!(plan.execution_target.database(), "test");
        assert_eq!(plan.refresh, vec![CatalogTarget::Databases]);
        assert!(
            matches!(plan.selection, CatalogSelectionHint::Object(ref id) if id.native_path == ["app"])
        );
        assert!(plan.baseline_fingerprint.is_none());
        assert_eq!(
            plan.object_type,
            CatalogObjectType::Catalog(CatalogKind::Database)
        );
    }

    #[tokio::test]
    async fn catalog_mutation_dispatch_edits_database() {
        use crate::db::catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationExecutionMode, CatalogMutationMode,
            CatalogObjectDefinition, CatalogObjectType, CatalogSelectionHint,
        };
        use crate::model::catalog_editor::{CatalogDraft, DatabaseDraft};

        let profile_id = Uuid::from_u128(104);
        let definition = catalog_dispatch_database_definition();
        let mut draft = DatabaseDraft::from_definition(&definition);
        draft.connection_limit = "12".into();
        let request = catalog_dispatch_request(
            profile_id,
            4,
            CatalogMutationMode::Edit,
            CatalogMutationAnchor::Catalog(CatalogId::new(
                profile_id,
                CatalogKind::Database,
                ["app"],
            )),
            CatalogObjectType::Catalog(CatalogKind::Database),
        );
        let plan = catalog_dispatch_connection(profile_id, 150_000)
            .plan_catalog_mutation(
                request,
                CatalogDraft::Database(draft),
                Some(CatalogObjectDefinition::Database(definition)),
            )
            .unwrap();

        assert!(
            plan.sql()
                .contains("ALTER DATABASE \"app\" CONNECTION LIMIT 12")
        );
        assert_eq!(
            plan.execution_mode,
            CatalogMutationExecutionMode::Autocommit
        );
        assert_eq!(plan.execution_target.database(), "test");
        assert_eq!(plan.baseline_fingerprint.as_deref(), Some("sha256:app"));
        assert!(
            matches!(plan.selection, CatalogSelectionHint::Object(ref id) if id.native_path == ["app"])
        );
        assert_eq!(
            plan.object_type,
            CatalogObjectType::Catalog(CatalogKind::Database)
        );
    }

    #[tokio::test]
    async fn catalog_mutation_dispatch_preserves_validation() {
        use crate::db::catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationError, CatalogMutationMode,
            CatalogObjectDefinition, CatalogObjectType,
        };
        use crate::model::catalog_editor::{CatalogDraft, DatabaseDraft, RoleDraft};

        let profile_id = Uuid::from_u128(105);
        let mut role = RoleDraft::new(false);
        role.name = "  ".into();
        let error = catalog_dispatch_connection(profile_id, 150_000)
            .plan_catalog_mutation(
                catalog_dispatch_request(
                    profile_id,
                    5,
                    CatalogMutationMode::Create,
                    CatalogMutationAnchor::Profile { profile_id },
                    CatalogObjectType::Role,
                ),
                CatalogDraft::Role(role),
                None,
            )
            .unwrap_err();
        assert!(matches!(error, CatalogMutationError::InvalidDraft { .. }));

        let definition = catalog_dispatch_database_definition();
        let mut database = DatabaseDraft::from_definition(&definition);
        database.name = "new_app".into();
        let error = catalog_dispatch_connection(profile_id, 150_000)
            .plan_catalog_mutation(
                crate::db::catalog_mutation::CatalogMutationRequest::new(
                    crate::identity::ConnectionIdentity {
                        profile_id,
                        generation: 1,
                    },
                    6,
                    1,
                    CatalogMutationMode::Edit,
                    CatalogMutationAnchor::Catalog(CatalogId::new(
                        profile_id,
                        CatalogKind::Database,
                        ["app"],
                    )),
                    CatalogObjectType::Catalog(CatalogKind::Database),
                )
                .unwrap()
                .with_current_database("app"),
                CatalogDraft::Database(database),
                Some(CatalogObjectDefinition::Database(definition)),
            )
            .unwrap_err();
        assert!(
            matches!(error, CatalogMutationError::InvalidPlan { reason } if reason.contains("currently connected database"))
        );
    }

    #[tokio::test]
    async fn catalog_mutation_dispatch_preserves_server_version() {
        use crate::db::catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationError, CatalogMutationMode, CatalogObjectType,
            ViewMutationOptionAvailability, ViewOption,
        };
        use crate::model::catalog_editor::{CatalogDraft, CatalogFormFocus, ViewDraft};

        let profile_id = Uuid::from_u128(106);
        let request = || {
            catalog_dispatch_request(
                profile_id,
                7,
                CatalogMutationMode::Create,
                CatalogMutationAnchor::Catalog(CatalogId::new(
                    profile_id,
                    CatalogKind::Schema,
                    ["test", "public"],
                )),
                CatalogObjectType::Catalog(CatalogKind::View),
            )
        };
        let draft = || {
            CatalogDraft::View(ViewDraft {
                name: "versioned_view".into(),
                schema: "public".into(),
                owner: "postgres".into(),
                comment: "".into(),
                query: "SELECT 1".into(),
                output_columns: "".into(),
                security_barrier: ViewOption::available(None),
                security_invoker: ViewOption::available(Some(true)),
                check_option: ViewOption::available(None),
                focus: CatalogFormFocus::Name,
                clickhouse_native: None,
            })
        };

        let error = catalog_dispatch_connection(profile_id, 140_000)
            .plan_catalog_mutation(request(), draft(), None)
            .unwrap_err();
        assert!(matches!(
            error,
            CatalogMutationError::UnsupportedOperation {
                object_type: CatalogObjectType::Catalog(CatalogKind::View)
            }
        ));
        let old_version_capabilities =
            catalog_dispatch_connection(profile_id, 140_000).catalog_mutation_capabilities();
        assert!(matches!(
            old_version_capabilities.view_options.security_invoker,
            ViewMutationOptionAvailability::Unavailable {
                reason: "security_invoker requires PostgreSQL 15 or newer"
            }
        ));

        let plan = catalog_dispatch_connection(profile_id, 150_000)
            .plan_catalog_mutation(request(), draft(), None)
            .unwrap();
        assert!(plan.sql().contains("security_invoker=true"));
    }

    #[tokio::test]
    async fn principal_mutation_plans_quote_targets_and_reject_invalid_privileges() {
        let principal = crate::db::principal::PrincipalEntry {
            id: crate::db::principal::PrincipalId {
                profile_id: Uuid::from_u128(1),
                scope: crate::db::principal::PrincipalScope::Cluster,
                native_id: "10".into(),
                host: None,
            },
            kind: crate::db::principal::PrincipalKind::User,
            name: "alice\"ops".into(),
            native_kind: "login_role".into(),
            system: false,
        };
        let adapter = super::PostgresAdapter {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://localhost/test")
                .unwrap(),
            connection_id: Uuid::from_u128(1),
            catalog_scope: crate::profile::CatalogScope::for_profile(
                crate::profile::DatabaseKind::Postgres,
                "test",
                None,
            ),
            server_version_num: 120_000,
        };
        let plan = adapter
            .plan_principal_mutation(
                &principal,
                crate::identity::ConnectionIdentity {
                    profile_id: Uuid::from_u128(1),
                    generation: 1,
                },
                Some("app"),
                crate::db::principal::PrincipalMutation::Grant {
                    target: crate::db::principal::PrincipalMutationTarget::Relation {
                        schema: "public".into(),
                        relation: "orders".into(),
                    },
                    privilege: "select".into(),
                    grant_option: true,
                },
            )
            .unwrap();
        assert_eq!(plan.database.as_deref(), Some("app"));
        assert!(
            plan.sql
                .contains("GRANT SELECT ON TABLE \"public\".\"orders\"")
        );
        assert!(plan.sql.contains("\"alice\"\"ops\""));
        let column_plan = adapter
            .plan_principal_mutation(
                &principal,
                crate::identity::ConnectionIdentity {
                    profile_id: Uuid::from_u128(1),
                    generation: 1,
                },
                Some("app"),
                crate::db::principal::PrincipalMutation::Grant {
                    target: crate::db::principal::PrincipalMutationTarget::Column {
                        schema: "public".into(),
                        relation: "orders".into(),
                        column: "customer.name".into(),
                    },
                    privilege: "SELECT".into(),
                    grant_option: false,
                },
            )
            .unwrap();
        assert_eq!(
            column_plan.sql,
            "GRANT SELECT (\"customer.name\") ON TABLE \"public\".\"orders\" TO \"alice\"\"ops\";"
        );
        assert!(
            adapter
                .plan_principal_mutation(
                    &principal,
                    crate::identity::ConnectionIdentity {
                        profile_id: Uuid::from_u128(1),
                        generation: 1,
                    },
                    None,
                    crate::db::principal::PrincipalMutation::Grant {
                        target: crate::db::principal::PrincipalMutationTarget::Database,
                        privilege: "SELECT; DROP ROLE".into(),
                        grant_option: false,
                    },
                )
                .is_err()
        );
        assert!(
            adapter
                .plan_principal_mutation(
                    &principal,
                    crate::identity::ConnectionIdentity {
                        profile_id: Uuid::from_u128(1),
                        generation: 1,
                    },
                    None,
                    crate::db::principal::PrincipalMutation::Grant {
                        target: crate::db::principal::PrincipalMutationTarget::Database,
                        privilege: "CONNECT".into(),
                        grant_option: false,
                    },
                )
                .is_err()
        );
    }

    #[test]
    fn postgres_placeholder_handles_numeric_typmods() {
        for type_name in [
            "numeric",
            "numeric(18,2)",
            "decimal(38,10)",
            " NUMERIC(18, 2) ",
        ] {
            assert_eq!(
                postgres_placeholder(2, type_name, PgParameterMode::Assignment).unwrap(),
                "$2::numeric"
            );
        }
    }

    #[test]
    fn postgres_placeholder_handles_typmods_and_keeps_parameter_numbers() {
        assert_eq!(
            postgres_placeholder(7, "timestamp(6)", PgParameterMode::Assignment).unwrap(),
            "$7::timestamp"
        );
        assert_eq!(
            postgres_placeholder(
                8,
                "timestamp(6) with time zone",
                PgParameterMode::Assignment
            )
            .unwrap(),
            "$8::timestamptz"
        );
        assert_eq!(
            postgres_placeholder(9, "varchar(32)", PgParameterMode::Assignment).unwrap(),
            "$9::varchar"
        );
        assert!(postgres_placeholder(10, "custom_type(1)", PgParameterMode::Assignment).is_err());
    }

    #[test]
    fn postgres_parameter_types_are_whitelisted_by_position() {
        let cases = [
            ("interval", "interval", true),
            ("text[]", "text[]", true),
            ("inet", "inet", true),
            ("cidr", "cidr", true),
            ("uuid", "uuid", true),
            ("json", "json", true),
            ("jsonb", "jsonb", true),
            ("timestamp(6) with time zone", "timestamptz", true),
            ("point", "point", true),
            ("custom_type", "", false),
            ("text[foo]", "", false),
            ("numeric(18, nope)", "", false),
            ("numeric(18,2", "", false),
        ];
        for (type_name, cast, supported) in cases {
            let result = postgres_placeholder(3, type_name, PgParameterMode::Assignment);
            assert_eq!(result.is_ok(), supported, "{type_name}");
            if supported {
                assert_eq!(result.unwrap(), format!("$3::{cast}"));
            }
        }
        assert!(postgres_placeholder(1, "json", PgParameterMode::Equality).is_err());
        assert!(postgres_placeholder(1, "point", PgParameterMode::Equality).is_err());
        assert!(postgres_placeholder(1, "text[]", PgParameterMode::Equality).is_ok());
    }

    #[test]
    fn postgres_placeholder_preserves_parameter_numbering_and_rejects_raw_type_text() {
        assert_eq!(
            postgres_placeholder(17, "numeric(38,10)", PgParameterMode::Equality).unwrap(),
            "$17::numeric"
        );
        assert!(
            postgres_placeholder(18, "public.weird_type", PgParameterMode::Assignment).is_err()
        );
        assert!(
            postgres_placeholder(19, "varchar(32) trailing", PgParameterMode::Assignment).is_err()
        );
    }

    #[test]
    fn parses_text_arrays_without_collapsing_null_values() {
        assert_eq!(
            super::parse_pg_text_array("{\"NULL\",NULL,\"with,comma\",\"back\\\\slash\"}"),
            Some(vec![
                Some("NULL".into()),
                None,
                Some("with,comma".into()),
                Some("back\\slash".into()),
            ])
        );
        assert_eq!(super::parse_pg_text_array("{}"), Some(Vec::new()));
        assert_eq!(super::parse_pg_text_array("{\"unterminated}"), None);
    }

    #[test]
    fn ddl_quoting_escapes_postgres_identifiers_and_literals() {
        assert_eq!(quote_identifier("odd\"name"), "\"odd\"\"name\"");
        assert_eq!(quote_literal("owner's note"), "'owner''s note'");
    }

    #[test]
    fn formats_postgres_interval_for_display() {
        assert_eq!(
            format_pg_interval(PgInterval {
                months: 0,
                days: 0,
                microseconds: 3_600_000_000,
            }),
            "0 years 0 mons 0 days 1 hours 0 mins 0.0 secs"
        );
        assert_eq!(
            format_pg_interval(PgInterval {
                months: 14,
                days: 3,
                microseconds: 4_505_250_000,
            }),
            "1 years 2 mons 3 days 1 hours 15 mins 5.25 secs"
        );
        assert_eq!(
            parse_pg_interval("1 year 2 mons 3 days 01:15:05.25"),
            Some(PgInterval {
                months: 14,
                days: 3,
                microseconds: 4_505_250_000,
            })
        );
        assert_eq!(
            parse_pg_interval("01:00:00"),
            Some(PgInterval {
                months: 0,
                days: 0,
                microseconds: 3_600_000_000,
            })
        );
        assert_eq!(
            format_pg_interval(parse_pg_interval("-00:00:00.5").unwrap()),
            "0 years 0 mons 0 days 0 hours 0 mins -0.5 secs"
        );
    }

    #[test]
    fn formats_postgres_text_arrays_without_losing_boundaries() {
        assert_eq!(
            format_pg_array(vec![Some("tag-1".into()), Some("odd".into())]),
            "{tag-1,odd}"
        );
        assert_eq!(
            format_pg_array(vec![
                Some("with,comma".into()),
                Some("NULL".into()),
                Some(String::new()),
                None,
            ]),
            "{\"with,comma\",\"NULL\",\"\",NULL}"
        );
    }

    #[test]
    fn delete_sql_has_a_complete_where_clause_for_row_locators() {
        let columns = vec![
            ("id".to_owned(), "integer".to_owned(), false),
            ("name".to_owned(), "text".to_owned(), true),
        ];
        let sql = postgres_delete_sql("\"public\".\"users\"", &columns, &[0]).unwrap();

        assert_eq!(
            sql,
            "DELETE FROM \"public\".\"users\" WHERE \"id\" IS NOT DISTINCT FROM $1::integer AND xmin = $2::xid"
        );
        assert!(postgres_delete_sql("\"public\".\"users\"", &columns, &[]).is_err());
    }

    #[test]
    fn delete_sql_casts_nullable_bigint_parameters() {
        let columns = vec![
            ("id".to_owned(), "bigint".to_owned(), false),
            ("dept_id".to_owned(), "bigint".to_owned(), true),
            (
                "manager".to_owned(),
                "character varying(30)".to_owned(),
                true,
            ),
        ];
        let sql = postgres_delete_sql("\"tools\".\"sys_user\"", &columns, &[0]).unwrap();

        assert!(sql.contains("\"id\" IS NOT DISTINCT FROM $1::bigint"));
        assert!(!sql.contains("dept_id"));
        assert!(!sql.contains("manager"));
        assert_eq!(sql.matches('$').count(), 2);
    }

    #[test]
    fn delete_sql_casts_numeric_typmod_snapshot_parameters() {
        let columns = vec![
            ("id".to_owned(), "bigint".to_owned(), false),
            ("amount".to_owned(), "numeric(18,2)".to_owned(), true),
        ];
        let sql = postgres_delete_sql("\"public\".\"payments\"", &columns, &[0]).unwrap();

        assert!(sql.contains("\"id\" IS NOT DISTINCT FROM $1::bigint"));
        assert!(!sql.contains("amount"));
        assert_eq!(sql.matches('$').count(), 2);
    }

    #[test]
    fn delete_sql_supports_composite_primary_key_without_business_columns() {
        let columns = vec![
            ("tenant_id".to_owned(), "bigint".to_owned(), false),
            ("item_id".to_owned(), "uuid".to_owned(), false),
            ("metadata".to_owned(), "jsonb".to_owned(), true),
        ];
        let sql = postgres_delete_sql("\"public\".\"items\"", &columns, &[0, 1]).unwrap();
        assert_eq!(
            sql,
            "DELETE FROM \"public\".\"items\" WHERE \"tenant_id\" IS NOT DISTINCT FROM $1::bigint AND \"item_id\" IS NOT DISTINCT FROM $2::uuid AND xmin = $3::xid"
        );
        assert!(!sql.contains("metadata"));
    }

    #[test]
    fn delete_sql_rejects_empty_or_out_of_range_locators() {
        let columns = vec![("id".to_owned(), "bigint".to_owned(), false)];
        assert!(postgres_delete_sql("\"public\".\"items\"", &columns, &[]).is_err());
        assert!(postgres_delete_sql("\"public\".\"items\"", &columns, &[1]).is_err());
    }

    #[test]
    fn column_definition_uses_exactly_one_generated_identity_or_default_clause() {
        let base = PgDdlColumn {
            name: "id".to_owned(),
            native_type: "bigint".to_owned(),
            default_expression: Some("nextval('ignored')".to_owned()),
            not_null: true,
            identity_kind: "d".to_owned(),
            generated_kind: String::new(),
        };
        assert_eq!(
            column_definition(&base).unwrap(),
            "\"id\" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL"
        );

        let generated = PgDdlColumn {
            name: "slug".to_owned(),
            native_type: "text".to_owned(),
            default_expression: Some("lower(name)".to_owned()),
            not_null: false,
            identity_kind: String::new(),
            generated_kind: "s".to_owned(),
        };
        assert_eq!(
            column_definition(&generated).unwrap(),
            "\"slug\" text GENERATED ALWAYS AS (lower(name)) STORED"
        );

        let defaulted = PgDdlColumn {
            name: "active".to_owned(),
            native_type: "boolean".to_owned(),
            default_expression: Some("true".to_owned()),
            not_null: false,
            identity_kind: String::new(),
            generated_kind: String::new(),
        };
        assert_eq!(
            column_definition(&defaulted).unwrap(),
            "\"active\" boolean DEFAULT true"
        );
    }

    #[test]
    fn assembles_table_constraints_comments_indexes_and_triggers_in_sections() {
        let ddl = assemble_relation_ddl(PgDdlRelation {
            schema: "odd schema".to_owned(),
            name: "accounts".to_owned(),
            relation_kind: "r".to_owned(),
            persistence: "p".to_owned(),
            view_definition: None,
            materialized_populated: true,
            partition_key: None,
            partition_parent: None,
            partition_bound: None,
            columns: vec![PgDdlColumn {
                name: "id".to_owned(),
                native_type: "integer".to_owned(),
                default_expression: None,
                not_null: true,
                identity_kind: String::new(),
                generated_kind: String::new(),
            }],
            constraints: vec![("accounts_pk".to_owned(), "PRIMARY KEY (id)".to_owned())],
            relation_comment: Some("owner's table".to_owned()),
            column_comments: vec![("id".to_owned(), "primary key".to_owned())],
            indexes: vec![("accounts_live_idx".to_owned(), "CREATE INDEX accounts_live_idx ON \"odd schema\".accounts (id)".to_owned())],
            triggers: vec![("audit".to_owned(), "CREATE TRIGGER audit AFTER UPDATE ON \"odd schema\".accounts EXECUTE FUNCTION audit_row()".to_owned())],
        })
        .unwrap();

        assert!(ddl.contains("-- Table\n\nCREATE TABLE \"odd schema\".\"accounts\" (\n  \"id\" integer NOT NULL,\n  CONSTRAINT \"accounts_pk\" PRIMARY KEY (id)\n);"));
        assert!(ddl.contains(
            "-- Comments\n\nCOMMENT ON TABLE \"odd schema\".\"accounts\" IS 'owner''s table';"
        ));
        assert!(
            ddl.contains("COMMENT ON COLUMN \"odd schema\".\"accounts\".\"id\" IS 'primary key';")
        );
        assert!(ddl.contains("-- Indexes\n\nCREATE INDEX accounts_live_idx"));
        assert!(ddl.contains("-- Triggers\n\nCREATE TRIGGER audit"));
    }

    #[test]
    fn assembles_view_and_omits_empty_optional_sections() {
        let ddl = assemble_relation_ddl(PgDdlRelation {
            schema: "public".to_owned(),
            name: "active_accounts".to_owned(),
            relation_kind: "v".to_owned(),
            persistence: "p".to_owned(),
            view_definition: Some(" SELECT id FROM accounts ".to_owned()),
            materialized_populated: true,
            partition_key: None,
            partition_parent: None,
            partition_bound: None,
            columns: vec![],
            constraints: vec![],
            relation_comment: None,
            column_comments: vec![],
            indexes: vec![],
            triggers: vec![],
        })
        .unwrap();

        assert_eq!(
            ddl,
            "-- View\n\nCREATE VIEW \"public\".\"active_accounts\" AS\nSELECT id FROM accounts;"
        );
        assert!(!ddl.contains("-- Comments"));
        assert!(!ddl.contains("-- Indexes"));
        assert!(!ddl.contains("-- Triggers"));
    }

    #[test]
    fn rejects_a_missing_main_view_definition() {
        let error = assemble_relation_ddl(PgDdlRelation {
            schema: "public".to_owned(),
            name: "missing_definition".to_owned(),
            relation_kind: "v".to_owned(),
            persistence: "p".to_owned(),
            view_definition: Some("  ".to_owned()),
            materialized_populated: true,
            partition_key: None,
            partition_parent: None,
            partition_bound: None,
            columns: vec![],
            constraints: vec![],
            relation_comment: None,
            column_comments: vec![],
            indexes: vec![],
            triggers: vec![],
        })
        .unwrap_err();

        assert!(error.message.contains("has no main view definition"));
    }
}
