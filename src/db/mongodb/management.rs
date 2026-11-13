use mongodb::{
    IndexModel,
    bson::{Document, doc},
    options::IndexOptions,
};

use super::{
    MongoAdapter,
    types::{MongoCollectionType, MongoNamespace},
};
use crate::db::DatabaseError;

impl MongoAdapter {
    pub async fn manage(
        &self,
        operation: &crate::model::mongodb_management::MongoManagementOperation,
        draft: Document,
    ) -> Result<(MongoNamespace, String), DatabaseError> {
        use crate::model::mongodb_management::MongoManagementOperation as Op;
        match operation {
            Op::CreateCollection { database } => {
                let name = required_string(&draft, "name")?;
                let mut command = doc! { "create": &name };
                if let Ok(options) = draft.get_document("options") {
                    for (key, value) in options {
                        command.insert(key, value.clone());
                    }
                }
                self.checked_client()?
                    .database(database)
                    .run_command(command)
                    .await
                    .map_err(super::map_error)?;
                Ok((
                    MongoNamespace::collection(database, &name),
                    format!("Created collection {database}.{name}"),
                ))
            }
            Op::CreateView { database } => {
                let name = required_string(&draft, "name")?;
                let view_on = required_string(&draft, "viewOn")?;
                let pipeline = draft
                    .get_array("pipeline")
                    .map_err(|_| DatabaseError::configuration("pipeline must be an array"))?
                    .clone();
                self.checked_client()?
                    .database(database)
                    .run_command(doc! { "create": &name, "viewOn": view_on, "pipeline": pipeline })
                    .await
                    .map_err(super::map_error)?;
                Ok((
                    MongoNamespace::collection(database, &name),
                    format!("Created view {database}.{name}"),
                ))
            }
            Op::RenameCollection { namespace } => {
                let target = required_string(&draft, "target")?;
                validate_name(&target)?;
                let source = namespace.full_name();
                let target_ns = format!("{}.{}", namespace.database, target);
                self.checked_client()?
                    .database("admin")
                    .run_command(doc! { "renameCollection": source, "to": target_ns })
                    .await
                    .map_err(super::map_error)?;
                Ok((
                    MongoNamespace::collection(&namespace.database, &target),
                    format!("Renamed collection to {target}"),
                ))
            }
            Op::DropCollection { namespace, kind } => {
                let collection = required_collection(namespace)?;
                if *kind == MongoCollectionType::View {
                    self.checked_client()?
                        .database(&namespace.database)
                        .run_command(doc! { "drop": collection })
                        .await
                        .map_err(super::map_error)?;
                } else {
                    self.checked_client()?
                        .database(&namespace.database)
                        .collection::<Document>(collection)
                        .drop()
                        .await
                        .map_err(super::map_error)?;
                }
                Ok((
                    namespace.clone(),
                    format!("Dropped {}", namespace.full_name()),
                ))
            }
            Op::CollMod { namespace } => {
                let collection = required_collection(namespace)?;
                if draft.is_empty() {
                    return Err(DatabaseError::configuration(
                        "collMod draft must include at least one option",
                    ));
                }
                if draft.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "validator"
                            | "validationLevel"
                            | "validationAction"
                            | "index"
                            | "viewOn"
                            | "pipeline"
                    )
                }) {
                    return Err(DatabaseError::configuration(
                        "collMod draft contains an unsupported option",
                    ));
                }
                let mut command = doc! { "collMod": collection };
                for (key, value) in draft {
                    command.insert(key, value);
                }
                self.checked_client()?
                    .database(&namespace.database)
                    .run_command(command)
                    .await
                    .map_err(super::map_error)?;
                Ok((
                    namespace.clone(),
                    format!("Updated collection options for {}", namespace.full_name()),
                ))
            }
            Op::CreateIndex { namespace } => {
                let collection = required_collection(namespace)?;
                let keys = draft
                    .get_document("keys")
                    .map_err(|_| {
                        DatabaseError::configuration("index keys must be an ordered object")
                    })?
                    .clone();
                if keys.is_empty() {
                    return Err(DatabaseError::configuration("index keys cannot be empty"));
                }
                let options_document = draft.get_document("options").cloned().unwrap_or_default();
                let options: IndexOptions = mongodb::bson::from_document(options_document)
                    .map_err(|_| DatabaseError::configuration("index options are invalid"))?;
                self.checked_client()?
                    .database(&namespace.database)
                    .collection::<Document>(collection)
                    .create_index(IndexModel::builder().keys(keys).options(options).build())
                    .await
                    .map_err(super::map_error)?;
                Ok((
                    namespace.clone(),
                    format!("Created index on {}", namespace.full_name()),
                ))
            }
            Op::DropIndex { namespace, name } => {
                if name == "_id_" {
                    return Err(DatabaseError::configuration(
                        "the _id_ index cannot be dropped",
                    ));
                }
                if name == "*" {
                    return Err(DatabaseError::configuration(
                        "dropping all indexes is not available through this action",
                    ));
                }
                let collection = required_collection(namespace)?;
                self.checked_client()?
                    .database(&namespace.database)
                    .collection::<Document>(collection)
                    .drop_index(name)
                    .await
                    .map_err(super::map_error)?;
                Ok((namespace.clone(), format!("Dropped index {name}")))
            }
        }
    }

    pub fn validate_management_request(
        operation: &crate::model::mongodb_management::MongoManagementOperation,
        draft: &Document,
    ) -> Result<(), DatabaseError> {
        use crate::model::mongodb_management::MongoManagementOperation as Op;
        match operation {
            Op::CreateCollection { .. } => {
                required_string(draft, "name")?;
                if draft.keys().any(|key| key != "name" && key != "options") {
                    return Err(DatabaseError::configuration(
                        "create collection draft contains an unsupported field",
                    ));
                }
                let options = draft.get_document("options").cloned().unwrap_or_default();
                if options.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "validator"
                            | "validationLevel"
                            | "validationAction"
                            | "collation"
                            | "timeseries"
                            | "expireAfterSeconds"
                            | "capped"
                            | "size"
                            | "max"
                    )
                }) {
                    return Err(DatabaseError::configuration(
                        "collection options contain an unsupported field",
                    ));
                }
            }
            Op::CreateView { .. } => {
                required_string(draft, "name")?;
                required_string(draft, "viewOn")?;
                if draft.get_array("pipeline").is_err()
                    || draft.keys().any(|key| {
                        !matches!(key.as_str(), "name" | "viewOn" | "pipeline" | "collation")
                    })
                {
                    return Err(DatabaseError::configuration(
                        "view draft must contain name, viewOn, pipeline and optional collation",
                    ));
                }
            }
            Op::RenameCollection { .. } => {
                required_string(draft, "target")?;
                if draft.len() != 1 {
                    return Err(DatabaseError::configuration(
                        "rename draft accepts only target",
                    ));
                }
            }
            Op::DropCollection { .. } => {
                if !draft.is_empty() {
                    return Err(DatabaseError::configuration(
                        "drop collection draft has no options",
                    ));
                }
            }
            Op::CollMod { .. } => {
                if draft.is_empty()
                    || draft.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "validator"
                                | "validationLevel"
                                | "validationAction"
                                | "index"
                                | "viewOn"
                                | "pipeline"
                        )
                    })
                {
                    return Err(DatabaseError::configuration(
                        "collMod draft contains no supported options",
                    ));
                }
            }
            Op::CreateIndex { .. } => {
                let keys = draft.get_document("keys").map_err(|_| {
                    DatabaseError::configuration("index keys must be an ordered object")
                })?;
                if keys.is_empty()
                    || draft
                        .keys()
                        .any(|key| !matches!(key.as_str(), "keys" | "options"))
                {
                    return Err(DatabaseError::configuration(
                        "index draft requires ordered keys and optional options",
                    ));
                }
                if let Some(options) = draft.get_document("options").ok()
                    && options.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "name"
                                | "unique"
                                | "sparse"
                                | "expireAfterSeconds"
                                | "partialFilterExpression"
                                | "collation"
                                | "hidden"
                                | "wildcardProjection"
                        )
                    })
                {
                    return Err(DatabaseError::configuration(
                        "index options contain an unsupported field",
                    ));
                }
            }
            Op::DropIndex { namespace, name } => {
                if name == "_id_" || name == "*" {
                    return Err(DatabaseError::configuration(
                        "the _id_ index and drop all indexes are protected",
                    ));
                }
                if !draft.is_empty() || namespace.collection.is_none() {
                    return Err(DatabaseError::configuration(
                        "drop index requires a collection and an empty options document",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn required_string(document: &Document, key: &str) -> Result<String, DatabaseError> {
    let value = document
        .get_str(key)
        .map_err(|_| DatabaseError::configuration(format!("{key} must be a string")))?
        .to_owned();
    validate_name(&value)?;
    Ok(value)
}

fn validate_name(name: &str) -> Result<(), DatabaseError> {
    if name.is_empty() || name.contains('\0') || name.starts_with("system.") {
        return Err(DatabaseError::configuration(
            "MongoDB collection or index name is invalid",
        ));
    }
    Ok(())
}

fn required_collection(namespace: &MongoNamespace) -> Result<&str, DatabaseError> {
    namespace
        .collection
        .as_deref()
        .ok_or_else(|| DatabaseError::configuration("MongoDB collection namespace is required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mongodb_management::MongoManagementOperation as Op;

    #[test]
    fn management_requests_reject_unknown_fields_and_protected_index_operations() {
        let namespace = MongoNamespace::collection("app", "items");
        assert!(
            MongoAdapter::validate_management_request(
                &Op::CreateCollection {
                    database: "app".into()
                },
                &doc! { "name": "items", "unexpected": true }
            )
            .is_err()
        );
        assert!(
            MongoAdapter::validate_management_request(
                &Op::DropIndex {
                    namespace: namespace.clone(),
                    name: "_id_".into()
                },
                &Document::new()
            )
            .is_err()
        );
        assert!(
            MongoAdapter::validate_management_request(
                &Op::DropIndex {
                    namespace: namespace.clone(),
                    name: "*".into()
                },
                &Document::new()
            )
            .is_err()
        );
        assert!(
            MongoAdapter::validate_management_request(
                &Op::DropCollection {
                    namespace,
                    kind: MongoCollectionType::Collection
                },
                &doc! { "force": true }
            )
            .is_err()
        );
    }

    #[test]
    fn management_requests_keep_ordered_index_keys_and_reject_empty_keys() {
        let operation = Op::CreateIndex {
            namespace: MongoNamespace::collection("app", "items"),
        };
        let value: serde_json::Value =
            serde_json::from_str(r#"{"keys":{"z":1,"a":-1},"options":{"unique":true}}"#).unwrap();
        let valid = mongodb::bson::Bson::try_from(value)
            .unwrap()
            .as_document()
            .unwrap()
            .clone();
        assert!(MongoAdapter::validate_management_request(&operation, &valid).is_ok());
        let keys = valid.get_document("keys").unwrap();
        assert_eq!(
            keys.keys().map(String::as_str).collect::<Vec<_>>(),
            ["z", "a"]
        );
        assert!(
            MongoAdapter::validate_management_request(
                &operation,
                &doc! { "keys": {}, "options": {} }
            )
            .is_err()
        );
    }
}
