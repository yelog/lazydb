use futures_util::TryStreamExt;
use mongodb::{bson::Document, results::CollectionType};

use crate::db::DatabaseError;

use super::{
    MongoAdapter,
    types::{
        DatabaseListAuthority, MongoCollectionInfo, MongoCollectionStructure, MongoCollectionType,
        MongoDatabaseListing, MongoIndexInfo, MongoNamespace,
    },
};

impl MongoAdapter {
    pub async fn list_databases(&self) -> Result<Vec<String>, DatabaseError> {
        self.list_databases_with_authority(DatabaseListAuthority::AuthorizedOnly)
            .await
            .map(|listing| listing.databases)
    }

    pub async fn list_databases_with_authority(
        &self,
        authority: DatabaseListAuthority,
    ) -> Result<MongoDatabaseListing, DatabaseError> {
        let authorized_only = authority == DatabaseListAuthority::AuthorizedOnly;
        match self
            .client()
            .list_database_names()
            .authorized_databases(authorized_only)
            .await
        {
            Ok(names) => Ok(MongoDatabaseListing {
                databases: names,
                authorized_only,
                warning: None,
            }),
            Err(error) if authorized_only && is_command_unsupported(&error) => {
                Ok(MongoDatabaseListing {
                    databases: vec![self.selected_database().to_owned()],
                    authorized_only: false,
                    warning: Some("server does not support authorized database listing; showing only the selected database".to_owned()),
                })
            }
            Err(error) => Err(super::map_error(error)),
        }
    }

    pub async fn list_collections(
        &self,
        database: &str,
    ) -> Result<Vec<MongoCollectionInfo>, DatabaseError> {
        let cursor = self
            .client()
            .database(database)
            .list_collections()
            .await
            .map_err(super::map_error)?;
        let descriptions = cursor
            .try_collect::<Vec<_>>()
            .await
            .map_err(super::map_error)?;
        descriptions
            .into_iter()
            .map(|description| {
                let options_ejson = Some(
                    mongodb::bson::to_bson(&description.options)
                        .map_err(|_| {
                            DatabaseError::configuration(
                                "MongoDB collection metadata is not valid BSON",
                            )
                        })?
                        .into_relaxed_extjson()
                        .to_string(),
                );
                Ok(MongoCollectionInfo {
                    namespace: MongoNamespace::collection(database, description.name),
                    collection_type: collection_type(description.collection_type),
                    options_ejson,
                })
            })
            .collect()
    }

    /// Loads the real metadata MongoDB reports for one collection: its type,
    /// its options, the validator when it defines one, and every index.
    ///
    /// The validator and its validation level/action are reported separately
    /// from the remaining options so the structure report can state both without
    /// repeating the validator inside the options.
    pub async fn collection_structure(
        &self,
        namespace: &MongoNamespace,
    ) -> Result<MongoCollectionStructure, DatabaseError> {
        let collection = namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        let specifications = self
            .client()
            .database(&namespace.database)
            .list_collections()
            .filter(mongodb::bson::doc! { "name": collection })
            .await
            .map_err(super::map_error)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(super::map_error)?;
        let description = specifications.into_iter().next().ok_or_else(|| {
            DatabaseError::configuration(format!(
                "MongoDB collection {} is no longer available",
                namespace.full_name()
            ))
        })?;
        let options = &description.options;
        let mut remaining_options = mongodb::bson::to_document(options).map_err(|_| {
            DatabaseError::configuration("MongoDB collection metadata is not valid BSON")
        })?;
        for key in ["validator", "validationLevel", "validationAction"] {
            remaining_options.remove(key);
        }
        let options_ejson = (!remaining_options.is_empty()).then(|| {
            mongodb::bson::Bson::Document(remaining_options)
                .into_relaxed_extjson()
                .to_string()
        });
        let validator_ejson = options.validator.as_ref().map(|validator| {
            let mut document = mongodb::bson::Document::new();
            document.insert("validator", validator.clone());
            if let Some(level) = &options.validation_level {
                document.insert("validationLevel", format!("{level:?}").to_lowercase());
            }
            if let Some(action) = &options.validation_action {
                document.insert("validationAction", format!("{action:?}").to_lowercase());
            }
            mongodb::bson::Bson::Document(document)
                .into_relaxed_extjson()
                .to_string()
        });
        // A view has no indexes of its own and a restricted user may not read
        // them; the report states that instead of claiming the collection has
        // none.
        let (indexes, indexes_error) = match self.list_indexes(namespace).await {
            Ok(indexes) => (indexes, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        Ok(MongoCollectionStructure {
            namespace: namespace.clone(),
            collection_type: collection_type(description.collection_type),
            options_ejson,
            validator_ejson,
            indexes,
            indexes_error,
        })
    }

    pub async fn list_indexes(
        &self,
        namespace: &MongoNamespace,
    ) -> Result<Vec<MongoIndexInfo>, DatabaseError> {
        let collection = namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        let cursor = self
            .client()
            .database(&namespace.database)
            .collection::<Document>(collection)
            .list_indexes()
            .await
            .map_err(super::map_error)?;
        cursor
            .try_collect::<Vec<_>>()
            .await
            .map_err(super::map_error)?
            .into_iter()
            .map(|model| {
                let name = model
                    .options
                    .as_ref()
                    .and_then(|options| options.name.clone())
                    .ok_or_else(|| {
                        DatabaseError::configuration("MongoDB returned an index without a name")
                    })?;
                let keys = model.keys.clone();
                let key_description = keys
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                let raw = mongodb::bson::to_document(&model).map_err(|_| {
                    DatabaseError::configuration("MongoDB returned invalid index metadata")
                })?;
                Ok(MongoIndexInfo {
                    name,
                    keys,
                    unique: model
                        .options
                        .as_ref()
                        .and_then(|options| options.unique)
                        .unwrap_or(false),
                    ttl_seconds: model
                        .options
                        .as_ref()
                        .and_then(|options| options.expire_after)
                        .map(|duration| duration.as_secs() as i64),
                    key_description,
                    raw,
                })
            })
            .collect()
    }
}

fn is_command_unsupported(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 59 || command.code == 72)
}

/// The catalog type of a collection, shared by the listing and the structure
/// report so both describe a collection the same way.
fn collection_type(collection_type: CollectionType) -> MongoCollectionType {
    match collection_type {
        CollectionType::Collection => MongoCollectionType::Collection,
        CollectionType::View => MongoCollectionType::View,
        CollectionType::Timeseries => MongoCollectionType::Timeseries,
        _ => MongoCollectionType::Other,
    }
}
