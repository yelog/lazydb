use mongodb::{
    Client,
    bson::{Document, doc},
};

#[derive(Clone, Debug, PartialEq)]
pub struct MongoMonitorSnapshot {
    pub sampled_at: std::time::SystemTime,
    pub server_status: Option<Document>,
    pub database_stats: Option<Document>,
    pub collection_stats: Option<Document>,
    pub current_operations: Vec<Document>,
    pub errors: Vec<MongoMonitorError>,
}

impl Default for MongoMonitorSnapshot {
    fn default() -> Self {
        Self {
            sampled_at: std::time::SystemTime::UNIX_EPOCH,
            server_status: None,
            database_stats: None,
            collection_stats: None,
            current_operations: Vec::new(),
            errors: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MongoMonitorError {
    pub command: &'static str,
    pub message: String,
}

pub async fn sample_adapter(
    adapter: &super::MongoAdapter,
    database: &str,
    collection: Option<&str>,
) -> MongoMonitorSnapshot {
    let client = match adapter.checked_client() {
        Ok(client) => client,
        Err(error) => {
            return MongoMonitorSnapshot {
                sampled_at: std::time::SystemTime::now(),
                errors: vec![MongoMonitorError {
                    command: "connection",
                    message: error.to_string(),
                }],
                ..MongoMonitorSnapshot::default()
            };
        }
    };
    sample_monitor(client, database, collection).await
}

pub async fn sample_monitor(
    client: &Client,
    database: &str,
    collection: Option<&str>,
) -> MongoMonitorSnapshot {
    let mut snapshot = MongoMonitorSnapshot {
        sampled_at: std::time::SystemTime::now(),
        ..MongoMonitorSnapshot::default()
    };
    let admin = client.database("admin");
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        admin.run_command(doc! {"serverStatus": 1}),
    )
    .await
    {
        Err(_) => snapshot.errors.push(MongoMonitorError {
            command: "serverStatus",
            message: "monitor request timed out".into(),
        }),
        Ok(Ok(value)) => snapshot.server_status = Some(value),
        Ok(Err(error)) => snapshot.errors.push(MongoMonitorError {
            command: "serverStatus",
            message: error.to_string(),
        }),
    }
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.database(database).run_command(doc! {"dbStats": 1}),
    )
    .await
    {
        Err(_) => snapshot.errors.push(MongoMonitorError {
            command: "dbStats",
            message: "monitor request timed out".into(),
        }),
        Ok(Ok(value)) => snapshot.database_stats = Some(value),
        Ok(Err(error)) => snapshot.errors.push(MongoMonitorError {
            command: "dbStats",
            message: error.to_string(),
        }),
    }
    if let Some(collection) = collection {
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client
                .database(database)
                .run_command(doc! {"collStats": collection}),
        )
        .await
        {
            Err(_) => snapshot.errors.push(MongoMonitorError {
                command: "collStats",
                message: "monitor request timed out".into(),
            }),
            Ok(Ok(value)) => snapshot.collection_stats = Some(value),
            Ok(Err(error)) => snapshot.errors.push(MongoMonitorError {
                command: "collStats",
                message: error.to_string(),
            }),
        }
    }
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        admin.run_command(doc! {"currentOp": 1, "$all": true}),
    )
    .await
    {
        Err(_) => snapshot.errors.push(MongoMonitorError {
            command: "currentOp",
            message: "monitor request timed out".into(),
        }),
        Ok(Ok(value)) => {
            if let Ok(operations) = value.get_array("inprog") {
                snapshot.current_operations = operations
                    .iter()
                    .filter_map(|value| value.as_document().cloned())
                    .take(500)
                    .collect();
            }
        }
        Ok(Err(error)) => snapshot.errors.push(MongoMonitorError {
            command: "currentOp",
            message: error.to_string(),
        }),
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_snapshot_keeps_partial_results_and_errors() {
        let snapshot = MongoMonitorSnapshot {
            sampled_at: std::time::SystemTime::UNIX_EPOCH,
            server_status: Some(doc! {"ok": 1}),
            database_stats: None,
            collection_stats: None,
            current_operations: Vec::new(),
            errors: vec![MongoMonitorError {
                command: "currentOp",
                message: "unauthorized".into(),
            }],
        };
        assert!(snapshot.server_status.is_some());
        assert_eq!(snapshot.errors[0].command, "currentOp");
    }
}
