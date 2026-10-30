//! Structure report for a MongoDB collection in the relation DDL view.
//!
//! MongoDB has no DDL (S2), so the DDL tab of a collection shows a report with
//! two halves, in this order:
//!
//! 1. the metadata the server really reports — collection type, options, the
//!    validator when the collection defines one, and every index;
//! 2. a schema inferred from the documents the DATA grid already loaded — one
//!    line per top-level field with the observed BSON types and the percentage
//!    of loaded documents carrying it.
//!
//! Both halves are pure functions of their inputs, so the report is testable
//! without a server. Metadata that is absent is stated as absent; when no page
//! is loaded the inference says so instead of guessing.

use mongodb::bson::Bson;

use crate::db::{
    catalog::{
        CatalogCompleteness, CatalogCount, CatalogEntry, CatalogId, CatalogKind, CatalogPage,
        CatalogRequestKey, CatalogTarget, DdlProvenance, OptionalMetadata, RelationDdl,
    },
    mongodb::types::{
        MongoCollectionStructure, MongoCollectionType, MongoDocumentPage, MongoIndexInfo,
    },
};
use crate::model::relation::{RelationDescriptor, RelationRequest};

/// One top-level field observed in the loaded page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MongoFieldShape {
    pub name: String,
    /// Observed BSON type labels in first-seen order, deduplicated.
    pub types: Vec<String>,
    /// Documents of the page that carry the field.
    pub present: usize,
}

impl MongoFieldShape {
    /// Percentage of loaded documents carrying the field, rounded down.
    pub fn presence_percent(&self, documents: usize) -> usize {
        if documents == 0 {
            return 0;
        }
        self.present.saturating_mul(100) / documents
    }
}

/// The schema observed in one loaded page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MongoSchemaInference {
    pub documents: usize,
    pub fields: Vec<MongoFieldShape>,
}

/// Everything the structure report is rendered from.
#[derive(Clone, Debug, PartialEq)]
pub struct MongoStructureSnapshot {
    pub metadata: MongoCollectionStructure,
    /// `None` when the DATA grid has not loaded a page, so nothing is observed.
    pub schema: Option<MongoSchemaInference>,
}

/// Infers per-field BSON types and presence from the loaded document page.
///
/// Only top-level fields are reported: a nested document is described as a
/// nested shape, so its keys never become top-level fields of the report.
pub fn infer_schema(page: &MongoDocumentPage) -> MongoSchemaInference {
    infer_schema_documents(&page.documents)
}

/// Infers the schema from the exact MongoDB documents retained by a loaded DATA
/// page. This lets the DDL report share that snapshot without issuing another
/// database read.
pub fn infer_schema_documents(
    documents: &[crate::db::mongodb::types::MongoDocument],
) -> MongoSchemaInference {
    let mut fields: Vec<MongoFieldShape> = Vec::new();
    for document in documents {
        for (name, value) in document.bson.iter() {
            let label = type_label(value);
            match fields.iter_mut().find(|field| &field.name == name) {
                Some(field) => {
                    field.present = field.present.saturating_add(1);
                    if !field.types.contains(&label) {
                        field.types.push(label);
                    }
                }
                None => fields.push(MongoFieldShape {
                    name: name.clone(),
                    types: vec![label],
                    present: 1,
                }),
            }
        }
    }
    MongoSchemaInference {
        documents: documents.len(),
        fields,
    }
}

/// BSON type label in Mongo shell spelling.
///
/// An embedded document keeps its sorted keys inside the marker, so the report
/// shows the nested shape instead of leaking the keys as top-level fields.
fn type_label(value: &Bson) -> String {
    match value {
        Bson::Double(_) => "double".to_owned(),
        Bson::String(_) => "string".to_owned(),
        Bson::Array(_) => "array".to_owned(),
        Bson::Document(document) => {
            let mut keys = document.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            format!("object{{{}}}", keys.join(","))
        }
        Bson::Boolean(_) => "bool".to_owned(),
        Bson::Null => "null".to_owned(),
        Bson::RegularExpression(_) => "regex".to_owned(),
        Bson::JavaScriptCode(_) => "javascript".to_owned(),
        Bson::JavaScriptCodeWithScope(_) => "javascriptWithScope".to_owned(),
        Bson::Int32(_) => "int".to_owned(),
        Bson::Int64(_) => "long".to_owned(),
        Bson::Timestamp(_) => "timestamp".to_owned(),
        Bson::Binary(_) => "binData".to_owned(),
        Bson::ObjectId(_) => "objectId".to_owned(),
        Bson::DateTime(_) => "date".to_owned(),
        Bson::Symbol(_) => "symbol".to_owned(),
        Bson::Decimal128(_) => "decimal".to_owned(),
        Bson::Undefined => "undefined".to_owned(),
        Bson::MaxKey => "maxKey".to_owned(),
        Bson::MinKey => "minKey".to_owned(),
        Bson::DbPointer(_) => "dbPointer".to_owned(),
    }
}

/// Renders the whole structure report, metadata first and inferred schema last.
pub fn render_report(snapshot: &MongoStructureSnapshot) -> String {
    let metadata = &snapshot.metadata;
    let mut lines = vec![
        format!("MongoDB collection: {}", metadata.namespace.full_name()),
        format!(
            "collection type: {}",
            collection_type_label(metadata.collection_type)
        ),
        format!(
            "collection options: {}",
            metadata.options_ejson.as_deref().unwrap_or("none")
        ),
        format!(
            "validator: {}",
            metadata.validator_ejson.as_deref().unwrap_or("none")
        ),
    ];
    if let Some(error) = &metadata.indexes_error {
        lines.push(format!("indexes: unavailable ({error})"));
    } else if metadata.indexes.is_empty() {
        lines.push("indexes: none".to_owned());
    } else {
        lines.push("indexes:".to_owned());
        for index in &metadata.indexes {
            lines.push(format!("  - {}", index_line(index)));
        }
    }
    match &snapshot.schema {
        None => lines.push(
            "schema: no page is loaded in the DATA view, so no schema is inferred".to_owned(),
        ),
        Some(inference) if inference.documents == 0 => lines.push(
            "schema inferred from the loaded page (0 documents): no document is loaded, so no field is observed"
                .to_owned(),
        ),
        Some(inference) => {
            let documents = inference.documents;
            let noun = if documents == 1 {
                "document"
            } else {
                "documents"
            };
            lines.push(format!(
                "schema inferred from the loaded page ({documents} {noun}):"
            ));
            for field in &inference.fields {
                lines.push(format!(
                    "  {}: {} ({}% of {documents} {noun})",
                    field.name,
                    field.types.join(", "),
                    field.presence_percent(documents),
                ));
            }
        }
    }
    let mut report = lines.join("\n");
    report.push('\n');
    report
}

const fn collection_type_label(collection_type: MongoCollectionType) -> &'static str {
    match collection_type {
        MongoCollectionType::Collection => "collection",
        MongoCollectionType::View => "view",
        MongoCollectionType::Timeseries => "timeseries",
        MongoCollectionType::Other => "other",
    }
}

fn index_line(index: &MongoIndexInfo) -> String {
    let mut line = format!("{}: {}", index.name, index.keys);
    let mut flags = Vec::new();
    if index.unique {
        flags.push("unique".to_owned());
    }
    if index.sparse() {
        flags.push("sparse".to_owned());
    }
    if let Some(seconds) = index.ttl_seconds {
        flags.push(format!("ttl {seconds}s"));
    }
    if let Some(filter) = index.partial_filter_ejson() {
        flags.push(format!("partial {filter}"));
    }
    if !flags.is_empty() {
        line.push_str(&format!(" ({})", flags.join(", ")));
    }
    line
}

/// Wraps the report in the `RelationDdl` the DDL view already renders.
///
/// A collection reports no SQL columns, so the entry is derived from the
/// relation descriptor and the children page stays empty: the report text is
/// the whole structure and no SQL metadata is invented.
pub fn relation_ddl(
    request: &RelationRequest,
    descriptor: &RelationDescriptor,
    text: String,
) -> Result<RelationDdl, String> {
    let profile_id = descriptor.key.profile_id;
    let relation = CatalogEntry::relation(
        descriptor.key.object_id.clone(),
        // A collection has no schema; the descriptor's database segment is the
        // parent the SQL catalog shape requires, and no lookup uses it.
        CatalogId::new(
            profile_id,
            CatalogKind::Schema,
            [descriptor
                .qualified_name
                .database
                .clone()
                .unwrap_or_default()],
        ),
        descriptor.qualified_name.clone(),
        "collection",
        OptionalMetadata::Unsupported,
        false,
    )
    .map_err(|error| format!("MongoDB relation metadata is inconsistent: {error}"))?;
    // `CatalogPage::new` validates SQL target shapes that a
    // `[database, collection]` path cannot satisfy, so the page is built as the
    // inert container `RelationDdl` needs: the report text is the structure.
    let children = CatalogPage {
        key: CatalogRequestKey {
            connection: request.connection,
            catalog_epoch: 0,
            request_id: request.request_id,
            target: CatalogTarget::RelationChildren {
                relation: descriptor.key.object_id.clone(),
            },
            cursor: None,
        },
        entries: Vec::new(),
        group_summaries: Vec::new(),
        total_count: CatalogCount::Exact(0),
        next_cursor: None,
        completeness: CatalogCompleteness::Complete,
    };
    Ok(RelationDdl {
        relation,
        children,
        sql: text,
        provenance: DdlProvenance::AdapterGenerated,
    })
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Document, doc, oid::ObjectId, raw::RawDocumentBuf};

    use super::*;
    use crate::db::mongodb::types::{MongoDocument, MongoIndexInfo, MongoNamespace};

    fn profile_id() -> uuid::Uuid {
        uuid::Uuid::nil()
    }

    fn document(bson: Document) -> MongoDocument {
        MongoDocument {
            raw: RawDocumentBuf::from_bytes(mongodb::bson::to_vec(&bson).expect("encodes"))
                .expect("raw document"),
            bson,
            editable: true,
        }
    }

    fn page(documents: Vec<MongoDocument>) -> MongoDocumentPage {
        MongoDocumentPage {
            documents,
            next_offset: None,
            truncated: false,
            oversized_document: false,
            bytes: 0,
        }
    }

    fn index(name: &str, keys: Document) -> MongoIndexInfo {
        MongoIndexInfo {
            name: name.into(),
            key_description: keys
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            keys: keys.clone(),
            unique: false,
            ttl_seconds: None,
            raw: doc! { "name": name, "key": keys },
        }
    }

    fn metadata(indexes: Vec<MongoIndexInfo>) -> MongoCollectionStructure {
        MongoCollectionStructure {
            namespace: MongoNamespace::collection("speedbike_db", "bikes"),
            collection_type: MongoCollectionType::Collection,
            options_ejson: None,
            validator_ejson: None,
            indexes,
            indexes_error: None,
        }
    }

    fn relation_descriptor() -> RelationDescriptor {
        RelationDescriptor::mongodb_collection(
            profile_id(),
            &MongoNamespace::collection("speedbike_db", "bikes"),
        )
        .expect("collection descriptor")
    }

    fn request() -> RelationRequest {
        use crate::identity::ConnectionIdentity;
        use crate::model::{
            pagination::{PageRequest, PageSize},
            relation::{RelationKey, RelationRequestKind},
        };
        use crate::profile::CatalogScope;

        let descriptor = relation_descriptor();
        RelationRequest {
            tab_id: uuid::Uuid::new_v4(),
            console_id: uuid::Uuid::new_v4(),
            tab_generation: 0,
            request_id: 3,
            connection: ConnectionIdentity {
                profile_id: descriptor.key.profile_id,
                generation: 1,
            },
            relation: RelationKey {
                profile_id: descriptor.key.profile_id,
                object_id: descriptor.key.object_id.clone(),
            },
            kind: RelationRequestKind::Ddl,
            scope: CatalogScope::for_profile(
                crate::profile::DatabaseKind::MongoDb,
                "speedbike_db",
                None,
            ),
            options: Default::default(),
            page: PageRequest::first(PageSize::default()),
        }
    }

    #[test]
    fn inference_reports_every_observed_type_and_presence_per_field() {
        let inferred = infer_schema(&page(vec![
            document(doc! { "_id": ObjectId::new(), "stock": 4_i64, "name": "Speedbike" }),
            document(doc! { "_id": ObjectId::new(), "stock": 4_i32 }),
        ]));

        assert_eq!(inferred.documents, 2);
        assert_eq!(
            inferred
                .fields
                .iter()
                .map(|field| (field.name.as_str(), field.types.join(","), field.present))
                .collect::<Vec<_>>(),
            vec![
                ("_id", "objectId".to_string(), 2),
                ("stock", "long,int".to_string(), 2),
                ("name", "string".to_string(), 1),
            ],
            "every field keeps the types observed for it and the documents carrying it"
        );
        let name = inferred
            .fields
            .iter()
            .find(|field| field.name == "name")
            .expect("name observed");
        assert_eq!(
            name.presence_percent(inferred.documents),
            50,
            "a field present in half of the loaded documents is reported at 50%"
        );
    }

    #[test]
    fn a_nested_document_is_a_nested_shape_and_not_leaked_top_level_keys() {
        let inferred = infer_schema(&page(vec![document(doc! {
            "_id": 1_i32,
            "geometry": { "wheels": 2_i32, "frame": "carbon" },
        })]));

        assert_eq!(
            inferred
                .fields
                .iter()
                .map(|field| (field.name.as_str(), field.types.join(",")))
                .collect::<Vec<_>>(),
            vec![
                ("_id", "int".to_string()),
                ("geometry", "object{frame,wheels}".to_string()),
            ],
            "a nested document is one nested shape; its keys are not top-level fields"
        );
    }

    #[test]
    fn the_report_states_absent_metadata_as_absent_and_lists_every_index() {
        let report = render_report(&MongoStructureSnapshot {
            metadata: metadata(vec![
                index("_id_", doc! { "_id": 1_i32 }),
                index(
                    "race_position",
                    doc! { "raceId": 1_i32, "position": -1_i32 },
                ),
            ]),
            schema: Some(infer_schema(&page(vec![document(doc! {
                "_id": 1_i32,
                "raceId": "race-1",
            })]))),
        });

        assert!(
            report.starts_with("MongoDB collection: speedbike_db.bikes"),
            "the report names the collection it describes:\n{report}"
        );
        assert!(
            report.contains("collection type: collection"),
            "the report states the collection type:\n{report}"
        );
        assert!(
            report.contains("collection options: none"),
            "absent collection options are stated as absent:\n{report}"
        );
        assert!(
            report.contains("validator: none"),
            "an absent validator is stated as absent, never invented:\n{report}"
        );
        assert!(
            report.contains("_id_") && report.contains("\"_id\": 1"),
            "every index name and key document is listed:\n{report}"
        );
        assert!(
            report.contains("race_position") && report.contains("\"position\": -1"),
            "every index name and key document is listed:\n{report}"
        );
        let indexes_at = report.find("indexes:").expect("indexes section");
        let schema_at = report
            .find("schema inferred from the loaded page")
            .expect("schema section");
        assert!(
            indexes_at < schema_at,
            "real metadata is reported first and the inferred schema last:\n{report}"
        );
        assert!(
            report.contains("_id: objectId") || report.contains("_id: int"),
            "the inferred schema names each field and its observed type:\n{report}"
        );
    }

    #[test]
    fn every_index_flag_the_server_reports_is_shown() {
        let mut sparse = index("name_sparse", doc! { "name": 1_i32 });
        sparse.raw.insert("sparse", true);
        let mut ttl = index("seen_ttl", doc! { "seenAt": 1_i32 });
        ttl.ttl_seconds = Some(3600);
        let mut partial = index("email_partial", doc! { "email": 1_i32 });
        partial.raw.insert(
            "partialFilterExpression",
            doc! { "email": { "$exists": true } },
        );
        let report = render_report(&MongoStructureSnapshot {
            metadata: metadata(vec![
                index("_id_", doc! { "_id": 1_i32 }),
                sparse,
                ttl,
                partial,
            ]),
            schema: None,
        });

        fn line_with<'a>(report: &'a str, needle: &str) -> &'a str {
            report
                .lines()
                .find(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("no index line for {needle}:\n{report}"))
        }

        assert!(
            line_with(&report, "name_sparse").contains("sparse"),
            "a sparse index says so:\n{report}"
        );
        assert!(
            line_with(&report, "seen_ttl").contains("ttl 3600s"),
            "a TTL index says so:\n{report}"
        );
        assert!(
            line_with(&report, "email_partial").contains("partial {"),
            "a partial index shows its filter:\n{report}"
        );
        let plain = line_with(&report, "_id_");
        for flag in ["unique", "sparse", "ttl", "partial"] {
            assert!(
                !plain.contains(flag),
                "an index without {flag} does not claim it:\n{report}"
            );
        }
    }

    #[test]
    fn the_report_states_when_no_page_was_loaded_instead_of_guessing() {
        let report = render_report(&MongoStructureSnapshot {
            metadata: metadata(vec![index("_id_", doc! { "_id": 1_i32 })]),
            schema: None,
        });

        assert!(
            report.contains("schema: no page is loaded in the DATA view"),
            "an unloaded page is stated instead of inventing a schema:\n{report}"
        );
    }

    #[test]
    fn a_loaded_page_without_documents_states_that_no_field_was_observed() {
        let report = render_report(&MongoStructureSnapshot {
            metadata: metadata(Vec::new()),
            schema: Some(infer_schema(&page(Vec::new()))),
        });

        assert!(
            report.contains("indexes: none"),
            "a collection without indexes says so instead of listing nothing:\n{report}"
        );
        assert!(
            report.contains("no document is loaded, so no field is observed"),
            "an empty page is stated instead of inventing fields:\n{report}"
        );
    }

    #[test]
    fn the_relation_ddl_wraps_the_report_without_inventing_sql_columns() {
        let descriptor = relation_descriptor();
        let request = request();

        let ddl = relation_ddl(&request, &descriptor, "MongoDB collection: x".into())
            .expect("mongodb relation ddl");

        assert_eq!(ddl.sql, "MongoDB collection: x");
        assert_eq!(ddl.provenance, DdlProvenance::AdapterGenerated);
        assert_eq!(ddl.relation.id, descriptor.key.object_id);
        assert!(
            ddl.children.entries.is_empty(),
            "a collection reports no SQL columns, so none are invented"
        );
        assert_eq!(ddl.children.key, request_catalog_key(&request));
    }

    fn request_catalog_key(request: &RelationRequest) -> CatalogRequestKey {
        CatalogRequestKey {
            connection: request.connection,
            catalog_epoch: 0,
            request_id: request.request_id,
            target: CatalogTarget::RelationChildren {
                relation: request.relation.object_id.clone(),
            },
            cursor: None,
        }
    }
}
