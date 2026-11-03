use futures_util::TryStreamExt;
use mongodb::{
    bson::{Bson, Document, doc},
    options::{Acknowledgment, WriteConcern},
    results::CollectionType,
};

use crate::db::DatabaseError;

use super::{MongoAdapter, map_error, types::MongoNamespace};

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "certainty", rename_all = "snake_case")]
pub enum MongoMutationOutcome {
    Confirmed { affected: u64 },
    Partial { affected: u64, requested: u64 },
    Unknown { requested: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MongoCompareAndSetOutcome {
    Applied,
    Conflict,
    Unknown,
}

pub fn compare_and_set_outcome(matched: u64, modified: Option<u64>) -> MongoCompareAndSetOutcome {
    if matched == 0 {
        MongoCompareAndSetOutcome::Conflict
    } else if modified.is_some_and(|modified| modified == 1) {
        MongoCompareAndSetOutcome::Applied
    } else if modified == Some(0) {
        MongoCompareAndSetOutcome::Conflict
    } else {
        MongoCompareAndSetOutcome::Unknown
    }
}

pub fn error_certainty(error: &mongodb::error::Error) -> MongoMutationOutcome {
    use mongodb::error::{ErrorKind, WriteFailure};
    match error.kind.as_ref() {
        ErrorKind::Command(_)
        | ErrorKind::Write(WriteFailure::WriteError(_))
        | ErrorKind::InvalidArgument { .. }
        | ErrorKind::Authentication { .. }
        | ErrorKind::BsonDeserialization(_)
        | ErrorKind::BsonSerialization(_) => MongoMutationOutcome::Confirmed { affected: 0 },
        ErrorKind::Io(_)
        | ErrorKind::ConnectionPoolCleared { .. }
        | ErrorKind::ServerSelection { .. }
        | ErrorKind::InsertMany(_)
        | ErrorKind::Write(WriteFailure::WriteConcernError(_)) => {
            MongoMutationOutcome::Unknown { requested: 1 }
        }
        _ => MongoMutationOutcome::Unknown { requested: 1 },
    }
}

pub fn error_is_definitely_rejected(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    matches!(
        error.kind.as_ref(),
        ErrorKind::Command(_)
            | ErrorKind::Write(WriteFailure::WriteError(_))
            | ErrorKind::InvalidArgument { .. }
            | ErrorKind::Authentication { .. }
            | ErrorKind::BsonDeserialization(_)
            | ErrorKind::BsonSerialization(_)
    )
}

/// A multi-document delete can fail after deleting some matching documents.
/// Neither command/write errors nor response decoding errors prove that it
/// removed nothing; only failures known to precede the write are definite.
fn bulk_delete_error_is_definitely_rejected(error: &mongodb::error::Error) -> bool {
    use mongodb::error::ErrorKind;
    matches!(
        error.kind.as_ref(),
        ErrorKind::InvalidArgument { .. }
            | ErrorKind::Authentication { .. }
            | ErrorKind::BsonSerialization(_)
    )
}

impl MongoMutationOutcome {
    pub const fn is_fully_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }
}

/// Builds one ordered delete command from the original loaded BSON snapshots.
/// Each statement compares the whole document and can delete at most one row,
/// including when `_id` is not globally unique across shards.
pub fn delete_documents_command(
    collection: &str,
    documents: &[Document],
) -> Result<Document, DatabaseError> {
    if collection.is_empty() || collection.contains('\0') {
        return Err(DatabaseError::configuration(
            "MongoDB collection namespace is required",
        ));
    }
    if collection.starts_with("system.") {
        return Err(DatabaseError::configuration(
            "MongoDB staged deletion requires an ordinary collection",
        ));
    }
    if documents.is_empty() || documents.len() > 100_000 {
        return Err(DatabaseError::configuration(
            "a document delete requires 1..=100000 original documents",
        ));
    }
    let deletes = documents
        .iter()
        .map(|document| {
            let id = document.get("_id").ok_or_else(|| {
                DatabaseError::configuration("a document delete requires each original _id")
            })?;
            if matches!(id, Bson::RegularExpression(_)) {
                return Err(DatabaseError::configuration(
                    "a document delete requires literal _id values",
                ));
            }
            Ok(doc! {
                "q": super::exact_document_guard(id.clone(), document),
                "limit": 1,
                "collation": { "locale": "simple" },
            })
        })
        .collect::<Result<Vec<_>, DatabaseError>>()?;
    let command = doc! { "delete": collection, "deletes": deletes, "ordered": true };
    validate_delete_command_size(&command)?;
    Ok(command)
}

fn validate_delete_command_size(command: &Document) -> Result<(), DatabaseError> {
    let encoded = mongodb::bson::to_vec(command)
        .map_err(|_| DatabaseError::configuration("MongoDB delete command is not valid BSON"))?;
    if encoded.len() > 16 * 1024 * 1024 {
        return Err(DatabaseError::configuration(
            "MongoDB delete command exceeds the 16 MiB BSON limit; select fewer documents",
        ));
    }
    Ok(())
}

fn apply_delete_write_concern(
    command: &mut Document,
    write_concern: Option<&WriteConcern>,
) -> Result<(), DatabaseError> {
    if let Some(concern) = write_concern {
        if concern.w == Some(Acknowledgment::Nodes(0)) {
            return Err(DatabaseError::configuration(
                "MongoDB staged deletion requires acknowledged write concern (w must not be 0)",
            ));
        }
        command.insert(
            "writeConcern",
            mongodb::bson::to_document(concern).map_err(|_| {
                DatabaseError::configuration("MongoDB write concern is not valid BSON")
            })?,
        );
        validate_delete_command_size(command)?;
    }
    Ok(())
}

/// The message the tab shows when a delete did not apply, or `None` when every
/// addressed document really was deleted.
///
/// A delete is not idempotent, so an outcome the server did not fully confirm
/// asks for a reload instead of a retry.
pub fn delete_outcome_message(outcome: &MongoMutationOutcome) -> Option<&'static str> {
    match outcome {
        MongoMutationOutcome::Confirmed { affected: 0 } => {
            Some("the document no longer exists in the collection")
        }
        MongoMutationOutcome::Confirmed { .. } => None,
        MongoMutationOutcome::Partial { .. } | MongoMutationOutcome::Unknown { .. } => {
            // The refresh the user needs is refused while a mark is pending, so
            // the message names the step that makes it possible instead of
            // telling them to reload and letting that fail.
            Some(
                "MongoDB delete outcome is unknown; discard the pending changes and reload the page before retrying",
            )
        }
    }
}

impl MongoAdapter {
    /// Deletes the marked rows only while their original BSON snapshots match.
    /// The standard delete command limits every statement to one document and
    /// preserves the database/client write concern. Ordinary collections need
    /// only collection metadata access, with no topology or index privileges.
    pub async fn delete_documents_detailed(
        &self,
        namespace: &MongoNamespace,
        documents: Vec<Document>,
    ) -> Result<MongoMutationOutcome, DatabaseError> {
        if namespace.database.is_empty()
            || namespace.database.contains('\0')
            || namespace.database.contains('.')
        {
            return Err(DatabaseError::configuration(
                "MongoDB database namespace is invalid",
            ));
        }
        let collection = namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        let mut command = delete_documents_command(collection, &documents)?;
        let database = self.client().database(&namespace.database);
        // run_command does not inherit write concern automatically. Reject w:0
        // before any I/O and explicitly preserve the configured acknowledgement.
        apply_delete_write_concern(&mut command, database.write_concern())?;
        let mut descriptions = database
            .list_collections()
            .filter(doc! { "name": collection })
            .await
            .map_err(map_error)?;
        let description = descriptions
            .try_next()
            .await
            .map_err(map_error)?
            .ok_or_else(|| {
                DatabaseError::configuration("MongoDB collection is no longer available")
            })?;
        if description.name != collection
            || !matches!(description.collection_type, CollectionType::Collection)
            || description.options.timeseries.is_some()
            || description.options.view_on.is_some()
            || description.options.capped == Some(true)
        {
            return Err(DatabaseError::configuration(
                "MongoDB staged deletion requires an ordinary collection",
            ));
        }
        let requested = documents.len() as u64;
        let result = database.run_command(command).await;
        match result {
            Ok(response) => Ok(delete_response_outcome(&response, requested)),
            Err(error) if bulk_delete_error_is_definitely_rejected(&error) => Err(map_error(error)),
            Err(_) => Ok(MongoMutationOutcome::Unknown { requested }),
        }
    }
}

fn delete_response_outcome(response: &Document, requested: u64) -> MongoMutationOutcome {
    let successful = matches!(response.get("ok"), Some(Bson::Int32(1) | Bson::Int64(1)))
        || response.get_f64("ok") == Ok(1.0);
    if !successful
        || response.contains_key("writeConcernError")
        || response
            .get("writeErrors")
            .is_some_and(|errors| !matches!(errors, Bson::Array(errors) if errors.is_empty()))
    {
        return MongoMutationOutcome::Unknown { requested };
    }
    let affected = match response.get("n") {
        Some(Bson::Int32(count)) if *count >= 0 => *count as u64,
        Some(Bson::Int64(count)) if *count >= 0 => *count as u64,
        _ => return MongoMutationOutcome::Unknown { requested },
    };
    bounded_delete_outcome(affected, requested)
}

/// The outcome of a bounded bulk delete, from how many documents the server
/// removed out of how many the filter addressed.
///
/// A delete that removed fewer documents than it addressed is `Partial`, never
/// `Confirmed`: at least one marked document was already gone. Removing more
/// documents than requested violates the bound and must remain `Unknown`.
fn bounded_delete_outcome(affected: u64, requested: u64) -> MongoMutationOutcome {
    if affected == requested {
        MongoMutationOutcome::Confirmed { affected }
    } else if affected > requested {
        MongoMutationOutcome::Unknown { requested }
    } else {
        MongoMutationOutcome::Partial {
            affected,
            requested,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MongoCompareAndSetOutcome, MongoMutationOutcome, compare_and_set_outcome,
        delete_outcome_message,
    };

    /// An irreversible delete must never look applied when it was not. Every
    /// outcome other than a confirmed delete reports something to the user, and
    /// an unknown outcome never invites a blind retry.
    #[test]
    fn the_delete_outcome_maps_every_outcome_safely() {
        assert_eq!(
            delete_outcome_message(&MongoMutationOutcome::Confirmed { affected: 1 }),
            None,
            "a delete that removed one document is the only silent outcome"
        );
        assert_eq!(
            delete_outcome_message(&MongoMutationOutcome::Confirmed { affected: 0 }),
            Some("the document no longer exists in the collection"),
            "deleting nothing is reported, not treated as success"
        );
        for undecided in [
            MongoMutationOutcome::Unknown { requested: 1 },
            MongoMutationOutcome::Partial {
                affected: 0,
                requested: 1,
            },
        ] {
            let message = delete_outcome_message(&undecided)
                .expect("an undecided delete must never look like success");
            assert!(
                message.contains("unknown") && message.contains("reload the page"),
                "an undecided delete asks for a reload instead of a retry, got {message}"
            );
        }
    }

    #[test]
    fn the_delete_command_limits_each_original_bson_snapshot_to_one_document() {
        use mongodb::bson::{doc, oid::ObjectId};

        let documents = vec![
            doc! { "_id": ObjectId::new(), "value": { "$gt": 0 } },
            doc! { "_id": "natural-key", "value": [1, 2, 3] },
        ];
        let command = super::delete_documents_command("items", &documents).unwrap();
        assert_eq!(command.get_str("delete"), Ok("items"));
        assert_eq!(command.get_bool("ordered"), Ok(true));
        let deletes = command.get_array("deletes").unwrap();
        assert_eq!(deletes.len(), documents.len());
        for (delete, original) in deletes.iter().zip(documents) {
            assert_eq!(
                delete.as_document().unwrap(),
                &doc! {
                    "q": {
                        "_id": { "$eq": original.get("_id").unwrap().clone() },
                        "$expr": { "$eq": ["$$ROOT", { "$literal": original }] },
                    },
                    "limit": 1,
                    "collation": { "locale": "simple" },
                },
                "the full typed snapshot is a literal and every statement is bounded"
            );
        }
    }

    #[test]
    fn invalid_delete_snapshots_are_refused_before_a_command_can_be_sent() {
        use mongodb::bson::{Bson, Regex, doc};

        for documents in [
            Vec::new(),
            vec![doc! { "value": 1 }],
            vec![doc! { "_id": Bson::RegularExpression(Regex {
                pattern: ".*".into(), options: String::new(),
            }) }],
            vec![doc! { "_id": 1, "value": "x".repeat(16 * 1024 * 1024) }],
        ] {
            assert!(super::delete_documents_command("items", &documents).is_err());
        }
        assert!(super::delete_documents_command("", &[doc! { "_id": 1 }]).is_err());
        assert!(super::delete_documents_command("bad\0name", &[doc! { "_id": 1 }]).is_err());
        assert!(super::delete_documents_command("system.items", &[doc! { "_id": 1 }]).is_err());
    }

    #[test]
    fn a_bulk_delete_that_removed_fewer_documents_is_partial() {
        assert_eq!(
            super::bounded_delete_outcome(3, 3),
            MongoMutationOutcome::Confirmed { affected: 3 },
            "removing every addressed document is the only confirmed outcome"
        );
        assert_eq!(
            super::bounded_delete_outcome(1, 3),
            MongoMutationOutcome::Partial {
                affected: 1,
                requested: 3
            },
            "a document that was already gone makes the outcome partial"
        );
        assert_eq!(
            super::bounded_delete_outcome(0, 2),
            MongoMutationOutcome::Partial {
                affected: 0,
                requested: 2
            },
            "an addressed set that vanished is partial, never confirmed"
        );
    }

    #[test]
    fn a_bulk_delete_that_exceeded_the_bound_is_unknown() {
        assert_eq!(
            super::bounded_delete_outcome(2, 1),
            MongoMutationOutcome::Unknown { requested: 1 },
            "removing two documents for one loaded id must never confirm success"
        );
    }

    #[test]
    fn bulk_delete_server_and_response_errors_do_not_prove_rejection() {
        use mongodb::{
            bson::{doc, from_document},
            error::{CommandError, ErrorKind, WriteError, WriteFailure},
        };
        for code in [13, 50, 112, 11601] {
            let response = doc! { "code": code, "errmsg": "delete failed" };
            let command = from_document::<CommandError>(response.clone()).unwrap();
            let write = from_document::<WriteError>(response).unwrap();
            for error in [
                ErrorKind::Command(command).into(),
                ErrorKind::Write(WriteFailure::WriteError(write)).into(),
            ] {
                assert!(!super::bulk_delete_error_is_definitely_rejected(&error));
            }
        }
        let decoding_error = from_document::<u64>(doc! {}).unwrap_err();
        assert!(!super::bulk_delete_error_is_definitely_rejected(
            &decoding_error.into()
        ));
        let serialization_error = ErrorKind::BsonSerialization(serde::ser::Error::custom(
            "cannot serialize the delete request",
        ))
        .into();
        assert!(super::bulk_delete_error_is_definitely_rejected(
            &serialization_error
        ));
    }

    #[test]
    fn delete_responses_confirm_only_valid_counts_without_server_errors() {
        use mongodb::bson::doc;

        assert_eq!(
            super::delete_response_outcome(&doc! { "ok": 1.0, "n": 2 }, 2),
            MongoMutationOutcome::Confirmed { affected: 2 }
        );
        assert_eq!(
            super::delete_response_outcome(&doc! { "ok": 1, "n": 1_i64, "writeErrors": [] }, 2),
            MongoMutationOutcome::Partial {
                affected: 1,
                requested: 2
            }
        );
        assert_eq!(
            super::delete_response_outcome(&doc! { "ok": 1, "n": 0 }, 2),
            MongoMutationOutcome::Partial {
                affected: 0,
                requested: 2
            }
        );
        for response in [
            doc! { "ok": 1, "n": 2, "writeErrors": [{ "index": 1, "code": 50 }] },
            doc! { "ok": 1, "n": 2, "writeConcernError": { "code": 64 } },
            doc! { "ok": 1, "n": 2, "writeErrors": "malformed" },
            doc! { "ok": 1, "n": 3 },
            doc! { "ok": 1, "n": -1 },
            doc! { "ok": 1, "n": "2" },
            doc! { "ok": 1 },
            doc! { "n": 2 },
            doc! { "ok": 0, "n": 2 },
        ] {
            assert_eq!(
                super::delete_response_outcome(&response, 2),
                MongoMutationOutcome::Unknown { requested: 2 },
                "an error or invalid count cannot prove a fully applied delete: {response:?}"
            );
        }
    }

    #[test]
    fn raw_delete_preserves_write_concern_and_refuses_unacknowledged_writes() {
        use mongodb::{
            bson::doc,
            options::{Acknowledgment, WriteConcern},
        };

        let mut command = super::delete_documents_command("items", &[doc! { "_id": 1 }]).unwrap();
        let default = command.clone();
        super::apply_delete_write_concern(&mut command, None).unwrap();
        assert_eq!(
            command, default,
            "an unspecified concern keeps the server default"
        );
        let acknowledged = WriteConcern::builder()
            .w(Acknowledgment::Majority)
            .w_timeout(std::time::Duration::from_millis(1234))
            .journal(true)
            .build();
        super::apply_delete_write_concern(&mut command, Some(&acknowledged)).unwrap();
        assert_eq!(
            command.get_document("writeConcern").unwrap(),
            &mongodb::bson::to_document(&acknowledged).unwrap()
        );
        let unacknowledged = WriteConcern::builder().w(Acknowledgment::Nodes(0)).build();
        assert!(super::apply_delete_write_concern(&mut command, Some(&unacknowledged)).is_err());
    }

    #[test]
    fn certainty_distinguishes_partial_and_unknown_from_confirmed() {
        assert!(MongoMutationOutcome::Confirmed { affected: 2 }.is_fully_confirmed());
        assert!(
            !MongoMutationOutcome::Partial {
                affected: 1,
                requested: 3
            }
            .is_fully_confirmed()
        );
        assert!(!MongoMutationOutcome::Unknown { requested: 1 }.is_fully_confirmed());
    }

    #[test]
    fn cas_requires_one_match_and_distinguishes_conflict_from_noop() {
        assert_eq!(
            compare_and_set_outcome(0, Some(0)),
            MongoCompareAndSetOutcome::Conflict
        );
        assert_eq!(
            compare_and_set_outcome(1, Some(1)),
            MongoCompareAndSetOutcome::Applied
        );
        assert_eq!(
            compare_and_set_outcome(1, Some(0)),
            MongoCompareAndSetOutcome::Conflict
        );
        assert_eq!(
            compare_and_set_outcome(1, None),
            MongoCompareAndSetOutcome::Unknown
        );
    }

    #[test]
    fn insert_many_partial_count_uses_unique_failed_indexes() {
        let failed_indexes = [1usize, 3, 3];
        let affected = 5u64.saturating_sub(
            failed_indexes
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .len() as u64,
        );
        assert_eq!(affected, 3);
        assert_eq!(
            MongoMutationOutcome::Partial {
                affected,
                requested: 5
            },
            MongoMutationOutcome::Partial {
                affected: 3,
                requested: 5
            }
        );
    }
}
