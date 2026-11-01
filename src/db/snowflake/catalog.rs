use uuid::Uuid;

use crate::{
    db::{
        DatabaseError,
        catalog::{
            CatalogCapabilities, CatalogCount, CatalogCursor, CatalogDiscovery, CatalogEntry,
            CatalogGroupSummary, CatalogId, CatalogKind, CatalogMetadata, CatalogPage,
            CatalogRequest, CatalogSearchHit, CatalogSearchPage, CatalogSearchRequest,
            CatalogTarget, ColumnMetadata, ColumnMetadataCapabilities, DiscoveredDatabase,
            NamespaceModel, ObjectGroup, OptionalMetadata, QualifiedName, finalize_keyset_page,
            search_query,
        },
        query::{QueryBudget, ResultSet},
        value::CellValue,
    },
    profile::{CatalogScope, CatalogSelection},
};

use super::{SnowflakeAdapter, invalid_response, quote_identifier, quote_literal};

const DISCOVERY_PAGE_SIZE: usize = 500;
const MAX_DISCOVERY_NAMES: usize = 10_000;
const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;

impl SnowflakeAdapter {
    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![ObjectGroup::Tables, ObjectGroup::Views],
            column_metadata: ColumnMetadataCapabilities {
                default_expression: true,
                identity: true,
                numeric_precision_and_scale: true,
                character_length: true,
                comment: true,
                ..Default::default()
            },
            supports_lazy_children: true,
        }
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let all = CatalogScope {
            databases: CatalogSelection::All,
        };
        let mut discovery = CatalogDiscovery {
            databases: Vec::new(),
            warnings: Vec::new(),
        };
        let mut database_after = None;
        let mut names_retained = 0;
        loop {
            let (databases, more_databases) = self
                .discover_database_names_page(database_after.as_deref(), DISCOVERY_PAGE_SIZE)
                .await?;
            for database in databases {
                names_retained += 1;
                if names_retained > MAX_DISCOVERY_NAMES {
                    discovery.warnings.push("Snowflake catalog discovery reached its name limit; use lazy catalog browsing for additional namespaces".into());
                    return Ok(discovery);
                }
                database_after = Some(database.clone());
                let mut schemas = Vec::new();
                let mut schema_after = None;
                loop {
                    let sql = schema_query(
                        &database,
                        &all,
                        &all,
                        schema_after.as_deref(),
                        DISCOVERY_PAGE_SIZE + 1,
                    );
                    let result = match self.catalog_result(&sql, DISCOVERY_PAGE_SIZE + 1).await {
                        Ok(result) => result,
                        Err(error) => {
                            discovery
                                .warnings
                                .push(crate::security::sanitize_terminal_text(&format!(
                                    "Unable to discover schemas for database {database}: {error}"
                                )));
                            break;
                        }
                    };
                    let more_schemas = result.rows.len() > DISCOVERY_PAGE_SIZE;
                    for row in result.rows.iter().take(DISCOVERY_PAGE_SIZE) {
                        let schema = text(row, 0)?.to_owned();
                        names_retained += 1;
                        if names_retained > MAX_DISCOVERY_NAMES {
                            discovery.databases.push(DiscoveredDatabase {
                                name: database,
                                schemas,
                            });
                            discovery.warnings.push("Snowflake catalog discovery reached its name limit; use lazy catalog browsing for additional namespaces".into());
                            return Ok(discovery);
                        }
                        schema_after = Some(schema.clone());
                        schemas.push(schema);
                    }
                    if !more_schemas {
                        break;
                    }
                }
                discovery.databases.push(DiscoveredDatabase {
                    name: database,
                    schemas,
                });
            }
            if !more_databases {
                break;
            }
        }
        Ok(discovery)
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        validate_catalog_request(self.profile_id, &self.catalog_scope, request)?;
        let after = if matches!(request.key.target, CatalogTarget::RelationChildren { .. }) {
            None
        } else {
            name_cursor(request.key.cursor.as_ref())?
        };
        let limit = request.page_size + 1;
        let mut entries = match &request.key.target {
            CatalogTarget::Databases => {
                let (names, has_more) = self
                    .database_names_page(&request.scope, after, request.page_size)
                    .await?;
                let cursor = if has_more {
                    names
                        .last()
                        .map(|name| CatalogCursor::from_keyset(name, name))
                        .transpose()
                        .map_err(catalog_validation)?
                } else {
                    None
                };
                let entries = names
                    .iter()
                    .map(|name| database_entry(self.profile_id, name))
                    .collect::<Result<Vec<_>, _>>()?;
                return CatalogPage::new(request, entries, CatalogCount::Unknown, cursor)
                    .map_err(catalog_validation);
            }
            CatalogTarget::Schemas { database } => {
                let name = &database.native_path[0];
                let sql = schema_query(name, &self.catalog_scope, &request.scope, after, limit);
                self.catalog_result(&sql, limit)
                    .await?
                    .rows
                    .iter()
                    .map(|row| {
                        schema_entry(self.profile_id, name, text(row, 0)?, optional_text(row, 1)?)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            CatalogTarget::Groups { .. } => {
                let mut groups = [
                    ("tables", ObjectGroup::Tables),
                    ("views", ObjectGroup::Views),
                ]
                .into_iter()
                .filter(|(name, _)| after.is_none_or(|after| *name > after))
                .collect::<Vec<_>>();
                let cursor = finalize_keyset_page(
                    &mut groups,
                    request.page_size,
                    |(name, _)| (*name).to_owned(),
                    |(name, _)| (*name).to_owned(),
                )
                .map_err(catalog_validation)?;
                return CatalogPage::groups(
                    request,
                    groups
                        .into_iter()
                        .map(|(_, group)| CatalogGroupSummary {
                            group,
                            object_count: CatalogCount::Unknown,
                        })
                        .collect(),
                    CatalogCount::Unknown,
                    cursor,
                )
                .map_err(catalog_validation);
            }
            CatalogTarget::Objects { schema, group } => {
                let native_kind = match group {
                    ObjectGroup::Tables => "BASE TABLE",
                    ObjectGroup::Views => "VIEW",
                    _ => {
                        return Err(DatabaseError::unsupported(
                            "Snowflake supports only table and view catalog groups",
                        ));
                    }
                };
                let database = &schema.native_path[0];
                let namespace = &schema.native_path[1];
                let sql = object_query(database, namespace, native_kind, after, limit);
                self.catalog_result(&sql, limit)
                    .await?
                    .rows
                    .iter()
                    .map(|row| relation_entry(self.profile_id, database, namespace, row))
                    .collect::<Result<Vec<_>, _>>()?
            }
            CatalogTarget::RelationChildren { relation } => {
                let sql = column_query(relation, request.key.cursor.as_ref(), limit)?;
                // A stale table ID must not expose the columns of a replacement view.
                if self
                    .resolve_relation_identity_with_scope(relation, &request.scope)
                    .await?
                    .is_none()
                {
                    return Err(invalid_response(
                        "Snowflake relation no longer exists with the requested kind",
                    ));
                }
                let mut entries = self
                    .catalog_result(&sql, limit)
                    .await?
                    .rows
                    .iter()
                    .map(|row| column_entry(relation, row))
                    .collect::<Result<Vec<_>, _>>()?;
                let cursor = finalize_keyset_page(
                    &mut entries,
                    request.page_size,
                    column_sort_key,
                    |entry| entry.qualified_name.object.clone(),
                )
                .map_err(catalog_validation)?;
                return CatalogPage::new(request, entries, CatalogCount::Unknown, cursor)
                    .map_err(catalog_validation);
            }
            CatalogTarget::ProjectionChildren { .. } => {
                return Err(DatabaseError::unsupported(
                    "Snowflake projection children are unavailable",
                ));
            }
        };
        let cursor = finalize_keyset_page(
            &mut entries,
            request.page_size,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )
        .map_err(catalog_validation)?;
        CatalogPage::new(request, entries, CatalogCount::Unknown, cursor)
            .map_err(catalog_validation)
    }

    pub async fn search_catalog(
        &self,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        request.validate().map_err(catalog_validation)?;
        if request.connection.profile_id != self.profile_id {
            return Err(DatabaseError::configuration(
                "Snowflake catalog search belongs to a different profile",
            ));
        }
        validate_scopes(&self.catalog_scope, &request.scope)?;
        let mut hits = Vec::new();
        let mut after = None;
        loop {
            let (databases, more) = self
                .database_names_page(&request.scope, after.as_deref(), DISCOVERY_PAGE_SIZE)
                .await?;
            for database in databases {
                let remaining = request.limit + 1 - hits.len();
                let sql = search_query_sql(&database, &self.catalog_scope, request, remaining);
                for row in self.catalog_result(&sql, remaining).await?.rows {
                    let schema = text(&row, 0)?;
                    let entry = relation_entry(self.profile_id, &database, schema, &row[1..])?;
                    if !request.object_scope.includes(entry.kind) {
                        continue;
                    }
                    hits.push(CatalogSearchHit {
                        entry,
                        ancestors: vec![
                            database_entry(self.profile_id, &database)?,
                            schema_entry(self.profile_id, &database, schema, None)?,
                        ],
                    });
                }
                if hits.len() > request.limit {
                    hits.truncate(request.limit);
                    return CatalogSearchPage::new(request, hits, None, true)
                        .map_err(catalog_validation);
                }
                after = Some(database);
            }
            if !more {
                break;
            }
        }
        let count = hits.len();
        CatalogSearchPage::new(request, hits, Some(count), false).map_err(catalog_validation)
    }

    pub async fn resolve_relation_identity(
        &self,
        relation: &CatalogId,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        self.resolve_relation_identity_with_scope(relation, &self.catalog_scope)
            .await
    }

    pub async fn resolve_relation_identity_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        let (database, schema, name) =
            validate_relation_identity(self.profile_id, &self.catalog_scope, scope, relation)?;
        let native_kind = if relation.kind == CatalogKind::View {
            "VIEW"
        } else {
            "BASE TABLE"
        };
        let sql = format!(
            "SELECT TABLE_NAME, TABLE_TYPE, COMMENT FROM {} WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {} AND TABLE_TYPE = {} LIMIT 1",
            information_schema(database, "TABLES"),
            quote_literal(schema),
            quote_literal(name),
            quote_literal(native_kind),
        );
        self.catalog_result(&sql, 1)
            .await?
            .rows
            .first()
            .map(|row| relation_entry(self.profile_id, database, schema, row))
            .transpose()
    }

    async fn database_names_page(
        &self,
        scope: &CatalogScope,
        after: Option<&str>,
        size: usize,
    ) -> Result<(Vec<String>, bool), DatabaseError> {
        let Some(names) = selected_database_names(&self.catalog_scope, scope)? else {
            return self.discover_database_names_page(after, size).await;
        };
        let mut names = names
            .into_iter()
            .filter(|name| after.is_none_or(|after| name.as_str() > after))
            .take(size + 1)
            .collect::<Vec<_>>();
        let has_more = names.len() > size;
        names.truncate(size);
        Ok((names, has_more))
    }

    async fn discover_database_names_page(
        &self,
        after: Option<&str>,
        size: usize,
    ) -> Result<(Vec<String>, bool), DatabaseError> {
        let sql = show_databases_query(after, size + 1);
        let result = self.catalog_result(&sql, size + 1).await?;
        let name_index = result
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case("name"))
            .ok_or_else(|| invalid_response("Snowflake SHOW DATABASES returned no name column"))?;
        let mut names = result
            .rows
            .iter()
            .map(|row| text(row, name_index).map(str::to_owned))
            .collect::<Result<Vec<_>, _>>()?;
        if names.windows(2).any(|pair| pair[0] >= pair[1])
            || names
                .first()
                .is_some_and(|name| after.is_some_and(|after| name.as_str() <= after))
        {
            return Err(invalid_response(
                "Snowflake database pagination did not advance in name order",
            ));
        }
        let has_more = names.len() > size;
        names.truncate(size);
        Ok((names, has_more))
    }

    pub(super) async fn catalog_result(
        &self,
        sql: &str,
        max_rows: usize,
    ) -> Result<ResultSet, DatabaseError> {
        let mut result = self
            .execute_with_budget(
                sql,
                QueryBudget {
                    max_rows,
                    max_bytes: MAX_CATALOG_BYTES,
                },
            )
            .await?;
        if result.stats.truncated {
            return Err(invalid_response(
                "Snowflake metadata exceeded its bounded result budget",
            ));
        }
        if result.result_sets.len() != 1 {
            return Err(invalid_response(
                "Snowflake metadata query returned an invalid number of result sets",
            ));
        }
        Ok(result.result_sets.remove(0))
    }
}

fn catalog_validation(error: impl std::fmt::Display) -> DatabaseError {
    DatabaseError::configuration(error.to_string())
}

pub(super) fn validate_scopes(
    profile: &CatalogScope,
    requested: &CatalogScope,
) -> Result<(), DatabaseError> {
    profile.validate("", None).map_err(catalog_validation)?;
    requested.validate("", None).map_err(catalog_validation)
}

fn validate_catalog_request(
    profile_id: Uuid,
    profile: &CatalogScope,
    request: &CatalogRequest,
) -> Result<(), DatabaseError> {
    request
        .validate_for_profile(profile_id)
        .map_err(catalog_validation)?;
    validate_scopes(profile, &request.scope)?;
    let target_id = match &request.key.target {
        CatalogTarget::Databases => None,
        CatalogTarget::Schemas { database } => Some(database),
        CatalogTarget::Groups { schema } | CatalogTarget::Objects { schema, .. } => Some(schema),
        CatalogTarget::RelationChildren { relation } => Some(relation),
        CatalogTarget::ProjectionChildren { table } => Some(table),
    };
    if target_id.is_some_and(|id| id.native_path.iter().any(|name| name.contains('\0'))) {
        return Err(DatabaseError::configuration(
            "Snowflake native names must contain no NUL",
        ));
    }
    let id = match &request.key.target {
        CatalogTarget::Databases => return Ok(()),
        CatalogTarget::Schemas { database } => {
            if !profile.allows_database(&database.native_path[0]) {
                return Err(DatabaseError::configuration(
                    "Snowflake database is outside profile catalog scope",
                ));
            }
            return Ok(());
        }
        CatalogTarget::Groups { schema } | CatalogTarget::Objects { schema, .. } => schema,
        CatalogTarget::RelationChildren { relation } => {
            validate_relation_identity(profile_id, profile, &request.scope, relation)?;
            return Ok(());
        }
        CatalogTarget::ProjectionChildren { table } => {
            validate_relation_identity(profile_id, profile, &request.scope, table)?;
            return Ok(());
        }
    };
    if !profile.allows_schema(&id.native_path[0], &id.native_path[1]) {
        return Err(DatabaseError::configuration(
            "Snowflake schema is outside profile catalog scope",
        ));
    }
    Ok(())
}

pub(super) fn validate_relation_identity<'a>(
    profile_id: Uuid,
    profile: &CatalogScope,
    requested: &CatalogScope,
    relation: &'a CatalogId,
) -> Result<(&'a str, &'a str, &'a str), DatabaseError> {
    validate_scopes(profile, requested)?;
    if relation.connection_id != profile_id
        || !matches!(relation.kind, CatalogKind::Table | CatalogKind::View)
    {
        return Err(DatabaseError::configuration(
            "Snowflake relation requires a table or view identity from this profile",
        ));
    }
    let [database, schema, name] = relation.native_path.as_slice() else {
        return Err(DatabaseError::configuration(
            "Snowflake relation identity must contain exactly database, schema, and object names",
        ));
    };
    if [database, schema, name]
        .iter()
        .any(|name| name.is_empty() || name.contains('\0'))
    {
        return Err(DatabaseError::configuration(
            "Snowflake native names must be nonempty and contain no NUL",
        ));
    }
    if !profile.allows_schema(database, schema) || !requested.allows_schema(database, schema) {
        return Err(DatabaseError::configuration(
            "Snowflake relation is outside profile or request catalog scope",
        ));
    }
    Ok((database, schema, name))
}

fn selected_database_names(
    profile: &CatalogScope,
    requested: &CatalogScope,
) -> Result<Option<Vec<String>>, DatabaseError> {
    validate_scopes(profile, requested)?;
    let selected = match (&profile.databases, &requested.databases) {
        (CatalogSelection::Selected(names), _) | (_, CatalogSelection::Selected(names)) => names,
        (CatalogSelection::All, CatalogSelection::All) => return Ok(None),
    };
    let mut names = selected
        .iter()
        .filter(|database| {
            profile.allows_database(&database.name) && requested.allows_database(&database.name)
        })
        .filter(|database| schema_selections_overlap(profile, requested, &database.name))
        .map(|database| database.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(Some(names))
}

fn schemas_for_database<'a>(
    scope: &'a CatalogScope,
    database: &str,
) -> Option<&'a CatalogSelection<String>> {
    match &scope.databases {
        CatalogSelection::All => None,
        CatalogSelection::Selected(databases) => databases
            .iter()
            .find(|entry| entry.name == database)
            .map(|entry| &entry.schemas),
    }
}

fn schema_selections_overlap(
    profile: &CatalogScope,
    requested: &CatalogScope,
    database: &str,
) -> bool {
    match (
        schemas_for_database(profile, database),
        schemas_for_database(requested, database),
    ) {
        (Some(CatalogSelection::Selected(left)), Some(CatalogSelection::Selected(right))) => {
            left.iter().any(|name| right.contains(name))
        }
        _ => true,
    }
}

fn schema_scope_predicate(
    profile: &CatalogScope,
    requested: &CatalogScope,
    database: &str,
    column: &str,
) -> String {
    let mut clauses = Vec::new();
    for scope in [profile, requested] {
        if !scope.allows_database(database) {
            return "FALSE".into();
        }
        if let Some(CatalogSelection::Selected(schemas)) = schemas_for_database(scope, database) {
            clauses.push(format!(
                "{column} IN ({})",
                schemas
                    .iter()
                    .map(|name| quote_literal(name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if clauses.is_empty() {
        "TRUE".into()
    } else {
        clauses.join(" AND ")
    }
}

pub(super) fn information_schema(database: &str, table: &str) -> String {
    format!(
        "{}.\"INFORMATION_SCHEMA\".{}",
        quote_identifier(database),
        quote_identifier(table)
    )
}

fn name_cursor(cursor: Option<&CatalogCursor>) -> Result<Option<&str>, DatabaseError> {
    cursor
        .map(|cursor| {
            let (sort, tie) = cursor.keyset_parts().map_err(catalog_validation)?;
            if sort != tie {
                return Err(DatabaseError::configuration(
                    "invalid Snowflake catalog name cursor",
                ));
            }
            Ok(tie)
        })
        .transpose()
}

fn show_databases_query(after: Option<&str>, limit: usize) -> String {
    let mut sql = format!("SHOW DATABASES LIMIT {limit}");
    if let Some(after) = after {
        sql.push_str(" FROM ");
        sql.push_str(&quote_literal(after));
    }
    sql
}

fn schema_query(
    database: &str,
    profile: &CatalogScope,
    requested: &CatalogScope,
    after: Option<&str>,
    limit: usize,
) -> String {
    let mut sql = format!(
        "SELECT SCHEMA_NAME, COMMENT FROM {} WHERE {}",
        information_schema(database, "SCHEMATA"),
        schema_scope_predicate(profile, requested, database, "SCHEMA_NAME")
    );
    append_name_keyset(&mut sql, "SCHEMA_NAME", after, limit);
    sql
}

fn object_query(
    database: &str,
    schema: &str,
    native_kind: &str,
    after: Option<&str>,
    limit: usize,
) -> String {
    let mut sql = format!(
        "SELECT TABLE_NAME, TABLE_TYPE, COMMENT FROM {} WHERE TABLE_SCHEMA = {} AND TABLE_TYPE = {}",
        information_schema(database, "TABLES"),
        quote_literal(schema),
        quote_literal(native_kind)
    );
    append_name_keyset(&mut sql, "TABLE_NAME", after, limit);
    sql
}

fn append_name_keyset(sql: &mut String, column: &str, after: Option<&str>, limit: usize) {
    if let Some(after) = after {
        sql.push_str(&format!(" AND {column} > {}", quote_literal(after)));
    }
    sql.push_str(&format!(" ORDER BY {column} LIMIT {limit}"));
}

fn column_query(
    relation: &CatalogId,
    cursor: Option<&CatalogCursor>,
    limit: usize,
) -> Result<String, DatabaseError> {
    let [database, schema, table] = relation.native_path.as_slice() else {
        return Err(DatabaseError::configuration(
            "invalid Snowflake column owner path",
        ));
    };
    let mut sql = format!(
        "SELECT COLUMN_NAME, DATA_TYPE, ORDINAL_POSITION, IS_NULLABLE, COLUMN_DEFAULT, COMMENT, IS_IDENTITY, NUMERIC_PRECISION, NUMERIC_SCALE, CHARACTER_MAXIMUM_LENGTH, COLLATION_NAME, CHARACTER_SET_NAME FROM {} WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {}",
        information_schema(database, "COLUMNS"),
        quote_literal(schema),
        quote_literal(table)
    );
    if let Some(cursor) = cursor {
        let (sort, tie) = cursor.keyset_parts().map_err(catalog_validation)?;
        let ordinal = sort
            .parse::<u32>()
            .map_err(|_| DatabaseError::configuration("invalid Snowflake column cursor ordinal"))?;
        if sort != format!("{ordinal:020}") || ordinal == 0 {
            return Err(DatabaseError::configuration(
                "invalid Snowflake column cursor ordinal",
            ));
        }
        sql.push_str(&format!(" AND (ORDINAL_POSITION > {ordinal} OR (ORDINAL_POSITION = {ordinal} AND COLUMN_NAME > {}))", quote_literal(tie)));
    }
    sql.push_str(&format!(
        " ORDER BY ORDINAL_POSITION, COLUMN_NAME LIMIT {limit}"
    ));
    Ok(sql)
}

fn search_query_sql(
    database: &str,
    profile: &CatalogScope,
    request: &CatalogSearchRequest,
    limit: usize,
) -> String {
    let (query, normalized) = search_query(&request.query);
    let searchable = |column: &str| {
        if normalized {
            format!("REGEXP_REPLACE(LOWER(COALESCE({column}, '')), '[^[:alnum:]]', '')")
        } else {
            format!("LOWER(COALESCE({column}, ''))")
        }
    };
    format!(
        "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, COMMENT FROM {} WHERE TABLE_TYPE IN ('BASE TABLE', 'VIEW') AND {} AND (CONTAINS({}, {}) OR CONTAINS({}, {})) ORDER BY TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE LIMIT {limit}",
        information_schema(database, "TABLES"),
        schema_scope_predicate(profile, &request.scope, database, "TABLE_SCHEMA"),
        searchable("TABLE_NAME"),
        quote_literal(&query),
        searchable("COMMENT"),
        quote_literal(&query)
    )
}

fn database_entry(profile: Uuid, name: &str) -> Result<CatalogEntry, DatabaseError> {
    CatalogEntry::database(
        CatalogId::new(profile, CatalogKind::Database, [name]),
        QualifiedName {
            database: Some(name.to_owned()),
            schema: None,
            object: name.to_owned(),
        },
        "DATABASE",
        OptionalMetadata::Unsupported,
        true,
    )
    .map_err(catalog_validation)
}

fn schema_entry(
    profile: Uuid,
    database: &str,
    schema: &str,
    comment: Option<String>,
) -> Result<CatalogEntry, DatabaseError> {
    CatalogEntry::schema(
        CatalogId::new(profile, CatalogKind::Schema, [database, schema]),
        CatalogId::new(profile, CatalogKind::Database, [database]),
        QualifiedName {
            database: Some(database.to_owned()),
            schema: Some(schema.to_owned()),
            object: schema.to_owned(),
        },
        "SCHEMA",
        OptionalMetadata::Supported(comment),
        true,
    )
    .map_err(catalog_validation)
}

fn relation_entry(
    profile: Uuid,
    database: &str,
    schema: &str,
    row: &[CellValue],
) -> Result<CatalogEntry, DatabaseError> {
    let name = text(row, 0)?;
    let native_kind = text(row, 1)?;
    let kind = match native_kind {
        "BASE TABLE" => CatalogKind::Table,
        "VIEW" => CatalogKind::View,
        _ => {
            return Err(invalid_response(
                "Snowflake returned an unsupported relation type",
            ));
        }
    };
    CatalogEntry::relation(
        CatalogId::new(profile, kind, [database, schema, name]),
        CatalogId::new(profile, CatalogKind::Schema, [database, schema]),
        QualifiedName {
            database: Some(database.to_owned()),
            schema: Some(schema.to_owned()),
            object: name.to_owned(),
        },
        native_kind,
        OptionalMetadata::Supported(optional_text(row, 2)?),
        true,
    )
    .map_err(catalog_validation)
}

fn column_entry(relation: &CatalogId, row: &[CellValue]) -> Result<CatalogEntry, DatabaseError> {
    let name = text(row, 0)?;
    let native_type = text(row, 1)?;
    let ordinal =
        unsigned(row, 2)?.ok_or_else(|| invalid_response("Snowflake column ordinal is missing"))?;
    let ordinal = u32::try_from(ordinal)
        .ok()
        .filter(|ordinal| *ordinal > 0)
        .ok_or_else(|| invalid_response("Snowflake column ordinal is invalid"))?;
    let mut metadata = ColumnMetadata::new(ordinal, native_type, yes_no(row, 3)?);
    metadata.default_expression = OptionalMetadata::Supported(optional_text(row, 4)?);
    metadata.comment = OptionalMetadata::Supported(optional_text(row, 5)?);
    metadata.identity = OptionalMetadata::Supported(Some(yes_no(row, 6)?));
    metadata.numeric_precision = OptionalMetadata::Supported(optional_u32(row, 7)?);
    metadata.numeric_scale = OptionalMetadata::Supported(optional_u32(row, 8)?);
    metadata.character_maximum_length = OptionalMetadata::Supported(unsigned(row, 9)?);
    // Snowflake documents COLLATION_NAME and CHARACTER_SET_NAME as not applicable.
    metadata.collation = OptionalMetadata::Unsupported;
    metadata.character_set = OptionalMetadata::Unsupported;
    let mut path = relation.native_path.clone();
    path.push(name.to_owned());
    CatalogEntry::relation_child(
        CatalogId::new(relation.connection_id, CatalogKind::Column, path),
        relation.clone(),
        QualifiedName {
            database: Some(relation.native_path[0].clone()),
            schema: Some(relation.native_path[1].clone()),
            object: name.to_owned(),
        },
        native_type,
        metadata.comment.clone(),
        CatalogMetadata::Column(metadata),
    )
    .map_err(catalog_validation)
}

fn column_sort_key(entry: &CatalogEntry) -> String {
    match &entry.metadata {
        CatalogMetadata::Column(column) => format!("{:020}", column.ordinal_position),
        _ => unreachable!("column pages contain only column entries"),
    }
}

pub(super) fn text(row: &[CellValue], index: usize) -> Result<&str, DatabaseError> {
    match row.get(index) {
        Some(CellValue::Text(value)) if !value.is_empty() => Ok(value),
        _ => Err(invalid_response(
            "Snowflake catalog response contains a missing or invalid text field",
        )),
    }
}

fn optional_text(row: &[CellValue], index: usize) -> Result<Option<String>, DatabaseError> {
    match row.get(index) {
        Some(CellValue::Text(value)) => Ok((!value.is_empty()).then(|| value.clone())),
        Some(CellValue::Null) => Ok(None),
        _ => Err(invalid_response(
            "Snowflake catalog response contains an invalid optional text field",
        )),
    }
}

pub(super) fn unsigned(row: &[CellValue], index: usize) -> Result<Option<u64>, DatabaseError> {
    match row.get(index) {
        Some(CellValue::Unsigned(value)) => Ok(Some(*value)),
        Some(CellValue::Integer(value)) if *value >= 0 => Ok(Some(*value as u64)),
        Some(CellValue::Text(value)) => value
            .parse()
            .map(Some)
            .map_err(|_| invalid_response("Snowflake catalog response contains an invalid number")),
        Some(CellValue::Null) => Ok(None),
        _ => Err(invalid_response(
            "Snowflake catalog response contains an invalid number",
        )),
    }
}

fn optional_u32(row: &[CellValue], index: usize) -> Result<Option<u32>, DatabaseError> {
    unsigned(row, index)?
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| invalid_response("Snowflake catalog numeric metadata is out of range"))
        })
        .transpose()
}

fn yes_no(row: &[CellValue], index: usize) -> Result<bool, DatabaseError> {
    match row.get(index) {
        Some(CellValue::Text(value)) if value == "YES" => Ok(true),
        Some(CellValue::Text(value)) if value == "NO" => Ok(false),
        Some(CellValue::Boolean(value)) => Ok(*value),
        _ => Err(invalid_response(
            "Snowflake catalog response contains an invalid YES/NO value",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::catalog::{CatalogRequestKey, CatalogSearchObjectScope};
    use crate::{identity::ConnectionIdentity, profile::DatabaseScope};

    fn scope(database: &str, schemas: &[&str]) -> CatalogScope {
        CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: database.into(),
                schemas: CatalogSelection::Selected(
                    schemas.iter().map(|value| (*value).into()).collect(),
                ),
            }]),
        }
    }

    #[test]
    fn selected_names_intersect_database_and_schema_scopes_without_show() {
        let all = CatalogScope {
            databases: CatalogSelection::All,
        };
        let profile = scope("DB", &["PUBLIC"]);
        assert_eq!(
            selected_database_names(&profile, &all).unwrap(),
            Some(vec!["DB".into()])
        );
        assert_eq!(
            selected_database_names(&all, &profile).unwrap(),
            Some(vec!["DB".into()])
        );
        assert_eq!(
            selected_database_names(&profile, &scope("OTHER", &["PUBLIC"])).unwrap(),
            Some(vec![])
        );
        assert_eq!(
            selected_database_names(&profile, &scope("DB", &["PRIVATE"])).unwrap(),
            Some(vec![])
        );
        assert_eq!(selected_database_names(&all, &all).unwrap(), None);
    }

    #[test]
    fn requests_reject_wrong_profile_paths_and_both_scope_boundaries() {
        let profile = Uuid::new_v4();
        let captured = scope("DB", &["PUBLIC"]);
        let mut request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: profile,
                    generation: 1,
                },
                catalog_epoch: 0,
                request_id: 1,
                target: CatalogTarget::RelationChildren {
                    relation: CatalogId::new(profile, CatalogKind::Table, ["DB", "PUBLIC", "T"]),
                },
                cursor: None,
            },
            scope: captured.clone(),
            page_size: 10,
        };
        assert!(validate_catalog_request(profile, &captured, &request).is_ok());
        assert!(validate_catalog_request(Uuid::new_v4(), &captured, &request).is_err());
        assert!(validate_catalog_request(profile, &scope("DB", &["PRIVATE"]), &request).is_err());
        request.scope = scope("DB", &["PRIVATE"]);
        assert!(validate_catalog_request(profile, &captured, &request).is_err());
        request.scope = captured.clone();
        if let CatalogTarget::RelationChildren { relation } = &mut request.key.target {
            relation.native_path.push("extra".into());
        }
        assert!(validate_catalog_request(profile, &captured, &request).is_err());
    }

    #[test]
    fn metadata_sql_quotes_identifiers_literals_and_advances_strictly() {
        let sql = object_query("Mixed.\"Db", "s'chema", "VIEW", Some("a'b\\c"), 11);
        assert!(sql.contains("\"Mixed.\"\"Db\".\"INFORMATION_SCHEMA\".\"TABLES\""));
        assert!(sql.contains("TABLE_SCHEMA = 's''chema'"));
        assert!(sql.contains("TABLE_NAME > 'a''b\\\\c'"));
        assert!(sql.ends_with("ORDER BY TABLE_NAME LIMIT 11"));
        assert_eq!(
            show_databases_query(Some("O'Reilly"), 11),
            "SHOW DATABASES LIMIT 11 FROM 'O''Reilly'"
        );
        let relation = CatalogId::new(Uuid::new_v4(), CatalogKind::Table, ["DB", "S", "T"]);
        let cursor = CatalogCursor::from_keyset("00000000000000000009", "quoted'col").unwrap();
        let sql = column_query(&relation, Some(&cursor), 11).unwrap();
        assert!(sql.contains(
            "ORDINAL_POSITION > 9 OR (ORDINAL_POSITION = 9 AND COLUMN_NAME > 'quoted''col')"
        ));
        assert!(!sql.contains("COLUMN_NAME >="));
        assert!(
            column_query(
                &relation,
                Some(&CatalogCursor::from_keyset("9", "C").unwrap()),
                11
            )
            .is_err()
        );
    }

    #[test]
    fn search_is_limited_scoped_and_has_valid_ancestors() {
        let profile = Uuid::new_v4();
        let request = CatalogSearchRequest {
            connection: ConnectionIdentity {
                profile_id: profile,
                generation: 1,
            },
            session_id: 1,
            generation: 1,
            query: "user_name".into(),
            scope: scope("DB", &["PUBLIC", "EXTRA"]),
            object_scope: CatalogSearchObjectScope::RelationsOnly,
            limit: 10,
        };
        let sql = search_query_sql("DB", &scope("DB", &["PUBLIC"]), &request, 11);
        assert!(sql.contains("TABLE_SCHEMA IN ('PUBLIC') AND TABLE_SCHEMA IN ('PUBLIC', 'EXTRA')"));
        assert!(sql.contains("TABLE_TYPE IN ('BASE TABLE', 'VIEW')"));
        assert!(sql.contains("'username'"));
        assert!(sql.ends_with("LIMIT 11"));
        let row = vec![
            CellValue::Text("USERS".into()),
            CellValue::Text("BASE TABLE".into()),
            CellValue::Null,
        ];
        let hit = CatalogSearchHit {
            entry: relation_entry(profile, "DB", "PUBLIC", &row).unwrap(),
            ancestors: vec![
                database_entry(profile, "DB").unwrap(),
                schema_entry(profile, "DB", "PUBLIC", None).unwrap(),
            ],
        };
        assert!(CatalogSearchPage::new(&request, vec![hit], Some(1), false).is_ok());
    }

    #[test]
    fn column_pages_preserve_ordinal_order_and_continue_after_last_visible_row() {
        let relation = CatalogId::new(Uuid::new_v4(), CatalogKind::Table, ["DB", "S", "T"]);
        let make_column = |ordinal: i64, name: &str| {
            column_entry(
                &relation,
                &[
                    CellValue::Text(name.into()),
                    CellValue::Text("TEXT".into()),
                    CellValue::Integer(ordinal),
                    CellValue::Text("YES".into()),
                    CellValue::Null,
                    CellValue::Null,
                    CellValue::Text("NO".into()),
                    CellValue::Null,
                    CellValue::Null,
                    CellValue::Integer(255),
                    CellValue::Null,
                    CellValue::Null,
                ],
            )
            .unwrap()
        };
        let mut entries = vec![
            make_column(9, "Z"),
            make_column(10, "A"),
            make_column(11, "B"),
        ];
        let cursor = finalize_keyset_page(&mut entries, 2, column_sort_key, |entry| {
            entry.qualified_name.object.clone()
        })
        .unwrap()
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            cursor.keyset_parts().unwrap(),
            ("00000000000000000010", "A")
        );
        let sql = column_query(&relation, Some(&cursor), 3).unwrap();
        assert!(sql.contains("ORDINAL_POSITION > 10"));
        let mut next = vec![make_column(11, "B")];
        assert!(
            finalize_keyset_page(&mut next, 2, column_sort_key, |entry| entry
                .qualified_name
                .object
                .clone())
            .unwrap()
            .is_none()
        );
        assert!(name_cursor(Some(&cursor)).is_err());
    }

    #[test]
    fn column_metadata_preserves_nulls_precision_and_owning_identity() {
        let relation = CatalogId::new(Uuid::new_v4(), CatalogKind::View, ["DB", "S", "V"]);
        let row = vec![
            CellValue::Text("AMOUNT".into()),
            CellValue::Text("NUMBER".into()),
            CellValue::Integer(2),
            CellValue::Text("YES".into()),
            CellValue::Null,
            CellValue::Text("money".into()),
            CellValue::Text("NO".into()),
            CellValue::Integer(38),
            CellValue::Integer(9),
            CellValue::Null,
            CellValue::Null,
            CellValue::Null,
        ];
        let entry = column_entry(&relation, &row).unwrap();
        assert_eq!(entry.id.native_path, ["DB", "S", "V", "AMOUNT"]);
        assert_eq!(entry.parent_id.as_ref(), Some(&relation));
        let CatalogMetadata::Column(metadata) = entry.metadata else {
            panic!("expected column metadata")
        };
        assert!(metadata.nullable);
        assert_eq!(
            metadata.numeric_precision,
            OptionalMetadata::Supported(Some(38))
        );
        assert_eq!(metadata.numeric_scale, OptionalMetadata::Supported(Some(9)));
        assert_eq!(
            metadata.character_maximum_length,
            OptionalMetadata::Supported(None)
        );
        assert_eq!(
            metadata.default_expression,
            OptionalMetadata::Supported(None)
        );
    }
}
