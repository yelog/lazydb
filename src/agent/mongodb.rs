use serde::Serialize;

use crate::{
    agent::{
        policy::{WriteCapability, WritePolicy},
        selection::{AgentError, AgentErrorCode},
        service::AgentService,
    },
    db::{
        DatabaseConnection,
        mongodb::{
            MongoAdapter,
            types::{FindSpec, MongoNamespace},
        },
    },
    mongodb_command::{
        MongoCommandCapability, MongoCommandKind, MongoCommandRisk, MongoWritePolicy,
        authorize_command, parse_command,
    },
    profile::DatabaseKind,
};

#[derive(Clone, Debug, Serialize)]
pub struct MongoAgentResult {
    pub connection: String,
    pub database: String,
    pub command: String,
    pub documents_ejson: Vec<String>,
    pub reply_ejson: Option<String>,
    pub count: Option<u64>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MongoAgentChangeResult {
    pub connection: String,
    pub database: String,
    pub command: String,
    pub outcome: crate::db::mongodb::mutation::MongoMutationOutcome,
}

pub(super) async fn execute(
    service: &AgentService,
    selector: Option<&str>,
    database: &str,
    command_text: &str,
) -> Result<MongoAgentResult, AgentError> {
    let selected = service.select(selector)?;
    if selected.profile.kind != DatabaseKind::MongoDb {
        return Err(agent_error(
            AgentErrorCode::DatabaseFailure,
            "Mongo query requires a MongoDB connection",
        ));
    }
    let parsed = parse_command(command_text, database).map_err(|error| {
        agent_error(
            AgentErrorCode::PolicyDenied,
            &format!("Mongo command rejected: {error}"),
        )
    })?;
    parsed.validate_supported_fields().map_err(|error| {
        agent_error(
            AgentErrorCode::PolicyDenied,
            &format!("Mongo command rejected: {error}"),
        )
    })?;
    authorize_command(selected.profile, &parsed, MongoWritePolicy::Deny).map_err(|error| {
        agent_error(
            AgentErrorCode::PolicyDenied,
            &format!("Mongo command rejected: {error}"),
        )
    })?;
    if parsed.risk != MongoCommandRisk::ReadOnly || parsed.capability().is_err() {
        return Err(agent_error(
            AgentErrorCode::PolicyDenied,
            "Mongo Agent query accepts read-only capabilities only",
        ));
    }
    let password = service
        .credential_resolver
        .resolve_headless(selected.profile)
        .await
        .map_err(|error| agent_error(AgentErrorCode::CredentialFailure, &error.to_string()))?;
    let connection = service
        .connector
        .connect(selected.profile, password.as_ref())
        .await
        .map_err(|error| agent_error(AgentErrorCode::DatabaseFailure, &error.to_string()))?;
    let result = match connection {
        DatabaseConnection::MongoDb(adapter) => {
            let result = execute_read(
                &adapter,
                &parsed,
                service.max_rows,
                service.max_result_bytes,
            )
            .await;
            adapter.close().await;
            result
        }
        _ => Err(agent_error(
            AgentErrorCode::DatabaseFailure,
            "MongoDB adapter unavailable",
        )),
    };
    result.and_then(|(documents_ejson, reply_ejson, count, truncated)| {
        let encoded_size = documents_ejson
            .iter()
            .map(String::len)
            .sum::<usize>()
            .saturating_add(reply_ejson.as_ref().map_or(0, String::len));
        if encoded_size > service.max_result_bytes {
            return Err(agent_error(
                AgentErrorCode::ResultTooLarge,
                "Mongo result exceeds the configured byte budget",
            ));
        }
        Ok(MongoAgentResult {
            connection: selected.profile.name.clone(),
            database: database.to_owned(),
            command: parsed.command_name,
            documents_ejson,
            reply_ejson,
            count,
            truncated,
        })
    })
}

pub(super) async fn execute_change(
    service: &AgentService,
    selector: Option<&str>,
    database: &str,
    command_text: &str,
    write_policy: WritePolicy,
) -> Result<MongoAgentChangeResult, AgentError> {
    let selected = service.select(selector)?;
    if selected.profile.kind != DatabaseKind::MongoDb {
        return Err(agent_error(
            AgentErrorCode::DatabaseFailure,
            "Mongo change requires a MongoDB connection",
        ));
    }
    let parsed = parse_command(command_text, database).map_err(|error| {
        agent_error(
            AgentErrorCode::PolicyDenied,
            &format!("Mongo command rejected: {error}"),
        )
    })?;
    parsed.validate_supported_fields().map_err(|error| {
        agent_error(
            AgentErrorCode::PolicyDenied,
            &format!("Mongo command rejected: {error}"),
        )
    })?;
    let mongo_policy = match write_policy {
        WritePolicy::Deny => MongoWritePolicy::Deny,
        WritePolicy::NonProduction => MongoWritePolicy::NonProduction,
        WritePolicy::All => MongoWritePolicy::All,
    };
    authorize_command(selected.profile, &parsed, mongo_policy).map_err(|error| {
        agent_error(
            AgentErrorCode::PolicyDenied,
            &format!("Mongo command rejected: {error}"),
        )
    })?;
    if parsed.risk == MongoCommandRisk::ReadOnly {
        return Err(agent_error(
            AgentErrorCode::PolicyDenied,
            "Mongo execute-change accepts write commands only",
        ));
    }
    if let WriteCapability::Denied(error) =
        super::policy::write_capability(selected.profile, write_policy)
    {
        return Err(agent_error(
            AgentErrorCode::PolicyDenied,
            &format!("Mongo command rejected: {error}"),
        ));
    }
    let collection = match parsed.command.get(&parsed.command_name) {
        Some(mongodb::bson::Bson::String(collection)) if !collection.is_empty() => {
            collection.clone()
        }
        _ => {
            return Err(agent_error(
                AgentErrorCode::PolicyDenied,
                "Mongo change requires a collection name",
            ));
        }
    };
    let operation = match parsed.kind {
        MongoCommandKind::Insert => {
            if parsed
                .command
                .keys()
                .any(|key| !matches!(key.as_str(), "insert" | "documents"))
            {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo insert accepts only the collection and documents fields",
                ));
            }
            let documents = parsed.command.get_array("documents").map_err(|_| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "insert documents must be an array",
                )
            })?;
            if documents.len() != 1 {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo writes currently accept exactly one document per command",
                ));
            }
            let document = documents[0].as_document().cloned().ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "insert document must be an object",
                )
            })?;
            WriteOperation::Insert(document)
        }
        MongoCommandKind::Update => {
            if parsed
                .command
                .keys()
                .any(|key| !matches!(key.as_str(), "update" | "updates"))
            {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo update accepts only the collection and updates fields",
                ));
            }
            let updates = parsed.command.get_array("updates").map_err(|_| {
                agent_error(AgentErrorCode::PolicyDenied, "updates must be an array")
            })?;
            if updates.len() != 1 {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo writes currently accept exactly one update per command",
                ));
            }
            let update = updates[0].as_document().ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "update entry must be an object",
                )
            })?;
            if update.keys().any(|key| !matches!(key.as_str(), "q" | "u")) {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo update entry accepts only q and u",
                ));
            }
            let update_spec = update
                .get("u")
                .and_then(mongodb::bson::Bson::as_document)
                .cloned()
                .ok_or_else(|| {
                    agent_error(
                        AgentErrorCode::PolicyDenied,
                        "update must contain a modifier document",
                    )
                })?;
            if update_spec.keys().any(|key| !key.starts_with('$')) {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo update requires modifier operators; replacement documents are unsupported",
                ));
            }
            WriteOperation::Update(
                update.get_document("q").cloned().map_err(|_| {
                    agent_error(AgentErrorCode::PolicyDenied, "update query is required")
                })?,
                update_spec,
            )
        }
        MongoCommandKind::Delete => {
            if parsed
                .command
                .keys()
                .any(|key| !matches!(key.as_str(), "delete" | "deletes"))
            {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo delete accepts only the collection and deletes fields",
                ));
            }
            let deletes = parsed.command.get_array("deletes").map_err(|_| {
                agent_error(AgentErrorCode::PolicyDenied, "deletes must be an array")
            })?;
            if deletes.len() != 1 {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo writes currently accept exactly one delete per command",
                ));
            }
            let delete = deletes[0].as_document().ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "delete entry must be an object",
                )
            })?;
            let filter = delete.get_document("q").cloned().map_err(|_| {
                agent_error(AgentErrorCode::PolicyDenied, "delete query is required")
            })?;
            if filter.is_empty() {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo delete requires a non-empty query filter",
                ));
            }
            if delete
                .keys()
                .any(|key| !matches!(key.as_str(), "q" | "limit"))
                || delete.get_i32("limit").unwrap_or(1) != 1
            {
                return Err(agent_error(
                    AgentErrorCode::PolicyDenied,
                    "Agent Mongo delete only accepts limit: 1",
                ));
            }
            WriteOperation::Delete(filter)
        }
        _ => {
            return Err(agent_error(
                AgentErrorCode::PolicyDenied,
                "Agent Mongo execute-change currently supports single insert, update, and delete operations only",
            ));
        }
    };
    let password = service
        .credential_resolver
        .resolve_headless(selected.profile)
        .await
        .map_err(|error| agent_error(AgentErrorCode::CredentialFailure, &error.to_string()))?;
    let connection = service
        .connector
        .connect(selected.profile, password.as_ref())
        .await
        .map_err(|error| agent_error(AgentErrorCode::DatabaseFailure, &error.to_string()))?;
    let outcome = match connection {
        DatabaseConnection::MongoDb(adapter) => {
            let result = run_single_write(&adapter, database, &collection, operation).await;
            adapter.close().await;
            result
        }
        _ => Err(agent_error(
            AgentErrorCode::DatabaseFailure,
            "MongoDB adapter unavailable",
        )),
    }?;
    Ok(MongoAgentChangeResult {
        connection: selected.profile.name.clone(),
        database: database.to_owned(),
        command: parsed.command_name,
        outcome,
    })
}

enum WriteOperation {
    Insert(mongodb::bson::Document),
    Update(mongodb::bson::Document, mongodb::bson::Document),
    Delete(mongodb::bson::Document),
}

async fn run_single_write(
    adapter: &MongoAdapter,
    database: &str,
    collection: &str,
    operation: WriteOperation,
) -> Result<crate::db::mongodb::mutation::MongoMutationOutcome, AgentError> {
    let collection = adapter
        .client()
        .database(database)
        .collection::<mongodb::bson::Document>(collection);
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        match operation {
            WriteOperation::Insert(document) => collection.insert_one(document).await.map(|_| 1),
            WriteOperation::Update(filter, update) => collection
                .update_one(filter, update)
                .await
                .map(|result| result.modified_count),
            WriteOperation::Delete(filter) => collection
                .delete_one(filter)
                .await
                .map(|result| result.deleted_count),
        }
    })
    .await;
    match result {
        Ok(Ok(affected)) => {
            Ok(crate::db::mongodb::mutation::MongoMutationOutcome::Confirmed { affected })
        }
        Ok(Err(error)) if crate::db::mongodb::mutation::error_is_definitely_rejected(&error) => {
            Err(agent_error(
                AgentErrorCode::DatabaseFailure,
                "Mongo server rejected the change",
            ))
        }
        Ok(Err(_)) | Err(_) => {
            Ok(crate::db::mongodb::mutation::MongoMutationOutcome::Unknown { requested: 1 })
        }
    }
}

pub(crate) async fn execute_read(
    adapter: &MongoAdapter,
    command: &crate::mongodb_command::ParsedMongoCommand,
    max_rows: usize,
    max_bytes: usize,
) -> Result<(Vec<String>, Option<String>, Option<u64>, bool), AgentError> {
    if command.kind == MongoCommandKind::Explain {
        let inner = command.command.get_document("explain").map_err(|_| {
            agent_error(AgentErrorCode::PolicyDenied, "explain command is required")
        })?;
        let inner_name = inner
            .keys()
            .next()
            .ok_or_else(|| agent_error(AgentErrorCode::PolicyDenied, "explain command is empty"))?;
        if !matches!(
            inner_name.as_str(),
            "find" | "count" | "distinct" | "aggregate"
        ) {
            return Err(agent_error(
                AgentErrorCode::PolicyDenied,
                "explain supports find, count, distinct, or aggregate only",
            ));
        }
        let mut explain = mongodb::bson::doc! { "explain": inner.clone() };
        if let Ok(verbosity) = command.command.get_str("verbosity") {
            explain.insert("verbosity", verbosity);
        }
        if let Some(comment) = command.command.get("comment") {
            explain.insert("comment", comment.clone());
        }
        let reply = adapter
            .client()
            .database(&command.database)
            .run_command(explain)
            .await
            .map_err(|error| agent_error(AgentErrorCode::DatabaseFailure, &error.to_string()))?;
        let encoded = canonical(&reply)?;
        if encoded.len() > max_bytes {
            return Err(agent_error(
                AgentErrorCode::ResultTooLarge,
                "Mongo explain result exceeds the configured byte budget",
            ));
        }
        return Ok((Vec::new(), Some(encoded), None, false));
    }
    let collection = command.command.get_str(&command.command_name).ok();
    match command
        .capability()
        .map_err(|error| agent_error(AgentErrorCode::PolicyDenied, &error.to_string()))?
    {
        MongoCommandCapability::ReadDocuments => {
            let name = collection.ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "find collection must be a string",
                )
            })?;
            let namespace = MongoNamespace::collection(&command.database, name);
            let mut spec = FindSpec::new(namespace);
            spec.filter = command
                .command
                .get_document("filter")
                .cloned()
                .unwrap_or_default();
            spec.max_page_bytes = max_bytes;
            spec.max_document_bytes = max_bytes;
            spec.sort = command.command.get_document("sort").ok().cloned();
            spec.projection = command.command.get_document("projection").ok().cloned();
            if let Ok(skip) = command.command.get_i64("skip") {
                spec.skip = u64::try_from(skip).map_err(|_| {
                    agent_error(
                        AgentErrorCode::PolicyDenied,
                        "find skip must be non-negative",
                    )
                })?;
            }
            if let Ok(limit) = command.command.get_i64("limit") {
                spec.limit = Some(u64::try_from(limit).map_err(|_| {
                    agent_error(
                        AgentErrorCode::PolicyDenied,
                        "find limit must be non-negative",
                    )
                })?);
            }
            spec.limit = Some(spec.limit.unwrap_or(max_rows as u64).min(max_rows as u64));
            spec.page_size = (max_rows.clamp(1, FindSpec::MAX_PAGE_SIZE as usize)) as u32;
            spec.max_page_bytes = max_bytes;
            spec.max_document_bytes = max_bytes;
            let page = adapter.find_page(&spec).await.map_err(db_error)?;
            let values = page
                .documents
                .iter()
                .map(|document| canonical(&document.bson))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((
                values,
                None,
                None,
                page.truncated || page.next_offset.is_some(),
            ))
        }
        MongoCommandCapability::Count => {
            let name = collection.ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "count collection must be a string",
                )
            })?;
            let mut spec = FindSpec::new(MongoNamespace::collection(&command.database, name));
            spec.filter = command
                .command
                .get_document("query")
                .ok()
                .cloned()
                .unwrap_or_default();
            spec.max_page_bytes = max_bytes;
            spec.max_document_bytes = max_bytes;
            if let Ok(limit) = command.command.get_i64("limit") {
                spec.limit = Some(u64::try_from(limit).map_err(|_| {
                    agent_error(
                        AgentErrorCode::PolicyDenied,
                        "count limit must be non-negative",
                    )
                })?);
            }
            if let Ok(skip) = command.command.get_i64("skip") {
                spec.skip = u64::try_from(skip).map_err(|_| {
                    agent_error(
                        AgentErrorCode::PolicyDenied,
                        "count skip must be non-negative",
                    )
                })?;
            }
            let count = adapter.count_documents(&spec).await.map_err(db_error)?;
            Ok((Vec::new(), None, Some(count), false))
        }
        MongoCommandCapability::Distinct => {
            let name = collection.ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "distinct collection must be a string",
                )
            })?;
            let field = command.command.get_str("key").map_err(|_| {
                agent_error(AgentErrorCode::PolicyDenied, "distinct key is required")
            })?;
            let mut spec = FindSpec::new(MongoNamespace::collection(&command.database, name));
            spec.filter = command
                .command
                .get_document("query")
                .ok()
                .cloned()
                .unwrap_or_default();
            spec.max_page_bytes = max_bytes;
            spec.max_document_bytes = max_bytes;
            let values = adapter
                .distinct_values(&spec, field)
                .await
                .map_err(db_error)?;
            let reply = mongodb::bson::doc! {"values": values};
            Ok((Vec::new(), Some(canonical(&reply)?), None, false))
        }
        MongoCommandCapability::Aggregate => {
            let name = collection.ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "aggregate collection must be a string",
                )
            })?;
            let stages = command.command.get_array("pipeline").map_err(|_| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "aggregate pipeline is required",
                )
            })?;
            let pipeline = stages
                .iter()
                .map(|value| match value {
                    mongodb::bson::Bson::Document(doc) => Ok(doc.clone()),
                    _ => Err(agent_error(
                        AgentErrorCode::PolicyDenied,
                        "aggregate stage must be a document",
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let page = adapter
                .aggregate_page(
                    &MongoNamespace::collection(&command.database, name),
                    pipeline,
                    max_rows.clamp(1, FindSpec::MAX_PAGE_SIZE as usize) as u32,
                    max_bytes,
                )
                .await
                .map_err(db_error)?;
            let values = page
                .documents
                .iter()
                .map(|document| canonical(&document.bson))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((values, None, None, page.truncated))
        }
        MongoCommandCapability::ListCollections => {
            let collections = adapter
                .list_collections(&command.database)
                .await
                .map_err(db_error)?;
            let filter = command
                .command
                .get_document("filter")
                .cloned()
                .unwrap_or_default();
            let name_only = command.command.get_bool("nameOnly").unwrap_or(false);
            let docs = collections.iter().filter_map(|value| {
                let name = value.namespace.collection.clone().unwrap_or_default();
                if filter.get_str("name").is_ok_and(|wanted| wanted != name) { return None; }
                Some(if name_only { mongodb::bson::doc! {"name": name} } else { mongodb::bson::doc! {"name": name, "type": format!("{:?}", value.collection_type), "options": value.options_ejson.clone().unwrap_or_default()} })
            }).take(max_rows.saturating_add(1)).collect::<Vec<_>>();
            let values = docs
                .iter()
                .take(max_rows)
                .map(canonical)
                .collect::<Result<Vec<_>, _>>()?;
            if values.iter().map(String::len).sum::<usize>() > max_bytes {
                return Err(agent_error(
                    AgentErrorCode::ResultTooLarge,
                    "Mongo result exceeds the configured byte budget",
                ));
            }
            Ok((values, None, None, docs.len() > max_rows))
        }
        MongoCommandCapability::ListIndexes => {
            let name = collection.ok_or_else(|| {
                agent_error(
                    AgentErrorCode::PolicyDenied,
                    "listIndexes collection must be a string",
                )
            })?;
            let indexes = adapter
                .list_indexes(&MongoNamespace::collection(&command.database, name))
                .await
                .map_err(db_error)?;
            let indexes = indexes
                .iter()
                .take(max_rows.saturating_add(1))
                .collect::<Vec<_>>();
            let values = indexes
                .iter()
                .take(max_rows)
                .map(|index| canonical(&index.raw))
                .collect::<Result<Vec<_>, _>>()?;
            if values.iter().map(String::len).sum::<usize>() > max_bytes {
                return Err(agent_error(
                    AgentErrorCode::ResultTooLarge,
                    "Mongo result exceeds the configured byte budget",
                ));
            }
            Ok((values, None, None, indexes.len() > max_rows))
        }
    }
}

pub(super) async fn collections(
    service: &AgentService,
    selector: Option<&str>,
    database: &str,
) -> Result<Vec<crate::db::mongodb::types::MongoCollectionInfo>, AgentError> {
    let selected = service.select(selector)?;
    if selected.profile.kind != DatabaseKind::MongoDb
        || !selected.profile.catalog_scope.allows_database(database)
    {
        return Err(agent_error(
            AgentErrorCode::PolicyDenied,
            "Mongo database is outside the visible profile scope",
        ));
    }
    let password = service
        .credential_resolver
        .resolve_headless(selected.profile)
        .await
        .map_err(|error| agent_error(AgentErrorCode::CredentialFailure, &error.to_string()))?;
    let connection = service
        .connector
        .connect(selected.profile, password.as_ref())
        .await
        .map_err(|error| agent_error(AgentErrorCode::DatabaseFailure, &error.to_string()))?;
    let result = match connection {
        DatabaseConnection::MongoDb(adapter) => {
            let result = adapter.list_collections(database).await.map_err(db_error);
            adapter.close().await;
            result
        }
        _ => Err(agent_error(
            AgentErrorCode::DatabaseFailure,
            "MongoDB adapter unavailable",
        )),
    }?;
    Ok(result)
}

pub(super) async fn describe_collection(
    service: &AgentService,
    selector: Option<&str>,
    database: &str,
    collection: &str,
) -> Result<serde_json::Value, AgentError> {
    let selected = service.select(selector)?;
    if selected.profile.kind != DatabaseKind::MongoDb
        || !selected.profile.catalog_scope.allows_database(database)
    {
        return Err(agent_error(
            AgentErrorCode::PolicyDenied,
            "Mongo database is outside the visible profile scope",
        ));
    }
    let password = service
        .credential_resolver
        .resolve_headless(selected.profile)
        .await
        .map_err(|error| agent_error(AgentErrorCode::CredentialFailure, &error.to_string()))?;
    let connection = service
        .connector
        .connect(selected.profile, password.as_ref())
        .await
        .map_err(|error| agent_error(AgentErrorCode::DatabaseFailure, &error.to_string()))?;
    let result = match connection {
        DatabaseConnection::MongoDb(adapter) => {
            let namespace = MongoNamespace::collection(database, collection);
            let collection_info = adapter
                .list_collections(database)
                .await
                .map_err(db_error)?
                .into_iter()
                .find(|info| info.namespace.collection.as_deref() == Some(collection));
            let indexes = adapter.list_indexes(&namespace).await.map_err(db_error)?;
            let indexes = indexes
                .iter()
                .map(|index| canonical(&index.raw))
                .collect::<Result<Vec<_>, _>>()?;
            let result = serde_json::json!({ "database": database, "collection": collection,
                "type": collection_info.as_ref().map(|info| info.collection_type),
                "options_ejson": collection_info.and_then(|info| info.options_ejson),
                "indexes_ejson": indexes });
            adapter.close().await;
            Ok(result)
        }
        _ => Err(agent_error(
            AgentErrorCode::DatabaseFailure,
            "MongoDB adapter unavailable",
        )),
    }?;
    Ok(result)
}

fn canonical(document: &mongodb::bson::Document) -> Result<String, AgentError> {
    mongodb::bson::to_bson(document)
        .map(|value| value.into_canonical_extjson().to_string())
        .map_err(|_| {
            agent_error(
                AgentErrorCode::DatabaseFailure,
                "MongoDB result could not be encoded as Canonical EJSON",
            )
        })
}

fn db_error(error: crate::db::DatabaseError) -> AgentError {
    agent_error(AgentErrorCode::DatabaseFailure, &error.message)
}
fn agent_error(code: AgentErrorCode, message: &str) -> AgentError {
    AgentError {
        code,
        message: message.to_owned(),
    }
}
