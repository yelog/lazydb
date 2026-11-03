use std::collections::BTreeMap;

use reqwest::Method;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    db::{
        DatabaseError,
        catalog::{
            CatalogCapabilities, CatalogCount, CatalogEntry, CatalogGroupSummary, CatalogId,
            CatalogKind, CatalogMetadata, CatalogPage, CatalogRequest, CatalogSearchHit,
            CatalogSearchPage, CatalogSearchRequest, CatalogTarget, ColumnMetadata,
            ColumnMetadataCapabilities, NamespaceModel, ObjectGroup, OptionalMetadata,
            QualifiedName, finalize_keyset_page, search_text_matches,
        },
    },
    profile::CatalogScope,
};

use super::{ElasticsearchAdapter, index_path};

impl ElasticsearchAdapter {
    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseIsSchema,
            top_level_groups: vec![ObjectGroup::Tables],
            column_metadata: ColumnMetadataCapabilities::default(),
            supports_lazy_children: true,
        }
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        request
            .validate_for_profile(self.profile_id)
            .map_err(catalog_error)?;
        if self.database.is_empty() {
            return Err(catalog_error(
                "Search REST virtual database cannot be empty",
            ));
        }
        let entries = match &request.key.target {
            CatalogTarget::Databases => {
                if request.scope.allows_database(&self.database) {
                    vec![database_entry(self.profile_id, &self.database)?]
                } else {
                    Vec::new()
                }
            }
            CatalogTarget::Schemas { database } => {
                validate_identity(
                    self.profile_id,
                    &self.database,
                    database,
                    CatalogKind::Database,
                )?;
                vec![schema_entry(self.profile_id, &self.database)?]
                    .into_iter()
                    .filter(|_| request.scope.allows_schema(&self.database, &self.database))
                    .collect()
            }
            CatalogTarget::Groups { schema } => {
                validate_identity(self.profile_id, &self.database, schema, CatalogKind::Schema)?;
                let count = self.index_names().await?.len() as u64;
                let include = request.key.cursor.as_ref().is_none_or(|cursor| {
                    cursor
                        .keyset_parts()
                        .is_ok_and(|parts| parts < ("tables", "tables"))
                });
                return CatalogPage::groups(
                    request,
                    if include {
                        vec![CatalogGroupSummary {
                            group: ObjectGroup::Tables,
                            object_count: CatalogCount::Exact(count),
                        }]
                    } else {
                        Vec::new()
                    },
                    CatalogCount::Exact(1),
                    None,
                )
                .map_err(catalog_error);
            }
            CatalogTarget::Objects { schema, group } => {
                validate_identity(self.profile_id, &self.database, schema, CatalogKind::Schema)?;
                if *group != ObjectGroup::Tables {
                    return Err(DatabaseError::unsupported(
                        "Search REST catalog only supports the Tables (INDEX) group",
                    ));
                }
                self.index_names()
                    .await?
                    .into_iter()
                    .map(|index| index_entry(self.profile_id, &self.database, &index))
                    .collect::<Result<Vec<_>, _>>()?
            }
            CatalogTarget::RelationChildren { relation } => {
                validate_identity(
                    self.profile_id,
                    &self.database,
                    relation,
                    CatalogKind::Table,
                )?;
                let index = &relation.native_path[2];
                let mapping = self
                    .request_json(Method::GET, &index_path(index, "_mapping"), None)
                    .await?;
                mapping_entries(self.profile_id, &self.database, relation, &mapping)?
            }
            CatalogTarget::ProjectionChildren { table } => {
                validate_identity(self.profile_id, &self.database, table, CatalogKind::Table)?;
                return Err(DatabaseError::unsupported(
                    "Search REST catalog does not support projections",
                ));
            }
        };
        entry_page(request, entries)
    }

    pub async fn search_catalog(
        &self,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        request.validate().map_err(catalog_error)?;
        if request.connection.profile_id != self.profile_id {
            return Err(catalog_error(
                "Search REST catalog search belongs to a different profile",
            ));
        }
        if self.database.is_empty() {
            return Err(catalog_error(
                "Search REST virtual database cannot be empty",
            ));
        }
        if !request.scope.allows_schema(&self.database, &self.database) {
            return CatalogSearchPage::new(request, Vec::new(), Some(0), false)
                .map_err(catalog_error);
        }
        let names = self
            .index_names()
            .await?
            .into_iter()
            .filter(|name| search_text_matches(name, &request.query))
            .collect::<Vec<_>>();
        let total = names.len();
        let hits = names
            .into_iter()
            .take(request.limit)
            .map(|name| {
                Ok(CatalogSearchHit {
                    entry: index_entry(self.profile_id, &self.database, &name)?,
                    ancestors: vec![
                        database_entry(self.profile_id, &self.database)?,
                        schema_entry(self.profile_id, &self.database)?,
                    ],
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        CatalogSearchPage::new(request, hits, Some(total), total > request.limit)
            .map_err(catalog_error)
    }

    /// Resolves a captured virtual catalog identity; existence is checked by the native request.
    pub fn resolve_relation_identity(&self, relation: &CatalogId) -> Option<CatalogEntry> {
        validate_identity(
            self.profile_id,
            &self.database,
            relation,
            CatalogKind::Table,
        )
        .ok()?;
        index_entry(self.profile_id, &self.database, &relation.native_path[2]).ok()
    }

    async fn index_names(&self) -> Result<Vec<String>, DatabaseError> {
        let value = self
            .request_json(
                Method::GET,
                "/_cat/indices?format=json&h=index&expand_wildcards=open",
                None,
            )
            .await?;
        parse_index_names(&value)
    }
}

pub(crate) fn validate_relation_scope(
    profile_id: Uuid,
    database: &str,
    relation: &CatalogId,
    scope: &CatalogScope,
) -> Result<(), DatabaseError> {
    scope.validate("", None).map_err(catalog_error)?;
    validate_identity(profile_id, database, relation, CatalogKind::Table)?;
    if !scope.allows_schema(database, database) {
        return Err(catalog_error(
            "Search REST index is outside the captured catalog scope",
        ));
    }
    Ok(())
}

fn validate_identity(
    profile_id: Uuid,
    database: &str,
    id: &CatalogId,
    kind: CatalogKind,
) -> Result<(), DatabaseError> {
    let length = match kind {
        CatalogKind::Database => 1,
        CatalogKind::Schema => 2,
        CatalogKind::Table => 3,
        _ => {
            return Err(catalog_error(
                "unsupported search REST catalog identity kind",
            ));
        }
    };
    if database.is_empty()
        || id.connection_id != profile_id
        || id.kind != kind
        || id.native_path.len() != length
        || id.native_path[0] != database
        || (length >= 2 && id.native_path[1] != database)
        || (length == 3 && !is_user_index(&id.native_path[2]))
    {
        return Err(catalog_error(
            "Search REST catalog identity must belong to this profile and virtual namespace",
        ));
    }
    Ok(())
}

fn is_user_index(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['.', '_', '-', '+'])
        && !name.chars().any(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    '*' | '?' | ',' | '/' | '\\' | '#' | ':' | '"' | '<' | '>' | '|'
                )
        })
}

fn parse_index_names(value: &Value) -> Result<Vec<String>, DatabaseError> {
    let rows = value
        .as_array()
        .ok_or_else(|| catalog_error("Search REST index catalog is not an array"))?;
    let mut names = Vec::new();
    for row in rows {
        let name = row
            .get("index")
            .and_then(Value::as_str)
            .ok_or_else(|| catalog_error("Search REST index catalog has no index name"))?;
        if name.starts_with('.') {
            continue;
        }
        if !is_user_index(name) {
            return Err(catalog_error("Search REST returned an invalid index name"));
        }
        names.push(name.to_owned());
    }
    names.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    names.dedup();
    Ok(names)
}

fn database_entry(profile_id: Uuid, database: &str) -> Result<CatalogEntry, DatabaseError> {
    CatalogEntry::database(
        CatalogId::new(profile_id, CatalogKind::Database, [database]),
        QualifiedName {
            database: Some(database.into()),
            schema: None,
            object: database.into(),
        },
        "VIRTUAL_DATABASE",
        OptionalMetadata::Unsupported,
        true,
    )
    .map_err(catalog_error)
}

fn schema_entry(profile_id: Uuid, database: &str) -> Result<CatalogEntry, DatabaseError> {
    CatalogEntry::schema(
        CatalogId::new(profile_id, CatalogKind::Schema, [database, database]),
        CatalogId::new(profile_id, CatalogKind::Database, [database]),
        qualified(database, database),
        "VIRTUAL_SCHEMA",
        OptionalMetadata::Unsupported,
        true,
    )
    .map_err(catalog_error)
}

fn index_entry(
    profile_id: Uuid,
    database: &str,
    index: &str,
) -> Result<CatalogEntry, DatabaseError> {
    CatalogEntry::relation(
        CatalogId::new(profile_id, CatalogKind::Table, [database, database, index]),
        CatalogId::new(profile_id, CatalogKind::Schema, [database, database]),
        qualified(database, index),
        "INDEX",
        OptionalMetadata::Unsupported,
        true,
    )
    .map_err(catalog_error)
}

fn qualified(database: &str, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.into()),
        schema: Some(database.into()),
        object: object.into(),
    }
}

fn mapping_entries(
    profile_id: Uuid,
    database: &str,
    relation: &CatalogId,
    value: &Value,
) -> Result<Vec<CatalogEntry>, DatabaseError> {
    let index = &relation.native_path[2];
    let mappings = value
        .get(index)
        .and_then(|value| value.get("mappings"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            catalog_error("Search REST response has no mappings for the requested index")
        })?;
    let mut fields = BTreeMap::new();
    if let Some(properties) = mappings.get("properties") {
        collect_fields(properties, "", &mut fields)?;
    }
    let mut fields = fields.into_iter().collect::<Vec<_>>();
    fields.sort_by_cached_key(|(name, _)| (name.to_lowercase(), name.clone()));
    fields
        .into_iter()
        .enumerate()
        .map(|(position, (name, native_type))| {
            let mut path = relation.native_path.clone();
            path.push(name.clone());
            CatalogEntry::relation_child(
                CatalogId::new(profile_id, CatalogKind::Column, path),
                relation.clone(),
                qualified(database, &name),
                native_type.clone(),
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(
                    u32::try_from(position + 1)
                        .map_err(|_| catalog_error("too many search REST mapping fields"))?,
                    native_type,
                    true,
                )),
            )
            .map_err(catalog_error)
        })
        .collect()
}

fn collect_fields(
    value: &Value,
    prefix: &str,
    fields: &mut BTreeMap<String, String>,
) -> Result<(), DatabaseError> {
    let properties = value
        .as_object()
        .ok_or_else(|| catalog_error("Search REST mapping fields must be an object"))?;
    for (name, definition) in properties {
        if name.is_empty() || !definition.is_object() {
            return Err(catalog_error(
                "Search REST mapping has an invalid field definition",
            ));
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let native_type = match definition.get("type") {
            Some(Value::String(value)) if !value.is_empty() => value.as_str(),
            None if definition.get("properties").is_some() => "object",
            None => "unknown",
            _ => {
                return Err(catalog_error(
                    "Search REST mapping field type must be a string",
                ));
            }
        };
        if let Some(previous) = fields.insert(path.clone(), native_type.into())
            && previous != native_type
        {
            return Err(catalog_error(
                "Search REST mapping has conflicting dotted field paths",
            ));
        }
        for key in ["properties", "fields"] {
            if let Some(children) = definition.get(key) {
                collect_fields(children, &path, fields)?;
            }
        }
    }
    Ok(())
}

fn entry_page(
    request: &CatalogRequest,
    mut entries: Vec<CatalogEntry>,
) -> Result<CatalogPage, DatabaseError> {
    entries.sort_by_cached_key(|entry| {
        (
            entry.qualified_name.object.to_lowercase(),
            entry.qualified_name.object.clone(),
        )
    });
    let total = entries.len() as u64;
    if let Some(cursor) = &request.key.cursor {
        let (sort, tie) = cursor.keyset_parts().map_err(catalog_error)?;
        entries.retain(|entry| {
            let name = &entry.qualified_name.object;
            (name.to_lowercase().as_str(), name.as_str()) > (sort, tie)
        });
    }
    let next_cursor = finalize_keyset_page(
        &mut entries,
        request.page_size,
        |entry| entry.qualified_name.object.to_lowercase(),
        |entry| entry.qualified_name.object.clone(),
    )
    .map_err(catalog_error)?;
    CatalogPage::new(request, entries, CatalogCount::Exact(total), next_cursor)
        .map_err(catalog_error)
}

fn catalog_error(error: impl std::fmt::Display) -> DatabaseError {
    DatabaseError::configuration(format!("Search REST catalog: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::catalog::CatalogRequestKey, identity::ConnectionIdentity, profile::CatalogSelection,
    };
    use serde_json::json;

    fn request(profile_id: Uuid, target: CatalogTarget) -> CatalogRequest {
        CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                catalog_epoch: 2,
                request_id: 3,
                target,
                cursor: None,
            },
            scope: CatalogScope {
                databases: CatalogSelection::All,
            },
            page_size: 1,
        }
    }

    #[test]
    fn indices_exclude_system_indices_sort_and_deduplicate() {
        let value = json!([{"index":"z"},{"index":".security"},{"index":"a"},{"index":"a"}]);
        assert_eq!(parse_index_names(&value).unwrap(), vec!["a", "z"]);
        assert!(parse_index_names(&json!({})).is_err());
        assert!(parse_index_names(&json!([{}])).is_err());
        assert!(parse_index_names(&json!([{"index":"logs,*"}])).is_err());
    }

    #[test]
    fn nested_and_multifield_mappings_preserve_dotted_paths_across_pages() {
        let profile = Uuid::new_v4();
        let relation = CatalogId::new(profile, CatalogKind::Table, ["default", "default", "logs"]);
        let value = json!({"logs":{"mappings":{"properties":{
            "user":{"properties":{"name":{"type":"text","fields":{"raw":{"type":"keyword"}}}}},
            "z":{"type":"long"}
        }}}});
        let entries = mapping_entries(profile, "default", &relation, &value).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.qualified_name.object.as_str())
                .collect::<Vec<_>>(),
            vec!["user", "user.name", "user.name.raw", "z"]
        );
        assert_eq!(entries[0].native_kind, "object");
        assert_eq!(entries[2].native_kind, "keyword");
        assert_eq!(
            entries[2].id.native_path,
            ["default", "default", "logs", "user.name.raw"]
        );
        let mut request = request(profile, CatalogTarget::RelationChildren { relation });
        for expected in ["user", "user.name", "user.name.raw", "z"] {
            let page = entry_page(&request, entries.clone()).unwrap();
            assert_eq!(page.entries[0].qualified_name.object, expected);
            assert_eq!(page.total_count, CatalogCount::Exact(4));
            page.validate_for(&request).unwrap();
            request.key.cursor = page.next_cursor;
        }
        assert!(request.key.cursor.is_none());
    }

    #[test]
    fn identities_reject_foreign_namespace_wrong_shape_and_selectors() {
        let profile = Uuid::new_v4();
        let mut id = CatalogId::new(profile, CatalogKind::Table, ["default", "default", "logs"]);
        assert!(validate_identity(profile, "default", &id, CatalogKind::Table).is_ok());
        assert!(validate_identity(Uuid::new_v4(), "default", &id, CatalogKind::Table).is_err());
        for index in ["*", "_all", "logs,other", ".security", "logs/other", ""] {
            id.native_path[2] = index.into();
            assert!(validate_identity(profile, "default", &id, CatalogKind::Table).is_err());
        }
        id.native_path[2] = "logs".into();
        id.native_path[1] = "foreign".into();
        assert!(validate_identity(profile, "default", &id, CatalogKind::Table).is_err());
        id.native_path[1] = "default".into();
        id.native_path.push("extra".into());
        assert!(validate_identity(profile, "default", &id, CatalogKind::Table).is_err());
    }

    #[test]
    fn virtual_namespace_and_index_entries_satisfy_catalog_contract() {
        let profile = Uuid::new_v4();
        let database = database_entry(profile, "virtual").unwrap();
        let schema = schema_entry(profile, "virtual").unwrap();
        let index = index_entry(profile, "virtual", "logs").unwrap();
        for (target, entry) in [
            (CatalogTarget::Databases, database.clone()),
            (
                CatalogTarget::Schemas {
                    database: database.id,
                },
                schema.clone(),
            ),
            (
                CatalogTarget::Objects {
                    schema: schema.id,
                    group: ObjectGroup::Tables,
                },
                index,
            ),
        ] {
            entry_page(&request(profile, target), vec![entry]).unwrap();
        }
    }
}
