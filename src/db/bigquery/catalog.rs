//! Metadata listing uses REST and never submits a billable query.
//! API contracts: https://cloud.google.com/bigquery/docs/reference/rest/v2/projects/list
//! https://cloud.google.com/bigquery/docs/reference/rest/v2/datasets/list
//! https://cloud.google.com/bigquery/docs/reference/rest/v2/tables/list
//! https://cloud.google.com/bigquery/docs/reference/rest/v2/tables/get
//! https://cloud.google.com/bigquery/docs/reference/rest/v2/tables#TableFieldSchema

use std::collections::BTreeSet;

use serde_json::Value;
use uuid::Uuid;

use crate::{
    db::{
        DatabaseError, ErrorCategory,
        catalog::{
            CatalogCapabilities, CatalogCount, CatalogCursor, CatalogDiscovery, CatalogEntry,
            CatalogGroupSummary, CatalogId, CatalogKind, CatalogMetadata, CatalogPage,
            CatalogRequest, CatalogSearchHit, CatalogSearchPage, CatalogSearchRequest,
            CatalogTarget, ColumnMetadata, ColumnMetadataCapabilities, DiscoveredDatabase,
            NamespaceModel, ObjectGroup, OptionalMetadata, QualifiedName, finalize_keyset_page,
            search_text_matches,
        },
    },
    profile::{CatalogScope, CatalogSelection},
};

use super::{BigQueryAdapter, invalid_response, quote_identifier};

const MAX_NAMES: usize = 10_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_REST_PAGES: usize = 100;
const GROUPS: [(&str, ObjectGroup); 3] = [
    ("materialized_views", ObjectGroup::MaterializedViews),
    ("tables", ObjectGroup::Tables),
    ("views", ObjectGroup::Views),
];
const PROJECT_DISCOVERY_WARNING: &str = "BigQuery projects.list only discovers projects with a project-level role and the BigQuery API enabled; dataset-only grants in other projects require explicitly selecting their project";

struct Enumeration {
    items: Vec<Value>,
    warnings: Vec<String>,
}

impl BigQueryAdapter {
    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: GROUPS.iter().map(|(_, group)| *group).collect(),
            column_metadata: ColumnMetadataCapabilities {
                default_expression: true,
                numeric_precision_and_scale: true,
                character_length: true,
                collation: true,
                comment: true,
                ..Default::default()
            },
            supports_lazy_children: true,
        }
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let mut warnings = Vec::new();
        let projects = self
            .project_names(&self.catalog_scope, &mut warnings)
            .await?;
        let mut databases = Vec::new();
        let mut retained = projects.len();
        for project in projects {
            let schemas = match self.dataset_names(&project, &self.catalog_scope).await {
                Ok((schemas, partial)) => {
                    warnings.extend(partial);
                    schemas
                }
                Err(error) if error.category == ErrorCategory::Permission => {
                    warnings.push(warning(&format!(
                        "Unable to discover datasets in {project}: {error}"
                    )));
                    configured_datasets(self, &project, &self.catalog_scope)
                }
                Err(error) => return Err(error),
            };
            retained += schemas.len();
            if retained > MAX_NAMES {
                warnings.push(
                    "BigQuery discovery reached its name limit; narrow the selected catalog scope"
                        .into(),
                );
                break;
            }
            databases.push(DiscoveredDatabase {
                name: project,
                schemas,
            });
        }
        Ok(CatalogDiscovery {
            databases,
            warnings,
        })
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        validate_catalog_request(self.profile_id, &self.catalog_scope, request)?;
        let after = name_cursor(request.key.cursor.as_ref())?;
        let mut entries = match &request.key.target {
            CatalogTarget::Databases => {
                let mut warnings = Vec::new();
                self.project_names(&request.scope, &mut warnings)
                    .await?
                    .iter()
                    .map(|project| database_entry(self.profile_id, project))
                    .collect::<Result<Vec<_>, _>>()?
            }
            CatalogTarget::Schemas { database } => {
                let project = &database.native_path[0];
                let (names, warnings) = self.dataset_names(project, &request.scope).await?;
                require_complete(&warnings)?;
                names
                    .iter()
                    .map(|dataset| schema_entry(self.profile_id, project, dataset))
                    .collect::<Result<Vec<_>, _>>()?
            }
            CatalogTarget::Groups { .. } => {
                let mut groups = GROUPS
                    .into_iter()
                    .filter(|(name, _)| after.is_none_or(|after| *name > after))
                    .collect::<Vec<_>>();
                let cursor = finalize_keyset_page(
                    &mut groups,
                    request.page_size,
                    |(name, _)| (*name).into(),
                    |(name, _)| (*name).into(),
                )
                .map_err(validation)?;
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
                .map_err(validation);
            }
            CatalogTarget::Objects { schema, group } => {
                if !GROUPS.iter().any(|(_, supported)| supported == group) {
                    return Err(DatabaseError::unsupported(
                        "BigQuery supports tables, views, and materialized views",
                    ));
                }
                self.table_entries(&schema.native_path[0], &schema.native_path[1])
                    .await?
                    .into_iter()
                    .filter(|entry| group.contains_kind(entry.kind))
                    .collect()
            }
            CatalogTarget::RelationChildren { relation } => {
                let Some((_, table)) = self.relation_resource(relation, &request.scope).await?
                else {
                    return Err(invalid_response(
                        "BigQuery relation no longer exists with the requested kind",
                    ));
                };
                column_entries(relation, &table)?
            }
            CatalogTarget::ProjectionChildren { .. } => {
                return Err(DatabaseError::unsupported(
                    "BigQuery projection children are unavailable",
                ));
            }
        };
        // REST list order and opaque tokens are not CatalogCursor keysets. Enumerate within
        // explicit bounds, sort native names, and resume strictly after the visible key.
        entries.sort_by(|left, right| left.qualified_name.object.cmp(&right.qualified_name.object));
        entries
            .retain(|entry| after.is_none_or(|after| entry.qualified_name.object.as_str() > after));
        let cursor = finalize_keyset_page(
            &mut entries,
            request.page_size,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )
        .map_err(validation)?;
        CatalogPage::new(request, entries, CatalogCount::Unknown, cursor).map_err(validation)
    }

    pub async fn search_catalog(
        &self,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        request.validate().map_err(validation)?;
        if request.connection.profile_id != self.profile_id {
            return Err(DatabaseError::configuration(
                "BigQuery search belongs to another profile",
            ));
        }
        validate_scopes(&self.catalog_scope, &request.scope)?;
        // projects.list cannot prove that dataset-only grants in unlisted projects do not exist.
        if selected_projects(&self.catalog_scope, &request.scope).is_none() {
            return Err(DatabaseError::unsupported(
                "BigQuery cannot exhaustively search All projects through projects.list; select the projects to search",
            ));
        }
        let mut warnings = Vec::new();
        let projects = self.project_names(&request.scope, &mut warnings).await?;
        require_complete(&warnings)?;
        let mut hits = Vec::new();
        let mut examined = projects.len();
        for project in projects {
            let (datasets, warnings) = self.dataset_names(&project, &request.scope).await?;
            require_complete(&warnings)?;
            examined += datasets.len();
            for dataset in datasets {
                let entries = self.table_entries(&project, &dataset).await?;
                examined += entries.len();
                if examined > MAX_NAMES {
                    return Err(DatabaseError::unsupported(
                        "BigQuery search exceeded its enumeration budget; narrow the selected scope",
                    ));
                }
                for entry in entries {
                    if request.object_scope.includes(entry.kind)
                        && (search_text_matches(&entry.qualified_name.object, &request.query)
                            || matches!(&entry.comment, OptionalMetadata::Supported(Some(comment)) if search_text_matches(comment, &request.query)))
                    {
                        hits.push(CatalogSearchHit {
                            entry,
                            ancestors: vec![
                                database_entry(self.profile_id, &project)?,
                                schema_entry(self.profile_id, &project, &dataset)?,
                            ],
                        });
                    }
                }
            }
        }
        let count = hits.len();
        hits.truncate(request.limit);
        CatalogSearchPage::new(request, hits, Some(count), count > request.limit)
            .map_err(validation)
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
        Ok(self
            .relation_resource(relation, scope)
            .await?
            .map(|(entry, _)| entry))
    }

    pub(super) async fn relation_resource(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<Option<(CatalogEntry, Value)>, DatabaseError> {
        let (project, dataset, name) =
            validate_relation_identity(self.profile_id, &self.catalog_scope, scope, relation)?;
        let table = match self
            .get(
                &["projects", project, "datasets", dataset, "tables", name],
                &[],
            )
            .await
        {
            Ok(table) => table,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        let entry = relation_entry(self.profile_id, project, dataset, &table)?;
        if entry.id != *relation {
            if entry.id.native_path == relation.native_path {
                return Ok(None);
            }
            return Err(invalid_response(
                "BigQuery tables.get returned a different relation",
            ));
        }
        Ok(Some((entry, table)))
    }

    async fn project_names(
        &self,
        requested: &CatalogScope,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<String>, DatabaseError> {
        validate_scopes(&self.catalog_scope, requested)?;
        if let Some(names) = selected_projects(&self.catalog_scope, requested) {
            return Ok(names);
        }
        warnings.push(PROJECT_DISCOVERY_WARNING.into());
        let mut projects = BTreeSet::new();
        // The execution/billing project need not contain data; the configured data project
        // must remain addressable even when projects.list cannot discover it.
        projects.insert(self.data_project.clone());
        match self.enumerate(&["projects"], "projects").await {
            Ok(result) => {
                warnings.extend(result.warnings);
                for project in result.items {
                    let name = project_name(&project)?;
                    projects.insert(name.into());
                }
            }
            Err(error) if error.category == ErrorCategory::Permission => warnings.push(warning(
                &format!("Unable to list BigQuery projects: {error}"),
            )),
            Err(error) => return Err(error),
        }
        Ok(projects
            .into_iter()
            .filter(|name| {
                self.catalog_scope.allows_database(name) && requested.allows_database(name)
            })
            .collect())
    }

    async fn dataset_names(
        &self,
        project: &str,
        requested: &CatalogScope,
    ) -> Result<(Vec<String>, Vec<String>), DatabaseError> {
        if let Some(names) = selected_datasets(&self.catalog_scope, requested, project) {
            return Ok((names, Vec::new()));
        }
        let result = self
            .enumerate(&["projects", project, "datasets"], "datasets")
            .await?;
        let mut names = BTreeSet::new();
        for dataset in result.items {
            let reference = dataset
                .get("datasetReference")
                .ok_or_else(|| invalid_response("BigQuery dataset reference is missing"))?;
            if required_text(reference, "projectId")? != project {
                return Err(invalid_response(
                    "BigQuery dataset belongs to another project",
                ));
            }
            let name = required_text(reference, "datasetId")?;
            validate_component(name)?;
            if self.catalog_scope.allows_schema(project, name)
                && requested.allows_schema(project, name)
            {
                names.insert(name.into());
            }
        }
        names.extend(configured_datasets(self, project, requested));
        Ok((names.into_iter().collect(), result.warnings))
    }

    async fn table_entries(
        &self,
        project: &str,
        dataset: &str,
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        let result = self
            .enumerate(
                &["projects", project, "datasets", dataset, "tables"],
                "tables",
            )
            .await?;
        require_complete(&result.warnings)?;
        let mut entries = result
            .items
            .iter()
            .map(|table| relation_entry(self.profile_id, project, dataset, table))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by(|a, b| a.id.native_path.cmp(&b.id.native_path));
        if entries
            .windows(2)
            .any(|pair| pair[0].id.native_path == pair[1].id.native_path)
        {
            return Err(invalid_response(
                "BigQuery listing contains duplicate table identities",
            ));
        }
        Ok(entries)
    }

    async fn enumerate(&self, path: &[&str], field: &str) -> Result<Enumeration, DatabaseError> {
        let mut items = Vec::new();
        let mut warnings = Vec::new();
        let mut token: Option<String> = None;
        let mut seen = BTreeSet::new();
        let mut bytes = 0usize;
        for _ in 0..MAX_REST_PAGES {
            let mut params = vec![("maxResults", "500".into())];
            if field == "datasets" {
                params.push(("all", "true".into()));
            }
            if let Some(token) = &token {
                params.push(("pageToken", token.clone()));
            }
            let response = self.get(path, &params).await?;
            bytes = bytes.saturating_add(response.to_string().len());
            if bytes > MAX_BYTES {
                return Err(invalid_response(
                    "BigQuery metadata exceeded its byte budget; narrow the scope",
                ));
            }
            let page = list_items(&response, field)?;
            if items.len().saturating_add(page.len()) > MAX_NAMES {
                return Err(DatabaseError::unsupported(
                    "BigQuery metadata exceeded its enumeration budget; narrow the scope",
                ));
            }
            items.extend_from_slice(page);
            warnings.extend(partial_warnings(&response)?);
            let Some(next) = optional_text(&response, "nextPageToken")? else {
                return Ok(Enumeration { items, warnings });
            };
            if !seen.insert(next.clone()) {
                return Err(invalid_response("BigQuery REST page token did not advance"));
            }
            token = Some(next);
        }
        Err(invalid_response(
            "BigQuery metadata exceeded its page budget; narrow the scope",
        ))
    }
}

fn validation(error: impl std::fmt::Display) -> DatabaseError {
    DatabaseError::configuration(error.to_string())
}
fn warning(message: &str) -> String {
    crate::security::sanitize_terminal_text(message)
}
fn require_complete(warnings: &[String]) -> Result<(), DatabaseError> {
    if warnings.is_empty() {
        Ok(())
    } else {
        Err(invalid_response(format!(
            "BigQuery catalog enumeration is incomplete: {}",
            warnings.join("; ")
        )))
    }
}

pub(super) fn is_not_found(error: &DatabaseError) -> bool {
    matches!(
        error.code.as_deref(),
        Some("404" | "notFound" | "NOT_FOUND")
    )
}

pub(super) fn validate_component(name: &str) -> Result<(), DatabaseError> {
    // A complete GoogleSQL path is quoted as one identifier. A dot in a component
    // would change its namespace even inside that quoted path.
    if name.is_empty() || name.chars().any(|c| c.is_control() || c == '.') {
        return Err(DatabaseError::configuration(
            "BigQuery native path components must be nonempty and contain no dots or control characters",
        ));
    }
    Ok(())
}

fn validate_scopes(profile: &CatalogScope, requested: &CatalogScope) -> Result<(), DatabaseError> {
    for scope in [profile, requested] {
        scope.validate("", None).map_err(validation)?;
        if let CatalogSelection::Selected(projects) = &scope.databases {
            for project in projects {
                validate_component(&project.name)?;
                if let CatalogSelection::Selected(datasets) = &project.schemas {
                    for dataset in datasets {
                        validate_component(dataset)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_catalog_request(
    profile_id: Uuid,
    profile: &CatalogScope,
    request: &CatalogRequest,
) -> Result<(), DatabaseError> {
    request
        .validate_for_profile(profile_id)
        .map_err(validation)?;
    validate_scopes(profile, &request.scope)?;
    let id = match &request.key.target {
        CatalogTarget::Databases => return Ok(()),
        CatalogTarget::Schemas { database } => database,
        CatalogTarget::Groups { schema } | CatalogTarget::Objects { schema, .. } => schema,
        CatalogTarget::RelationChildren { relation } => {
            validate_relation_identity(profile_id, profile, &request.scope, relation)?;
            return Ok(());
        }
        CatalogTarget::ProjectionChildren { .. } => {
            return Err(DatabaseError::unsupported(
                "BigQuery has no projection children",
            ));
        }
    };
    for part in &id.native_path {
        validate_component(part)?;
    }
    if !profile.allows_database(&id.native_path[0])
        || (id.native_path.len() == 2
            && !profile.allows_schema(&id.native_path[0], &id.native_path[1]))
    {
        return Err(DatabaseError::configuration(
            "BigQuery catalog target is outside profile scope",
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
    if relation.connection_id != profile_id || !relation.kind.is_relation() {
        return Err(DatabaseError::configuration(
            "BigQuery requires a relation identity from this profile",
        ));
    }
    let [project, dataset, name] = relation.native_path.as_slice() else {
        return Err(DatabaseError::configuration(
            "BigQuery relation identity must contain exactly project, dataset, and table",
        ));
    };
    for part in [project, dataset, name] {
        validate_component(part)?;
    }
    if !profile.allows_schema(project, dataset) || !requested.allows_schema(project, dataset) {
        return Err(DatabaseError::configuration(
            "BigQuery relation is outside profile or request scope",
        ));
    }
    Ok((project, dataset, name))
}

fn selected_projects(profile: &CatalogScope, requested: &CatalogScope) -> Option<Vec<String>> {
    let selected = match (&profile.databases, &requested.databases) {
        (CatalogSelection::Selected(names), _) | (_, CatalogSelection::Selected(names)) => names,
        _ => return None,
    };
    let names = selected
        .iter()
        .filter(|project| {
            profile.allows_database(&project.name) && requested.allows_database(&project.name)
        })
        .filter(|project| {
            selected_datasets(profile, requested, &project.name)
                .is_none_or(|datasets| !datasets.is_empty())
        })
        .map(|project| project.name.clone())
        .collect::<BTreeSet<_>>();
    Some(names.into_iter().collect())
}

fn dataset_selection<'a>(scope: &'a CatalogScope, project: &str) -> Option<&'a [String]> {
    if let CatalogSelection::Selected(projects) = &scope.databases
        && let Some(project) = projects.iter().find(|entry| entry.name == project)
        && let CatalogSelection::Selected(names) = &project.schemas
    {
        return Some(names);
    }
    None
}

fn selected_datasets(
    profile: &CatalogScope,
    requested: &CatalogScope,
    project: &str,
) -> Option<Vec<String>> {
    if !profile.allows_database(project) || !requested.allows_database(project) {
        return Some(Vec::new());
    }
    let names =
        dataset_selection(profile, project).or_else(|| dataset_selection(requested, project))?;
    Some(
        names
            .iter()
            .filter(|name| {
                profile.allows_schema(project, name) && requested.allows_schema(project, name)
            })
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    )
}

fn configured_datasets(
    adapter: &BigQueryAdapter,
    project: &str,
    requested: &CatalogScope,
) -> Vec<String> {
    if project == adapter.data_project {
        adapter
            .dataset
            .iter()
            .filter(|name| {
                adapter.catalog_scope.allows_schema(project, name)
                    && requested.allows_schema(project, name)
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    }
}

fn name_cursor(cursor: Option<&CatalogCursor>) -> Result<Option<&str>, DatabaseError> {
    cursor
        .map(|cursor| {
            let (sort, tie) = cursor.keyset_parts().map_err(validation)?;
            if sort != tie {
                return Err(DatabaseError::configuration(
                    "invalid BigQuery catalog name cursor",
                ));
            }
            validate_component(tie)?;
            Ok(tie)
        })
        .transpose()
}

fn project_name(project: &Value) -> Result<&str, DatabaseError> {
    // projects[].id is opaque; only ProjectReference carries the API project ID.
    let reference = project
        .get("projectReference")
        .ok_or_else(|| invalid_response("BigQuery project reference is missing"))?;
    let name = required_text(reference, "projectId")?;
    validate_component(name)?;
    Ok(name)
}

fn list_items<'a>(response: &'a Value, field: &str) -> Result<&'a [Value], DatabaseError> {
    if !response.is_object() {
        return Err(invalid_response("BigQuery list response is not an object"));
    }
    match response.get(field) {
        None => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        _ => Err(invalid_response(
            "BigQuery list response contains an invalid item array",
        )),
    }
}

fn partial_warnings(response: &Value) -> Result<Vec<String>, DatabaseError> {
    match response.get("unreachable") {
        None => Ok(Vec::new()),
        Some(Value::Array(locations)) => locations
            .iter()
            .map(|location| {
                location
                    .as_str()
                    .map(|name| {
                        warning(&format!("BigQuery dataset location {name} was unreachable"))
                    })
                    .ok_or_else(|| invalid_response("BigQuery unreachable locations are invalid"))
            })
            .collect(),
        _ => Err(invalid_response(
            "BigQuery unreachable locations are invalid",
        )),
    }
}

pub(super) fn required_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, DatabaseError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            invalid_response(format!(
                "BigQuery metadata field {field} is missing or invalid"
            ))
        })
}
fn optional_text(value: &Value, field: &str) -> Result<Option<String>, DatabaseError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok((!text.is_empty()).then(|| text.clone())),
        _ => Err(invalid_response(format!(
            "BigQuery metadata field {field} is not text"
        ))),
    }
}

fn database_entry(profile: Uuid, project: &str) -> Result<CatalogEntry, DatabaseError> {
    validate_component(project)?;
    CatalogEntry::database(
        CatalogId::new(profile, CatalogKind::Database, [project]),
        QualifiedName {
            database: Some(project.into()),
            schema: None,
            object: project.into(),
        },
        "PROJECT",
        OptionalMetadata::Unsupported,
        true,
    )
    .map_err(validation)
}
fn schema_entry(
    profile: Uuid,
    project: &str,
    dataset: &str,
) -> Result<CatalogEntry, DatabaseError> {
    validate_component(dataset)?;
    CatalogEntry::schema(
        CatalogId::new(profile, CatalogKind::Schema, [project, dataset]),
        CatalogId::new(profile, CatalogKind::Database, [project]),
        QualifiedName {
            database: Some(project.into()),
            schema: Some(dataset.into()),
            object: dataset.into(),
        },
        "DATASET",
        OptionalMetadata::Unsupported,
        true,
    )
    .map_err(validation)
}

fn relation_entry(
    profile: Uuid,
    project: &str,
    dataset: &str,
    table: &Value,
) -> Result<CatalogEntry, DatabaseError> {
    let reference = table
        .get("tableReference")
        .ok_or_else(|| invalid_response("BigQuery table reference is missing"))?;
    if required_text(reference, "projectId")? != project
        || required_text(reference, "datasetId")? != dataset
    {
        return Err(invalid_response(
            "BigQuery table belongs to another namespace",
        ));
    }
    let name = required_text(reference, "tableId")?;
    validate_component(name)?;
    let native = required_text(table, "type")?;
    let kind = match native {
        "TABLE" | "EXTERNAL" | "SNAPSHOT" | "CLONE" => CatalogKind::Table,
        "VIEW" => CatalogKind::View,
        "MATERIALIZED_VIEW" => CatalogKind::MaterializedView,
        _ => {
            return Err(invalid_response(
                "BigQuery returned an unsupported table type",
            ));
        }
    };
    // tables.list has no description field. Do not claim its absence means no comment.
    let comment = if table.get("description").is_some() {
        OptionalMetadata::Supported(optional_text(table, "description")?)
    } else {
        OptionalMetadata::Unsupported
    };
    CatalogEntry::relation(
        CatalogId::new(profile, kind, [project, dataset, name]),
        CatalogId::new(profile, CatalogKind::Schema, [project, dataset]),
        QualifiedName {
            database: Some(project.into()),
            schema: Some(dataset.into()),
            object: name.into(),
        },
        native,
        comment,
        true,
    )
    .map_err(validation)
}

fn column_entries(relation: &CatalogId, table: &Value) -> Result<Vec<CatalogEntry>, DatabaseError> {
    let fields = table
        .get("schema")
        .and_then(|schema| schema.get("fields"))
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_response("BigQuery tables.get returned no schema fields"))?;
    if fields.len() > MAX_NAMES {
        return Err(invalid_response(
            "BigQuery schema exceeded its column budget",
        ));
    }
    let mut names = BTreeSet::new();
    fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let name = required_text(field, "name")?;
            validate_component(name)?;
            if !names.insert(name) {
                return Err(invalid_response(
                    "BigQuery schema has duplicate column names",
                ));
            }
            let native_type = field_type(field, 0)?;
            let mode = field_mode(field)?;
            let mut metadata =
                ColumnMetadata::new((index + 1) as u32, &native_type, mode == "NULLABLE");
            metadata.default_expression =
                OptionalMetadata::Supported(optional_text(field, "defaultValueExpression")?);
            metadata.comment = OptionalMetadata::Supported(optional_text(field, "description")?);
            metadata.numeric_precision = OptionalMetadata::Supported(
                number(field, "precision")?
                    .map(u32::try_from)
                    .transpose()
                    .map_err(validation)?,
            );
            metadata.numeric_scale = OptionalMetadata::Supported(
                number(field, "scale")?
                    .map(u32::try_from)
                    .transpose()
                    .map_err(validation)?
                    .or_else(|| {
                        matches!(
                            &metadata.numeric_precision,
                            OptionalMetadata::Supported(Some(_))
                        )
                        .then_some(0)
                    }),
            );
            metadata.character_maximum_length =
                OptionalMetadata::Supported(number(field, "maxLength")?);
            metadata.collation = OptionalMetadata::Supported(optional_text(field, "collation")?);
            let mut path = relation.native_path.clone();
            path.push(name.into());
            CatalogEntry::relation_child(
                CatalogId::new(relation.connection_id, CatalogKind::Column, path),
                relation.clone(),
                QualifiedName {
                    database: Some(relation.native_path[0].clone()),
                    schema: Some(relation.native_path[1].clone()),
                    object: name.into(),
                },
                native_type,
                metadata.comment.clone(),
                CatalogMetadata::Column(metadata),
            )
            .map_err(validation)
        })
        .collect()
}

fn field_mode(field: &Value) -> Result<&str, DatabaseError> {
    match field.get("mode") {
        None => Ok("NULLABLE"),
        Some(Value::String(mode))
            if matches!(mode.as_str(), "NULLABLE" | "REQUIRED" | "REPEATED") =>
        {
            Ok(mode)
        }
        _ => Err(invalid_response("BigQuery schema field mode is invalid")),
    }
}

fn field_type(field: &Value, depth: usize) -> Result<String, DatabaseError> {
    if depth > 15 {
        return Err(invalid_response(
            "BigQuery schema nesting exceeds 15 levels",
        ));
    }
    let native = required_text(field, "type")?;
    let mut native = if matches!(native, "RECORD" | "STRUCT") {
        let fields = field
            .get("fields")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_response("BigQuery record has no nested schema"))?;
        let fields = fields
            .iter()
            .map(|field| {
                let name = required_text(field, "name")?;
                Ok(format!(
                    "{} {}",
                    quote_identifier(name),
                    field_type(field, depth + 1)?
                ))
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        format!("{native}<{}>", fields.join(", "))
    } else if native == "RANGE" {
        let element = field
            .get("rangeElementType")
            .ok_or_else(|| invalid_response("BigQuery RANGE has no element type"))?;
        format!("RANGE<{}>", required_text(element, "type")?)
    } else {
        native.to_owned()
    };
    if let Some(precision) = number(field, "precision")? {
        native.push_str(&format!(
            "({precision}, {})",
            number(field, "scale")?.unwrap_or(0)
        ));
    } else if let Some(length) = number(field, "maxLength")? {
        native.push_str(&format!("({length})"));
    }
    if field_mode(field)? == "REPEATED" {
        native = format!("ARRAY<{native}>");
    }
    Ok(native)
}
fn number(value: &Value, field: &str) -> Result<Option<u64>, DatabaseError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => value
            .parse()
            .map(Some)
            .map_err(|_| invalid_response("BigQuery schema numeric metadata is invalid")),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid_response("BigQuery schema numeric metadata is invalid")),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{
        db::catalog::{CatalogRequestKey, CatalogSearchObjectScope},
        identity::ConnectionIdentity,
        profile::DatabaseScope,
    };
    use serde_json::json;

    /// A scripted HTTP fixture, with strict ordering and no production endpoint override.
    pub(in crate::db::bigquery) async fn fixture_server(
        replies: Vec<(u16, Value)>,
    ) -> (url::Url, tokio::task::JoinHandle<Vec<(String, Value)>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = url::Url::parse(&format!(
            "http://{}/bigquery/v2/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, response) in replies {
                let (mut socket, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0, "request headers ended early");
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(offset) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        break offset + 4;
                    }
                };
                let header = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length = header
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, length)| length.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0, "request body ended early");
                    bytes.extend_from_slice(&chunk[..count]);
                }
                let body = if length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                };
                requests.push((header.lines().next().unwrap().to_owned(), body));
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (endpoint, handle)
    }

    fn request(adapter: &BigQueryAdapter, target: CatalogTarget, size: usize) -> CatalogRequest {
        CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: adapter.profile_id,
                    generation: 1,
                },
                catalog_epoch: 1,
                request_id: 1,
                target,
                cursor: None,
            },
            scope: adapter.catalog_scope.clone(),
            page_size: size,
        }
    }

    #[tokio::test]
    async fn rest_discovery_follows_project_and_dataset_tokens_and_keeps_configured_project() {
        let dataset = |project: &str, name: &str| json!({"datasetReference":{"projectId":project,"datasetId":name}});
        let (endpoint, server) = fixture_server(vec![
            (200, json!({"projects":[{"id":"opaque-id","projectReference":{"projectId":"a-project"}}],"nextPageToken":"projects/token+2"})),
            (200, json!({"projects":[{"projectReference":{"projectId":"z-project"}}]})),
            (200, json!({"datasets":[dataset("a-project", "first")],"nextPageToken":"datasets/token+2"})),
            (200, json!({"datasets":[dataset("a-project", "second")],"unreachable":["eu"]})),
            (200, json!({"datasets":[]})),
            (403, json!({"error":{"errors":[{"reason":"accessDenied","message":"fixture denial"}],"message":"fixture denial"}})),
        ]).await;
        let mut adapter = BigQueryAdapter::fixture(endpoint);
        adapter.catalog_scope = all();
        adapter.data_project = "data-project".into();
        adapter.dataset = Some("configured".into());
        let discovery = adapter.discover_catalog_scope().await.unwrap();
        assert_eq!(
            discovery
                .databases
                .iter()
                .map(|db| db.name.as_str())
                .collect::<Vec<_>>(),
            ["a-project", "data-project", "z-project"]
        );
        assert_eq!(discovery.databases[0].schemas, ["first", "second"]);
        assert_eq!(discovery.databases[1].schemas, ["configured"]);
        assert!(
            discovery
                .warnings
                .iter()
                .any(|warning| warning.contains("unreachable"))
        );
        assert!(
            discovery
                .warnings
                .iter()
                .any(|warning| warning.contains("z-project"))
        );
        assert!(
            discovery
                .warnings
                .iter()
                .any(|warning| warning.contains("project-level role"))
        );
        let requests = server.await.unwrap();
        assert!(requests[1].0.contains("pageToken=projects%2Ftoken%2B2"));
        assert!(requests[3].0.contains("pageToken=datasets%2Ftoken%2B2"));
        assert!(requests[2].0.contains("all=true"));
        assert!(requests.iter().all(|(line, _)| line.starts_with("GET ")));
    }

    #[tokio::test]
    async fn rest_table_pages_are_sorted_filtered_and_resume_with_validated_keysets() {
        let mut a = table("TABLE");
        a["tableReference"]["tableId"] = json!("a");
        let mut b = table("TABLE");
        b["tableReference"]["tableId"] = json!("b");
        let replies = vec![
            (200, json!({"tables":[b],"nextPageToken":"opaque+token"})),
            (200, json!({"tables":[table("VIEW"),a]})),
        ];
        let (endpoint, server) =
            fixture_server(replies.clone().into_iter().chain(replies).collect()).await;
        let mut adapter = BigQueryAdapter::fixture(endpoint);
        adapter.catalog_scope = scope("data-project", &["public"]);
        let mut request = request(
            &adapter,
            CatalogTarget::Objects {
                schema: CatalogId::new(
                    adapter.profile_id,
                    CatalogKind::Schema,
                    ["data-project", "public"],
                ),
                group: ObjectGroup::Tables,
            },
            1,
        );
        let first = adapter.load_catalog_page(&request).await.unwrap();
        assert_eq!(first.entries[0].qualified_name.object, "a");
        request.key.cursor = first.next_cursor;
        let second = adapter.load_catalog_page(&request).await.unwrap();
        assert_eq!(second.entries[0].qualified_name.object, "b");
        assert!(second.next_cursor.is_none());
        let requests = server.await.unwrap();
        assert!(requests[1].0.contains("pageToken=opaque%2Btoken"));
    }

    #[tokio::test]
    async fn search_fails_explicitly_for_unreachable_or_permission_partial_catalogs() {
        for reply in [
            (200, json!({"unreachable":["eu"]})),
            (
                403,
                json!({"error":{"errors":[{"reason":"accessDenied"}],"message":"fixture denied"}}),
            ),
        ] {
            let (endpoint, server) = fixture_server(vec![reply]).await;
            let mut adapter = BigQueryAdapter::fixture(endpoint);
            adapter.catalog_scope = CatalogScope {
                databases: CatalogSelection::Selected(vec![DatabaseScope {
                    name: "data-project".into(),
                    schemas: CatalogSelection::All,
                }]),
            };
            let request = CatalogSearchRequest {
                connection: ConnectionIdentity {
                    profile_id: adapter.profile_id,
                    generation: 1,
                },
                session_id: 1,
                generation: 1,
                query: "users".into(),
                scope: adapter.catalog_scope.clone(),
                object_scope: CatalogSearchObjectScope::RelationsOnly,
                limit: 10,
            };
            assert!(adapter.search_catalog(&request).await.is_err());
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn rest_relation_identity_distinguishes_missing_replacement_and_permission_errors() {
        let (endpoint, server) = fixture_server(vec![
            (
                404,
                json!({"error":{"errors":[{"reason":"notFound"}],"message":"missing"}}),
            ),
            (200, table("VIEW")),
            (
                403,
                json!({"error":{"errors":[{"reason":"accessDenied"}],"message":"denied"}}),
            ),
        ])
        .await;
        let mut adapter = BigQueryAdapter::fixture(endpoint);
        adapter.catalog_scope = scope("data-project", &["public"]);
        let relation = CatalogId::new(
            adapter.profile_id,
            CatalogKind::Table,
            ["data-project", "public", "users"],
        );
        assert!(
            adapter
                .resolve_relation_identity(&relation)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            adapter
                .resolve_relation_identity(&relation)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            adapter
                .resolve_relation_identity(&relation)
                .await
                .unwrap_err()
                .category,
            ErrorCategory::Permission
        );
        assert_eq!(server.await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn selected_names_and_invalid_requests_do_not_require_rest_discovery() {
        let mut adapter =
            BigQueryAdapter::fixture(url::Url::parse("http://127.0.0.1:1/bigquery/v2/").unwrap());
        adapter.catalog_scope = scope("unlisted-project", &["unlisted_dataset"]);
        let discovery = adapter.discover_catalog_scope().await.unwrap();
        assert!(discovery.warnings.is_empty());
        assert_eq!(discovery.databases[0].name, "unlisted-project");
        assert_eq!(discovery.databases[0].schemas, ["unlisted_dataset"]);
        let relation = CatalogId::new(
            adapter.profile_id,
            CatalogKind::Table,
            ["unlisted-project", "outside", "t"],
        );
        let request = request(&adapter, CatalogTarget::RelationChildren { relation }, 10);
        assert_eq!(
            adapter
                .load_catalog_page(&request)
                .await
                .unwrap_err()
                .category,
            ErrorCategory::Configuration
        );
    }

    fn scope(project: &str, datasets: &[&str]) -> CatalogScope {
        CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: project.into(),
                schemas: CatalogSelection::Selected(datasets.iter().map(|s| (*s).into()).collect()),
            }]),
        }
    }
    fn all() -> CatalogScope {
        CatalogScope {
            databases: CatalogSelection::All,
        }
    }
    fn table(kind: &str) -> Value {
        json!({"tableReference":{"projectId":"data-project","datasetId":"public","tableId":"users"},"type":kind})
    }

    #[test]
    fn selected_names_intersect_both_scope_boundaries() {
        let left = scope("data-project", &["public", "private"]);
        let right = scope("data-project", &["public"]);
        assert_eq!(
            selected_projects(&all(), &left),
            Some(vec!["data-project".into()])
        );
        assert_eq!(
            selected_datasets(&left, &right, "data-project"),
            Some(vec!["public".into()])
        );
        assert_eq!(
            selected_projects(&left, &scope("other", &["public"])),
            Some(vec![])
        );
        assert_eq!(
            selected_projects(&right, &scope("data-project", &["private"])),
            Some(vec![])
        );
        assert_eq!(selected_projects(&all(), &all()), None);
    }

    #[test]
    fn request_validation_rejects_wrong_profile_scope_and_path_before_io() {
        let profile = Uuid::nil();
        let scope = scope("data-project", &["public"]);
        let relation = CatalogId::new(
            profile,
            CatalogKind::Table,
            ["data-project", "public", "users"],
        );
        let mut request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: profile,
                    generation: 1,
                },
                catalog_epoch: 1,
                request_id: 1,
                target: CatalogTarget::RelationChildren {
                    relation: relation.clone(),
                },
                cursor: None,
            },
            scope: scope.clone(),
            page_size: 1,
        };
        assert!(validate_catalog_request(profile, &scope, &request).is_ok());
        assert!(validate_catalog_request(Uuid::from_u128(1), &scope, &request).is_err());
        assert!(
            validate_relation_identity(
                profile,
                &scope,
                &scope,
                &CatalogId::new(profile, CatalogKind::View, ["data-project", "private", "v"])
            )
            .is_err()
        );
        request.scope = all();
        request.key.target = CatalogTarget::RelationChildren {
            relation: CatalogId::new(
                profile,
                CatalogKind::Table,
                ["data-project", "public", "users", "extra"],
            ),
        };
        assert!(validate_catalog_request(profile, &scope, &request).is_err());
        for bad in ["", "bad.path", "bad\0name", "bad\nname"] {
            assert!(validate_component(bad).is_err());
        }
    }

    #[test]
    fn nested_repeated_schema_preserves_native_type_and_owner_shape() {
        let relation = CatalogId::new(
            Uuid::nil(),
            CatalogKind::Table,
            ["data-project", "public", "users"],
        );
        let resource = json!({"schema":{"fields":[
            {"name":"items","type":"RECORD","mode":"REPEATED","fields":[
                {"name":"amount","type":"NUMERIC","precision":"20","scale":"4"},
                {"name":"tags","type":"STRING","mode":"REPEATED","maxLength":"30"}
            ]},
            {"name":"required","type":"INT64","mode":"REQUIRED","description":"counter","defaultValueExpression":"0"}
        ]}});
        let entries = column_entries(&relation, &resource).unwrap();
        let CatalogMetadata::Column(column) = &entries[0].metadata else {
            panic!()
        };
        assert_eq!(
            column.native_type,
            "ARRAY<RECORD<`amount` NUMERIC(20, 4), `tags` ARRAY<STRING(30)>>>"
        );
        assert!(!column.nullable);
        assert_eq!(
            entries[0].id.native_path,
            ["data-project", "public", "users", "items"]
        );
        assert_eq!(entries[0].parent_id.as_ref(), Some(&relation));
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: Uuid::nil(),
                    generation: 0,
                },
                catalog_epoch: 0,
                request_id: 0,
                target: CatalogTarget::RelationChildren { relation },
                cursor: None,
            },
            scope: all(),
            page_size: 10,
        };
        CatalogPage::new(&request, entries, CatalogCount::Unknown, None).unwrap();
    }

    #[test]
    fn project_reference_uses_api_id_instead_of_opaque_listing_id() {
        assert_eq!(project_name(&json!({"id":"opaque-numeric-identity", "projectReference":{"projectId":"data-project"}})).unwrap(), "data-project");
        assert!(project_name(&json!({"id":"not-an-api-reference"})).is_err());
    }

    #[test]
    fn list_partial_and_malformed_payloads_are_distinguished() {
        assert!(list_items(&json!({}), "datasets").unwrap().is_empty());
        assert!(list_items(&json!({"datasets":null}), "datasets").is_err());
        let warnings = partial_warnings(&json!({"unreachable":["us-east1"]})).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(require_complete(&warnings).is_err());
        assert!(partial_warnings(&json!({"unreachable":"us"})).is_err());
        assert!(name_cursor(Some(&CatalogCursor::new("opaque-token"))).is_err());
        assert!(name_cursor(Some(&CatalogCursor::from_keyset("a", "b").unwrap())).is_err());
    }

    #[test]
    fn native_types_and_search_ancestors_validate_against_shared_contract() {
        let profile = Uuid::nil();
        for (native, expected) in [
            ("TABLE", CatalogKind::Table),
            ("EXTERNAL", CatalogKind::Table),
            ("SNAPSHOT", CatalogKind::Table),
            ("VIEW", CatalogKind::View),
            ("MATERIALIZED_VIEW", CatalogKind::MaterializedView),
        ] {
            let entry = relation_entry(profile, "data-project", "public", &table(native)).unwrap();
            assert_eq!(entry.kind, expected);
            let request = CatalogSearchRequest {
                connection: ConnectionIdentity {
                    profile_id: profile,
                    generation: 1,
                },
                session_id: 1,
                generation: 1,
                query: "user_s".into(),
                scope: all(),
                object_scope: CatalogSearchObjectScope::RelationsOnly,
                limit: 10,
            };
            assert!(search_text_matches(
                &entry.qualified_name.object,
                &request.query
            ));
            CatalogSearchPage::new(
                &request,
                vec![CatalogSearchHit {
                    entry,
                    ancestors: vec![
                        database_entry(profile, "data-project").unwrap(),
                        schema_entry(profile, "data-project", "public").unwrap(),
                    ],
                }],
                Some(1),
                false,
            )
            .unwrap();
        }
        assert!(relation_entry(profile, "other-project", "public", &table("TABLE")).is_err());
    }

    #[test]
    fn keyset_pages_advance_without_using_opaque_server_tokens() {
        let mut names = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let cursor = finalize_keyset_page(&mut names, 2, Clone::clone, Clone::clone)
            .unwrap()
            .unwrap();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(name_cursor(Some(&cursor)).unwrap(), Some("b"));
    }
}
