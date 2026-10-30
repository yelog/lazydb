use futures_util::StreamExt;
use mongodb::bson::{Document, raw::RawDocumentBuf};
use std::time::{Duration, Instant};

use crate::db::{
    DatabaseError, ErrorCategory, RelationPreview,
    query::{ColumnMeta, QueryOutcome, ResultSet},
    value::CellValue,
};
use crate::model::pagination::{PageRequest, PageSize, ResultPagination, TotalRows};

use super::{
    MongoAdapter,
    types::{FindSpec, MongoDocument, MongoDocumentPage, MongoNamespace},
};

/// The field the document grid sorts on so one page request is stable.
const PAGINATION_SORT_FIELD: &str = "_id";

/// One relation page plus the BSON documents its grid rows were mapped from.
///
/// The modal needs the originals: the grid keeps only `CellValue`s, and an
/// `_id` rendered as text is not the value a write must be bounded to.
#[derive(Clone, Debug)]
pub struct MongoRelationPage {
    pub preview: RelationPreview,
    pub documents: Vec<MongoDocument>,
}

impl MongoAdapter {
    /// Loads one page of documents for the relation DATA grid.
    ///
    /// The page budget is the `FindSpec` skip/limit budget, so the grid reads a
    /// bounded page of the collection instead of draining it.
    pub async fn relation_page(
        &self,
        namespace: &MongoNamespace,
        request: PageRequest,
    ) -> Result<MongoRelationPage, DatabaseError> {
        let spec = relation_find_spec(namespace, request);
        let started = Instant::now();
        let page = self.find_page(&spec).await?;
        let execution = started.elapsed();
        let started = Instant::now();
        let pagination = relation_pagination(&spec, &page);
        let result = page_result_set(&page);
        let fetch = started.elapsed();
        Ok(MongoRelationPage {
            preview: RelationPreview {
                sql: find_description(&spec),
                result: QueryOutcome::from_result_set(result, execution, fetch),
                pagination,
                row_versions: None,
            },
            documents: page.documents,
        })
    }

    /// Loads one page of documents for the relation DATA grid, discarding the
    /// BSON originals.
    pub async fn relation_preview(
        &self,
        namespace: &MongoNamespace,
        request: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        Ok(self.relation_page(namespace, request).await?.preview)
    }

    pub async fn distinct_values(
        &self,
        spec: &FindSpec,
        field: &str,
    ) -> Result<Vec<mongodb::bson::Bson>, DatabaseError> {
        spec.validate().map_err(DatabaseError::configuration)?;
        if field.is_empty() || field.starts_with('$') {
            return Err(DatabaseError::configuration("distinct field is invalid"));
        }
        let collection = self
            .client()
            .database(&spec.namespace.database)
            .collection::<Document>(
                spec.namespace
                    .collection
                    .as_deref()
                    .expect("validated collection"),
            );
        collection
            .distinct(field, spec.filter.clone())
            .await
            .map_err(super::map_error)
    }

    pub async fn aggregate_page(
        &self,
        namespace: &super::types::MongoNamespace,
        pipeline: Vec<Document>,
        page_size: u32,
        max_bytes: usize,
    ) -> Result<MongoDocumentPage, DatabaseError> {
        if namespace.collection.as_deref().is_none_or(str::is_empty) {
            return Err(DatabaseError::configuration(
                "MongoDB collection namespace is required",
            ));
        }
        if page_size == 0 || page_size > FindSpec::MAX_PAGE_SIZE || max_bytes == 0 {
            return Err(DatabaseError::configuration(
                "aggregate result budget is invalid",
            ));
        }
        if pipeline.iter().any(|stage| {
            stage.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "$out" | "$merge" | "$where" | "$function" | "$accumulator"
                )
            })
        }) {
            return Err(DatabaseError::unsupported(
                "aggregate pipeline contains an unsupported write or executable stage",
            ));
        }
        let collection = self
            .client()
            .database(&namespace.database)
            .collection::<Document>(namespace.collection.as_deref().expect("checked collection"));
        let mut cursor = collection
            .aggregate(pipeline)
            .with_options(
                mongodb::options::AggregateOptions::builder()
                    .batch_size(Some(page_size))
                    .max_time(Some(Duration::from_secs(30)))
                    .build(),
            )
            .await
            .map_err(super::map_error)?;
        let mut documents = Vec::with_capacity(page_size as usize);
        let mut bytes = 0usize;
        let mut more = false;
        while let Some(document) = cursor.next().await {
            let document = document.map_err(super::map_error)?;
            let raw_bytes = mongodb::bson::to_vec(&document).map_err(|_| {
                DatabaseError::configuration("aggregate returned a document that cannot be encoded")
            })?;
            if raw_bytes.len() > max_bytes
                || bytes.saturating_add(raw_bytes.len()) > max_bytes
                || documents.len() >= page_size as usize
            {
                more = true;
                break;
            }
            bytes += raw_bytes.len();
            let raw = RawDocumentBuf::from_bytes(raw_bytes).map_err(|_| {
                DatabaseError::configuration("aggregate returned an invalid raw BSON document")
            })?;
            documents.push(MongoDocument {
                bson: document,
                raw,
                editable: false,
            });
        }
        Ok(MongoDocumentPage {
            documents,
            next_offset: None,
            truncated: more,
            oversized_document: false,
            bytes,
        })
    }

    /// Loads exactly one bounded page. Pagination uses an explicit stable `_id`
    /// sort and offset so each request can be independently cancelled/reissued;
    /// it never drains the full collection.
    pub async fn find_page(&self, spec: &FindSpec) -> Result<MongoDocumentPage, DatabaseError> {
        spec.validate().map_err(DatabaseError::configuration)?;
        let collection_name = spec
            .namespace
            .collection
            .as_deref()
            .expect("validated collection");
        let collection = self
            .client()
            .database(&spec.namespace.database)
            .collection::<Document>(collection_name);
        let remaining = spec.limit.map(|limit| limit.saturating_sub(spec.skip));
        if remaining == Some(0) {
            return Ok(MongoDocumentPage {
                documents: Vec::new(),
                next_offset: None,
                truncated: false,
                oversized_document: false,
                bytes: 0,
            });
        }
        let take = u64::from(spec.page_size).min(remaining.unwrap_or(u64::MAX));
        let mut options = mongodb::options::FindOptions::builder()
            .skip(spec.skip)
            .limit(Some(take as i64))
            .batch_size(Some(spec.batch_size.min(spec.page_size)))
            .max_time(Some(Duration::from_secs(30)))
            .build();
        options.sort = spec.sort.clone();
        options.projection = spec.projection.clone();
        let mut cursor = collection
            .find(spec.filter.clone())
            .with_options(options)
            .batch()
            .await
            .map_err(super::map_error)?;
        let mut documents = Vec::with_capacity(spec.page_size as usize);
        let mut bytes = 0usize;
        let mut oversized_document = false;
        let mut more = false;
        while let Some(batch) = cursor.next().await {
            let batch = batch.map_err(super::map_error)?;
            for raw in batch.doc_slices().map_err(super::map_error)? {
                let raw = raw
                    .map_err(|_| DatabaseError {
                        category: ErrorCategory::Internal,
                        code: Some("mongodb_bson_decode".to_owned()),
                        message: "MongoDB returned an invalid raw document".to_owned(),
                        diagnostic: None,
                    })?
                    .as_document()
                    .ok_or_else(|| DatabaseError {
                        category: ErrorCategory::Internal,
                        code: Some("mongodb_bson_decode".to_owned()),
                        message: "MongoDB returned a non-document value in a document cursor"
                            .to_owned(),
                        diagnostic: None,
                    })?;
                let raw_len = raw.as_bytes().len();
                if raw_len > spec.max_document_bytes {
                    oversized_document = true;
                    more = true;
                    break;
                }
                if documents.len() >= spec.page_size as usize
                    || bytes.saturating_add(raw_len) > spec.max_page_bytes
                {
                    more = true;
                    break;
                }
                let bson: Document =
                    mongodb::bson::from_slice(raw.as_bytes()).map_err(|_| DatabaseError {
                        category: ErrorCategory::Internal,
                        code: Some("mongodb_bson_decode".to_owned()),
                        message: "MongoDB returned a document that could not be decoded".to_owned(),
                        diagnostic: None,
                    })?;
                bytes += raw_len;
                documents.push(MongoDocument {
                    bson,
                    raw: RawDocumentBuf::from_bytes(raw.as_bytes().to_vec()).map_err(|_| {
                        DatabaseError {
                            category: ErrorCategory::Internal,
                            code: Some("mongodb_bson_decode".to_owned()),
                            message: "MongoDB returned a document that could not be retained"
                                .to_owned(),
                            diagnostic: None,
                        }
                    })?,
                    editable: true,
                });
            }
            if more {
                break;
            }
        }
        if !more
            && documents.len() as u64 == take
            && remaining.is_none_or(|remaining| remaining > take)
        {
            // The server limit is deliberately bounded to one page, so it cannot
            // tell us whether another result exists without issuing another query.
            more = true;
        }
        let next_offset = more.then(|| spec.skip.saturating_add(documents.len() as u64));
        let truncated = oversized_document || (more && documents.is_empty());
        let next_offset = if oversized_document {
            None
        } else {
            next_offset
        };
        Ok(MongoDocumentPage {
            documents,
            next_offset,
            truncated,
            oversized_document,
            bytes,
        })
    }

    pub async fn count_documents(&self, spec: &FindSpec) -> Result<u64, DatabaseError> {
        spec.validate().map_err(DatabaseError::configuration)?;
        let collection_name = spec
            .namespace
            .collection
            .as_deref()
            .expect("validated collection");
        self.client()
            .database(&spec.namespace.database)
            .collection::<Document>(collection_name)
            .count_documents(spec.filter.clone())
            .await
            .map_err(super::map_error)
    }

    pub async fn estimated_document_count(&self, spec: &FindSpec) -> Result<u64, DatabaseError> {
        spec.validate().map_err(DatabaseError::configuration)?;
        let collection_name = spec
            .namespace
            .collection
            .as_deref()
            .expect("validated collection");
        self.client()
            .database(&spec.namespace.database)
            .collection::<Document>(collection_name)
            .estimated_document_count()
            .await
            .map_err(super::map_error)
    }
}

/// Builds the `FindSpec` for one relation grid page.
///
/// The grid page sizes are larger than one bounded page the driver budget
/// allows, so the size is clamped to [`FindSpec::MAX_PAGE_SIZE`]. The limit
/// asks for one lookahead document so a full page still advertises a next page.
pub fn relation_find_spec(namespace: &MongoNamespace, request: PageRequest) -> FindSpec {
    let page_size = request.size.get().min(FindSpec::MAX_PAGE_SIZE as usize) as u32;
    let mut spec = FindSpec::new(namespace.clone());
    // An explicit stable sort keeps offsets comparable between page requests.
    spec.sort = Some(mongodb::bson::doc! { PAGINATION_SORT_FIELD: 1 });
    spec.page_size = page_size;
    spec.batch_size = page_size;
    spec.skip = request.offset;
    spec.limit = Some(
        request
            .offset
            .saturating_add(u64::from(page_size))
            .saturating_add(1),
    );
    spec
}

/// Human-readable description of the `find` command the preview executes.
///
/// The relation DATA view always shows the statement behind a snapshot, so the
/// MongoDB preview describes its own read instead of claiming to be SQL.
pub fn find_description(spec: &FindSpec) -> String {
    let collection = spec.namespace.collection.as_deref().unwrap_or_default();
    let mut description = format!("db.{collection}.find({})", spec.filter);
    if let Some(sort) = &spec.sort {
        description.push_str(&format!(".sort({sort})"));
    }
    if let Some(projection) = &spec.projection {
        description.push_str(&format!(".projection({projection})"));
    }
    description.push_str(&format!(".skip({})", spec.skip));
    if let Some(limit) = spec.limit {
        description.push_str(&format!(".limit({limit})"));
    }
    description
}

/// Maps one document page onto the relation grid result set.
pub fn page_result_set(page: &MongoDocumentPage) -> ResultSet {
    let columns = crate::model::mongodb_browser::page_columns(page);
    ResultSet {
        rows: page
            .documents
            .iter()
            .map(|document| {
                columns
                    .iter()
                    .map(|column| document_cell_value(document, column))
                    .collect()
            })
            .collect(),
        columns: columns
            .iter()
            .map(|name| ColumnMeta {
                name: name.clone(),
                type_name: column_type_name(page, name),
            })
            .collect(),
        affected_rows: 0,
    }
}

/// One grid cell for a top-level field.
///
/// BSON maps onto the existing [`CellValue`] variants: scalars keep their type
/// and every other value reuses the document grid text, so an `ObjectId` is its
/// hex form and nested shapes are their `{n fields}` / `[n]` markers.
fn document_cell_value(document: &MongoDocument, field: &str) -> CellValue {
    use mongodb::bson::Bson;

    let Some(value) = document.bson.get(field) else {
        return CellValue::Null;
    };
    match value {
        Bson::String(text) => CellValue::Text(text.clone()),
        Bson::Int32(number) => CellValue::Integer(i64::from(*number)),
        Bson::Int64(number) => CellValue::Integer(*number),
        Bson::Double(number) => CellValue::Float(*number),
        Bson::Boolean(flag) => CellValue::Boolean(*flag),
        Bson::Null | Bson::Undefined => CellValue::Null,
        _ => CellValue::Text(crate::model::mongodb_browser::document_cell(
            document, field,
        )),
    }
}

/// The observed BSON type of a column, taken from the first document carrying it.
fn column_type_name(page: &MongoDocumentPage, field: &str) -> String {
    page.documents
        .iter()
        .find_map(|document| document.bson.get(field))
        .map(|value| format!("{:?}", value.element_type()))
        .unwrap_or_default()
}

/// Paginates the page that was actually fetched, never the requested size.
fn relation_pagination(spec: &FindSpec, page: &MongoDocumentPage) -> ResultPagination {
    let size = PageSize::try_from(spec.page_size as usize).unwrap_or(PageSize::FiveHundred);
    let mut pagination = ResultPagination::from_page(
        PageRequest {
            size,
            offset: spec.skip,
            resolve_total: false,
        },
        page.documents.len(),
    );
    if page.next_offset.is_some() {
        pagination.has_next = true;
        pagination.total = TotalRows::LowerBound(
            spec.skip
                .saturating_add(pagination.visible_rows as u64)
                .saturating_add(1),
        );
    }
    pagination
}
