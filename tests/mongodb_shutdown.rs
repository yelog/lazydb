mod support;

use std::time::Duration;

use lazydb::{
    db::{DatabaseConnection, mongodb::MongoAdapter},
    profile::import_connection_url,
};

fn with_database(url: &str, database: &str) -> String {
    let (address, query) = url.split_once('?').unwrap_or((url, ""));
    let address = address.trim_end_matches('/');
    if query.is_empty() {
        format!("{address}/{database}")
    } else {
        format!("{address}/{database}?{query}")
    }
}

async fn connect(url: &str) -> MongoAdapter {
    let imported = import_connection_url(url, Some("mongodb-shutdown-test")).unwrap();
    match DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
        .await
        .unwrap()
    {
        DatabaseConnection::MongoDb(adapter) => adapter,
        _ => panic!("expected MongoDB adapter"),
    }
}

#[tokio::test]
async fn close_is_prompt_when_the_mongodb_server_is_healthy() {
    let Some(url) = support::mongodb_standalone_url() else {
        return;
    };
    let database = format!("lazydb_shutdown_{}", uuid::Uuid::new_v4().simple());
    let adapter = connect(&with_database(&url, &database)).await;

    tokio::time::timeout(Duration::from_secs(2), adapter.close())
        .await
        .expect("healthy MongoDB close must finish promptly");
}

#[tokio::test]
async fn multiple_client_closes_share_one_parallel_deadline_with_live_sessions() {
    let Some(url) = support::mongodb_standalone_url() else {
        return;
    };
    let mut adapters = Vec::new();
    let mut retained_sessions = Vec::new();
    for _ in 0..4 {
        let database = format!("lazydb_shutdown_{}", uuid::Uuid::new_v4().simple());
        let adapter = connect(&with_database(&url, &database)).await;
        let client = adapter.client().clone();
        let mut session = client.start_session().await.expect("start session");
        client
            .database(&database)
            .run_command(mongodb::bson::doc! { "ping": 1 })
            .session(&mut session)
            .await
            .expect("use session against the live server");
        adapters.push(adapter);
        retained_sessions.push((client, session));
    }

    let started = tokio::time::Instant::now();
    tokio::time::timeout(
        Duration::from_millis(1_250),
        futures_util::future::join_all(adapters.into_iter().map(MongoAdapter::close)),
    )
    .await
    .expect("parallel MongoDB cleanup must use one shared one-second budget");
    assert!(started.elapsed() < Duration::from_millis(1_250));

    drop(retained_sessions);
}
