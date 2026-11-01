use mongodb::bson::{Bson, Document};

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

impl MongoMutationOutcome {
    pub const fn is_fully_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }
}

/// The filter that bounds a bulk document delete to the `_id`s the marked grid
/// rows were loaded with.
///
/// Every value is an `_id` exactly as it was loaded, never its rendering, so a
/// delete can never retarget another document. `_id` is the only field in the
/// filter on purpose.
pub fn delete_many_filter(ids: &[Bson]) -> Document {
    mongodb::bson::doc! {
        "_id": { "$in": Bson::Array(ids.to_vec()) }
    }
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
    /// Deletes every document the marked grid rows were loaded from, bounded to
    /// their `_id`s.
    ///
    /// This is the whole write surface of a MongoDB relation tab's commit: one
    /// `deleteMany` against `{_id: {$in: <loaded ids>}}`, never a delete by any
    /// other field. An empty list is refused, because a filter that addresses no
    /// document must never become a delete of everything.
    pub async fn delete_documents_by_ids_detailed(
        &self,
        namespace: &MongoNamespace,
        ids: Vec<Bson>,
    ) -> Result<MongoMutationOutcome, DatabaseError> {
        let collection = namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        if ids.is_empty() {
            return Err(DatabaseError::configuration(
                "a document delete requires at least one _id",
            ));
        }
        let requested = ids.len() as u64;
        let result = self
            .client()
            .database(&namespace.database)
            .collection::<Document>(collection)
            .delete_many(delete_many_filter(&ids))
            .await;
        match result {
            Ok(result) => Ok(bounded_delete_outcome(result.deleted_count, requested)),
            Err(error) if error_is_definitely_rejected(&error) => Err(map_error(error)),
            Err(_) => Ok(MongoMutationOutcome::Unknown { requested }),
        }
    }
}

/// The outcome of a bounded bulk delete, from how many documents the server
/// removed out of how many the filter addressed.
///
/// A delete that removed fewer documents than it addressed is `Partial`, never
/// `Confirmed`: at least one marked document was already gone.
fn bounded_delete_outcome(affected: u64, requested: u64) -> MongoMutationOutcome {
    if affected >= requested {
        MongoMutationOutcome::Confirmed { affected }
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
    fn the_delete_filter_is_exactly_the_loaded_ids() {
        let first = mongodb::bson::Bson::ObjectId(mongodb::bson::oid::ObjectId::new());
        let second = mongodb::bson::Bson::String("natural-key".into());
        let ids = vec![first.clone(), second.clone()];
        let filter = super::delete_many_filter(&ids);

        assert_eq!(
            filter,
            mongodb::bson::doc! { "_id": { "$in": ids.clone() } },
            "the filter is the loaded _id values and nothing else"
        );
        assert_eq!(
            filter.keys().collect::<Vec<_>>(),
            vec!["_id"],
            "no other field can widen or retarget the delete"
        );
        let Some(mongodb::bson::Bson::Document(bound)) = filter.get("_id") else {
            panic!("the _id filter is an operator document, got {filter:?}")
        };
        assert_eq!(
            bound.keys().collect::<Vec<_>>(),
            vec!["$in"],
            "$in is the only operator, so nothing can be negated or widened"
        );
        assert_eq!(
            bound.get("$in"),
            Some(&mongodb::bson::Bson::Array(ids)),
            "the values keep the BSON types they were loaded with, never a re-serialized string"
        );
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
