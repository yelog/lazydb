use mongodb::{
    ClientSession,
    bson::{Document, doc},
};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct MongoTransactionWorkerSender(mpsc::UnboundedSender<MongoTransactionRequest>);

impl PartialEq for MongoTransactionWorkerSender {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_channel(&other.0)
    }
}

impl Eq for MongoTransactionWorkerSender {}

impl MongoTransactionWorkerSender {
    pub fn send(
        &self,
        request: MongoTransactionRequest,
    ) -> Result<(), mpsc::error::SendError<MongoTransactionRequest>> {
        self.0.send(request)
    }
}

use crate::db::DatabaseError;

use super::MongoAdapter;
use crate::mongodb_command::{MongoCommandRisk, parse_command};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MongoTransactionOutcome {
    Committed,
    Aborted,
    Unknown,
}

pub fn supports_transactions(hello: &Document) -> bool {
    hello.get_str("setName").is_ok() || hello.get_str("msg").ok() == Some("isdbgrid")
}

pub struct MongoSessionTransaction {
    session: ClientSession,
    active: bool,
    route: crate::db::transport::ConnectionRoute,
}

impl Drop for MongoSessionTransaction {
    fn drop(&mut self) {
        // ClientSession owns the server session. If a worker is cancelled while
        // an operation is in flight, dropping it releases that session and the
        // server aborts the unfinished transaction.
        self.active = false;
    }
}

#[derive(Debug)]
pub enum MongoTransactionRequest {
    Command { database: String, command: Document },
    Commit,
    Abort,
}

pub fn spawn_transaction_worker(
    mut transaction: MongoSessionTransaction,
    database: String,
    tab_id: uuid::Uuid,
    generation: u64,
    sender: tokio::sync::mpsc::UnboundedSender<crate::action::Action>,
) -> MongoTransactionWorkerSender {
    let (request_sender, mut requests) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            match request {
                MongoTransactionRequest::Command {
                    database: request_database,
                    command,
                } => {
                    if request_database != database {
                        let _ = sender.send(crate::action::Action::MongoTransactionFailed { tab_id, generation, message: "Mongo transaction command database does not match the selected database".into() });
                        break;
                    }
                    match transaction.run_command(&database, command).await {
                        Ok(reply) => {
                            let reply_ejson = mongodb::bson::Bson::Document(reply)
                                .into_relaxed_extjson()
                                .to_string();
                            let _ = sender.send(
                                crate::action::Action::MongoTransactionCommandCompleted {
                                    tab_id,
                                    generation,
                                    reply_ejson,
                                },
                            );
                        }
                        Err(error) => {
                            let message = error.message.clone();
                            let outcome = transaction
                                .abort()
                                .await
                                .unwrap_or(MongoTransactionOutcome::Unknown);
                            let _ = sender.send(crate::action::Action::MongoTransactionFailed {
                                tab_id,
                                generation,
                                message: if outcome == MongoTransactionOutcome::Aborted {
                                    format!("Transaction command failed and was aborted: {message}")
                                } else {
                                    format!("Transaction command failed; abort outcome is unknown: {message}")
                                },
                            });
                            break;
                        }
                    }
                }
                MongoTransactionRequest::Commit => {
                    let outcome = transaction
                        .commit()
                        .await
                        .unwrap_or(MongoTransactionOutcome::Unknown);
                    let _ = sender.send(crate::action::Action::MongoTransactionFinished {
                        tab_id,
                        generation,
                        outcome,
                    });
                    break;
                }
                MongoTransactionRequest::Abort => {
                    let outcome = transaction
                        .abort()
                        .await
                        .unwrap_or(MongoTransactionOutcome::Unknown);
                    let _ = sender.send(crate::action::Action::MongoTransactionFinished {
                        tab_id,
                        generation,
                        outcome,
                    });
                    break;
                }
            }
        }
    });
    MongoTransactionWorkerSender(request_sender)
}

impl MongoAdapter {
    pub async fn begin_session_transaction(
        &self,
    ) -> Result<MongoSessionTransaction, DatabaseError> {
        let hello = self
            .checked_client()?
            .database("admin")
            .run_command(doc! { "hello": 1 })
            .await
            .map_err(super::map_error)?;
        if !supports_transactions(&hello) {
            return Err(DatabaseError::unsupported(
                "MongoDB multi-document transactions require a replica set or sharded cluster",
            ));
        }
        let mut session = self
            .checked_client()?
            .start_session()
            .await
            .map_err(super::map_error)?;
        session
            .start_transaction()
            .await
            .map_err(super::map_error)?;
        Ok(MongoSessionTransaction {
            session,
            active: true,
            route: self.route.clone(),
        })
    }
}

impl MongoSessionTransaction {
    pub async fn run_command(
        &mut self,
        database: &str,
        command: Document,
    ) -> Result<Document, DatabaseError> {
        if !self.active {
            return Err(DatabaseError::configuration(
                "MongoDB transaction is no longer active",
            ));
        }
        self.route.ensure_alive()?;
        let parsed = parse_command(
            &mongodb::bson::to_bson(&command)
                .map_err(|error| DatabaseError::configuration(error.to_string()))?
                .into_relaxed_extjson()
                .to_string(),
            database,
        )
        .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        if parsed.risk != MongoCommandRisk::ReadOnly
            || parsed.kind != crate::mongodb_command::MongoCommandKind::Find
        {
            return Err(DatabaseError::unsupported(
                "Mongo transactions currently accept only supported collection find commands",
            ));
        }
        parsed
            .validate_supported_fields()
            .map_err(|error| DatabaseError::unsupported(error.to_string()))?;
        self.session
            .client()
            .database(database)
            .run_command(command)
            .session(&mut self.session)
            .await
            .map_err(super::map_error)
    }

    pub async fn commit(&mut self) -> Result<MongoTransactionOutcome, DatabaseError> {
        if !self.active {
            return Err(DatabaseError::configuration(
                "MongoDB transaction is no longer active",
            ));
        }
        self.route.ensure_alive()?;
        match self.session.commit_transaction().await {
            Ok(()) => {
                self.active = false;
                Ok(MongoTransactionOutcome::Committed)
            }
            Err(error) if has_label(&error, "UnknownTransactionCommitResult") => {
                for _ in 0..2 {
                    match self.session.commit_transaction().await {
                        Ok(()) => {
                            self.active = false;
                            return Ok(MongoTransactionOutcome::Committed);
                        }
                        Err(retry_error)
                            if has_label(&retry_error, "UnknownTransactionCommitResult") =>
                        {
                            continue;
                        }
                        Err(_retry_error) => {
                            self.active = false;
                            return Ok(MongoTransactionOutcome::Unknown);
                        }
                    }
                }
                self.active = false;
                Ok(MongoTransactionOutcome::Unknown)
            }
            Err(error) if has_label(&error, "TransientTransactionError") => {
                self.active = false;
                Ok(MongoTransactionOutcome::Aborted)
            }
            Err(_) => {
                self.active = false;
                Ok(MongoTransactionOutcome::Unknown)
            }
        }
    }

    pub async fn abort(&mut self) -> Result<MongoTransactionOutcome, DatabaseError> {
        if !self.active {
            return Err(DatabaseError::configuration(
                "MongoDB transaction is no longer active",
            ));
        }
        self.route.ensure_alive()?;
        match self.session.abort_transaction().await {
            Ok(()) => {
                self.active = false;
                Ok(MongoTransactionOutcome::Aborted)
            }
            Err(error) => {
                self.active = false;
                let _ = error;
                Ok(MongoTransactionOutcome::Unknown)
            }
        }
    }

    pub async fn abort_on_drop(mut self) {
        if self.active {
            if self.route.is_alive() {
                let _ = self.session.abort_transaction().await;
            }
            self.active = false;
        }
    }
}

fn has_label(error: &mongodb::error::Error, label: &str) -> bool {
    error.contains_label(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_commands_use_the_existing_read_capability_policy() {
        let read = parse_command(r#"{"find":"items","filter":{}}"#, "app").unwrap();
        assert_eq!(read.risk, MongoCommandRisk::ReadOnly);
        assert!(read.capability().is_ok());
        assert!(read.validate_supported_fields().is_ok());

        let write = parse_command(r#"{"insert":"items","documents":[{"x":1}]}"#, "app").unwrap();
        assert_ne!(write.risk, MongoCommandRisk::ReadOnly);
        assert!(write.capability().is_err());
        assert!(parse_command(r#"{"find":"items","$db":"admin"}"#, "app").is_err());
        for command in [
            r#"{"insert":"items","documents":[{"x":1}]}"#,
            r#"{"update":"items","updates":[{"q":{"_id":1},"u":{"$set":{"x":2}}}]}"#,
            r#"{"delete":"items","deletes":[{"q":{"_id":1}}]}"#,
        ] {
            let parsed = parse_command(command, "app").unwrap();
            assert_ne!(parsed.risk, MongoCommandRisk::ReadOnly);
            assert_ne!(parsed.kind, crate::mongodb_command::MongoCommandKind::Find);
        }
        let drop = parse_command(r#"{"drop":"items"}"#, "app").unwrap();
        assert_eq!(drop.kind, crate::mongodb_command::MongoCommandKind::Drop);
        assert!(!matches!(
            drop.kind,
            crate::mongodb_command::MongoCommandKind::Find
                | crate::mongodb_command::MongoCommandKind::Insert
                | crate::mongodb_command::MongoCommandKind::Update
                | crate::mongodb_command::MongoCommandKind::Delete
        ));
    }

    #[test]
    fn transactions_require_replica_set_or_mongos_hello_shape() {
        assert!(supports_transactions(
            &mongodb::bson::doc! { "setName": "rs0" }
        ));
        assert!(supports_transactions(
            &mongodb::bson::doc! { "msg": "isdbgrid" }
        ));
        assert!(!supports_transactions(
            &mongodb::bson::doc! { "ismaster": true }
        ));
    }
}
