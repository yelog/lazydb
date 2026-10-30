use mongodb::bson::{Bson, Document, raw::RawDocumentBuf};
use serde::{Deserialize, Serialize};

/// A Mongo namespace is always a database plus an optional collection name.
/// Names are kept verbatim; dots are legal collection-name characters.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct MongoNamespace {
    pub database: String,
    pub collection: Option<String>,
}

impl MongoNamespace {
    pub fn database(database: impl Into<String>) -> Self {
        Self {
            database: database.into(),
            collection: None,
        }
    }

    pub fn collection(database: impl Into<String>, collection: impl Into<String>) -> Self {
        Self {
            database: database.into(),
            collection: Some(collection.into()),
        }
    }

    pub fn full_name(&self) -> String {
        match &self.collection {
            Some(collection) => format!("{}.{}", self.database, collection),
            None => self.database.clone(),
        }
    }

    /// Rebuilds the namespace from a catalog native path (`[database, collection]`).
    pub fn from_native_path(path: &[String]) -> Option<Self> {
        match path {
            [database, collection] => Some(Self::collection(database.clone(), collection.clone())),
            _ => None,
        }
    }
}

/// Holds typed BSON as the source of truth. JSON rendering is explicitly
/// Extended JSON so ObjectId, Decimal128, dates, binary and numeric widths are
/// not silently coerced into ordinary JSON values.
#[derive(Clone, Debug, PartialEq)]
pub struct MongoDocument {
    pub bson: Document,
    pub raw: RawDocumentBuf,
    pub editable: bool,
}

impl MongoDocument {
    pub fn canonical_ejson(&self) -> Result<String, mongodb::bson::ser::Error> {
        Ok(mongodb::bson::to_bson(&self.bson)?
            .into_canonical_extjson()
            .to_string())
    }

    pub fn relaxed_ejson(&self) -> Result<String, mongodb::bson::ser::Error> {
        Ok(mongodb::bson::to_bson(&self.bson)?
            .into_relaxed_extjson()
            .to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MongoCollectionInfo {
    pub namespace: MongoNamespace,
    pub collection_type: MongoCollectionType,
    pub options_ejson: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MongoIndexListing {
    pub namespace: MongoNamespace,
    pub indexes: Vec<MongoIndexInfo>,
    pub error: Option<String>,
    pub permission_denied: bool,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MongoCollectionStructure {
    pub namespace: MongoNamespace,
    pub collection_type: MongoCollectionType,
    /// The options the server reports, without the validator it reports
    /// separately. `None` when the collection defines no other option.
    pub options_ejson: Option<String>,
    /// The validator and its validation level/action, exactly as configured.
    /// `None` when the collection defines no validator.
    pub validator_ejson: Option<String>,
    pub indexes: Vec<MongoIndexInfo>,
    /// Why the index list is unavailable, when the server refused to report it.
    pub indexes_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MongoCollectionType {
    Collection,
    View,
    Timeseries,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MongoIndexInfo {
    pub name: String,
    pub keys: Document,
    pub unique: bool,
    pub ttl_seconds: Option<i64>,
    pub raw: Document,
    pub key_description: Vec<(String, Bson)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FindSpec {
    pub namespace: MongoNamespace,
    pub filter: Document,
    pub sort: Option<Document>,
    pub projection: Option<Document>,
    pub skip: u64,
    pub limit: Option<u64>,
    pub page_size: u32,
    pub batch_size: u32,
    pub max_document_bytes: usize,
    pub max_page_bytes: usize,
}

impl FindSpec {
    pub const DEFAULT_PAGE_SIZE: u32 = 100;
    pub const MAX_PAGE_SIZE: u32 = 500;
    pub const DEFAULT_MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
    pub const DEFAULT_MAX_PAGE_BYTES: usize = 16 * 1024 * 1024;

    pub fn new(namespace: MongoNamespace) -> Self {
        Self {
            namespace,
            filter: Document::new(),
            sort: None,
            projection: None,
            skip: 0,
            limit: None,
            page_size: Self::DEFAULT_PAGE_SIZE,
            batch_size: 100,
            max_document_bytes: Self::DEFAULT_MAX_DOCUMENT_BYTES,
            max_page_bytes: Self::DEFAULT_MAX_PAGE_BYTES,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.namespace.database.is_empty()
            || self
                .namespace
                .collection
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err("database and collection are required");
        }
        if self.page_size == 0 || self.page_size > Self::MAX_PAGE_SIZE {
            return Err("page size must be between 1 and 500");
        }
        if self.batch_size == 0 || self.batch_size > Self::MAX_PAGE_SIZE {
            return Err("batch size must be between 1 and 500");
        }
        if self.max_document_bytes == 0 || self.max_page_bytes == 0 {
            return Err("document and page byte budgets must be positive");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MongoDocumentPage {
    pub documents: Vec<MongoDocument>,
    pub next_offset: Option<u64>,
    pub truncated: bool,
    pub oversized_document: bool,
    pub bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseListAuthority {
    AuthorizedOnly,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MongoDatabaseListing {
    pub databases: Vec<String>,
    pub authorized_only: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum MongoExplorerNodeId {
    Database(String),
    Collection(MongoNamespace),
    Index {
        namespace: MongoNamespace,
        name: String,
    },
    RetryCollections(String),
    RetryIndexes(MongoNamespace),
    Monitor(String),
    Users(String),
}

impl MongoIndexInfo {
    pub fn key_description(&self) -> Vec<(String, Bson)> {
        self.keys
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    /// The server reports `sparse` only for indexes that set it.
    pub fn sparse(&self) -> bool {
        self.raw.get_bool("sparse").unwrap_or(false)
    }

    /// The filter a partial index applies, when it only indexes matching
    /// documents.
    pub fn partial_filter_ejson(&self) -> Option<String> {
        self.raw
            .get_document("partialFilterExpression")
            .ok()
            .map(|filter| {
                Bson::Document(filter.clone())
                    .into_relaxed_extjson()
                    .to_string()
            })
    }
}
